# Persistence Write Amplification and Retention Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut SQLite write volume to rows whose content actually changed since the last write, bound worst-case reconcile frequency during watcher bursts, and make the existing 90-day retention prune (currently inert) actually remove stale rows — for both `session_summaries` and the previously-unpruned `file_cursors`.

**Architecture:** `Db` gains an in-memory "last written" cache (folded into the same `Mutex` as the `Connection`, so there's no new lock-ordering surface) that `save_summaries`/`save_cursors` consult before issuing any SQL — unchanged rows are skipped entirely, which is also what makes `updated_at` (and therefore retention) meaningful again, since it now only advances when content actually changes. Separately, `lib.rs`'s reconcile loop gets a minimum spacing between watcher-triggered reconciles (the independent 5-second baseline interval is untouched) to bound frequency for genuinely-active sessions that the cache can't help with.

**Tech Stack:** Rust, rusqlite, tokio (existing dependencies only — no new crates).

**Spec:** [docs/superpowers/specs/2026-09-10-persistence-write-amplification-retention-design.md](../specs/2026-09-10-persistence-write-amplification-retention-design.md)

## Global Constraints

- Content-aware writes must skip the SQL statement entirely for a row whose content matches the cache, and must skip opening a transaction at all when every row in a batch is unchanged.
- The cache lives in the *same* `Mutex` as the `Connection` (one guarded struct, not two separately-locked fields) — no new lock-ordering surface.
- The cache starts empty on every process launch; the first write after each restart is allowed to be a full, unoptimized write. Do not seed the cache from `load_summaries`/`load_cursors` at open time — deliberately out of scope per the spec.
- The minimum watcher-reconcile spacing (1 second) applies only to watcher-triggered wakes. The existing 5-second baseline `interval.tick()` is untouched and keeps firing on its own independent schedule.
- `prune_cursors` mirrors `prune_summaries` exactly (same shape, same `SUMMARY_RETENTION_DAYS` constant — no new retention-window constant).
- No SQLite schema migration: `updated_at` already exists on both `session_summaries` and `file_cursors`.
- `Db::save_summaries`/`save_cursors`/`load_summaries`/`load_cursors`/`prune_summaries`/`is_durable` keep their existing public signatures — this is an internal behavior change, not an API change. Every existing caller in `commands.rs` and `lib.rs` must compile unchanged except for the new `prune_cursors` call this plan adds.
- Run `cargo test --manifest-path src-tauri/Cargo.toml --locked` before considering any task done, per `AGENTS.md`. All pre-existing tests must continue to pass.

---

### Task 1: Content-aware writes in `Db`

**Files:**
- Modify: `src-tauri/src/observer/db.rs`

**Interfaces:**
- Produces: `Db::save_summaries`/`save_cursors` (unchanged signatures, new skip-if-unchanged behavior), `#[cfg(test)] pub(crate) fn Db::summary_updated_at(&self, id: &str) -> Option<String>`, `#[cfg(test)] pub(crate) fn Db::cursor_updated_at(&self, path: &str) -> Option<String>` (test-only diagnostic accessors, consumed by this task's own tests and by Task 4's cross-module integration test).
- Consumes: nothing new — restructures existing internals only.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/observer/db.rs` (after the existing `saving_the_same_summary_id_twice_upserts_not_duplicates` test):

```rust
    #[test]
    fn resaving_unchanged_summary_does_not_touch_updated_at() {
        let db = Db::in_memory();
        let s = sample_session("claude_code:a");
        db.save_summaries(&[s.clone()]).unwrap();
        let first = db.summary_updated_at("claude_code:a").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        db.save_summaries(&[s]).unwrap();
        let second = db.summary_updated_at("claude_code:a").unwrap();

        assert_eq!(first, second, "unchanged content must not touch updated_at");
    }

    #[test]
    fn resaving_changed_summary_updates_data_and_updated_at() {
        let db = Db::in_memory();
        let mut s = sample_session("claude_code:a");
        db.save_summaries(&[s.clone()]).unwrap();
        let first = db.summary_updated_at("claude_code:a").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        s.status = crate::observer::model::SessionStatus::Working;
        db.save_summaries(&[s]).unwrap();
        let second = db.summary_updated_at("claude_code:a").unwrap();

        assert_ne!(first, second, "changed content must update updated_at");
        let loaded = db.load_summaries().unwrap();
        assert_eq!(
            loaded[0].status,
            crate::observer::model::SessionStatus::Working
        );
    }

    #[test]
    fn batch_save_only_rewrites_changed_summary_rows() {
        let db = Db::in_memory();
        let a = sample_session("claude_code:a");
        let b = sample_session("claude_code:b");
        db.save_summaries(&[a.clone(), b.clone()]).unwrap();
        let a_before = db.summary_updated_at("claude_code:a").unwrap();
        let b_before = db.summary_updated_at("claude_code:b").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut b2 = b;
        b2.status = crate::observer::model::SessionStatus::Working;
        db.save_summaries(&[a, b2]).unwrap();

        let a_after = db.summary_updated_at("claude_code:a").unwrap();
        let b_after = db.summary_updated_at("claude_code:b").unwrap();
        assert_eq!(a_before, a_after, "unchanged row a must not be rewritten");
        assert_ne!(b_before, b_after, "changed row b must be rewritten");
    }

    #[test]
    fn resaving_unchanged_cursor_does_not_touch_updated_at() {
        let db = Db::in_memory();
        let record = CursorRecord {
            path: "C:/fixture/f.jsonl".into(),
            provider: "claude_code".into(),
            offset: 10,
            ..Default::default()
        };
        db.save_cursors(&[record.clone()]).unwrap();
        let first = db.cursor_updated_at("C:/fixture/f.jsonl").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        db.save_cursors(&[record]).unwrap();
        let second = db.cursor_updated_at("C:/fixture/f.jsonl").unwrap();

        assert_eq!(first, second, "unchanged cursor must not touch updated_at");
    }

    #[test]
    fn resaving_changed_cursor_updates_data_and_updated_at() {
        let db = Db::in_memory();
        let record = CursorRecord {
            path: "C:/fixture/f.jsonl".into(),
            provider: "claude_code".into(),
            offset: 10,
            ..Default::default()
        };
        db.save_cursors(&[record.clone()]).unwrap();
        let first = db.cursor_updated_at("C:/fixture/f.jsonl").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut moved = record;
        moved.offset = 20;
        db.save_cursors(&[moved]).unwrap();
        let second = db.cursor_updated_at("C:/fixture/f.jsonl").unwrap();

        assert_ne!(first, second, "changed cursor must update updated_at");
        let loaded = db.load_cursors().unwrap();
        assert_eq!(loaded[0].offset, 20);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml summary_updated_at -- --nocapture`
Expected: FAIL to compile — `no method named 'summary_updated_at' found for struct 'Db'` (and similarly for `cursor_updated_at` once that line is reached).

- [ ] **Step 3: Restructure `Db` and implement content-aware writes**

Replace the top of `src-tauri/src/observer/db.rs` (from the `use` block through the end of `impl Db`) with:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection};

use super::model::Session;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CursorRecord {
    pub path: String,
    pub provider: String,
    pub offset: u64,
    pub initial_len: u64,
    pub malformed: u64,
    pub session_id: Option<String>,
    pub host: Option<String>,
    pub created_ns: Option<i64>,
}

/// The connection plus an in-memory "last written" cache, kept in one
/// mutex so checking the cache and writing SQLite can never race or need
/// separate lock ordering. The cache starts empty every process launch —
/// the first write after each restart is a full, unoptimized write; every
/// write after that skips rows whose content hasn't changed since the last
/// successful write, which is also what makes `updated_at`-based retention
/// meaningful again (it only advances when content actually changes).
struct ConnState {
    conn: Connection,
    summary_cache: HashMap<String, String>,
    cursor_cache: HashMap<String, CursorRecord>,
}

pub struct Db {
    state: Mutex<ConnState>,
    /// The on-disk path this `Db` was opened against, or `None` when backed
    /// by an in-memory connection (either `Db::in_memory()` directly, or
    /// `lib.rs`'s fallback after `Db::open` fails). Used by
    /// `commands::storage_health` to distinguish "durable but a write
    /// failed" from "not durable at all" — both look identical from inside
    /// a single save call, since in-memory saves always succeed.
    path: Option<PathBuf>,
}

/// The platform app-data directory Aperture uses for its own files.
/// Overridable via `APERTURE_DATA_DIR` for tests and diagnostics, mirroring
/// the existing `APERTURE_HOOK_DIR` override in `hook_bridge.rs`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APERTURE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("Aperture")
        })
}

const MIGRATIONS: &[&str] = &[include_str!("migrations/0001_init.sql")];

impl Db {
    /// Open (creating if needed) the database at `path`, running any
    /// pending migrations inside a transaction per migration. On failure the
    /// original file is left untouched (see `migrate`); the caller should
    /// fall back to `Db::in_memory()` for that run.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        migrate(&conn, path, MIGRATIONS)?;
        Ok(Db {
            state: Mutex::new(ConnState {
                conn,
                summary_cache: HashMap::new(),
                cursor_cache: HashMap::new(),
            }),
            path: Some(path.to_path_buf()),
        })
    }

    pub fn in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("open in-memory sqlite");
        migrate(&conn, Path::new(":memory:"), MIGRATIONS).expect("migrate in-memory sqlite");
        Db {
            state: Mutex::new(ConnState {
                conn,
                summary_cache: HashMap::new(),
                cursor_cache: HashMap::new(),
            }),
            path: None,
        }
    }

    /// Whether this `Db` is backed by a real on-disk file (`Db::open`
    /// succeeded) rather than an in-memory connection (`Db::in_memory()`,
    /// used both directly by tests and as `lib.rs`'s fallback when
    /// `Db::open` fails). Writes to an in-memory `Db` always succeed but
    /// vanish on restart, so callers reporting persistence health need this
    /// alongside the save result.
    pub fn is_durable(&self) -> bool {
        self.path.is_some()
    }

    pub fn save_summaries(&self, sessions: &[Session]) -> rusqlite::Result<()> {
        let mut guard = self.state.lock().expect("db lock");
        let state = &mut *guard;

        let mut changed: Vec<(String, String)> = Vec::new();
        for s in sessions {
            let data = serde_json::to_string(s).expect("Session serializes");
            if state.summary_cache.get(&s.id) != Some(&data) {
                changed.push((s.id.clone(), data));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();
        {
            let tx = state.conn.transaction()?;
            for (id, data) in &changed {
                tx.execute(
                    "INSERT INTO session_summaries (id, data, updated_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
                    params![id, data, now],
                )?;
            }
            tx.commit()?;
        }
        for (id, data) in changed {
            state.summary_cache.insert(id, data);
        }
        Ok(())
    }

    pub fn load_summaries(&self) -> rusqlite::Result<Vec<Session>> {
        let guard = self.state.lock().expect("db lock");
        let mut stmt = guard.conn.prepare("SELECT data FROM session_summaries")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(s) = serde_json::from_str::<Session>(&r?) {
                out.push(s);
            }
        }
        Ok(out)
    }

    pub fn save_cursors(&self, cursors: &[CursorRecord]) -> rusqlite::Result<()> {
        let mut guard = self.state.lock().expect("db lock");
        let state = &mut *guard;

        let changed: Vec<&CursorRecord> = cursors
            .iter()
            .filter(|c| state.cursor_cache.get(&c.path) != Some(*c))
            .collect();
        if changed.is_empty() {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();
        {
            let tx = state.conn.transaction()?;
            for c in &changed {
                tx.execute(
                    "INSERT INTO file_cursors
                        (path, provider, offset, initial_len, malformed, session_id, host, created_ns, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(path) DO UPDATE SET
                        provider = excluded.provider, offset = excluded.offset,
                        initial_len = excluded.initial_len, malformed = excluded.malformed,
                        session_id = excluded.session_id, host = excluded.host,
                        created_ns = excluded.created_ns, updated_at = excluded.updated_at",
                    params![
                        c.path,
                        c.provider,
                        c.offset as i64,
                        c.initial_len as i64,
                        c.malformed as i64,
                        c.session_id,
                        c.host,
                        c.created_ns,
                        now
                    ],
                )?;
            }
            tx.commit()?;
        }
        for c in changed {
            state.cursor_cache.insert(c.path.clone(), c.clone());
        }
        Ok(())
    }

    pub fn load_cursors(&self) -> rusqlite::Result<Vec<CursorRecord>> {
        let guard = self.state.lock().expect("db lock");
        let mut stmt = guard.conn.prepare(
            "SELECT path, provider, offset, initial_len, malformed, session_id, host, created_ns
             FROM file_cursors",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CursorRecord {
                path: row.get(0)?,
                provider: row.get(1)?,
                offset: row.get::<_, i64>(2)? as u64,
                initial_len: row.get::<_, i64>(3)? as u64,
                malformed: row.get::<_, i64>(4)? as u64,
                session_id: row.get(5)?,
                host: row.get(6)?,
                created_ns: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Delete summaries not updated in `days` days. Returns the number removed.
    pub fn prune_summaries(&self, days: i64) -> rusqlite::Result<usize> {
        let guard = self.state.lock().expect("db lock");
        let cutoff = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        guard.conn.execute(
            "DELETE FROM session_summaries WHERE updated_at < ?1",
            params![cutoff],
        )
    }

    /// Test-only accessor: the raw `updated_at` column for one summary row,
    /// used to prove content-aware writes skip unchanged rows without
    /// exposing this as production API surface.
    #[cfg(test)]
    pub(crate) fn summary_updated_at(&self, id: &str) -> Option<String> {
        self.state
            .lock()
            .expect("db lock")
            .conn
            .query_row(
                "SELECT updated_at FROM session_summaries WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok()
    }

    /// Test-only accessor: the raw `updated_at` column for one cursor row.
    /// See `summary_updated_at`.
    #[cfg(test)]
    pub(crate) fn cursor_updated_at(&self, path: &str) -> Option<String> {
        self.state
            .lock()
            .expect("db lock")
            .conn
            .query_row(
                "SELECT updated_at FROM file_cursors WHERE path = ?1",
                params![path],
                |r| r.get(0),
            )
            .ok()
    }
}
```

Leave `migrate` and everything below it (down to `#[cfg(test)] mod tests`) unchanged — `migrate` takes a plain `&Connection`, constructed before it's wrapped in `ConnState`, so it needs no changes.

- [ ] **Step 4: Fix the existing `prune_removes_only_old_summaries` test**

That existing test (already in the file, before your new tests) directly does `let conn = db.conn.lock().unwrap();` — this no longer compiles because `conn` is now a private field of `ConnState`, not `Db`. Update it to:

```rust
    #[test]
    fn prune_removes_only_old_summaries() {
        let db = Db::in_memory();
        db.save_summaries(&[sample_session("claude_code:fresh")])
            .unwrap();
        {
            let guard = db.state.lock().unwrap();
            let old = (ChronoUtc::now() - chrono::Duration::days(200)).to_rfc3339();
            guard.conn.execute(
                "INSERT INTO session_summaries (id, data, updated_at) VALUES ('old', '{}', ?1)",
                params![old],
            )
            .unwrap();
        }
        let removed = db.prune_summaries(90).unwrap();
        assert_eq!(removed, 1);
        let remaining = db.load_summaries().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "claude_code:fresh");
    }
```

(Only the `db.conn.lock()` line changes to `db.state.lock()`, and the subsequent `conn.execute(...)` call becomes `guard.conn.execute(...)`.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including the five new tests from Step 1 and the fixed `prune_removes_only_old_summaries`. The pre-existing `summaries_round_trip`, `saving_the_same_summary_id_twice_upserts_not_duplicates`, `cursors_round_trip`, `migrations_are_idempotent`, `a_failed_migration_leaves_the_original_file_byte_identical`, and `migrate_commits_user_version_and_schema_together` tests must all still pass unchanged — they exercise `Db`'s public API and migration path, neither of which changed shape.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/observer/db.rs
git commit -m "Make Db writes content-aware: skip SQL for unchanged rows"
```

---

### Task 2: Cursor retention (`prune_cursors`)

**Files:**
- Modify: `src-tauri/src/observer/db.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `ConnState`/`Db::state` (from Task 1).
- Produces: `impl Db { pub fn prune_cursors(&self, days: i64) -> rusqlite::Result<usize>; }`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/observer/db.rs`'s test module, after `prune_removes_only_old_summaries`:

```rust
    #[test]
    fn prune_cursors_removes_only_old_cursors() {
        let db = Db::in_memory();
        db.save_cursors(&[CursorRecord {
            path: "fresh.jsonl".into(),
            provider: "claude_code".into(),
            ..Default::default()
        }])
        .unwrap();
        {
            let guard = db.state.lock().unwrap();
            let old = (ChronoUtc::now() - chrono::Duration::days(200)).to_rfc3339();
            guard.conn.execute(
                "INSERT INTO file_cursors (path, provider, offset, initial_len, malformed, updated_at)
                 VALUES ('old.jsonl', 'claude_code', 0, 0, 0, ?1)",
                params![old],
            )
            .unwrap();
        }
        let removed = db.prune_cursors(90).unwrap();
        assert_eq!(removed, 1);
        let remaining = db.load_cursors().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, "fresh.jsonl");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml prune_cursors_removes_only_old_cursors -- --nocapture`
Expected: FAIL with "no method named `prune_cursors` found"

- [ ] **Step 3: Implement `prune_cursors`**

Add to `impl Db` in `src-tauri/src/observer/db.rs`, directly after `prune_summaries`:

```rust
    /// Delete file cursors not updated in `days` days. Returns the number
    /// removed. Mirrors `prune_summaries` exactly — `updated_at` on this
    /// table now has the same meaning it does on `session_summaries` since
    /// `save_cursors` only advances it when a cursor's content actually
    /// changed.
    pub fn prune_cursors(&self, days: i64) -> rusqlite::Result<usize> {
        let guard = self.state.lock().expect("db lock");
        let cutoff = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        guard.conn.execute(
            "DELETE FROM file_cursors WHERE updated_at < ?1",
            params![cutoff],
        )
    }
```

- [ ] **Step 4: Wire it into startup**

In `src-tauri/src/lib.rs`, find this existing line (inside `run()`, right after opening/falling-back the `Db`):

```rust
    let _ = db.prune_summaries(SUMMARY_RETENTION_DAYS);
```

Add directly after it:

```rust
    let _ = db.prune_cursors(SUMMARY_RETENTION_DAYS);
```

- [ ] **Step 5: Run the tests and build**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including the new `prune_cursors_removes_only_old_cursors`.

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: succeeds — confirms the `lib.rs` call site compiles against the new method.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/observer/db.rs src-tauri/src/lib.rs
git commit -m "Add file_cursors retention pruning, wired into startup"
```

---

### Task 3: Coalesce watcher-triggered reconciles

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `fn watcher_reconcile_delay(elapsed: std::time::Duration, min_spacing: std::time::Duration) -> Option<std::time::Duration>` (pure, unit-tested — the extractable sub-logic the spec calls for, following the same pattern as the existing `is_wake_gap`).
- Consumes: nothing new — restructures the existing reconcile loop in `run()`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src-tauri/src/lib.rs` (alongside the existing `is_wake_gap` tests):

```rust
    #[test]
    fn no_delay_needed_when_spacing_already_satisfied() {
        assert_eq!(
            watcher_reconcile_delay(
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn no_delay_needed_when_spacing_exactly_satisfied() {
        assert_eq!(
            watcher_reconcile_delay(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn delay_needed_when_within_minimum_spacing() {
        assert_eq!(
            watcher_reconcile_delay(
                std::time::Duration::from_millis(300),
                std::time::Duration::from_secs(1)
            ),
            Some(std::time::Duration::from_millis(700))
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml watcher_reconcile_delay -- --nocapture`
Expected: FAIL with "cannot find function `watcher_reconcile_delay`"

- [ ] **Step 3: Implement `watcher_reconcile_delay` and wire it into the loop**

Add this function to `src-tauri/src/lib.rs`, directly after `is_wake_gap`:

```rust
const MIN_WATCHER_RECONCILE_SPACING: std::time::Duration = std::time::Duration::from_secs(1);

/// How much longer to wait before a watcher-triggered reconcile, given
/// `elapsed` time since the last reconcile and the minimum allowed
/// spacing. `None` means proceed immediately. This bounds reconcile
/// frequency during a burst of filesystem-watcher activity (e.g. an
/// actively streaming session) without touching the independent 5-second
/// baseline `interval.tick()`, which is unaffected by this and keeps
/// firing on its own schedule regardless.
fn watcher_reconcile_delay(
    elapsed: std::time::Duration,
    min_spacing: std::time::Duration,
) -> Option<std::time::Duration> {
    min_spacing.checked_sub(elapsed).filter(|d| !d.is_zero())
}
```

Then, in `run()`'s `setup` closure, replace this block:

```rust
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
                let mut last_poll_at = Utc::now();

                loop {
                    tokio::select! {
                        _ = watch_rx.recv() => {}
                        _ = interval.tick() => {}
                    }
```

with:

```rust
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
                let mut last_poll_at = Utc::now();
                let mut last_reconcile_instant = std::time::Instant::now();

                loop {
                    tokio::select! {
                        _ = watch_rx.recv() => {
                            // Collapse a burst of debounced signals into one wake.
                            while watch_rx.try_recv().is_ok() {}
                            if let Some(remaining) = watcher_reconcile_delay(
                                last_reconcile_instant.elapsed(),
                                MIN_WATCHER_RECONCILE_SPACING,
                            ) {
                                tokio::time::sleep(remaining).await;
                                while watch_rx.try_recv().is_ok() {}
                            }
                        }
                        _ = interval.tick() => {}
                    }
```

And find this line, later in the same loop (right after the `spawn_blocking` reconcile call):

```rust
                    last_poll_at = Utc::now();
                    push_snapshot(&handle, &store).await;
```

Replace it with:

```rust
                    last_poll_at = Utc::now();
                    last_reconcile_instant = std::time::Instant::now();
                    push_snapshot(&handle, &store).await;
```

- [ ] **Step 4: Run the tests and build**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including the three new `watcher_reconcile_delay` tests and the pre-existing `a_short_gap_is_not_a_wake`/`a_gap_past_the_threshold_is_a_wake` tests (unaffected — `is_wake_gap` and `last_poll_at` are untouched by this change).

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: succeeds.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "Coalesce watcher-triggered reconciles to a 1s minimum spacing"
```

---

### Task 4: End-to-end proof and final verification

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `Db::summary_updated_at` (test-only, from Task 1), `reconcile_and_persist` (existing, unchanged signature).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/commands.rs`, after the existing `reconcile_and_persist_reports_storage_health_degraded_when_in_memory_fallback` test:

```rust
    #[test]
    fn reconcile_and_persist_writes_nothing_new_when_nothing_changed() {
        use crate::observer::db::Db;

        let root = std::env::temp_dir().join(format!(
            "aperture-noop-reconcile-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("s.jsonl");
        let line = serde_json::json!({
            "type":"user","sessionId":"noop","timestamp":Utc::now().to_rfc3339(),
            "message":{"content":"hi"}
        })
        .to_string();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let mut observer = Observer::new(root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        let db = Db::open(&root.join("noop.db")).unwrap();

        reconcile_and_persist(&mut observer, &mut store, &db);
        assert_eq!(db.load_summaries().unwrap().len(), 1);
        let updated_at_after_first = db.summary_updated_at("claude_code:noop").unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        reconcile_and_persist(&mut observer, &mut store, &db);
        let updated_at_after_second = db.summary_updated_at("claude_code:noop").unwrap();

        assert_eq!(
            updated_at_after_first, updated_at_after_second,
            "a second reconcile with no underlying file change must not rewrite the summary row"
        );

        drop(db);
        std::fs::remove_dir_all(&root).unwrap();
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml reconcile_and_persist_writes_nothing_new -- --nocapture`
Expected: FAIL — either a compile error if `summary_updated_at` isn't `pub(crate)`-visible here (it is, from Task 1 — this should actually compile and run), or, if Task 1/2/3 are already merged by the time this task runs, the test should PASS immediately since the underlying mechanism is already in place. Either outcome is fine: if it fails to compile, something regressed in Task 1 and must be fixed before proceeding; if it passes immediately, that's expected confirmation the mechanism works end-to-end through the real `reconcile_and_persist` call path, not just at the `Db` unit level.

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass — this is the full regression check across all four tasks in this plan plus every pre-existing test in the repo (the durable-observation-recovery suite this plan is a follow-up to).

- [ ] **Step 4: Final build and frontend check**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: succeeds, no warnings.

Run: `npm run build`
Expected: succeeds — this plan touches no `.ts`/`.tsx` files, so this is a sanity check that nothing was accidentally broken, not an expected source of changes.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "Add end-to-end proof that unchanged reconciles write nothing new"
```

---

## Self-Review Notes

- **Spec coverage:** content-aware writes for both summaries and cursors (Task 1), watcher-reconcile coalescing with the pure `watcher_reconcile_delay` sub-logic the spec called for (Task 3), cursor retention via `prune_cursors` wired into startup (Task 2), and the end-to-end integration proof through the real `reconcile_and_persist` call path (Task 4) are each covered. No schema migration is introduced anywhere in this plan, matching the spec. The two explicit non-goals (in-memory `Store` eviction, `Session` serde forward-compatibility) are untouched by every task.
- **Type consistency:** `ConnState`/`Db::state` introduced in Task 1 is consumed identically in Task 2 (`prune_cursors`) and Task 4 (`summary_updated_at`, a Task 1 interface). `watcher_reconcile_delay`'s signature (Task 3) is defined and used consistently within that single task — nothing downstream depends on it. `prune_cursors(&self, days: i64) -> rusqlite::Result<usize>` matches `prune_summaries`'s existing shape exactly, as required.
