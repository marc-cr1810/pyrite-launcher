//! Storage panel: disk-usage reporting, unused-cache scanning, and pruning.
//!
//! All filesystem walking happens on the runtime's blocking pool so a large
//! `.minecraft` directory never freezes the UI; results are marshalled back to
//! the Slint event loop via `upgrade_in_event_loop`.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Weak};

use crate::app::convert;
use crate::app::state::AppState;
use crate::core::storage::{self, OrphanScan};
use crate::{Logic, MainWindow};

/// Cache the most recent orphan scan so `prune` deletes exactly what the user
/// was shown, without rescanning between confirmation and deletion.
type SharedScan = Arc<Mutex<Option<OrphanScan>>>;

fn last_scan() -> &'static SharedScan {
    use std::sync::OnceLock;
    static SCAN: OnceLock<SharedScan> = OnceLock::new();
    SCAN.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn set_scanning(weak: &Weak<MainWindow>, scanning: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.global::<Logic>().set_storage_scanning(scanning);
    });
}

/// Measure disk usage and populate the Storage card.
pub fn refresh(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_scanning(&weak, true);
        let game_dir = state.game_dir();
        let report = tokio::task::spawn_blocking(move || storage::compute_report(&game_dir))
            .await
            .unwrap_or_default();
        let total = storage::format_bytes(report.total_bytes);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let logic = ui.global::<Logic>();
            logic.set_storage_items(convert::storage_model(&report));
            logic.set_storage_total(total.into());
            logic.set_storage_scanning(false);
        });
    });
}

/// Find versions/libraries/assets no installed instance references and show a
/// summary; the actual deletion waits for the confirm dialog.
pub fn scan(state: &AppState, weak: &Weak<MainWindow>) {
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_scanning(&weak, true);
        let game_dir = state.game_dir();
        let scan = tokio::task::spawn_blocking(move || storage::find_orphans(&game_dir))
            .await
            .unwrap_or_default();

        let summary = if scan.is_empty() {
            "Nothing to clean — caches are fully in use.".to_string()
        } else {
            format!(
                "{} files · {}",
                scan.count(),
                storage::format_bytes(scan.total_bytes)
            )
        };
        let has_orphans = !scan.is_empty();
        *last_scan().lock().unwrap() = Some(scan);

        let _ = weak.upgrade_in_event_loop(move |ui| {
            let logic = ui.global::<Logic>();
            logic.set_storage_orphan_summary(summary.into());
            logic.set_storage_has_orphans(has_orphans);
            logic.set_storage_scanning(false);
        });
    });
}

/// Delete the files from the most recent scan, then refresh the report.
pub fn prune(state: &AppState, weak: &Weak<MainWindow>) {
    let Some(scan) = last_scan().lock().unwrap().take() else {
        return;
    };
    let state = state.clone();
    let weak = weak.clone();
    state.rt.clone().spawn(async move {
        set_scanning(&weak, true);
        let freed = tokio::task::spawn_blocking(move || storage::prune(&scan))
            .await
            .unwrap_or(0);
        let msg = format!("Freed {}.", storage::format_bytes(freed));

        let _ = weak.upgrade_in_event_loop({
            let msg = msg.clone();
            move |ui| {
                let logic = ui.global::<Logic>();
                logic.set_storage_orphan_summary("".into());
                logic.set_storage_has_orphans(false);
                crate::app::ui::set_status(&ui, msg);
            }
        });
        // Re-measure so the card reflects the reclaimed space.
        refresh(&state, &weak);
    });
}

/// Open an instance's folder in the system file manager.
pub fn open_instance_folder(state: &AppState, id: String) {
    let path = state.game_dir().join("instances").join(&id);
    let _ = open::that(path);
}
