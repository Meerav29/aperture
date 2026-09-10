# Durable Observation and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Aperture a SQLite-backed store for session summaries and file
read cursors, replace the fixed 2-second poll with debounced filesystem
watching, and make restart/sleep-wake recovery never double-count history or
present a stale session as live.

**Architecture:** A new `observer::db` module owns one SQLite connection
(bundled, no system dependency) behind a mutex, touched only from
`spawn_blocking` tasks — the same pattern the existing `Observer` already
uses. `observer::watch` wraps `notify-debouncer-mini` to turn filesystem
changes into a debounced wake signal. `lib.rs`'s setup loop moves from a
blind `sleep(2s)` to `tokio::select!` between that signal and a 5-second
interval, persisting summaries + cursors after every poll in the same
blocking task (single writer, no extra locking). On startup, persisted
summaries are loaded and forced to `live: false`/stale before the first
snapshot is pushed, and persisted cursors seed `Observer` so already-ingested
transcript lines are never re-read.

**Tech Stack:** Rust, Tauri 2, tokio, rusqlite (bundled SQLite),
notify-debouncer-mini, chrono, serde_json.

**Spec:** [docs/superpowers/specs/2026-09-09-durable-observation-recovery-design.md](../specs/2026-09-09-durable-observation-recovery-design.md)

## Global Constraints

- Never trust a persisted `live: true` on restore — always force `live: false`
  and downgrade `observation: "recent"` to `"stale"` when loading a summary
  from disk (spec: "Recovery semantics").
- Cursors resume at their saved byte offset; never re-scan a tailed file from
  offset zero unless the existing replaced-file detection in
  `read_file` (`src-tauri/src/observer/passive.rs`) says the file was
  truncated or replaced.
- SQLite access happens only inside `spawn_blocking` closures, through a
  single `Mutex<rusqlite::Connection>` — no second writer path.
- A DB open/migration failure must never crash the app or destroy the
  original file; fall back to an in-memory `Db` for that run and log to
  stderr.
- The 5-second reconcile interval is a strict superset of the spec's 30s
  watcher-fallback requirement (`poll()` already re-walks the full directory
  tree every call) — do not add a second fallback timer.
- No frontend changes are needed or in scope; storage health reuses the
  existing generic `integrations: IntegrationHealth[]` shape with a
  synthetic `provider: "storage"` row.
- Run `cargo test --manifest-path src-tauri/Cargo.toml --locked` and
  `npm run build` before considering any task done, per `AGENTS.md`.

---

### Task 1: `observer::db` — schema, migrations, and summary/cursor persistence

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `rusqlite`, dev-dependency none needed)
- Create: `src-tauri/src/observer/db.rs`
- Modify: `src-tauri/src/observer/mod.rs` (add `pub mod db;`)

**Interfaces:**
- Produces: `pub struct Db`, `pub struct CursorRecord { pub path: String, pub provider: String, pub offset: u64, pub initial_len: u64, pub malformed: u64, pub session_id: Option<String>, pub host: Option<String>, pub created_ns: Option<i64> }`, `pub fn data_dir() -> PathBuf`, `impl Db { pub fn open(path: &Path) -> rusqlite::Result<Self>; pub fn in_memory() -> Self; pub fn save_summaries(&self, sessions: &[Session]) -> rusqlite::Result<()>; pub fn load_summaries(&self) -> rusqlite::Result<Vec<Session>>; pub fn save_cursors(&self, cursors: &[CursorRecord]) -> rusqlite::Result<()>; pub fn load_cursors(&self) -> rusqlite::Result<Vec<CursorRecord>>; pub fn prune_summaries(&self, days: i64) -> rusqlite::Result<usize>; }`
- Consumes: `super::model::Session` (already `Serialize`/`Deserialize`).

- [ ] **Step 1: Add the `rusqlite` dependency**

Edit `src-tauri/Cargo.toml`, in the `[dependencies]` section, add after the
`open = "5"` line:

```toml
rusqlite = { version = "0.31", features = ["bundled"] }
```

- [ ] **Step 2: Update the lockfile**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: succeeds, `src-tauri/Cargo.lock` is updated with the new
dependency tree. Commit the lockfile change together with this task.

- [ ] **Step 3: Write the failing tests for `db.rs`**

