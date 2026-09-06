//! Tauri commands: the only surface the frontend calls. Keep these thin;
//! logic lives in `observer/`.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::observer::{hooks_installer, model::Snapshot, state::Store, transcript};

pub struct Shared {
    pub store: Arc<Mutex<Store>>,
}

pub const SNAPSHOT_EVENT: &str = "sessions:snapshot";

pub async fn push_snapshot(app: &AppHandle, store: &Arc<Mutex<Store>>) {
    let snap = store.lock().await.snapshot();
    let _ = app.emit(SNAPSHOT_EVENT, snap);
}

#[tauri::command]
pub async fn get_snapshot(shared: State<'_, Shared>) -> Result<Snapshot, String> {
    Ok(shared.store.lock().await.snapshot())
}

#[tauri::command]
pub async fn install_hooks(app: AppHandle, shared: State<'_, Shared>) -> Result<(), String> {
    let port = shared.store.lock().await.listener_port;
    hooks_installer::install(port).map_err(|e| e.to_string())?;
    shared.store.lock().await.hooks_installed = true;
    push_snapshot(&app, &shared.store).await;
    Ok(())
}

#[tauri::command]
pub async fn uninstall_hooks(app: AppHandle, shared: State<'_, Shared>) -> Result<(), String> {
    hooks_installer::uninstall().map_err(|e| e.to_string())?;
    shared.store.lock().await.hooks_installed = false;
    push_snapshot(&app, &shared.store).await;
    Ok(())
}

/// Scan `~/.claude/projects` and merge into the store. Runs on a blocking
/// thread since it's file IO over possibly hundreds of transcripts.
#[tauri::command]
pub async fn rescan_transcripts(app: AppHandle, shared: State<'_, Shared>) -> Result<usize, String> {
    let summaries = tokio::task::spawn_blocking(transcript::scan_all)
        .await
        .map_err(|e| e.to_string())?;
    let n = summaries.len();
    {
        let mut store = shared.store.lock().await;
        for s in summaries {
            store.apply_transcript(s);
        }
    }
    push_snapshot(&app, &shared.store).await;
    Ok(n)
}

#[tauri::command]
pub async fn forget_session(app: AppHandle, shared: State<'_, Shared>, id: String) -> Result<(), String> {
    shared.store.lock().await.remove(&id);
    push_snapshot(&app, &shared.store).await;
    Ok(())
}

/// Best-effort "jump to it". Spike scope: macOS terminal focus via the PID.
/// Returns a message describing what happened so the UI can show it.
#[tauri::command]
pub async fn jump_to_session(shared: State<'_, Shared>, id: String) -> Result<String, String> {
    let (pid, transcript) = {
        let store = shared.store.lock().await;
        let snap = store.snapshot();
        let s = snap.sessions.iter().find(|s| s.id == id).ok_or("unknown session")?;
        (s.pid, s.transcript_path.clone())
    };

    #[cfg(target_os = "macos")]
    if let Some(pid) = pid {
        // Find the terminal app owning this PID by walking up until we hit
        // a process with a bundle, then activate it. Timebox this on day 3;
        // AppleScript per-terminal is a rabbit hole.
        let script = format!(
            r#"tell application "System Events"
                 set p to first process whose unix id is {pid}
                 repeat while (unix id of p) is not 1
                   try
                     if background only of p is false then
                       set frontmost of p to true
                       return name of p
                     end if
                   end try
                   set p to parent of p
                 end repeat
               end tell"#
        );
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(format!("Focused {}", String::from_utf8_lossy(&out.stdout).trim()));
        }
    }

    let _ = pid;
    match transcript {
        Some(t) => Ok(format!("No window found. Transcript: {t}")),
        None => Ok("No window or transcript known for this session".into()),
    }
}
