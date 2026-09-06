# Aperture

One dashboard for every Claude Code session on your machine, whichever app
started it. Currently a Phase 0 spike; see `SPIKE.md` for the plan.

## Run it

Prerequisites: Rust stable, Node 20+, and the Tauri v2 platform deps
(https://v2.tauri.app/start/prerequisites/).

```
npm install
npm run tauri dev
```

First run, click **Install hooks**. Then start Claude Code anywhere.

## Layout

```
src-tauri/src/observer/   Rust core, no Tauri types
  model.rs                Session + Snapshot (mirrored in src/features/sessions/types.ts)
  hook_payload.rs         parses the JSON Claude Code sends to hooks
  state.rs                the session state machine
  transcript.rs           backfill from ~/.claude/projects/**/*.jsonl
  hooks_installer.rs      merges our hooks into ~/.claude/settings.json
  listener.rs             POST /hook on 127.0.0.1:47831
src-tauri/hooks/          the scripts installed to ~/.claude/hooks/
src-tauri/src/commands.rs Tauri commands, the only bridge to the UI
src/features/sessions/    the grid, the card, the needs-you strip
```

## Useful checks

```
cargo test --manifest-path src-tauri/Cargo.toml      # state machine, installer, parser
curl http://127.0.0.1:47831/health                    # is the app listening
echo '{"session_id":"t","hook_event_name":"SessionStart","cwd":"/tmp"}' \
  | ~/.claude/hooks/aperture.sh                    # fake an event
APERTURE_PORT=50000 npm run tauri dev              # use another port
```
