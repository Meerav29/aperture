# Aperture

One desktop dashboard for externally started Claude Code and Codex sessions,
across terminals, VS Code, and their desktop apps on Windows and macOS.

**Current implementation:** a read-only Windows observer for Claude Code and
Codex session files. Both providers update in one dashboard with identity,
activity, attention evidence, freshness, and independent integration health.
See [Windows validation](docs/validation/README.md) for measured coverage and
limitations. Worktree grouping and cross-platform navigation remain planned.

## Product and implementation documents

- [Requirements](docs/requirements.md): product scope and acceptance criteria.
- [Detailed specification](docs/specification.md): architecture, interfaces,
  behavior, verification, and phases from right now through production readiness.
- [Immediate spike](SPIKE.md): Phase 0 checklist and compatibility evidence.
- [Observer decision](docs/adr/0001-observer-first.md): why sessions remain externally owned.

## Run it

Prerequisites: Rust stable, Node 20+, and the Tauri v2 platform deps
(https://v2.tauri.app/start/prerequisites/).

```
npm install
npm run tauri dev
```

No installation or provider settings changes are required. Aperture reads
`~/.claude/projects` and `~/.codex/sessions` every two seconds. Set
`CLAUDE_CONFIG_DIR` or `CODEX_HOME` in Aperture's environment for non-default
provider roots. Start and operate all sessions externally.

The old hook installer, listener, and navigation commands are not registered
by the desktop. No session launch/resume/control command is exposed.
## Layout

- `src-tauri/src/observer/passive.rs`: read-only discovery, incremental JSONL readers, provider adapters, normalized updates.
- `src-tauri/src/observer/model.rs` and `state.rs`: snapshot contract, sorting, freshness.
- `src-tauri/src/commands.rs`: snapshot and rescan only.
- `src/features/sessions`: mixed-provider cards and attention display.
- `src-tauri/src/bin/observe.rs`: diagnostic CLI using the same observer as the desktop.
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
