# Branch model

Two long-lived branches. Everything else is short-lived.

| Branch | Role | Who moves it |
|---|---|---|
| `main` | **Production.** Only tested, owner-reviewed work. What the friend clones for the macOS handoff and what a release is cut from. | Owner, by merging a reviewed `staging` → `main` PR. |
| `staging` | **Integration / latest app.** Every merged slice lands here first. It is the newest runnable version of Aperture. | Slice PRs, after CI is green. |

`staging` was created on 2026-10-04 from `main`, consolidating the previously
scattered implementation branches (see below).

## Flow

```
feature / slice branch ──PR──▶ staging ──reviewed PR──▶ main
                                  ▲
                          autopilot (auto/queue)
```

1. Branch from `staging` (`claude/<topic>` or `auto/aperture-slice-N-<topic>`).
2. Open the PR against `staging`. CI (`npm run build`, `cargo test --locked`)
   must pass.
3. Promote with a single `staging` → `main` PR when the owner has reviewed it.
   Promotion is the owner's call; a green CI run is evidence about code, not
   about a real host (see [AGENTS.md](../AGENTS.md)).
4. After promotion, `main` and `staging` are the same commit. If `main` receives
   a hotfix, merge `main` back into `staging` immediately.

Keep work in progress to the roadmap cap of two open implementation branches
([roadmap](roadmap.md)).

## What "latest" means

`staging` can be ahead of `main` by merged-but-unpromoted slices. A slice being
on `staging` does **not** mean it is validated on a real host, and it does not
move any roadmap gate. Gate status, `docs/validation/**`, and the macOS handoff
stay the owner's to record. Mac runtime validation remains pending until the
friend runs it; hand them a `main` commit, not `staging`, unless the owner says
otherwise.

## Consolidation record (2026-10-04)

Merged into `staging` on top of `main` (`614ee19`):

| Source | Content |
|---|---|
| `origin/auto/queue` | Autopilot slices 1-3: surface unloadable summary rows (#18), evict sessions idle 14 days from the live store (#17), migration test coverage (#20); queue and decision-log updates |
| `origin/claude/github-quick-wins-1m20eg` | `goals.md` / `specification.md` sync with persistence work (#22) |

Already contained in `main` (no action needed), and now stale:
`claude/aperture-external-session-observation-f82473`,
`claude/github-issues-review-47bd8b`, `claude/dev-roadmap-planning-c6e877`,
`claude/manual-validation-checklist-e7766c`, `claude/consolidate-staging-branch-7c63ae`.

Superseded by `auto/queue` (their code is identical there):
`auto/aperture-slice-1-load-summaries-visibility`,
`auto/aperture-slice-2-bound-live-store`,
`auto/aperture-slice-3-migration-test-coverage`.

These stale branches are safe to delete once the owner agrees; nothing has been
deleted as part of the consolidation.

## Autopilot

The autopilot routines still use `auto/queue` as their base branch
([queue](autopilot/queue.md)). Until the routine configuration is changed,
treat `auto/queue` as an input feeding `staging`: merge `auto/queue` into
`staging` by PR. Re-pointing the routines at `staging` is a change to the
scheduled routines, not to this repository.
