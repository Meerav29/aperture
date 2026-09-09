//! Optional, fail-open observation bridge. Provider settings are never written.
//! Only allowlisted metadata reaches the Aperture-owned spool; prompts and tool
//! arguments are discarded. Each complete event is published by atomic rename.
use super::{
    model::{Session, SessionStatus},
    state::Store,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_INPUT: u64 = 1024 * 1024;
#[derive(Debug, Serialize, Deserialize)]
pub struct Event {
    version: u8,
    provider: String,
    session_id: String,
    event: String,
    at: DateTime<Utc>,
    tool: Option<String>,
    #[serde(default)]
    tool_use_id: Option<String>,
}

pub fn inbox() -> Option<PathBuf> {
    std::env::var_os("APERTURE_HOOK_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|p| p.join("Aperture").join("hook-events")))
}

fn events(provider: &str) -> io::Result<Vec<&'static str>> {
    let mut events = vec![
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "Stop",
        "SessionEnd",
    ];
    match provider {
        "claude_code" => events.extend(["PostToolUseFailure", "StopFailure"]),
        "codex" => events.push("Interrupt"),
        _ => return Err(io::Error::other("expected claude_code or codex")),
    }
    Ok(events)
}

/// Print only a snippet; a human merges these handlers into existing settings.
/// Reject shell metacharacters rather than embedding an unsafe executable path.
pub fn config(provider: &str, executable: &Path) -> io::Result<Value> {
    let path = executable
        .to_str()
        .ok_or_else(|| io::Error::other("non-UTF8 executable path"))?;
    if path
        .chars()
        .any(|c| c.is_control() || "\"'`$%&|<>^!".contains(c))
    {
        return Err(io::Error::other(
            "executable path contains shell metacharacters; move the helper to a simple path",
        ));
    }
    let command = format!("\"{}\" collect {provider}", path.replace('\\', "/"));
    let mut hooks = serde_json::Map::new();
    for event in events(provider)? {
        hooks.insert(
            event.into(),
            json!([{"hooks":[{"type":"command","command":command,"timeout":1}]}]),
        );
    }
    Ok(json!({"hooks": hooks}))
}

pub fn collect(provider: &str, input: impl Read, root: &Path) -> io::Result<()> {
    let supported = events(provider)?;
    let mut bytes = Vec::new();
    input.take(MAX_INPUT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(io::Error::other("oversized hook input"));
    }
    let v: Value = serde_json::from_slice(&bytes)?;
    // Providers may use the parent session ID for child hooks. Never let a child
    // finish or block its parent's lifecycle.
    if v.get("agent_id").is_some_and(|v| !v.is_null())
        || v.get("agent_transcript_path").is_some_and(|v| !v.is_null())
        || v["isSidechain"].as_bool() == Some(true)
        || v["transcript_path"]
            .as_str()
            .is_some_and(|p| p.replace('\\', "/").contains("/subagents/"))
    {
        return Ok(());
    }
    let id = v["session_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("invalid session ID"))?;
    let event = v["hook_event_name"]
        .as_str()
        .filter(|s| supported.contains(s))
        .ok_or_else(|| io::Error::other("unsupported hook event"))?;
    let record = Event {
        version: 1,
        provider: provider.into(),
        session_id: id.into(),
        event: event.into(),
        at: Utc::now(),
        tool: v["tool_name"]
            .as_str()
            .map(|s| s.chars().filter(|c| !c.is_control()).take(80).collect()),
        tool_use_id: v["tool_use_id"]
            .as_str()
            .filter(|s| valid_id(s))
            .map(str::to_owned),
    };
    std::fs::create_dir_all(root)?;
    // Soft cap (concurrent helpers can briefly exceed it). Fail open when the
    // app has been closed long enough to fill its spool; never fill the disk.
    if std::fs::read_dir(root)?.take(4096).count() >= 4096 {
        return Err(io::Error::other("hook spool full"));
    }
    // No provider-controlled text is used in paths; PID plus timestamp is unique
    // across independently launched helper processes.
    let name = format!(
        "aperture-v1-{}-{}",
        record.at.timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let temp = root.join(format!("{name}.tmp"));
    let ready = root.join(format!("{name}.json"));
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(&record)?)?;
    drop(file);
    std::fs::rename(temp, ready)
}

/// Drain only Aperture's ready spool records. A missing inbox means not enabled.
/// Return per-provider observations, plus read errors, for integration health.
pub fn poll(root: &Path, store: &mut Store) -> usize {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return 0,
        Err(_) => return 1,
    };
    let mut errors = 0;
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry)
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                    && entry.file_name().to_str().is_some_and(owned_name) =>
            {
                paths.push(entry.path())
            }
            Err(_) => errors += 1,
            _ => {}
        }
    }
    paths.sort();
    for path in paths.into_iter().take(512) {
        let read = std::fs::File::open(&path).and_then(|f| {
            let mut bytes = Vec::new();
            f.take(4097).read_to_end(&mut bytes)?;
            if bytes.len() > 4096 {
                return Err(io::Error::other("oversized spool record"));
            }
            serde_json::from_slice::<Event>(&bytes).map_err(io::Error::other)
        });
        match read {
            Ok(event)
                if event.version == 1
                    && valid_id(&event.session_id)
                    && (0..86400).contains(&(Utc::now() - event.at).num_seconds()) =>
            {
                apply(store, event)
            }
            Ok(_) => errors += 1,
            Err(_) => errors += 1,
        }
        if std::fs::remove_file(&path).is_err() {
            errors += 1;
        }
    }
    errors
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

