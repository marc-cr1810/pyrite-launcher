//! Wires every Slint `Logic` callback to its controller.
use slint::ComponentHandle;

pub mod accounts;
pub mod instances;
pub mod launch;
pub mod settings;
pub mod versions;

use crate::app::state::AppState;
use crate::{Logic, MainWindow};

pub fn wire(ui: &MainWindow, state: &AppState) {
    let logic = ui.global::<Logic>();

    // --- Accounts ---
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_add_offline(move |username| {
            accounts::add_offline(&st, &weak, username.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_select_account(move |uuid| accounts::select(&st, &weak, uuid.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_remove_account(move |uuid| accounts::remove(&st, &weak, uuid.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_start_microsoft(move || accounts::start_microsoft(&st, &weak));
    }
    {
        let st = state.clone();
        logic.on_cancel_microsoft(move || accounts::cancel_microsoft(&st));
    }
    logic.on_open_url(move |url| accounts::open_url(url.to_string()));
    logic.on_copy_code(move |code| accounts::copy_code(code.to_string()));

    // --- Instances ---
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_select_instance(move |id| instances::select(&st, &weak, id.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_delete_instance(move |id| instances::delete(&st, &weak, id.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_create_instance(move |name, version, loader, loader_version| {
            instances::create(
                &st,
                &weak,
                name.to_string(),
                version.to_string(),
                loader.to_string(),
                loader_version.to_string(),
            );
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_edit_instance(move |id, name, jvm_args, java_path, pre_launch, post_exit| {
            instances::edit(
                &st,
                &weak,
                id.to_string(),
                name.to_string(),
                jvm_args.to_string(),
                java_path.to_string(),
                pre_launch.to_string(),
                post_exit.to_string(),
            );
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_load_versions(move || versions::load_versions(&st, &weak));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_load_loader_versions(move |loader, game_version| {
            versions::load_loader_versions(&st, &weak, loader.to_string(), game_version.to_string());
        });
    }

    // --- Play ---
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_play(move || launch::play(&st, &weak));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_clear_logs(move || launch::clear_logs(&st, &weak));
    }
    {
        let st = state.clone();
        logic.on_copy_logs(move || launch::copy_logs(&st));
    }

    // --- Settings ---
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_save_settings(move |game_dir, java_path, jvm_args| {
            settings::save(
                &st,
                &weak,
                game_dir.to_string(),
                java_path.to_string(),
                jvm_args.to_string(),
            );
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_select_theme(move |name| settings::select_theme(&st, &weak, name.to_string()));
    }
}
