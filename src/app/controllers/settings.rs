//! Settings: game directory, Java, JVM args, and theme selection.
use slint::ComponentHandle;

use std::path::PathBuf;

use slint::Weak;

use crate::app::state::AppState;
use crate::app::ui;
use crate::MainWindow;

pub fn save(
    state: &AppState,
    weak: &Weak<MainWindow>,
    game_dir: String,
    java_path: String,
    jvm_args: String,
) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.game_dir = PathBuf::from(game_dir.trim());
        cfg.java_path = PathBuf::from(if java_path.trim().is_empty() {
            "java"
        } else {
            java_path.trim()
        });
        cfg.jvm_args = jvm_args
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let _ = cfg.save();
    }
    // game_dir may have changed instance discovery; refresh everything.
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui::refresh_all(&ui, &state);
        ui::set_status(&ui, "Settings saved.");
    });
}

/// Download an Adoptium JRE for the given major version into `<game_dir>/runtime`
/// and set it as the active Java path. Streams progress into the status bar.
pub fn install_java(state: &AppState, weak: &Weak<MainWindow>, major: i32) {
    if major <= 0 {
        return;
    }
    let major = major as u32;
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_java_installing(&weak, true);
        ui::progress_indeterminate_async(&weak, true);

        let game_dir = state.game_dir();
        let weak_log = weak.clone();
        let log_fn = move |msg: String| {
            let w = weak_log.clone();
            let _ = w.upgrade_in_event_loop(move |ui| ui::set_status(&ui, msg));
        };

        match crate::core::java::install_java_if_needed(&game_dir, major, log_fn).await {
            Ok(path) => {
                {
                    let mut cfg = state.config.lock().unwrap();
                    cfg.java_path = path.clone();
                    let _ = cfg.save();
                }
                let st = state.clone();
                let summary = format!("Java {major} ready: {}", path.display());
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui::refresh_settings(&ui, &st.config.lock().unwrap());
                    ui::set_status(&ui, summary);
                });
            }
            Err(e) => status(&weak, format!("Java install failed: {e}")),
        }

        set_java_installing(&weak, false);
        ui::progress_indeterminate_async(&weak, false);
    });
}

pub fn use_java_runtime(state: &AppState, weak: &Weak<MainWindow>, major: i32) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let game_dir = state.game_dir();
        let path = if major == 0 {
            std::path::PathBuf::from("java")
        } else {
            match crate::core::java::get_installed_java(&game_dir, major as u32) {
                Some(p) => p,
                None => {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui::set_status(&ui, format!("Java {major} is not installed locally."));
                    });
                    return;
                }
            }
        };

        {
            let mut cfg = state.config.lock().unwrap();
            cfg.java_path = path.clone();
            let _ = cfg.save();
        }

        let st = state.clone();
        let summary = if major == 0 {
            "Java path set to Auto-detect.".to_string()
        } else {
            format!("Java path set to Java {major}: {}", path.display())
        };

        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui::refresh_settings(&ui, &st.config.lock().unwrap());
            ui::set_status(&ui, summary);
        });
    });
}

fn set_java_installing(weak: &Weak<MainWindow>, installing: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.global::<crate::Logic>().set_java_installing(installing);
    });
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}

pub fn select_theme(state: &AppState, weak: &Weak<MainWindow>, name: String) {
    *state.active_theme.lock().unwrap() = name.clone();
    // Persist active theme name in a sidecar file next to config.
    persist_active_theme(&name);

    let themes = state.themes.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        themes.apply(&ui, &name);
        ui.global::<crate::Logic>().set_active_theme(name.into());
    });
}

fn theme_pref_path() -> Option<PathBuf> {
    crate::core::config::Config::config_path()
        .and_then(|p| p.parent().map(|d| d.join("active_theme.txt")))
}

pub fn persist_active_theme(name: &str) {
    if let Some(path) = theme_pref_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, name);
    }
}

pub fn load_active_theme() -> Option<String> {
    theme_pref_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
