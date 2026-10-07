# Decisions made without you — aperture

Every entry here is a design question the requirements and specification did not
settle, which a build routine resolved on its own so the slice could ship.
Appended by the routines; newest last.

Per the design's ambiguity policy, a routine picks the most defensible option,
records it here and in the PR body, and proceeds. It does not stop, and it does
not guess silently.

A routine may **not** use this file to record anything about host compatibility,
runtime behavior, or validation status. Those belong to you and to
`docs/validation/`, which autopilot cannot write.

**Read this before reviewing the `auto/queue` diff.**

Format:

```
## <date> — slice-<n>, PR #<n>
Question:    <what the spec left open>
Chosen:      <what was done>
Rejected:    <the alternative, and why not>
Blast radius: <files or functions affected, and how reversible>
```

---

## 2026-09-21 — slice-1, PR #33
Question:    Issue #18 asks for a ruling it deliberately leaves open: does
             `Session` get `#[serde(default)]` / tolerant deserialization so
             additive shape changes stop orphaning persisted rows, or are
             breaking shape changes accepted as a now-visible migration cost?
Chosen:      Accept breaking shape changes as a now-visible migration cost, and
             add **no** `#[serde(default)]`, because measurement showed the
             tolerant half was already there. The first draft of this slice did
             add `#[serde(default)]` to the eight `Option` fields; running the
             new tests against a reverted tree to check they actually fail
             without the change showed
             `a_row_written_before_an_optional_field_existed_still_loads`
             passing either way. Serde's derive already resolves a missing
             field for `Option<T>` to `None`, so those eight attributes were
             no-ops. They were removed rather than shipped as decoration that
             implies a mechanism it does not provide. The test stayed: it now
             pins serde's behavior so a later change cannot quietly remove it.

             So the policy, written down rather than newly built: an additive
             `Option` field (`SessionKey` in #9, Git identity in #7) already
             costs no stored history, and `None` is an honest "not known". The
             required fields (`id`, `provider`, `native_id`, `attention`,
             `observation`, `status`, `host`, `cwd`, the counters, `live`) stay
             strict, so a breaking change to those is a migration cost
             `load_summaries` now surfaces instead of swallowing. The other
             direction — an older build reading a row a newer build wrote —
             was already tolerant, since serde ignores unknown fields.
Rejected:    (a) Blanket tolerance via container-level `#[serde(default)]`.
             It needs `impl Default for Session`, and it would turn a corrupt
             row into a session with an empty `id`, an empty `attention` and a
             manufactured `status` — a plausible-looking session assembled out
             of unreadable data. That is the same "silent loss presented as a
             clean state" this issue exists to stop, wearing a different hat,
             and it would defeat the detection the rest of the slice adds.
             (b) Keeping the eight no-op `#[serde(default)]` attributes as
             documentation of intent. A reader would reasonably assume they
             were load-bearing, and a future field added as non-`Option` would
             inherit that false assurance. The doc comment on `Session` says
             the same thing without pretending to do work.
Blast radius: `src-tauri/src/observer/model.rs` — comment only; no attribute,
             field, rename or removal, so `Session`'s serialized form and the
             frontend contract in `src/features/sessions/types.ts` are
             untouched. The behavior this entry rules on is serde's, not this
             repo's, so there is nothing here to revert.

## 2026-09-21 — slice-1, PR #33
Question:    Where should a failed row surface? The issue asks for "log output
             at minimum" and `docs/roadmap.md` Phase A item 1 says "surface
             load failures in the storage health entry", but summaries are
             loaded exactly once at startup while that health entry is rebuilt
             on every reconcile cycle, so a per-load return value alone cannot
             reach it.
Chosen:      Both, from one source. `load_summaries` returns `SummaryLoad
             { sessions, failed }` and logs each failure to stderr (capped at
             10 lines plus a total, so a broadly corrupt database cannot bury
             every other startup message), and `Db` keeps the failure count
             from the most recent load so `commands::storage_health` can report
             it every cycle. The health row goes `degraded` with a detail that
             says the rows were kept, not deleted. The count is per-load, so it
             clears once a load succeeds cleanly.
Rejected:    (a) Log only. Satisfies the issue's minimum but not the roadmap's
             "surface in the storage health entry", and it is discoverable only
             by someone watching stderr. (b) Recomputing the count inside
             `storage_health` by re-reading every row each cycle: it would turn
             a startup-only cost into a per-cycle full table scan, against the
             write-amplification work in the 2026-09-10 persistence design.
Blast radius: `src-tauri/src/observer/db.rs`, `src-tauri/src/commands.rs`
             (`storage_health`), `src-tauri/src/lib.rs` (one call site).
             `storage_health`'s existing `"ok"` / in-memory / write-failure
             details are unchanged; the new clause is appended, so a cycle that
             is both write-degraded and missing rows reports both. No schema or
             migration change — `SELECT id, data` reads a column migration 1
             already created.

## 2026-09-23 — slice-2, PR #41
Question:    Issue #17 lists three candidate eviction policies (age-based, a
             bounded LRU-style cap, or hiding from the default view) and does
             not choose. `docs/roadmap.md` Phase A item 1 proposes 14 days as
             a *proposal*, and `docs/specification.md` §"Storage and historical
             ingestion" names two other numbers nearby: seven days for
             normalized activity, 90 days for historical summaries. Which
             number governs the live store, and which shape of policy?
