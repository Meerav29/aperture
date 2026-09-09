# Windows compatibility follow-up

Updated: 2026-09-08. This report distinguishes delivered code and automated
evidence from real host validation. **The full Windows compatibility goal is
still in progress. macOS runtime validation is pending with the owner's friend.**

## Goal

> Make Aperture's Windows observation of externally started Claude Code and
> Codex sessions trustworthy across terminal, VS Code, and desktop hosts:
> close supported attention and host-identification gaps, preserve passive
> discovery and provider settings, and document verified versus unknown
> capabilities without controlling sessions.

## What changed

- Passive discovery remains the default. Claude's real transcript `entrypoint`
  values now identify `claude-desktop`, `claude-vscode`, and `cli`. Unknown values
  stay unknown. Codex's `codex_exec` is Headless, not an interactive terminal;
  explicit CLI metadata maps to Terminal, unfamiliar metadata stays Unknown.
- Cards display the observed host. Provider health separately reports the last
  optional hook receipt, including absence of evidence and inbox read errors.
- An optional `aperture-hook` helper receives observational hooks for either
  provider. It writes metadata to an Aperture-owned file inbox, consumed by the
  same two-second observer poll. No listener, provider command, settings writer,
  approval response, or session-control operation is introduced.
- Permission/question waits are provider-qualified and survive incidental
  transcript activity. Matching tool-call IDs resolve waits only after all
  recorded IDs resolve. Missing IDs leave the wait unresolved until a lifecycle
  event establishes progress/completion. Tool names alone cannot clear a wait.
  Claude StopFailure creates error attention; Codex Interrupt clears the turn.
- Records with explicit child markers are ignored. Subagent lifecycle events
  are not subscribed. Complete child-hook discrimination still needs actual
  installed-version fixtures, especially where the provider reuses a parent ID.

## Explicit optional setup on Windows

Run from the repository root:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin aperture-hook
& ./src-tauri/target/debug/aperture-hook.exe config claude_code
& ./src-tauri/target/debug/aperture-hook.exe config codex
```

Each `config` command **prints a merge snippet only**. It does not install or
change anything. Keep the executable at the printed absolute path; for durable
use, copy it to a stable location before generating the snippets. This is a
development helper, not yet a bundled installer. Paths containing shell
metacharacters are rejected. Actual provider shell invocation must be checked
in each host/version before marking that host supported.

1. Back up the existing provider configuration. Parse it as JSON and stop if
   malformed; never replace an unreadable document with an empty one.
2. For Claude, merge into `settings.json` in `CLAUDE_CONFIG_DIR`, or normally
   `%USERPROFILE%/.claude/settings.json`. For Codex, merge into `hooks.json` in
   the actual `CODEX_HOME`, or normally `%USERPROFILE%/.codex/hooks.json`.
   Both snippets have a top-level `hooks` object. For each event key, append
   the snippet's matcher group to the existing event array. Preserve every
   existing top-level key, group, and handler. Do not replace the whole file,
   `hooks` object, or event array. Do not add an identical handler twice.
3. If Codex already uses inline `[hooks]` tables in `config.toml`, use that
   existing representation and translate each printed group/handler into the
   equivalent TOML array tables; do not add a second representation. This
   helper does not automatically merge or validate existing TOML.
4. Review/trust hooks through the provider's supported interface. Do not edit
   trust records or bypass policy. Apply provider-required reload/start steps
   yourself; Aperture does not resume or control existing sessions.
5. Keep Aperture open and exercise a permission request yourself. Check both
   the original application and dashboard, then record the exact versions and
   observed transitions. “Last hook received” is evidence of receipt, not proof
   every event is supported or that a permission dialog remains visible.

The emitted handler calls `aperture-hook collect claude_code` or `collect codex`
with a one-second provider timeout. Collect mode writes nothing to stdout or
stderr and exits successfully on malformed input, unsupported events, or an
unavailable/full inbox. It never sends decisions or context back to the model.
Input is limited to 1 MiB. A startup/IO failure or provider timeout may still be
reported by the provider; per-host timing remains to be measured.

The default inbox is `%LOCALAPPDATA%/Aperture/hook-events` on Windows and the
platform local-data directory's `Aperture/hook-events` elsewhere. To override it,
set `APERTURE_HOOK_DIR` to the same **dedicated Aperture directory** in the
environment inherited by both the dashboard and helper. Do not point it at a
provider settings or transcript directory.

The inbox stores only a version marker, provider, session ID, event name,
receipt timestamp, sanitized tool name, and optional tool-use ID. It excludes
prompts, tool inputs/results, working directories, transcript text, and errors.
Events publish via a temporary file and rename. There is a soft limit of 4,096
entries (simultaneous helpers can briefly exceed it), a 512-record drain per
poll, a 4 KiB record read limit, and a 24-hour replay limit. Ready records are
removed after consumption, including invalid/expired records. Only matching
`aperture-v1-<timestamp>-<pid>.json` files are consumed; unrelated files and
temporary files are untouched. Abandoned temporary files count toward the cap
and require cleanup in this dedicated directory. This spool is not durable
session history, does not guarantee delivery, and does not survive every crash.

### Removal

In the same provider configuration used for setup, remove only handlers whose
`command` exactly equals the emitted absolute helper command for that provider.
If a group contains other handlers, retain the group and those handlers. Delete
only groups made empty by this removal, and preserve unrelated event keys and
settings. Apply the provider's normal reload procedure yourself. Remove the
helper executable only after its handlers are no longer configured. Passive
discovery continues without hooks. Do not invoke the old dormant installer in
the source tree; it is not part of this setup workflow.

## Evidence obtained

Read-only version commands reported `codex-cli 0.153.4` and `Claude Code 2.1.263`.
These identify installed CLIs, not versions embedded in every desktop or IDE.
No provider session was launched, resumed, messaged, approved, stopped, or
otherwise controlled during this work. No provider settings were edited.

A metadata-only scan sampled the first 12 records of each non-subagent Claude
transcript on this Windows machine. It found 269 `claude-desktop`, 23
`claude-vscode`, and 9 `cli` **records**, not unique sessions. This disproves the
older blanket claim that Claude logs lack host metadata; it does not prove live
activity or approval coverage in those applications. Prompts and transcript
content were not retained in this report.

Automated tests cover provider identity, wait correlation, multiple pending
permissions, absent IDs, incidental passive activity, error attention, known
host mappings, child exclusion, input redaction, malformed/oversized/future
records, preservation of unowned inbox files, and CLI config/silent failure.
The CLI integration test starts only Aperture's helper with synthetic stdin and
a private test inbox; it does not start either provider. These checks are code
evidence, not a substitute for live compatibility records.

Validation commands:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run build
```