Create `src-tauri/src/observer/db.rs` with just the module doc, imports, and
the test module below (implementation comes in the next step, so these fail
to compile — that's the expected "red" state for this step):

```rust
//! SQLite-backed persistence for session summaries and per-file read
//! cursors. One connection, guarded by a mutex, touched only from blocking
//! tasks — see `lib.rs`'s reconcile loop for the single-writer contract.

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

pub struct Db {
    conn: Mutex<Connection>,
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
            conn: Mutex::new(conn),
        })
    }

    pub fn in_memory() -> Self {
        let conn = Connection::open_in_memory().expect("open in-memory sqlite");
        migrate(&conn, Path::new(":memory:"), MIGRATIONS).expect("migrate in-memory sqlite");
        Db {
            conn: Mutex::new(conn),
        }
    }

    pub fn save_summaries(&self, sessions: &[Session]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        for s in sessions {
            let data = serde_json::to_string(s).expect("Session serializes");
            tx.execute(
                "INSERT INTO session_summaries (id, data, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
                params![s.id, data, now],
            )?;
        }
        tx.commit()
    }

    pub fn load_summaries(&self) -> rusqlite::Result<Vec<Session>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare("SELECT data FROM session_summaries")?;
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
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        for c in cursors {
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
        tx.commit()
    }

    pub fn load_cursors(&self) -> rusqlite::Result<Vec<CursorRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
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
        let conn = self.conn.lock().expect("db lock");
        let cutoff = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        conn.execute(
            "DELETE FROM session_summaries WHERE updated_at < ?1",
            params![cutoff],
        )
    }
}

/// Apply any migrations in `migrations` beyond `PRAGMA user_version`, each
/// inside its own transaction. Before the first pending migration, back up
/// `path` to `<path>.bak` (skipped for `:memory:`). If a migration's
/// transaction fails it rolls back automatically, so the connection (and
/// on-disk file) are left at the last successfully applied version.
fn migrate(conn: &Connection, path: &Path, migrations: &[&str]) -> rusqlite::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let version = version as usize;
    if version >= migrations.len() {
        return Ok(());
    }
    if path != Path::new(":memory:") && path.exists() {
        let backup = PathBuf::from(format!("{}.bak", path.display()));
        let _ = std::fs::copy(path, backup);
    }
    for (i, sql) in migrations.iter().enumerate().skip(version) {
        conn.execute_batch(&format!("BEGIN;\n{sql}\nCOMMIT;"))?;
        conn.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc as ChronoUtc;

    fn sample_session(id: &str) -> Session {
        let mut s = Session::new(id.into(), "/repo".into(), ChronoUtc::now());
        s.native_id = id.into();
        s
    }

    #[test]
    fn summaries_round_trip() {
        let db = Db::in_memory();
        let sessions = vec![sample_session("claude_code:a"), sample_session("codex:b")];
        db.save_summaries(&sessions).unwrap();
        let mut loaded = db.load_summaries().unwrap();
        loaded.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "claude_code:a");
        assert_eq!(loaded[1].id, "codex:b");
    }

    #[test]
    fn saving_the_same_summary_id_twice_upserts_not_duplicates() {
        let db = Db::in_memory();
        let mut s = sample_session("claude_code:a");
        db.save_summaries(&[s.clone()]).unwrap();
        s.status = crate::observer::model::SessionStatus::Working;
        db.save_summaries(&[s]).unwrap();
        let loaded = db.load_summaries().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, crate::observer::model::SessionStatus::Working);
    }

    #[test]
    fn cursors_round_trip() {
        let db = Db::in_memory();
        let record = CursorRecord {
            path: "C:/fixture/session.jsonl".into(),
            provider: "claude_code".into(),
            offset: 1234,
            initial_len: 1000,
            malformed: 2,
            session_id: Some("abc".into()),
            host: Some("terminal".into()),
            created_ns: Some(555),
        };
        db.save_cursors(&[record.clone()]).unwrap();
        let loaded = db.load_cursors().unwrap();
        assert_eq!(loaded, vec![record]);
    }

    #[test]
    fn migrations_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("aperture-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("idempotent.db");
        let _ = std::fs::remove_file(&path);
        {
            let db = Db::open(&path).unwrap();
            db.save_summaries(&[sample_session("claude_code:a")]).unwrap();
        }
        // Reopening re-runs `migrate`, which must be a no-op against an
        // already-migrated file and must not lose existing rows.
        let db = Db::open(&path).unwrap();
        assert_eq!(db.load_summaries().unwrap().len(), 1);
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(dir.join("idempotent.db.bak"));
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn a_failed_migration_leaves_the_original_file_byte_identical() {
        let dir = std::env::temp_dir().join(format!("aperture-db-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.db");
        let _ = std::fs::remove_file(&path);

        let conn = Connection::open(&path).unwrap();
        migrate(&conn, &path, &["CREATE TABLE ok (x INTEGER);"]).unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();

        let conn = Connection::open(&path).unwrap();
        let broken = ["CREATE TABLE ok (x INTEGER);", "THIS IS NOT VALID SQL;"];
        let result = migrate(&conn, &path, &broken);
        assert!(result.is_err());
        drop(conn);

        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after);
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(dir.join("broken.db.bak"));
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn prune_removes_only_old_summaries() {
        let db = Db::in_memory();
        db.save_summaries(&[sample_session("claude_code:fresh")])
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            let old = (ChronoUtc::now() - chrono::Duration::days(200)).to_rfc3339();
            conn.execute(
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
}
```

Also create the migration file `src-tauri/src/observer/migrations/0001_init.sql`:

```sql
CREATE TABLE session_summaries (
    id TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE file_cursors (
    path TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    offset INTEGER NOT NULL,
    initial_len INTEGER NOT NULL,
    malformed INTEGER NOT NULL,
    session_id TEXT,
    host TEXT,
    created_ns INTEGER,
    updated_at TEXT NOT NULL
);
```

Add `pub mod db;` to `src-tauri/src/observer/mod.rs`'s module list (alongside
the existing `pub mod` lines, alphabetically after `state`... actually keep
the existing order and add it after `hook_payload`, matching alphabetical
placement: `hook_bridge, hook_payload, db` is not alphabetical — insert
`pub mod db;` as the first line of the list since `d` < `h`).