Chosen:      Age-based, 14 days, from the roadmap's proposal. Nothing in the
             code made it untenable, which the queue set as the bar for
             departing from it.
             Age beats an LRU cap because a cap has to be tuned against a
             number nobody has measured yet — issue #13 has not run the scale
             fixture — and because the failure mode of a wrong cap is worse:
             it evicts by rank, so on a busy machine a session the owner is
             actively watching can be pushed out by newer ones. Age evicts
             only what nothing has observed for two weeks, so the dashboard's
             contents depend on this machine's activity and not on how many
             other sessions happen to exist.
             14, not 7: seven days is the retention window for normalized
             activity records, a different thing from a session summary, and
             borrowing it would evict sessions from a fortnight-long piece of
             work that the owner would reasonably still expect to see. 14, not
             90: matching retention would make the live store hold the entire
             persisted history, which is the growth this issue is about.
             The threshold is `state::LIVE_STORE_IDLE_DAYS`, one constant, and
             both call sites take it as an argument so the policy is visible
             where it is applied and testable without waiting two weeks.
Rejected:    (a) A bounded LRU-style cap — see above; revisit if #13's scale
             numbers show age alone leaves the store too large on a busy
             machine, since the two compose.
             (b) Issue #17's third option, "hide from the default view while
             keeping them queryable", read literally: keeping every session
             resident and filtering at render time. It fixes the dashboard and
             not the working set, and the working set is what the issue,
             `docs/requirements.md` §4 and the 24-hour soak gate are about.
Blast radius: `src-tauri/src/observer/state.rs` (new `evict_idle`, a filter in
             `restore_summaries`, one counter field), `src-tauri/src/
             commands.rs` (`reconcile_and_persist`, `storage_health`),
             `src-tauri/src/observer/db.rs` (`forget_cached_summaries`),
             `src-tauri/src/lib.rs` and `src-tauri/tests/durable_recovery.rs`
             (call sites). No schema change, no migration, no frontend change.
             Fully reversible: set `LIVE_STORE_IDLE_DAYS` arbitrarily high and
             the behavior is the old one, since nothing is ever deleted.

## 2026-09-23 — slice-2, PR #41
Question:    Where in the reconcile cycle does eviction run? The issue says
             "wire `Store::remove` into the reconcile path" without saying
             where, and the two obvious positions are not equivalent.
