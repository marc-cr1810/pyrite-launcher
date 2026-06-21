//! Instance lifecycle: create (resolving mod loaders), select, delete.

use std::path::Path;

use slint::ComponentHandle;
use slint::Weak;

use crate::app::state::AppState;
use crate::app::ui;
use crate::core::api::ApiClient;
use crate::core::instance::Instance;
use crate::MainWindow;

pub fn select(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.active_instance = Some(id);
        let _ = cfg.save();
    }
    refresh(state, weak);
}

pub fn delete(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    {
        let cfg = state.config.lock().unwrap();
        let path = cfg.game_dir.join("instances").join(&id);
        if let Ok(inst) = Instance::load(&id, path) {
            let _ = inst.delete();
        }
    }
    {
        let mut cfg = state.config.lock().unwrap();
        if cfg.active_instance.as_deref() == Some(id.as_str()) {
            cfg.active_instance = Instance::load_all(&cfg.game_dir).first().map(|i| i.id.clone());
        }
        let _ = cfg.save();
    }
    refresh(state, weak);
}

/// Edit a vanilla, non-version field on an existing instance: display name and
/// the per-instance launch overrides. Version/loader changes are intentionally
/// out of scope here (they require re-resolving the loader profile).
#[allow(clippy::too_many_arguments)]
pub fn edit(
    state: &AppState,
    weak: &Weak<MainWindow>,
    id: String,
    name: String,
    jvm_args: String,
    java_path: String,
    pre_launch: String,
    post_exit: String,
    icon_id: String,
) {
    let name = name.trim().to_string();
    if name.is_empty() {
        status(weak, "Instance name cannot be empty.");
        return;
    }

    let game_dir = state.game_dir();
    let path = game_dir.join("instances").join(&id);
    let mut inst = match Instance::load(&id, path) {
        Ok(inst) => inst,
        Err(e) => {
            status(weak, format!("Failed to load instance: {e}"));
            return;
        }
    };

    inst.config.name = name;
    inst.config.jvm_args = split_args(&jvm_args);
    inst.config.java_path = none_if_blank(java_path);
    inst.config.pre_launch = none_if_blank(pre_launch);
    inst.config.post_exit = none_if_blank(post_exit);
    if let Err(e) = apply_icon(state, &mut inst, &icon_id) {
        status(weak, format!("Failed to set icon: {e}"));
        return;
    }

    if let Err(e) = inst.save() {
        status(weak, format!("Failed to save instance: {e}"));
        return;
    }
    status(weak, "Instance updated.");
    refresh(state, weak);
}

/// Open a native file dialog to pick a custom icon PNG. On selection, remember
/// the path (copied into the instance on save) and preview it in the dialog.
pub fn pick_custom_icon(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let weak = weak.clone();
    // Run the blocking native dialog off the UI thread.
    state.rt.spawn(async move {
        let picked = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_title("Choose instance icon")
            .pick_file();
        if let Some(path) = picked {
            *state.pending_icon_path.lock().unwrap() = Some(path.clone());
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let logic = ui.global::<crate::Logic>();
                logic.set_pending_icon_id("custom".into());
                logic.set_pending_icon_image(
                    slint::Image::load_from_path(&path).unwrap_or_default(),
                );
            });
        }
    });
}

/// Clear any pending custom-icon file path (e.g. when re-opening a dialog), so
/// a save without a fresh pick doesn't reuse a stale selection.
pub fn reset_pending_icon(state: &AppState) {
    *state.pending_icon_path.lock().unwrap() = None;
}

