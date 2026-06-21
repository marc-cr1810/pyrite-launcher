//! Shared application state passed into every UI callback.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

use crate::app::avatars::AvatarEntry;
use crate::app::convert::ModrinthDetailData;
use crate::app::theme::ThemeStore;
use crate::core::api::ModrinthSearchHit;
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
    /// Decoded player-head avatars, keyed by account UUID. Populated lazily by
    /// background fetches from Crafatar; the UI thread builds `slint::Image`s
    /// from the cached pixels. See `app::avatars`.
    pub avatar_cache: Arc<Mutex<HashMap<String, AvatarEntry>>>,
    /// Cached Mojang manifest as (version id, type) pairs, fetched once when the
    /// New Instance dialog opens so the type filter can re-derive the list
    /// without hitting the network again. Type is "release"/"snapshot"/
    /// "old_beta"/"old_alpha".
    pub version_cache: Arc<Mutex<Vec<(String, String)>>>,
    /// Path of a custom icon the user just picked in the New Instance / edit
    /// dialog, pending copy into the instance folder on save. Cleared after use.
    pub pending_icon_path: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// The current Modrinth search results, kept so the results model can be
    /// rebuilt on the UI thread as icon thumbnails arrive. See `app::controllers::modrinth`.
    pub modrinth_results: Arc<Mutex<Vec<ModrinthSearchHit>>>,
    /// Decoded Modrinth project icons, keyed by project id. Populated lazily by
    /// background fetches, mirroring `avatar_cache`.
    pub modrinth_icon_cache: Arc<Mutex<HashMap<String, AvatarEntry>>>,
    /// The currently-open Modrinth project detail (project + versions + author),
    /// kept so the detail model can be rebuilt on the UI thread as gallery images
    /// arrive. `None` when no detail is open.
    pub modrinth_detail: Arc<Mutex<Option<ModrinthDetailData>>>,
    /// Decoded detail icon + gallery images, keyed by image url.
    pub modrinth_gallery_cache: Arc<Mutex<HashMap<String, AvatarEntry>>>,
    /// The active Modrinth search (query/kind/scope + total hit count), so
    /// "load more" can fetch the next page and append to `modrinth_results`.
    pub modrinth_search: Arc<Mutex<ModrinthSearchCtx>>,
}

/// Parameters of the in-progress Modrinth search, for pagination.
#[derive(Default, Clone)]
pub struct ModrinthSearchCtx {
    pub query: String,
    pub kind: String,
    pub instance_id: String,
    pub total: usize,
}

impl AppState {
    pub fn game_dir(&self) -> std::path::PathBuf {
        self.config.lock().unwrap().game_dir.clone()
    }
}
