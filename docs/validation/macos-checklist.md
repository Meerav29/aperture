# macOS validation checklist (manual — needs a real Mac)

Nothing here can be done from the Windows machine this repo was otherwise
validated on. `docs/validation/README.md` documents Windows evidence; this
file exists so the same rigor gets applied on macOS instead of being inferred
from Windows, which `docs/specification.md` explicitly forbids ("never infer
macOS results from Windows").

Do this on a Mac with: Claude Code CLI, the Claude Code VS Code extension,
Claude Desktop (Code tab), the Codex CLI, the Codex VS Code extension, and the
Codex desktop app all installed and already signed in. You do not need to
touch this repo's Rust/TS code to do the validation runs — only to build and
launch the app.

## 0. Build and run

```
git clone <this repo> aperture   # or pull this branch
cd aperture
npm install
npm run tauri dev
```

Confirm the window opens and the top panel shows `Claude Code watching` and
`Codex watching` (or `no_sessions`/`degraded` — note which, and why, if either
provider isn't `watching`). Leave it running for the rest of this checklist;
it polls automatically every 2 seconds and needs no clicks.

## 1. Six real sessions, one per row

For each of the six combinations below, start a **disposable** session (a
throwaway prompt like "reply with the word pong" — do not interrupt real
work) and watch the Aperture window pick it up within a few seconds.

| # | Provider | Host | How to start it |
|---|---|---|---|
| 1 | Claude Code | Terminal | `claude -p "reply with the word pong"` in Terminal.app |
| 2 | Claude Code | VS Code | Open a folder in VS Code, use the Claude Code extension's panel/chat to send a prompt |
| 3 | Claude Code | Desktop | Open the Claude desktop app, Code tab, start a session |
| 4 | Codex | Terminal | `codex exec "reply with the word pong"` in Terminal.app |
| 5 | Codex | VS Code | Open a folder in VS Code, use the Codex extension to send a prompt |
| 6 | Codex | Desktop | Open the Codex desktop app, start a session |

For each one, record:
- Did a new card appear in Aperture within ~5 seconds of the session
  producing its first output? (This is "Events: Verified/Unverified".)
- Provider label shown (Claude Code / Codex) — correct?
- Status shown (working/idle/etc.) and whether it changed when the session
  went from generating to done — correct?
- `cwd`/folder name shown — correct?

## 2. Real transcript/session file locations

Confirm these are the actual roots on macOS (they may differ from Windows —
Codex in particular is not guaranteed to use `~/.codex`):

```
ls ~/.claude/projects/*/*.jsonl | head -3
ls ~/.codex/sessions/**/*.jsonl | head -3
```

If either path differs from `~/.claude/projects` or `~/.codex/sessions`
(check `CLAUDE_CONFIG_DIR`/`CODEX_HOME` env vars too), that's a required code
change in `src-tauri/src/observer/passive.rs::Observer::default()`, not just
a doc update — flag it, don't silently work around it locally.

## 3. Host-label fields — confirm they still hold on macOS

Run this against a real Codex session file from each of the three Codex hosts
above:

```
grep -o '"originator":"[^"]*"' ~/.codex/sessions/**/*.jsonl | sort -u
```

Expected values (confirmed on Windows, needs re-confirmation here):
`"Codex Desktop"`, `"codex_vscode"`, and for a terminal `codex exec` run,
`"codex_exec"`. If macOS reports different strings, `infer_codex_host` in
`passive.rs` needs new cases — it currently only matches on `vscode` and
`desktop` substrings, so a differently-worded macOS originator may already
work, but confirm it rather than assume it.

For Claude Code, confirm the negative result also holds on macOS — that
`entrypoint` (`cli`/`claude-vscode`/`claude-desktop`) does **not** appear as a
top-level field in `~/.claude/projects/*/*.jsonl` (it should only show up
nested inside some other plugin's echoed hook debug output, if that plugin is
installed):

```
grep -c '"entrypoint"' ~/.claude/projects/*/*.jsonl
```

## 4. Navigation

Click "Open folder" and "Reveal transcript" on a couple of cards. Confirm
Finder opens the right folder both times. Note whether macOS shows any
permission prompt for this (Automation/Full Disk Access) that Windows didn't.

## 5. Write up the results

Copy `docs/validation/README.md`'s format: which sessions you used (IDs,
provider versions — `claude --version` / `codex --version`), what Aperture
showed, timestamps, and any surprises. Update the `macOS` rows in `SPIKE.md`'s
compatibility table from `Unverified`/`Pending` to `Verified` (or leave
specific cells `Unverified` with a one-line reason if something didn't work —
do not mark a row verified because the other five looked fine). If you hit a
divergence from step 2 or 3, fix `passive.rs` and re-run `cargo test
--manifest-path src-tauri/Cargo.toml` before calling it done.
