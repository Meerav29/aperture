//! Read-only adapters for externally owned session logs. No provider commands or settings writes.
use super::{
    model::{Host, IntegrationHealth, Session, SessionStatus},
    state::Store,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_LINE: usize = 1024 * 1024;

#[derive(Default)]
struct Cursor {
    offset: u64,
    initial_len: u64,
    malformed: usize,
    /// Complete lines over `MAX_LINE`, skipped unparsed. Kept apart from
    /// `malformed` because a valid record can be this large (a user prompt
    /// with pasted screenshots inlined as base64, #95), so it is not
    /// evidence of a broken transcript.
    oversized: usize,
    id: Option<String>,
    host: Option<Host>,
    created: Option<std::time::SystemTime>,
}

pub struct Observer {
    roots: Vec<(String, PathBuf)>,
    cursors: HashMap<PathBuf, Cursor>,
    /// Test-only replacement for `hook_bridge::inbox()`, so a test's `poll`
    /// does not fold in this machine's real hook inbox. `None` in production.
    hook_inbox: Option<PathBuf>,
}

impl Default for Observer {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"));
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        Self::new(claude.join("projects"), codex.join("sessions"))
    }
}

impl Observer {
    pub fn new(claude: PathBuf, codex: PathBuf) -> Self {
        Self {
            roots: vec![("claude_code".into(), claude), ("codex".into(), codex)],
            cursors: HashMap::new(),
            hook_inbox: None,
        }
    }

    #[cfg(test)]
    fn with_hook_inbox(mut self, inbox: PathBuf) -> Self {
        self.hook_inbox = Some(inbox);
        self
    }

    pub fn poll(&mut self, store: &mut Store) {
        let hook_errors = self
            .hook_inbox
            .clone()
            .or_else(super::hook_bridge::inbox)
            .map(|root| super::hook_bridge::poll(&root, store))
            .unwrap_or(0);
        let mut health = Vec::new();
        for (provider, root) in &self.roots {
            let mut files = Vec::new();
            let mut errors = 0;
            discover(root, &mut files, &mut errors, 0);
            files.sort();
            let mut malformed = 0;
            let mut oversized = 0;
            for path in &files {
                let initial = !self.cursors.contains_key(path);
                let cursor = self.cursors.entry(path.clone()).or_default();
                match read_file(path, provider, cursor, store, initial) {
                    Ok(_) => {
                        malformed += cursor.malformed;
                        oversized += cursor.oversized;
                    }
                    Err(_) => errors += 1,
                }
            }
            let last = store
                .sessions
                .values()
                .filter(|s| &s.provider == provider)
                .map(|s| s.last_event_at)
                .max();
            health.push(IntegrationHealth {
                provider: provider.clone(),
                root: root.to_string_lossy().into_owned(),
                files: files.len(),
                last_event_at: last,
                state: passive_state(errors, malformed, files.len()).into(),
                detail: passive_detail(errors, malformed, oversized),
            });
        }
        store.integrations = health;
        for integration in &mut store.integrations {
            let last_hook = store.hook_activity.get(&integration.provider);
            integration.detail = format!(
                "Read-only files; debounced file watching, 5s reconcile. Optional hook metadata: {}. Permission requests are observations, not proof a dialog remains open. Process liveness unknown. {hook_errors} hook inbox errors. Provider settings untouched.",
                last_hook.map(|at| format!("last received {}", at.to_rfc3339())).unwrap_or_else(|| "not observed; permission coverage unknown".into())
            ) + &format!(" {}", integration.detail);
            if hook_errors > 0 {
                integration.state = "degraded".into();
            }
            if let Some(at) = last_hook {
                integration.last_event_at =
                    Some(integration.last_event_at.map_or(*at, |old| old.max(*at)));
            }
        }
        store.revision += 1;
    }

    /// Provider transcript roots, for the filesystem watcher to subscribe to.
    pub fn watch_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|(_, p)| p.clone()).collect()
    }

    /// Seed cursors from persisted state (Task 1's `db::CursorRecord`), so
    /// tailing resumes at the saved byte offset instead of re-reading a file
    /// from the start after a restart.
    pub fn restore_cursors(&mut self, records: Vec<super::db::CursorRecord>) {
        for r in records {
            self.cursors.insert(
                PathBuf::from(&r.path),
                Cursor {
                    offset: r.offset,
                    initial_len: r.initial_len,
                    malformed: r.malformed as usize,
                    oversized: r.oversized as usize,
                    id: r.session_id,
                    host: r.host.as_deref().map(host_from_str),
                    created: r
                        .created_ns
                        .map(|ns| std::time::UNIX_EPOCH + std::time::Duration::from_nanos(ns as u64)),
                },
            );
        }
    }

    /// Export current cursor state for persistence.
    pub fn export_cursors(&self) -> Vec<super::db::CursorRecord> {
        self.cursors
            .iter()
            .map(|(path, c)| super::db::CursorRecord {
                path: path.to_string_lossy().into_owned(),
                provider: self.provider_for(path).unwrap_or_default(),
                offset: c.offset,
                initial_len: c.initial_len,
                malformed: c.malformed as u64,
                oversized: c.oversized as u64,
                session_id: c.id.clone(),
                host: c.host.map(host_to_str),
                created_ns: c
                    .created
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64),
            })
            .collect()
    }

    fn provider_for(&self, path: &Path) -> Option<String> {
        self.roots
            .iter()
            .find(|(_, root)| path.starts_with(root))
            .map(|(p, _)| p.clone())
    }
}

