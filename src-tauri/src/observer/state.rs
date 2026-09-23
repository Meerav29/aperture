//! In-memory session store and the state machine that folds events into it.
//!
//! The desktop is fed by passive provider adapters and an optional metadata
//! inbox. The apply_hook/apply_transcript reducers below are legacy paths;
//! the active optional hook reducer lives in hook_bridge.rs.
//!
//! Active merging preserves unresolved attention against incidental transcript
//! activity. Legacy backfill keeps the historical hook/count merge behavior.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

use super::hook_payload::HookPayload;
use super::model::{Host, Session, SessionStatus, Snapshot};
use super::transcript::TranscriptSummary;

/// A session with no observation for this long stops being tracked in the
/// live in-memory store (issue #17, the rule proposed in `docs/roadmap.md`
/// Phase A item 1).
///
/// This is an eviction threshold, not a deletion one. The session's SQLite
/// summary row is left exactly where it is, still subject to the separate
/// 90-day `Db::prune_summaries` retention, so nothing observed inside the
/// retention window is lost — it stops being carried in memory and in the
/// dashboard's live view.
///
/// Two neighbouring numbers this deliberately is not: the 60-second
/// staleness downgrade in `Store::snapshot` (a freshness label, not a
/// lifetime) and the 7-day normalized-activity retention in
/// `docs/specification.md` (raw activity records, not session summaries).
/// 14 days is far enough past the 60-second liveness window that eviction
/// can never touch a session anything currently claims is live.
pub const LIVE_STORE_IDLE_DAYS: i64 = 14;

/// Whether `s` has gone unobserved for longer than `max_idle` as of `now`.
///
/// `last_event_at` is the only timestamp that means "we saw something",
/// which is what the policy is about: a session whose transcript is still
/// being appended to keeps a fresh `last_event_at` through
/// `passive::apply` and can never age out while it is in use.
fn idle_past(s: &Session, now: DateTime<Utc>, max_idle: Duration) -> bool {
    now - s.last_event_at > max_idle
}

#[derive(Default)]
pub struct Store {
    pub(crate) hook_activity: HashMap<String, chrono::DateTime<Utc>>,
    pub(crate) hook_pending: HashMap<String, std::collections::HashSet<String>>,
    pub revision: u64,
    pub integrations: Vec<super::model::IntegrationHealth>,
    pub(crate) sessions: HashMap<String, Session>,
    pub hooks_installed: bool,
    pub listener_port: u16,
    /// How many sessions have left (or never entered) the live store under
    /// the idle policy since this process started, counting both
    /// `evict_idle` and rows `restore_summaries` declined to admit.
    ///
    /// A session dropping off the dashboard with no explanation reads
    /// exactly like the missed session the Phase A dogfood log exists to
    /// catch, so `commands::storage_health` reports this count and says the
    /// summaries were kept. It counts events, not distinct sessions: one
    /// that ages out, is resumed, and ages out again counts twice.
    pub evicted_idle: usize,
}