/// Apply the chosen icon to an instance's config (and filesystem). `icon_id` is
/// "" (monogram), a built-in id, or "custom" (use the pending picked PNG, or
/// keep the existing icon.png if none was picked).
pub(crate) fn apply_icon(state: &AppState, inst: &mut Instance, icon_id: &str) -> Result<(), String> {
    let icon_png = inst.path.join("icon.png");
    let pending = state.pending_icon_path.lock().unwrap().take();
    match icon_id {
        "" => {
            let _ = std::fs::remove_file(&icon_png);
            inst.config.icon = None;
        }
        "custom" => {
            if let Some(src) = pending {
                std::fs::copy(&src, &icon_png)
                    .map_err(|e| format!("Failed to copy icon: {e}"))?;
            }
            // If no new file was picked, the existing icon.png is kept as-is.
            inst.config.icon = Some("custom".to_string());
        }
        builtin => {
            let _ = std::fs::remove_file(&icon_png);
            inst.config.icon = Some(builtin.to_string());
        }
    }
    Ok(())
}

/// Build the JVM args vector from a max-memory field (MB) plus extra args.
/// Returns `None` when both are empty (so the field is omitted from the toml).
pub(crate) fn build_jvm_args(memory_mb: &str, extra: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    if let Ok(mb) = memory_mb.trim().parse::<u64>()
        && mb > 0
    {
        args.push(format!("-Xmx{mb}M"));
    }
    args.extend(extra.split_whitespace().map(|a| a.to_string()));
    if args.is_empty() {
        None
    } else {
        Some(args)
    }
}

/// Split a whitespace-separated argument string into a vector, or `None` if it
/// has no arguments (so the field is omitted from instance.toml).
fn split_args(s: &str) -> Option<Vec<String>> {
    let args: Vec<String> = s.split_whitespace().map(|a| a.to_string()).collect();
    if args.is_empty() {
        None
    } else {
        Some(args)
    }
}

