//! Instance content management for the detail-pane tabs: worlds, resource
//! packs, shaderpacks, screenshots, and mods. Listing happens on the UI thread
//! (it builds Slint models, which aren't `Send`); mutations run off-thread and
//! then schedule a reload.

use slint::ComponentHandle;
use slint::Weak;

use crate::app::state::AppState;
use crate::app::{convert, ui};
use crate::core::assets;
use crate::core::instance::Instance;
use crate::{Logic, MainWindow};

/// Absolute path of an instance directory.
fn instance_path(state: &AppState, id: &str) -> std::path::PathBuf {
    state.game_dir().join("instances").join(id)
}

/// Read every content list for `id` and push it into the `Logic` globals. Runs
/// on the Slint event-loop thread (Slint models must be built there).
fn populate(ui: &MainWindow, state: &AppState, id: &str) {
    let logic = ui.global::<Logic>();
    let path = instance_path(state, id);
    let inst = match Instance::load(id, path) {
        Ok(inst) => inst,
        Err(_) => return,
    };
    let (_version, loader) = inst.get_game_version_and_loader(&state.game_dir());

    let worlds = assets::list_worlds(&inst.path).unwrap_or_default();
    let resourcepacks = assets::list_resourcepacks(&inst.path).unwrap_or_default();
    let shaderpacks = assets::list_shaderpacks(&inst.path).unwrap_or_default();
    let screenshots = assets::list_screenshots(&inst.path).unwrap_or_default();
    let mods = inst.get_mods().unwrap_or_default();

    logic.set_detail_id(id.into());
    logic.set_inst_worlds(convert::worlds_model(&inst.path, &worlds));
    logic.set_inst_resourcepacks(convert::assets_model(
        &inst.path.join("resourcepacks"),
        &resourcepacks,
    ));
    logic.set_inst_shaderpacks(convert::assets_model(
        &inst.path.join("shaderpacks"),
        &shaderpacks,
    ));
    logic.set_inst_screenshots(convert::screenshots_model(&inst.path, &screenshots));
    logic.set_inst_mods(convert::mods_model(&mods));
    logic.set_inst_supports_mods(loader.is_some());
    logic.set_inst_supports_shaders(assets::detect_shader_support(&inst.path));
}

/// (Re)load the content lists for an instance. Safe to call from any thread.
pub fn load(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| populate(&ui, &state, &id));
}

/// Open one of the instance's content subdirs in the system file manager.
pub fn open_dir(state: &AppState, id: String, subdir: String) {
    let path = instance_path(state, &id).join(&subdir);
    let _ = std::fs::create_dir_all(&path);
    let _ = open::that(&path);
}

/// Native multi-file picker that copies the chosen files into the instance's
/// `resourcepacks` / `shaderpacks` / `mods` subdir. `kind` is that subdir name.
pub fn add_files(state: &AppState, weak: &Weak<MainWindow>, id: String, kind: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let (title, ext): (&str, &str) = if kind == "mods" {
            ("Add mods", "jar")
        } else {
            ("Add packs", "zip")
        };
        let picked = rfd::FileDialog::new()
            .add_filter(title, &[ext])
            .set_title(title)
            .pick_files();
        let Some(files) = picked else { return };

        let dest = instance_path(&state, &id).join(&kind);
        if let Err(e) = std::fs::create_dir_all(&dest) {
            status(&weak, format!("Failed to create folder: {e}"));
            return;
        }
        let mut copied = 0;
        for src in &files {
            let Some(name) = src.file_name() else { continue };
            match std::fs::copy(src, dest.join(name)) {
                Ok(_) => copied += 1,
                Err(e) => {
                    status(&weak, format!("Failed to copy {}: {e}", name.to_string_lossy()));
                }
            }
        }
        if copied > 0 {
            status(&weak, format!("Added {copied} file(s)."));
            load(&state, &weak, id);
        }
    });
}

pub fn delete_world(state: &AppState, weak: &Weak<MainWindow>, id: String, folder: String) {
    mutate(state, weak, id, move |path| {
        assets::delete_world(path, &folder).map(|_| format!("Deleted world “{folder}”."))
    });
}

pub fn backup_world(state: &AppState, weak: &Weak<MainWindow>, id: String, folder: String) {
    mutate(state, weak, id, move |path| {
        assets::backup_world(path, &folder).map(|p| {
            format!(
                "Backed up “{folder}” to {}.",
                p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            )
        })
    });
}

pub fn toggle_resourcepack(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    mutate(state, weak, id, move |path| {
        assets::toggle_resourcepack(path, &filename).map(|_| "Resource pack updated.".to_string())
    });
}

pub fn delete_resourcepack(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    mutate(state, weak, id, move |path| {
        assets::delete_resourcepack(path, &filename).map(|_| "Resource pack deleted.".to_string())
    });
}

pub fn toggle_shaderpack(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    mutate(state, weak, id, move |path| {
        assets::toggle_shaderpack(path, &filename).map(|_| "Shader pack updated.".to_string())
    });
}

pub fn delete_shaderpack(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    mutate(state, weak, id, move |path| {
        assets::delete_shaderpack(path, &filename).map(|_| "Shader pack deleted.".to_string())
    });
}

pub fn delete_screenshot(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    mutate(state, weak, id, move |path| {
        assets::delete_screenshot(path, &filename).map(|_| "Screenshot deleted.".to_string())
    });
}

pub fn toggle_mod(
    state: &AppState,
    weak: &Weak<MainWindow>,
    id: String,
    filename: String,
    enable: bool,
) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let path = instance_path(&state, &id);
        let result = match Instance::load(&id, path) {
            Ok(inst) => {
                if enable {
                    inst.enable_mod(&filename)
                } else {
                    inst.disable_mod(&filename)
                }
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(_) => {
                status(&weak, "Mod updated.");
                load(&state, &weak, id);
            }
            Err(e) => status(&weak, format!("Failed: {e}")),
        }
    });
}

pub fn remove_mod(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let path = instance_path(&state, &id);
        let result = match Instance::load(&id, path) {
            Ok(mut inst) => inst.remove_mod(&filename, true),
            Err(e) => Err(e),
        };
        match result {
            Ok(_) => {
                status(&weak, "Mod removed.");
                load(&state, &weak, id);
            }
            Err(e) => status(&weak, format!("Failed: {e}")),
        }
    });
}

/// Run a filesystem mutation against the instance dir off-thread, report its
/// result in the status bar, and reload the content lists on success.
fn mutate<F>(state: &AppState, weak: &Weak<MainWindow>, id: String, op: F)
where
    F: FnOnce(&std::path::Path) -> Result<String, String> + Send + 'static,
{
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let path = instance_path(&state, &id);
        match op(&path) {
            Ok(msg) => {
                status(&weak, msg);
                load(&state, &weak, id);
            }
            Err(e) => status(&weak, format!("Failed: {e}")),
        }
    });
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