fn owned_name(name: &str) -> bool {
    name.strip_prefix("aperture-v1-")
        .and_then(|s| s.strip_suffix(".json"))
        .is_some_and(|s| {
            s.split_once('-').is_some_and(|(at, pid)| {
                !at.is_empty()
                    && !pid.is_empty()
                    && at.bytes().all(|c| c.is_ascii_digit())
                    && pid.bytes().all(|c| c.is_ascii_digit())
            })
        })
}

fn apply(store: &mut Store, e: Event) {
    if e.version != 1
        || !valid_id(&e.session_id)
        || e.at > Utc::now()
        || !events(&e.provider).is_ok_and(|v| v.contains(&e.event.as_str()))
    {
        return;
    }
    let key = format!("{}:{}", e.provider, e.session_id);
    store
        .hook_activity
        .entry(e.provider.clone())
        .and_modify(|at| *at = (*at).max(e.at))
        .or_insert(e.at);
    let s = store.sessions.entry(key.clone()).or_insert_with(|| {
        let mut s = Session::new(key.clone(), String::new(), e.at);
        s.provider = e.provider;
        s.native_id = e.session_id;
        s
    });
    if e.at < s.last_event_at {
        return;
    }
    s.last_event_at = e.at;
    let age = (Utc::now() - e.at).num_seconds();
    s.live = (0..=60).contains(&age);
    s.observation = if s.live { "recent" } else { "history_only" }.into();
    if matches!(e.event.as_str(), "PostToolUse" | "PostToolUseFailure")
        && s.status == SessionStatus::Blocked
    {
        let Some(id) = e.tool_use_id.as_ref() else {
            return;
        };
        let Some(pending) = store.hook_pending.get_mut(&key) else {
            return;
        };
        if !pending.remove(id) || !pending.is_empty() {
            return;
        }
    }
    let is_question = e.tool.as_deref().is_some_and(|t| {
        matches!(
            t,
            "AskUserQuestion"
                | "request_user_input"
                | "functions.request_user_input"
                | "functions.request_user_input_async"
        )
    });
    match e.event.as_str() {
        "PermissionRequest" => {
            store
                .hook_pending
                .entry(key)
                .or_default()
                .insert(e.tool_use_id.clone().unwrap_or_default());
            s.status = SessionStatus::Blocked;
            s.attention = "permission".into();
            s.blocked_on = e.tool.clone();
            s.activity = Some("Permission requested; confirm in original app".into());
        }
        "PreToolUse" if is_question => {
            store
                .hook_pending
                .entry(key)
                .or_default()
                .insert(e.tool_use_id.clone().unwrap_or_default());
            s.status = SessionStatus::Blocked;
            s.attention = "explicit_input".into();
            s.blocked_on = e.tool;
            s.activity = Some("Question requested".into());
        }
        "PreToolUse" if s.status == SessionStatus::Blocked => {}
        name => {
            store.hook_pending.remove(&key);
            s.attention = "unknown".into();
            s.blocked_on = None;
            let (status, activity) = match name {
                "Stop" => (SessionStatus::Idle, "Turn complete"),
                "Interrupt" => (SessionStatus::Idle, "Turn interrupted"),
                "SessionEnd" => (SessionStatus::Ended, "Session ended"),
                "SessionStart" => (SessionStatus::Unknown, "Session started"),
                "StopFailure" => (SessionStatus::Errored, "Turn failed"),
                "PostToolUseFailure" => {
                    (SessionStatus::Working, "Tool failed; session may continue")
                }
                "PostToolUse" => (SessionStatus::Working, "Tool returned"),
                _ => (SessionStatus::Working, "Working"),
            };
            s.status = status;
            if status == SessionStatus::Errored {
                s.attention = "error".into();
            }
            s.activity = Some(activity.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(provider: &str, name: &str, tool: &str) -> Event {
        Event {
            version: 1,
            provider: provider.into(),
            session_id: "same".into(),
            event: name.into(),
            at: Utc::now(),
            tool: Some(tool.into()),
            tool_use_id: Some(tool.into()),
        }
    }
    #[test]
    fn attention_is_provider_qualified_and_unrelated_tool_does_not_clear_it() {
        let mut store = Store::default();
        for provider in ["claude_code", "codex"] {
            apply(&mut store, event(provider, "PermissionRequest", "Bash"));
        }
        apply(&mut store, event("codex", "PostToolUse", "Read"));
        assert_eq!(store.sessions["codex:same"].attention, "permission");
        apply(&mut store, event("codex", "PostToolUse", "Bash"));
        assert_eq!(store.sessions["codex:same"].status, SessionStatus::Working);
        assert_eq!(
            store.sessions["claude_code:same"].status,
            SessionStatus::Blocked
        );
        apply(&mut store, event("claude_code", "Stop", ""));
        assert_eq!(store.sessions["claude_code:same"].blocked_on, None);
    }
    #[test]
    fn collector_redacts_payload_and_ignores_children() {
        let root = std::env::temp_dir().join(format!("aperture-hook-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let payload = json!({"session_id":"same","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"SECRET"},"prompt":"SECRET","cwd":"SECRET"});
        collect("codex", payload.to_string().as_bytes(), &root).unwrap();
        let path = std::fs::read_dir(&root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(!std::fs::read_to_string(path).unwrap().contains("SECRET"));
        let mut child = payload;
        child["agent_id"] = json!("child");
        collect("claude_code", child.to_string().as_bytes(), &root).unwrap();
        let mut store = Store::default();
        assert_eq!(poll(&root, &mut store), 0);
        assert_eq!(store.sessions.len(), 1);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn config_is_metadata_only_and_rejects_unsafe_paths() {
        let value = config(
            "codex",
            Path::new("C:/Program Files/Aperture/aperture-hook.exe"),
        )
        .unwrap();
        assert!(value["hooks"].get("PermissionRequest").is_some());
        assert!(value["hooks"].get("Notification").is_none());
        assert!(config("codex", Path::new("C:/bad$path/helper.exe")).is_err());
        assert!(config("unknown", Path::new("/helper")).is_err());
    }

    #[test]
    fn missing_ids_and_same_name_concurrent_results_do_not_resolve_permission() {
        let mut store = Store::default();
        let mut permission = event("codex", "PermissionRequest", "Bash");
        permission.tool_use_id = None;
        apply(&mut store, permission);
        apply(&mut store, event("codex", "PostToolUse", "Bash"));
        assert_eq!(store.sessions["codex:same"].status, SessionStatus::Blocked);
        let permission = event("codex", "PermissionRequest", "Bash");
        apply(&mut store, permission);
        let mut unrelated = event("codex", "PostToolUse", "Bash");
        unrelated.tool_use_id = Some("different-call".into());
        apply(&mut store, unrelated);
        assert_eq!(store.sessions["codex:same"].status, SessionStatus::Blocked);
    }

    #[test]
    fn concurrent_permissions_require_all_ids_and_errors_need_attention() {
        let mut store = Store::default();
        apply(&mut store, event("codex", "PermissionRequest", "A"));
        apply(&mut store, event("codex", "PermissionRequest", "B"));
        apply(&mut store, event("codex", "PostToolUse", "B"));
        assert_eq!(store.sessions["codex:same"].attention, "permission");
        apply(&mut store, event("codex", "PostToolUse", "A"));
        assert_eq!(store.sessions["codex:same"].status, SessionStatus::Working);
        apply(&mut store, event("claude_code", "StopFailure", ""));
        assert_eq!(store.sessions["claude_code:same"].attention, "error");
    }

    #[test]
    fn invalid_future_and_oversize_spool_preserve_unowned_files() {
        let root =
            std::env::temp_dir().join(format!("aperture-hook-negative-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("unrelated.json"), b"preserve").unwrap();
        let mut future = event("codex", "PermissionRequest", "Bash");
        future.at = Utc::now() + chrono::Duration::hours(1);
        std::fs::write(
            root.join("aperture-v1-1-1.json"),
            serde_json::to_vec(&future).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("aperture-v1-2-1.json"), b"not json").unwrap();
        std::fs::write(root.join("aperture-v1-3-1.json"), vec![b'x'; 4097]).unwrap();
        let mut invalid = event("codex", "PermissionRequest", "Bash");
        invalid.session_id.clear();
        std::fs::write(
            root.join("aperture-v1-4-1.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        let mut store = Store::default();
        assert_eq!(poll(&root, &mut store), 4);
        assert!(store.sessions.is_empty());
        assert_eq!(
            std::fs::read(root.join("unrelated.json")).unwrap(),
            b"preserve"
        );
        assert!(collect(
            "codex",
            vec![b'x'; MAX_INPUT as usize + 1].as_slice(),
            &root
        )
        .is_err());
        assert!(collect("codex", b"{}".as_slice(), &root).is_err());
        std::fs::remove_file(root.join("unrelated.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
