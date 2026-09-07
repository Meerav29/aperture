# Windows two-provider observer validation

Validated on Windows x64 on September 6, 2026 (EDT; evidence timestamps are UTC).
This is a passive file-observation milestone, not completion of every phase in
`docs/specification.md` or the cross-platform compatibility matrix in `SPIKE.md`.

## Actual external sessions

| Provider | Session | Reported working directory | Evidence |
|---|---|---|---|
| Claude Code 2.1.260 | `e8de9487-e1a0-4cbb-9a13-6fa43d610868` | `C:\Users\meera\Github-Projects` | Prompt -> Bash -> tool result -> turn complete, 02:43:49–02:44:00 UTC |
| Codex | `01a079ba-ab46-7d82-a5db-989390b8cea5` | `C:\Users\meera\Github-Projects\aperture` | Turn complete -> thinking -> turn complete, 02:43:31–02:44:27 UTC |

Both were present with recent observation in 35 consecutive snapshots from the
same poller. `windows-live-evidence.json` retains distinct state changes from
that interval, with provider health and both session identities. This is real
file growth, not injected hook events or resumed sessions. Neither provider
was launched, resumed, messaged, focused, or controlled by the observer or the
validation commands. The Codex task performing this implementation is excluded
from this evidence pair.

The user's earlier Codex session `01a079b7-97de-7b53-b57b-ec0a2acfb327`, reporting
`GitHub-Projects`, was discovered but did not append another turn during the
recorded window. Its metadata reports Codex Desktop / CLI 0.153.4. The sanitized
Codex fixture comes from that session. Do not confuse it with the observed
updating Codex session above.

`windows-observer.png` shows the real Tauri dashboard after the activity window,
including history-only labels after restarting the final build. The captured
trace also verifies stale labels after 60 seconds of silence. It is a later
layout check, not a screenshot claiming simultaneous generation.

## Safety and integration health

- `settings-preserved.json` records identical SHA-256 hashes before and after
  for Claude `settings.json` and Codex `config.toml`.
- Desktop IPC exposes only snapshot retrieval and read-only rescan. Provider
  installation/removal and session navigation/control commands are absent.
- No HTTP collector or provider process is started. Existing provider settings
  and hook scripts are left untouched. Old hook implementation files remain
  in the repository but are not started or exposed by the desktop.
- The validated roots contained 34 Claude and 65 Codex session files, with no
  read errors or malformed records during the captured interval. The final
  startup scan reported four skipped Codex records and correctly displayed
  degraded health; Claude remained watching. The parser limits records to
  1 MiB and counts malformed/oversized records rather than hiding the loss.
- Initial sandbox execution resolved the sandbox account's home folder and
  correctly showed degraded health. Validation then set explicit provider-root
  environment variables on Aperture only. No provider configuration was edited.

## Checks

- Production frontend build and TypeScript check.
- Nine passing Rust tests, including provider-qualified ID collisions, independent lifecycle,
  partial line buffering, duplicate polls, truncation, stale observation,
  historical replay, explicit question tools, ignored unknown/child events,
  and rejection of older state updates.
- Sanitized real Windows records for both providers replay to independent idle
  sessions without a live claim.
- Native Windows Tauri window launched and captured with correct DPI handling.

## Capabilities and limits

Aperture tails complete JSONL records by byte offset. It uses provider metadata,
not model names, for identity. It reads tool names, timestamps, lifecycle and
working directory; it does not display prompt text, arguments, tool output,
reasoning, or token estimates. New append activity is recent for 60 seconds;
silence becomes stale without inventing idle/ended transitions. Initial replay
is history only. Provider roots honor `CLAUDE_CONFIG_DIR` and `CODEX_HOME`.

Explicit question tools can show input attention. Permission prompts are not
reliably present in these files, so attention otherwise remains unknown rather
than claiming that no action is needed. This is not full hook-based attention
coverage. Concurrent question resolution and complete error/session-exit
coverage are not established by this milestone. Host/process liveness is
unknown, including whether a completed turn still has an open terminal.

There is no durable database, filesystem watcher, git worktree grouping, or
macOS/other-host compatibility claim. Cursors are in memory, polling scans the
configured roots, and large files are read in bounded chunks per scan. Replaced
or truncated logs are replayed; same-length in-place rewriting without a file
identity change is not guaranteed to be detected. No provider configuration changes or write
access to provider files are required.

## Update — September 7, 2026 (Windows x64, same machine)

This continues the milestone above rather than replacing it; all limitations
recorded above still apply except where superseded below.

- **Title parsing fixed**: `transcript.rs`'s title extraction assumed
  `{"type":"summary","summary":"..."}`. A real transcript
  (`~/.claude/projects/c--Users-meera-MyPortfolio-MyPortfolio/e302a3db-....jsonl`)
  has zero such lines; the real title source is
  `{"type":"custom-title","customTitle":"..."}`. Fixed and covered by
  `title_comes_from_custom_title_line`. This only affects the legacy
  hooks/backfill path (`transcript.rs`), not the passive desktop path
  (`passive.rs`), which doesn't parse titles at all yet.
- **Codex host label implemented**: `passive.rs::infer_codex_host` reads
  `session_meta.payload.originator`/`source`, which Codex self-reports.
  Confirmed real values on this machine: `"Codex Desktop"`, `"codex_vscode"`,
  and (newly captured this session) `"codex_exec"` from a live, disposable
  `codex exec -c 'notify=[]' --sandbox read-only "reply with the word pong"`
  run in a plain terminal (session `01a07a06-712b-7d91-86eb-e4affbbca013`,
  cwd `C:\Users\meera\AppData\Local\Temp\codex-terminal-test`; the prompt
  itself hit an account usage-limit error, which is irrelevant to and does
  not affect the observation evidence — the session file and its
  `session_meta` line were written before the API call failed). Codex Desktop
  and VS Code originators come from real historical session files already on
  this machine, not synthetic fixtures. Claude Code has no equivalent
  self-reported field in its transcript (see `SPIKE.md`'s compatibility
  table); this remains an explicit, open gap, not an oversight.
- **Navigation implemented (partial, honest)**: `open_session_folder` and
  `reveal_transcript` Tauri commands open a session's `cwd` or the folder
  containing its `transcript_path` via the `open` crate. Both look the
  session up by Aperture's own store `id`, not an arbitrary caller-supplied
  path, per the specification's navigation-safety requirement. Window/tab
  focus and exact-session targeting remain unimplemented for every host.
- **Confirmed the Codex approval-request gap is a provider limitation, not a
  missing feature**: per Codex's own documentation and this machine's
  `~/.codex/config.toml`, the external `notify` hook fires only on
  `agent-turn-complete`; `approval-requested` only reaches a local terminal/OS
  notification that nothing outside that process can observe. No file or hook
  channel currently exposes Codex approval prompts to an external observer.
- **Still not done**: Claude Code host label (needs the hook channel, not
  just files — see `SPIKE.md`); Claude Code permission-prompt attention via
  the passive desktop path (the hook-based code for this already exists in
  `hooks_installer.rs`/`state.rs` but isn't wired into `passive.rs`'s store);
  macOS validation of any of the above (see `docs/validation/macos-checklist.md`).