/// Oversized records are deliberately absent: only read errors and
/// unparseable records mean the transcript could not be read as written.
fn passive_state(errors: usize, malformed: usize, files: usize) -> &'static str {
    if errors > 0 || malformed > 0 {
        "degraded"
    } else if files == 0 {
        "no_sessions"
    } else {
        "watching"
    }
}

/// Oversized skips stay in the detail even though they no longer degrade the
/// row: the record's event was still not applied, and that loss is reported.
fn passive_detail(errors: usize, malformed: usize, oversized: usize) -> String {
    let mut detail = format!("{errors} file read errors; {malformed} skipped records.");
    if oversized > 0 {
        let records = if oversized == 1 { "record" } else { "records" };
        detail += &format!(
            " {oversized} oversized {records} over 1 MiB skipped unread (likely inline attachments such as pasted images); their events were not applied."
        );
    }
    detail
}

fn discover(root: &Path, out: &mut Vec<PathBuf>, errors: &mut usize, depth: usize) {
    if depth > 8 {
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => {
            *errors += 1;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *errors += 1;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *errors += 1;
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let p = entry.path();
        // Claude subagents belong to their parent and must not replace its lifecycle.
        if kind.is_dir() && entry.file_name() != "subagents" {
            discover(&p, out, errors, depth + 1);
        } else if kind.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

fn read_file(
    path: &Path,
    provider: &str,
    cursor: &mut Cursor,
    store: &mut Store,
    initial: bool,
) -> std::io::Result<usize> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    let replaced = cursor.created.is_some() && cursor.created != meta.created().ok();
    if meta.len() < cursor.offset || replaced {
        cursor.offset = 0;
        cursor.id = None;
        cursor.host = None;
        cursor.initial_len = meta.len();
        cursor.malformed = 0;
        cursor.oversized = 0;
    }
    if initial {
        cursor.initial_len = meta.len();
    }
    cursor.created = meta.created().ok();
    file.seek(SeekFrom::Start(cursor.offset))?;
    let mut reader = BufReader::new(file);
    let mut malformed = 0;
    let mut oversized = 0;
    // Bounded per poll, with incomplete records left for the next poll.
    let mut consumed = 0;
    while consumed < 8 * 1024 * 1024 {
        let mut bytes = Vec::new();
        let mut total = 0u64;
        let mut complete = false;
        loop {
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            let n = chunk
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(chunk.len());
            complete = chunk[n - 1] == b'\n';
            if bytes.len() + n <= MAX_LINE {
                bytes.extend_from_slice(&chunk[..n]);
            }
            total += n as u64;
            reader.consume(n);
            if complete {
                break;
            }
        }
        if !complete {
            break;
        }
        cursor.offset += total;
        consumed += total;
        if total > MAX_LINE as u64 {
            oversized += 1;
            continue;
        }
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => {
                if let Some(event) = adapt(provider, &v, &mut cursor.id, &mut cursor.host) {
                    apply(
                        store,
                        provider,
                        path,
                        event,
                        cursor.offset > cursor.initial_len,
                    );
                }
            }
            Err(_) => malformed += 1,
        }
    }
    cursor.malformed += malformed;
    cursor.oversized += oversized;
    Ok(malformed)
}

struct Event {
    id: String,
    at: DateTime<Utc>,
    cwd: Option<String>,
    status: SessionStatus,
    activity: String,
    attention: &'static str,
    host: Option<Host>,
}

fn adapt(
    provider: &str,
    v: &Value,
    id: &mut Option<String>,
    host: &mut Option<Host>,
) -> Option<Event> {
    let kind = v["type"].as_str()?;
    let (status, activity, attention, cwd) = if provider == "codex" {
        let p = &v["payload"];
        if kind == "session_meta" {
            *id = p["id"].as_str().map(str::to_owned);
            // Prefer self-reported host metadata over process-name guesses.
            // Missing or unfamiliar metadata remains Unknown.
            *host = Some(infer_codex_host(
                p["originator"].as_str(),
                p["source"].as_str(),
            ));
        }
        let subtype = p["type"].as_str().unwrap_or("");
        let (state, text) = match (kind, subtype) {
            ("session_meta", _) => (SessionStatus::Unknown, "Session discovered".to_string()),
            ("turn_context", _) => (SessionStatus::Working, "Turn context updated".into()),
            ("event_msg", "task_started" | "turn_started" | "user_message") => {
                (SessionStatus::Working, "Thinking".into())
            }
            ("event_msg", "task_complete" | "turn_complete" | "turn_completed") => {
                (SessionStatus::Idle, "Turn complete".into())
            }
            ("event_msg", "turn_aborted") => (SessionStatus::Idle, "Turn interrupted".into()),
            ("response_item", "function_call" | "custom_tool_call") => (
                SessionStatus::Working,
                format!("Tool: {}", safe_name(p["name"].as_str())),
            ),
            ("response_item", "function_call_output" | "custom_tool_call_output") => {
                (SessionStatus::Working, "Tool returned".into())
            }
            ("response_item", "reasoning" | "message")
                if p["role"] != "user" && p["role"] != "developer" =>
            {
                (SessionStatus::Working, "Generating response".into())
            }
            _ => return None,
        };
        let input = matches!(subtype, "function_call" | "custom_tool_call")
            && matches!(
                p["name"].as_str(),
                Some(
                    "request_user_input"
                        | "functions.request_user_input"
                        | "functions.request_user_input_async"
                )
            );
        (
            if input { SessionStatus::Blocked } else { state },
            text,
            if input { "explicit_input" } else { "unknown" },
            p["cwd"].as_str().map(str::to_owned),
        )
    } else {
        if v["isSidechain"].as_bool() == Some(true) {
            return None;
        }
        if let Some(entrypoint) = v["entrypoint"].as_str() {
            *host = Some(match entrypoint {
                "claude-desktop" => Host::DesktopApp,
                "claude-vscode" => Host::VsCode,
                "cli" => Host::Terminal,
                _ => Host::Unknown,
            });
        }
        if let Some(native) = v["sessionId"].as_str() {
            *id = Some(native.into());
        }
        let (state, text, attention) = match kind {
            "user" if v["isMeta"].as_bool() != Some(true) => (
                SessionStatus::Working,
                "Prompt or tool result received".into(),
                "unknown",
            ),
            "assistant" => {
                // Parallel tool calls share one message, so a question may sit
                // behind unrelated tool_use blocks; it outranks them.
                let tools: Vec<&Value> = v
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter(|b| b["type"] == "tool_use").collect())
                    .unwrap_or_default();
                let tool = tools
                    .iter()
                    .find(|b| b["name"] == "AskUserQuestion")
                    .or_else(|| tools.first());
                if let Some(tool) = tool {
                    let input = tool["name"] == "AskUserQuestion";
                    (
                        if input {
                            SessionStatus::Blocked
                        } else {
                            SessionStatus::Working
                        },
                        format!("Tool: {}", safe_name(tool["name"].as_str())),
                        if input { "explicit_input" } else { "unknown" },
                    )
                } else if v.pointer("/message/stop_reason").and_then(Value::as_str)
                    == Some("end_turn")
                {
                    (SessionStatus::Idle, "Turn complete".into(), "unknown")
                } else {
                    (
                        SessionStatus::Working,
                        "Generating response".into(),
                        "unknown",
                    )
                }
            }
            _ => return None,
        };
        (state, text, attention, v["cwd"].as_str().map(str::to_owned))
    };
    Some(Event {
        id: id.clone()?,
        at: v["timestamp"].as_str()?.parse().ok()?,
        cwd,
        status,
        activity,
        attention,
        host: *host,
    })
}

