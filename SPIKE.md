# Phase 0 — Two-provider observation and compatibility

Status: Windows passive milestone demonstrated; full compatibility/attention
goal **in progress**. Reviewed 2026-09-07 after consolidation at 9df871d.

Read [product vision](docs/product-vision.md), [requirements](docs/requirements.md),
[goals](docs/goals.md), and [specification](docs/specification.md). This file
tracks evidence, not aspirations marked as complete.

## Working baseline

Aperture passively reads Claude Code and Codex session files every two seconds,
keeps bounded incremental cursors in memory, and displays both providers,
activity, attention evidence, freshness, and integration health. It provides
working-folder and transcript-folder navigation. The legacy HTTP listener and
installer are not exposed by the baseline desktop.

Real Windows evidence establishes simultaneous external two-provider observation
and unchanged provider settings. It does not establish complete permission
attention, process liveness, or all hosts. See [Windows records](docs/validation/README.md).

At consolidation, the frontend build/TypeScript and 14 Rust tests passed.
These historical counts are superseded by the current
[Windows follow-up](docs/validation/windows-compatibility-followup.md).

## Current goal

Close Windows host-identification and supported attention gaps while preserving
passive discovery and external session ownership. Optional observational hooks
may enrich fields missing from files, using explicit configuration and no agent
decisions. No launching, resuming, prompting, stopping, or approval control.

The owner has no Mac. The friend's [Mac handoff](docs/validation/macos-checklist.md)
is a separate pending validation task. Windows work can proceed; complete
cross-platform Phase 0 and production claims cannot precede that evidence.

## Evidence matrix

This baseline separates discovery from full live lifecycle validation.
“Historical” does not mean a live attention test passed. See the follow-up report
for additional evidence produced by the current implementation agent.

| OS | Provider | Host | Discovery evidence at consolidation | Full live lifecycle/attention | Navigation |
|---|---|---|---|---|---|
| Windows | Claude Code | Interactive terminal | Non-interactive run recorded; interactive case not separately evidenced | Pending | Folder fallback implemented |
| Windows | Claude Code | VS Code | Historical record reported | Pending | Folder fallback implemented |
| Windows | Claude Code | Desktop Code | Live dashboard evidence reported | Full attention pending | Folder fallback implemented |
| Windows | Codex | Interactive terminal | codex exec record reported; interactive TUI not separately evidenced | Pending | Folder fallback implemented |
| Windows | Codex | VS Code | Historical record reported | Pending | Folder fallback implemented |
| Windows | Codex | Desktop | Historical metadata and Windows two-provider trace; see record provenance | Full attention pending | Folder fallback implemented |
| macOS | Claude Code | Interactive terminal | Pending friend validation | Pending | Pending real Finder check |
| macOS | Claude Code | VS Code | Pending friend validation | Pending | Pending real Finder check |
| macOS | Claude Code | Desktop Code | Pending friend validation | Pending | Pending real Finder check |
| macOS | Codex | Interactive terminal | Pending friend validation | Pending | Pending real Finder check |
| macOS | Codex | VS Code | Pending friend validation | Pending | Pending real Finder check |
| macOS | Codex | Desktop | Pending friend validation | Pending | Pending real Finder check |

No exact-session/window/tab navigation has been verified by this matrix. Folder
fallback implementation and helper unit tests are not evidence of native Mac
navigation. A headless/non-interactive run does not substitute for interactive
CLI validation.

## Corrections to earlier assumptions

- The earlier statement that Claude transcripts never expose host metadata was
  too broad. The current follow-up found entrypoint fields in real local
  records. Parse only evidenced values and retain unknown for missing/unrecognized
  metadata; do not treat an arbitrary string as a known host.
- The earlier claim that Codex has no external permission-request channel
  conflated legacy notify behavior with the lifecycle hooks interface.
  [Official Codex hooks](https://learn.chatgpt.com/docs/hooks) describe
  PermissionRequest. Its availability and coverage in each installed host/version
  still require real tests. This is an unresolved integration check, not proof
  that full cross-host approval observation is impossible.
- Recorded file-based activity is useful evidence but cannot establish every
  lifecycle transition, concurrent question resolution, or current process liveness.
- Neither the file-reader approach nor hook transport is the product itself.
  Keep passive discovery; enrich only missing signals with observational behavior.

## Immediate implementation checklist

1. Preserve the passive Windows baseline and sanitized fixtures.
2. Improve provider host mapping from real metadata without process-name guesses.
3. Add optional sanitized local hook enrichment; print merge instructions rather
   than automatically changing settings. Do not enable the unsafe legacy installer.
4. Merge by provider/session with ordered attention handling, stale/history rules,
   and child-event isolation. Test malformed/oversized events and app-off behavior.
5. Build/test and capture any available real external activity without controlling
   user sessions. List manual host/permission tests that remain pending.
6. Give the Mac tester the exact committed revision and reproducible checklist.
7. Update evidence and capability labels from observed outcomes, not task status.

## Exit gate for complete Phase 0

- Both externally started providers remain distinguishable and observable.
- Required lifecycle/attention checks have real evidence for all 12 combinations,
  with documented versions and known missing capabilities.
- Missing signals remain explicit unknown; failures do not change agent decisions.
- Configuration preservation and event-normalization regression checks pass.
- Navigation fallback capabilities are labeled accurately and tested per OS.
- Mac rows cannot pass on Windows or from CI builds alone.

Documentation and the Windows subgoal can finish while this full-phase gate
remains open. Subsequent phases cover durable recovery, Git/worktree organization,
daily-use desktop beta, and production ready state.