- [ ] **Step 4: Run the tests to see them fail/not compile**

Run: `cargo test --manifest-path src-tauri/Cargo.toml db::tests -- --nocapture`
Expected: compiles and passes, since the implementation was written in the
same step as the tests above (this codebase's convention, matching the
existing test modules in `passive.rs`/`hook_bridge.rs`, is implementation
and its test module together in one file). If it does not compile, fix the
implementation now before proceeding — do not move to Step 5 with a red build.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including the five new `db::tests::*` tests and
every pre-existing test.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/observer/db.rs src-tauri/src/observer/migrations/0001_init.sql src-tauri/src/observer/mod.rs
git commit -m "Add SQLite-backed session summary and cursor storage"
```

---

### Task 2: `Store::restore_summaries` — never trust a persisted `live`

**Files:**
- Modify: `src-tauri/src/observer/state.rs`

**Interfaces:**
- Consumes: `super::model::Session` (unchanged).
- Produces: `impl Store { pub fn restore_summaries(&mut self, sessions: Vec<Session>); }`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/observer/state.rs`:

```rust
    #[test]
    fn restore_forces_live_false_and_downgrades_recent_to_stale() {
        let mut st = Store::default();
        let mut recent = Session::new("claude_code:a".into(), "/repo".into(), Utc::now());
        recent.live = true;
        recent.observation = "recent".into();
        let mut history = Session::new("codex:b".into(), "/repo".into(), Utc::now());
        history.live = true; // a persisted bug/edge case; must still be forced false
        history.observation = "history_only".into();

        st.restore_summaries(vec![recent, history]);

        let a = &st.sessions["claude_code:a"];
        assert!(!a.live);
        assert_eq!(a.observation, "stale");
        let b = &st.sessions["codex:b"];
        assert!(!b.live);
        assert_eq!(b.observation, "history_only");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml restore_forces_live_false -- --nocapture`
Expected: FAIL with "no method named `restore_summaries` found"

- [ ] **Step 3: Implement `restore_summaries`**

Add to `impl Store` in `src-tauri/src/observer/state.rs`, after the existing
`remove` method:

```rust
    /// Seed the store from persisted history on startup. Never trust a
    /// persisted `live` claim: force it false and downgrade a "recent"
    /// observation to "stale" so nothing is presented as currently live
    /// before the first post-restart reconcile.
    pub fn restore_summaries(&mut self, sessions: Vec<Session>) {
        for mut s in sessions {
            s.live = false;
            if s.observation == "recent" {
                s.observation = "stale".into();
            }
            self.sessions.insert(s.id.clone(), s);
        }
        self.revision += 1;
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including the new one.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/observer/state.rs
git commit -m "Add Store::restore_summaries for restart recovery"
```

---

### Task 3: `Observer` cursor export/import and watch roots

**Files:**
- Modify: `src-tauri/src/observer/passive.rs`

**Interfaces:**
- Consumes: `super::db::CursorRecord` (from Task 1), `super::model::Host`.
- Produces: `impl Observer { pub fn restore_cursors(&mut self, records: Vec<CursorRecord>); pub fn export_cursors(&self) -> Vec<CursorRecord>; pub fn watch_roots(&self) -> Vec<PathBuf>; }`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/observer/passive.rs`:

```rust
    #[test]
    fn cursor_export_then_restore_round_trips_and_resumes_tailing() {
        let root = std::env::temp_dir().join(format!("aperture-cursor-rt-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.jsonl");
        let line = json!({"type":"user","sessionId":"resume","timestamp":Utc::now().to_rfc3339(),"message":{"content":"hi"}}).to_string();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let mut first = Observer::new(root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        first.poll(&mut store);
        assert_eq!(store.sessions.len(), 1);
        let exported = first.export_cursors();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].provider, "claude_code");
        assert!(exported[0].offset > 0);

        // A fresh Observer, as after a restart, resumes from the exported
        // cursor instead of re-reading from byte zero.
        let mut second = Observer::new(root.clone(), root.join("codex-unused"));
        second.restore_cursors(exported.clone());
        let mut second_store = Store::default();
        second.poll(&mut second_store);
        // No new lines were appended, so re-polling must not re-apply the
        // already-ingested line as a fresh event / duplicate session entry.
        assert_eq!(second_store.sessions.len(), 0);

        let still = second.export_cursors();
        assert_eq!(still[0].offset, exported[0].offset);

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn watch_roots_returns_configured_provider_directories() {
        let claude = std::env::temp_dir().join("aperture-watch-claude");
        let codex = std::env::temp_dir().join("aperture-watch-codex");
        let observer = Observer::new(claude.clone(), codex.clone());
        let roots = observer.watch_roots();
        assert_eq!(roots, vec![claude, codex]);
    }
```

Add `use serde_json::json;` to the test module's imports if not already
present (it already is, via `use serde_json::json;` at the top of the
existing `#[cfg(test)] mod tests` block — verify before adding a duplicate).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cursor_export_then_restore -- --nocapture`
Expected: FAIL with "no method named `export_cursors`/`restore_cursors`/`watch_roots` found"

- [ ] **Step 3: Implement the methods**

Add to `impl Observer` in `src-tauri/src/observer/passive.rs`, after the
existing `poll` method:

```rust
    /// Provider transcript roots, for the filesystem watcher to subscribe to.
    pub fn watch_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|(_, p)| p.clone()).collect()
    }

    /// Seed cursors from persisted state (Task 1's `db::CursorRecord`), so
    /// tailing resumes at the saved byte offset instead of re-reading a file
    /// from the start after a restart.
    pub fn restore_cursors(&mut self, records: Vec<super::db::CursorRecord>) {
        for r in records {
            self.cursors.insert(
                PathBuf::from(&r.path),
                Cursor {
                    offset: r.offset,
                    initial_len: r.initial_len,
                    malformed: r.malformed as usize,
                    id: r.session_id,
                    host: r.host.as_deref().map(host_from_str),
                    created: r
                        .created_ns
                        .map(|ns| std::time::UNIX_EPOCH + std::time::Duration::from_nanos(ns as u64)),
                },
            );
        }
    }

    /// Export current cursor state for persistence.
    pub fn export_cursors(&self) -> Vec<super::db::CursorRecord> {
        self.cursors
            .iter()
            .map(|(path, c)| super::db::CursorRecord {
                path: path.to_string_lossy().into_owned(),
                provider: self.provider_for(path).unwrap_or_default(),
                offset: c.offset,
                initial_len: c.initial_len,
                malformed: c.malformed as u64,
                session_id: c.id.clone(),
                host: c.host.map(host_to_str),
                created_ns: c
                    .created
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64),
            })
            .collect()
    }

    fn provider_for(&self, path: &Path) -> Option<String> {
        self.roots
            .iter()
            .find(|(_, root)| path.starts_with(root))
            .map(|(p, _)| p.clone())
    }
```

Add these two free functions near the bottom of the file, after `safe_name`:

```rust
fn host_to_str(h: Host) -> String {
    match serde_json::to_value(h) {
        Ok(Value::String(s)) => s,
        _ => "unknown".into(),
    }
}

fn host_from_str(s: &str) -> Host {
    serde_json::from_value(Value::String(s.into())).unwrap_or(Host::Unknown)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/observer/passive.rs
git commit -m "Add Observer cursor export/import for restart recovery"
```

---

### Task 4: `observer::watch` — debounced filesystem watcher

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `notify`, `notify-debouncer-mini`)
- Create: `src-tauri/src/observer/watch.rs`
- Modify: `src-tauri/src/observer/mod.rs` (add `pub mod watch;`)

