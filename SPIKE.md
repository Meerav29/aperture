# Phase 0 — Right now: prove external observation for both providers

Status: next implementation phase; not completed.

This replaces the original Claude-only, four-day spike. See the authoritative
[requirements](docs/requirements.md) and [detailed specification](docs/specification.md).

## Goal

Show an externally started Claude Code session and an externally started Codex
session updating together in Aperture. Neither may be launched, resumed, or
adopted by Aperture to make the demonstration work.

## Baseline

The existing Tauri/Rust/React prototype has a Claude hook listener, an in-memory
state machine, manual/startup transcript scanning, and session cards. No Codex
collector or repository identity resolution exists. Windows host detection and
navigation are absent; macOS navigation is experimental.

During the preceding assessment, TypeScript checking, the frontend production
build, and five Rust tests passed on Windows. This is build evidence, not proof
of real host integration or macOS support. The frontend build passed after a
sandbox access restriction was resolved.

## Work in order

1. Fix the Claude installer before using real settings: reject malformed or
   unreadable configuration, preserve unrelated handlers, match ownership
   exactly, and back up and atomically replace changed files.
2. Introduce provider-qualified session identity and a normalized event boundary.
   Keep the Claude parser behind its adapter. Add provider badges and separate
   integration health for Claude Code and Codex.
3. Implement Codex lifecycle-hook collection against verified installed-version
   schemas. Isolate Codex transcript parsing. App-server attachment to unrelated
   running sessions is unproven; do not substitute Aperture-launched sessions.
4. Capture sanitized real hook and transcript fixtures for both providers.
   Record versions, configuration roots, event coverage, transcript locations,
   and process ancestry. Do not commit private prompts or unsanitized logs.
5. Demonstrate both providers together. Exercise prompts, tools, approval
   waiting, continuation, completion, interruption, and termination where
   supported. Unknown signals must produce explicit degraded/unknown state.
6. Test every OS/host row below. Distinguish documented expectations from
   observed behavior. Use sessions started both before and after Aperture.
7. Probe navigation separately: exact session, app/window focus, folder, and
   transcript reveal. Record the action actually achieved for each host.

## Compatibility evidence

Windows terminal baseline is Git Bash/PowerShell; macOS baseline is Terminal.app
with zsh (not yet tested — see below). VS Code means the provider extension
executing locally. Desktop means the local coding experience. "Host label"
is what the observer itself can determine from files alone, independent of
whether a human watching the OS could tell hosts apart.

| OS | Provider | Host | Events | Transcript discovery | Host label | Navigation | Evidence |
|---|---|---|---|---|---|---|---|
| Windows | Claude Code | Terminal | Verified | Verified | **Gap: Unknown** | Partial (open folder) | Real `claude -p` run + live dashboard, 2026-09-07. See below. |
| Windows | Claude Code | VS Code | Verified | Verified | **Gap: Unknown** | Partial (open folder) | Real historical transcript, `entrypoint:"claude-vscode"`, 2026-07-24. See below. |
| Windows | Claude Code | Desktop Code | Verified | Verified | **Gap: Unknown** | Partial (open folder) | Live dashboard screenshot of this session (Claude Desktop Code tab), 2026-09-06. See below. |
| Windows | Codex | Terminal | Verified | Verified | Verified (`codex_exec`) | Partial (open folder) | Real `codex exec` run, session `01a07a06-712b-7d91-86eb-e4affbbca013`, 2026-09-07. See below. |
| Windows | Codex | VS Code | Verified | Verified | Verified (`codex_vscode`) | Partial (open folder) | Real historical transcript, `originator:"codex_vscode"`. See below. |
| Windows | Codex | Desktop | Verified | Verified | Verified (`Codex Desktop`) | Partial (open folder) | Real historical transcript, `originator:"Codex Desktop"`. See below. |
| macOS | Claude Code | Terminal | Unverified | Unverified | Unverified | Unverified | Pending — needs a real Mac, see `docs/validation/macos-checklist.md` |
| macOS | Claude Code | VS Code | Unverified | Unverified | Unverified | Unverified | Pending — see `docs/validation/macos-checklist.md` |
| macOS | Claude Code | Desktop Code | Unverified | Unverified | Unverified | Unverified | Pending — see `docs/validation/macos-checklist.md` |
| macOS | Codex | Terminal | Unverified | Unverified | Unverified | Unverified | Pending — see `docs/validation/macos-checklist.md` |
| macOS | Codex | VS Code | Unverified | Unverified | Unverified | Unverified | Pending — see `docs/validation/macos-checklist.md` |
| macOS | Codex | Desktop | Unverified | Unverified | Unverified | Unverified | Pending — see `docs/validation/macos-checklist.md` |

