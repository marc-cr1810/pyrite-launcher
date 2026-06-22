//! Pre-flight conflict check: before the user presses Play, statically scan the
//! active instance's mod jars (`Instance::preflight`) for duplicate mods and
//! missing mandatory dependencies, and surface them as a dismissible banner on
//! the Play view. Each issue can carry a one-click fix (install the dependency
//! from Modrinth, or disable the duplicate jar).

use slint::ComponentHandle;
use slint::Weak;

use crate::app::controllers::crashfix;
use crate::app::state::AppState;
use crate::app::ui;
use crate::core::instance::{Instance, PreflightFix};
use crate::{Logic, MainWindow, PreflightItem};

/// Recompute the pre-flight issues for the active instance and update the banner.
pub fn run(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let id = state.config.lock().unwrap().active_instance.clone();
        let Some(id) = id else {
            return publish(&weak, Vec::new());
        };
        let game_dir = state.game_dir();
        let issues = match Instance::load(&id, game_dir.join("instances").join(&id)) {
            Ok(inst) => inst.preflight(),
            Err(_) => Vec::new(),
        };

        let items: Vec<PreflightItem> = issues
            .into_iter()
            .map(|i| {
                let (fix_kind, fix_arg, fix_label) = match i.fix {
                    Some(PreflightFix::InstallDependency { query, display }) => {
                        ("install-dep".to_string(), query, format!("Install {display}"))
                    }
                    Some(PreflightFix::DisableMod { filename }) => {
                        ("disable-mod".to_string(), filename, "Disable duplicate".to_string())
                    }
                    None => (String::new(), String::new(), String::new()),
                };
                PreflightItem {
                    kind: i.kind.into(),
                    message: i.message.into(),
                    fix_kind: fix_kind.into(),
                    fix_arg: fix_arg.into(),
                    fix_label: fix_label.into(),
                }
            })
            .collect();

        publish(&weak, items);
    });
}

/// Apply a fix from the banner, then recompute the issues.
pub fn fix(state: &AppState, weak: &Weak<MainWindow>, fix_kind: String, fix_arg: String) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        let id = match state.config.lock().unwrap().active_instance.clone() {
            Some(id) => id,
            None => return,
        };
        set_fixing(&weak, true);

        let result: Result<String, String> = match fix_kind.as_str() {
            "install-dep" => {
                status(&weak, format!("Installing {fix_arg}…"));
                crashfix::install_dependency(&state, &id, &fix_arg)
                    .await
                    .map(|name| format!("Installed {name}."))
            }
            "disable-mod" => {
                let game_dir = state.game_dir();
                match Instance::load(&id, game_dir.join("instances").join(&id)) {
                    Ok(inst) => inst
                        .disable_mod(&fix_arg)
                        .map(|_| format!("Disabled {fix_arg}.")),
                    Err(e) => Err(e),
                }
            }
            other => Err(format!("Unknown fix '{other}'.")),
        };

        set_fixing(&weak, false);
        match result {
            Ok(msg) => status(&weak, msg),
            Err(e) => status(&weak, format!("Fix failed: {e}")),
        }
        run(&state, &weak);
    });
}

/// Hide the banner without changing anything.
pub fn dismiss(weak: &Weak<MainWindow>) {
    let _ = weak.upgrade_in_event_loop(|ui| ui.global::<Logic>().set_preflight_visible(false));
}

fn publish(weak: &Weak<MainWindow>, items: Vec<PreflightItem>) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        let logic = ui.global::<Logic>();
        let visible = !items.is_empty();
        logic.set_preflight_issues(slint::ModelRc::new(slint::VecModel::from(items)));
        logic.set_preflight_visible(visible);
    });
}

fn set_fixing(weak: &Weak<MainWindow>, fixing: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui.global::<Logic>().set_preflight_fixing(fixing));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
