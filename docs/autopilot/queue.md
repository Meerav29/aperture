# Autopilot queue — aperture

Design: `grid-prophet/docs/superpowers/specs/2026-09-19-autopilot-design.md`
Rotation: Mon / Wed / Fri. Base branch: `auto/queue` (consolidated into `staging`; see [branching](../branching.md)). Window: 2026-09-20 → 09-25.

## Read this before picking up a slice

Read [AGENTS.md](../../AGENTS.md), [docs/roadmap.md](../roadmap.md), and
[docs/product-vision.md](../product-vision.md) first. They override this file on
approach, on sequencing, and on what may be claimed.

We are inside **Phase A — Dogfood gate (Sep 16 → Oct 9)**. Phase A's gate is
five consecutive clean workdays in the dogfood log plus six evidenced Windows
rows. **Autopilot cannot advance that gate and does not try.** Items 2 through 5
of Phase A — hook enrichment on the owner's machine, live-host evidence, the
dogfood log, stability fixes found by the log — all require the owner at a real
machine.

What autopilot *can* do is Phase A item 1, the correctness leftovers, which the
roadmap describes as "small and unblock long-running daily use." That is slices
1 and 2 below. Slice 3 is protective test coverage for the same storage layer.

### Binding constraints

- **Never write to `docs/validation/**`.** That lane is the owner's.
- **Never assert host compatibility, runtime behavior, or validation status.**
  Passing tests are evidence about code, not about a real host.
- **Never edit `docs/roadmap.md` or `docs/goals.md`.** Gate status is the
  owner's to record. (`docs/goals.md` is separately stale per issue #22, being
  handled on the `claude/github-quick-wins-1m20eg` branch — leave it alone.)
- Never launch, resume, message, or control a real agent session to make a test
  pass. `AGENTS.md` forbids it.
- Passive discovery must keep working without hooks or provider settings edits.
- **Every PR ships a review pack** — what the owner runs and looks at, sized to
  30 minutes or less. The roadmap is explicit: a PR without one is not ready.
- The roadmap caps work in progress at **two open implementation branches**.
  Autopilot halts if the previous slice has not merged, so it holds at one.
- If a criterion cannot be checked, say so in the PR body and let the slice be
  rejected. Do not soften the criterion.

Status values: `todo` → `in-progress` → `in-review` → `merged`.
Terminal: `held` (owner vetoed), `blocked` (review rejected).

---

## slice-1 — Issue #18: stop `load_summaries` silently dropping rows

