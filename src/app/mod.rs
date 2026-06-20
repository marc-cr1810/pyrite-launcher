//! GUI glue layer: state, theming, model conversion, and controllers that
//! bridge the Slint UI to the frontend-agnostic `crate::core`.
use slint::ComponentHandle;

pub mod avatars;
pub mod controllers;
pub mod convert;
pub mod icons;
pub mod state;
pub mod theme;
pub mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app::state::AppState;
use crate::app::theme::ThemeStore;
use crate::core::config::Config;
use crate::{IconOption, Logic, MainWindow};

/// Push the built-in instance-icon set into the UI for the picker grid.
fn populate_icon_options(window: &MainWindow) {
    let opts: Vec<IconOption> = icons::BUILTIN
        .iter()
        .map(|(id, glyph)| IconOption {
            id: (*id).into(),
            glyph: (*glyph).into(),
        })
        .collect();
    window
        .global::<Logic>()
        .set_icon_options(slint::ModelRc::new(slint::VecModel::from(opts)));
}

/// Build the window, wire everything up, and run the event loop.
pub fn run() -> Result<(), slint::PlatformError> {
    let config = Config::load();

    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime"),
    );

    let themes = Arc::new(ThemeStore::load());
    let active_theme = settings_initial_theme(&themes);

    let state = AppState {
        config: Arc::new(Mutex::new(config)),
        rt,
        themes,
        active_theme: Arc::new(Mutex::new(active_theme.clone())),
        busy: Arc::new(Mutex::new(false)),
        ms_cancel: Arc::new(AtomicBool::new(false)),
        log_buf: Arc::new(Mutex::new(Vec::new())),
        log_dirty: Arc::new(AtomicBool::new(false)),
        avatar_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        version_cache: Arc::new(Mutex::new(Vec::new())),
        pending_icon_path: Arc::new(Mutex::new(None)),
    };

    let window = MainWindow::new()?;
    state.themes.apply(&window, &active_theme);
    populate_icon_options(&window);
    ui::refresh_all(&window, &state);
    controllers::wire(&window, &state);

    // Flush the live game-log buffer into the UI on a fixed cadence. An active
    // repeating timer also keeps the event loop waking while the launcher is
    // unfocused (e.g. during gameplay), so log lines stream in real time.
    let log_timer = slint::Timer::default();
    {
        let weak = window.as_weak();
        let buf = state.log_buf.clone();
        let dirty = state.log_dirty.clone();
        log_timer.start(slint::TimerMode::Repeated, Duration::from_millis(120), move || {
            if dirty.swap(false, Ordering::SeqCst)
                && let Some(ui) = weak.upgrade()
            {
                let snapshot = buf.lock().unwrap().clone();
                ui.global::<Logic>().set_log_lines(convert::loglines_model(&snapshot));
            }
        });
    }

    window.run()
}

/// Resolve the theme to use at startup: the persisted choice if it still exists,
/// otherwise the built-in default.
fn settings_initial_theme(themes: &ThemeStore) -> String {
    let names = themes.names();
    controllers::settings::load_active_theme()
        .filter(|n| names.iter().any(|x| x == n))
        .unwrap_or_else(|| names.first().cloned().unwrap_or_default())
}
