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
    let _ = db.save_summaries(&sessions);
    let _ = db.save_cursors(&cursors);
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
        let persisted = db.load_summaries().unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].id, "claude_code:r1");
        let cursors = db.load_cursors().unwrap();
        assert_eq!(cursors.len(), 1);
        assert!(cursors[0].offset > 0);

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
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
