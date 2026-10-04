# macOS validation results — round 1

Status: **in progress. No provider/host row has passed yet.** This file
records real Mac runs of the [macOS checklist](macos-checklist.md) by the Mac
tester. A row is filled in only after the tester has run it. Build and test
success here is evidence about compilation and code, not about how any host
behaves.

## Tested revision and environment

| Field | Value |
|---|---|
| Date started | 2026-10-04 16:46 EDT |
| Code commit under test | `614ee19` (`main`). Unmerged `auto/queue` work (slices 1–3) is **not** covered |
| macOS version / architecture | macOS 27.0 (26A428), arm64 |
| Node / npm | v22.18.0 / 10.9.3 |
| Rust | rustc 1.99.0, cargo 1.99.0 (stable, via rustup) |
| Claude Code CLI | Not installed on PATH: terminal row blocked until installed |
| Codex CLI | Not installed on PATH: terminal row blocked until installed |
| Claude desktop app | Claude.app 2.19675.0 |
| Claude Code VS Code extension | anthropic.claude-code 2.1.289 |
| Codex VS Code extension | openai.chatgpt 26.930.41038 |
| Codex desktop app | Not installed: desktop row blocked until installed |
| Effective roots | `~/.claude/projects`, `~/.codex/sessions` (defaults; no `CLAUDE_CONFIG_DIR`/`CODEX_HOME`) |
| Hooks | Not enabled (passive only) |

## 1. Build and test preflight

Run from a clean checkout of `614ee19` on 2026-10-04:

- `npm ci`: succeeded.
- `npm run build` (tsc + vite): succeeded; 34 modules, no type errors.
- `cargo test --manifest-path src-tauri/Cargo.toml --locked`: 51 passed,
  0 failed (49 lib, 1 `durable_recovery`, 1 `hook_bridge_cli`).
- `cargo run --bin observe -- 1` (one poll, read-only): both integrations
  reported `watching`, with 7 Claude Code files and 26 Codex files. The tool
  inferred `vs_code` and `desktop_app` hosts from **historical** files. This
  is discovery from existing history only. It is not a live lifecycle result
  for any row.

Observed during preflight, not yet triaged against existing issues:
`user_messages`, `assistant_messages` and the token counts were 0 for every
session, and `title` was null (consistent with open issue #42).

## 2. Provider/host rows

| Provider | Host | Result |
|---|---|---|
| Claude Code | Terminal.app | Blocked: CLI not installed |
| Claude Code | VS Code | Pending |
| Claude Code | Desktop Code | Pending |
| Codex | Terminal.app | Blocked: CLI not installed |
| Codex | VS Code | Pending |
| Codex | Desktop | Blocked: app not installed |

Each row gets its own record below, using the checklist template, once it has
been run.

<!--
Template (copy per row):

### <Provider> · <Host>

Date/time/timezone:
Commit:
macOS version / architecture:
Provider / CLI version / app or extension version:
Host / local execution:
Passive or hooks enabled:
Effective root (redacted):
Disposable session ID (redacted consistently if needed):
Steps:
Discovery result:
Prompt/tool/finish result:
Permission/input request and resolution result:
Interruption/session-close result:
Restart/sleep-wake result:
Navigation action and actual result:
Build/test outputs:
Evidence references (sanitized screenshot or trace):
Known failures / pending checks:
-->
