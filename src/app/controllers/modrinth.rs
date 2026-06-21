//! Modrinth integration: browsing modpacks/mods/packs/shaders, installing them
//! into instances, creating instances from modpacks, and `.mrpack` import/export.
//!
//! Search results are cached in `AppState::modrinth_results` so the Slint model
//! can be rebuilt on the UI thread as icon thumbnails arrive asynchronously
//! (same lazy-icon pattern as `app::avatars`). All network/IO runs off the UI
//! thread via `rt.spawn`.

use slint::ComponentHandle;
use slint::Weak;
use tokio::sync::mpsc;

use crate::app::avatars::AvatarEntry;
use crate::app::controllers::{details, instances};
use crate::app::state::AppState;
use crate::app::{convert, ui};
use crate::core::api::{ApiClient, ModrinthSearchHit};
use crate::core::downloader::ProgressUpdate;
use crate::core::instance::Instance;
use crate::{Logic, MainWindow};

/// Map a UI "kind" to a Modrinth `project_type` facet value.
fn project_type_for(kind: &str) -> &'static str {
    match kind {
        "modpack" => "modpack",
        "resourcepack" => "resourcepack",
        "shader" => "shader",
        _ => "mod",
    }
}

/// Search Modrinth. For mod/pack/shader kinds the query is scoped to the
/// instance's game version (and loader, for mods); modpacks pass `instance_id`
/// empty and search unfiltered.
pub fn search(
    state: &AppState,
    weak: &Weak<MainWindow>,
    query: String,
    kind: String,
    instance_id: String,
) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_loading(&weak, true);
        set_status(&weak, "");

        let game_dir = state.game_dir();
        let (game_version, loader) = if instance_id.is_empty() {
            (None, None)
        } else {
            let path = game_dir.join("instances").join(&instance_id);
            match Instance::load(&instance_id, path) {
                Ok(inst) => {
                    let (gv, ld) = inst.get_game_version_and_loader(&game_dir);
                    (Some(gv), ld)
                }
                Err(_) => (None, None),
            }
        };

        let api = ApiClient::new();
        let project_type = project_type_for(&kind);
        let result = api
            .search_projects(&query, game_version.as_deref(), loader.as_deref(), project_type)
            .await;

        match result {
            Ok(hits) => {
                let empty = hits.is_empty();
                *state.modrinth_results.lock().unwrap() = hits.clone();
                push_results(&state, &weak);
                if empty {
                    set_status(&weak, "No results found.");
                }
                ensure_icons(&state, &weak, hits);
            }
            Err(e) => {
                state.modrinth_results.lock().unwrap().clear();
                push_results(&state, &weak);
                set_status(&weak, format!("Search failed: {e}"));
            }
        }
        set_loading(&weak, false);
    });
}

/// Install a browsed mod / resource pack / shader into an existing instance,
/// auto-picking the newest compatible version.
pub fn install(
    state: &AppState,
    weak: &Weak<MainWindow>,
    instance_id: String,
    project_id: String,
    kind: String,
) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let game_dir = state.game_dir();
        let path = game_dir.join("instances").join(&instance_id);
        let mut inst = match Instance::load(&instance_id, path) {
            Ok(i) => i,
            Err(e) => {
                set_status(&weak, format!("Failed to load instance: {e}"));
                return;
            }
        };
        let (game_version, loader) = inst.get_game_version_and_loader(&game_dir);

        set_status(&weak, "Resolving version…");
        let api = ApiClient::new();
        let versions = match api.fetch_modpack_versions(&project_id).await {
            Ok(v) => v,
            Err(e) => {
                set_status(&weak, format!("Failed to fetch versions: {e}"));
                return;
            }
        };

        // Newest version compatible with this instance's MC version (and loader,
        // for mods). Resource packs / shaders only need a matching game version.
        let chosen = versions.into_iter().find(|v| {
            let mc_ok = v.game_versions.contains(&game_version);
            let loader_ok = kind != "mod"
                || loader
                    .as_ref()
                    .map_or(true, |l| v.loaders.iter().any(|x| x.eq_ignore_ascii_case(l)));
            mc_ok && loader_ok
        });
        let version = match chosen {
            Some(v) => v,
            None => {
                set_status(&weak, format!("No compatible version for Minecraft {game_version}."));
                return;
            }
        };
        let file = match version.files.iter().find(|f| f.primary).or_else(|| version.files.first()) {
            Some(f) => f.clone(),
            None => {
                set_status(&weak, "That version has no downloadable file.");
                return;
            }
        };

        set_status(&weak, format!("Installing {}…", file.filename));
        let sha1 = file.hashes.sha1.as_deref();
        let result = match kind.as_str() {
            "mod" => inst.install_mod_from_url(&game_dir, &file.filename, &file.url, sha1, true).await,
            "resourcepack" => {
                inst.install_asset_from_url(&game_dir, &file.filename, &file.url, sha1, "resourcepack", None)
                    .await
            }
            "shader" => {
                inst.install_asset_from_url(&game_dir, &file.filename, &file.url, sha1, "shaderpack", None)
                    .await
            }
            other => Err(format!("Unsupported content type '{other}'.")),
        };

        match result {
            Ok(_) => {
                set_status(&weak, format!("Installed {}.", file.filename));
                details::load(&state, &weak, instance_id);
            }
            Err(e) => set_status(&weak, format!("Install failed: {e}")),
        }
    });
}

