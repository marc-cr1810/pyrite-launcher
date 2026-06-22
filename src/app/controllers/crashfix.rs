//! One-click crash remedies. The crash analyzer (`core::crash_analyzer`) names a
//! `CrashFix`; here we turn it into a button label (`describe`) and carry it out
//! against the active instance (`apply`) — bumping memory, switching to
//! auto-detected Java, disabling an offending mod, or installing a missing
//! dependency from Modrinth. Memory/Java fixes relaunch the game when done.

use slint::ComponentHandle;
use slint::Weak;

use crate::app::controllers::{instances, launch, preflight};
use crate::app::state::AppState;
use crate::app::ui;
use crate::core::api::ApiClient;
use crate::core::crash_analyzer::CrashFix;
use crate::core::instance::Instance;
use crate::{Logic, MainWindow};

/// Suggest a new max-heap (MB) given the current `-Xmx` (0 = unset) and physical
/// RAM: roughly +50% with a 6 GB floor, capped at 75% of system RAM.
pub fn suggest_memory(current_mb: i32, system_mb: i32) -> i32 {
    let base = if current_mb > 0 { current_mb } else { 4096 };
    let bumped = (base * 3 / 2).max(6144);
    let cap = (system_mb * 3 / 4).max(base + 512);
    bumped.min(cap)
}

/// Map a `CrashFix` to the UI's `(kind, label, arg)` triple. `kind` is "" when
/// there's no one-click remedy.
pub fn describe(fix: &Option<CrashFix>, inst: &Instance, system_ram_mb: i32) -> (String, String, String) {
    match fix {
        Some(CrashFix::IncreaseMemory) => {
            let args = inst.config.jvm_args.clone().unwrap_or_default();
            let (current, _) = instances::split_memory_args(&args);
            let target = suggest_memory(current.unwrap_or(0) as i32, system_ram_mb);
            ("memory".to_string(), format!("Allocate {} & relaunch", human_mem(target)), String::new())
        }
        Some(CrashFix::UseAutoJava) => (
            "java".to_string(),
            "Use auto-detected Java & relaunch".to_string(),
            String::new(),
        ),
        Some(CrashFix::DisableMod { name_hint }) => (
            "disable-mod".to_string(),
            format!("Disable {name_hint} & relaunch"),
            name_hint.clone(),
        ),
        Some(CrashFix::InstallDependency { query, display }) => {
            ("install-dep".to_string(), format!("Install {display}"), query.clone())
        }
        None => (String::new(), String::new(), String::new()),
    }
}

/// Carry out the remedy identified by `kind` (+ `arg`) against the active
/// instance.
pub fn apply(state: &AppState, weak: &Weak<MainWindow>, kind: String, arg: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_fixing(&weak, true);

        let id = match state.config.lock().unwrap().active_instance.clone() {
            Some(id) => id,
            None => {
                set_fixing(&weak, false);
                return status(&weak, "Select an instance first.");
            }
        };
        let game_dir = state.game_dir();
        let mut inst = match Instance::load(&id, game_dir.join("instances").join(&id)) {
            Ok(i) => i,
            Err(e) => {
                set_fixing(&weak, false);
                return status(&weak, format!("Failed to load instance: {e}"));
            }
        };

        match kind.as_str() {
            "memory" => {
                let args = inst.config.jvm_args.clone().unwrap_or_default();
                let (current, extra) = instances::split_memory_args(&args);
                let system = crate::core::config::system_ram_mb().unwrap_or(8192).min(i32::MAX as u64) as i32;
                let target = suggest_memory(current.unwrap_or(0) as i32, system);
                inst.config.jvm_args = instances::build_jvm_args(target, &extra);
                if let Err(e) = inst.save() {
                    set_fixing(&weak, false);
                    return status(&weak, format!("Failed to save settings: {e}"));
                }
                finish_and_relaunch(&state, &weak, format!("Allocated {} — relaunching…", human_mem(target)));
            }
            "java" => {
                inst.config.java_path = None;
                inst.config.java_version = None;
                if let Err(e) = inst.save() {
                    set_fixing(&weak, false);
                    return status(&weak, format!("Failed to save settings: {e}"));
                }
                finish_and_relaunch(&state, &weak, "Switched to auto-detected Java — relaunching…");
            }
            "disable-mod" => {
                match disable_matching_mod(&inst, &arg) {
                    Ok(filename) => {
                        preflight::run(&state, &weak);
                        finish_and_relaunch(&state, &weak, format!("Disabled {filename} — relaunching…"));
                    }
                    Err(e) => {
                        set_fixing(&weak, false);
                        status(&weak, e);
                    }
                }
            }
            "install-dep" => {
                status(&weak, format!("Installing {arg}…"));
                match install_dependency(&state, &id, &arg).await {
                    Ok(name) => {
                        hide_crash(&weak);
                        set_fixing(&weak, false);
                        status(&weak, format!("Installed {name}. Press Play to try again."));
                        preflight::run(&state, &weak);
                    }
                    Err(e) => {
                        set_fixing(&weak, false);
                        status(&weak, format!("Install failed: {e}"));
                    }
                }
            }
            other => {
                set_fixing(&weak, false);
                status(&weak, format!("Unknown fix '{other}'."));
            }
        }
    });
}

