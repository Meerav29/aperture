//! The session model. This is the contract with the frontend: every field
//! here is serialized as-is to `src/features/sessions/types.ts`. Keep them in
//! sync by hand for now; generate with `ts-rs` once the shape settles.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Available evidence does not establish a lifecycle state.
    Unknown,
    /// Waiting for the human to type something.
    Idle,
    /// Observed generating or running tools; not proof the process is alive.
    Working,
    /// A permission or question request was observed; resolution is unknown.
    Blocked,
    /// The last turn ended with an API error (rate limit, overload, billing).
    Errored,
    /// SessionEnd fired.
    Ended,
}

/// Host inferred from provider transcript metadata. Unknown values remain
/// unknown; this label does not establish a live window or navigation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Terminal,
    VsCode,
    DesktopApp,
    Headless,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub provider: String,
    pub native_id: String,
    pub attention: String,
    pub observation: String,
    pub status: SessionStatus,
    pub host: Host,
    /// Working directory as reported by the provider. For worktree sessions
    /// this is the worktree path, not the main checkout.
    pub cwd: String,
    /// Repository root, if `cwd` is inside a git checkout or worktree.
    pub repo_root: Option<String>,
    pub git_branch: Option<String>,
    pub transcript_path: Option<String>,
    /// Legacy hook PID. Active adapters do not establish process liveness.
    pub pid: Option<u32>,
    pub title: Option<String>,
    /// Human-readable "what is it doing right now", e.g. "Edit src/app.ts".
    pub activity: Option<String>,
    /// Short attention description/tool name; optional hooks exclude raw input.
    pub blocked_on: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub last_event_at: DateTime<Utc>,
    pub user_messages: u32,
    pub assistant_messages: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Recent observation flag, not a process-liveness guarantee.
    pub live: bool,
}

impl Session {
    pub fn new(id: String, cwd: String, now: DateTime<Utc>) -> Self {
        Session {
            native_id: id.clone(),
            id,
            provider: "claude_code".into(),
            attention: "unknown".into(),
            observation: "history_only".into(),
            status: SessionStatus::Unknown,
            host: Host::Unknown,
            cwd,
            repo_root: None,
            git_branch: None,
            transcript_path: None,
            pid: None,
            title: None,
            activity: None,
            blocked_on: None,
            started_at: None,
            last_event_at: now,
            user_messages: 0,
            assistant_messages: 0,
            input_tokens: 0,
            output_tokens: 0,
            live: false,
        }
    }
}

/// Full snapshot pushed to the window after every change. Simple and
/// sufficient for a spike; switch to deltas if you ever have hundreds of
/// sessions.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub integrations: Vec<IntegrationHealth>,
    pub sessions: Vec<Session>,
    pub hooks_installed: bool,
    pub listener_port: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationHealth {
    pub provider: String,
    pub state: String,
    pub root: String,
    pub files: usize,
    pub last_event_at: Option<DateTime<Utc>>,
    pub detail: String,
}
