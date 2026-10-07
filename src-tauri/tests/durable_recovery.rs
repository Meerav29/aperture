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

    // --- "Run 1": ingest, persist, then drop everything. ---
    {
        let db = Db::open(&db_path).unwrap();
        let mut observer = Observer::new(claude_root.clone(), root.join("codex-unused"));
        let mut store = Store::default();
        // Baseline poll against the empty file establishes the cursor's
        // initial_len, so the next line is seen as freshly appended (live)
        // rather than pre-existing history — matching how a real app starts
        // watching before any lines exist.
        observer.poll(&mut store);
        assert_eq!(store.snapshot().sessions.len(), 0);

        append_line(&file_path, &first_line);
        observer.poll(&mut store);
        let sessions = store.snapshot().sessions;
        assert_eq!(sessions.len(), 1, "run 1 should see the first line");
        assert!(
            sessions[0].live,
            "a line appended within the last minute should be observed live"
        );
        assert_eq!(sessions[0].observation, "recent");

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
    let load = db.load_summaries().unwrap();
    assert!(
        load.failed.is_empty(),
        "a real restart must read back every row this build wrote: {:?}",
        load.failed
    );
    store.restore_summaries(
        load.sessions,
        chrono::Utc::now(),
        chrono::Duration::days(aperture_lib::observer::state::LIVE_STORE_IDLE_DAYS),
    );

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
    // This is the assertion that actually distinguishes "cursor was resumed
    // from the saved offset" from "cursor plumbing is broken and the file
    // was re-scanned from zero". Both scenarios converge on the same final
    // byte offset and session count, so neither of those alone proves
    // anything. But `live`/`observation` diverge: `live` requires
    // `cursor.offset > cursor.initial_len` (passive.rs::apply), and
    // `initial_len` is only correctly anchored at the pre-restart file
    // length when `restore_cursors` actually restored the saved cursor. If
    // restore were a no-op, this poll would treat the file as newly
    // discovered, set `initial_len` to the current (post-append) length,
    // and nothing read here would ever count as "appended" — so `live`
    // would be `false` and `observation` would stay `"history_only"`.
    assert!(
        snap.sessions[0].live,
        "cursor must have resumed from the saved offset: the post-restart \
         poll only reads the newly-appended assistant line, which must be \
         seen as a live append, not a first-ever read of the whole file"
    );
    assert_eq!(
        snap.sessions[0].observation, "recent",
        "cursor must have resumed from the saved offset (see live assertion above)"
    );

    let cursors_after = observer.export_cursors();
    assert_eq!(cursors_after.len(), 1);
    let file_len = std::fs::metadata(&file_path).unwrap().len();
    assert_eq!(
        cursors_after[0].offset, file_len,
        "the cursor must have advanced past both lines, not re-read from zero"
    );

    // Drop the SQLite connection explicitly: on Windows the file stays
    // locked until the handle is released, which would make the directory
    // removal below fail.
    drop(db);
    std::fs::remove_dir_all(&root).unwrap();
}
