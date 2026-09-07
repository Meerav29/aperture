# Aperture product requirements

Status: agreed product direction; implementation incomplete.

This document supersedes the scope of the original Claude-only spike.
[SPIKE.md](../SPIKE.md) now describes the immediate phase; the
[specification](specification.md) defines implementation behavior and release gates.

## 1. Product and audience

Aperture is a Windows and macOS desktop dashboard for a developer running
multiple Claude Code and Codex sessions across a handful of local repositories
and Git worktrees. Sessions are created outside Aperture, in terminals, VS Code,
and provider desktop applications. The user leaves Aperture on a second screen
to see what is working, what needs attention, and where to return.

Both providers are required for the initial useful release. A Claude-only
release does not satisfy this definition. Agent provider, model, host
application, repository, worktree, and session are separate concepts.

## 2. Confirmed scope and boundaries

- Monitor and jump: show session state and activity, then return the user to
  the original environment for interaction.
- Local execution on Windows and macOS, with an independent dashboard on each
  machine. Cross-machine aggregation is not included.
- Claude Code CLI, local VS Code extension, and local desktop Code sessions;
  Codex CLI, local VS Code extension, and local desktop sessions.
- Observe existing worktrees; do not create, switch, merge, or remove them.
- Discover sessions started before Aperture where local evidence exists;
  historical records alone do not prove a session is currently running.

Out of scope through the production milestone: launching or resuming agent work,
embedded terminals, replacement chat UI, sending prompts, approval decisions,
stopping sessions, Git writes, WSL, containers, SSH/cloud execution, account sync,
team dashboards, other providers, and USD cost estimates. Ordinary Claude
Chat/Cowork are outside Claude Code coverage. Subagents are shown as children
when supported; agent-team orchestration is excluded.

## 3. Functional requirements

| ID | Requirement | Acceptance |
|---|---|---|
| R1 | Observe both providers without owning sessions | Externally started Claude and Codex sessions update together without launch, resume, or adoption by Aperture |
| R2 | Support target OS/host combinations | All 12 combinations in the spike matrix have real observation evidence and documented versions |
| R3 | Show trustworthy state | Working, idle, approval/input waiting, error, ended, and unknown are distinct; freshness and historical provenance are visible |
| R4 | Show useful activity | Cards show provider, host when known, title/fallback, latest activity, branch/worktree, and last observation time |
| R5 | Direct attention | Approval requests, explicit input requests, and errors appear in attention; ordinary idle sessions do not continuously alert |
| R6 | Organize by repository and worktree | Linked worktrees group under one local repository; sessions in the same worktree remain distinct; non-Git directories work |
| R7 | Navigate honestly | Offer exact-session navigation only when verified; otherwise label app/window focus, open folder, or reveal transcript explicitly |
| R8 | Recover across restarts | Retain metadata/history, hidden entries, and settings; restored status stays unverified until reconciled |
| R9 | Discover and refresh history | Initial discovery, incremental updates, and rescan work for both providers with partial-failure diagnostics |
| R10 | Configure integrations safely | Install/repair/remove each provider independently without deleting unrelated settings or changing agent decisions |
| R11 | Support daily desktop use | Search/filter, details, tray/menu-bar operation, optional notifications, and keyboard-accessible controls |
| R12 | Explain integration health | Distinguish configuration from receiving events, disabled policy, missing provider, unavailable collector, and unsupported version |

## 4. Quality requirements

- Local storage and processing by default; no prompt, transcript, or activity
  uploads. Diagnostic export is explicit and sanitized.
- Collection is observational and bounded: dashboard unavailability must not
  block agent work or return approval/denial decisions.
- Validate input, protect local ingestion, preserve configuration, and constrain
  navigation. Never execute commands derived from event text.
- Target load: 50 active sessions and 10,000 historical summaries. Live events
  reach the visible dashboard within one second at p95; persisted initial results
  appear within two seconds at p95 on documented release test machines.
- UI stays responsive during backfill; history is paginated or virtualized.
  No full transcript scan or unbounded snapshot emission per event.
- Installed Windows/macOS builds pass upgrade, restart, sleep/wake, and
  uninstall checks. Distribution is signed, with macOS notarization.

These are acceptance targets, not current prototype measurements.

## 5. Success scenario

The developer starts Claude in a terminal, Codex in its desktop app, and both
providers in VS Code across two repositories and several worktrees. Aperture
discovers and distinguishes the sessions, groups worktrees correctly, shows
activity, and directs attention when approval is needed. The user returns via
the best supported navigation action. Aperture restarts or sleeps without losing
history or falsely claiming old work is active. Verify on Windows and macOS.

## 6. Release interpretation

The phases end at **production ready state**, defined in the specification.
Missing observation for a required host is a release blocker, not permission
to reduce scope silently. Exact-tab focus is capability-dependent; every host
must have a working, accurately labeled navigation fallback. Pin supported OS
versions/architectures and minimum provider versions from Phase 0 evidence
before beta distribution.