/// Create a new instance from a Modrinth modpack: download its latest `.mrpack`,
/// import it, then apply the chosen name / icon / launch options.
pub fn create_instance(
    state: &AppState,
    weak: &Weak<MainWindow>,
    project_id: String,
    name: String,
    icon_id: String,
    memory: String,
    jvm_args: String,
) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_busy(&weak, true);
        set_status(&weak, "Resolving modpack…");
        let game_dir = state.game_dir();
        let api = ApiClient::new();

        // The pack's own icon, looked up from the cached search hit.
        let icon_url = state
            .modrinth_results
            .lock()
            .unwrap()
            .iter()
            .find(|h| h.project_id == project_id)
            .and_then(|h| h.icon_url.clone());

        let versions = match api.fetch_modpack_versions(&project_id).await {
            Ok(v) => v,
            Err(e) => return fail(&weak, format!("Failed: {e}")),
        };
        let version = match versions.into_iter().next() {
            Some(v) => v,
            None => return fail(&weak, "This modpack has no published versions."),
        };
        let file = match version.files.iter().find(|f| f.primary).or_else(|| version.files.first()) {
            Some(f) => f.clone(),
            None => return fail(&weak, "This modpack version has no downloadable file."),
        };

        // Download the .mrpack into the shared cache.
        let cache_dir = game_dir.join("cache").join("modpacks");
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            return fail(&weak, format!("Failed: {e}"));
        }
        let pack_path = cache_dir.join(&file.filename);
        set_status(&weak, format!("Downloading {}…", file.filename));
        match download_bytes(&file.url).await {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&pack_path, &bytes) {
                    return fail(&weak, format!("Failed to save modpack: {e}"));
                }
            }
            Err(e) => return fail(&weak, format!("Download failed: {e}")),
        }

        // Import as a new instance, streaming progress to the UI.
        let id = instances::unique_id(&game_dir, &name);
        let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
        let import = {
            let gd = game_dir.clone();
            let pp = pack_path.clone();
            let iid = id.clone();
            tokio::spawn(async move { Instance::import_mrpack(&gd, &pp, &iid, &tx).await })
        };
        forward_progress(&weak, &mut rx).await;
        let mut inst = match import.await {
            Ok(Ok(inst)) => inst,
            Ok(Err(e)) => return fail(&weak, format!("Import failed: {e}")),
            Err(e) => return fail(&weak, format!("Import failed: {e}")),
        };

        // Apply the chosen name, launch overrides, and icon.
        inst.config.name = name.clone();
        inst.config.jvm_args = instances::build_jvm_args(&memory, &jvm_args);
        if icon_id == "modpack" {
            if let Some(url) = icon_url {
                if let Ok(bytes) = download_bytes(&url).await {
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        let _ = img.save_with_format(inst.path.join("icon.png"), image::ImageFormat::Png);
                        inst.config.icon = Some("custom".to_string());
                    }
                }
            }
        } else {
            let _ = instances::apply_icon(&state, &mut inst, &icon_id);
        }
        if let Err(e) = inst.save() {
            return fail(&weak, format!("Created, but failed to save settings: {e}"));
        }

        {
            let mut cfg = state.config.lock().unwrap();
            cfg.active_instance = Some(inst.id.clone());
            let _ = cfg.save();
        }
        set_status(&weak, format!("Created instance “{name}”."));
        finish(&weak);
        refresh_all(&state, &weak);
    });
}

/// Import a local `.mrpack` file as a new instance via a native file picker.
pub fn import_file(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let picked = rfd::FileDialog::new()
            .add_filter("Modrinth modpack", &["mrpack"])
            .set_title("Import .mrpack")
            .pick_file();
        let Some(path) = picked else { return };

        set_busy(&weak, true);
        let game_dir = state.game_dir();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("modpack");
        let id = instances::unique_id(&game_dir, stem);

        let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
        let import = {
            let gd = game_dir.clone();
            let pp = path.clone();
            let iid = id.clone();
            tokio::spawn(async move { Instance::import_mrpack(&gd, &pp, &iid, &tx).await })
        };
        forward_progress(&weak, &mut rx).await;
        match import.await {
            Ok(Ok(inst)) => {
                {
                    let mut cfg = state.config.lock().unwrap();
                    cfg.active_instance = Some(inst.id.clone());
                    let _ = cfg.save();
                }
                set_status(&weak, format!("Imported “{}”.", inst.config.name));
            }
            Ok(Err(e)) => set_status(&weak, format!("Import failed: {e}")),
            Err(e) => set_status(&weak, format!("Import failed: {e}")),
        }
        finish(&weak);
        refresh_all(&state, &weak);
    });
}

