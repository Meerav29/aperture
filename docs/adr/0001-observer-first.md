# ADR 0001: Build the observer before the spawner

Status: accepted, 2026-09-04

## Decision

Phase 0 tracks sessions we did not start, via user-level Claude Code hooks
and transcript backfill. Spawning our own sessions (the `Runner` trait over
stream-json) comes after.

## Why

- No existing desktop tool (Conductor, Code Bar, claude-squad, herdr) shows
  sessions it didn't create. This is the differentiator.
- Hooks are a stable, documented interface. Bidirectional stream-json is not.
- It's useful on day one without changing how you already use Claude Code.

## Findings to fill in during the spike

- Does the desktop app fire user-level hooks?            [ ] yes  [ ] no
- Does it write transcripts to ~/.claude/projects/?       [ ] yes  [ ] no
- Real transcript field names (paste `head -3` output):
- Windows: does Claude Code run `.cmd` hooks directly?    [ ] yes  [ ] no
- Jump-to-it: which terminals worked?
