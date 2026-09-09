# Aperture product vision

Status: enduring product direction, established from the project conversation and
confirmed by the owner through September 7, 2026. This is the central source of
truth for **what Aperture is intended to become**, not a completion report.

## The product we are building

Aperture is a Windows and macOS desktop dashboard for a developer running
externally created **Claude Code and Codex sessions** across terminals, VS Code,
and the providers' desktop coding apps. Both providers are first-class
requirements. The user can keep Aperture on another screen while working in
those applications, across several repositories and Git worktrees.

The dashboard answers: Which sessions are working? What are they doing? Which
ones need my attention? Which repository and worktree do they belong to? How do
I return to the right place to act?

The user keeps their existing agent workflow. Aperture does not require sessions
to originate inside it, and it does not take ownership of their execution.
"Agent manager" means **monitoring, organization, and navigation** in this
release, rather than orchestration or remote control.

## User-confirmed decisions

- Deliver a graphical desktop application for Windows and macOS, not a
  terminal-based product. A diagnostic CLI may support development.
- Observe both Claude Code and Codex, including sessions started before the
  dashboard where discoverable evidence exists.
- Cover local sessions in terminals, VS Code, and provider desktop coding apps.
  Provider support must not depend on using a single preferred host.
- Organize multiple sessions across multiple repositories and worktrees. Two
  sessions in the same checkout remain distinct, and linked worktrees belong
  together under their repository.
- Show activity and attention, then help the user return to the original
  environment. Launching, resuming, prompting, stopping, and answering agent
  approvals are outside this release's monitor-and-jump scope.
- Develop through explicit, verifiable goals and implementation phases ending
  in a **production ready state**. A prototype demonstration does not establish
  production readiness or compatibility across all hosts.
- The owner currently has no Mac. A friend with a Mac can clone the repository,
  run Aperture with locally signed-in providers, and perform macOS validation.
  This constraint changes validation ownership, not the macOS product target.

## Shared language

| Concept | Meaning |
|---|---|
| Provider | The agent product/integration: Claude Code or Codex |
| Model | The model used inside a provider; it does not identify the provider or host |
| Host | The terminal, VS Code, or desktop coding application driving a session |
| Session | A provider-native unit of agent work, identified independently of its folder and qualified by provider |
| Repository | The local Git repository that groups its main checkout and linked worktrees; separate clones are distinct local repositories |
| Worktree | A specific checkout and working directory within that repository |
| Activity | What the latest evidence says the session was doing |
| Attention | Evidence that user action is needed, such as approval, explicit input, or an error |
| Observation | How and when information was obtained; separate from whether a process is alive |

Do not use model names to infer provider, a repository path as a session ID, or
a completed turn as proof that the session process ended or its objective was
achieved. Exact names and field representations belong in the specification.

## Product principles and engineering defaults

These defaults support the confirmed workflow. They are implementation choices
and product guardrails, not claims that every behavior is already implemented.

- **Trust the evidence.** Distinguish current observations, stale information,
  historical discovery, and unknowns. Silence is not proof of idle, ended, or no
  pending approval. Explain partial integration health independently for each
  provider so a healthy collector does not imply complete signal coverage.
- **Navigate honestly.** Label exact-session navigation, app/window focus,
  opening a folder, and revealing a transcript according to what each action
  actually achieves. Use verified fallbacks when exact targeting is unavailable.
- **Remain observational.** Provider work continues if Aperture is closed or
  unavailable. Optional integrations must preserve unrelated settings, remain
  explicitly enabled, and never supply permission decisions or alter agent work.
- **Keep data local.** Process session evidence locally by default; do not upload
  prompts, transcripts, reasoning, or activity. Minimize collected/displayed
  content and require explicit, sanitized diagnostic export.
- **Prefer the existing desktop foundation.** Retain Tauri, Rust, React, and
  TypeScript unless evidence justifies changing them. Provider-specific adapters
  feed a shared session model. Passive discovery is the working baseline;
  additional observational channels are justified by verified capability gaps.
- **Keep the architecture replaceable.** A hook API, transcript format, polling
  interval, database, or process heuristic is not the product vision. Revise the
  implementation when provider evidence changes without silently reducing scope.

Current boundaries exclude embedded terminals, replacement chat, agent/team
orchestration, Git/worktree mutation, other providers, ordinary Claude Chat or
Cowork, USD cost estimates, account sync, and team dashboards. WSL, containers,
SSH/cloud execution, and cross-machine aggregation are outside the current
local-machine release. These boundaries are engineering scope defaults carried
from the requirements; expand them only through an explicit scope decision.

## Success and evidence

The representative success scenario is a developer operating both providers in
several host apps, across two or more repositories and several worktrees, while
Aperture stays on the other screen. Sessions are distinguishable and organized;
activity and attention are understandable; navigation returns the user through
the best verified action. Restart and sleep/wake preserve useful context without
making unsupported live-status claims. This must work on Windows and macOS.

The baseline inspected for this document has a two-provider passive observer,
shared cards and integration health, and folder/transcript-folder navigation.
It does not establish full attention coverage, durable recovery, repository
grouping, exact-session navigation, or all-platform compatibility. Consult the
[validation records](validation/README.md) and current code for the latest
implementation evidence; this paragraph is a dated baseline, not a live ledger.

Real validation must identify the build/commit, OS, provider and host versions,
procedure, observed outcome, and limitations. Recorded historical files,
sanitized fixtures, automated tests, and real live host runs prove different
things. Missing evidence remains pending even if a conversation called a goal
complete. A headless run does not substitute for interactive terminal coverage.

All six macOS provider/host combinations require real Mac runtime evidence.
macOS CI builds and tests can reduce platform risk but cannot establish signed-in
desktop/IDE/terminal behavior or window navigation. Until the friend or another
authorized Mac tester runs the [macOS checklist](validation/macos-checklist.md),
those checks stay pending and the compatibility goal stays open. Continue
independent Windows work and test preparation without waiving the Mac gate.

## Source-of-truth order and future-session instructions

1. New explicit owner decisions govern product intent; update this file when
   they change direction. Do not let an implementation shortcut become an
   unacknowledged product decision.
2. This vision defines the enduring purpose and boundaries.
3. [Requirements](requirements.md) define measurable product acceptance;
   [specification](specification.md) defines implementation contracts and phases.
   Keep them aligned with the vision and identify changes explicitly.
4. [SPIKE.md](../SPIKE.md) tracks immediate compatibility work;
   [validation](validation/README.md), current source, and reproducible results
   establish what is actually implemented and verified. Intent documents and
   chat assertions cannot override contrary implementation evidence.
5. [ADRs](adr/0001-observer-first.md) explain architectural decisions, and the
   [README](../README.md) provides the entry point and current run instructions.

At the beginning of a future development session, read this file, requirements,
the relevant specification phase, and current validation gaps; inspect the
checkout before selecting work. Define each goal by an observable outcome,
required implementation, tests, real-workflow evidence, and a completion audit.
Report remaining gaps precisely. Preserve both providers, external ownership,
and both operating systems when revising a plan. Update affected documentation
when code or evidence changes, and never mark a phase complete from a narrower
milestone's results.