**Interfaces:**
- Produces: `pub struct Watcher` (RAII handle — drop stops watching), `pub fn watch(roots: &[PathBuf], tx: tokio::sync::mpsc::UnboundedSender<()>) -> Watcher`

- [ ] **Step 1: Add the watcher dependencies**

Edit `src-tauri/Cargo.toml`, add after the `rusqlite` line added in Task 1:

```toml
notify = "6"
notify-debouncer-mini = "0.4"
```

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: succeeds, lockfile updated.

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/observer/watch.rs`:

```rust
//! Debounced filesystem watching for provider transcript roots. This emits
//! a plain wake signal, not paths — `Observer::poll()` already re-discovers
//! whatever changed on every call, so callers only need to know "something
//! changed, poll again."

use notify::RecommendedWatcher;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// RAII handle: dropping this stops the underlying watcher.
pub struct Watcher {
    _debouncer: Debouncer<RecommendedWatcher>,
}

/// Watch `roots` for `.jsonl` changes, debounced by 250ms, sending `()` on
/// `tx` for each debounced batch. A root that does not exist yet (a provider
/// that is not installed) is skipped, not an error.
pub fn watch(roots: &[PathBuf], tx: UnboundedSender<()>) -> Watcher {
    let mut debouncer = new_debouncer(Duration::from_millis(250), move |res: DebounceEventResult| {
        let Ok(events) = res else { return };
        let relevant = events
            .iter()
            .any(|e| e.path.extension().and_then(|s| s.to_str()) == Some("jsonl"));
        if relevant {
            let _ = tx.send(());
        }
    })
    .expect("create fs watcher");
    for root in roots {
        if root.exists() {
            let _ = debouncer
                .watcher()
                .watch(root, notify::RecursiveMode::Recursive);
        }
    }
    Watcher {
        _debouncer: debouncer,
    }
}