Chosen:      After `save_summaries`, not before. Backfill means a session can
             be discovered already past the threshold — `Observer::poll`
             reading a months-old transcript for the first time inserts it
             with that old `last_event_at` — so evicting before the save would
             drop it before its summary row was ever written. "Evicted
             sessions remain visible in history, backed by the durable SQLite
             summaries" would then be false for exactly the sessions most
             likely to be evicted. Saving first makes the durable row the
             thing eviction falls back on, which is what the criterion asks
             for. `push_snapshot` runs after `reconcile_and_persist` in both
             call sites, so the window still never renders the evicted
             session.
             Same cycle, the ids `evict_idle` returns are passed to
             `Db::forget_cached_summaries`, because `Db`'s write-skipping
             `summary_cache` holds a serialized copy of every summary written
             this process. Bounding `Store.sessions` while leaving that map
             unbounded would move the growth rather than remove it, and the
             cached JSON is larger than the `Session` it stands for.
Rejected:    (a) Evicting before the save — see above. (b) Leaving the
             `summary_cache` alone and saying so in the PR body. It is four
             lines to do properly, and an eviction policy whose memory saving
             is cancelled by another map is not worth reviewing.
Blast radius: `src-tauri/src/commands.rs` (`reconcile_and_persist`, ordering
             only) and `src-tauri/src/observer/db.rs` (one new method that
             touches no SQL). Forgetting a cache entry is safe by
             construction: a cache miss makes the next save write the row
             rather than skip it, so the worst case is one redundant write
             when an evicted session is resumed.

## 2026-09-23 — slice-2, PR #41 (correction, after review)
Question:    Not a spec gap — a correction. The review of PR #41 found one
             counter standing for two different events, and the build
             routine's account of the blocking criterion weaker than the truth.
Chosen:      (a) Split `Store.evicted_idle` into `evicted_idle` (sessions
             `evict_idle` removed from the live store) and
             `idle_not_restored` (rows `restore_summaries` declined to admit
             at startup), reported as two clauses. One cumulative counter
             rendered both as "N sessions ... left the live view", wrong
             about the startup group: those rows never entered the live view
             to leave it. They also differ in lifetime — `idle_not_restored`
             is one-shot, `evicted_idle` accumulates. `commands::tests::
             health_does_not_report_a_startup_skipped_row_as_having_left_the_
             live_view` pins it.
             (b) The PR body said criterion 2 failed because "there is no
             history view to be visible in" (issue #11, Phase C). That
             understated it. Verified in the code, not taken from the review:
             `restore_summaries` admits every persisted row at startup as
             `history_only`, and `lib.rs` pushes that snapshot before the
             first reconcile so the window shows prior context. And
             `Db::load_summaries` has exactly one non-test caller (that
             restore) — no command in `invoke_handler` reads persisted
             summaries. So an evicted session is not merely absent from a
             view that does not exist yet — it is unreachable from any code
             path in the app, and sessions aged 14–90 days go from rendered
             as `history_only` cards to rendered nowhere. The PR body now
             says that.
Rejected:    Summing the counters under "are not in the live view" — true of
             both, but it hides that they are different events. On (b),
             keeping the weaker wording because the criterion was unchecked
             either way: the owner picks between three options on the
             strength of that paragraph, and an understated regression is the
             failure the honesty rule exists to stop, pointing the other way.
Blast radius: `src-tauri/src/observer/state.rs` (one field split in two) and
             `src-tauri/src/commands.rs` (`storage_health` signature, the
             detail strings, a `plural` helper, one new test). No schema,
             migration or frontend change. Corrects how the policy is
             *reported*, not what it does, so it does not address the
             blocking criterion.

## 2026-09-28 — slice-3, PR #56
Question:    Issue #20 asks for a test of "a `0002_*.sql` added later", but
             `MIGRATIONS` holds exactly one entry and this slice adds no
             feature that needs a schema change. So where does the second
             migration come from?