**What "Verified" means here, precisely:** the passive poller correctly reads
real, unmodified session files from that host and derives a plausible
lifecycle status from them. It does not mean Aperture can tell a human which
host a Claude Code session came from (see the Host label gap below), and it
does not mean exact-window navigation works (see Navigation).

**Host label gap for Claude Code:** Codex's own transcript self-reports which
host started it (`session_meta.payload.originator`/`source`: `"Codex Desktop"`,
`"codex_vscode"`, `"codex_exec"` — all three confirmed against real files on
this machine and now used by `passive.rs`/`infer_codex_host`). Claude Code's
transcript carries no equivalent field: the only place Claude Code reports its
`entrypoint` (`cli` / `claude-vscode` / `claude-desktop`, also confirmed real)
is in the JSON it sends live to hooks at `SessionStart`, not in anything
persisted to `~/.claude/projects/*.jsonl`. This is a real, structural
limitation of passive-only observation, not a bug: closing it requires the
hook channel (`hooks_installer.rs`/`listener.rs`, present in the repo but not
wired into the desktop's passive store) to capture `entrypoint` once at
session start and merge it into the same session record by ID. That is
tracked as the next concrete step for Phase 0/1, not done in this pass.

**Interactive Codex terminal not separately confirmed:** the Terminal row
above used `codex exec` (non-interactive). The interactive `codex` TUI in a
plain terminal was not run separately to confirm it reports the same
`codex_exec` originator; treat that specific case as inferred, not verified.

**Permission-prompt attention is an explicit, currently unclosable gap for
both providers under passive observation:**
- Claude Code: real hook events (`PermissionRequest`/`Notification`) exist
  and are already implemented in `hooks_installer.rs`/`state.rs`, but are not
  wired into the desktop app's passive store in this pass.
- Codex: confirmed via Codex's own docs and `~/.codex/config.toml` that the
  external `notify` hook fires only on `agent-turn-complete`, never on
  `approval-requested` (the only other defined notify event name). Approval
  prompts otherwise only reach a local OS/TUI notification
  (`[tui] notifications = ["approval-requested"]`), which nothing outside the
  terminal process can observe. There is currently no supported, external,
  cross-host way for Aperture to see a Codex approval prompt. This is a
  provider limitation, not a missing Aperture feature — do not build a
  workaround that assumes a signal Codex does not emit.

**Navigation, honestly:** only "open the session's working directory" and
"reveal the transcript's folder" are implemented
(`open_session_folder`/`reveal_transcript` in `commands.rs`), using the OS
file manager. Exact-session focus, window/tab focus, and provider URL scheme
navigation are not implemented for any host on any OS.

Each row needs version/date, reproducible steps, fixture references, observed
results, and limitations. Builds and synthetic POSTs cannot mark a real
integration passed. Failure blocks the support claim and does not silently
remove the provider/platform from requirements.

## Exit gate

- Both externally started providers appear with distinct identities.
- Core lifecycle transitions have real evidence for all 12 combinations, with
  unsupported signals and navigation limitations explicitly recorded.
- Closed Aperture or collection failure never approves, denies, or stops work.
- Installer preservation and normalized event tests pass.
- Real fixtures establish discovery/schema assumptions for Phase 1.
- Required host observation remains a blocker until proven.

Later phases: reliable collection/recovery; repositories/worktrees/navigation;
daily-use desktop beta; production ready state. Their deliverables and gates
are in the specification. No phase is complete merely because it is documented.
