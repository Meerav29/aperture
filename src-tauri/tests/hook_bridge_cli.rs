//! Tests the real helper executable in a private spool; no providers are started.
use aperture_lib::observer::{hook_bridge, state::Store};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn helper_is_silent_fail_open_and_config_is_json() {
    let root = std::env::temp_dir().join(format!("aperture-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let executable = env!("CARGO_BIN_EXE_aperture-hook");
    for provider in ["claude_code", "codex"] {
        let config = Command::new(executable)
            .args(["config", provider])
            .output()
            .unwrap();
        assert!(config.status.success());
        let value: serde_json::Value = serde_json::from_slice(&config.stdout).unwrap();
        assert!(value["hooks"].get("PermissionRequest").is_some());
        for payload in [
            r#"{"session_id":"cli-test","hook_event_name":"PermissionRequest","tool_name":"Bash"}"#,
            "not json",
        ] {
            let mut child = Command::new(executable)
                .args(["collect", provider])
                .env("APERTURE_HOOK_DIR", &root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(result.status.success());
            assert!(result.stdout.is_empty());
            assert!(result.stderr.is_empty());
        }
    }
    let mut store = Store::default();
    assert_eq!(hook_bridge::poll(&root, &mut store), 0);
    let snap = store.snapshot();
    assert_eq!(snap.sessions.len(), 2);
    assert!(snap.sessions.iter().all(|s| s.attention == "permission"));
    std::fs::remove_dir(root).unwrap();
}

/// Allowance for process creation and scheduling on a loaded CI runner. The
/// helper measures its budget from its own start, which an outside observer
/// cannot separate from the time `spawn` takes.
const SPAWN_SLACK: Duration = Duration::from_secs(2);

#[test]
fn collect_exits_silently_within_budget_when_stdin_never_closes() {
    let root = std::env::temp_dir().join(format!("aperture-cli-hang-{}", std::process::id()));
    let budget = hook_bridge::COLLECT_BUDGET;
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_aperture-hook"))
        .args(["collect", "codex"])
        .env("APERTURE_HOOK_DIR", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Half a payload, and the write end stays open: the helper is mid-read.
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(br#"{"session_id":"hang","hook_event_name":"#)
        .unwrap();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if started.elapsed() > budget + SPAWN_SLACK {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let elapsed = started.elapsed();
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    drop(stdin);
    let status = status.unwrap_or_else(|| {
        panic!("helper still running {elapsed:?} after spawn with stdin open (budget {budget:?})")
    });
    assert!(status.success(), "helper exited with {status:?}");
    // It waited on stdin rather than failing early on the partial payload.
    assert!(
        elapsed >= budget,
        "helper exited after {elapsed:?}, before the budget"
    );
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
    let mut store = Store::default();
    assert_eq!(hook_bridge::poll(&root, &mut store), 0);
    assert!(store.snapshot().sessions.is_empty());
    let _ = std::fs::remove_dir_all(root);
}