Chosen:      Synthetic, defined in the test module and passed to the private
             `migrate(conn, path, migrations)` as the second element of a
             two-item slice whose first element is the real
             `MIGRATIONS[0]` (`0001_init.sql`). `migrate` already takes the
             migration list as a parameter — the existing
             `a_failed_migration_leaves_the_original_file_byte_identical`
             uses that seam — so the sequencing under test is the real code
             path, with only the SQL of step 2 invented. Using the real
             `0001_init.sql` as step 1 matters: the setup is an actual
             `user_version = 1` database written by `Db::open` and
             `save_summaries`, which is the "already-migrated, not fresh"
             precondition the issue says no existing test reaches.
Rejected:    Adding a real no-op `0002_*.sql` to `MIGRATIONS` so the const
             has two entries. It would bump every installed database's
             `user_version` to 2 for a table nothing reads, ship a schema
             change whose only purpose is to be tested, and make the next
             genuine migration `0003`. A test should not put state on a
             user's disk. Also rejected: hand-writing the post-migration-1
             file as a fixture, which would let the fixture drift from what
             `0001_init.sql` actually produces.
Blast radius: `src-tauri/src/observer/db.rs`, inside `#[cfg(test)] mod tests`
             only — two `#[test]` functions, two `const &str`, and four
             helpers (`migration_scratch_dir`, `summary_rows`,
             `table_exists`, `user_version`). Nothing outside the test module
             changes, so reverting cannot affect shipped behavior. The tests
             write under `std::env::temp_dir()` with a process-id-suffixed
             directory, matching the existing migration tests rather than
             adding a `tempfile` dependency.

## 2026-09-28 — slice-3, PR #56
Question:    What shape should the *failing* second migration take? The issue
             asks only that it be "invalid SQL", and the existing failure test
             uses `"THIS IS NOT VALID SQL;"` — invalid at its first statement.
Chosen:      A migration 2 that succeeds at statement 1 (`CREATE TABLE
             session_labels …`) and fails at statement 2 (an `INSERT` naming a
             column that does not exist), so the transaction holds uncommitted
             work at the moment it fails and the rollback has something to
             undo. Measured rather than assumed: with `BEGIN;`/`COMMIT;`
             stripped out of `migrate`'s per-migration batch, the existing
             `a_failed_migration_leaves_the_original_file_byte_identical` and
             `migrate_commits_user_version_and_schema_together` both still
             pass, while the new test fails. A migration that dies at its
             first statement writes nothing whether or not a transaction was
             ever opened, so it cannot distinguish a working rollback from a
             missing one.
Rejected:    Reusing the existing invalid-from-statement-one shape, which
             would have satisfied the criterion's wording while testing
             nothing the tree did not already cover. The criterion was not
             weakened to make this easier — it was read as requiring a
             rollback, and a migration that cannot exercise one does not
             prove it.

             Related, and deliberately *not* acted on: `migrate` never issues
             an explicit `ROLLBACK` after `execute_batch` fails. `BEGIN` has
             run, so the connection is left holding an open transaction, and
             the rollback happens only because the connection is dropped.
             That is correct for the sole real caller (`Db::open` propagates
             with `?` and drops its local `Connection`), and the new test
             depends on the same drop with a comment saying so. Adding a
             `ROLLBACK` would be a behavior change to the function under
             test, which a test-coverage slice should not smuggle in. Recorded
             in the PR body under "Observed, not changed" for the owner to
             file if wanted.
Blast radius: Same as the entry above — one `const &str` in the test module.
             No change to `migrate`, `MIGRATIONS`, or any migration SQL.

## 2026-10-05 — slice-4, PR #77
Question:    Issue #30 asks for retention "gated by a simple last-pruned
             timestamp" but does not say which clock. Monotonic
             (`std::time::Instant`, which the loop already keeps as
             `last_reconcile_instant`) or wall-clock (`DateTime<Utc>`, which
             the loop already keeps as `last_poll_at`)?
