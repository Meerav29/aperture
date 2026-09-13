//! Passive Claude Code/Codex observation and optional hook metadata enrichment.
//! No Tauri types in here; `commands.rs` is the desktop bridge.

pub mod db;
pub mod hook_payload;
pub mod hook_bridge;
pub mod model;
pub mod passive;
pub mod state;
pub mod transcript;
pub mod watch;
