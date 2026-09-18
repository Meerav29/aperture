# Aperture development roadmap: September 2026 to March 2027

Status: planning document, written 2026-09-16. It sequences the phases in
[specification.md](specification.md) onto a calendar with two checkpoints, and
it says what the owner verifies at each gate. It does not change product scope;
[product-vision.md](product-vision.md) still governs intent and
[requirements.md](requirements.md) still governs acceptance. Update this file
when a gate is passed, missed, or moved; keep [goals.md](goals.md) as the
short current-status ledger.

## Checkpoints

| Checkpoint | Date | Meaning |
|---|---|---|
| Dogfood gate | 2026-10-09 | The owner leaves Aperture open every workday because the grid is trustworthy on Windows |
| **Checkpoint 1: v0.2 personal beta** | **2026-12-18** | Daily-use features done, unsigned builds installed by friends, first Mac report returned |
| **Checkpoint 2: v1.0 production release** | **2027-03-26** | Signed/notarized builds, all requirements R1-R12 evidenced on advertised systems |

Dates are targets with slack built in, not promises. A missed gate moves the
gate, and the slip rules below say what drops out first.

## How this plan is sized

Implementation runs mostly through Claude Code and Codex sessions and can
happen around the clock. The scarce resource is the owner's one to two hours a
day for review and manual validation, plus a few friends who test and
occasionally fix bugs. One friend has a Mac and can run a checklist every few
weeks. So:

- Every goal ships with a **review pack**: what the owner runs and looks at,
  sized to 30 minutes or less. A PR without a review pack is not ready.
- A phase gate is verified by the owner on a real machine, never by a merged PR
  or a green test run. This is the rule already in goals.md, applied per phase.
- Work in progress is capped at two open implementation branches. A third
  waits. Agents can implement faster than the owner can verify, and unverified
  merges are how "done" claims drift away from evidence.
- Manual validation rows are spread out, one row per review session, instead
  of batched into a single long evening.
- Packaged builds arrive in Phase C, earlier than the spec's original Phase 3
  placement, because friends need something to install before they can test.

## Phase map

| Phase | Dates | Theme | Gate | Issues |
|---|---|---|---|---|
| A | Sep 16 - Oct 9 | Dogfood gate | Five straight workdays open with no missed or falsely live session; six Windows rows evidenced | #5, #17, #18 |
| B | Oct 12 - Nov 6 | Organization | Real repositories and linked worktrees group correctly; typed session keys; scale fixture stays responsive | #7, #9, #13 (part) |
| C | Nov 9 - Dec 18 | Daily-use beta | Eight-hour soak; friends install a pre-release; Mac round 1 report exists | #11, #16, #4 (round 1) |
| Slack | Dec 21 - Jan 3 | Holiday and friend-bug intake | No new features | |
| D | Jan 4 - Feb 5 | Hardening | Safe automated hook setup; CI on both OSes; performance targets measured; Mac round 2 | #15, #8, #6, #13 |
| E | Feb 8 - Mar 26 | Production | Signed/notarized builds; upgrade and uninstall tests; 24-hour soak; Mac round 3 on the release candidate | #12, #4 (final) |

Spec mapping: Phase A finishes the Windows half of spec Phase 0 and the
correctness leftovers of spec Phase 1. Phase B is spec Phase 2 minus exact
navigation. Phase C is spec Phase 3. Phase D picks up the spec Phase 1 and 2
items that were deferred (automated installer, child handling, exact
navigation). Phase E is spec Phase 4.

## Phase A — Dogfood gate (Sep 16 to Oct 9)

Goal: the owner can rely on the grid to show every agent working on this
machine, with attention that is right when it is shown and explicitly unknown
when it is not.

Work, in order:

1. **Correctness leftovers from the persistence work.** Issue #18: give
   `Session` serde forward-compatibility and stop `load_summaries` from
   silently dropping rows; surface load failures in the storage health entry.
   Issue #17: define and implement eviction from the in-memory store. Proposed
   rule: a session with no observation for 14 days leaves the live store but
   stays in SQLite and reappears through history views in Phase C. Both are
   small and unblock long-running daily use.
2. **Hook enrichment on the owner's machine.** Follow the manual merge
   procedure in the [Windows follow-up](validation/windows-compatibility-followup.md)
   for both providers. Measure helper latency as seen by the provider. Record
   whether PermissionRequest supplies tool-use IDs for each installed version.
   Capture sanitized real fixtures for permission, question, interrupt, error,
   and any child/subagent events; these feed Phase D.
3. **Windows live-host evidence, issue #5.** One row per review session:
   Claude Code and Codex in an interactive terminal, VS Code, and the desktop
   app. Disposable tasks only. Record each row in a new
   `docs/validation/windows-live-matrix.md` using the evidence template from
   the [macOS checklist](validation/macos-checklist.md). Then update the
   SPIKE.md matrix from observed outcomes.