Chosen:      Wall-clock, as a `DateTime<Utc>` named `last_prune_at`, with
             `now < last` counted as due. Three reasons, in order of weight.
             (a) The window being enforced is wall-clock: `prune_summaries`
             derives its cutoff from `Utc::now()` minus 90 days, so a gate on
             a different clock measures something other than what it gates.
             (b) `std::time::Instant` is specified only as monotonically
             non-decreasing; whether it advances while the machine is
             suspended is a platform detail, and this loop already has a
             sleep/wake story it reasons about in wall-clock terms
             (`is_wake_gap`). A laptop closed overnight should prune on the
             first reconcile after it wakes, not an hour later.
             (c) Wall-clock is testable without a clock abstraction: the
             tests step a `DateTime<Utc>` forward by 5 seconds a tick and
             drive the real gate, where `Instant` would have needed either a
             trait or `tokio::time::pause`.
             The cost of (b) and (c) is that wall-clock can move backwards.
             `now < last` is therefore treated as due rather than as "not yet":
             a six-hour NTP correction or a user clock change would otherwise
             park retention for six hours, and an arbitrary backwards jump
             parks it indefinitely. One extra indexed `DELETE` is the cheaper
             mistake, and
             `a_backwards_clock_jump_does_not_stall_retention` pins it.
Rejected:    (a) `Instant`-based gating, for the reasons above. (b) Reusing
             the existing `last_poll_at` as the gate, which would conflate
             two independent cadences — a change to the reconcile interval
             would silently change the retention interval.
Blast radius: `src-tauri/src/lib.rs` — one new `const`, two new free
             functions (`summary_prune_due`, `prune_summaries_if_due`), one
             new loop local. No signature in `Db` changes and no SQL changes,
             so reverting is deleting the gate and the call. An installed
             database is unaffected either way: retention deletes the same
             rows the startup prune already deleted, just sooner.

## 2026-10-05 — slice-4, PR #77
Question:    Where should the periodic prune run, and what should happen when
             it fails? The issue says "from within the reconcile loop" but the
             loop does its SQL inside a `spawn_blocking` task, and
             `prune_summaries` is blocking.
Chosen:      Fold retention into the reconcile tick's existing
             `spawn_blocking` closure, after the store and observer guards are
             dropped, returning `Option<rusqlite::Result<usize>>` so the
             caller can tell "skipped, no SQL issued" from "ran". On an error
             the caller still advances `last_prune_at`, so a database that
             cannot prune is retried in an hour rather than on every 5-second
             tick.
Rejected:    (a) A separate `tokio::time::interval` task for retention. It
             would be a second writer to the one connection the module header
             documents as "touched only from blocking tasks — see `lib.rs`'s
             reconcile loop for the single-writer contract", for a job that
             runs once an hour. (b) A second `spawn_blocking` per tick just to
             evaluate the gate — 720 task spawns an hour to answer a
             subtraction. The gate is pure and the closure already holds the
             `Arc<Db>`. (c) Leaving `last_prune_at` unmoved on error so the
             next tick retries: on a locked or corrupt database that turns a
             once-an-hour `DELETE` into one every five seconds, which is the
             hot-path cost the issue's second criterion exists to prevent.
Blast radius: Same two functions plus the `match` that replaced the loop's
             `if let Err(e) = …` on the join result. The reconcile call itself
             is unchanged — it moved inside a block so its guards drop before
             retention runs. The `Err(e) => eprintln!("Observer failed: {e}")`
             arm preserves the previous behavior for a panicked task.

