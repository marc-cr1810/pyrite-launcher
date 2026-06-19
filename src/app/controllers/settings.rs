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
