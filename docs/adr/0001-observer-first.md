# ADR 0001: Observe externally owned agent sessions

Status: accepted 2026-09-04; amended after the requirements review to include
Codex alongside Claude Code and retain monitor-and-jump as the release scope.

## Decision

Aperture observes sessions started in other applications. Claude Code and Codex
are equal first-release providers. Provider adapters translate lifecycle events
and historical metadata into a shared model; Git identity, persistence,
attention handling, and the dashboard are shared services.

The product is a Tauri desktop app for Windows and macOS. Users start and
interact with agents in their preferred terminal, VS Code, or local provider
desktop app. Aperture reports activity and offers verified navigation back.

Spawning sessions, creating worktrees, sending prompts, stopping agents, and
answering approvals are outside the current production release. They are not
prerequisites or promised follow-on phases. Persistence and transcript watching
belong to the observer and do not depend on a future spawner.

## Rationale and consequences

- Observing external sessions preserves the user's existing workflow.
- Provider identity is independent of host application and model name.
- Passive session-file observation is the implemented baseline for both providers.
  Optional observational hooks enrich attention and metadata missing from files;
  enabling them must remain explicit and must never change agent decisions.
- No universal app-server attachment, transcript schema, process ancestry, or
  exact-tab navigation is assumed. Compatibility must be demonstrated.
- Unknown or stale evidence is displayed honestly; missing data is not idle.
- Integration failure must not change agent behavior or permissions.
- The earlier market-wide claim that no other tool observes external sessions
  is withdrawn; this decision does not depend on an unverified competitor claim.

## Evidence and follow-up

The original desktop-hook, transcript, Windows command-hook, and navigation
questions remain empirical checks. Track both providers and OSes in the
[Phase 0 matrix](../../SPIKE.md), not as unchecked assumptions here.

See [requirements](../requirements.md) and [specification](../specification.md)
for the product contract, integration sources, and release gates.

The [product vision](../product-vision.md) is the central intent reference.
The owner has no Mac; a friend will run the [macOS handoff](../validation/macos-checklist.md).
Windows work can proceed, but pending Mac runtime validation is not waived.
