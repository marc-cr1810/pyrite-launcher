//! Mod update checking: hash an instance's installed mod jars, ask Modrinth
//! which have newer compatible versions, and apply updates by downloading the
//! new file and removing the old one. Results are cached in
//! `AppState::mod_updates` so the Mods-tab model can show per-mod flags.

use std::collections::HashMap;
use std::path::PathBuf;

use sha1::{Digest, Sha1};
use slint::ComponentHandle;
use slint::Weak;

use crate::app::controllers::details;
use crate::app::state::{AppState, ModUpdate, ModUpdateCache};
use crate::app::ui;
use crate::core::api::ApiClient;
use crate::core::instance::Instance;
use crate::{Logic, MainWindow};

fn instance_path(state: &AppState, id: &str) -> PathBuf {
    state.game_dir().join("instances").join(id)
}

/// SHA-1 (hex, lowercase) of a file's contents, or `None` if it can't be read.
fn file_sha1(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// Check every enabled mod in the instance for a newer Modrinth version.
pub fn check(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_checking(&weak, true);
        status(&weak, "Checking for mod updates…");

        let game_dir = state.game_dir();
        let inst = match Instance::load(&id, instance_path(&state, &id)) {
            Ok(i) => i,
            Err(e) => {
                set_checking(&weak, false);
                return status(&weak, format!("Failed to load instance: {e}"));
            }
        };
        let (game_version, loader) = inst.get_game_version_and_loader(&game_dir);
        let mods_dir = inst.path.join("mods");

        // Map each installed (enabled) jar's sha1 → its on-disk filename.
        let mut hash_to_file: HashMap<String, String> = HashMap::new();
        for m in inst.get_mods().unwrap_or_default() {
            if !m.enabled {
                continue;
            }
            if let Some(hash) = file_sha1(&mods_dir.join(&m.filename)) {
                hash_to_file.insert(hash, m.filename);
            }
        }

        if hash_to_file.is_empty() {
            set_checking(&weak, false);
            return status(&weak, "No mods to check.");
        }

        let hashes: Vec<String> = hash_to_file.keys().cloned().collect();
        let loaders: Vec<String> = loader.iter().map(|l| l.to_lowercase()).collect();
        let game_versions = vec![game_version];

        let api = ApiClient::new();
        let latest = match api.check_updates(&hashes, &loaders, &game_versions).await {
            Ok(m) => m,
            Err(e) => {
                set_checking(&weak, false);
                return status(&weak, format!("Update check failed: {e}"));
            }
        };

        // Keep only entries whose newest file differs from what's installed.
        let mut updates: HashMap<String, ModUpdate> = HashMap::new();
        for (input_hash, filename) in &hash_to_file {
            let Some(version) = latest.get(input_hash) else { continue };
            let Some(file) = version.files.iter().find(|f| f.primary).or_else(|| version.files.first()) else {
                continue;
            };
            let new_sha1 = file.hashes.sha1.clone();
            if new_sha1.as_deref() == Some(input_hash.as_str()) {
                continue; // already up to date
            }
            updates.insert(
                filename.clone(),
                ModUpdate {
                    version_number: version.version_number.clone(),
                    url: file.url.clone(),
                    sha1: new_sha1,
                    new_filename: file.filename.clone(),
                },
            );
        }

        let count = updates.len();
        *state.mod_updates.lock().unwrap() = ModUpdateCache { instance_id: id.clone(), updates };

        set_checking(&weak, false);
        status(
            &weak,
            if count == 0 {
                "All mods are up to date.".to_string()
            } else {
                format!("{count} mod update(s) available.")
            },
        );
        details::load(&state, &weak, id);
    });
}

/// Update a single mod to its cached newer version.
pub fn update_one(state: &AppState, weak: &Weak<MainWindow>, id: String, filename: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let update = {
            let cache = state.mod_updates.lock().unwrap();
            if cache.instance_id != id {
                None
            } else {
                cache.updates.get(&filename).cloned()
            }
        };
        let Some(update) = update else {
            return status(&weak, "No update found for that mod.");
        };

        match apply_update(&state, &id, &filename, &update).await {
            Ok(_) => {
                state.mod_updates.lock().unwrap().updates.remove(&filename);
                status(&weak, format!("Updated to {}.", update.version_number));
                details::load(&state, &weak, id);
            }
            Err(e) => status(&weak, format!("Update failed: {e}")),
        }
    });
}

/// Update every mod with a cached newer version.
pub fn update_all(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let pending: Vec<(String, ModUpdate)> = {
            let cache = state.mod_updates.lock().unwrap();
            if cache.instance_id != id {
                Vec::new()
            } else {
                cache.updates.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            }
        };
        if pending.is_empty() {
            return status(&weak, "No updates to apply.");
        }

        let total = pending.len();
        let mut done = 0;
        for (filename, update) in pending {
            status(&weak, format!("Updating {} ({}/{total})…", filename, done + 1));
            match apply_update(&state, &id, &filename, &update).await {
                Ok(_) => {
                    state.mod_updates.lock().unwrap().updates.remove(&filename);
                    done += 1;
                }
                Err(e) => status(&weak, format!("Failed to update {filename}: {e}")),
            }
        }
        status(&weak, format!("Updated {done} of {total} mod(s)."));
        details::load(&state, &weak, id);
    });
}

/// Download the new file and drop the old jar (when the filename changed).
async fn apply_update(
    state: &AppState,
    id: &str,
    old_filename: &str,
    update: &ModUpdate,
) -> Result<(), String> {
    let game_dir = state.game_dir();
    let mut inst = Instance::load(id, instance_path(state, id))?;
    inst.install_mod_from_url(
        &game_dir,
        &update.new_filename,
        &update.url,
        update.sha1.as_deref(),
        true,
    )
    .await?;
    // If the new release uses a different filename, remove the superseded jar
    // (and its instance.toml entry).
    if update.new_filename != old_filename {
        let _ = inst.remove_mod(old_filename, true);
    }
    Ok(())
}

fn set_checking(weak: &Weak<MainWindow>, checking: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui.global::<Logic>().set_mod_updates_checking(checking));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
