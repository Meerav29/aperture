//! Tests the real helper executable in a private spool; no providers are started.
use aperture_lib::observer::{hook_bridge, state::Store};
use std::{
    io::Write,
    process::{Command, Stdio},
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
