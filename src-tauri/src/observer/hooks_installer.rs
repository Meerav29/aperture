//! Installs and removes our hooks in `~/.claude/settings.json`.
//!
//! Rules:
//! - Never touch keys other than `hooks`.
//! - Inside `hooks`, never touch entries whose command doesn't contain the
//!   `MARKER` string. Other tools' hooks are left alone.
//! - Write a timestamped backup before every change.
//! - Idempotent: installing twice yields one entry per event.
//!
//! The hook command is a script we write to `~/.claude/hooks/`. It reads the
//! payload from stdin, adds `aperture_pid` (claude's PID, the hook's parent) and
//! posts it to the local listener. It always exits 0, so a closed app never
//! affects a session.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

pub const MARKER: &str = "aperture";

/// Events we subscribe to. Adding one here is the only change needed.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

pub fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// Absolute path of the script we install. Tilde is not expanded in
/// settings.json, so this must be absolute.
pub fn script_path() -> Option<PathBuf> {
    let name = if cfg!(windows) { "aperture.cmd" } else { "aperture.sh" };
    dirs::home_dir().map(|h| h.join(".claude").join("hooks").join(name))
}

/// The scripts live as real files under `src-tauri/hooks/` so they can be
/// run and tested by hand; they're embedded at compile time and the port
/// placeholder is filled in at install.
const SCRIPT_SH: &str = include_str!("../../hooks/aperture.sh");
const SCRIPT_CMD: &str = include_str!("../../hooks/aperture.cmd");

pub fn script_body(port: u16) -> String {
    let src = if cfg!(windows) { SCRIPT_CMD } else { SCRIPT_SH };
    src.replace("__PORT__", &port.to_string())
}

/// The command stored in settings.json: just the script's absolute path.
pub fn hook_command() -> String {
    let p = script_path().expect("home dir");
    format!("\"{}\"", p.to_string_lossy())
}

pub fn write_script(port: u16) -> std::io::Result<()> {
    let p = script_path().ok_or_else(|| std::io::Error::other("no home dir"))?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, script_body(port))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn hook_entry() -> Value {
    json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": hook_command(),
            "timeout": 3
        }]
    })
}

fn is_ours(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hs| {
            hs.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .map(|c| c.to_ascii_lowercase().contains(MARKER))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn read_settings(path: &PathBuf) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| Value::Object(Map::new()))
}

fn write_settings(path: &PathBuf, v: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let backup = path.with_extension(format!("json.bak.{MARKER}.{stamp}"));
        std::fs::copy(path, backup)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(v)?)
}

/// Pure transform so it can be unit tested without touching disk.
pub fn with_hooks_installed(mut settings: Value) -> Value {
    let obj = settings.as_object_mut().expect("settings root is an object");
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().expect("hooks is an object");
    for ev in EVENTS {
        let list = hooks
            .entry(*ev)
            .or_insert_with(|| Value::Array(vec![]));
        let arr = list.as_array_mut().expect("hook list is an array");
        arr.retain(|e| !is_ours(e));
        arr.push(hook_entry());
    }
    settings
}

pub fn with_hooks_removed(mut settings: Value) -> Value {
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        for (_, list) in hooks.iter_mut() {
            if let Some(arr) = list.as_array_mut() {
                arr.retain(|e| !is_ours(e));
            }
        }
        hooks.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
    }
    settings
}

pub fn is_installed(settings: &Value) -> bool {
    settings
        .pointer("/hooks/SessionStart")
        .and_then(Value::as_array)
        .map(|a| a.iter().any(is_ours))
        .unwrap_or(false)
}

pub fn install(port: u16) -> std::io::Result<()> {
    let path = settings_path().ok_or_else(|| std::io::Error::other("no home dir"))?;
    write_script(port)?;
    let current = read_settings(&path);
    write_settings(&path, &with_hooks_installed(current))
}

pub fn uninstall() -> std::io::Result<()> {
    let path = settings_path().ok_or_else(|| std::io::Error::other("no home dir"))?;
    if !path.exists() {
        return Ok(());
    }
    let current = read_settings(&path);
    write_settings(&path, &with_hooks_removed(current))?;
    if let Some(p) = script_path() {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

pub fn check_installed() -> bool {
    settings_path()
        .map(|p| is_installed(&read_settings(&p)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_preserves_other_hooks() {
        let existing = json!({
            "model": "opus",
            "hooks": {
                "Stop": [{"matcher": "", "hooks": [{"type":"command","command":"say done"}]}]
            }
        });
        let once = with_hooks_installed(existing);
        let twice = with_hooks_installed(once.clone());
        assert_eq!(once, twice);
        assert_eq!(twice["model"], "opus");
        let stop = twice["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "theirs + ours");
        assert!(is_installed(&twice));

        let removed = with_hooks_removed(twice);
        assert!(!is_installed(&removed));
        assert_eq!(removed["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(removed["hooks"].get("SessionStart").is_none());
    }
}
