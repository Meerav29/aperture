# Autopilot queue — aperture

Design: `grid-prophet/docs/superpowers/specs/2026-09-19-autopilot-design.md`
Rotation: Mon / Wed / Fri. Base branch: `auto/queue`. Window: 2026-09-20 → 09-25.

## Read this before picking up a slice

Read [AGENTS.md](../../AGENTS.md) and [docs/product-vision.md](../product-vision.md)
first. They override this file on approach and on what may be claimed.

**Most of aperture's open work is not automatable and is deliberately not in this
queue.** The remaining Windows and macOS items need evidence from real installed
hosts. `docs/requirements.md` and `AGENTS.md` are explicit that a build, a
synthetic event, or a CI compile cannot establish runtime behavior, and that
missing evidence must never be recorded as success.

The slices below were chosen because each has a real unit-test gate and none
requires host evidence. Binding for every slice:

- **Never write to `docs/validation/**`.** That lane belongs to the owner.
- **Never assert host compatibility, runtime behavior, or validation status.**
  Passing tests are evidence about code, not about a real host.
- Never launch, resume, message, or control a real agent session to make a test
  pass. `AGENTS.md` forbids it.
- Passive discovery must keep working without hooks or provider settings edits.
- If a criterion cannot be checked, say so in the PR body and let the slice be
  rejected. Do not soften the criterion.

Status values: `todo` → `in-progress` → `in-review` → `merged`.
Terminal: `held` (owner vetoed), `blocked` (review rejected).

---

## slice-1 — Git identity service and repository/worktree grouping

Status: todo
Requirement: R6. Spec: `docs/specification.md` §"Git identity" and §4.
Currently listed as "Planned" in [docs/goals.md](../goals.md); the UI groups by
`cwd` and `repo_root` is unresolved.

Acceptance:

- [ ] Read-only Git queries run with **argument arrays** (not shell strings),
      **outside the reducer lock**, with a **two-second timeout**.
- [ ] Repository identity is the canonical absolute Git **common directory**;
      worktree identity is the **checkout root**. Linked worktrees group under
      one repository.
- [ ] **Separate clones remain separate repositories even when they share a
      remote URL.** A test covers this — it is the case a remote-URL-based
      implementation gets wrong.
- [ ] Branch resolves, and detached HEAD is shown as detached rather than as a
      branch name.
- [ ] Identity refreshes after a cwd change and on manual rescan.
- [ ] Native display paths are preserved. Paths are **not** lowercased on
      case-sensitive volumes, while identity comparison stays filesystem-aware.
- [ ] None of these break collection: subdirectories, paths with spaces, Unicode
      paths, linked-worktree `.git` files, deleted worktrees, non-Git folders,
      Git unavailable on PATH. Each has a test.
- [ ] Sessions in the same checkout remain distinct sessions.
- [ ] `cargo test` and `npm run build` pass.

Out of scope: navigation actions and their fallback labels; macOS path
behavior beyond what unit tests can cover.

---

## slice-2 — Process liveness signal

Status: todo
Spec: `docs/specification.md` §2 (status table), and the "Remaining real
validation" section of
[docs/validation/windows-compatibility-followup.md](../validation/windows-compatibility-followup.md),
which names process liveness as future work. **Read that file; do not edit it.**

The point of this slice is an honest signal. The spec is blunt that file scans
do not prove liveness, and `AGENTS.md` requires distinguishing recent
observation from process liveness from unknown.

Acceptance:

- [ ] Liveness is a distinct signal from recency of file activity. A session with
      fresh file writes and a dead process does not report as live.
- [ ] A **stale or reused PID** never yields a live claim. A test covers PID
      reuse explicitly — the spec's risk table names it.
- [ ] When liveness cannot be determined, the state is **unknown**, and unknown
      is distinct from both live and ended in the model and in the UI.
- [ ] Checks are bounded and do not block the reducer or collection.
- [ ] No session is started, resumed, signalled, or otherwise controlled. Read
      only.
- [ ] `cargo test` and `npm run build` pass.

Out of scope: claiming verified liveness coverage for any real host. Implement
the signal; the owner validates it.

---

## slice-3 — Search, filters, and details pane

Status: todo
Requirement: R11 (partial). Spec: `docs/specification.md` §4.

Acceptance:

- [ ] Default view groups **repository → worktree → session**, with the unified
      attention area above the groups and both providers in it.
- [ ] Controls exist for provider, status, repository, host, and text search,
      plus active / history / hidden views.
- [ ] Details include a bounded timeline, source freshness, host evidence,
      counts where known, children, and the available navigation actions.
- [ ] Title fallback is folder name plus short native ID. A provider-generated
      title is **never** used as a unique identity.
- [ ] Empty states distinguish *no sessions* from *disconnected integrations*
      from *scan in progress*. A test covers all three.
- [ ] The UI stays responsive during backfill; history is paginated or
      virtualized, with no unbounded render.
- [ ] `cargo test` and `npm run build` pass.

Out of scope: tray / menu-bar operation, notifications, and full keyboard
accessibility — the rest of R11, left for a later slice.
