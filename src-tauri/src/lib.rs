mod commands;
pub mod observer;

use commands::{push_snapshot, Shared};
use observer::{passive::Observer, state::Store};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store = Arc::new(Mutex::new(Store::default()));
    let observer = Arc::new(StdMutex::new(Observer::default()));
    tauri::Builder::default()
        .manage(Shared {
            store: store.clone(),
            observer: observer.clone(),
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
                loop {
                    let s = store.clone();
                    let o = observer.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        o.lock()
                            .expect("observer lock")
                            .poll(&mut s.blocking_lock());
                    })
                    .await
                    {
                        eprintln!("Observer failed: {e}");
                    }
                    push_snapshot(&handle, &store).await;
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running aperture");
}
