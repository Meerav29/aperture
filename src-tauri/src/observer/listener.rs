//! Loopback HTTP listener for hook events. Binds 127.0.0.1 only.
//!
//! One route: `POST /hook` with the JSON payload. Responds 204 immediately;
//! the state update and UI push happen after the response so the hook script
//! (and therefore Claude Code) is never waiting on us.
//!
//! `GET /health` exists so you can check the app is up from a terminal.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use tokio::sync::Mutex;

use super::hook_payload::HookPayload;
use super::state::Store;

pub const DEFAULT_PORT: u16 = 47831;

/// Called after every accepted event so the host (Tauri) can push a snapshot.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<Store>>,
    on_change: OnChange,
}

pub async fn serve(port: u16, store: Arc<Mutex<Store>>, on_change: OnChange) -> std::io::Result<()> {
    let state = AppState { store, on_change };
    let app = Router::new()
        .route("/hook", post(receive_hook))
        .route("/health", get(|| async { "ok" }))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, app).await
}

async fn receive_hook(
    State(st): State<AppState>,
    Json(payload): Json<HookPayload>,
) -> StatusCode {
    // Don't hold the lock across the notify callback; the callback may
    // itself want a snapshot.
    {
        let mut store = st.store.lock().await;
        store.apply_hook(payload);
    }
    (st.on_change)();
    StatusCode::NO_CONTENT
}
