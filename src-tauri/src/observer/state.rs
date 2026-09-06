//! In-memory session store and the state machine that folds events into it.
//!
//! Two event sources feed this: live hook payloads (`apply_hook`) and
//! transcript backfill (`apply_transcript`). Later, the spawner layer's
//! `Runner` becomes a third source, calling the same methods.
//!
//! Merge rule: hooks win for `status`/`activity`, transcripts win for counts
//! and tokens. A backfilled session never overwrites a live one's status.

use std::collections::HashMap;

use chrono::Utc;

use super::hook_payload::HookPayload;
use super::model::{Host, Session, SessionStatus, Snapshot};
use super::transcript::TranscriptSummary;

#[derive(Default)]
pub struct Store {
    sessions: HashMap<String, Session>,
    pub hooks_installed: bool,
    pub listener_port: u16,
}

impl Store {
    pub fn snapshot(&self) -> Snapshot {
        let mut sessions: Vec<Session> = self.sessions.values().cloned().collect();
        // Blocked first, then working, then by recency. The UI groups by repo
        // but relies on this order within a group.
        sessions.sort_by(|a, b| {
            rank(a.status)
                .cmp(&rank(b.status))
                .then(b.last_event_at.cmp(&a.last_event_at))
        });
        Snapshot {
            sessions,
            hooks_installed: self.hooks_installed,
            listener_port: self.listener_port,
        }
    }

    pub fn apply_hook(&mut self, p: HookPayload) {
        let now = Utc::now();
        let cwd = p.cwd.clone().unwrap_or_default();
        let s = self
            .sessions
            .entry(p.session_id.clone())
            .or_insert_with(|| Session::new(p.session_id.clone(), cwd.clone(), now));

        s.live = true;
        s.last_event_at = now;
        if !cwd.is_empty() {
            s.cwd = cwd;
        }
        if s.transcript_path.is_none() {
            s.transcript_path = p.transcript_path.clone();
        }
        if let Some(pid) = p.deck_pid {
            s.pid = Some(pid);
        }
        if s.host == Host::Unknown {
            s.host = infer_host(p.deck_host_hint.as_deref());
        }

        match p.hook_event_name.as_str() {
            "SessionStart" => {
                if s.started_at.is_none() {
                    s.started_at = Some(now);
                }
                s.status = SessionStatus::Idle;
                s.activity = None;
                s.blocked_on = None;
            }
            "UserPromptSubmit" => {
                s.status = SessionStatus::Working;
                s.activity = Some("Thinking".into());
                s.blocked_on = None;
                s.user_messages += 1;
            }
            "PreToolUse" => {
                s.status = SessionStatus::Working;
                s.activity = p.describe_tool();
                s.blocked_on = None;
            }
            "PostToolUse" => {
                s.status = SessionStatus::Working;
                s.activity = Some("Thinking".into());
            }
            "PermissionRequest" => {
                // Newer, more specific event than Notification/permission_prompt.
                s.status = SessionStatus::Blocked;
                s.blocked_on = p.describe_tool().or_else(|| p.message.clone());
            }
            "Notification" => match p.notification_type.as_deref() {
                Some("permission_prompt") => {
                    s.status = SessionStatus::Blocked;
                    s.blocked_on = p.message.clone();
                }
                Some("idle_prompt") => {
                    s.status = SessionStatus::Idle;
                    s.activity = None;
                }
                _ => {}
            },
            "Stop" => {
                s.status = SessionStatus::Idle;
                s.activity = None;
                s.blocked_on = None;
                s.assistant_messages += 1;
            }
            "StopFailure" => {
                s.status = SessionStatus::Errored;
                s.activity = p
                    .error
                    .as_ref()
                    .and_then(|e| e.get("message").and_then(|m| m.as_str()))
                    .map(str::to_string)
                    .or_else(|| p.message.clone());
            }
            "SessionEnd" => {
                s.status = SessionStatus::Ended;
                s.activity = p.reason.clone();
                s.blocked_on = None;
            }
            // SubagentStart/Stop, PreCompact, etc.: keep last_event_at fresh,
            // nothing else for the spike.
            _ => {}
        }
    }

