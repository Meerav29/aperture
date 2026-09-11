//! Debounced filesystem watching for provider transcript roots. This emits
//! a plain wake signal, not paths — `Observer::poll()` already re-discovers
//! whatever changed on every call, so callers only need to know "something
//! changed, poll again."

use notify::RecommendedWatcher;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use std::path::PathBuf;
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