/// Find an enabled jar whose filename, mod id, or display name contains
/// `name_hint` (case-insensitive) and disable it. Returns the disabled filename.
fn disable_matching_mod(inst: &Instance, name_hint: &str) -> Result<String, String> {
    let hint = name_hint.to_lowercase();
    let mods = inst.get_mods().unwrap_or_default();
    let target = mods.iter().find(|m| {
        m.enabled
            && (m.filename.to_lowercase().contains(&hint)
                || m.metadata.id.to_lowercase().contains(&hint)
                || m.metadata.name.to_lowercase().contains(&hint))
    });
    match target {
        Some(m) => {
            inst.disable_mod(&m.filename)?;
            Ok(m.filename.clone())
        }
        None => Err(format!("Couldn't find an installed mod matching '{name_hint}'.")),
    }
}

/// Resolve `query` to a Modrinth mod compatible with the instance and install
/// its newest matching version. Returns the installed filename. Shared with the
/// pre-flight checker.
pub(crate) async fn install_dependency(
    state: &AppState,
    instance_id: &str,
    query: &str,
) -> Result<String, String> {
    let game_dir = state.game_dir();
    let mut inst = Instance::load(instance_id, game_dir.join("instances").join(instance_id))?;
    let (game_version, loader) = inst.get_game_version_and_loader(&game_dir);

    let api = ApiClient::new();
    let resp = api
        .search_projects(query, Some(&game_version), loader.as_deref(), "mod", 0, 1)
        .await?;
    let hit = resp
        .hits
        .into_iter()
        .next()
        .ok_or_else(|| format!("No Modrinth mod found for '{query}'."))?;

    let versions = api.fetch_modpack_versions(&hit.project_id).await?;
    let version = versions
        .into_iter()
        .find(|v| {
            v.game_versions.contains(&game_version)
                && loader
                    .as_ref()
                    .map_or(true, |l| v.loaders.iter().any(|x| x.eq_ignore_ascii_case(l)))
        })
        .ok_or_else(|| format!("No '{query}' build for Minecraft {game_version}."))?;
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
        .ok_or("That version has no downloadable file.")?
        .clone();

    inst.install_mod_from_url(&game_dir, &file.filename, &file.url, file.hashes.sha1.as_deref(), true)
        .await?;
    Ok(file.filename)
}

/// Human-friendly memory string, e.g. 6144 → "6 GB", 5120 → "5 GB".
fn human_mem(mb: i32) -> String {
    if mb % 1024 == 0 {
        format!("{} GB", mb / 1024)
    } else {
        format!("{:.1} GB", mb as f32 / 1024.0)
    }
}

/// Hide the crash panel, clear the fixing flag, set a status, then relaunch.
fn finish_and_relaunch(state: &AppState, weak: &Weak<MainWindow>, msg: impl Into<String>) {
    hide_crash(weak);
    set_fixing(weak, false);
    status(weak, msg);
    launch::play(state, weak);
}

fn hide_crash(weak: &Weak<MainWindow>) {
    let _ = weak.upgrade_in_event_loop(|ui| ui.global::<Logic>().set_crash_visible(false));
}

fn set_fixing(weak: &Weak<MainWindow>, fixing: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui.global::<Logic>().set_crash_fixing(fixing));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
