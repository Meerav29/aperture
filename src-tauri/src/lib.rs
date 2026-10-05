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
/// How long between summary-retention passes inside a running process.
/// Startup pruning alone leaves a long-running process accumulating rows
/// past the retention window until it happens to restart (issue #30), and
/// this tool is meant to be left open for days.
const SUMMARY_PRUNE_INTERVAL_SECS: i64 = 60 * 60;

/// A wall-clock gap this large between reconciles means the process was
/// almost certainly suspended (OS sleep, laptop lid close) rather than just
/// busy — the loop below wakes on either a watcher signal or a 5s interval,
/// so a real 90s+ gap can only come from lost wall-clock time, not scheduling
/// jitter. No OS-specific power-event API is used; see the design doc's
/// non-goals.
fn is_wake_gap(last: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    (now - last).num_seconds() > SLEEP_WAKE_THRESHOLD_SECS
}

const MIN_WATCHER_RECONCILE_SPACING: std::time::Duration = std::time::Duration::from_secs(1);

/// How much longer to wait before a watcher-triggered reconcile, given
/// `elapsed` time since the last reconcile and the minimum allowed
/// spacing. `None` means proceed immediately. This bounds reconcile
/// frequency during a burst of filesystem-watcher activity (e.g. an
/// actively streaming session) without touching the independent 5-second
/// baseline `interval.tick()`, which is unaffected by this and keeps
/// firing on its own schedule regardless.
fn watcher_reconcile_delay(
    elapsed: std::time::Duration,
    min_spacing: std::time::Duration,
) -> Option<std::time::Duration> {
    min_spacing.checked_sub(elapsed).filter(|d| !d.is_zero())
}

/// Whether a summary-retention pass is due.
///
/// Wall-clock rather than monotonic, deliberately: the window retention
/// enforces is wall-clock (90 days of `updated_at`), `Db::prune_summaries`
/// derives its cutoff from `Utc::now()`, and this loop already reasons in
/// wall-clock time — see `is_wake_gap`. A machine suspended overnight should
/// prune on the first reconcile after it wakes, which a clock that stops
/// during suspend would not do.
///
/// `now < last` means wall-clock time moved backwards (an NTP correction, the
/// user changing the system clock). That counts as due. The alternative is
/// retention stalling until wall-clock time catches up, which for a large
/// backwards jump is indefinite; running one extra `DELETE` is the cheaper
/// mistake.
fn summary_prune_due(last: DateTime<Utc>, now: DateTime<Utc>, every_secs: i64) -> bool {
    now < last || (now - last).num_seconds() >= every_secs
}