/// Trim a string and return `None` if it is empty.
fn none_if_blank(s: String) -> Option<String> {
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create(
    state: &AppState,
    weak: &Weak<MainWindow>,
    name: String,
    game_version: String,
    loader: String,
    loader_version: String,
    icon_id: String,
    memory_mb: String,
    jvm_args: String,
) {
    let name = name.trim().to_string();
    if name.is_empty() || game_version.is_empty() {
        return;
    }
    let state = state.clone();
    let weak = weak.clone();

    state.rt.clone().spawn(async move {
        let game_dir = state.game_dir();
        let api = ApiClient::new();

        status(&weak, "Resolving version…");
        let version_id = if loader.is_empty() {
            game_version.clone()
        } else {
            let loader_opt = if loader_version.trim().is_empty() {
                None
            } else {
                Some(loader_version.trim().to_string())
            };
            match resolve_loader(&api, &game_dir, &game_version, &loader, loader_opt).await {
                Ok(id) => id,
                Err(e) => {
                    status(&weak, format!("Failed: {e}"));
                    return;
                }
            }
        };

        let id = unique_id(&game_dir, &name);
        let mut inst = match Instance::create(&game_dir, &id, &name, &version_id) {
            Ok(inst) => inst,
            Err(e) => {
                status(&weak, format!("Failed to create instance: {e}"));
                return;
            }
        };

        inst.config.jvm_args = build_jvm_args(&memory_mb, &jvm_args);
        if let Err(e) = apply_icon(&state, &mut inst, &icon_id) {
            status(&weak, format!("Instance created, but icon failed: {e}"));
        }
        if let Err(e) = inst.save() {
            status(&weak, format!("Failed to save instance: {e}"));
            return;
        }

        {
            let mut cfg = state.config.lock().unwrap();
            cfg.active_instance = Some(inst.id.clone());
            let _ = cfg.save();
        }
        status(&weak, "Instance created.");
        refresh(&state, &weak);
    });
}

/// Resolve a mod loader to a concrete version id and write its profile JSON to
/// the versions directory. Mirrors minecli's `resolve_and_setup_loader`.
async fn resolve_loader(
    api: &ApiClient,
    game_dir: &Path,
    game_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> Result<String, String> {
    let loader = loader.to_lowercase();
    let (version_id, profile) = if loader == "fabric" {
        let loaders = api.fetch_fabric_loaders(game_version).await?;
        let ver = match loader_version {
            Some(v) => {
                if !loaders.iter().any(|l| l.loader.version == v) {
                    return Err(format!("Fabric loader '{v}' not found for {game_version}."));
                }
                v
            }
            None => loaders
                .iter()
                .find(|l| l.loader.stable)
                .or_else(|| loaders.first())
                .ok_or_else(|| format!("No Fabric loaders for {game_version}."))?
                .loader
                .version
                .clone(),
        };
        let id = format!("fabric-loader-{ver}-{game_version}");
        let profile = api.fetch_fabric_profile(game_version, &ver).await?;
        (id, profile)
    } else if loader == "forge" || loader == "neoforge" {
        let is_neo = loader == "neoforge";
        let index = if is_neo {
            api.fetch_neoforge_versions().await?
        } else {
            api.fetch_forge_versions().await?
        };
        let matching: Vec<_> = index
            .versions
            .iter()
            .filter(|v| {
                v.requires
                    .iter()
                    .any(|r| r.uid == "net.minecraft" && r.equals == game_version)
            })
            .collect();
        if matching.is_empty() {
            return Err(format!("No {loader} versions for Minecraft {game_version}."));
        }
        let ver = match loader_version {
            Some(v) => {
                if !matching.iter().any(|m| m.version == v) {
                    return Err(format!("{loader} '{v}' not supported for {game_version}."));
                }
                v
            }
            None => matching
                .iter()
                .find(|m| m.recommended)
                .or_else(|| matching.first())
                .unwrap()
                .version
                .clone(),
        };
        let id = if is_neo {
            format!("neoforge-{ver}")
        } else {
            format!("forge-{ver}")
        };
        let profile = if is_neo {
            api.fetch_neoforge_profile(&ver).await?
        } else {
            api.fetch_forge_profile(&ver).await?
        };
        (id, profile)
    } else {
        return Err(format!("Unsupported loader '{loader}'."));
    };

    let version_dir = game_dir.join("versions").join(&version_id);
    std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;
    let json_path = version_dir.join(format!("{version_id}.json"));
    let json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
    Ok(version_id)
}

/// Build a filesystem-safe, unique instance id from a display name.
pub(crate) fn unique_id(game_dir: &Path, name: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() { "instance".to_string() } else { base };

    let instances_dir = game_dir.join("instances");
    let mut candidate = base.clone();
    let mut n = 1;
    while instances_dir.join(&candidate).exists() {
        n += 1;
        candidate = format!("{base}-{n}");
    }
    candidate
}

pub fn duplicate(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    let game_dir = state.game_dir();
    let path = game_dir.join("instances").join(&id);
    let inst = match Instance::load(&id, path) {
        Ok(inst) => inst,
        Err(e) => {
            status(weak, format!("Failed to load instance: {e}"));
            return;
        }
    };

    let new_name = format!("{} (copy)", inst.config.name);
    let new_id = unique_id(&game_dir, &new_name);
    match inst.duplicate(&game_dir, &new_id, &new_name) {
        Ok(_) => {
            status(weak, format!("Duplicated as \"{}\".", new_name));
            refresh(state, weak);
        }
        Err(e) => status(weak, format!("Failed to duplicate: {e}")),
    }
}

pub fn sort(state: &AppState, weak: &Weak<MainWindow>, key: String) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.global::<crate::Logic>().set_instance_sort(key.as_str().into());
        ui::refresh_instances(&ui, &state.config.lock().unwrap());
    });
}

pub fn search(state: &AppState, weak: &Weak<MainWindow>, query: String) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.global::<crate::Logic>().set_instance_search(query.as_str().into());
        ui::refresh_instances(&ui, &state.config.lock().unwrap());
    });
}

fn refresh(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::refresh_all(&ui, &state));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
