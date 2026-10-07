mod commands;
pub mod observer;

use chrono::{DateTime, Utc};
use commands::{push_snapshot, reconcile_and_persist, Shared};
use observer::{db::Db, passive::Observer, state::Store, watch};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};
use tokio::time::MissedTickBehavior;

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

/// The reconcile loop's baseline timer.
///
/// `MissedTickBehavior::Delay`, not tokio's `Burst` default. `Burst` replays
/// every deadline that passed while the loop was not polling — one immediate
/// tick each — so a process suspended for ten minutes resumes with 120
/// back-to-back reconciles, each a transcript re-scan, a SQLite write and a
/// `push_snapshot` event, before the 5-second cadence returns (issue #43).
/// `docs/specification.md` §3 "IPC and UI updates" caps snapshots at ten a
/// second, and a laptop lid-reopen is exactly the case the wake-gap detection
/// above exists to handle.
///
/// `Delay` collapses that backlog into a single catch-up tick and then
/// restarts the period from the moment it fired, so the interval never puts
/// two ticks less than `RECONCILE_INTERVAL_SECS` apart. That also covers a
/// reconcile pass that itself overruns the interval: under `Burst` the next
/// tick fires the instant a slow pass returns, under `Delay` it waits a full
/// period. `Skip` was the other candidate — see `docs/autopilot/decisions.md`.
fn reconcile_interval() -> tokio::time::Interval {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

/// Waits until the next reconcile should run, waking on either a
/// filesystem-watcher signal or the baseline interval tick.
///
/// This is the loop's wait step, lifted into a function so the tick schedule
/// is reachable from a test — the loop itself lives inside
/// `tauri::Builder::setup`, which no test can enter. A burst of debounced
/// watcher signals collapses into one wake, spaced by
/// `watcher_reconcile_delay`. `Interval::tick` is cancel-safe, so losing the
/// `select!` to the watcher branch does not consume a tick.
async fn wait_for_reconcile(
    interval: &mut tokio::time::Interval,
    watch_rx: &mut mpsc::UnboundedReceiver<()>,
    last_reconcile_instant: std::time::Instant,
) {
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

                let mut interval = reconcile_interval();
                let mut last_poll_at = Utc::now();
                let mut last_reconcile_instant = std::time::Instant::now();
                // Anchored to the startup prune in `run()` above, so the
                // first in-loop pass is one interval after launch rather
                // than immediately.
                let mut last_prune_at = Utc::now();

                loop {
                    wait_for_reconcile(&mut interval, &mut watch_rx, last_reconcile_instant).await;

                    let now = Utc::now();
                    if is_wake_gap(last_poll_at, now) {
                        // Describes only what happens next, which is the same
                        // incremental, cursor-offset reconcile every other
                        // iteration runs (issue #54). The previous wording,
                        // "forcing full reconciliation", named a recovery path
                        // that does not exist in this loop, so anyone chasing a
                        // post-sleep data gap could read the line and wrongly
                        // rule the gap out.
                        eprintln!(
                            "Aperture: {}s since the last reconcile; the process was \
                             probably suspended. Running the normal incremental \
                             reconcile, not a full rescan",
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

    /// The baseline reconcile period, as `std::time::Duration`.
    fn period() -> std::time::Duration {
        std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS)
    }

    /// Drives `wait_for_reconcile` once against a live-but-silent watcher
    /// channel, so only the interval branch of its `select!` can fire, and
    /// returns how much simulated time the wait consumed.
    ///
    /// Under the paused clock, tokio auto-advances to the next timer deadline
    /// once every task is idle, so a `ZERO` return means the tick was already
    /// pending — the loop body would have run again with no delay at all.
    async fn time_to_next_reconcile(
        interval: &mut tokio::time::Interval,
        watch_rx: &mut mpsc::UnboundedReceiver<()>,
    ) -> std::time::Duration {
        let before = tokio::time::Instant::now();
        wait_for_reconcile(interval, watch_rx, std::time::Instant::now()).await;
        tokio::time::Instant::now() - before
    }

    #[tokio::test(start_paused = true)]
    async fn a_suspend_gap_yields_one_catch_up_reconcile_not_a_storm() {
        // Issue #43's criterion. Ten minutes of lost wall-clock time is 120
        // missed 5-second deadlines. Under tokio's `Burst` default every one
        // of them fires back-to-back, each driving a transcript re-scan, a
        // SQLite write and a `push_snapshot` event. Under `Delay` the backlog
        // collapses into one catch-up tick.
        //
        // `_tx` is held, not dropped: a closed channel makes `recv()` resolve
        // immediately and the watcher branch would win every `select!`.
        let (_tx, mut watch_rx) = mpsc::unbounded_channel::<()>();
        let mut interval = reconcile_interval();

        // The loop's first iteration: `interval.tick()` always yields at once.
        assert_eq!(
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
            std::time::Duration::ZERO,
            "the first tick should be immediate"
        );

        // The suspend. Not a multiple of the period, so a `Skip` policy would
        // put the tick after the catch-up less than a full period away.
        tokio::time::advance(std::time::Duration::from_millis(600_001)).await;

        let gaps = [
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
        ];

        assert_eq!(
            gaps[0],
            std::time::Duration::ZERO,
            "the wake should produce one immediate catch-up reconcile"
        );
        assert_eq!(
            &gaps[1..],
            &[period(), period()],
            "after the catch-up tick the cadence should return to {}s, one \
             reconcile at a time; got {gaps:?}",
            RECONCILE_INTERVAL_SECS
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconcile_that_overruns_the_interval_does_not_double_tick() {
        // The same policy, on the path that does not involve sleep at all: a
        // single slow pass (a large transcript re-scan, a slow disk) outlasts
        // its own deadline. `Burst` fires the next tick the instant the pass
        // returns, so a machine that is merely slow reconciles twice with no
        // gap. `Delay` waits a full period from the catch-up tick.
        let (_tx, mut watch_rx) = mpsc::unbounded_channel::<()>();
        let mut interval = reconcile_interval();
        time_to_next_reconcile(&mut interval, &mut watch_rx).await;

        // One pass taking three periods, with nothing polling the interval.
        tokio::time::advance(period() * 3).await;

        assert_eq!(
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
            std::time::Duration::ZERO,
            "the overrun pass should be followed by one immediate tick"
        );
        assert_eq!(
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
            period(),
            "the two missed deadlines should not fire as extra reconciles"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_steady_state_cadence_is_one_reconcile_per_interval() {
        // A guard, not a proof of this change: with no missed deadline,
        // `Burst` and `Delay` behave identically, so this test passes with or
        // without `set_missed_tick_behavior`. It is here so a later change to
        // `reconcile_interval` cannot quietly alter the baseline 5-second
        // cadence that `docs/specification.md` §"Freshness and recovery"
        // specifies.
        let (_tx, mut watch_rx) = mpsc::unbounded_channel::<()>();
        let mut interval = reconcile_interval();
        time_to_next_reconcile(&mut interval, &mut watch_rx).await;

        for _ in 0..5 {
            assert_eq!(
                time_to_next_reconcile(&mut interval, &mut watch_rx).await,
                period()
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_watcher_signal_does_not_consume_an_interval_tick() {
        // `wait_for_reconcile`'s `select!` drops the losing branch's future.
        // `Interval::tick` is documented as cancel-safe, and this pins it:
        // after a watcher-driven wake the baseline tick is still due on its
        // own schedule rather than having been swallowed. Also passes before
        // this change — the extraction into `wait_for_reconcile` is what it
        // guards.
        let (tx, mut watch_rx) = mpsc::unbounded_channel::<()>();
        let mut interval = reconcile_interval();
        time_to_next_reconcile(&mut interval, &mut watch_rx).await;

        tokio::time::advance(period() / 2).await;
        tx.send(()).expect("watcher signal");
        // `last_reconcile_instant` is now, so `watcher_reconcile_delay` holds
        // this wake for the full `MIN_WATCHER_RECONCILE_SPACING`.
        let waited = time_to_next_reconcile(&mut interval, &mut watch_rx).await;
        assert_eq!(waited, MIN_WATCHER_RECONCILE_SPACING, "watcher spacing");

        // Half a period had already elapsed before the signal, and the
        // spacing wait covered another second, so the baseline tick is due
        // `period() - period()/2 - MIN_WATCHER_RECONCILE_SPACING` from here.
        assert_eq!(
            time_to_next_reconcile(&mut interval, &mut watch_rx).await,
            period() - period() / 2 - MIN_WATCHER_RECONCILE_SPACING
        );
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
