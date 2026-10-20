# Autopilot: window 2, Oct 20-26, 2026

**Read this before you touch anything else.** Scheduled cloud routines are building slices 7-11 from [docs/autopilot/queue.md](docs/autopilot/queue.md) while Meerav is travelling. They open PRs against `auto/queue`, and a separate review routine merges the ones that pass CI and review. **`main` does not move.**

## What's queued (one a day, about 9am ET; review about 2pm ET)

| Day | Slice |
|---|---|
| Wed Oct 21 | 7: #99 hook `collect` wall-clock timeout |
| Thu Oct 22 | 8: #95 oversized screenshot records no longer mark Claude Code degraded |
| Fri Oct 23 | 9: #51 / #52 / #53 reducer correctness |
| Sat Oct 24 | 10: #45 IntegrationHealth test coverage |
| Sun Oct 25 | 11: #89 bundle IBM Plex locally |

Log: issue [#101](https://github.com/Meerav29/aperture/issues/101).

## What autopilot was not allowed to touch

Same rules as window 1. They're binding, and the review routine rejects any PR that breaks them:

- **No writes to `docs/validation/**`, `docs/roadmap.md` or `docs/goals.md`.**
- **No claims about host compatibility, runtime behavior, or validation status.** A green CI run is evidence about code, not about a host.
- **The Phase A gate is exactly where you left it.** #87 (record passed, missed or moved) is yours, and you chose to keep it. Window 2 is gate-neutral fixes only.
- **No #9 or #7.** The Phase B contract and Git identity work need your sign-off (#100).
- **No `.github/workflows/` edits**, which is why #76 (fmt/clippy in CI) isn't queued.

## Review in this order when you're back

1. [docs/autopilot/decisions.md](docs/autopilot/decisions.md), entries from 2026-10-20 on. #99's timeout budget is a *provisional* pick there, waiting for your real hook-latency numbers.
2. Issue #101: digests, halts, rejections.
3. Any PR still open against `auto/queue`.
4. The `auto/queue` → `main` diff. Slice-11's review pack includes "run with the network off". Only you can check that.

## The thing to be skeptical about

The cloud sandbox may not be able to run `cargo test` (window 1 hit a permission block once). GitHub Actions on `windows-latest` is the real gate, and each PR body says which checks ran where.

## If you're asking a cloud session to do something here

- Read [AGENTS.md](AGENTS.md). It overrides everything else on what may be claimed.
- To continue autopilot work, branch from `auto/queue`. If a slice's PR is open, comment on it, or comment `HOLD` first if you mean to take it over.
- Linux deps for `cargo test`: `apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev`.

## Stopping it

Comment `HOLD` on a PR to stop that slice. Routines: `RemoteTrigger` update with `enabled: false` (IDs are in Meerav's Claude memory), or <https://claude.ai/code/routines>.

---

Window 1 (Sep 20 → Oct 19) built slices 1-6 (#18, #17, #20, #30, #43, #50), all merged and on `main` via #83 and later merges. Its log is issue #25. Design and review contract: `grid-prophet/docs/superpowers/specs/2026-09-19-autopilot-design.md`.
