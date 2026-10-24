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
| Claude Code | VS Code | **Partial**: discovery, resume, final status and folder navigation observed; permission prompt shows `working`/Tool: Bash/attention unknown (honest passive result); pasted images cause a false `degraded` (new finding); question, interrupt and restart pending |
| Claude Code | Desktop Code | Pending |
| Codex | Terminal.app | Blocked: CLI not installed |
| Codex | VS Code | Pending |
| Codex | Desktop | Blocked: app not installed |

Each row gets its own record below, using the checklist template, once it has
been run.

### Claude Code · VS Code (run 1, partial)

Date/time/timezone: 2026-10-09, about 15:08–15:10 EDT. Dashboard screenshot taken 2026-10-15
Commit: `614ee19` (app run with `npm run tauri dev`)
macOS version / architecture: macOS 27.0 (26A428), arm64
Provider / CLI version / app or extension version: Claude Code VS Code extension; the transcript records version 2.1.295 (updated from the 2.1.289 seen at preflight)
Host / local execution: VS Code, local, disposable folder `~/aperture-mac-test/claude-vscode`
Passive or hooks enabled: Passive only
Effective root (redacted): `~/.claude/projects`
Disposable session ID (redacted consistently if needed): `6de0ec7f…`
Steps: The tester started a new Claude Code chat in the disposable folder in VS Code and sent two small prompts, each of which ran one Bash tool call (it created `hello.txt`, then `check.txt`).
Discovery result: **Observed.** The card showed provider Claude Code, folder `~/aperture-mac-test/claude-vscode`, host `vs code`, and native ID prefix `6de0ec7f`. Last activity was 2026-10-09 15:09:46, which matches the transcript's final event (19:09:46Z). The transcript's `entrypoint` is `claude-vscode`.
Prompt/tool/finish result: Final state `idle` / "Turn complete" was observed. The transcript shows two turns: user prompt → Bash `tool_use` → successful `tool_result` → `end_turn`. **Pending:** whether the card was seen passing through working → tool → complete while the session ran, and how long each took.
Permission/input request and resolution result: **Not exercised.** Both Bash results arrived within about 0.5 s of the tool call, so no permission prompt was shown. Attention displayed "unknown (no confirmed attention signal)", which is correct for passive mode. Still to run: one permission request and one explicit question, each answered in VS Code.
Interruption/session-close result: Pending.
Restart/sleep-wake result: Pending. When viewed on 2026-10-15 (whether Aperture ran continuously since 2026-10-09 was not recorded), the session was listed with observation `stale · process liveness unknown` and was not shown as live. A deliberate restart during agent work, and a sleep/wake, are still pending.
Navigation action and actual result: Observed 2026-10-15. **Open folder** opened a Finder window titled `claude-vscode`, the session's working directory, shown in column view with the folder selected; it was empty, as expected after the deletions. **Reveal transcript** opened a Finder window on the transcript's containing folder, `~/.claude/projects/-Users-<user>-aperture-mac-test-claude-vscode`, with that folder highlighted under `projects`. The `.jsonl` file itself was not selected. Both match the implementation (`open::that` on `cwd` and on the transcript's parent directory) and the documented "folder/transcript-folder" labels. Neither action is exact-session focus: VS Code and the chat were not brought forward. The Rescan button showed "Rescanned: 32 transcripts".
Build/test outputs: See preflight section 1.
Evidence references (sanitized screenshot or trace): A dashboard screenshot from 2026-10-15 is held by the tester (not committed because it contains an unredacted home path). Transcript metadata was summarized locally by record type and tool name only; no prompt text was copied.
Run 2 (2026-10-15 22:05–22:06 EDT, same session `6de0ec7f…`, extension as above): The tester asked Claude to delete `check.txt`. The transcript shows Bash `tool_use` at 02:06:00.435Z, then a successful `tool_result` at 02:06:00.843Z, then `end_turn` at 02:06:01.911Z, and the file was removed. **No permission prompt was shown.** The result came back in 0.4 s, and no permission mode is set in user or project settings, so the chat was most likely in a mode that approves automatically. Attention was therefore still not exercised. Aperture showed the resumed session with observation `recent`, "just now", "Turn complete", and Claude Code last activity 22:06:01, which matches the final transcript event. The header count changed to "1 recently updating sessions".
Run 3 (2026-10-15 22:09 EDT, same session): The tester switched the chat to the default permission mode, and the transcript records `permissionMode: default`. They asked Claude to delete `hello.txt`. The transcript shows a Read call, then a Bash `tool_use` at 02:09:04.492Z. Its successful `tool_result` did not arrive until 02:09:55.522Z, a 51 s wait, which is consistent with a real permission prompt that the tester then approved. `end_turn` followed at 02:09:57Z and the file is gone. Afterwards Aperture showed `idle`, "Turn complete", and attention "unknown (no confirmed attention signal)", and Claude Code last activity was 22:09:57, which matches. **Pending:** what the card showed during the 51 s while the prompt was open (status, activity, attention). Passive mode has no permission signal, so attention is expected to stay "unknown". Only the tester's observation during the prompt can confirm that it did not wrongly show "none" or "idle".
Run 4 (2026-10-15 about 22:18 EDT, same session, default permission mode): A tester screenshot taken **while the Allow/Deny prompt was open** shows the card with `working`, "Tool: Bash", "just now", attention "unknown (no confirmed attention signal)", and observation `recent · process liveness unknown`. The header showed "1 recently updating sessions". This is the honest passive result: the tool call is visible, but the prompt itself is not. Attention did not falsely show `none` or `idle`.
**New finding in the same screenshot:** the Claude Code integration changed to **`degraded` with "2 skipped records"**. Investigation found the two skipped lines are user messages with three pasted screenshots: 1,476,319 bytes in the test session and 1,476,311 bytes in another local Claude Code session. Both exceed the passive reader's 1 MiB `MAX_LINE`, so each counts as malformed. Consequences: (a) pasting screenshots, an ordinary action, makes provider health permanently "degraded" for that file; (b) the user-prompt event on that line is dropped, so the turn's start is not observed from that record. Filed as #95; #45 (health-state test coverage) is related.
Known failures / pending checks: The storage-health row is labeled "Claude Code ok" rather than as storage, which reproduces open issue #28 on macOS. The session has an `ai-title` record, but the card shows the folder name instead (consistent with #42/#47). Live transitions, permission, question, interrupt, restart-during-work and navigation checks remain pending.

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
