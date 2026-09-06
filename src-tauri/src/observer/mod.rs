//! The observer: everything needed to see Claude Code sessions we didn't
//! start. No Tauri types in here; `commands.rs` is the only bridge.

pub mod hook_payload;
pub mod hooks_installer;
pub mod listener;
pub mod model;
pub mod state;
pub mod transcript;
