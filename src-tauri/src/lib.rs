mod commands;
pub mod observer;

use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use commands::{push_snapshot, Shared};
use observer::{hooks_installer, listener, state::Store};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let port = std::env::var("APERTURE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(listener::DEFAULT_PORT);

    let store = Arc::new(Mutex::new(Store {
        hooks_installed: hooks_installer::check_installed(),
        listener_port: port,
        ..Default::default()
    }));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Shared { store: store.clone() })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::install_hooks,
            commands::uninstall_hooks,
            commands::rescan_transcripts,
            commands::forget_session,
            commands::jump_to_session,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let store_for_listener = store.clone();
            let store_for_push = store.clone();

            // Every accepted hook event pushes a fresh snapshot to the window.
            let on_change: listener::OnChange = Arc::new(move || {
                let h = handle.clone();
                let s = store_for_push.clone();
                tauri::async_runtime::spawn(async move {
                    push_snapshot(&h, &s).await;
                });
            });

            tauri::async_runtime::spawn(async move {
                if let Err(e) = listener::serve(port, store_for_listener, on_change).await {
                    eprintln!("aperture: listener failed on port {port}: {e}");
                }
            });

            // Backfill once at startup so history appears immediately.
            let handle = app.handle().clone();
            let store = store.clone();
            tauri::async_runtime::spawn(async move {
                let summaries = tokio::task::spawn_blocking(observer::transcript::scan_all)
                    .await
                    .unwrap_or_default();
                {
                    let mut st = store.lock().await;
                    for s in summaries {
                        st.apply_transcript(s);
                    }
                }
                push_snapshot(&handle, &store).await;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aperture");
}
