# Durable observation and recovery — design

Status: approved design for implementation. Scoped subset of
[specification.md](../../specification.md) Phase 1 ("Reliable collection and
recovery"), covering durable storage and incremental watching only. Git
repo/worktree identity, the SessionKey/IPC contract revision, and hook-inbox/
child-request handling are explicitly out of scope and remain separate,
later goals — bundling them here would violate the project's own "one bounded
outcome per implementation goal" discipline ([goals.md](../../goals.md)).

## Problem

The current baseline (`src-tauri/src/observer/passive.rs`, `lib.rs`) holds all
session state and file-read cursors in memory only. Every restart re-derives
everything from a blind 2-second poll of the full provider directory tree, and
in-flight file offsets are lost, so recovery has no way to distinguish
"already ingested" from "new" content without re-reading from byte zero.
There is no durable storage, no filesystem watcher (only fixed-interval full
rescans), and no restart/sleep-wake reconciliation contract.

## Goals

- Persist session summaries and per-file read cursors in SQLite so state
  survives an app restart.
- Replace the fixed 2s poll with `notify`-based incremental watching, with a
  time-based fallback.
- On restart, present previously observed sessions as stale/history-only
  immediately, then reconcile before claiming anything is live again.
- Detect a sleep/wake gap and force an immediate reconcile.
- Never double-count or re-apply already-ingested transcript lines after a
  restart, and never present a stale session as currently live.

## Non-goals

- Git repository/worktree identity (`repo_root` resolution).
- The SessionKey/IPC contract revision (`hide_session`, `navigate_session`,
  typed session keys).
- Hook-inbox bounds/corruption handling or child/request-tracking changes.
- A bounded activity timeline table (7-day retention) — no UI consumes a
  timeline today; only current-state summaries are persisted (90-day
  retention). Add the timeline table when a feature needs it.
- OS-native sleep/wake notifications (`WM_POWERBROADCAST`, `NSWorkspace`).
  A wall-clock gap heuristic covers the same observable requirement
  cross-platform without platform-specific code.

## Architecture

```
provider roots --> notify-debouncer-mini watcher --\
                                                      >--> tokio::select! --> Observer::poll() --> Store (in-memory)
5s reconcile interval ------------------------------/                              |
                                                                                    v
                                                                         SQLite (session_summaries, file_cursors)
                                                                     (same spawn_blocking task = single writer)
```

Startup: open DB -> migrate -> load summaries (forced stale/history_only,
`live: false`) -> push snapshot immediately -> load cursors into `Observer`
-> run one immediate `poll()` -> normal loop begins.

## Storage

New module `src-tauri/src/observer/db.rs`. SQLite file at
`<data_local_dir>/Aperture/aperture.db`, overridable via `APERTURE_DATA_DIR`
(same pattern as the existing `APERTURE_HOOK_DIR`). Accessed through a single
`rusqlite::Connection` guarded by a `std::sync::Mutex`, touched only inside
`spawn_blocking` closures — mirrors the existing `Observer` locking pattern
and is what makes it a single background writer without extra coordination.

Schema:

```sql
CREATE TABLE schema_meta (version INTEGER NOT NULL);

CREATE TABLE session_summaries (
  id TEXT PRIMARY KEY,       -- Session.id ("{provider}:{native_id}")
  data TEXT NOT NULL,        -- Session, JSON-serialized
  updated_at TEXT NOT NULL   -- RFC3339, for retention pruning
);

CREATE TABLE file_cursors (
  path TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  offset INTEGER NOT NULL,
  initial_len INTEGER NOT NULL,
  malformed INTEGER NOT NULL,
  session_id TEXT,           -- cursor.id: native session id in progress
  host TEXT,                 -- cursor.host, if resolved
  created_ns INTEGER,        -- file creation time, unix nanos; detects replacement
  updated_at TEXT NOT NULL
);
```

`session_summaries.data` stores the existing `Session` struct as JSON rather
than normalized columns. `Session` already derives `Serialize`/`Deserialize`
and is documented as the frontend contract
(`src-tauri/src/observer/model.rs:1-3`); reusing it means adding a `Session`
field later needs no migration, at the cost of the row not being queryable by
field. Nothing queries by field today, so this is a deliberate, revisitable
simplification, not a limitation anyone hits yet.

Migrations run as an ordered list of SQL strings applied inside one
transaction, gated by `PRAGMA user_version` (or the `schema_meta` table).
Before applying any pending migration, copy the current db file to
`aperture.db.bak`. If the migration transaction fails, the original file is
already untouched (SQLite transactions are atomic to the file); the app logs
the failure and falls back to in-memory-only operation for that run rather
than crashing or silently replacing the database. This satisfies the spec's
"leave the original intact and offer recovery" requirement for the one
migration this change ships; the mechanism is general enough to reuse for
future migrations without changes.

Retention: on startup, prune `session_summaries` rows whose `updated_at` is
older than 90 days. No separate activity/event table exists to prune on a
7-day window (see Non-goals).

## Recovery semantics

On restart:

1. Load all `session_summaries` rows, deserialize `Session`, and force
   `live = false` and `observation` to `"stale"` (if it was `"recent"`) or
   leave `"history_only"` — never trust a persisted `live: true`.
2. Seed `Store.sessions` with these and push a snapshot immediately, so the
   UI shows prior context before any reconciliation happens.
3. Load all `file_cursors` rows into `Observer`'s cursor map, keyed by path.
   `read_file`'s existing replaced-file detection (`meta.len() < cursor.offset
   || replaced`, comparing `created_ns`) continues to work unchanged — it
   already resets the cursor to zero when a file was truncated or replaced,
   which is exactly the "don't trust a stale offset against a rotated file"
   case.
4. Run one immediate `poll()` before entering the normal watch/interval loop,
   so `last_event_at`-based staleness and `live` recompute against current
   file state before anything is presented as confirmed-live.

Because cursors resume at their saved byte offset, already-ingested lines are
never re-read after a restart, so `apply()` (which overwrites state
monotonically per line rather than incrementing counters — see
`passive.rs::apply`) cannot double-apply history. This is the concrete
mechanism behind "no double-counting."

## Watching and scheduling

`lib.rs`'s setup loop changes from `loop { poll(); sleep(2s) }` to:

```rust
loop {
    tokio::select! {
        _ = watch_rx.recv() => {}                 // debounced fs change
        _ = interval.tick() => {}                  // 5s baseline reconcile
    }
    let gap = Utc::now() - last_poll_at;
    // gap > threshold means we likely just resumed from sleep; poll()
    // below still runs either way, this only affects what gets logged/surfaced.
    poll_and_persist().await;
    last_poll_at = Utc::now();
}
```

- Watcher: `notify-debouncer-mini`, recursive on the two provider roots
  (`~/.claude/projects`, `~/.codex/sessions`, or their env overrides),
  filtered to `.jsonl` changes, with its own debounce window. Gives
  sub-second responsiveness instead of waiting up to 2s.
- 5s interval: satisfies the spec's "reconcile every five seconds" and
  subsumes the spec's "30-second reconciliation scan as a watcher fallback"
  — `poll()` already performs a full `discover()` directory walk on every
  call regardless of trigger, so a 5s ceiling is a strict superset of a 30s
  fallback; no separate timer is needed.
- Sleep/wake: a plain wall-clock check (`Utc::now()` gap between successful
  polls exceeding ~90s) rather than OS power-event APIs. This is a
  deliberate cross-platform simplification (see Non-goals) that still
  satisfies the observable requirement: a long wall-clock gap always
  triggers immediate reconciliation on the very next loop iteration, because
  the loop wakes on either the watcher or the 5s interval regardless of why
  time passed.
- Persistence: `poll_and_persist()` runs inside the same `spawn_blocking`
  closure that already holds the observer lock for `poll()`, writing updated
  summaries and cursors to SQLite before returning. One call site, one
  connection, no concurrent writers.

## Storage health surfacing

`Snapshot.integrations` is already a generic `IntegrationHealth[]` that the
frontend renders by mapping over arbitrary `provider` values
(`src/App.tsx:13-14`, keyed by `h.provider`). Storage health is surfaced as an
additional synthetic entry with `provider: "storage"`, reporting the db path,
`ok`/`degraded` state, and a detail string (e.g. "in-memory only: migration
failed" when running degraded). No frontend code changes are required.

## Testing

- Migration idempotency: running migrations twice against the same file is a
  no-op and leaves `schema_meta.version` correct.
- Summary/cursor round-trip: write, reopen a fresh `Db` handle against the
  same file, read back, assert equality.
- Two-"run" recovery integration test: ingest some lines into a temp
  provider file, persist cursor + summary, drop the `Observer`/`Store`/`Db`,
  reconstruct fresh instances against the same DB and file, append more
  lines, poll again — assert exactly one session exists, counts/activity
  reflect only the new lines, and the resumed cursor offset matches.
- Migration failure path: a deliberately broken migration leaves the
  original db file byte-identical and the app falls back to in-memory-only
  without panicking.
- Pure unit test for the sleep/wake threshold function (gap under/over
  threshold).
- Existing `passive.rs`/`state.rs`/`hook_bridge.rs` tests are unaffected —
  the reducer logic (`apply`, `apply_hook`, `apply_transcript`) is unchanged;
  only where cursors/summaries are seeded from changes.

## Dependencies

- `rusqlite` with the `bundled` feature (no system SQLite dependency for
  packaging, consistent with "prefer the existing desktop foundation").
- `notify-debouncer-mini` (debounced batched fs events out of the box,
  avoids hand-rolled debounce logic).

## Files touched

- `src-tauri/Cargo.toml` — new dependencies.
- `src-tauri/src/observer/db.rs` — new: schema, migrations, load/save,
  retention prune.
- `src-tauri/src/observer/watch.rs` — new: watcher setup, debounced channel.
- `src-tauri/src/observer/passive.rs` — expose cursor restore/export so
  `Observer`'s private `cursors` map can be seeded from and written to `db.rs`.
- `src-tauri/src/lib.rs` — startup recovery sequence; replace the fixed-sleep
  loop with the select!-based watch/interval loop; sleep/wake gap check.
- `src-tauri/src/commands.rs` — add a `Db` handle to `Shared`; persist after
  manual `rescan_transcripts` too.
- Rust test files alongside the above modules.