4. **Dogfood log.** Add `docs/validation/dogfood-log.md` with one line per
   workday: sessions actually running, sessions shown, false attention alerts,
   missed sessions, crashes, anything odd. Five consecutive clean workdays
   passes the gate. The log continues through every later phase.
5. **Stability fixes found by the log.** Budget the last week for whatever
   the log turns up. Nothing new starts until the log is clean.

Exit gate:

- Dogfood log shows five consecutive workdays with zero missed sessions, zero
  crashes, and at most one false attention alert per day.
- All six Windows rows have a live-evidence record with versions. Rows that
  genuinely cannot be exercised (for example a host that does not emit a
  permission prompt) are marked pending with the reason, not skipped.
- Issues #17 and #18 closed with tests.
- Startup to first snapshot with the owner's real history is two seconds or
  less, timed by hand.

Review pack per session: run the app from `npm run tauri dev`, exercise one
matrix row, write the dogfood line, check the storage health entry.

Slip rule: if the terminal and desktop rows pass but a VS Code row cannot be
evidenced, Phase B starts anyway and the row stays open under #5. If the
dogfood log is not clean by Oct 9, Phase B slips a week; it does not start on
an unreliable grid.

## Phase B — Organization (Oct 12 to Nov 6)

Goal: sessions are grouped repository to worktree to session, using real Git
identity, through a typed contract the later phases can build on.

Work, in order:

1. **Typed SessionKey and IPC revision, issue #9.** Rust-owned `SessionKey`
   (provider plus native ID, optional child ID), `NavigationResult`, and
   generated TypeScript bindings via `ts-rs` so the hand-synced `types.ts`
   goes away. Session and navigation commands take a key. Add persistent
   `hide_session` and `navigate_session(key, action)`. Reject snapshots older
   than the current revision on the frontend. This is first because the Git
   fields and the grouped UI both change the contract; changing it once is
   cheaper than twice.
2. **Git identity service, issue #7.** Read-only Git queries with argument
   arrays, a two-second timeout, run outside the store lock and cached per
   working directory. Canonical common directory identifies the repository,
   checkout root identifies the worktree, branch or detached HEAD is recorded.
   Refresh on rescan and when a session's working directory changes. Non-Git
   folders, deleted worktrees, and missing Git must not break collection.
3. **Grouped dashboard.** Default view repository to worktree to session with
   the attention strip above the groups. Sessions in the same worktree stay
   distinct. Keep a flat view as a toggle. Provider, status, and repository
   filters land here; text search and details wait for Phase C.
4. **Scale fixture, first half of issue #13.** A generator that produces
   50 active sessions, 10,000 summaries, five repositories, and several
   worktrees under a temporary provider root. Measure persisted initial
   results and ingestion-to-visible latency at a steady 20 events per second.
   Record the numbers on the owner's machine even if they miss target; fixing
   them is Phase D work.

Exit gate:

- The owner's real layout groups correctly: this repository with its linked
  worktrees under `.claude/worktrees`, plus at least one other repository and
  one non-Git folder. Two sessions in one worktree appear as two cards.
- A deleted worktree and a session in a subdirectory of a checkout both
  resolve without error.
- Scale fixture numbers are recorded with hardware and commit. Targets to
  compare against: p95 initial results within two seconds, p95 event to
  visible within one second.
- `types.ts` is generated, not hand-maintained.

Review pack per session: start a disposable session in a worktree, confirm the
card lands under the right repository and worktree, hide it, restart, confirm
it stays hidden.

Slip rule: the scale fixture can move to Phase D without moving the gate. Git
grouping cannot; it is the reason this phase exists.

## Phase C — Daily-use beta (Nov 9 to Dec 18)

Goal: Aperture behaves like an installed desktop app, friends can install it,
and the first Mac evidence exists.

Work, in order:

1. **Tray and lifecycle.** Tray or menu-bar icon, hide on window close after a
   one-time onboarding explanation, explicit Quit stops collection, single
   instance enforcement. Bundle fonts locally.
2. **Opt-in notifications.** Off by default. Notify on entry into permission,
   input, or error attention, deduplicated per request or turn, suppressed
   during backfill and restart. Content is provider and repository plus a
   generic message, never prompt text.
3. **Daily-use UI, issue #11.** Text search, session details with a bounded
   timeline and freshness evidence, active/history/hidden views, keyboard
   navigation, visible focus, reduced-motion support, sensible behavior at
   OS scaling. Evicted sessions from Phase A show in the history view.
4. **Diagnostic export and history purge, issue #16.** Sanitized export that
   excludes prompts and transcript text. Purge Aperture history without
   touching provider files, with a note that discovery rebuilds it.
