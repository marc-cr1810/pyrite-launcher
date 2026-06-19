//! Instance lifecycle: create (resolving mod loaders), select, delete.

use std::path::Path;

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

pub fn create(
    state: &AppState,
    weak: &Weak<MainWindow>,
    name: String,
    game_version: String,
    loader: String,
    loader_version: String,
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
        match Instance::create(&game_dir, &id, &name, &version_id) {
            Ok(inst) => {
                let mut cfg = state.config.lock().unwrap();
                cfg.active_instance = Some(inst.id);
                let _ = cfg.save();
            }
            Err(e) => {
                status(&weak, format!("Failed to create instance: {e}"));
                return;
            }
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
fn unique_id(game_dir: &Path, name: &str) -> String {
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

fn refresh(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::refresh_all(&ui, &state));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