/// Export an existing instance to a `.mrpack` via a native save dialog.
pub fn export_file(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let game_dir = state.game_dir();
        let path = game_dir.join("instances").join(&id);
        let inst = match Instance::load(&id, path) {
            Ok(i) => i,
            Err(e) => return set_status(&weak, format!("Failed to load instance: {e}")),
        };

        let default_name = format!("{}.mrpack", sanitize(&inst.config.name));
        let picked = rfd::FileDialog::new()
            .add_filter("Modrinth modpack", &["mrpack"])
            .set_file_name(&default_name)
            .set_title("Export .mrpack")
            .save_file();
        let Some(out) = picked else { return };

        set_busy(&weak, true);
        let result = inst.export_mrpack(&out);
        finish(&weak);
        match result {
            Ok(_) => {
                let fname = out
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                set_status(&weak, format!("Exported to {fname}."));
            }
            Err(e) => set_status(&weak, format!("Export failed: {e}")),
        }
    });
}

// --- Internal helpers ---

/// Rebuild the Slint results model from the cached hits on the UI thread,
/// picking up any icons decoded since the last build.
fn push_results(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let hits = state.modrinth_results.lock().unwrap();
        ui.global::<Logic>()
            .set_modrinth_results(convert::modrinth_results_model(&hits, &state));
    });
}

/// Spawn a best-effort icon fetch for each hit not already cached; rebuild the
/// model when one arrives. Mirrors `app::avatars::ensure`.
fn ensure_icons(state: &AppState, weak: &Weak<MainWindow>, hits: Vec<ModrinthSearchHit>) {
    let mut to_fetch = Vec::new();
    {
        let mut cache = state.modrinth_icon_cache.lock().unwrap();
        for hit in &hits {
            let Some(url) = hit.icon_url.clone() else { continue };
            if url.is_empty() || cache.contains_key(&hit.project_id) {
                continue;
            }
            cache.insert(hit.project_id.clone(), AvatarEntry::Pending);
            to_fetch.push((hit.project_id.clone(), url));
        }
    }
    for (project_id, url) in to_fetch {
        let state = state.clone();
        let weak = weak.clone();
        state.rt.clone().spawn(async move {
            let entry = match fetch_icon(&url).await {
                Some((rgba, width, height)) => AvatarEntry::Ready { rgba, width, height },
                None => AvatarEntry::Failed,
            };
            state.modrinth_icon_cache.lock().unwrap().insert(project_id, entry);
            push_results(&state, &weak);
        });
    }
}

async fn fetch_icon(url: &str) -> Option<(Vec<u8>, u32, u32)> {
    let resp = reqwest::get(url).await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((rgba.into_raw(), width, height))
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(resp.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

/// Drain a progress channel into the inline Modrinth status line.
async fn forward_progress(weak: &Weak<MainWindow>, rx: &mut mpsc::Receiver<ProgressUpdate>) {
    while let Some(update) = rx.recv().await {
        match update {
            ProgressUpdate::Started { message, .. } => set_status(weak, message),
            ProgressUpdate::Progress { completed, total, current_file } => {
                set_status(weak, format!("[{completed}/{total}] {current_file}"))
            }
            ProgressUpdate::Message(m) => set_status(weak, m),
            ProgressUpdate::Finished => {}
            ProgressUpdate::Error(e) => set_status(weak, format!("Error: {e}")),
        }
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn refresh_all(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::refresh_all(&ui, &state));
}

fn set_loading(weak: &Weak<MainWindow>, loading: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui.global::<Logic>().set_modrinth_loading(loading));
}

fn set_busy(weak: &Weak<MainWindow>, busy: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_busy(&ui, busy));
}

fn finish(weak: &Weak<MainWindow>) {
    set_busy(weak, false);
}

/// Report a failure: clear the busy flag and show the message.
fn fail(weak: &Weak<MainWindow>, msg: impl Into<String>) {
    finish(weak);
    set_status(weak, msg);
}

/// Show a message in the inline Modrinth status line and the global status text.
fn set_status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.global::<Logic>().set_modrinth_status(text.clone().into());
        ui::set_status(&ui, text);
    });
}
