mod commands;
pub mod observer;

use chrono::{DateTime, Utc};
use commands::{push_snapshot, reconcile_and_persist, Shared};
use observer::{db::Db, passive::Observer, state::Store, watch};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};

const RECONCILE_INTERVAL_SECS: u64 = 5;
const SLEEP_WAKE_THRESHOLD_SECS: i64 = 90;
const SUMMARY_RETENTION_DAYS: i64 = 90;

/// A wall-clock gap this large between reconciles means the process was
/// almost certainly suspended (OS sleep, laptop lid close) rather than just
/// busy — the loop below wakes on either a watcher signal or a 5s interval,
/// so a real 90s+ gap can only come from lost wall-clock time, not scheduling
/// jitter. No OS-specific power-event API is used; see the design doc's
/// non-goals.
fn is_wake_gap(last: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    (now - last).num_seconds() > SLEEP_WAKE_THRESHOLD_SECS
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = observer::db::data_dir().join("aperture.db");
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Aperture: database at {} unavailable ({e}); running in-memory only this session",
                db_path.display()
            );
            Db::in_memory()
        }
    };
    let _ = db.prune_summaries(SUMMARY_RETENTION_DAYS);

    let mut store = Store::default();
    if let Ok(sessions) = db.load_summaries() {
        store.restore_summaries(sessions);
    }

    let mut observer = Observer::default();
    if let Ok(cursors) = db.load_cursors() {
        observer.restore_cursors(cursors);
    }

    let db = Arc::new(db);
    let store = Arc::new(Mutex::new(store));
    let observer = Arc::new(StdMutex::new(observer));

    tauri::Builder::default()
        .manage(Shared {
            store: store.clone(),
            observer: observer.clone(),
            db: db.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::rescan_transcripts,
            commands::open_session_folder,
            commands::reveal_transcript
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Restored summaries are already stale/history_only and
                // live:false (Store::restore_summaries) — push them now so
                // the UI shows prior context before the first reconcile.
                push_snapshot(&handle, &store).await;

                let (tx, mut watch_rx) = mpsc::unbounded_channel();
                let roots = observer.lock().expect("observer lock").watch_roots();
                let _watcher = watch::watch(&roots, tx);

                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
                let mut last_poll_at = Utc::now();

                loop {
                    tokio::select! {
                        _ = watch_rx.recv() => {}
                        _ = interval.tick() => {}
                    }

                    let now = Utc::now();
                    if is_wake_gap(last_poll_at, now) {
                        eprintln!(
                            "Aperture: {}s since the last reconcile; forcing full reconciliation",
                            (now - last_poll_at).num_seconds()
                        );
                    }

                    let s = store.clone();
                    let o = observer.clone();
                    let d = db.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        let mut observer_guard = o.lock().expect("observer lock");
                        let mut store_guard = s.blocking_lock();
                        reconcile_and_persist(&mut observer_guard, &mut store_guard, &d);
                    })
                    .await
                    {
                        eprintln!("Observer failed: {e}");
                    }
                    last_poll_at = Utc::now();
                    push_snapshot(&handle, &store).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aperture");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn a_short_gap_is_not_a_wake() {
        let last = chrono::Utc::now();
        let now = last + Duration::seconds(10);
        assert!(!is_wake_gap(last, now));
    }

    #[test]
    fn a_gap_past_the_threshold_is_a_wake() {
        let last = chrono::Utc::now();
        let now = last + Duration::seconds(SLEEP_WAKE_THRESHOLD_SECS + 1);
        assert!(is_wake_gap(last, now));
    }
}
