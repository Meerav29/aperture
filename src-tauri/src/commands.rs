use crate::observer::{
    db::Db,
    hook_payload::shorten,
    model::{Session, Snapshot},
    passive::Observer,
    state::{Store, LIVE_STORE_IDLE_DAYS},
};
use chrono::{Duration, Utc};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

pub struct Shared {
    pub store: Arc<Mutex<Store>>,
    pub observer: Arc<StdMutex<Observer>>,
    pub db: Arc<Db>,
}
pub const SNAPSHOT_EVENT: &str = "sessions:snapshot";
pub async fn push_snapshot(app: &AppHandle, store: &Arc<Mutex<Store>>) {
    let _ = app.emit(SNAPSHOT_EVENT, store.lock().await.snapshot());
}

/// Poll for new activity, then persist the resulting summaries and cursors.
/// This is the single write path into SQLite — called from `lib.rs`'s
/// reconcile loop and from the manual `rescan_transcripts` command, always
/// from inside a `spawn_blocking` closure holding both locks.
pub fn reconcile_and_persist(observer: &mut Observer, store: &mut Store, db: &Db) {
    observer.poll(store);
    let sessions = store.snapshot().sessions;
    let cursors = observer.export_cursors();
    // Issue #50: `.is_ok()` here collapsed whatever `rusqlite::Error` the
    // write returned into a bare `bool` before `storage_health` could see
    // it, so the health row had one sentence for every cause.
    let write_failures: Vec<WriteFailure> = [
        write_failure(SUMMARY_WRITE, db.save_summaries(&sessions)),
        write_failure(CURSOR_WRITE, db.save_cursors(&cursors)),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Eviction runs *after* the save, not before (issue #17). Backfill
    // discovers old transcripts, so a session can enter the store already
    // past the idle threshold; evicting first would drop it before its
    // summary was ever written and turn "stays in SQLite" into silent loss.
    // Saving first makes the durable row the thing eviction falls back on.
    let evicted = store.evict_idle(Utc::now(), Duration::days(LIVE_STORE_IDLE_DAYS));
    // The live store is not the only per-session map: `Db`'s write-skipping
    // cache holds a serialized copy of every summary it has written, so
    // leaving those entries behind would just move the growth.
    db.forget_cached_summaries(&evicted);

    let evicted_idle = store.evicted_idle;
    let idle_not_restored = store.idle_not_restored;
    store.integrations.push(storage_health(
        db,
        &write_failures,
        evicted_idle,
        idle_not_restored,
    ));
}

/// Which durable write failed, in words the health row can print. Named
/// after the SQLite tables so a reported failure points at a real object in
/// the database rather than at a Rust function name.
const SUMMARY_WRITE: &str = "session summaries";
const CURSOR_WRITE: &str = "file cursors";

/// How many characters of a SQLite error message reach the health row.
/// Bounded because `detail` is rebuilt every reconcile cycle and shipped in
/// every snapshot event, which `docs/specification.md` §3 caps at ten per
/// second. A ceiling, not a budget: SQLite's own messages are short.
const MAX_DB_ERROR_CHARS: usize = 200;

/// One durable write that failed this cycle, carrying the error SQLite
/// actually returned. Issue #50: both save call sites were reduced to
/// `bool` with `.is_ok()`, so by the time `storage_health` ran the one fact
/// worth acting on — locked file? full disk? permissions? missing table? —
/// was already gone.
struct WriteFailure {
    /// Which write failed — `SUMMARY_WRITE` or `CURSOR_WRITE`.
    what: &'static str,
    /// The error, rendered for display by `sanitize_db_error`.
    detail: String,
}

/// `Some(WriteFailure)` when `result` is an error, `None` when it succeeded.
fn write_failure(what: &'static str, result: rusqlite::Result<()>) -> Option<WriteFailure> {
    result
        .err()
        .map(|e| WriteFailure { what, detail: sanitize_db_error(&e) })
}

/// Render a `rusqlite::Error` for the storage health row: first line only,
/// capped at `MAX_DB_ERROR_CHARS` characters on a character boundary.
///
/// That is all "sanitized" means here — the message text is passed through
/// as SQLite produced it. SQLite's diagnostics name schema objects and
/// failure conditions and do not echo bound parameter values, and the one
/// parameter carrying user content (the serialized `Session` blob) is bound,
/// never interpolated; a `CannotOpen`/`IoErr` message can name the database
/// path, which this row already publishes verbatim as `root`. That is an
/// argument from how SQLite reports errors, not a property any test here
/// proves, and it stops holding for a future caller that interpolates user
/// content into SQL. Decision 1 in `docs/autopilot/decisions.md`.
fn sanitize_db_error(e: &rusqlite::Error) -> String {
    shorten(&e.to_string(), MAX_DB_ERROR_CHARS)
}

/// Synthetic health row reporting whether SQLite persistence succeeded on
/// the last write cycle. `Observer::poll` has no reference to `Db` and
/// cannot know this; only this function, which actually calls
/// `db.save_summaries`/`db.save_cursors`, can.
///
/// Distinguishes three states: writes succeeded against a durable,
/// file-backed `Db` (`"ok"`); a durable `Db` had a write fail this cycle
/// (`"degraded"`, naming which write failed and the error SQLite returned —
/// issue #50); or `db` is in-memory-only, e.g. because `lib.rs` fell back to
/// `Db::in_memory()` after `Db::open` failed (`"degraded"`, in-memory
/// detail) — in that last case saves always succeed against the in-memory
/// connection, so an empty `write_failures` alone can't tell this case apart
/// from real persistence.
///
/// Note: because writes are content-aware, `save_summaries`/`save_cursors`
/// may skip SQLite entirely on a cycle where nothing changed — so `"ok"`
/// here means "no write failed this cycle", not "a write was attempted and
/// succeeded". A database that silently became unwritable while the app was
/// idle (nothing changed) won't be caught until there's actually something
/// new to persist. That is unchanged by issue #50: a richer message for a
/// failure that *was* reported does not make an unattempted write report
/// one.
///
/// It also reports rows the startup `load_summaries` could not deserialize
/// (issue #18). That is a read-side failure, not a write-side one, so it is
/// reported alongside the write state rather than replacing it: a cycle can
/// be writing perfectly well and still be missing history it failed to read.
///
/// `evicted_idle` and `idle_not_restored` are reported for a different reason
/// again: nothing is wrong, but sessions are absent from the live view under
/// issue #17's idle policy, and a card vanishing with no explanation is
/// indistinguishable from the missed session the Phase A dogfood log is
/// watching for. So they are stated, and the state stays `"ok"` — a working
/// policy is not a degradation.
///
/// They are two clauses rather than one sum because they are not the same
/// event: `evicted_idle` counts sessions that were in the live view and left
/// it, `idle_not_restored` counts stored rows that were never admitted at
/// startup. Adding them and calling the total "left the live view" would be
/// wrong about the second group.
fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// `["a"]` -> `"a"`, `["a", "b"]` -> `"a and b"`, `["a", "b", "c"]` ->
/// `"a, b and c"`. Only one or two causes exist today — there are exactly
/// two durable writes — but a list keeps a third from reading badly.
fn join_with_and(parts: &[String]) -> String {
    match parts.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn storage_health(
    db: &Db,
    write_failures: &[WriteFailure],
    evicted_idle: usize,
    idle_not_restored: usize,
) -> crate::observer::model::IntegrationHealth {
    let durable = db.is_durable();
    let unreadable = db.unreadable_summaries();
    let state = if durable && write_failures.is_empty() && unreadable == 0 {
        "ok"
    } else {
        "degraded"
    };
    let mut details = Vec::new();
    if !durable {
        details.push(
            "in-memory only: database unavailable, history will not survive a restart.".to_string(),
        );
    } else if !write_failures.is_empty() {
        // One clause per failed write: the two writes go to different tables
        // and can fail differently in the same cycle, so folding them would
        // have to drop one error — the `.is_ok()` loss, one level up.
        let causes: Vec<String> = write_failures
            .iter()
            .map(|f| format!("{} ({})", f.what, f.detail))
            .collect();
        details.push(format!(
            "SQLite write failed this cycle saving {}; running in-memory only until it \
             recovers.",
            join_with_and(&causes),
        ));
    }
    if unreadable > 0 {
        details.push(format!(
            "{unreadable} stored session {} could not be read at startup and {} missing from \
             history; the {} still in the database.",
            if unreadable == 1 {
                "summary"
            } else {
                "summaries"
            },
            if unreadable == 1 { "is" } else { "are" },
            if unreadable == 1 {
                "row is"
            } else {
                "rows are"
            },
        ));
    }
    if details.is_empty() {
        details.push("SQLite persistence writing normally.".to_string());
    }
    // Two separate counts, deliberately not summed: one names sessions that
    // were in the live view and left it, the other names stored rows that
    // were never admitted at startup. Reporting the total as "left the live
    // view" would be wrong about the second group.
    if evicted_idle > 0 {
        details.push(format!(
            "{evicted_idle} {} no observation in {LIVE_STORE_IDLE_DAYS} days and left the live \
             view; the {} still in the database.",
            plural(evicted_idle, "session had", "sessions had"),
            plural(evicted_idle, "summary is", "summaries are"),
        ));
    }
    if idle_not_restored > 0 {
        details.push(format!(
            "{idle_not_restored} stored {} not shown at startup, after \
             {LIVE_STORE_IDLE_DAYS} days with no observation; still in the database.",
            plural(idle_not_restored, "summary was", "summaries were"),
        ));
    }
    let detail = details.join(" ");
    crate::observer::model::IntegrationHealth {
        provider: "storage".into(),
        state: state.into(),
        root: crate::observer::db::data_dir().to_string_lossy().into_owned(),
        files: 0,
        last_event_at: None,
        detail,
    }
}

#[tauri::command]
pub async fn get_snapshot(shared: State<'_, Shared>) -> Result<Snapshot, String> {
    Ok(shared.store.lock().await.snapshot())
}
#[tauri::command]
pub async fn rescan_transcripts(
    app: AppHandle,
    shared: State<'_, Shared>,
) -> Result<usize, String> {
    let observer = shared.observer.clone();
    let store = shared.store.clone();
    let db = shared.db.clone();
    tokio::task::spawn_blocking(move || {
        let mut observer_guard = observer.lock().map_err(|e| e.to_string())?;
        let mut store_guard = store.blocking_lock();
        reconcile_and_persist(&mut observer_guard, &mut store_guard, &db);
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    push_snapshot(&app, &shared.store).await;
    Ok(shared.store.lock().await.snapshot().sessions.len())
}

/// Navigation fallback: reveal a session's working directory in the OS file
/// manager. Never spawns, resumes, or writes to the session; it only opens a
/// directory this app already trusts because it came from our own store, not
/// from an untrusted event field passed in from the frontend.
#[tauri::command]
pub async fn open_session_folder(shared: State<'_, Shared>, id: String) -> Result<(), String> {
    let snapshot = shared.store.lock().await.snapshot();
    let s = find_session(&snapshot.sessions, &id)?;
    if s.cwd.is_empty() {
        return Err("no known working directory for this session".into());
    }
    open::that(&s.cwd).map_err(|e| e.to_string())
}

/// Navigation fallback: reveal the folder containing a session's transcript
/// file. Used when the working directory isn't known but a transcript path
/// (recorded from our own polling, not an untrusted event) is.
#[tauri::command]
pub async fn reveal_transcript(shared: State<'_, Shared>, id: String) -> Result<(), String> {
    let snapshot = shared.store.lock().await.snapshot();
    let s = find_session(&snapshot.sessions, &id)?;
    let dir = transcript_dir(s).ok_or("no transcript path recorded for this session")?;
    open::that(dir).map_err(|e| e.to_string())
}

fn find_session<'a>(sessions: &'a [Session], id: &str) -> Result<&'a Session, String> {
    sessions
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| "unknown session".to_string())
}

fn transcript_dir(s: &Session) -> Option<PathBuf> {
    let path = s.transcript_path.as_deref()?;
    Some(
        Path::new(path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(path)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::passive::Observer;
    use chrono::Utc;

    #[test]
    fn reconcile_and_persist_polls_and_writes_summaries_and_cursors_to_the_db() {
        use crate::observer::db::Db;

        let root = std::env::temp_dir().join(format!("aperture-reconcile-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("s.jsonl");
        let line = serde_json::json!({
            "type":"user","sessionId":"r1","timestamp":Utc::now().to_rfc3339(),
            "message":{"content":"hi"}
        })
        .to_string();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let mut observer = Observer::new(root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        let db = Db::in_memory();

        reconcile_and_persist(&mut observer, &mut store, &db);

        assert_eq!(store.snapshot().sessions.len(), 1);
        let persisted = db.load_summaries().unwrap().sessions;
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].id, "claude_code:r1");
        let cursors = db.load_cursors().unwrap();
        assert_eq!(cursors.len(), 1);
        assert!(cursors[0].offset > 0);

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn reconcile_and_persist_reports_storage_health_ok_when_durable() {
        use crate::observer::db::Db;

        let root = std::env::temp_dir().join(format!("aperture-storage-health-cmd-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();
        // A real, file-backed Db (not Db::in_memory()) is required here:
        // storage_health now reports "ok" only when persistence is both
        // durable and the last write succeeded.
        let db = Db::open(&root.join("health.db")).unwrap();

        reconcile_and_persist(&mut observer, &mut store, &db);

        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "ok");
        assert!(storage.detail.contains("SQLite"));

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reconcile_and_persist_reports_storage_health_degraded_when_in_memory_fallback() {
        use crate::observer::db::Db;

        // Mirrors lib.rs's fallback path: when Db::open fails, the app
        // continues with Db::in_memory() rather than crashing. Writes
        // against that in-memory Db always succeed, so storage_health must
        // rely on Db::is_durable() (not just the save result) to catch this
        // and report "degraded" rather than misleadingly report "ok".
        let root = std::env::temp_dir().join(format!(
            "aperture-storage-health-inmem-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();
        let db = Db::in_memory();

        reconcile_and_persist(&mut observer, &mut store, &db);

        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "degraded");
        assert!(
            storage.detail.contains("in-memory"),
            "expected in-memory detail, got: {}",
            storage.detail
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn storage_health_reports_summary_rows_that_could_not_be_read() {
        use crate::observer::db::Db;

        // Issue #18: a startup load that dropped rows must not leave the
        // health entry reporting a clean "ok". The write path here is
        // perfectly healthy — durable and succeeding — so "ok" is exactly
        // what this row said before, over a history missing a session.
        let root = std::env::temp_dir().join(format!(
            "aperture-storage-health-unreadable-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = Db::open(&root.join("unreadable.db")).unwrap();
        db.insert_raw_summary("claude_code:broken", "{not json at all");
        let load = db.load_summaries().unwrap();
        assert_eq!(load.failed.len(), 1);

        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();
        reconcile_and_persist(&mut observer, &mut store, &db);

        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "degraded");
        assert!(
            storage.detail.contains("could not be read"),
            "expected the unreadable-rows detail, got: {}",
            storage.detail
        );
        assert!(
            storage.detail.contains("still in the database"),
            "the detail must say the rows were kept, not deleted: {}",
            storage.detail
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reconcile_evicts_an_idle_session_from_memory_but_keeps_its_summary_in_sqlite() {
        use crate::observer::db::Db;

        // Issue #17's wiring criterion, at the level the bug actually lived:
        // `Store::remove` existed and the reconcile path never called it, so
        // this session stayed resident for the life of the process. The
        // second half is the one that makes eviction honest — the durable row
        // must still be there, written by this same cycle before the evict.
        let root =
            std::env::temp_dir().join(format!("aperture-evict-reconcile-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Db::open(&root.join("evict.db")).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();

        let idle = Session::new(
            "claude_code:idle".into(),
            "/repo".into(),
            Utc::now() - Duration::days(LIVE_STORE_IDLE_DAYS + 1),
        );
        let fresh = Session::new("codex:fresh".into(), "/repo".into(), Utc::now());
        store.sessions.insert(idle.id.clone(), idle);
        store.sessions.insert(fresh.id.clone(), fresh);

        reconcile_and_persist(&mut observer, &mut store, &db);

        let live: Vec<String> = store
            .snapshot()
            .sessions
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(live, ["codex:fresh"], "the idle session must leave memory");

        let mut persisted: Vec<String> = db
            .load_summaries()
            .unwrap()
            .sessions
            .into_iter()
            .map(|s| s.id)
            .collect();
        persisted.sort();
        assert_eq!(
            persisted,
            ["claude_code:idle", "codex:fresh"],
            "eviction is not deletion: the summary must survive in SQLite"
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn storage_health_reports_sessions_that_left_the_live_view() {
        use crate::observer::db::Db;

        // A card disappearing with no explanation reads like the missed
        // session the dogfood log is watching for. Nothing is wrong here, so
        // the row must stay "ok" while still saying what happened.
        let root =
            std::env::temp_dir().join(format!("aperture-evict-health-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let db = Db::open(&root.join("health.db")).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();
        let idle = Session::new(
            "claude_code:idle".into(),
            "/repo".into(),
            Utc::now() - Duration::days(LIVE_STORE_IDLE_DAYS + 1),
        );
        store.sessions.insert(idle.id.clone(), idle);

        reconcile_and_persist(&mut observer, &mut store, &db);

        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "ok", "a working policy is not a degradation");
        assert!(
            storage.detail.contains("left the live view"),
            "expected the eviction detail, got: {}",
            storage.detail
        );
        assert!(
            storage.detail.contains("still in the database"),
            "the detail must say the summary was kept, not deleted: {}",
            storage.detail
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn health_does_not_report_a_startup_skipped_row_as_having_left_the_live_view() {
        use crate::observer::db::Db;

        // A row `restore_summaries` declined to admit was never in the live
        // view, so folding it into the "left the live view" count would
        // describe it wrongly. Both absences are reported; they are separate
        // sentences because they are separate events.
        let root = std::env::temp_dir().join(format!(
            "aperture-health-startup-skip-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = Db::open(&root.join("skip.db")).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();

        let old = Session::new(
            "claude_code:old".into(),
            "/repo".into(),
            Utc::now() - Duration::days(60),
        );
        store.restore_summaries(vec![old], Utc::now(), Duration::days(LIVE_STORE_IDLE_DAYS));
        assert_eq!(store.idle_not_restored, 1);
        assert_eq!(store.evicted_idle, 0);

        reconcile_and_persist(&mut observer, &mut store, &db);

        let detail = &store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present")
            .detail;
        assert!(
            detail.contains("not shown at startup"),
            "the startup-skipped row must still be reported: {detail}"
        );
        assert!(
            !detail.contains("left the live view"),
            "a row that never entered the live view must not be said to have \
             left it: {detail}"
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A fresh scratch directory, mirroring `db.rs`'s `migration_scratch_dir`.
    fn scratch_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("aperture-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// `scratch_dir` plus one Claude Code transcript under `<root>/claude`,
    /// so the observer exports a cursor for `save_cursors` to write. With no
    /// cursors `save_cursors` returns `Ok` without touching SQLite.
    fn root_with_one_transcript(name: &str) -> PathBuf {
        let root = scratch_dir(name);
        std::fs::create_dir_all(root.join("claude")).unwrap();
        let line = serde_json::json!({
            "type":"user","sessionId":"c1","timestamp":Utc::now().to_rfc3339(),
            "message":{"content":"hi"}
        })
        .to_string();
        std::fs::write(root.join("claude").join("s.jsonl"), format!("{line}\n")).unwrap();
        root
    }

    /// A store holding one session, so `save_summaries` has something to
    /// write. With an empty store `changed` is empty and `save_summaries`
    /// returns `Ok` without touching SQLite — a write-failure test that
    /// forgot this would pass against the broken code too.
    fn store_with_one_session() -> Store {
        let mut store = Store::default();
        let s = Session::new("claude_code:a".into(), "/repo".into(), Utc::now());
        store.sessions.insert(s.id.clone(), s);
        store
    }

    fn storage_detail(store: &Store) -> &str {
        &store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present")
            .detail
    }

    #[test]
    fn a_failed_summary_write_reports_the_error_sqlite_returned() {
        use crate::observer::db::Db;

        // Issue #50: the row said only "SQLite write failed this cycle" for
        // every cause. A real read-only connection produces a real
        // `SQLITE_READONLY`, and its text must reach the row.
        let root = scratch_dir("health-readonly");
        let path = root.join("readonly.db");
        drop(Db::open(&path).unwrap());

        let db = Db::open_read_only(&path).unwrap();
        assert!(db.is_durable(), "this must be the failed-write case, not the in-memory one");
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = store_with_one_session();

        reconcile_and_persist(&mut observer, &mut store, &db);

        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "degraded");
        assert!(
            storage.detail.contains("readonly"),
            "the detail must carry the error SQLite returned, not just the \
             generic sentence: {}",
            storage.detail
        );
        assert!(
            storage.detail.contains("session summaries"),
            "the detail must name which write failed: {}",
            storage.detail
        );
        assert!(
            !storage.detail.contains("in-memory only: database unavailable"),
            "a durable database whose write failed is not the in-memory \
             fallback case: {}",
            storage.detail
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn two_different_write_failures_produce_two_different_messages() {
        use crate::observer::db::Db;

        // The criterion issue #50 turns on. Both of these rendered the same
        // fixed string before, so the row could not tell a database you
        // cannot write from one whose table is gone.
        let root = scratch_dir("health-two-errors");
        let readonly_path = root.join("readonly.db");
        drop(Db::open(&readonly_path).unwrap());
        let readonly = Db::open_read_only(&readonly_path).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut readonly_store = store_with_one_session();
        reconcile_and_persist(&mut observer, &mut readonly_store, &readonly);
        let readonly_detail = storage_detail(&readonly_store).to_string();

        let missing_path = root.join("missing-table.db");
        let missing = Db::open(&missing_path).unwrap();
        missing.drop_table_for_test("session_summaries");
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut missing_store = store_with_one_session();
        reconcile_and_persist(&mut observer, &mut missing_store, &missing);
        let missing_detail = storage_detail(&missing_store).to_string();

        assert!(
            readonly_detail.contains("readonly"),
            "expected the read-only error: {readonly_detail}"
        );
        assert!(
            missing_detail.contains("no such table"),
            "expected the missing-table error: {missing_detail}"
        );
        assert_ne!(
            readonly_detail, missing_detail,
            "two different failures must not render as one generic sentence"
        );

        drop(readonly);
        drop(missing);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failed_cursor_write_is_reported_as_the_cursor_write() {
        use crate::observer::db::Db;

        // Proves the two call sites are plumbed separately: the summary write
        // succeeds here and only `save_cursors` fails.
        let root = root_with_one_transcript("health-cursor-fail");
        let db = Db::open(&root.join("cursor-fail.db")).unwrap();
        db.drop_table_for_test("file_cursors");
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();

        reconcile_and_persist(&mut observer, &mut store, &db);

        let detail = storage_detail(&store).to_string();
        assert!(
            detail.contains("file cursors") && detail.contains("no such table"),
            "the failing write must be named as the cursor write, with its \
             error: {detail}"
        );
        assert!(
            !detail.contains("session summaries"),
            "the summary write succeeded this cycle and must not be reported \
             as failing: {detail}"
        );
        // The summary write really did succeed, so this is the mixed case and
        // not two failures reported as one.
        assert_eq!(db.load_summaries().unwrap().sessions.len(), 1);

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn both_writes_failing_reports_both_errors_not_one() {
        use crate::observer::db::Db;

        // The case a single shared sentence would lose: one error picked,
        // the other dropped — the `.is_ok()` problem one level up.
        let root = root_with_one_transcript("health-both-fail");
        let path = root.join("both-fail.db");
        drop(Db::open(&path).unwrap());
        let db = Db::open_read_only(&path).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();

        reconcile_and_persist(&mut observer, &mut store, &db);

        let detail = storage_detail(&store).to_string();
        assert!(
            detail.contains("session summaries") && detail.contains("file cursors"),
            "both failed writes must be named: {detail}"
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_long_or_multiline_sqlite_error_is_bounded_to_one_line() {
        // The `sanitize_db_error` contract, tested directly. No SQLite error
        // seen in practice is this shape, which is why a test pins it.
        let long = "x".repeat(MAX_DB_ERROR_CHARS * 3);
        let e = rusqlite::Error::InvalidParameterName(format!("line one {long}\nline two"));
        let rendered = sanitize_db_error(&e);

        assert!(
            !rendered.contains('\n'),
            "a multi-line error must not break the single-line health row: {rendered}"
        );
        assert!(
            !rendered.contains("line two"),
            "only the first line is kept: {rendered}"
        );
        assert_eq!(
            rendered.chars().count(),
            MAX_DB_ERROR_CHARS,
            "the rendered error must be capped at MAX_DB_ERROR_CHARS characters"
        );
    }

    #[test]
    fn reconcile_and_persist_writes_nothing_new_when_nothing_changed() {
        use crate::observer::db::Db;

        let root =
            std::env::temp_dir().join(format!("aperture-noop-reconcile-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("s.jsonl");
        let line = serde_json::json!({
            "type":"user","sessionId":"noop","timestamp":Utc::now().to_rfc3339(),
            "message":{"content":"hi"}
        })
        .to_string();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let mut observer = Observer::new(root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        let db = Db::open(&root.join("noop.db")).unwrap();

        let cursor_path = path.to_string_lossy().into_owned();

        reconcile_and_persist(&mut observer, &mut store, &db);
        assert_eq!(db.load_summaries().unwrap().sessions.len(), 1);
        let updated_at_after_first = db.summary_updated_at("claude_code:noop").unwrap();
        let cursor_updated_at_after_first = db.cursor_updated_at(&cursor_path).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        reconcile_and_persist(&mut observer, &mut store, &db);
        let updated_at_after_second = db.summary_updated_at("claude_code:noop").unwrap();
        let cursor_updated_at_after_second = db.cursor_updated_at(&cursor_path).unwrap();

        assert_eq!(
            updated_at_after_first, updated_at_after_second,
            "a second reconcile with no underlying file change must not rewrite the summary row"
        );
        assert_eq!(
            cursor_updated_at_after_first, cursor_updated_at_after_second,
            "a second reconcile with no underlying file change must not rewrite the cursor row"
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_session_looks_up_by_id_not_native_id() {
        let mut s = Session::new("claude_code:abc".into(), "/repo".into(), Utc::now());
        s.native_id = "abc".into();
        let sessions = vec![s];
        assert!(find_session(&sessions, "claude_code:abc").is_ok());
        assert!(find_session(&sessions, "abc").is_err());
    }

    #[test]
    fn transcript_dir_is_the_parent_of_the_file() {
        let mut s = Session::new("id".into(), "/repo".into(), Utc::now());
        s.transcript_path = Some("/home/user/.claude/projects/p/sess.jsonl".into());
        assert_eq!(
            transcript_dir(&s).unwrap(),
            PathBuf::from("/home/user/.claude/projects/p")
        );
    }

    #[test]
    fn transcript_dir_is_none_when_unrecorded() {
        let s = Session::new("id".into(), "/repo".into(), Utc::now());
        assert_eq!(transcript_dir(&s), None);
    }
}