/// One reconcile tick's worth of summary retention.
///
/// `None` means this tick was skipped and **no SQL was issued at all** — the
/// common case, since reconcile runs every `RECONCILE_INTERVAL_SECS` seconds
/// and retention is hourly. `Some` carries `prune_summaries`' own result, and
/// the caller moves its `last_prune_at` forward on either outcome: a database
/// that fails to prune should be retried on the next retention cycle, not on
/// every tick.
fn prune_summaries_if_due(
    db: &Db,
    last_prune_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<rusqlite::Result<usize>> {
    if !summary_prune_due(last_prune_at, now, SUMMARY_PRUNE_INTERVAL_SECS) {
        return None;
    }
    Some(db.prune_summaries(SUMMARY_RETENTION_DAYS))
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
    // `load_summaries` logs and counts rows it could not read; the count
    // stays on `Db` so `commands::storage_health` keeps reporting it long
    // after this one-shot startup load.
    //
    // Retention keeps 90 days of summaries on disk but the live store keeps
    // `LIVE_STORE_IDLE_DAYS`, so most of that history is deliberately not
    // admitted here (issue #17). `Store` counts what it skipped and the
    // storage health entry reports it.
    if let Ok(load) = db.load_summaries() {
        store.restore_summaries(
            load.sessions,
            Utc::now(),
            chrono::Duration::days(observer::state::LIVE_STORE_IDLE_DAYS),
        );
    }

    let mut observer = Observer::default();
    if let Ok(cursors) = db.load_cursors() {
        let (live_cursors, dead_paths): (Vec<_>, Vec<_>) = cursors
            .into_iter()
            .partition(|c| std::path::Path::new(&c.path).exists());
        if !dead_paths.is_empty() {
            let _ = db.delete_cursors(
                &dead_paths.into_iter().map(|c| c.path).collect::<Vec<_>>(),
            );
        }
        observer.restore_cursors(live_cursors);
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
                // RAII guard: must stay alive for the loop's lifetime, or file-change notifications stop.
                let _watcher = watch::watch(&roots, tx);

                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
                let mut last_poll_at = Utc::now();
                let mut last_reconcile_instant = std::time::Instant::now();
                // Anchored to the startup prune in `run()` above, so the
                // first in-loop pass is one interval after launch rather
                // than immediately.
                let mut last_prune_at = Utc::now();

                loop {
                    tokio::select! {
                        _ = watch_rx.recv() => {
                            // Collapse a burst of debounced signals into one wake.
                            while watch_rx.try_recv().is_ok() {}
                            if let Some(remaining) = watcher_reconcile_delay(
                                last_reconcile_instant.elapsed(),
                                MIN_WATCHER_RECONCILE_SPACING,
                            ) {
                                tokio::time::sleep(remaining).await;
                                while watch_rx.try_recv().is_ok() {}
                            }
                        }
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
                    let prune_from = last_prune_at;
                    match tokio::task::spawn_blocking(move || {
                        {
                            let mut observer_guard = o.lock().expect("observer lock");
                            let mut store_guard = s.blocking_lock();
                            reconcile_and_persist(&mut observer_guard, &mut store_guard, &d);
                        }
                        // Retention rides this blocking task rather than
                        // spawning its own: the gate is hourly, so nearly
                        // every tick returns here without touching SQLite.
                        // The store and observer locks are released above
                        // first — retention needs neither.
                        prune_summaries_if_due(&d, prune_from, now)
                    })
                    .await
                    {
                        Ok(None) => {}
                        Ok(Some(Ok(removed))) => {
                            last_prune_at = now;
                            if removed > 0 {
                                eprintln!(
                                    "Aperture: retention removed {removed} session \
                                     summaries older than {SUMMARY_RETENTION_DAYS} days"
                                );
                            }
                        }
                        Ok(Some(Err(e))) => {
                            last_prune_at = now;
                            eprintln!(
                                "Aperture: summary retention failed ({e}); \
                                 retrying on the next retention cycle"
                            );
                        }
                        Err(e) => eprintln!("Observer failed: {e}"),
                    }
                    last_poll_at = Utc::now();
                    last_reconcile_instant = std::time::Instant::now();
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

    #[test]
    fn no_delay_needed_when_spacing_already_satisfied() {
        assert_eq!(
            watcher_reconcile_delay(
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn no_delay_needed_when_spacing_exactly_satisfied() {
        assert_eq!(
            watcher_reconcile_delay(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn delay_needed_when_within_minimum_spacing() {
        assert_eq!(
            watcher_reconcile_delay(
                std::time::Duration::from_millis(300),
                std::time::Duration::from_secs(1)
            ),
            Some(std::time::Duration::from_millis(700))
        );
    }

    /// One simulated reconcile tick's wall-clock step.
    fn tick() -> Duration {
        Duration::seconds(RECONCILE_INTERVAL_SECS as i64)
    }

    #[test]
    fn retention_is_not_due_before_the_interval_elapses() {
        let last = Utc::now();
        assert!(!summary_prune_due(
            last,
            last + Duration::seconds(SUMMARY_PRUNE_INTERVAL_SECS - 1),
            SUMMARY_PRUNE_INTERVAL_SECS
        ));
    }

    #[test]
    fn retention_is_due_once_the_interval_elapses() {
        let last = Utc::now();
        assert!(summary_prune_due(
            last,
            last + Duration::seconds(SUMMARY_PRUNE_INTERVAL_SECS),
            SUMMARY_PRUNE_INTERVAL_SECS
        ));
    }

    #[test]
    fn a_backwards_clock_jump_does_not_stall_retention() {
        // An NTP correction or a user clock change can move wall-clock time
        // backwards. Waiting for it to catch up would park retention for the
        // size of the jump, which is unbounded.
        let last = Utc::now();
        assert!(summary_prune_due(
            last,
            last - Duration::hours(6),
            SUMMARY_PRUNE_INTERVAL_SECS
        ));
    }

    #[test]
    fn a_long_running_process_prunes_an_expired_summary_without_restarting() {
        // Issue #30's first criterion. The simulated clock drives the *gate*
        // only — `prune_summaries` derives its cutoff from the real
        // `Utc::now()`, so the expired row has to be genuinely older than the
        // window at insert time. What this proves is that the in-loop path
        // reaches it at all; before this change nothing but a restart did.
        let db = Db::in_memory();
        db.insert_aged_summary("claude_code:expired", "{}", SUMMARY_RETENTION_DAYS + 10);
        db.insert_aged_summary("claude_code:recent", "{}", 1);

        let start = Utc::now();
        let mut last_prune_at = start;
        let mut now = start;
        let mut passes = 0usize;
        // Two hours of reconcile ticks, no restart in between.
        for _ in 0..(2 * 3600 / RECONCILE_INTERVAL_SECS) {
            now += tick();
            if let Some(result) = prune_summaries_if_due(&db, last_prune_at, now) {
                result.expect("prune");
                passes += 1;
                last_prune_at = now;
            }
        }

        assert_eq!(passes, 2, "hourly retention across two simulated hours");
        assert!(
            db.summary_updated_at("claude_code:expired").is_none(),
            "a row past the retention window survived a long-running process"
        );
        assert!(
            db.summary_updated_at("claude_code:recent").is_some(),
            "retention removed a row inside the window"
        );
    }

    #[test]
    fn retention_does_not_run_on_every_reconcile_tick() {
        // Issue #30's second criterion: no needless SQL on the hot path.
        // `prune_summaries_if_due` returning `None` is the evidence — it
        // returns before calling into `Db` at all.
        let db = Db::in_memory();
        let start = Utc::now();
        let mut last_prune_at = start;
        let mut now = start;
        let mut ticks = 0usize;
        let mut passes = 0usize;

        while now < start + Duration::hours(1) {
            now += tick();
            ticks += 1;
            if prune_summaries_if_due(&db, last_prune_at, now).is_some() {
                passes += 1;
                last_prune_at = now;
            }
        }

        assert_eq!(ticks, 720, "one hour at a 5-second reconcile cadence");
        assert_eq!(passes, 1, "retention ran {passes} times in one hour");
    }
}