/// Map Codex's self-reported `session_meta` fields to a host kind. Verified
/// real values: "Codex Desktop" (Desktop), "codex_vscode" (VS Code), and
/// "codex_exec" for a non-interactive `codex exec` run in a plain terminal
/// (Headless). Explicit CLI metadata maps to Terminal; the interactive TUI
/// still needs live validation. Missing or unfamiliar values remain Unknown.
fn infer_codex_host(originator: Option<&str>, source: Option<&str>) -> Host {
    let o = originator.unwrap_or("").to_ascii_lowercase();
    let s = source.unwrap_or("").to_ascii_lowercase();
    if o.contains("vscode") || s.contains("vscode") {
        Host::VsCode
    } else if o.contains("desktop") {
        Host::DesktopApp
    } else if o == "codex_exec" || s == "exec" {
        Host::Headless
    } else if o == "codex_cli_rs" || s == "cli" {
        Host::Terminal
    } else {
        Host::Unknown
    }
}

fn safe_name(name: Option<&str>) -> String {
    name.unwrap_or("unknown")
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect()
}

fn host_to_str(h: Host) -> String {
    match serde_json::to_value(h) {
        Ok(Value::String(s)) => s,
        _ => "unknown".into(),
    }
}

fn host_from_str(s: &str) -> Host {
    serde_json::from_value(Value::String(s.into())).unwrap_or(Host::Unknown)
}

