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
