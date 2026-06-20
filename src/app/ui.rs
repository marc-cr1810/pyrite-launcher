//! Helpers that push core state into the UI. All run on the Slint event-loop
//! thread (they take `&MainWindow`).
use slint::ComponentHandle;

use crate::app::state::AppState;
use crate::app::{avatars, convert};
use crate::core::config::Config;
use crate::core::instance::Instance;
use crate::{Logic, MainWindow};

pub fn refresh_accounts(ui: &MainWindow, config: &Config, state: &AppState) {
    ui.global::<Logic>()
        .set_accounts(convert::accounts_model(config, state));
    // Kick off avatar fetches for any accounts we haven't loaded heads for yet.
    let uuids: Vec<String> = config.accounts.iter().map(|a| a.uuid.clone()).collect();
    avatars::ensure(state, &ui.as_weak(), uuids);
}

pub fn refresh_instances(ui: &MainWindow, config: &Config) {
    let sort_key = ui.global::<Logic>().get_instance_sort();
    let search_query = ui.global::<Logic>().get_instance_search();
    ui.global::<Logic>().set_instances(convert::instances_model(config, sort_key.as_str(), search_query.as_str()));
}

pub fn refresh_summary(ui: &MainWindow, config: &Config, state: &AppState) {
    let logic = ui.global::<Logic>();

    let active_account = config.get_active_account();
    let account_name = active_account
        .map(|a| a.username.clone())
        .unwrap_or_default();
    let account_uuid = active_account.map(|a| a.uuid.as_str()).unwrap_or("");
    logic.set_active_account_name(account_name.clone().into());
    logic.set_active_account_initial(convert::initial(&account_name));
    logic.set_active_account_color(convert::avatar_color(account_uuid));
    logic.set_active_account_avatar(avatars::image_for(state, account_uuid));

    let mut inst_name = String::new();
    let mut inst_version = String::new();
    let mut inst_loader = String::new();
    let mut inst_glyph = slint::SharedString::new();
    let mut inst_image = slint::Image::default();
    if let Some(id) = config.active_instance.as_deref() {
        let path = config.game_dir.join("instances").join(id);
        if let Ok(inst) = Instance::load(id, path) {
            let (game_version, loader) = inst.get_game_version_and_loader(&config.game_dir);
            inst_name = inst.config.name.clone();
            inst_version = game_version;
            inst_loader = match loader.as_deref() {
                Some("fabric") => "Fabric".into(),
                Some("forge") => "Forge".into(),
                Some("neoforge") => "NeoForge".into(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            inst_glyph = convert::instance_icon_glyph(&inst.config.icon);
            inst_image = convert::instance_icon_image(&inst.path, &inst.config.icon);
        }
    }
    logic.set_active_instance_name(inst_name.clone().into());
    logic.set_active_instance_initial(convert::initial(&inst_name));
    logic.set_active_instance_color(
        convert::avatar_color(config.active_instance.as_deref().unwrap_or("")),
    );
    logic.set_active_instance_version(inst_version.into());
    logic.set_active_instance_loader(inst_loader.into());
    logic.set_active_instance_glyph(inst_glyph);
    logic.set_active_instance_image(inst_image);

    logic.set_can_play(!account_name.is_empty() && !inst_name.is_empty());
}

pub fn refresh_settings(ui: &MainWindow, config: &Config) {
    let logic = ui.global::<Logic>();
    logic.set_game_dir(config.game_dir.to_string_lossy().to_string().into());
    logic.set_java_path(config.java_path.to_string_lossy().to_string().into());
    logic.set_jvm_args(config.jvm_args.join(" ").into());
}

pub fn refresh_themes(ui: &MainWindow, state: &AppState) {
    let logic = ui.global::<Logic>();
    logic.set_themes(convert::string_model(state.themes.names()));
    logic.set_active_theme(state.active_theme.lock().unwrap().clone().into());
}

pub fn refresh_all(ui: &MainWindow, state: &AppState) {
    let config = state.config.lock().unwrap();
    refresh_accounts(ui, &config, state);
    refresh_instances(ui, &config);
    refresh_summary(ui, &config, state);
    refresh_settings(ui, &config);
    drop(config);
    refresh_themes(ui, state);
}

pub fn set_status(ui: &MainWindow, text: impl Into<slint::SharedString>) {
    ui.global::<Logic>().set_status_text(text.into());
}

pub fn set_busy(ui: &MainWindow, busy: bool) {
    ui.global::<Logic>().set_is_busy(busy);
}

#[allow(clippy::too_many_arguments)]
pub fn set_ms_dialog(
    ui: &MainWindow,
    open: bool,
    active: bool,
    code: impl Into<slint::SharedString>,
    uri: impl Into<slint::SharedString>,
    message: impl Into<slint::SharedString>,
) {
    let logic = ui.global::<Logic>();
    logic.set_ms_dialog_open(open);
    logic.set_ms_active(active);
    logic.set_ms_user_code(code.into());
    logic.set_ms_verification_uri(uri.into());
    logic.set_ms_message(message.into());
}

pub fn set_progress(ui: &MainWindow, active: bool, value: f32) {
    let logic = ui.global::<Logic>();
    logic.set_progress_active(active);
    logic.set_progress_indeterminate(false);
    logic.set_progress_value(value);
}

/// Update the determinate progress bar from a background task via the event loop.
pub fn progress_async(weak: &slint::Weak<MainWindow>, active: bool, value: f32) {
    let _ = weak.upgrade_in_event_loop(move |ui| set_progress(&ui, active, value));
}

/// Show an indeterminate (animated sweep) progress bar for phases with no
/// measurable progress, such as resolving manifests or launching.
pub fn progress_indeterminate_async(weak: &slint::Weak<MainWindow>, active: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let logic = ui.global::<Logic>();
        logic.set_progress_active(active);
        logic.set_progress_indeterminate(active);
    });
}