fn apply(store: &mut Store, provider: &str, path: &Path, event: Event, appended: bool) {
    let key = format!("{provider}:{}", event.id);
    let s = store.sessions.entry(key.clone()).or_insert_with(|| {
        let mut s = Session::new(key, event.cwd.clone().unwrap_or_default(), event.at);
        s.provider = provider.into();
        s.native_id = event.id.clone();
        s
    });
    // A stale line (e.g. replayed after a truncation reset) may fill a field
    // nothing has supplied yet, but never overwrites one a later line set.
    let stale = event.at < s.last_event_at;
    if let Some(cwd) = event.cwd {
        if !stale || s.cwd.is_empty() {
            s.cwd = cwd;
        }
    }
    if s.host == Host::Unknown {
        if let Some(h) = event.host {
            s.host = h;
        }
    }
    if !stale || s.transcript_path.is_none() {
        s.transcript_path = Some(path.to_string_lossy().into_owned());
    }
    if stale {
        return;
    }
    // Permission hooks carry evidence absent from generic transcript entries.
    // Only a terminal lifecycle record or matching hook may resolve that wait.
    if matches!(s.attention.as_str(), "permission" | "explicit_input")
        && !matches!(
            event.status,
            SessionStatus::Idle | SessionStatus::Ended | SessionStatus::Errored
        )
    {
        return;
    }
    s.status = event.status;
    if s.status != SessionStatus::Blocked {
        store.hook_pending.remove(&s.id);
    }
    s.activity = Some(event.activity);
    s.attention = event.attention.into();
    s.blocked_on = if s.status == SessionStatus::Blocked {
        s.activity.clone()
    } else {
        None
    };
    s.transcript_path = Some(path.to_string_lossy().into_owned());
    s.last_event_at = event.at;
    s.started_at.get_or_insert(event.at);
    let age = (Utc::now() - event.at).num_seconds();
    s.live = appended && (0..=60).contains(&age);
    s.observation = if s.live { "recent" } else { "history_only" }.into();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sanitized_windows_sessions_finish_independently() {
        let mut store = Store::default();
        for (provider, fixture) in [
            (
                "claude_code",
                include_str!("../../tests/fixtures/claude-windows.jsonl"),
            ),
            (
                "codex",
                include_str!("../../tests/fixtures/codex-windows.jsonl"),
            ),
        ] {
            let mut id = None;
            let mut host = None;
            for line in fixture.lines() {
                let v = serde_json::from_str(line).unwrap();
                if let Some(event) = adapt(provider, &v, &mut id, &mut host) {
                    apply(&mut store, provider, Path::new("fixture"), event, false);
                }
            }
        }
        assert_eq!(store.sessions.len(), 2);
        for s in store.sessions.values() {
            assert_eq!(s.status, SessionStatus::Idle);
            assert_eq!(s.observation, "history_only");
            assert_eq!(s.cwd, "C:\\fixture");
        }
    }

    #[test]
    fn questions_unknown_events_children_and_old_events() {
        let ts = Utc::now().to_rfc3339();
        let mut id = Some("s".into());
        let mut host = None;
        let mut store = Store::default();
        let question = json!({"type":"assistant","sessionId":"s","timestamp":ts,"message":{"content":[{"type":"tool_use","name":"AskUserQuestion"}]}});
        apply(
            &mut store,
            "claude_code",
            Path::new("fixture"),
            adapt("claude_code", &question, &mut id, &mut host).unwrap(),
            true,
        );
        assert_eq!(store.sessions["claude_code:s"].attention, "explicit_input");
        let mut child = question.clone();
        child["isSidechain"] = json!(true);
        assert!(adapt("claude_code", &child, &mut id, &mut host).is_none());
        let unknown = json!({"type":"event_msg","timestamp":ts,"payload":{"type":"not_supported"}});
        assert!(adapt("codex", &unknown, &mut id, &mut host).is_none());
        let old = json!({"type":"user","sessionId":"s","timestamp":"2000-01-01T00:00:00Z"});
        apply(
            &mut store,
            "claude_code",
            Path::new("fixture"),
            adapt("claude_code", &old, &mut id, &mut host).unwrap(),
            true,
        );
        assert_eq!(
            store.sessions["claude_code:s"].status,
            SessionStatus::Blocked
        );
        let codex = json!({"type":"response_item","timestamp":ts,"payload":{"type":"function_call","name":"request_user_input"}});
        assert_eq!(
            adapt("codex", &codex, &mut id, &mut host)
                .unwrap()
                .attention,
            "explicit_input"
        );
    }
    #[test]
    fn provider_identity_and_lifecycle() {
        let mut store = Store::default();
        let mut id = Some("same".into());
        let mut host = None;
        let ts = Utc::now().to_rfc3339();
        for (provider, v) in [
            (
                "claude_code",
                json!({"type":"assistant","sessionId":"same","timestamp":ts,"message":{"content":[{"type":"tool_use","name":"Read"}]}}),
            ),
            (
                "codex",
                json!({"type":"event_msg","timestamp":ts,"payload":{"type":"task_started"}}),
            ),
        ] {
            apply(
                &mut store,
                provider,
                Path::new("fixture"),
                adapt(provider, &v, &mut id, &mut host).unwrap(),
                true,
            );
        }
        assert_eq!(store.sessions.len(), 2);
        assert!(store
            .sessions
            .values()
            .all(|s| s.status == SessionStatus::Working && s.attention == "unknown"));
        let complete =
            json!({"type":"event_msg","timestamp":ts,"payload":{"type":"task_complete"}});
        apply(
            &mut store,
            "codex",
            Path::new("fixture"),
            adapt("codex", &complete, &mut id, &mut host).unwrap(),
            true,
        );
        assert_eq!(store.sessions["codex:same"].status, SessionStatus::Idle);
        assert_eq!(
            store.sessions["claude_code:same"].status,
            SessionStatus::Working
        );
    }

    #[test]
    fn codex_host_comes_from_self_reported_originator() {
        // Shapes verified against real ~/.codex/sessions/*.jsonl on this
        // machine: "Codex Desktop" and "codex_vscode" are real originator
        // values; a bare CLI run reports no originator field at all.
        let ts = Utc::now().to_rfc3339();
        let mut store = Store::default();

        let mut id = None;
        let mut host = None;
        let desktop = json!({"type":"session_meta","timestamp":ts,"payload":{"id":"d1","originator":"Codex Desktop","cwd":"C:\\r"}});
        apply(
            &mut store,
            "codex",
            Path::new("f"),
            adapt("codex", &desktop, &mut id, &mut host).unwrap(),
            true,
        );

        let mut id = None;
        let mut host = None;
        let vscode = json!({"type":"session_meta","timestamp":ts,"payload":{"id":"v1","originator":"codex_vscode","source":"vscode","cwd":"C:\\r"}});
        apply(
            &mut store,
            "codex",
            Path::new("f"),
            adapt("codex", &vscode, &mut id, &mut host).unwrap(),
            true,
        );

        let mut id = None;
        let mut host = None;
        let cli = json!({"type":"session_meta","timestamp":ts,"payload":{"id":"c1","cwd":"C:\\r"}});
        apply(
            &mut store,
            "codex",
            Path::new("f"),
            adapt("codex", &cli, &mut id, &mut host).unwrap(),
            true,
        );

        assert_eq!(store.sessions["codex:d1"].host, Host::DesktopApp);
        assert_eq!(store.sessions["codex:v1"].host, Host::VsCode);
        assert_eq!(store.sessions["codex:c1"].host, Host::Unknown);
    }

    #[test]
    fn hosts_are_explicit_and_claude_attachment_metadata_survives() {
        assert_eq!(infer_codex_host(Some("unrecognized"), None), Host::Unknown);
        assert_eq!(infer_codex_host(Some("codex_exec"), None), Host::Headless);
        assert_eq!(
            infer_codex_host(Some("codex_cli_rs"), Some("cli")),
            Host::Terminal
        );
        for (entrypoint, expected) in [
            ("claude-desktop", Host::DesktopApp),
            ("claude-vscode", Host::VsCode),
            ("cli", Host::Terminal),
            ("new-host", Host::Unknown),
        ] {
            let mut id = None;
            let mut host = None;
            let attachment = json!({"type":"attachment","sessionId":"s","entrypoint":entrypoint});
            assert!(adapt("claude_code", &attachment, &mut id, &mut host).is_none());
            let prompt = json!({"type":"user","sessionId":"s","timestamp":Utc::now().to_rfc3339()});
            assert_eq!(
                adapt("claude_code", &prompt, &mut id, &mut host)
                    .unwrap()
                    .host,
                Some(expected)
            );
        }
    }

    #[test]
    fn permission_survives_incidental_passive_activity_but_completion_resolves_it() {
        let mut store = Store::default();
        let now = Utc::now();
        let mut session = Session::new("codex:s".into(), String::new(), now);
        session.attention = "permission".into();
        session.status = SessionStatus::Blocked;
        store.sessions.insert(session.id.clone(), session);
        let mut id = Some("s".into());
        let mut host = None;
        let tool = json!({"type":"response_item","timestamp":(now + chrono::Duration::milliseconds(1)).to_rfc3339(),"payload":{"type":"function_call_output"}});
        apply(
            &mut store,
            "codex",
            Path::new("fixture"),
            adapt("codex", &tool, &mut id, &mut host).unwrap(),
            true,
        );
        assert_eq!(store.sessions["codex:s"].status, SessionStatus::Blocked);
        let done = json!({"type":"event_msg","timestamp":(now + chrono::Duration::milliseconds(2)).to_rfc3339(),"payload":{"type":"task_complete"}});
        apply(
            &mut store,
            "codex",
            Path::new("fixture"),
            adapt("codex", &done, &mut id, &mut host).unwrap(),
            true,
        );
        assert_eq!(store.sessions["codex:s"].status, SessionStatus::Idle);
        assert_eq!(store.sessions["codex:s"].attention, "unknown");
    }

    #[test]
    fn partial_lines_duplicates_truncation_and_staleness() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("aperture-passive-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.jsonl");
        let line = json!({"type":"user","sessionId":"test","timestamp":Utc::now().to_rfc3339(),"message":{"content":"redacted"}}).to_string();
        std::fs::write(&path, &line).unwrap();
        let mut cursor = Cursor::default();
        let mut store = Store::default();
        read_file(&path, "claude_code", &mut cursor, &mut store, true).unwrap();
        assert!(store.sessions.is_empty());
        assert_eq!(cursor.offset, 0);
        writeln!(std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap())
        .unwrap();
        read_file(&path, "claude_code", &mut cursor, &mut store, false).unwrap();
        assert_eq!(store.sessions.len(), 1);
        read_file(&path, "claude_code", &mut cursor, &mut store, false).unwrap();
        assert_eq!(store.sessions.len(), 1);
        store
            .sessions
            .get_mut("claude_code:test")
            .unwrap()
            .last_event_at = Utc::now() - chrono::Duration::seconds(61);
        assert_eq!(store.snapshot().sessions[0].observation, "stale");
        assert!(!store.snapshot().sessions[0].live);
        std::fs::write(&path, b"bad\n").unwrap();
        assert_eq!(
            read_file(&path, "claude_code", &mut cursor, &mut store, false).unwrap(),
            1
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn a_question_behind_another_tool_use_block_still_blocks() {
        let ts = Utc::now().to_rfc3339();
        let mut id = None;
        let mut host = None;
        let mut store = Store::default();
        let parallel = json!({"type":"assistant","sessionId":"p","timestamp":ts,"message":{"content":[
            {"type":"text","text":"redacted"},
            {"type":"tool_use","name":"Read"},
            {"type":"tool_use","name":"AskUserQuestion"}
        ]}});
        apply(
            &mut store,
            "claude_code",
            Path::new("fixture"),
            adapt("claude_code", &parallel, &mut id, &mut host).unwrap(),
            true,
        );
        let s = &store.sessions["claude_code:p"];
        assert_eq!(s.status, SessionStatus::Blocked);
        assert_eq!(s.attention, "explicit_input");
    }

    #[test]
    fn a_replay_after_truncation_does_not_revert_cwd_or_host() {
        let root =
            std::env::temp_dir().join(format!("aperture-passive-replay-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("replay.jsonl");
        let now = Utc::now();
        let line = |at: chrono::DateTime<Utc>, cwd: &str, entrypoint: &str| {
            json!({"type":"user","sessionId":"r","timestamp":at.to_rfc3339(),"cwd":cwd,
                "entrypoint":entrypoint,"message":{"content":"redacted"}})
            .to_string()
        };
        let earlier = line(now - chrono::Duration::seconds(30), "C:\\old", "cli");
        let later = line(now, "C:\\new", "claude-vscode");
        std::fs::write(&path, format!("{later}\n")).unwrap();
        let mut cursor = Cursor::default();
        let mut store = Store::default();
        read_file(&path, "claude_code", &mut cursor, &mut store, false).unwrap();
        assert_eq!(store.sessions["claude_code:r"].cwd, "C:\\new");
        // Shorter rewrite: read_file resets to offset 0 and replays the old line.
        std::fs::write(&path, format!("{earlier}\n")).unwrap();
        read_file(&path, "claude_code", &mut cursor, &mut store, false).unwrap();
        assert_eq!(cursor.offset, earlier.len() as u64 + 1);
        let s = &store.sessions["claude_code:r"];
        assert_eq!(s.cwd, "C:\\new");
        assert_eq!(s.host, Host::VsCode);
        assert_eq!(s.last_event_at, now);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn cursor_export_then_restore_round_trips_and_resumes_tailing() {
        let root = std::env::temp_dir().join(format!("aperture-cursor-rt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.jsonl");
        let line = json!({"type":"user","sessionId":"resume","timestamp":Utc::now().to_rfc3339(),"message":{"content":"hi"}}).to_string();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let mut first = Observer::new(root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        first.poll(&mut store);
        assert_eq!(store.sessions.len(), 1);
        let exported = first.export_cursors();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].provider, "claude_code");
        assert!(exported[0].offset > 0);

        // A fresh Observer, as after a restart, resumes from the exported
        // cursor instead of re-reading from byte zero.
        let mut second = Observer::new(root.clone(), root.join("codex-unused"));
        second.restore_cursors(exported.clone());
        let mut second_store = Store::default();
        second.poll(&mut second_store);
        // No new lines were appended, so re-polling must not re-apply the
        // already-ingested line as a fresh event / duplicate session entry.
        assert_eq!(second_store.sessions.len(), 0);

        let still = second.export_cursors();
        assert_eq!(still[0].offset, exported[0].offset);

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    /// A Claude Code `user` record shaped like the one in #95: three pasted
    /// screenshots inlined as base64 `image` blocks, then the prompt text.
    /// Synthesized, not copied from a real transcript, and well over 1 MiB.
    fn screenshot_record(session: &str) -> String {
        let image = |n: usize| json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"A".repeat(n)}});
        let line = json!({"type":"user","sessionId":session,"timestamp":Utc::now().to_rfc3339(),"message":{"role":"user","content":[image(500_000), image(500_000), image(476_000), {"type":"text","text":"redacted"}]}}).to_string();
        assert!(line.len() > MAX_LINE, "fixture must exceed MAX_LINE");
        serde_json::from_str::<Value>(&line).expect("fixture must be valid JSON");
        line
    }

    #[test]
    fn an_oversized_screenshot_record_is_not_counted_as_malformed() {
        let root = std::env::temp_dir().join(format!("aperture-oversized-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("shots.jsonl");
        let after = json!({"type":"assistant","sessionId":"shots","timestamp":Utc::now().to_rfc3339(),"message":{"content":[{"type":"tool_use","name":"Read"}]}}).to_string();
        std::fs::write(&path, format!("{}\n{after}\n", screenshot_record("shots"))).unwrap();

        let mut cursor = Cursor::default();
        let mut store = Store::default();
        let malformed = read_file(&path, "claude_code", &mut cursor, &mut store, true).unwrap();
        assert_eq!((malformed, cursor.malformed, cursor.oversized), (0, 0, 1));
        // The oversized line is skipped whole and the next record still lands.
        assert_eq!(cursor.offset, std::fs::metadata(&path).unwrap().len());
        assert_eq!(
            store.sessions["claude_code:shots"].activity.as_deref(),
            Some("Tool: Read")
        );

        // An unparseable line still counts as malformed, beside the oversized one.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut file, b"not json\n").unwrap();
        drop(file);
        read_file(&path, "claude_code", &mut cursor, &mut store, false).unwrap();
        assert_eq!((cursor.malformed, cursor.oversized), (1, 1));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn oversized_records_alone_do_not_degrade_but_stay_in_the_detail() {
        assert_eq!(passive_state(0, 0, 1), "watching");
        assert_eq!(passive_state(0, 1, 1), "degraded");
        assert_eq!(passive_state(1, 0, 1), "degraded");
        assert_eq!(passive_state(0, 0, 0), "no_sessions");
        assert_eq!(
            passive_detail(0, 0, 0),
            "0 file read errors; 0 skipped records."
        );
        assert!(passive_detail(0, 0, 2).contains("2 oversized records over 1 MiB skipped"));
        assert!(passive_detail(0, 0, 1).contains("1 oversized record over 1 MiB skipped"));

        // Through `poll` and a cursor round trip: the oversized count reaches
        // the health detail, separately from skipped records, and survives a
        // restart. `state` through `poll`, with the real hook inbox swapped
        // out, is covered by `malformed_and_oversized_counts_reach_the_polled_detail`.
        let root =
            std::env::temp_dir().join(format!("aperture-oversized-poll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("s.jsonl"),
            format!("{}\n", screenshot_record("s")),
        )
        .unwrap();
        let detail = |observer: &mut Observer| {
            let mut store = Store::default();
            observer.poll(&mut store);
            store.integrations[0].detail.clone()
        };
        let mut first = Observer::new(root.clone(), root.join("codex-unused"));
        let before = detail(&mut first);
        assert!(
            before.contains("0 skipped records. 1 oversized record"),
            "{before}"
        );
        let mut second = Observer::new(root.clone(), root.join("codex-unused"));
        second.restore_cursors(first.export_cursors());
        let restored = detail(&mut second);
        assert!(
            restored.contains("0 skipped records. 1 oversized record"),
            "{restored}"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A scratch directory holding both provider roots, polled with a hook
    /// inbox that does not exist so the real one cannot affect `state` (#45).
    fn health_scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("aperture-health-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn poll_health(root: &Path) -> Vec<IntegrationHealth> {
        let mut observer = Observer::new(root.join("claude"), root.join("codex"))
            .with_hook_inbox(root.join("no-hook-inbox"));
        let mut store = Store::default();
        observer.poll(&mut store);
        store.integrations
    }

    fn write_codex_session(root: &Path) {
        let ts = Utc::now().to_rfc3339();
        let dir = root.join("codex").join("2026").join("10").join("24");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            json!({"type":"session_meta","timestamp":ts,"payload":{"id":"c1","cwd":"C:\\r"}}),
            json!({"type":"event_msg","timestamp":ts,"payload":{"type":"task_started"}}),
        ];
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(dir.join("rollout-c1.jsonl"), body).unwrap();
    }

    #[test]
    fn a_missing_or_unreadable_root_degrades_only_its_own_provider() {
        let root = health_scratch("missing");
        write_codex_session(&root);
        // Claude's root does not exist at all.
        let health = poll_health(&root);
        let (claude, codex) = (&health[0], &health[1]);
        assert_eq!(claude.provider, "claude_code");
        assert_eq!(claude.state, "degraded");
        assert_eq!(claude.files, 0);
        assert!(
            claude.detail.contains("1 file read errors"),
            "{}",
            claude.detail
        );
        assert_eq!(codex.provider, "codex");
        assert_eq!(codex.state, "watching");
        assert_eq!(codex.files, 1);
        assert!(codex.last_event_at.is_some());
        assert!(
            codex
                .detail
                .contains("0 file read errors; 0 skipped records."),
            "{}",
            codex.detail
        );

        // A root path that is a file, not a directory, cannot be listed either.
        std::fs::write(root.join("claude"), b"not a directory").unwrap();
        let health = poll_health(&root);
        assert_eq!(
            (health[0].state.as_str(), health[1].state.as_str()),
            ("degraded", "watching")
        );
        assert!(
            health[0].detail.contains("1 file read errors"),
            "{}",
            health[0].detail
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_root_with_no_session_files_reports_no_sessions() {
        let root = health_scratch("empty");
        std::fs::create_dir_all(root.join("claude").join("C--empty-project")).unwrap();
        std::fs::write(root.join("claude").join("notes.txt"), b"not a transcript").unwrap();
        std::fs::create_dir_all(root.join("codex")).unwrap();
        let health = poll_health(&root);
        for h in &health {
            assert_eq!(h.state, "no_sessions", "{}: {}", h.provider, h.detail);
            assert_eq!(h.files, 0);
            assert_eq!(h.last_event_at, None);
            assert!(
                h.detail.contains("0 file read errors; 0 skipped records."),
                "{}",
                h.detail
            );
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn malformed_and_oversized_counts_reach_the_polled_detail() {
        let root = health_scratch("records");
        write_codex_session(&root);
        let project = root.join("claude").join("C--p");
        std::fs::create_dir_all(&project).unwrap();
        // Only an oversized record: reported, but not degraded (#95).
        std::fs::write(
            project.join("big.jsonl"),
            format!("{}\n", screenshot_record("big")),
        )
        .unwrap();
        let health = poll_health(&root);
        assert_eq!(health[0].state, "watching");
        assert!(
            health[0]
                .detail
                .contains("0 skipped records. 1 oversized record"),
            "{}",
            health[0].detail
        );

        // Two unparseable lines in another file degrade the row and are counted.
        let ts = Utc::now().to_rfc3339();
        let ok = json!({"type":"assistant","sessionId":"bad","timestamp":ts,"message":{"content":[{"type":"text","text":"hi"}]}});
        std::fs::write(
            project.join("bad.jsonl"),
            format!("not json\n{ok}\n{{\"truncated\n"),
        )
        .unwrap();
        let health = poll_health(&root);
        assert_eq!(health[0].state, "degraded");
        assert!(
            health[0]
                .detail
                .contains("0 file read errors; 2 skipped records. 1 oversized record"),
            "{}",
            health[0].detail
        );
        assert_eq!(health[0].files, 2);
        // The other provider's row is unaffected.
        assert_eq!(health[1].state, "watching");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn watch_roots_returns_configured_provider_directories() {
        let claude = std::env::temp_dir().join("aperture-watch-claude");
        let codex = std::env::temp_dir().join("aperture-watch-codex");
        let observer = Observer::new(claude.clone(), codex.clone());
        let roots = observer.watch_roots();
        assert_eq!(roots, vec![claude, codex]);
    }
}