Final validation on 2026-09-08: `cargo test --manifest-path
src-tauri/Cargo.toml --locked` passed all **22 unit tests plus one helper CLI
integration test**, with binary and doc-test targets also passing. The parent
task separately verified `npm run build`, including TypeScript compilation.

## Remaining real validation

For each provider in Windows terminal, VS Code, and desktop, record installed
version, configuration/trust procedure, live prompt/tool/approval/deny/completion
transitions, interruptions/errors, child activity, and app-closed behavior.
Capture sanitized snapshots plus what the original host actually displayed.
In particular, verify helper command invocation and timeout behavior, whether
PermissionRequest supplies tool IDs, and whether child hooks are distinguishable.
Do not label historical discovery as a completed interactive run. Refer to
[the earlier Windows evidence](README.md) and [Phase 0 matrix](../../SPIKE.md)
for the existing historical/live coverage breakdown.

PermissionRequest means a request occurred. Another hook/policy can resolve it
without a visible dialog; missing resolution IDs can leave attention pending
until completion, and missing events remain unknown. Simultaneous question and
permission types currently share one session attention label. Process liveness,
exact-session navigation, crash recovery, durable cursors, and packaging remain
future work. Opening a folder/revealing a transcript stays the navigation fallback.

The owner has no Mac. Follow the [macOS friend checklist](macos-checklist.md)
on an actual Mac for all six combinations. A Windows build, synthetic event,
or CI compile cannot establish native window/host/runtime behavior.

## Sources checked

- [Codex hooks](https://learn.chatgpt.com/docs/hooks): lifecycle input, trust,
  settings locations, tool coverage, and no-output successful hook behavior.
  PermissionRequest is documented but its table does not promise a tool-use ID.
- [Claude Code hooks](https://code.claude.com/docs/en/hooks): lifecycle input,
  permission and error events, and subagent markers.

Documentation establishes candidate integrations. Installed-host execution is
still a separate acceptance gate.