Status: merged
Issue: [#18](https://github.com/Meerav29/aperture/issues/18) (Phase A)
PR: [#33](https://github.com/Meerav29/aperture/pull/33) — merged to `auto/queue`
2026-09-21 as `185dbab`.
Code: `src-tauri/src/observer/db.rs` (`Db::load_summaries`)

**This one goes first for a reason.** The issue argues that `Session`'s shape
will change as #9 (SessionKey) and #7 (Git identity) land, and each such change
risks silently orphaning persisted rows unless this is fixed first. Sequencing
the Git-identity work ahead of it would be backwards.

Acceptance (from the issue — do not weaken):

- [ ] A test proves a row with an incompatible or corrupted JSON body is
      **detected — counted or logged** — rather than silently vanishing, while
      the remaining valid rows still load.
- [ ] The failure is visible somewhere a developer or user could actually
      notice, at minimum log output. Not merely absent from the returned
      `Vec<Session>`.
- [ ] A decision is recorded on whether `Session` gets `#[serde(default)]` /
      tolerant deserialization for additive changes, **or** whether breaking
      shape changes are accepted as a now-visible migration cost. Either answer
      is defensible; record which and why in `docs/autopilot/decisions.md`.
- [ ] Consistent with the honesty principle this issue cites from
      `docs/specification.md` §"Storage and historical ingestion": a failure is
      surfaced, never presented as a clean empty state.
- [ ] `cargo test` passes; `npm run build` passes.

---

## slice-2 — Issue #17: bound `Store.sessions` with an eviction policy

Status: merged
PR: [#41](https://github.com/Meerav29/aperture/pull/41) — merged to `auto/queue`
by the owner 2026-09-27 as `37a0e03`, after criterion 2 was reworded to its
durability half (owner decision 2026-09-26, option 1 from the review on #41).
Making evicted sessions visible in a history view moved to #11.
Issue: [#17](https://github.com/Meerav29/aperture/issues/17) (Phase A)
Code: `src-tauri/src/observer/state.rs` (`Store::remove`, currently uncalled)

The roadmap proposes the rule: a session with no observation for **14 days**
leaves the live store but stays in SQLite and returns through history views in
Phase C. Use that unless the code makes it untenable; if it does, say so and
record the alternative.

Acceptance (from the issue — do not weaken; criterion 2 reworded by the owner
on 2026-09-26, see Status):

- [ ] A documented, tested policy bounds `Store.sessions` independent of total
      historical session count.
- [ ] **Eviction from memory is not deletion.** An evicted session's summary
      row stays in SQLite, and a test proves it is still there after eviction.
      Making evicted sessions visible in a history view is #11's job, not this
      slice's.
- [ ] A regression test proves `Store.sessions` does not grow unbounded across
      many simulated reconcile cycles with aging sessions.
- [ ] `Store::remove` (or equivalent) is actually wired into the reconcile path
      — the issue's point is that the method exists but has no caller.
- [ ] `cargo test` passes; `npm run build` passes.

Do **not** claim the 24-hour soak or idle-memory targets are met. The issue is
explicit that measurement stays issue #13's job. Making the targets *achievable
in principle* is this slice's bar.

---

## slice-3 — Issue #20: multi-step migration test coverage

Status: merged
Issue: [#20](https://github.com/Meerav29/aperture/issues/20)
PR: [#56](https://github.com/Meerav29/aperture/pull/56) — merged to `auto/queue`
by the review routine 2026-09-28 as `9f604ed`.
Code: `src-tauri/src/observer/db.rs` (`migrate`, `MIGRATIONS`)

Both criteria were verified against the diff, and both of the PR's mutation
claims were reproduced independently rather than taken on trust: with
`.skip(version)` removed from `migrate`'s loop only
`a_second_migration_applies_on_top_of_an_already_migrated_database` fails, and
with `BEGIN;`/`COMMIT;` removed from the per-migration batch only
`a_failing_second_migration_leaves_an_already_migrated_database_intact` fails.
Every pre-existing test passes against both breaks, which is issue #20's claim
confirmed. `cargo test --locked` (65 passed) and `npm run build` were re-run in
a Linux sandbox; CI was green on `d273162`, `rust` on `windows-latest`.

Carried forward, not fixed here: `migrate` issues no explicit `ROLLBACK` after
`execute_batch` fails, so the connection is left holding an open transaction and
the rollback depends on the connection being dropped. Correct for the sole real
caller (`Db::open`), a trap for any future one. The PR flags it under "Observed,
not changed" for the owner to file.

Pure test work on the storage layer that slices 1 and 2 both touch. No product
behavior changes, so it is sequence-neutral and safe to land last.

Acceptance (from the issue — do not weaken):

- [ ] A test proves a second migration applies cleanly on top of an
      **already-migrated** database (not a fresh one), reaching
      `user_version = 2` with migration 1's rows untouched.
- [ ] A test proves a **failing** second migration rolls back cleanly, leaving
      the database at its last successfully committed version with data intact
      — byte-identical to the post-migration-1 state, not to a fresh file.
- [ ] `cargo test` passes; `npm run build` passes.

---

## slice-4 — Issue #30: run `prune_summaries` during a long-running process

Status: merged
PR: [#77](https://github.com/Meerav29/aperture/pull/77) — merged to `auto/queue`
by the review routine 2026-10-05 as `e42c33f`.
Issue: [#30](https://github.com/Meerav29/aperture/issues/30)
Code: `src-tauri/src/observer/db.rs` (`prune_summaries`), startup/reconcile wiring

Both of issue #30's criteria were verified against the diff rather than against
the PR body. CI was green on `fe09a28` — both jobs, `rust` on `windows-latest`.
The diff is 283 lines across 4 files, well inside the 600-line review cap, and
touches nothing outside the slice: `lib.rs` (the change), a `#[cfg(test)]`
`insert_aged_summary` helper in `db.rs` that the retention test needs,
`decisions.md`, and this file.

The wiring is live on the production path and not only in tests: the reconcile
loop computes `now = Utc::now()` each tick and calls `prune_summaries_if_due`
inside the `spawn_blocking` task it already spawns, after the observer and store
guards drop, advancing `last_prune_at` on both the success and the error arm. So
a failed prune is retried in an hour rather than on every 5-second tick, which
is the hot-path cost criterion 2 exists to prevent.

**The review routine could not execute `cargo test` in its sandbox** — the test
command was blocked by the environment's permission classifier, not by a missing
toolchain (the Tauri system deps were installed successfully first). Both of the
PR's mutation claims were therefore re-derived from the test source rather than
re-run, and both hold exactly as stated: with `prune_summaries_if_due` returning
`None` unconditionally — precisely the pre-change behavior — only
`a_long_running_process_prunes_an_expired_summary_without_restarting` and
`retention_does_not_run_on_every_reconcile_tick` fail, because the three
remaining new tests drive the pure `summary_prune_due` predicate directly (2
failures, 68 pass). With `summary_prune_due` returning `true` unconditionally,
those two fail on their pass counts (720 and 1440 against the asserted 1 and 2)
along with `retention_is_not_due_before_the_interval_elapses` (3 failures, 67
pass). The first break is the one criterion 1 turns on. The assertions reach
real SQLite rather than a mock: `insert_aged_summary` backdates `updated_at`
directly and `summary_updated_at` reads the column back, against the same
`updated_at < cutoff` predicate `prune_summaries` issues.

Two things carried forward, neither blocking:

- **The production call site is not test-protected.** Deleting the
  `prune_summaries_if_due(&d, prune_from, now)` line from the reconcile loop
  leaves all 70 tests green, because the loop lives inside
  `tauri::Builder::setup` and no test reaches it. The helper is the testable
  seam and the tests sit on it, so the behavior does have failing-without cover;
  what is missing is regression cover on one line of wiring. This is the same
  gap PR #10's review flagged in grid-prophet, and it is structurally harder to
  close here than it was there — extracting the tick body into a callable
  function would be the way, and that is a refactor this slice did not need.
- **The PR body understates its own `db.rs` change.** It says "no change to
  `Db::prune_summaries` or any other `Db` signature," which is true of
  production code, but the diff does add a new `#[cfg(test)]` method to `Db` and
  rewrites the existing `prune_summaries` test to use it. Harmless and in scope
  — the refactor removes a hand-rolled `INSERT` from that test — but the body
  reads as though `db.rs` were untouched.

The PR's own honesty is worth recording: it states that its simulated clock
drives the gate only, that `prune_summaries` still derives its cutoff from the
real `Utc::now()` so a row cannot be made to *cross* the window mid-test, and
that nothing here is evidence about a real host, a soak, or bounded memory over
an 8- or 24-hour run. That last measurement remains issue #13's. `AGENTS.md`'s
bar against turning missing evidence into success is met.

Queued 2026-10-01 after the queue ran dry. Storage-layer correctness in the same
area as slices 1-3; no host evidence needed. Acceptance is in the issue; do not
weaken it. At minimum: retention runs on a schedule or reconcile cadence, not
only once at startup, and a test proves rows older than the window are pruned
by the periodic path. `cargo test` and `npm run build` must pass.

---

## slice-5 — Issue #43: reconcile `interval` bursts after sleep/wake

Status: merged
PR: [#81](https://github.com/Meerav29/aperture/pull/81) — merged to `auto/queue`
by the review routine 2026-10-07 as `5e694a3`.
Issue: [#43](https://github.com/Meerav29/aperture/issues/43)
Code: `src-tauri/src/lib.rs` (reconcile loop)

`reconcile_interval()` sets `MissedTickBehavior::Delay`; the loop's
`tokio::select!` wait moved into `wait_for_reconcile` so the cadence is
reachable from a test at all. Four paused-clock tests; two of them fail
without the change (`Burst` restored: 72 pass, 2 fail — and the suspend test's
failure output is `[0ns, 0ns, 0ns]`, the storm itself), one also fails under
the rejected `Skip` policy (`[0ns, 4.999s, 5s]`). The other two are labelled
guards, not evidence. `cargo test` (74 passed) and `npm run build` both ran in
a Linux sandbox after installing the Tauri system libraries, and CI was green
on `ee71a42` — both jobs, `rust` on `windows-latest`.

**Both mutation claims were re-run by the review routine, not read.** With
`set_missed_tick_behavior` removed — precisely the pre-change tree — the suite
reports 72 passed, 2 failed, and the suspend test's own output is
`got [0ns, 0ns, 0ns]`, the storm itself, matching the PR body verbatim; the
overrun test fails alongside it. With `Skip` substituted for `Delay` the suite
reports 73 passed, 1 failed, `got [0ns, 4.999s, 5s]` — the sub-period second
reconcile Decision 1 rejects. Every pre-existing test passes under both
substitutions, and both of the PR's self-labelled guard tests
(`the_steady_state_cadence_is_one_reconcile_per_interval`,
`a_watcher_signal_does_not_consume_an_interval_tick`) pass under `Burst` as the
body says they do. `cargo test --locked` (74 passed) and `npm run build` (tsc
clean, 34 modules) were both re-run in the review sandbox against the pristine
head; `--locked` passing is independent confirmation of the body's claim that
`test-util` adds no crate and `Cargo.lock` is unchanged. The diff is 408 lines
across 4 files, inside the 600-line cap, and the extracted `select!` is
byte-identical to the block it replaced — `last_reconcile_instant` is `Copy`
and the loop reassigns its own copy at the end of every iteration, so passing
it by value changes nothing.

**Two documentation defects, neither blocking, both worth fixing in the next
slice that touches these files.** The three new `decisions.md` entries are
headed `PR #TBD` — the second commit filled the PR number into this file but
not into that one, and no earlier entry in `decisions.md` uses a placeholder.
And the PR body has no `## Deliberately not included` section, which spec §9
lists as required; its content is genuinely present, spread across "On #54",
"Observed, not changed" and the Decisions' *Rejected* lists, and PR #77 was
merged with the same omission, so this was not treated as a rejection — but it
is now two slices running.

Carried forward, in the PR under "Acceptance criteria" and "Observed, not
changed": the test measures when the loop is *released to run*, not the
reconcile and `push_snapshot` that follow, because the loop body is still
inside `tauri::Builder::setup` — this narrows slice-4's carry-forward rather
than closing it. And `is_wake_gap`'s doc comment now claims slightly more than
it can ("can only come from lost wall-clock time"), since under `Delay` a
single 90s reconcile pass can trip the detector; not changed, as the threshold
would need issue #13's numbers.

#54's log message was reworded, which the queue permits "only if it falls out
of the change" — the PR argues it does and says so. **#54 is not closed by
this**: its other route, a genuinely different wake-gap rescan, needs host
evidence and is the owner's call.

Set an explicit `MissedTickBehavior` and prove with a paused-clock test that a
long gap yields one reconcile, not a storm. Do not claim real sleep/wake
behavior is validated; that is host evidence (see binding constraints). Issue
#54 (misleading "forcing full reconciliation" log) touches the same lines; fix
the log message only if it falls out of the change, and say so in the PR.

---

## slice-6 — Issue #50: `storage_health` discards the real error

Status: todo
Issue: [#50](https://github.com/Meerav29/aperture/issues/50)
Code: `src-tauri/src/observer/` (`storage_health`)

Surface the real `rusqlite::Error` (as a sanitized message) in the health row
instead of one generic string. A test proves two different failures produce two
different messages. Also take the carry-forward from slice-3: `migrate` issues
no explicit `ROLLBACK` after `execute_batch` fails. Add one with a test, or
record in `decisions.md` why not.

---

## Explicitly not in this queue

- **#7** (Git repository/worktree identity) and **#9** (SessionKey contract) are
  Phase B, starting Oct 12, and #18 argues they should follow slice 1 anyway.
- **#11** (tray, notifications, filters, accessibility) is Phase C — December.
- **#5**, **#4**, **#6**, **#13** need real-host or real-fixture evidence.
- **#22** is in flight on another branch.
- **#24** (no CI, no frontend test tooling) is **half-addressed** by the CI
  workflow added in the autopilot setup commit. Frontend test tooling remains
  open; the issue should not be closed on the strength of that workflow alone.
