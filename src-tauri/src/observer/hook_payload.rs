//! The JSON Claude Code writes to a hook's stdin. Every event carries the
//! common fields; each event adds its own. We parse the common part strictly
//! and keep the rest as `serde_json::Value` so an unfamiliar field never
//! breaks ingestion.
//!
//! Our hook script (see `hooks_installer.rs`) also injects `aperture_pid`
//! (the hook's `$PPID`, i.e. the claude process) so we can jump to it later.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct HookPayload {
    pub session_id: String,
    pub hook_event_name: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,

    // Event-specific fields we care about. All optional.
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub source: Option<String>, // SessionStart: startup | resume | clear | compact

    // Injected by our hook wrapper, not by Claude Code.
    #[serde(default)]
    pub aperture_pid: Option<u32>,
    #[serde(default)]
    pub aperture_host_hint: Option<String>,

    #[serde(flatten)]
    pub extra: Value,
}

impl HookPayload {
    /// A short, human-readable description of a tool call for the activity
    /// line. "Edit src/app.ts", "Bash npm test", "Read README.md".
    pub fn describe_tool(&self) -> Option<String> {
        let name = self.tool_name.as_deref()?;
        let input = self.tool_input.as_ref();
        let detail = input.and_then(|i| {
            i.get("file_path")
                .or_else(|| i.get("path"))
                .or_else(|| i.get("command"))
                .or_else(|| i.get("pattern"))
                .or_else(|| i.get("description"))
                .and_then(Value::as_str)
        });
        Some(match detail {
            Some(d) => {
                let d = shorten(d, 60);
                format!("{name} {d}")
            }
            None => name.to_string(),
        })
    }
}

fn shorten(s: &str, max: usize) -> String {
    let s = s.lines().next().unwrap_or(s);
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pre_tool_use() {
        let raw = r#"{
          "session_id":"abc","hook_event_name":"PreToolUse",
          "cwd":"/x","transcript_path":"/t.jsonl",
          "tool_name":"Edit","tool_input":{"file_path":"src/app.ts"},
          "some_new_field": 1
        }"#;
        let p: HookPayload = serde_json::from_str(raw).unwrap();
        assert_eq!(p.describe_tool().as_deref(), Some("Edit src/app.ts"));
        assert_eq!(p.extra["some_new_field"], 1);
    }
}
