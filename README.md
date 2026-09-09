# Aperture

One desktop dashboard for externally started Claude Code and Codex sessions,
across terminals, VS Code, and their desktop apps on Windows and macOS.

**Current implementation:** a read-only Windows observer for Claude Code and
Codex session files. Both providers update in one dashboard with identity,
activity, attention evidence, freshness, and independent integration health.
See [Windows validation](docs/validation/README.md) for measured coverage and
limitations. Folder/transcript-folder navigation is implemented; exact-session
focus and repository/worktree identity remain planned. macOS runtime validation
is pending a friend with a Mac; Windows evidence does not establish Mac support.

## Product and implementation documents

- [Product vision](docs/product-vision.md): central source of truth for product intent; read first.
- [Development goals](docs/goals.md): current assignment, audited status, and Mac handoff.
- [Requirements](docs/requirements.md): product scope and acceptance criteria.
- [Detailed specification](docs/specification.md): architecture, interfaces,
  behavior, verification, and phases from right now through production readiness.
- [Immediate spike](SPIKE.md): Phase 0 checklist and compatibility evidence.
- [Observer decision](docs/adr/0001-observer-first.md): why sessions remain externally owned.
- [macOS handoff](docs/validation/macos-checklist.md): clone/build/manual validation instructions.

## Run it

Prerequisites: Rust stable, Node 20+, and the Tauri v2 platform deps
(https://v2.tauri.app/start/prerequisites/).

```
npm install
npm run tauri dev
```

Passive mode requires no integration installation or provider settings changes. Aperture reads
`~/.claude/projects` and `~/.codex/sessions` every two seconds. Set
`CLAUDE_CONFIG_DIR` or `CODEX_HOME` in Aperture's environment for non-default
provider roots. Start and operate all sessions externally.

The legacy hook installer and HTTP listener are not registered by the desktop.
The app exposes folder/transcript-folder navigation, not session control.

## Optional attention hooks (Windows development build)

File observation cannot expose every approval prompt. An optional helper records
sanitized lifecycle events in a local Aperture inbox. It never edits provider
configuration or returns an approval/denial decision.

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin aperture-hook
./src-tauri/target/debug/aperture-hook.exe config claude_code
./src-tauri/target/debug/aperture-hook.exe config codex
```

These commands **print JSON snippets for manual merging**, not replacement
settings. Use the provider's supported hook configuration for the installed
version and preserve unrelated entries; do not redirect this output over an
existing settings/config file. See the [Windows follow-up](docs/validation/windows-compatibility-followup.md)
for exact setup/removal, supported events, and remaining real-host checks.

The default Windows inbox is `%LOCALAPPDATA%/Aperture/hook-events`. If overriding
`APERTURE_HOOK_DIR`, use the same absolute directory in the helper's and app's
environments. A configured hook is not proof its host/version actually emits
every event. Passive mode remains available when hooks are absent.
## Layout

- `src-tauri/src/observer/passive.rs`: read-only discovery, incremental JSONL readers, provider adapters, normalized updates.
- `src-tauri/src/observer/model.rs` and `state.rs`: snapshot contract, sorting, freshness.
- `src-tauri/src/commands.rs`: snapshots, rescan, and folder/transcript-folder navigation.
- `src/features/sessions`: mixed-provider cards and attention display.
- `src-tauri/src/bin/observe.rs`: diagnostic CLI using the same observer as the desktop.
- `src-tauri/src/bin/aperture-hook.rs`: optional observational helper and configuration output.
- `src-tauri/tests/fixtures`: sanitized Windows session records.

## Useful checks

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo run --manifest-path src-tauri/Cargo.toml --bin observe -- 10
```

The diagnostic argument is the number of polls, two seconds apart. It prints
summaries only. Closing Aperture or the diagnostic stops observation and has no
connection to either provider process. Cached summaries/cursors are in memory;
restart backfills history without claiming that old sessions are live.
