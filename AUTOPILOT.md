# Autopilot ran while you were away

**Window: 2026-09-20 → 2026-09-25. Read this before you touch anything else.**

While you were away, scheduled cloud routines built slices from
[docs/autopilot/queue.md](docs/autopilot/queue.md), opened PRs, and merged the
ones that passed CI and review.

**None of it is on `main`.** Everything landed on the `auto/queue` branch.
`main` has only the setup commit that added this file, CI, and the queue.

## Review this in order

1. **[docs/autopilot/decisions.md](docs/autopilot/decisions.md)** — every design
   call a routine made on your behalf because the spec did not settle it, with
   the alternative it rejected. Read it first.
2. **The `auto/queue` → `main` pull request.** One diff, all merged slices.
3. **The tracking issue** `Autopilot run log — 2026-09-20 to 09-25` — daily
   digests, halts, and anything blocked or vetoed.
4. **Any still-open PR** against `auto/queue` — rejected or held, never merged.

## What autopilot was not allowed to touch

Most of aperture's open work needs evidence from real installed hosts, which a
cloud routine cannot produce. So the routines were restricted to code with unit
test gates, and were **forbidden from writing to `docs/validation/**` or
asserting host compatibility, runtime behavior, or validation status**.

That means:

- **Nothing here advances the Windows compatibility goal.** Its status in
  [docs/goals.md](docs/goals.md) is unchanged and still open.
- **Nothing here is macOS evidence.** The friend handoff is untouched.
- A green CI run in these PRs is evidence about code, not about a host.

If you find a merged slice that claims otherwise, that is a bug in the
automation and worth telling me about — `AGENTS.md` forbids it explicitly and
the review routine was supposed to reject it.

## The thing to be skeptical about

The cloud sandbox likely could not run `cargo test` locally (Tauri needs system
libraries it may not have), so aperture slices got **less pre-flight checking
than grid-prophet's**. GitHub Actions on `windows-latest` was the real gate.
Each PR body states which checks ran locally and which did not.

## Stopping it

Routines are disabled from Claude Code (`RemoteTrigger` update,
`enabled: false`). Permanent deletion is the web UI at
<https://claude.ai/code/routines>.

## How it was set up

The design lives in the grid-prophet repo:
`grid-prophet/docs/superpowers/specs/2026-09-19-autopilot-design.md`. It covers
the branch model, the review contract, the prohibitions, and the reasoning
behind each. The same design governs grid-prophet, which ran on alternating
days.
