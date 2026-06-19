//! Shared application state passed into every UI callback.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

use crate::app::theme::ThemeStore;
use crate::core::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub rt: Arc<Runtime>,
    pub themes: Arc<ThemeStore>,
    /// Active theme name, kept here so it survives config reloads.
    pub active_theme: Arc<Mutex<String>>,
    /// True while a download/launch is running, to prevent overlapping launches.
    pub busy: Arc<Mutex<bool>>,
    /// Set when the user cancels the Microsoft device-code login.
    pub ms_cancel: Arc<AtomicBool>,
    /// Live game-log buffer. Background readers append here; a UI-thread timer
    /// flushes it into the log model so output streams even while the launcher
    /// window is unfocused (the event loop would otherwise sleep).
    pub log_buf: Arc<Mutex<Vec<String>>>,
    /// Set when `log_buf` has new lines the timer hasn't flushed yet.
    pub log_dirty: Arc<AtomicBool>,
}

impl AppState {
    pub fn game_dir(&self) -> std::path::PathBuf {
        self.config.lock().unwrap().game_dir.clone()
    }
}
