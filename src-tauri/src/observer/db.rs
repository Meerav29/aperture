//! SQLite-backed persistence for session summaries and per-file read
//! cursors. One connection, guarded by a mutex, touched only from blocking
//! tasks — see `lib.rs`'s reconcile loop for the single-writer contract.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// One stored summary row that could not be turned back into a `Session`.
/// `id` comes from the row's own `id` column, which is readable even when the
/// `data` blob is unusable, so a failure can name the session it lost.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryLoadFailure {
    pub id: String,
    pub error: String,
}

/// The result of `load_summaries`: the rows that deserialized, and the rows
/// that did not. Returning both is deliberate — the previous signature was
/// `Vec<Session>`, which made a row that failed to deserialize
/// indistinguishable from a row that was never written, so callers had no way
/// to tell a restored history from a partially lost one.
#[derive(Debug, Default)]
pub struct SummaryLoad {
    pub sessions: Vec<Session>,
    pub failed: Vec<SummaryLoadFailure>,
}

/// How many individual failures `load_summaries` names on stderr before it
/// stops and leaves the rest to the summary line. A database that is broadly
/// corrupt should not bury every other startup message.
const MAX_LOGGED_LOAD_FAILURES: usize = 10;

pub struct Db {
    state: Mutex<ConnState>,
    /// The on-disk path this `Db` was opened against, or `None` when backed
    /// by an in-memory connection (either `Db::in_memory()` directly, or
    /// `lib.rs`'s fallback after `Db::open` fails). Used by
    /// `commands::storage_health` to distinguish "durable but a write
    /// failed" from "not durable at all" — both look identical from inside
    /// a single save call, since in-memory saves always succeed.
    path: Option<PathBuf>,
    /// How many rows the most recent `load_summaries` could not deserialize.
    /// Kept on `Db` rather than only returned because summaries are loaded
    /// once, at startup, while the storage health entry is rebuilt on every
    /// reconcile cycle — without this the health row would go on reporting a
    /// clean `"ok"` over a history that is quietly missing sessions.
    unreadable_summaries: AtomicUsize,
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
            unreadable_summaries: AtomicUsize::new(0),
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
            unreadable_summaries: AtomicUsize::new(0),
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

    /// Load every stored summary, reporting the rows that could not be read
    /// instead of dropping them on the floor. A row whose `data` no longer
    /// matches the current `Session` shape — a field added by a later build,
    /// a genuine corruption — is counted, logged, and named in
    /// `SummaryLoad::failed`; the remaining valid rows still load.
    ///
    /// The unreadable rows are deliberately left in SQLite. This mirrors the
    /// migration-failure rule in `docs/specification.md` ("Storage and
    /// historical ingestion"): leave the original intact and surface the
    /// failure, rather than silently presenting a clean empty state.
    pub fn load_summaries(&self) -> rusqlite::Result<SummaryLoad> {
        let mut out = SummaryLoad::default();
        {
            let guard = self.state.lock().expect("db lock");
            let mut stmt = guard
                .conn
                .prepare("SELECT id, data FROM session_summaries")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for r in rows {
                let (id, data) = r?;
                match serde_json::from_str::<Session>(&data) {
                    Ok(s) => out.sessions.push(s),
                    Err(e) => out.failed.push(SummaryLoadFailure {
                        id,
                        error: e.to_string(),
                    }),
                }
            }
        }

        self.unreadable_summaries
            .store(out.failed.len(), Ordering::Relaxed);
        for f in out.failed.iter().take(MAX_LOGGED_LOAD_FAILURES) {
            eprintln!(
                "Aperture: stored session summary {} could not be read ({}); \
                 it is missing from history but was left in the database",
                f.id, f.error
            );
        }
        if !out.failed.is_empty() {
            eprintln!(
                "Aperture: {} of {} stored session summaries could not be read",
                out.failed.len(),
                out.failed.len() + out.sessions.len()
            );
        }
        Ok(out)
    }

