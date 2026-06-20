//! Wires every Slint `Logic` callback to its controller.
use slint::ComponentHandle;

pub mod accounts;
pub mod details;
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
        logic.on_create_instance(
            move |name, version, loader, loader_version, icon_id, memory, jvm_args| {
                instances::create(
                    &st,
                    &weak,
                    name.to_string(),
                    version.to_string(),
                    loader.to_string(),
                    loader_version.to_string(),
                    icon_id.to_string(),
                    memory.to_string(),
                    jvm_args.to_string(),
                );
            },
        );
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_edit_instance(
            move |id, name, jvm_args, java_path, pre_launch, post_exit, icon_id| {
                instances::edit(
                    &st,
                    &weak,
                    id.to_string(),
                    name.to_string(),
                    jvm_args.to_string(),
                    java_path.to_string(),
                    pre_launch.to_string(),
                    post_exit.to_string(),
                    icon_id.to_string(),
                );
            },
        );
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_duplicate_instance(move |id| instances::duplicate(&st, &weak, id.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_sort_instances(move |key| instances::sort(&st, &weak, key.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_search_instances(move |query| instances::search(&st, &weak, query.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_pick_custom_icon(move || instances::pick_custom_icon(&st, &weak));
    }
    {
        let st = state.clone();
        logic.on_reset_pending_icon_file(move || instances::reset_pending_icon(&st));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_load_versions(move || versions::load_versions(&st, &weak));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_filter_versions(move |ty| versions::filter_versions(&st, &weak, ty.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_load_loader_versions(move |loader, game_version| {
            versions::load_loader_versions(&st, &weak, loader.to_string(), game_version.to_string());
        });
    }

    // --- Instance content management (detail-pane tabs) ---
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_load_instance_details(move |id| details::load(&st, &weak, id.to_string()));
    }
    {
        let st = state.clone();
        logic.on_open_instance_dir(move |id, subdir| {
            details::open_dir(&st, id.to_string(), subdir.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_add_files(move |id, kind| {
            details::add_files(&st, &weak, id.to_string(), kind.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_delete_world(move |id, folder| {
            details::delete_world(&st, &weak, id.to_string(), folder.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_backup_world(move |id, folder| {
            details::backup_world(&st, &weak, id.to_string(), folder.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_toggle_resourcepack(move |id, filename| {
            details::toggle_resourcepack(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_delete_resourcepack(move |id, filename| {
            details::delete_resourcepack(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_toggle_shaderpack(move |id, filename| {
            details::toggle_shaderpack(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_delete_shaderpack(move |id, filename| {
            details::delete_shaderpack(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_toggle_mod(move |id, filename, enable| {
            details::toggle_mod(&st, &weak, id.to_string(), filename.to_string(), enable);
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_remove_mod(move |id, filename| {
            details::remove_mod(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_delete_screenshot(move |id, filename| {
            details::delete_screenshot(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_create_backup(move |id| details::create_backup(&st, &weak, id.to_string()));
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_restore_backup(move |id, filename| {
            details::restore_backup(&st, &weak, id.to_string(), filename.to_string());
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_delete_backup(move |id, filename| {
            details::delete_backup(&st, &weak, id.to_string(), filename.to_string());
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
    {
        let st = state.clone();
        let weak = ui.as_weak();
        logic.on_install_java(move |major| settings::install_java(&st, &weak, major));
    }
}