5. **Packaging for friends.** GitHub Actions workflow that builds an unsigned
   Windows installer and an unsigned macOS app bundle on tags matching
   `v0.2.0-beta.*` and publishes a pre-release. Add `TESTING.md` describing
   install, what to try, and how to file an issue. Signing waits for Phase E;
   friends accept the SmartScreen and Gatekeeper warnings knowingly.
6. **Mac round 1, week of Dec 1.** The friend runs the
   [macOS checklist](validation/macos-checklist.md) against the pre-release
   and returns `docs/validation/macos-results.md`. Failures are expected and
   are the point of the round.

Exit gate:

- Eight-hour soak on the owner's machine with at least one sleep/wake and one
  provider restart: no crash, no session stuck at confirmed working, memory
  not growing without bound (record start and end working set).
- At least two friends installed the pre-release and filed or confirmed the
  absence of issues.
- Notification deduplication verified: one permission request produces one
  notification, resolved requests produce none, backfill produces none.
- Mac round 1 report exists with all six rows attempted. Passing is not
  required; a documented result is.
- Requirement R11 has end-to-end coverage per the spec's UI/lifecycle test
  family.

Review pack per session: install the latest pre-release over the previous one,
work a normal day with it in the tray, note anything in the dogfood log.

Slip rule: search and details can slip into the holiday window. Tray,
notifications, and packaging cannot; without them friends have nothing to test
and Checkpoint 1 is not a beta.

**Checkpoint 1 on Dec 18: tag v0.2.0.** Update goals.md and SPIKE.md from
evidence, not from the issue list.

## Holiday slack (Dec 21 to Jan 3)

No new features. Triage friend reports into issues, fix what is small, and
write the Phase D review packs. If Phase C slipped, this window absorbs it.

## Phase D — Hardening (Jan 4 to Feb 5)

Goal: the pieces deferred from spec Phases 1 and 2 land with the safety
contract the spec demands, and the numbers get measured against targets.

Work, in order:

1. **Safe automated hook setup, issue #15.** Per-provider enable, repair, and
   remove in the integrations screen, meeting the configuration-safety
   contract in the specification: provider-compatible parsers that preserve
   unknown fields and TOML comments, backup with collision-resistant names,
   atomic replace after an unchanged check, exact owned-handler tracking,
   and removal that leaves mixed groups intact. The spec's installer test
   family is the acceptance list. The removed legacy installer is not reused.
2. **Child and subagent handling, issue #6.** Use the fixtures captured in
   Phase A. Children aggregate under the parent; a stopped child never marks
   the parent complete.
3. **Navigation beyond folder, issue #8.** Time-boxed to two weeks. Per host,
   find a verifiable target: VS Code window focus through a documented
   mechanism, Windows foreground-window focus only with a verified PID and
   process start time, and provider URL schemes only where documented and
   tested. Anything not verified stays an accurately labeled fallback.
   Expect the outcome to be partial; the deliverable is the tested set plus
   the documented unverifiable set.
4. **Performance against targets, issue #13.** Rerun the Phase B fixture.
   Idle soak with average CPU under 2 percent and working set under 300 MiB,
   p95 latency targets as above. Profile and fix, or revise the requirement
   explicitly with a reason.
5. **CI.** GitHub Actions on `windows-latest` and `macos-latest`: `npm run
   build`, `cargo test --locked`, and a Tauri build. Required on pull
   requests. This catches compile and test regressions on macOS between friend
   rounds; it proves nothing about signed-in host behavior and the docs say so.
6. **Mac round 2, mid-January**, against a fresh pre-release with the round 1
   fixes.
7. **Start the signing paperwork.** Apple Developer Program enrollment and a
   Windows code-signing option (Azure Trusted Signing or an OV certificate)
   both have lead time. Begin here so Phase E is not blocked on a vendor.

Exit gate:

- Repeated install, repair, remove, and a concurrent hand edit of provider
  settings leave every unrelated entry byte-identical. SHA-256 before and
  after, as in the original Windows validation.
- Performance numbers recorded against every target, with a pass or an
  explicit requirement revision for each.
- CI green on both runners for a week of merges.
- Mac round 2 report shows the round 1 failures addressed or reclassified.
- Navigation matrix lists a tested action or an honest fallback for every
  host.

Review pack per session: run an install/repair/remove cycle on the owner's
real settings with a backup in hand; check one navigation action per host.

Slip rule: exact navigation is the first thing to drop; the fallback already
works. The installer is the second; it can ship in v1.1 if the manual merge
procedure is documented well enough for release, but that decision gets
recorded in goals.md, not made by omission.

## Phase E — Production (Feb 8 to Mar 26)

Goal: someone who is not a friend can install Aperture from a GitHub release
and trust it.

Work, in order:

