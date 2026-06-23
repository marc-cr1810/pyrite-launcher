//! Verify & repair an instance's game files. Re-hashes the client JAR, the
//! libraries, and every asset object against the version manifest (no network),
//! then — only if something is missing or corrupt — re-downloads exactly the bad
//! files via the normal downloader. Surfaced as a button in the instance detail
//! Settings tab; results land in the global status line.

use slint::ComponentHandle;
use slint::Weak;

use crate::app::controllers::launch;
use crate::app::state::AppState;
use crate::app::ui;
use crate::core::downloader::Downloader;
use crate::core::instance::Instance;
use crate::{Logic, MainWindow};

/// Verify the active version files for instance `id`, repairing anything broken.
pub fn verify_and_repair(state: &AppState, weak: &Weak<MainWindow>, id: String) {
    // Don't run on top of a launch/download.
    {
        let mut busy = state.busy.lock().unwrap();
        if *busy {
            return;
        }
        *busy = true;
    }

    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_busy(&weak, true);
        ui::progress_indeterminate_async(&weak, true);

        let (config, version) = {
            let cfg = state.config.lock().unwrap();
            let path = cfg.game_dir.join("instances").join(&id);
            match Instance::load(&id, path) {
                Ok(inst) => (cfg.clone(), inst.config.version.clone()),
                Err(e) => {
                    status(&weak, format!("Failed to load instance: {e}"));
                    finish(&state, &weak);
                    return;
                }
            }
        };

        // Resolve the version's full details (cheap when the JSON is already on
        // disk), then run the check-only pass.
        status(&weak, "Resolving version files…");
        let details = match launch::resolve_version_details(&config, &version).await {
            Ok(d) => d,
            Err(e) => {
                status(&weak, format!("Verify failed: {e}"));
                finish(&state, &weak);
                return;
            }
        };

        status(&weak, "Verifying game files…");
        let game_dir = config.game_dir.clone();
        let report = {
            let details = details.clone();
            match tokio::task::spawn_blocking(move || Downloader::verify_version(&game_dir, &details))
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    status(&weak, format!("Verify failed: {e}"));
                    finish(&state, &weak);
                    return;
                }
            }
        };

        if report.is_ok() {
            status(&weak, "All game files verified — no problems found.");
            finish(&state, &weak);
            return;
        }

        // Repair: re-download only the bad files (the downloader skips valid ones).
        let bad = report.total_bad();
        status(&weak, format!("Found {bad} bad file(s) ({}). Repairing…", report.summary()));
        match launch::download_resolved(&config, &details, &weak).await {
            Ok(()) => status(&weak, format!("Repaired {bad} file(s).")),
            Err(e) => status(&weak, format!("Repair failed: {e}")),
        }
        finish(&state, &weak);
    });
}

fn finish(state: &AppState, weak: &Weak<MainWindow>) {
    *state.busy.lock().unwrap() = false;
    set_busy(weak, false);
    ui::progress_async(weak, false, 0.0);
}

fn set_busy(weak: &Weak<MainWindow>, busy: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_busy(&ui, busy));
}

/// Update both the global status line and the inline repair status (the latter
/// is what's visible on the Instances view, where this flow runs).
fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui::set_status(&ui, text.clone());
        ui.global::<Logic>().set_repair_status(text.into());
    });
}
