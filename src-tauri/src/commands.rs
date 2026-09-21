use crate::observer::{
    db::Db,
    model::{Session, Snapshot},
    passive::Observer,
    state::Store,
};
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
    let summaries_ok = db.save_summaries(&sessions).is_ok();
    let cursors_ok = db.save_cursors(&cursors).is_ok();
    store
        .integrations
        .push(storage_health(db, summaries_ok && cursors_ok));
}

/// Synthetic health row reporting whether SQLite persistence succeeded on
/// the last write cycle. `Observer::poll` has no reference to `Db` and
/// cannot know this; only this function, which actually calls
/// `db.save_summaries`/`db.save_cursors`, can.
///
/// Distinguishes three states: writes succeeded against a durable,
/// file-backed `Db` (`"ok"`); a durable `Db` had a write fail this cycle
/// (`"degraded"`, save failure detail); or `db` is in-memory-only, e.g.
/// because `lib.rs` fell back to `Db::in_memory()` after `Db::open` failed
/// (`"degraded"`, in-memory detail) — in that last case saves always
/// succeed against the in-memory connection, so `ok` alone can't tell this
/// case apart from real persistence.
///
/// Note: because writes are content-aware, `save_summaries`/`save_cursors`
/// may skip SQLite entirely on a cycle where nothing changed — so `"ok"`
/// here means "no write failed this cycle", not "a write was attempted and
/// succeeded". A database that silently became unwritable while the app was
/// idle (nothing changed) won't be caught until there's actually something
/// new to persist.
///
/// It also reports rows the startup `load_summaries` could not deserialize
/// (issue #18). That is a read-side failure, not a write-side one, so it is
/// reported alongside the write state rather than replacing it: a cycle can
/// be writing perfectly well and still be missing history it failed to read.
fn storage_health(db: &Db, ok: bool) -> crate::observer::model::IntegrationHealth {
    let durable = db.is_durable();
    let unreadable = db.unreadable_summaries();
    let state = if durable && ok && unreadable == 0 {
        "ok"
    } else {
        "degraded"
    };
    let mut details = Vec::new();
    if !durable {
        details.push(
            "in-memory only: database unavailable, history will not survive a restart.".to_string(),
        );
    } else if !ok {
        details.push(
            "SQLite write failed this cycle; running in-memory only until it recovers.".to_string(),
        );
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
