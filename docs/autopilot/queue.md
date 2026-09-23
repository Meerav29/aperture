# Autopilot queue — aperture

Design: `grid-prophet/docs/superpowers/specs/2026-09-19-autopilot-design.md`
Rotation: Mon / Wed / Fri. Base branch: `auto/queue`. Window: 2026-09-20 → 09-25.

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

Status: in-progress
Issue: [#17](https://github.com/Meerav29/aperture/issues/17) (Phase A)
Code: `src-tauri/src/observer/state.rs` (`Store::remove`, currently uncalled)

The roadmap proposes the rule: a session with no observation for **14 days**
leaves the live store but stays in SQLite and returns through history views in
Phase C. Use that unless the code makes it untenable; if it does, say so and
record the alternative.

Acceptance (from the issue — do not weaken):

- [ ] A documented, tested policy bounds `Store.sessions` independent of total
      historical session count.
- [ ] **Evicted sessions remain visible in history**, backed by the durable
      SQLite summaries. Eviction from memory is not deletion.
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

Status: todo
Issue: [#20](https://github.com/Meerav29/aperture/issues/20)
Code: `src-tauri/src/observer/db.rs` (`migrate`, `MIGRATIONS`)

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

## Explicitly not in this queue

- **#7** (Git repository/worktree identity) and **#9** (SessionKey contract) are
  Phase B, starting Oct 12, and #18 argues they should follow slice 1 anyway.
- **#11** (tray, notifications, filters, accessibility) is Phase C — December.
- **#5**, **#4**, **#6**, **#13** need real-host or real-fixture evidence.
- **#22** is in flight on another branch.
- **#24** (no CI, no frontend test tooling) is **half-addressed** by the CI
  workflow added in the autopilot setup commit. Frontend test tooling remains
  open; the issue should not be closed on the strength of that workflow alone.
