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
        running_pid: Arc::new(Mutex::new(None)),
        ms_cancel: Arc::new(AtomicBool::new(false)),
        log_buf: Arc::new(Mutex::new(Vec::new())),
        log_dirty: Arc::new(AtomicBool::new(false)),
        avatar_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        version_cache: Arc::new(Mutex::new(Vec::new())),
        pending_icon_path: Arc::new(Mutex::new(None)),
        modrinth_results: Arc::new(Mutex::new(Vec::new())),
        modrinth_icon_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        modrinth_detail: Arc::new(Mutex::new(None)),
        modrinth_gallery_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        modrinth_search: Arc::new(Mutex::new(Default::default())),
        mod_updates: Arc::new(Mutex::new(Default::default())),
    };

    let window = MainWindow::new()?;
    state.themes.apply(&window, &active_theme);
    populate_icon_options(&window);
    ui::refresh_all(&window, &state);
    controllers::wire(&window, &state);
    // Scan the initially-active instance for mod conflicts.
    controllers::preflight::run(&state, &window.as_weak());

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

    // Poll the running game's resource usage once a second and publish it into
    // the Play view's live stats. Cheap no-op while nothing is running.
    let resource_timer = slint::Timer::default();
    {
        let weak = window.as_weak();
        let running_pid = state.running_pid.clone();
        let cores = std::thread::available_parallelism().map(|n| n.get() as f32).unwrap_or(1.0);
        let mut sys = sysinfo::System::new();
        resource_timer.start(slint::TimerMode::Repeated, Duration::from_millis(1000), move || {
            let Some(pid) = *running_pid.lock().unwrap() else { return };
            let Some(ui) = weak.upgrade() else { return };
            let spid = sysinfo::Pid::from_u32(pid);
            // Must refresh `All`: sysinfo only computes per-process CPU usage on a
            // full refresh (`Some(..)` updates memory but leaves cpu_usage at 0).
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let logic = ui.global::<Logic>();
            if let Some(proc) = sys.process(spid) {
                let mem_mb = proc.memory() as f32 / (1024.0 * 1024.0);
                let cpu = (proc.cpu_usage() / cores).clamp(0.0, 100.0);
                logic.set_game_mem_mb(mem_mb);
                logic.set_game_cpu_pct(cpu.round() as i32);
                logic.set_game_uptime(format_uptime(proc.run_time()).into());
            }
        });
    }

    window.run()
}

/// Format a process run-time (seconds) as "mm:ss", or "h:mm:ss" past an hour.
fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Resolve the theme to use at startup: the persisted choice if it still exists,
/// otherwise the built-in default.
fn settings_initial_theme(themes: &ThemeStore) -> String {
    let names = themes.names();
    controllers::settings::load_active_theme()
        .filter(|n| names.iter().any(|x| x == n))
        .unwrap_or_else(|| names.first().cloned().unwrap_or_default())
}
