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
    id: Option<String>,
    host: Option<Host>,
    created: Option<std::time::SystemTime>,
}

pub struct Observer {
    roots: Vec<(String, PathBuf)>,
    cursors: HashMap<PathBuf, Cursor>,
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
        }
    }

    pub fn poll(&mut self, store: &mut Store) {
        let hook_errors = super::hook_bridge::inbox()
            .map(|root| super::hook_bridge::poll(&root, store))
            .unwrap_or(0);
        let mut health = Vec::new();
        for (provider, root) in &self.roots {
            let mut files = Vec::new();
            let mut errors = 0;
            discover(root, &mut files, &mut errors, 0);
            files.sort();
            let mut malformed = 0;
            for path in &files {
                let initial = !self.cursors.contains_key(path);
                let cursor = self.cursors.entry(path.clone()).or_default();
                match read_file(path, provider, cursor, store, initial) {
                    Ok(_) => malformed += cursor.malformed,
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
                state: if errors > 0 || malformed > 0 {
                    "degraded"
                } else if files.is_empty() {
                    "no_sessions"
                } else {
                    "watching"
                }
                .into(),
                detail: format!("{errors} file read errors; {malformed} skipped records."),
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
    }
    if initial {
        cursor.initial_len = meta.len();
    }
    cursor.created = meta.created().ok();
    file.seek(SeekFrom::Start(cursor.offset))?;
    let mut reader = BufReader::new(file);
    let mut malformed = 0;
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
            malformed += 1;
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
                let tool = v
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().find(|b| b["type"] == "tool_use"));
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
    if let Some(cwd) = event.cwd {
        s.cwd = cwd;
    }
    if s.host == Host::Unknown {
        if let Some(h) = event.host {
            s.host = h;
        }
    }
    s.transcript_path = Some(path.to_string_lossy().into_owned());
    if event.at < s.last_event_at {
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

    #[test]
    fn watch_roots_returns_configured_provider_directories() {
        let claude = std::env::temp_dir().join("aperture-watch-claude");
        let codex = std::env::temp_dir().join("aperture-watch-codex");
        let observer = Observer::new(claude.clone(), codex.clone());
        let roots = observer.watch_roots();
        assert_eq!(roots, vec![claude, codex]);
    }
}
