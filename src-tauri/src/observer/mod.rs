//! Passive Claude Code/Codex observation and optional hook metadata enrichment.
//! No Tauri types in here; `commands.rs` is the desktop bridge. Legacy HTTP and
//! installer modules are retained but not started by the desktop.

pub mod db;
pub mod hook_payload;
pub mod hook_bridge;
pub mod hooks_installer;
pub mod listener;
pub mod model;
pub mod passive;
pub mod state;
pub mod transcript;
