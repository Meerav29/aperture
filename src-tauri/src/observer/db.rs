//! SQLite-backed persistence for session summaries and per-file read
//! cursors. One connection, guarded by a mutex, touched only from blocking
//! tasks — see `lib.rs`'s reconcile loop for the single-writer contract.

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
        // `PRAGMA user_version` is set inside the same transaction as the
        // DDL: SQLite treats both as transactional within an explicit
        // BEGIN/COMMIT, so a crash between them is impossible — either both
        // land or neither does. Setting the version as a separate statement
        // after COMMIT would leave a window where a crash commits the DDL
        // but not the version bump; the next launch would then re-run this
        // migration against an already-migrated schema and fail permanently
        // (e.g. "table already exists"), bricking the on-disk database.
        conn.execute_batch(&format!("BEGIN;\n{sql}\nPRAGMA user_version = {};\nCOMMIT;", i + 1))?;
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
        // Windows keeps the sqlite file locked while the connection is
        // open; drop it before deleting the file.
        drop(db);
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
    fn migrate_commits_user_version_and_schema_together() {
        // Regression test for the non-atomic version bump: `user_version`
        // must land in the same transaction as the DDL, so after a
        // successful `migrate` the two are always consistent — there's no
        // window where the schema exists but the version wasn't recorded
        // (or vice versa).
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, Path::new(":memory:"), MIGRATIONS).unwrap();

        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'session_summaries'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_exists, 1);

        // Re-running migrate against an already-migrated connection must be
        // a no-op, not an attempt to re-apply DDL that would now fail with
        // "table already exists".
        migrate(&conn, Path::new(":memory:"), MIGRATIONS).unwrap();
    }

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
}
