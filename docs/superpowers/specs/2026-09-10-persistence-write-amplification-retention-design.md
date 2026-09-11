# Reduce persistence write amplification; make retention effective — design

Status: approved design for implementation. Follow-up to
[2026-09-09-durable-observation-recovery-design.md](2026-09-09-durable-observation-recovery-design.md),
addressing two of three findings the final whole-branch review for that
work deferred to a follow-up branch (PR #3): unbounded write amplification
in the reconcile loop, and an inert 90-day summary retention prune. The
third deferred item — `Session`'s lack of serde forward-compatibility and
silently-swallowed summary-load failures — is explicitly out of scope here;
it is unrelated to write volume or retention and was not requested for this
follow-up.

## Problem

`commands::reconcile_and_persist` (`src-tauri/src/commands.rs:26-35`) calls
`Db::save_summaries`/`Db::save_cursors` every reconcile cycle, and both
unconditionally `INSERT ... ON CONFLICT DO UPDATE` **every** row currently
held in memory — not just what changed. Two consequences:

1. **Write amplification.** The reconcile loop
   (`src-tauri/src/lib.rs:82-110`) wakes on either a 5-second interval or a
   debounced filesystem-watcher signal (`observer::watch`, 250ms debounce
   window). Nothing bounds how often the watcher side of that
   `tokio::select!` can fire, and the channel is never drained — during
   active multi-line-per-second transcript writes the watcher can trigger
   several reconciles per second, each rewriting every session's and every
   file's row regardless of whether that particular row changed. Volume is
   proportional to total working-set size × wake frequency, not to what
   actually changed.

2. **Inert retention.** `Db::save_summaries` sets `updated_at =
   Utc::now()` on every write (`src-tauri/src/observer/db.rs:87-100`),
   including no-op rewrites of unchanged rows. Since `reconcile_and_persist`
   re-saves every session in `Store.sessions` on every cycle, and nothing
   currently removes sessions from that in-memory map, every persisted
   summary's `updated_at` is refreshed every few seconds for the lifetime of
   the app. `Db::prune_summaries(90)` (called once at startup,
   `src-tauri/src/lib.rs:37`) can therefore never find a row old enough to
   delete during normal operation — the mechanism exists and is tested in
   isolation, but is unreachable in practice. `file_cursors` has no
   retention at all.

## Goals

- Cut persisted write volume to rows whose content actually changed since
  the last successful write, for both summaries and cursors.
- Bound worst-case reconcile frequency during bursts of filesystem-watcher
  activity, independent of the always-firing 5-second baseline interval.
- Make the existing 90-day summary retention prune actually remove rows for
  sessions that have gone genuinely quiet.
- Add the missing retention prune for `file_cursors`.
- No schema migration: reuse the existing `updated_at` column on both
  tables; change only when it's written and add one new prune call.

## Non-goals

- Evicting long-idle sessions from the in-memory `Store.sessions` map.
  Nothing today ever calls `Store::remove` from the passive-observation
  path, so the in-memory working set (not just the persisted one) also
  grows for the life of the process. Deciding when a session should stop
  being tracked in the live dashboard is a separate, larger product
  decision — not a storage-layer concern — and is not addressed here.
- `Session`'s serde forward-compatibility and `load_summaries`'s silent
  failure-swallowing (the third item the prior final review deferred).
  Unrelated to write volume or retention; left for its own follow-up if and
  when it's requested.
- Any new user-facing configuration for the retention window or the
  reconcile-spacing constant. Both stay fixed constants, consistent with
  the existing `RECONCILE_INTERVAL_SECS`/`SLEEP_WAKE_THRESHOLD_SECS`/
  `SUMMARY_RETENTION_DAYS` pattern already in `lib.rs`.

## Design

### Content-aware writes (the core mechanism — fixes both problems)

`Db` gains an in-memory "last written" cache, guarded by the *same* mutex
as the `Connection` (folding both into one guarded struct) so there is no
new lock-ordering surface and no TOCTOU window between checking the cache
and writing:

```rust
struct ConnState {
    conn: Connection,
    summary_cache: HashMap<String, String>,       // session id -> last-written JSON
    cursor_cache: HashMap<String, CursorRecord>,    // file path -> last-written record
}

pub struct Db {
    state: Mutex<ConnState>,
    path: Option<PathBuf>,
}
```

`save_summaries`: for each `Session`, serialize it (as today) and compare
against `summary_cache.get(&s.id)`. If equal, skip the row entirely — no
SQL statement, no `updated_at` touch. If different (or absent), include it
in the transaction and update the cache entry after a successful commit. If
every row in the batch is unchanged, skip opening a transaction at all and
return `Ok(())` immediately.

`save_cursors`: identical shape, comparing the incoming `CursorRecord`
(already `#[derive(PartialEq)]`) against `cursor_cache.get(&c.path)`.

This is the whole fix for write amplification: volume becomes proportional
to what changed, not to the size of the working set. It is also the whole
fix for retention: because `updated_at` is now only written when content
actually changes, a session that goes quiet stops advancing its
`updated_at` and ages normally toward the retention cutoff — no separate
"is this session still active" signal needs to be invented.

The cache starts empty on every process launch (it is not seeded from
`load_summaries`/`load_cursors` at `Db::open` time), so the first reconcile
after each restart performs a full, unoptimized write regardless of what
actually changed. This is a deliberate simplification: it is a one-time
cost per app launch, not per cycle, and seeding the cache from the loaded
rows would need re-serializing every persisted row at startup for a benefit
that only applies to the very first cycle.

### Coalescing watcher-triggered reconciles

Content-aware writes don't help when a session is genuinely streaming new
lines on every debounce window — that content really is changing every
cycle. For that case, `lib.rs`'s reconcile loop gets a minimum spacing
between *watcher-triggered* reconciles:

```rust
const MIN_WATCHER_RECONCILE_SPACING: Duration = Duration::from_secs(1);
```

On waking from `watch_rx.recv()` (not from the 5s `interval.tick()`, which
is untouched and keeps firing on its own independent schedule as the
existing floor): drain any other signals already queued in the same burst
(`while watch_rx.try_recv().is_ok() {}`), then if less than
`MIN_WATCHER_RECONCILE_SPACING` has elapsed since the last reconcile,
sleep out the remainder and drain once more before proceeding. This caps
watcher-driven reconcile frequency at roughly once per second regardless of
how fast the debouncer fires, while leaving the 5-second baseline interval
as an independent, always-firing safety net exactly as it is today.

### Cursor retention

`Db::prune_cursors(days: i64) -> rusqlite::Result<usize>`, identical shape
to the existing `prune_summaries`, deleting `file_cursors` rows whose
`updated_at` is older than the cutoff. Called once at startup in `lib.rs`
alongside the existing `db.prune_summaries(SUMMARY_RETENTION_DAYS)` call,
reusing the same `SUMMARY_RETENTION_DAYS` constant for both tables (no new
constant — one retention window, applied consistently, matching the
existing single-knob pattern rather than inventing a second one without a
stated need for it to differ).

## Testing

- `Db` gains tests proving: (a) re-saving an unchanged `Session`/
  `CursorRecord` issues no SQL write and leaves `updated_at` untouched
  (verifiable by checking the row's `updated_at` is identical across two
  `save_summaries` calls with identical content, with a real time gap
  between them); (b) saving a genuinely changed row does update both the
  data and `updated_at`; (c) a batch containing a mix of changed and
  unchanged rows only rewrites the changed ones (assert row count changed
  via a `changes()` check or equivalent, not just final content).
- `prune_cursors` gets a test mirroring the existing `prune_removes_only_old_summaries`
  test: an old cursor row (hand-inserted with a stale `updated_at`, same
  pattern as the existing summary prune test) is removed; a fresh one is
  retained.
- An integration-level test (in `commands.rs` or extending the existing
  `reconcile_and_persist_*` tests) proving that calling
  `reconcile_and_persist` twice in a row with no underlying file change
  results in zero additional SQL writes — the most direct proof that the
  write-amplification fix actually holds end-to-end through the real call
  path, not just at the `Db` unit level.
- A `lib.rs`-level unit test for the coalescing logic is impractical to
  write in isolation (it's wired into the async `tokio::select!` loop
  inside `run()`, which is not unit-testable the same way `is_wake_gap`
  was); rely on the existing pattern of extracting any genuinely pure
  sub-logic (e.g. "how long until the next allowed reconcile" as a plain
  function of two instants, if such a function is worth factoring out) and
  test that in isolation, the same way `is_wake_gap` was tested. Manual
  verification (rapid file writes to a watched transcript, observing
  reconcile frequency does not exceed ~1/sec) is the practical check for
  the loop wiring itself.
- Existing tests (`db.rs`, `commands.rs`, `durable_recovery.rs`) must
  continue to pass unchanged — this is a behavior refinement of the write
  path, not a change to the persisted schema or the public `Db`/
  `CursorRecord` shapes consumed elsewhere.

## Files touched

- `src-tauri/src/observer/db.rs` — `ConnState` restructuring, content-aware
  `save_summaries`/`save_cursors`, new `prune_cursors`.
- `src-tauri/src/lib.rs` — reconcile-loop coalescing (minimum watcher
  spacing, signal draining), added `db.prune_cursors(...)` call at startup.
- Test additions alongside both files above.