    pub fn apply_transcript(&mut self, t: TranscriptSummary) {
        let s = self
            .sessions
            .entry(t.session_id.clone())
            .or_insert_with(|| Session::new(t.session_id.clone(), t.cwd.clone(), t.last_at));

        // Counts and tokens always come from the transcript, it's the source
        // of truth for history.
        s.user_messages = t.user_messages;
        s.assistant_messages = t.assistant_messages;
        s.input_tokens = t.input_tokens;
        s.output_tokens = t.output_tokens;
        if s.transcript_path.is_none() {
            s.transcript_path = Some(t.path);
        }
        if s.git_branch.is_none() {
            s.git_branch = t.git_branch;
        }
        if s.title.is_none() {
            s.title = t.title;
        }
        if s.started_at.is_none() {
            s.started_at = Some(t.first_at);
        }
        if !s.live {
            s.last_event_at = t.last_at;
            // We can't know whether a non-live session is still open, so
            // leave it Unknown rather than guessing Idle.
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.sessions.remove(id);
    }
}

fn rank(s: SessionStatus) -> u8 {
    match s {
        SessionStatus::Blocked => 0,
        SessionStatus::Errored => 1,
        SessionStatus::Working => 2,
        SessionStatus::Idle => 3,
        SessionStatus::Unknown => 4,
        SessionStatus::Ended => 5,
    }
}

fn infer_host(hint: Option<&str>) -> Host {
    // The hook script passes the grandparent process name when it can get
    // it cheaply. Match loosely; this is a label, not a decision.
    let h = hint.unwrap_or("").to_ascii_lowercase();
    if h.contains("code") && !h.contains("claude") {
        Host::VsCode
    } else if h.contains("claude") {
        Host::DesktopApp
    } else if h.contains("term") || h.contains("zsh") || h.contains("bash") || h.contains("pwsh") {
        Host::Terminal
    } else if h.is_empty() {
        Host::Unknown
    } else {
        Host::Headless
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, name: &str, extra: &str) -> HookPayload {
        let raw = format!(
            r#"{{"session_id":"{id}","hook_event_name":"{name}","cwd":"/repo" {extra}}}"#
        );
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn prompt_then_permission_then_stop() {
        let mut st = Store::default();
        st.apply_hook(ev("s1", "SessionStart", ""));
        assert_eq!(st.snapshot().sessions[0].status, SessionStatus::Idle);

        st.apply_hook(ev("s1", "UserPromptSubmit", ""));
        assert_eq!(st.snapshot().sessions[0].status, SessionStatus::Working);

        st.apply_hook(ev(
            "s1",
            "Notification",
            r#","notification_type":"permission_prompt","message":"Allow Edit?""#,
        ));
        let s = &st.snapshot().sessions[0];
        assert_eq!(s.status, SessionStatus::Blocked);
        assert_eq!(s.blocked_on.as_deref(), Some("Allow Edit?"));

        st.apply_hook(ev("s1", "Stop", ""));
        let s = &st.snapshot().sessions[0];
        assert_eq!(s.status, SessionStatus::Idle);
        assert_eq!(s.assistant_messages, 1);
    }

    #[test]
    fn blocked_sorts_first() {
        let mut st = Store::default();
        st.apply_hook(ev("a", "SessionStart", ""));
        st.apply_hook(ev("b", "SessionStart", ""));
        st.apply_hook(ev("b", "UserPromptSubmit", ""));
        st.apply_hook(ev(
            "b",
            "Notification",
            r#","notification_type":"permission_prompt""#,
        ));
        assert_eq!(st.snapshot().sessions[0].id, "b");
    }
}