impl Store {
    pub fn snapshot(&self) -> Snapshot {
        let mut sessions: Vec<Session> = self.sessions.values().cloned().collect();
        for s in &mut sessions {
            if s.observation == "recent" && (Utc::now() - s.last_event_at).num_seconds() > 60 {
                s.observation = "stale".into();
                s.live = false;
            }
        }
        // Most recently observed activity first; status breaks timestamp ties.
        sessions.sort_by(|a, b| {
            b.last_event_at
                .cmp(&a.last_event_at)
                .then(rank(a.status).cmp(&rank(b.status)))
        });
        Snapshot {
            revision: self.revision,
            integrations: self.integrations.clone(),
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
        if let Some(pid) = p.aperture_pid {
            s.pid = Some(pid);
        }
        if s.host == Host::Unknown {
            s.host = infer_host(p.aperture_host_hint.as_deref());
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
        self.revision += 1;
    }

    /// Drop every session with no observation in `max_idle`, returning the
    /// ids removed. This is the caller `Store::remove` never had: issue #17
    /// is that the method exists and nothing in the reconcile path calls it,
    /// so `Store.sessions` grows for the life of the process. Wired into
    /// `commands::reconcile_and_persist`, after the summaries are written.
    ///
    /// Eviction is not deletion. The SQLite summary row survives — that is
    /// the whole point of running this after the save — and a session whose
    /// transcript is appended to again is re-admitted by the next
    /// `Observer::poll` with a fresh `last_event_at`.
    pub fn evict_idle(&mut self, now: DateTime<Utc>, max_idle: Duration) -> Vec<String> {
        let idle: Vec<String> = self
            .sessions
            .values()
            .filter(|s| idle_past(s, now, max_idle))
            .map(|s| s.id.clone())
            .collect();
        for id in &idle {
            self.remove(id);
        }
        self.evicted_idle += idle.len();
        idle
    }

    /// Seed the store from persisted history on startup. Never trust a
    /// persisted `live` claim: force it false and downgrade a "recent"
    /// observation to "stale" so nothing is presented as currently live
    /// before the first post-restart reconcile.
    ///
    /// Rows already past the idle threshold are counted and skipped rather
    /// than admitted. `prune_summaries` keeps 90 days of history but the
    /// live store keeps `max_idle`, so without this the store would open at
    /// the size of the whole retention window and then shed most of it
    /// seconds later on the first reconcile — a visible flicker, and a
    /// startup working set proportional to total historical session count,
    /// which is the growth issue #17 is about.
    pub fn restore_summaries(
        &mut self,
        sessions: Vec<Session>,
        now: DateTime<Utc>,
        max_idle: Duration,
    ) {
        for mut s in sessions {
            if idle_past(&s, now, max_idle) {
                self.evicted_idle += 1;
                continue;
            }
            s.live = false;
            if s.observation == "recent" {
                s.observation = "stale".into();
            }
            self.sessions.insert(s.id.clone(), s);
        }
        self.revision += 1;
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
        let raw =
            format!(r#"{{"session_id":"{id}","hook_event_name":"{name}","cwd":"/repo" {extra}}}"#);
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

    #[test]
    fn restore_forces_live_false_and_downgrades_recent_to_stale() {
        let mut st = Store::default();
        let mut recent = Session::new("claude_code:a".into(), "/repo".into(), Utc::now());
        recent.live = true;
        recent.observation = "recent".into();
        let mut history = Session::new("codex:b".into(), "/repo".into(), Utc::now());
        history.live = true; // a persisted bug/edge case; must still be forced false
        history.observation = "history_only".into();

        st.restore_summaries(vec![recent, history], Utc::now(), max_idle());

        let a = &st.sessions["claude_code:a"];
        assert!(!a.live);
        assert_eq!(a.observation, "stale");
        let b = &st.sessions["codex:b"];
        assert!(!b.live);
        assert_eq!(b.observation, "history_only");
    }

    fn max_idle() -> Duration {
        Duration::days(LIVE_STORE_IDLE_DAYS)
    }

    fn session_last_seen(id: &str, at: DateTime<Utc>) -> Session {
        Session::new(id.into(), "/repo".into(), at)
    }

    #[test]
    fn evict_idle_drops_sessions_past_the_cutoff_and_keeps_the_rest() {
        let now = Utc::now();
        let mut st = Store::default();
        for (id, idle_for) in [
            ("claude_code:ancient", Duration::days(90)),
            ("claude_code:just_past", max_idle() + Duration::minutes(1)),
            ("claude_code:just_inside", max_idle() - Duration::minutes(1)),
            ("codex:fresh", Duration::zero()),
        ] {
            let s = session_last_seen(id, now - idle_for);
            st.sessions.insert(s.id.clone(), s);
        }

        let mut evicted = st.evict_idle(now, max_idle());
        evicted.sort();

        assert_eq!(evicted, ["claude_code:ancient", "claude_code:just_past"]);
        let mut kept: Vec<&str> = st.sessions.keys().map(String::as_str).collect();
        kept.sort();
        assert_eq!(kept, ["claude_code:just_inside", "codex:fresh"]);
        assert_eq!(st.evicted_idle, 2, "the count the health row reports");
    }

    #[test]
    fn a_session_observed_now_is_never_evicted() {
        // The safety property the threshold rests on: `snapshot` only calls a
        // session live within 60 seconds of `last_event_at`, so a cutoff of
        // days cannot remove anything the dashboard claims is live. Asserted
        // rather than assumed, because shrinking the constant to something
        // near the liveness window would silently break it.
        let now = Utc::now();
        let mut st = Store::default();
        let mut live = session_last_seen("claude_code:live", now);
        live.live = true;
        live.observation = "recent".into();
        st.sessions.insert(live.id.clone(), live);

        assert!(st.evict_idle(now, max_idle()).is_empty());
        assert!(st.snapshot().sessions[0].live);
    }

    #[test]
    fn the_live_store_stays_bounded_across_many_reconcile_cycles() {
        // Issue #17's regression test: one new session per simulated
        // reconcile, a day apart, for far longer than the idle window. Before
        // eviction was wired in this map only ever grew — it would end at
        // `CYCLES`, and a real long-running process would do the same thing
        // with every session it had ever observed.
        const CYCLES: i64 = 365;
        let start = Utc::now() - Duration::days(CYCLES);
        let mut st = Store::default();

        for cycle in 0..CYCLES {
            let now = start + Duration::days(cycle);
            let s = session_last_seen(&format!("claude_code:s{cycle}"), now);
            st.sessions.insert(s.id.clone(), s);
            st.evict_idle(now, max_idle());
            assert!(
                st.sessions.len() <= LIVE_STORE_IDLE_DAYS as usize + 1,
                "cycle {cycle}: {} sessions resident",
                st.sessions.len()
            );
        }

        assert_eq!(st.evicted_idle as i64, CYCLES - LIVE_STORE_IDLE_DAYS - 1);
        assert!(
            (st.sessions.len() as i64) < CYCLES,
            "the store must not be proportional to total sessions observed"
        );
    }

    #[test]
    fn restore_admits_only_sessions_inside_the_idle_window() {
        // `prune_summaries` keeps 90 days; the live store keeps 14. Restoring
        // all 90 days and evicting on the first reconcile would reach the
        // same steady state seconds later, but the startup working set — and
        // the first snapshot the window is painted from — would still be
        // proportional to the full history.
        let now = Utc::now();
        let mut st = Store::default();
        let old = session_last_seen("claude_code:old", now - Duration::days(60));
        let recent = session_last_seen("codex:recent", now - Duration::days(1));

        st.restore_summaries(vec![old, recent], now, max_idle());

        assert_eq!(st.sessions.len(), 1);
        assert!(st.sessions.contains_key("codex:recent"));
        assert_eq!(
            st.evicted_idle, 1,
            "the skipped row is reported, not hidden"
        );
    }
}
