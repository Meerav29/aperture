# Development goals and current status

Updated: 2026-09-16. Read [product vision](product-vision.md) before choosing a
goal. [Requirements](requirements.md) define acceptance; [specification](specification.md)
defines the target design; [validation](validation/README.md) contains evidence.
The [roadmap](roadmap.md) sequences the specification phases onto a calendar
(Phase A dogfood gate through Phase E production, Sep 2026 to Mar 2027) and
maps each open issue to a GitHub milestone. Pick work from the current roadmap
phase; update the table below and the roadmap together when a gate moves.

## Completed and incomplete work

| Goal | Audited status | Evidence / remaining work |
|---|---|---|
| Product requirements and phased specification | Written; maintained as implementation evolves | Requirements/specification, not proof of runtime completion |
| Windows two-provider observation | Core passive milestone demonstrated | Real two-provider file growth and preserved settings in validation records; permission coverage remains limited |
| All-host Windows/macOS compatibility and attention | In progress, not complete | Windows historical/live evidence is mixed; macOS has no execution evidence |
| Durable collection/recovery | Implemented | SQLite summaries/cursors, debounced watcher, restart/sleep-wake reconciliation; git identity and IPC/SessionKey revision remain planned |
| Repository/worktree dashboard | Planned | Directory grouping is not repository identity |
| Daily-use desktop beta | Planned | See Phase 3 gates |
| Production ready state | Planned | Requires all advertised platform release gates |

## Current dispatched implementation goal

> Make Aperture's Windows observation of externally started Claude Code and
> Codex sessions trustworthy across terminal, VS Code, and desktop hosts:
> close supported attention and host-identification gaps, preserve passive
> discovery and provider settings, and document verified versus unknown
> capabilities without controlling sessions.

Deliverables: provider-specific observational enrichment where needed, safe
explicit integration setup, meaningful regression tests, and a dated
[Windows follow-up report](validation/windows-compatibility-followup.md).
Do not count synthetic tests, historical discovery, or event-name existence as
proof of a real running host's complete lifecycle coverage.

Acceptance:

- Passive discovery continues without requiring hooks or provider settings edits.
- Supported host and attention signals merge into the right provider/session.
- Setup preserves unrelated settings, rejects malformed input, and is reversible.
- Aperture emits no decisions that approve/deny tools and does not start/resume,
  message, stop, or otherwise control sessions.
- Build and relevant tests pass; available real evidence is recorded with versions.
- Unavailable real-host/approval checks remain explicit pending work.

This is a bounded Windows goal. Its completion does not complete the full
cross-platform phase. Consult the follow-up report for actual implementation
and validation status; dispatch alone is not completion.

Implementation update, 2026-09-08: host metadata mapping and the optional local
hook helper are implemented with automated regression coverage. Setup remains
manual merge-only; no provider settings were edited by this task. Real installed
host approval/request-resolution checks remain pending, so the full Windows
compatibility goal is still open. See the follow-up report for final test counts
and exact setup/removal instructions.

## macOS handoff goal — awaiting a friend with a Mac

The project owner has no Mac. A friend can clone the same revision, build and
run Aperture, and validate both providers in Terminal, VS Code, and desktop apps.
Use the [macOS checklist](validation/macos-checklist.md), including the evidence
template and interactive CLI tests. Record failures as failures or pending,
not as unsupported requirements removed from scope.

A macOS CI build/test job is an optional future build check. It does not provide
signed-in desktop app sessions, real approval interaction, or verified Finder/
window behavior. No CI job or Mac runtime result is claimed in this handoff.
A remote Mac with interactive access could also perform these tests, but no
such machine is configured here. The friend's Mac is the current practical path.

Windows work may continue while this handoff is pending; cross-platform
compatibility and production readiness stay open until the Mac evidence exists.

## Documentation work delivered

Product vision and future-session instructions are written. README, requirements,
specification, ADR, spike, validation history, Mac handoff, and icon documentation
have been aligned with the audited passive baseline and optional enrichment.
Historical evidence is retained with dated corrections. Implementation reports
distinguish completed code, automated tests, and pending manual evidence.

## Working discipline

One bounded outcome per implementation goal, followed by code review, automated
checks, real workflow evidence where applicable, and an explicit completion audit.
Use one development branch/worktree per active implementation stream; do not
mark a goal done because its PR merged. Preserve unfinished work before syncing.

Consolidation audit on 2026-09-07 synchronized main to origin/main at 9df871d.
The prior dirty checkout was saved as stash@{0}, named
aperture-pre-consolidation-2026-09-07. This is a historical record: stash indices
and branch counts can change, so inspect Git before cleanup. The merged Claude
branch/worktree was retained; no deletion was authorized by this record.