fn _unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    #[tokio::test]
    async fn a_jsonl_write_under_a_watched_root_sends_a_signal() {
        let root = std::env::temp_dir().join(format!("aperture-watch-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _watcher = watch(&[root.clone()], tx);

        // Give the watcher a moment to register before writing.
        tokio::time::sleep(StdDuration::from_millis(100)).await;
        std::fs::write(root.join("session.jsonl"), b"{}\n").unwrap();

        let signal = tokio::time::timeout(StdDuration::from_secs(5), rx.recv()).await;
        assert!(signal.is_ok(), "expected a debounced signal within 5s");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn a_non_jsonl_write_does_not_send_a_signal() {
        let root = std::env::temp_dir().join(format!("aperture-watch-ignore-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _watcher = watch(&[root.clone()], tx);

        tokio::time::sleep(StdDuration::from_millis(100)).await;
        std::fs::write(root.join("notes.txt"), b"irrelevant").unwrap();

        let signal = tokio::time::timeout(StdDuration::from_millis(800), rx.recv()).await;
        assert!(signal.is_err(), "a non-.jsonl write should not trigger a signal");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
```

Remove the placeholder `fn _unused(_: &Path) {}` — it was only listed above
to keep the `use std::path::Path` import from looking unused while drafting;
delete both the function and the `Path` half of the `use std::path::{Path,
PathBuf};` line, leaving `use std::path::PathBuf;`.

Add `pub mod watch;` to `src-tauri/src/observer/mod.rs`.

- [ ] **Step 3: Run the tests to verify the signal test fails first, if the module didn't exist**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked -p aperture watch::tests`
Expected: PASS for both tests (implementation was written alongside the
tests, matching this codebase's convention). If `new_debouncer`'s signature
doesn't match what's installed (crate API drift), the compiler error will
name the mismatch — adjust the closure/`Debouncer` type parameters to match
the installed `notify-debouncer-mini` version's actual signature and rerun.

- [ ] **Step 4: Run the full suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/observer/watch.rs src-tauri/src/observer/mod.rs
git commit -m "Add debounced filesystem watcher for provider transcript roots"
```

---

### Task 5: `commands::reconcile_and_persist` and `Shared.db`

**Files:**
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `observer::db::Db` (Task 1), `observer::passive::Observer::{poll, export_cursors}` (Task 3), `observer::state::Store::snapshot`.
- Produces: `pub struct Shared { pub store: Arc<Mutex<Store>>, pub observer: Arc<StdMutex<Observer>>, pub db: Arc<Db> }`, `pub fn reconcile_and_persist(observer: &mut Observer, store: &mut Store, db: &Db)`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/commands.rs`:

```rust
    #[test]
    fn reconcile_and_persist_polls_and_writes_summaries_and_cursors_to_the_db() {
        use crate::observer::db::Db;

        let root = std::env::temp_dir().join(format!("aperture-reconcile-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("s.jsonl");
        let line = serde_json::json!({
            "type":"user","sessionId":"r1","timestamp":Utc::now().to_rfc3339(),
            "message":{"content":"hi"}
        })
        .to_string();
        std::fs::write(&path, format!("{line}\n")).unwrap();

        let mut observer = Observer::new(root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        let db = Db::in_memory();

        reconcile_and_persist(&mut observer, &mut store, &db);

        assert_eq!(store.snapshot().sessions.len(), 1);
        let persisted = db.load_summaries().unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].id, "claude_code:r1");
        let cursors = db.load_cursors().unwrap();
        assert_eq!(cursors.len(), 1);
        assert!(cursors[0].offset > 0);

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }
```

Add `use crate::observer::passive::Observer;` and `use chrono::Utc;` to the
test module's imports if not already present.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml reconcile_and_persist_polls -- --nocapture`
Expected: FAIL with "cannot find function `reconcile_and_persist`"

- [ ] **Step 3: Implement `reconcile_and_persist` and update `Shared`**

In `src-tauri/src/commands.rs`, update the imports and `Shared` struct:

```rust
use crate::observer::{
    db::Db,
    model::{Session, Snapshot},
    passive::Observer,
    state::Store,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

pub struct Shared {
    pub store: Arc<Mutex<Store>>,
    pub observer: Arc<StdMutex<Observer>>,
    pub db: Arc<Db>,
}

/// Poll for new activity, then persist the resulting summaries and cursors.
/// This is the single write path into SQLite — called from `lib.rs`'s
/// reconcile loop and from the manual `rescan_transcripts` command, always
/// from inside a `spawn_blocking` closure holding both locks.
pub fn reconcile_and_persist(observer: &mut Observer, store: &mut Store, db: &Db) {
    observer.poll(store);
    let sessions = store.snapshot().sessions;
    let cursors = observer.export_cursors();
    let _ = db.save_summaries(&sessions);
    let _ = db.save_cursors(&cursors);
}
```

Update `rescan_transcripts` to use it and persist after a manual rescan:

```rust
#[tauri::command]
pub async fn rescan_transcripts(
    app: AppHandle,
    shared: State<'_, Shared>,
) -> Result<usize, String> {
    let observer = shared.observer.clone();
    let store = shared.store.clone();
    let db = shared.db.clone();
    tokio::task::spawn_blocking(move || {
        let mut observer_guard = observer.lock().map_err(|e| e.to_string())?;
        let mut store_guard = store.blocking_lock();
        reconcile_and_persist(&mut observer_guard, &mut store_guard, &db);
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;
    push_snapshot(&app, &shared.store).await;
    Ok(shared.store.lock().await.snapshot().sessions.len())
}
```

The rest of `commands.rs` (`get_snapshot`, `open_session_folder`,
`reveal_transcript`, `find_session`, `transcript_dir`) is unchanged.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass. Note this will not yet compile cleanly end-to-end
because `lib.rs` still constructs `Shared` without a `db` field — that's
fixed in Task 6. If `cargo test` fails to build the `aperture-lib` crate at
this point because of `lib.rs`, that is expected; proceed to Task 6
immediately rather than trying to make Task 5 build in isolation.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "Add reconcile_and_persist and wire Shared.db"
```

---

### Task 6: Wire recovery and the watch/reconcile loop into `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `observer::db::{Db, data_dir}` (Task 1), `Store::restore_summaries` (Task 2), `Observer::{restore_cursors, watch_roots}` (Task 3), `observer::watch::watch` (Task 4), `commands::{Shared, reconcile_and_persist}` (Task 5).
- Produces: `pub(crate) fn is_wake_gap(last: DateTime<Utc>, now: DateTime<Utc>) -> bool` (sleep/wake heuristic, unit-tested here since `lib.rs` is where it is used).

- [ ] **Step 1: Write the failing test for the sleep/wake heuristic**

`lib.rs` currently has no test module. Add one at the bottom of
`src-tauri/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn a_short_gap_is_not_a_wake() {
        let last = chrono::Utc::now();
        let now = last + Duration::seconds(10);
        assert!(!is_wake_gap(last, now));
    }

    #[test]
    fn a_gap_past_the_threshold_is_a_wake() {
        let last = chrono::Utc::now();
        let now = last + Duration::seconds(SLEEP_WAKE_THRESHOLD_SECS + 1);
        assert!(is_wake_gap(last, now));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml is_wake_gap -- --nocapture`
Expected: FAIL with "cannot find function `is_wake_gap`" / "cannot find
value `SLEEP_WAKE_THRESHOLD_SECS`"

- [ ] **Step 3: Rewrite `lib.rs`**

Replace the full contents of `src-tauri/src/lib.rs`:

```rust
mod commands;
pub mod observer;

use chrono::{DateTime, Utc};
use commands::{push_snapshot, reconcile_and_persist, Shared};
use observer::{db::Db, passive::Observer, state::Store, watch};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex};

const RECONCILE_INTERVAL_SECS: u64 = 5;
const SLEEP_WAKE_THRESHOLD_SECS: i64 = 90;
const SUMMARY_RETENTION_DAYS: i64 = 90;

/// A wall-clock gap this large between reconciles means the process was
/// almost certainly suspended (OS sleep, laptop lid close) rather than just
/// busy — the loop below wakes on either a watcher signal or a 5s interval,
/// so a real 90s+ gap can only come from lost wall-clock time, not scheduling
/// jitter. No OS-specific power-event API is used; see the design doc's
/// non-goals.
fn is_wake_gap(last: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    (now - last).num_seconds() > SLEEP_WAKE_THRESHOLD_SECS
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = observer::db::data_dir().join("aperture.db");
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Aperture: database at {} unavailable ({e}); running in-memory only this session",
                db_path.display()
            );
            Db::in_memory()
        }
    };
    let _ = db.prune_summaries(SUMMARY_RETENTION_DAYS);

    let mut store = Store::default();
    if let Ok(sessions) = db.load_summaries() {
        store.restore_summaries(sessions);
    }

    let mut observer = Observer::default();
    if let Ok(cursors) = db.load_cursors() {
        observer.restore_cursors(cursors);
    }

    let db = Arc::new(db);
    let store = Arc::new(Mutex::new(store));
    let observer = Arc::new(StdMutex::new(observer));

    tauri::Builder::default()
        .manage(Shared {
            store: store.clone(),
            observer: observer.clone(),
            db: db.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::rescan_transcripts,
            commands::open_session_folder,
            commands::reveal_transcript
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Restored summaries are already stale/history_only and
                // live:false (Store::restore_summaries) — push them now so
                // the UI shows prior context before the first reconcile.
                push_snapshot(&handle, &store).await;

                let (tx, mut watch_rx) = mpsc::unbounded_channel();
                let roots = observer.lock().expect("observer lock").watch_roots();
                let _watcher = watch::watch(&roots, tx);

                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
                let mut last_poll_at = Utc::now();

                loop {
                    tokio::select! {
                        _ = watch_rx.recv() => {}
                        _ = interval.tick() => {}
                    }

                    let now = Utc::now();
                    if is_wake_gap(last_poll_at, now) {
                        eprintln!(
                            "Aperture: {}s since the last reconcile; forcing full reconciliation",
                            (now - last_poll_at).num_seconds()
                        );
                    }

                    let s = store.clone();
                    let o = observer.clone();
                    let d = db.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        let mut observer_guard = o.lock().expect("observer lock");
                        let mut store_guard = s.blocking_lock();
                        reconcile_and_persist(&mut observer_guard, &mut store_guard, &d);
                    })
                    .await
                    {
                        eprintln!("Observer failed: {e}");
                    }
                    last_poll_at = Utc::now();
                    push_snapshot(&handle, &store).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aperture");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn a_short_gap_is_not_a_wake() {
        let last = chrono::Utc::now();
        let now = last + Duration::seconds(10);
        assert!(!is_wake_gap(last, now));
    }

    #[test]
    fn a_gap_past_the_threshold_is_a_wake() {
        let last = chrono::Utc::now();
        let now = last + Duration::seconds(SLEEP_WAKE_THRESHOLD_SECS + 1);
        assert!(is_wake_gap(last, now));
    }
}
```

(This supersedes the test module stub added in Step 1 — it's included in
full here so the file is complete and correct in one piece.)

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, across every module touched in Tasks 1-6.

- [ ] **Step 5: Build the app binary**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: succeeds with no errors.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "Wire durable recovery and watch/reconcile loop into app startup"
```

---

### Task 7: Two-run restart/recovery integration test

**Files:**
- Create: `src-tauri/tests/durable_recovery.rs`

**Interfaces:**
- Consumes: `aperture_lib::observer::{db::Db, passive::Observer, state::Store}` (all prior tasks) as a black box, exactly as a real restart would use them.

- [ ] **Step 1: Write the test**

Create `src-tauri/tests/durable_recovery.rs`:

```rust
//! Simulates an app restart against real files and a real (file-backed)
//! SQLite database: ingest, persist, drop everything, reconstruct fresh
//! instances, append more, and verify no duplication or data loss.
use aperture_lib::observer::{db::Db, passive::Observer, state::Store};
use std::io::Write;

fn append_line(path: &std::path::Path, json: &serde_json::Value) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{json}").unwrap();
}

#[test]
fn restart_resumes_cursors_without_duplicating_or_losing_sessions() {
    let root = std::env::temp_dir().join(format!("aperture-restart-test-{}", std::process::id()));
    let claude_root = root.join("claude");
    std::fs::create_dir_all(&claude_root).unwrap();
    let db_path = root.join("aperture.db");
    let file_path = claude_root.join("session.jsonl");
    std::fs::write(&file_path, b"").unwrap();

    let first_line = serde_json::json!({
        "type": "user",
        "sessionId": "restart-1",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "message": {"content": "first"}
    });
    append_line(&file_path, &first_line);

    // --- "Run 1": ingest, persist, then drop everything. ---
    {
        let db = Db::open(&db_path).unwrap();
        let mut observer = Observer::new(claude_root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        observer.poll(&mut store);
        assert_eq!(store.snapshot().sessions.len(), 1, "run 1 should see the first line");

        let sessions = store.snapshot().sessions;
        db.save_summaries(&sessions).unwrap();
        db.save_cursors(&observer.export_cursors()).unwrap();
        // observer, store, and db all drop here — nothing carries over except the files.
    }

    let assistant_line = serde_json::json!({
        "type": "assistant",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "message": {"content": [{"type": "text", "text": "reply"}], "stop_reason": "end_turn"}
    });
    append_line(&file_path, &assistant_line);

    // --- "Run 2": reconstruct fresh instances, as a real restart would. ---
    let db = Db::open(&db_path).unwrap();
    let mut observer = Observer::new(claude_root.clone(), root.join("codex-unused"));
    observer.restore_cursors(db.load_cursors().unwrap());
    let mut store = Store::default();
    store.restore_summaries(db.load_summaries().unwrap());

    let restored = &store.snapshot().sessions[0];
    assert!(!restored.live, "a restored session must never start out live");
    assert_eq!(restored.observation, "stale");

    observer.poll(&mut store);
    let snap = store.snapshot();
    assert_eq!(
        snap.sessions.len(),
        1,
        "restart must not duplicate the session as a second entry"
    );
    assert_eq!(
        snap.sessions[0].id, "claude_code:restart-1",
        "the same provider-qualified id must be reused across restart"
    );

    let cursors_after = observer.export_cursors();
    assert_eq!(cursors_after.len(), 1);
    let file_len = std::fs::metadata(&file_path).unwrap().len();
    assert_eq!(
        cursors_after[0].offset, file_len,
        "the cursor must have advanced past both lines, not re-read from zero"
    );

    std::fs::remove_dir_all(&root).unwrap();
}
```

- [ ] **Step 2: Run the test to verify it fails first (before this file existed, the target module paths didn't compile against a fresh checkout)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test durable_recovery -- --nocapture`
Expected: PASS. (As with the other tasks in this plan, the implementation
already exists from Tasks 1-6; this integration test is the first thing that
exercises them together end-to-end exactly as a restart would. If it fails,
the bug is in how Tasks 1-6 compose — fix the relevant task's code, not this
test, unless the test itself has a mistake.)

- [ ] **Step 3: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/durable_recovery.rs
git commit -m "Add end-to-end restart recovery integration test"
```

---

### Task 8: Storage health surfacing, final verification, and docs

**Files:**
- Modify: `src-tauri/src/observer/passive.rs` (`Observer::poll`, `IntegrationHealth` push)
- Modify: `docs/goals.md`
- Modify: `docs/specification.md` (current-state table row)

**Interfaces:**
- Consumes: `model::IntegrationHealth` (unchanged shape).
- Produces: nothing new — this task only adds a synthetic health row and updates docs; no new public functions.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/observer/passive.rs`'s test module:

```rust
    #[test]
    fn poll_reports_storage_health_as_a_synthetic_integration() {
        let root = std::env::temp_dir().join(format!("aperture-storage-health-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();
        observer.poll(&mut store);
        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "in_memory_only");
        std::fs::remove_dir_all(&root).unwrap();
    }
```

This exercises `Observer::poll` directly without a `Db`, so it must default
to reporting `"in_memory_only"` when no database handle has been attached —
which is the current state of `Observer::poll` after Tasks 1-7 (it never
touches `Db` itself; persistence happens in `commands::reconcile_and_persist`
after `poll` returns). Since `Observer` has no reference to `Db`, model this
honestly: `Observer` cannot know whether persistence is working, only
`reconcile_and_persist` (Task 5) can, because that's the only place that
calls `db.save_summaries`/`db.save_cursors`. Move the health push there
instead of into `Observer::poll` — see Step 3.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml poll_reports_storage_health -- --nocapture`
Expected: FAIL — `store.integrations` has no `"storage"` entry yet.

Delete this test before proceeding to Step 3 — it tests the wrong location
(see the reasoning in Step 1) and is replaced by the test in Step 3's
`commands.rs` change, which correctly tests where the health signal actually
originates.

- [ ] **Step 3: Add storage health reporting to `reconcile_and_persist`**

In `src-tauri/src/commands.rs`, add a test to the `#[cfg(test)] mod tests`
block:

```rust
    #[test]
    fn reconcile_and_persist_reports_storage_health() {
        use crate::observer::db::Db;

        let root = std::env::temp_dir().join(format!("aperture-storage-health-cmd-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut observer = Observer::new(root.join("claude"), root.join("codex"));
        let mut store = Store::default();
        let db = Db::in_memory();

        reconcile_and_persist(&mut observer, &mut store, &db);

        let storage = store
            .integrations
            .iter()
            .find(|h| h.provider == "storage")
            .expect("a storage health row must be present");
        assert_eq!(storage.state, "ok");
        assert!(storage.detail.contains("SQLite"));

        std::fs::remove_dir_all(&root).unwrap();
    }
```

Run it to confirm it fails (`cannot find field` / assertion failure since no
row exists yet), then update `reconcile_and_persist` in
`src-tauri/src/commands.rs`:

```rust
pub fn reconcile_and_persist(observer: &mut Observer, store: &mut Store, db: &Db) {
    observer.poll(store);
    let sessions = store.snapshot().sessions;
    let cursors = observer.export_cursors();
    let summaries_ok = db.save_summaries(&sessions).is_ok();
    let cursors_ok = db.save_cursors(&cursors).is_ok();
    store.integrations.push(storage_health(summaries_ok && cursors_ok));
}

fn storage_health(ok: bool) -> crate::observer::model::IntegrationHealth {
    crate::observer::model::IntegrationHealth {
        provider: "storage".into(),
        state: if ok { "ok" } else { "degraded" }.into(),
        root: crate::observer::db::data_dir().to_string_lossy().into_owned(),
        files: 0,
        last_event_at: None,
        detail: if ok {
            "SQLite persistence writing normally.".into()
        } else {
            "SQLite write failed this cycle; running in-memory only until it recovers.".into()
        },
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --locked`
Expected: all tests pass, including `reconcile_and_persist_reports_storage_health`.
The earlier `reconcile_and_persist_polls_and_writes_summaries_and_cursors_to_the_db`
test from Task 5 must still pass unchanged.

- [ ] **Step 5: Frontend typecheck (no code change expected)**

Run: `npm run build`
Expected: succeeds. `IntegrationHealth` in `src/features/sessions/types.ts`
already matches the Rust shape field-for-field, and the frontend already
renders `snap.integrations` generically — no `.ts`/`.tsx` edits are needed
for the new `"storage"` row to appear.

- [ ] **Step 6: Update `docs/goals.md`**

In `docs/goals.md`, change the "Durable collection/recovery" row (line 14)
from:

```
| Durable collection/recovery | Planned | No SQLite/durable cursors yet |
```

to:

```
| Durable collection/recovery | Implemented | SQLite summaries/cursors, debounced watcher, restart/sleep-wake reconciliation; git identity and IPC/SessionKey revision remain planned |
```

- [ ] **Step 7: Update `docs/specification.md`**

In the "Current state and design delta" table (around line 19 and line 23),
update the "Ingestion" and "History" rows to reflect what now exists:

```
| Ingestion | Debounced filesystem watching plus 5s reconcile, bounded incremental JSONL reads for both providers | Optional attention enrichment |
| History | SQLite summaries and recoverable cursors; restart loads stale/history_only then reconciles | Retention/migration evidence at scale; git identity |
```

- [ ] **Step 8: Final full verification**

Run, in order:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

```bash
cargo build --manifest-path src-tauri/Cargo.toml
```

```bash
npm run build
```

Expected: all three succeed with no errors.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/commands.rs docs/goals.md docs/specification.md
git commit -m "Surface storage health and update docs for durable recovery"
```

---

## Self-Review Notes

- **Spec coverage:** SQLite storage (Task 1), no-double-counting via cursor
  resume (Tasks 1, 3, 7), never-live-on-restore (Tasks 2, 7), debounced
  watching (Task 4), 5s reconcile subsuming the 30s fallback (Task 6),
  sleep/wake heuristic (Task 6), single background writer (Tasks 5-6),
  migration backup/failure handling (Task 1), 90-day summary retention
  (Task 1), storage health surfacing via the existing `IntegrationHealth`
  shape (Task 8), and doc updates (Task 8) are each covered by a task.
  Git identity, SessionKey/IPC revision, and hook-inbox rework are
  out-of-scope per the approved design and untouched.
- **Type consistency:** `CursorRecord` (Task 1) is consumed identically in
  `Observer::restore_cursors`/`export_cursors` (Task 3),
  `reconcile_and_persist` (Task 5), and the integration test (Task 7).
  `Db::{save,load}_summaries`/`{save,load}_cursors` signatures are defined
  once in Task 1 and used verbatim thereafter. `is_wake_gap` and
  `SLEEP_WAKE_THRESHOLD_SECS` are defined and tested in the same task (6).
