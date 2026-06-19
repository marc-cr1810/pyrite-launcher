//! Populate the version pickers in the "New Instance" dialog.
use slint::ComponentHandle;

use slint::Weak;

use crate::app::convert;
use crate::app::state::AppState;
use crate::core::api::ApiClient;
use crate::{Logic, MainWindow};

/// Fetch the Mojang manifest and fill the Minecraft version list.
pub fn load_versions(state: &AppState, weak: &Weak<MainWindow>) {
    let weak = weak.clone();
    set_versions_loading(&weak, true);
    state.rt.spawn(async move {
        let api = ApiClient::new();
        let versions: Vec<String> = match api.fetch_version_manifest().await {
            Ok(manifest) => manifest.versions.into_iter().map(|v| v.id).collect(),
            Err(_) => Vec::new(),
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let logic = ui.global::<Logic>();
            logic.set_version_list(convert::string_model(versions));
            logic.set_versions_loading(false);
        });
    });
}

/// Fetch available loader versions for the chosen loader + Minecraft version.
pub fn load_loader_versions(
    state: &AppState,
    weak: &Weak<MainWindow>,
    loader: String,
    game_version: String,
) {
    let weak = weak.clone();
    set_loader_loading(&weak, true);
    state.rt.spawn(async move {
        let api = ApiClient::new();
        let list: Vec<String> = match loader.to_lowercase().as_str() {
            "fabric" => api
                .fetch_fabric_loaders(&game_version)
                .await
                .map(|loaders| loaders.into_iter().map(|l| l.loader.version).collect())
                .unwrap_or_default(),
            "forge" | "neoforge" => {
                let index = if loader.eq_ignore_ascii_case("neoforge") {
                    api.fetch_neoforge_versions().await
                } else {
                    api.fetch_forge_versions().await
                };
                index
                    .map(|idx| {
                        idx.versions
                            .into_iter()
                            .filter(|v| {
                                v.requires.iter().any(|r| {
                                    r.uid == "net.minecraft" && r.equals == game_version
                                })
                            })
                            .map(|v| v.version)
                            .collect()
                    })
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let logic = ui.global::<Logic>();
            logic.set_loader_version_list(convert::string_model(list));
            logic.set_loader_versions_loading(false);
        });
    });
}

fn set_versions_loading(weak: &Weak<MainWindow>, loading: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui.global::<Logic>().set_versions_loading(loading));
}

fn set_loader_loading(weak: &Weak<MainWindow>, loading: bool) {
    let _ = weak
        .upgrade_in_event_loop(move |ui| ui.global::<Logic>().set_loader_versions_loading(loading));
}