    /// How many rows the most recent `load_summaries` could not deserialize,
    /// for the storage health entry. Zero before any load.
    pub fn unreadable_summaries(&self) -> usize {
        self.unreadable_summaries.load(Ordering::Relaxed)
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

    /// Delete specific file cursors by path. Returns the number removed.
    /// Callers are responsible for deciding which paths to delete — this
    /// method is deliberately filesystem-agnostic (`Db` never touches the
    /// filesystem directly); see `lib.rs`'s startup sequence, which deletes
    /// cursors whose underlying transcript file no longer exists on disk.
    /// This replaces a prior time-based `prune_cursors`, which was removed:
    /// pruning a cursor by elapsed time while its file is still present
    /// re-arms a full re-read of that file on the next poll (the cursor's
    /// absence makes `Observer::poll` treat the file as newly discovered),
    /// which silently undid `prune_summaries`'s deletion of the same
    /// session on every subsequent launch.
    pub fn delete_cursors(&self, paths: &[String]) -> rusqlite::Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }
        let guard = self.state.lock().expect("db lock");
        let mut removed = 0;
        for path in paths {
            removed += guard
                .conn
                .execute("DELETE FROM file_cursors WHERE path = ?1", params![path])?;
        }
        Ok(removed)
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

    /// Test-only writer: store a summary row's `data` verbatim, bypassing
    /// `Session` serialization, so tests can plant a row whose body does not
    /// match the current shape. There is no production path that writes an
    /// unparseable summary — the real one is a build whose `Session` differs
    /// from the build that wrote the row, which a test cannot reproduce.
    #[cfg(test)]
    pub(crate) fn insert_raw_summary(&self, id: &str, data: &str) {
        self.state
            .lock()
            .expect("db lock")
            .conn
            .execute(
                "INSERT INTO session_summaries (id, data, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data",
                params![id, data, Utc::now().to_rfc3339()],
            )
            .expect("insert raw summary");
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
        let mut loaded = db.load_summaries().unwrap().sessions;
        loaded.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "claude_code:a");
        assert_eq!(loaded[1].id, "codex:b");
    }

    /// The same `Session` JSON a real row holds, with `drop` keys removed —
    /// how a row written by a build with a different `Session` shape looks to
    /// this one.
    fn summary_json_without(id: &str, drop: &[&str]) -> String {
        let value = serde_json::to_value(sample_session(id)).unwrap();
        let mut obj = value.as_object().unwrap().clone();
        for key in drop {
            assert!(obj.remove(*key).is_some(), "{key} is not a Session field");
        }
        serde_json::Value::Object(obj).to_string()
    }

    #[test]
    fn load_summaries_reports_unreadable_rows_and_still_loads_the_rest() {
        // The bug this replaces: `load_summaries` skipped any row that failed
        // to deserialize with no log, no count, and no way for a caller to
        // tell the row apart from one that was never written.
        let db = Db::in_memory();
        db.save_summaries(&[sample_session("claude_code:good")])
            .unwrap();
        db.insert_raw_summary("claude_code:shape_changed", r#"{"id":"whatever"}"#);
        db.insert_raw_summary("claude_code:corrupt", "{not json at all");

        let load = db.load_summaries().unwrap();

        assert_eq!(load.sessions.len(), 1, "valid rows must still load");
        assert_eq!(load.sessions[0].id, "claude_code:good");

        let mut failed: Vec<&str> = load.failed.iter().map(|f| f.id.as_str()).collect();
        failed.sort();
        assert_eq!(failed, ["claude_code:corrupt", "claude_code:shape_changed"]);
        assert!(
            load.failed.iter().all(|f| !f.error.is_empty()),
            "each failure must carry why it failed"
        );
    }

    #[test]
    fn unreadable_rows_are_counted_for_health_and_left_in_the_database() {
        let db = Db::in_memory();
        db.save_summaries(&[sample_session("claude_code:good")])
            .unwrap();
        db.insert_raw_summary("claude_code:corrupt", "{");

        db.load_summaries().unwrap();

        // The count outlives the load itself: summaries are read once at
        // startup, but `commands::storage_health` is rebuilt every cycle.
        assert_eq!(db.unreadable_summaries(), 1);

        // Eviction from the returned Vec is not deletion — the row stays put
        // so the owner can recover it.
        let rows: i64 = db
            .state
            .lock()
            .unwrap()
            .conn
            .query_row("SELECT COUNT(*) FROM session_summaries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn a_later_clean_load_clears_the_unreadable_count() {
        let db = Db::in_memory();
        db.insert_raw_summary("claude_code:corrupt", "{");
        db.load_summaries().unwrap();
        assert_eq!(db.unreadable_summaries(), 1);

        db.insert_raw_summary(
            "claude_code:corrupt",
            &serde_json::to_string(&sample_session("claude_code:corrupt")).unwrap(),
        );
        db.load_summaries().unwrap();

        assert_eq!(
            db.unreadable_summaries(),
            0,
            "health must stop reporting a failure the database no longer has"
        );
    }

    #[test]
    fn a_row_written_before_an_optional_field_existed_still_loads() {
        // Characterization, not a new guarantee: serde's derive already
        // resolves a missing `Option` field to `None`, so this passes before
        // and after issue #18's change. It is here to pin that — the
        // deserialization decision in `docs/autopilot/decisions.md` rests on
        // additive `Option` fields being free, and a later `deny_unknown_
        // fields`, a hand-written Deserialize, or a field changed from
        // `Option<T>` to `T` would silently make every older row unreadable.
        let db = Db::in_memory();
        db.insert_raw_summary(
            "claude_code:a",
            &summary_json_without("claude_code:a", &["title", "git_branch"]),
        );

        let load = db.load_summaries().unwrap();

        assert!(load.failed.is_empty(), "{:?}", load.failed);
        assert_eq!(load.sessions.len(), 1);
        assert!(load.sessions[0].title.is_none());
        assert!(load.sessions[0].git_branch.is_none());
    }

    #[test]
    fn a_row_missing_a_required_field_is_reported_rather_than_defaulted() {
        // The strict half: defaulting `attention` would produce a session
        // claiming a state nothing observed, which is the failure mode this
        // issue exists to stop. It must surface as a failure instead.
        let db = Db::in_memory();
        db.insert_raw_summary(
            "claude_code:a",
            &summary_json_without("claude_code:a", &["attention"]),
        );

        let load = db.load_summaries().unwrap();

        assert!(load.sessions.is_empty(), "no fabricated session");
        assert_eq!(load.failed.len(), 1);
        assert_eq!(load.failed[0].id, "claude_code:a");
    }

    #[test]
    fn saving_the_same_summary_id_twice_upserts_not_duplicates() {
        let db = Db::in_memory();
        let mut s = sample_session("claude_code:a");
        db.save_summaries(&[s.clone()]).unwrap();
        s.status = crate::observer::model::SessionStatus::Working;
        db.save_summaries(&[s]).unwrap();
        let loaded = db.load_summaries().unwrap().sessions;
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
        let loaded = db.load_summaries().unwrap().sessions;
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
        assert_eq!(db.load_summaries().unwrap().sessions.len(), 1);
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
        let remaining = db.load_summaries().unwrap().sessions;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "claude_code:fresh");
    }

    #[test]
    fn delete_cursors_removes_only_the_named_paths() {
        let db = Db::in_memory();
        db.save_cursors(&[
            CursorRecord {
                path: "keep.jsonl".into(),
                provider: "claude_code".into(),
                ..Default::default()
            },
            CursorRecord {
                path: "remove.jsonl".into(),
                provider: "claude_code".into(),
                ..Default::default()
            },
        ])
        .unwrap();

        let removed = db.delete_cursors(&["remove.jsonl".to_string()]).unwrap();
        assert_eq!(removed, 1);

        let remaining = db.load_cursors().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, "keep.jsonl");
    }

    #[test]
    fn delete_cursors_with_empty_list_is_a_no_op() {
        let db = Db::in_memory();
        db.save_cursors(&[CursorRecord {
            path: "keep.jsonl".into(),
            provider: "claude_code".into(),
            ..Default::default()
        }])
        .unwrap();

        let removed = db.delete_cursors(&[]).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(db.load_cursors().unwrap().len(), 1);
    }
}