## 2026-10-07 — slice-5, PR #81
Question:    Issue #43 suggests
             `set_missed_tick_behavior(MissedTickBehavior::Delay)` but says
             "(or equivalent)", and tokio offers two non-default policies that
             both satisfy the issue's "collapse missed ticks into a single
             catch-up tick": `Delay` (the next deadline becomes `now + period`,
             so two ticks are never closer than the period) and `Skip` (the
             next deadline is the next multiple of the period measured from the
             interval's original start, so the original phase is preserved).
             Nothing in `docs/specification.md` picks between them.
Chosen:      `Delay`. The property this slice exists to guarantee is a *floor*
             on the spacing between reconciles, because what a reconcile costs
             is a transcript re-scan, a SQLite write and a `push_snapshot`
             event, and `docs/specification.md` §3 "IPC and UI updates" caps
             snapshots at ten a second. `Delay` provides that floor by
             construction: the period restarts from the tick that fired, so
             after any gap of any size the next tick is a full
             `RECONCILE_INTERVAL_SECS` away.
             `Skip` does not. It realigns to the original phase, so the gap
             between the catch-up tick and the one after it is whatever is left
             of the current period — anywhere from a full period down to very
             nearly zero, depending only on where in the 5-second grid the
             machine happened to wake. A wake at 600.001s after start gives
             4.999s; a wake at 604.999s gives 1ms. That is a second reconcile
             arbitrarily close behind the first, which is the burst this issue
             is about, smaller. Measured, not assumed: with `Skip` substituted
             for `Delay`, `a_suspend_gap_yields_one_catch_up_reconcile_not_a_
             storm` fails with `[0ns, 4.999s, 5s]` — the test's simulated
             suspend is deliberately 600.001s rather than a round ten minutes
             so that it can tell the two policies apart at all.
             Phase alignment, the thing `Skip` buys, is worth nothing here:
             the loop is woken by filesystem signals as well as the interval,
             so reconciles are already not on a fixed grid, and no requirement
             asks for one.
Rejected:    (a) `Skip`, above. (b) Leaving the `Burst` default and instead
             draining the backlog by hand at the top of the loop — a
             `while interval.poll_tick().is_ready()` style drain. It reaches
             the same outcome through a hand-rolled version of what the policy
             already does, keeps the misleading `Burst` reading of the timer
             in the source, and adds a `poll`-level construct to a loop that
             currently reads as plain `async`/`await`. (c) Reducing
             `RECONCILE_INTERVAL_SECS` or reaching for an OS power-event API:
             neither is this issue, and the design doc's non-goals rule the
             second out.
Blast radius: `src-tauri/src/lib.rs` — one new `fn reconcile_interval()`, one
             `use` line, and the two lines in `tauri::Builder::setup` that
             built the interval inline. No const, signature, schema, migration
             or frontend change. Fully reversible: delete the
             `set_missed_tick_behavior` call and the behavior is tokio's
             default again. Nothing persists, so there is no state to migrate
             in either direction.

## 2026-10-07 — slice-5, PR #81
Question:    Issue #43's second criterion asks for "a test or documented manual
             check" that a simulated multi-tick gap yields at most one
             immediate reconcile. The reconcile loop lives inside
             `tauri::Builder::setup`, which no test can enter — the same gap
             slice-4's review recorded as a carry-forward. So is the criterion
             met by a test, or does it fall to a manual check the owner has to
             perform on a real machine?
Chosen:      By a test, after lifting the loop's wait step into
             `async fn wait_for_reconcile(interval, watch_rx,
             last_reconcile_instant)`. The `tokio::select!` moved verbatim;
             the loop body now calls it. That makes the thing under test the
             real wait the real loop performs, driven by
             `#[tokio::test(start_paused = true)]` plus
             `tokio::time::advance`, rather than a reimplementation of the
             select in the test module.
             The limit, stated rather than papered over: the test measures
             **when the loop would be released to run**, not the reconcile or
             the `push_snapshot` that follows. Those are still unreachable, so
             "at most one reconcile/snapshot" is proved at the timer that
             drives them and inferred, not observed, one step downstream. One
             release of the wait is one iteration of the loop, so the
             inference is tight, but it is an inference. This slice narrows
             slice-4's carry-forward; it does not close it.
             `test-util` is added to tokio under `[dev-dependencies]`, so the
             paused clock is a test-only feature and the shipped binary does
             not carry it (resolver v2 does not enable dev-dependency features
             for a normal build). `Cargo.lock` is unchanged — `test-util`
             pulls in no new crate.
Rejected:    (a) A documented manual check, i.e. asking the owner to suspend a
             laptop and read stderr. It is the slower half of the criterion's
             "or", it spends the scarce resource `docs/roadmap.md` is built
             around (the owner's review hour), and `AGENTS.md`'s bar on
             evidence cuts the other way too: a test that fails without the
             change is better evidence about the code than a one-off
             observation, even though neither is evidence about a real host.
             (b) Testing `reconcile_interval()` alone, with the `select!` left
             inline in the loop. It would pin the policy without proving the
             loop uses it, and the loop building its own interval inline is
             exactly how the `Burst` default survived unnoticed.
             (c) Extracting the whole tick body — reconcile, retention,
             `push_snapshot` — into a callable function so iterations could be
             counted outright. That is the refactor that would actually close
             slice-4's carry-forward, and it needs a seam for the `AppHandle`
             that `push_snapshot` takes. Out of proportion to this slice, and
             it would put the diff well past the point where it stops being
             reviewable in half an hour.
Blast radius: `src-tauri/src/lib.rs` (the `select!` moved out of the loop into
             `wait_for_reconcile`, called with the same three values it read
             from scope before) and `src-tauri/Cargo.toml` (one
             `[dev-dependencies]` block). The select's arms, the drain loops
             and the `watcher_reconcile_delay` call are byte-identical to what
             was inline. Reverting is inlining the body again.

## 2026-10-07 — slice-5, PR #81
Question:    The queue says issue #54 — the wake-gap log claiming "forcing full
             reconciliation" when the reconcile that follows is the ordinary
             incremental one — "touches the same lines; fix the log message
             only if it falls out of the change". #54 itself offers two routes:
             reword the log to describe what happens, **or** implement a
             genuinely different wake-gap path (a non-incremental rescan of all
             roots). Which, and does it fall out of this change at all?
Chosen:      Reword only, and yes it falls out: the three lines above the
             `eprintln!` are the lines this slice rewrites, and the sentence
             "forcing full reconciliation" is now wrong in a second way as
             well as the first. Before this change a wake produced a storm of
             reconciles; after it, exactly one. A reader reconciling that log
             line against the new behavior would be reading a claim that was
             never true about the path's content and is now also wrong about
             its count. Leaving it would mean shipping a change to post-wake
             behavior while the only post-wake log line describes something
             else.
             The new text names the gap, says the process was probably
             suspended, and says the reconcile is the normal incremental one
             and not a full rescan — nothing the code does not do.
Rejected:    (a) The second route, implementing a real full rescan on a wake
             gap. It is a behavior change to recovery semantics that nothing in
             this slice requires, it would need its own evidence about whether
             an incremental reconcile actually misses anything across a
             suspend — which is host evidence this routine cannot produce — and
             the queue scoped the log to "only if it falls out".
             (b) Leaving the message alone as out of scope. The queue permits
             the fix on exactly this condition and asks that the PR say so,
             which it does.
             Not claimed: that this closes #54. The reword satisfies #54's
             first acceptance route for this one log line, and whether that is
             enough to close the issue or whether the owner wants the real
             rescan instead is the owner's call. No regression test covers the
             wording; it is a string literal, and a test asserting its text
             would pin the phrasing rather than any behavior.
Blast radius: `src-tauri/src/lib.rs` — one `eprintln!` format string and the
             comment above it. `is_wake_gap`, its threshold, its two existing
             tests and the `if` that gates the log are untouched, so the
             detection logic and when the line is emitted are unchanged. Only
             the words change.