1. **Signing and notarization, issue #12.** Signed Windows installer, Developer
   ID signed and notarized macOS distribution, release CI for the advertised
   architectures (x64 Windows, Apple silicon and Intel macOS unless Phase D
   evidence narrows it). Manual signed upgrades; automatic updates are not in
   scope.
2. **Upgrade, migration, and uninstall.** Install v0.2.0, upgrade to the
   candidate, confirm the SQLite migration backup and recovery path, confirm
   an induced migration failure leaves the original database intact. Clean
   install and uninstall on a fresh Windows user profile. Uninstall removes
   Aperture-owned hooks and keeps everything else.
3. **24-hour mixed-provider soak.** Both providers, sleep/wake, app and
   provider restarts. No sustained memory growth, no stuck confirmed status,
   no unbounded queue or inbox.
4. **Documentation.** User guide, supported-version and compatibility matrix
   from real evidence, troubleshooting, versioned release notes, and the
   diagnostic workflow for support. Pin minimum provider versions from the
   evidence gathered since Phase A.
5. **Release candidate on Mar 9.** Tag `v1.0.0-rc.1`. Mac round 3 runs
   against it. Two weeks for fixes, then the final tag.

Exit gate: the spec's Phase 4 exit, unchanged. R1-R12 and the quality
requirements pass on advertised systems; no known release-blocking
configuration loss, false identity, ingestion failure, or required-host gap.
Publish only after the evidence is recorded.

**Checkpoint 2 on Mar 26, 2027: tag v1.0.0.**

Slip rule: if Mac round 3 fails on a required row, v1.0 is not published with
a silently narrowed scope. The choice is a dated slip or an explicit
Windows-only 1.0 recorded in product-vision.md as an owner decision, with
macOS moving to 1.1.

## Recurring tracks

- **Dogfood log**, daily, from Phase A onward. It is the cheapest early
  warning the project has.
- **Mac rounds**: round 1 early December, round 2 mid-January, round 3 on the
  March release candidate. Each round gets a tagged pre-release, the current
  checklist, and a results file back. Between rounds, macOS CI covers
  compile and test only.
- **Weekly roadmap check**, ten minutes: update the goals.md status table,
  move issues between milestones if a slip rule fired, note the date.
- **Friend testing**: issues filed against the pre-release tag using the
  template in `TESTING.md`. Friends' bug-fix PRs follow the same review-pack
  rule as everything else.

## Benchmarks in one place

| Measure | Target | First measured | Must pass by |
|---|---|---|---|
| Startup to first snapshot, real history | 2 s or less | Phase A | Phase A |
| Consecutive clean dogfood workdays | 5 | Phase A | Phase A |
| False attention alerts | 1 or fewer per day | Phase A | Phase A |
| p95 persisted initial results, scale fixture | 2 s or less | Phase B | Phase D |
| p95 event to visible at 20 events/s | 1 s or less | Phase B | Phase D |
| Idle CPU, soak | under 2 percent average | Phase C | Phase D |
| Working set, soak | under 300 MiB | Phase C | Phase D |
| Soak duration without stuck state or growth | 8 h | Phase C | Phase C |
| Soak duration without stuck state or growth | 24 h | Phase E | Phase E |
| Windows live-host rows evidenced | 6 of 6 | Phase A | Phase A |
| macOS live-host rows evidenced | 6 of 6 | Phase C | Phase E |
| Settings preserved through install/repair/remove | byte-identical unrelated entries | Phase D | Phase D |

## Risks

- **Provider format drift.** Codex documents its transcript format as
  unstable, and Claude Code ships often. Each phase reserves a few days for
  adapter fixes, and sanitized fixtures are captured whenever a new shape is
  seen so a regression is a test failure rather than a surprise.
- **The review bottleneck.** The WIP cap and review packs exist for this.
  If the owner falls behind, the correct response is fewer parallel branches,
  not less verification.
- **Mac availability.** The friend's schedule can move a round by weeks. The
  Windows path never waits on it, and Phase E carries the explicit
  Windows-only fallback decision rather than an implicit one.
- **Signing lead time and cost.** Apple enrollment and a Windows certificate
  can take weeks and cost real money. Starting the paperwork in Phase D is the
  mitigation; if it still slips, v1.0 slips rather than shipping unsigned.
- **Scope creep from friends.** Feature requests go to a `v1.1` milestone by
  default. The scope boundaries in product-vision.md are not reopened by a
  bug tracker.

## Issue to milestone mapping

| Milestone | Issues |
|---|---|
| Phase A: Dogfood gate | #5, #17, #18 |
| Phase B: Organization | #7, #9 |
| Phase C: Daily-use beta | #11, #16 |
| Phase D: Hardening | #6, #8, #13, #15 |
| Phase E: Production | #4, #12 |

Issue #4 (macOS validation) stays open across all three rounds and closes in
Phase E. Issue #13 (performance) is measured in Phase B and closed in Phase D.
