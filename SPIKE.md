# Phase 0 spike: the observer

Goal: a desktop window that shows every Claude Code session on this machine —
whichever app started it — with live status, and a loud strip at the top when
one of them is waiting on you.

No spawning, no worktree management, no chat UI. Those come after this works.

## Architecture in one paragraph

Claude Code fires user-level hooks (`~/.claude/settings.json`) for every
session regardless of host (terminal, VS Code, desktop app, headless). We
install one hook per lifecycle event whose command is a `curl` POST to
`http://127.0.0.1:47831/hook`. The Rust core runs that listener, folds events
into a per-session state machine, and pushes snapshots to the React window
over Tauri's event bus. On launch, and on demand, it backfills from the JSONL
transcripts under `~/.claude/projects/` so sessions that predate the app still
appear.

```
 claude (any host) --hook stdin JSON--> curl --POST--> listener.rs
                                                          |
 ~/.claude/projects/**/*.jsonl --scan--> transcript.rs --> state.rs
                                                          |
                                             emit "sessions:snapshot"
                                                          |
                                                  React SessionGrid
```

## Session state machine

```
              SessionStart
                   |
                   v
   +-----------> Idle <---------------------+
   |               |                        |
   |     UserPromptSubmit                  Stop
   |               |                        |
   |               v                        |
   |            Working ---PreToolUse----> Working (activity = tool)
   |               |                        ^
   |         Notification                   |
   |     (permission_prompt)          user answers
   |               |                        |
   |               v                        |
   |           Blocked ---------------------+
   |
   +-- SessionEnd --> Ended
       StopFailure --> Errored (rate_limit / overloaded / billing_error)
```

`Notification` with `idle_prompt` means "turn finished, waiting for input";
treat as Idle. Only `permission_prompt` is Blocked.

## Day-by-day

**Day 1 — prove the hook channel.**
1. `npm install`, then `npm run tauri dev`. Confirm the window opens on your
   primary OS. Push the repo and get it building on the other OS the same day;
   platform issues found on day 1 cost hours, found on day 20 cost weeks.
2. Click "Install hooks". Inspect `~/.claude/settings.json` and confirm the
   hooks were merged in without touching anything else. A backup is written
   alongside.
3. Start a Claude Code session in a terminal. It should appear in the grid
   within a second. Type a prompt: card goes Working. Ask it to edit a file
   with permissions on default: card goes Blocked and the strip appears.
4. Repeat from the VS Code extension and from the desktop app. **This is the
   go/no-go check**: confirm both fire user-level hooks and write transcripts
   to `~/.claude/projects/`. If the desktop app doesn't, note it in
   `docs/adr/0001-observer.md` and decide whether that's acceptable.

**Day 2 — backfill.**
5. Click "Rescan transcripts". Sessions from before the app launched appear,
   with message counts and token totals. Verify the transcript field names in
   `transcript.rs` against a real file (`head -3 ~/.claude/projects/*/*.jsonl`);
   the parser is tolerant but was written from memory, not from a live file.
6. Decide the merge rule when a hook event and a transcript disagree
   (current rule: hooks win for status, transcripts win for counts).

**Day 3 — jump-to-it.**
7. For a terminal session, use the PID captured at SessionStart
   (`$PPID` of the hook is claude's PID) to focus the owning terminal on
   macOS via AppleScript. Windows: `SetForegroundWindow` on the process's main
   window. This is the one platform-specific feature in the spike; timebox it.
8. Desktop-app sessions: check whether the app registers a URL scheme that
   opens a session by ID. If not, "jump" just reveals the transcript path.

**Day 4 — write it up.**
9. `docs/adr/0001-observer.md`: what worked, what the desktop app did, what
   the real transcript schema was. This is your first blog post.

## What is deliberately not here

- Cost in USD. Tokens only until you decide on a pricing table.
- File watching on `~/.claude/projects/`. Hooks cover live sessions; rescan
  covers history. Add `notify` later if you want transcripts to update live
  for sessions without hooks.
- Persistence. State is in memory. SQLite comes with the spawner layer.
- Any spawning. See the earlier plan for the `Runner` trait; it plugs into
  `state.rs` as a second event source.

## Known risks

- **Not compiled yet.** The Rust in `src-tauri/` was written without a
  toolchain available. Expect a handful of type errors on first `cargo check`.
  Nothing structural should be wrong.
- Hook command uses `curl`. Present on macOS and Windows 10+. If the app isn't
  running, curl exits 7 (connection refused) in well under a second; Claude
  Code treats non-2 exit codes as non-blocking, so sessions are unaffected.
- `~/.claude/settings.json` may also contain hooks from other tools. The
  installer only touches entries whose command contains `aperture`.
