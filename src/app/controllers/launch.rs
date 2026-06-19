//! Play flow: ensure version files are downloaded (with progress), then launch
//! the game streaming its log output into the UI.
use slint::ComponentHandle;

use std::sync::atomic::Ordering;

use slint::Weak;
use tokio::sync::mpsc;

use crate::app::convert;
use crate::app::state::AppState;
use crate::app::ui;
use crate::core::api::ApiClient;
use crate::core::config::Config;
use crate::core::downloader::{Downloader, ProgressUpdate};
use crate::core::launcher::Launcher;
use crate::{Logic, MainWindow};

pub fn clear_logs(state: &AppState, weak: &Weak<MainWindow>) {
    state.log_buf.lock().unwrap().clear();
    state.log_dirty.store(false, std::sync::atomic::Ordering::SeqCst);
    let _ = weak.upgrade_in_event_loop(|ui| {
        ui.global::<Logic>().set_log_lines(convert::loglines_model(&[]));
    });
}

/// Copy the full game log to the system clipboard.
pub fn copy_logs(state: &AppState) {
    let text = state.log_buf.lock().unwrap().join("\n");
    if text.trim().is_empty() {
        return;
    }
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

pub fn play(state: &AppState, weak: &Weak<MainWindow>) {
    // Guard against overlapping launches.
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

        // Snapshot what we need from config.
        let (config, instance, account) = {
            let cfg = state.config.lock().unwrap();
            let inst_id = cfg.active_instance.clone();
            let account = cfg.get_active_account().cloned();
            let instance = inst_id.and_then(|id| {
                let path = cfg.game_dir.join("instances").join(&id);
                crate::core::instance::Instance::load(&id, path).ok()
            });
            (cfg.clone(), instance, account)
        };

        let (instance, account) = match (instance, account) {
            (Some(i), Some(a)) => (i, a),
            _ => {
                status(&weak, "Select an account and instance first.");
                finish(&state, &weak);
                return;
            }
        };

        // Reset the live log buffer for this launch; the UI-thread timer in
        // `app::run` flushes it into the model.
        state.log_buf.lock().unwrap().clear();
        state.log_dirty.store(true, Ordering::SeqCst);

        // 1. Ensure version files are present.
        status(&weak, "Checking game files…");
        if let Err(e) = download_version_files(&config, &instance.config.version, &weak).await {
            status(&weak, format!("Download failed: {e}"));
            finish(&state, &weak);
            return;
        }
        ui::progress_async(&weak, false, 1.0);

        // 2. Launch, streaming logs.
        //
        // `launch_with_logs` blocks on `child.wait()`, so we both run it on a
        // blocking thread and drain its log channel on a *dedicated OS thread*.
        // A tokio task here would be starved behind the blocking launch and only
        // flush once the game exits.
        status(&weak, "Launching Minecraft…");
        ui::progress_indeterminate_async(&weak, true);
        let (log_tx, mut log_rx) = mpsc::channel::<String>(1024);
        let log_buf = state.log_buf.clone();
        let log_dirty = state.log_dirty.clone();
        let consumer = std::thread::spawn(move || {
            while let Some(line) = log_rx.blocking_recv() {
                log_buf.lock().unwrap().push(line);
                log_dirty.store(true, Ordering::SeqCst);
            }
        });

        let launcher = Launcher::new(config.clone());
        let result = launcher
            .launch_with_logs(&instance, &account, Some(log_tx))
            .await;

        let _ = consumer.join();

        match result {
            Ok(()) => status(&weak, "Minecraft exited."),
            Err(e) => status(&weak, format!("Launch error: {e}")),
        }
        finish(&state, &weak);
    });
}

/// Mirrors minecli's `download_version_files`: resolve details (from local JSON
/// or by fetching the appropriate profile), persist them, then download.
async fn download_version_files(
    config: &Config,
    version_id: &str,
    weak: &Weak<MainWindow>,
) -> Result<(), String> {
    let api = ApiClient::new();
    let json_path = config
        .game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{version_id}.json"));

    let details = if json_path.exists() {
        Launcher::new(config.clone()).load_version_details_raw(version_id)?
    } else if let Some(rest) = version_id.strip_prefix("fabric-loader-") {
        let (loader_ver, game_ver) = rest
            .split_once('-')
            .ok_or_else(|| format!("Invalid Fabric version id: {version_id}"))?;
        api.fetch_fabric_profile(game_ver, loader_ver).await?
    } else if let Some(ver) = version_id.strip_prefix("forge-") {
        api.fetch_forge_profile(ver).await?
    } else if let Some(ver) = version_id.strip_prefix("neoforge-") {
        api.fetch_neoforge_profile(ver).await?
    } else {
        let manifest = api.fetch_version_manifest().await?;
        let brief = manifest
            .versions
            .iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| format!("Version '{version_id}' not found in Mojang manifest."))?;
        api.fetch_version_details(&brief.url).await?
    };

    if !json_path.exists() {
        if let Some(parent) = json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = serde_json::to_string_pretty(&details).map_err(|e| e.to_string())?;
        std::fs::write(&json_path, content).map_err(|e| e.to_string())?;
    }

    let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
    let downloader = Downloader::new(tx);
    let game_dir = config.game_dir.clone();
    let dl_details = details.clone();
    let dl_task = tokio::spawn(async move {
        downloader.download_version(&game_dir, &dl_details).await
    });

    // Track per-phase timing so we can show a download rate and rough ETA.
    // Progress is file-count based, so the rate is in files/s.
    let mut phase_start = std::time::Instant::now();
    while let Some(update) = rx.recv().await {
        match update {
            ProgressUpdate::Started { message, .. } => {
                phase_start = std::time::Instant::now();
                status(weak, message);
                // No counts yet for this phase — sweep until the first Progress.
                ui::progress_indeterminate_async(weak, true);
            }
            ProgressUpdate::Progress { completed, total, current_file } => {
                let frac = if total > 0 { completed as f32 / total as f32 } else { 0.0 };
                ui::progress_async(weak, true, frac);
                status(weak, format!("[{completed}/{total}] {current_file}{}",
                    rate_suffix(completed, total, phase_start.elapsed())));
            }
            ProgressUpdate::Message(m) => {
                status(weak, m);
                ui::progress_indeterminate_async(weak, true);
            }
            ProgressUpdate::Finished => ui::progress_async(weak, true, 1.0),
            ProgressUpdate::Error(e) => return Err(e),
        }
    }

    dl_task.await.map_err(|e| e.to_string())?
}

/// Format a "  ·  N files/s  ·  ETA Ms" suffix from the count completed so far
/// in the current phase. Returns empty until there's enough signal to be useful.
fn rate_suffix(completed: usize, total: usize, elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs_f32();
    if completed == 0 || secs < 0.5 {
        return String::new();
    }
    let rate = completed as f32 / secs;
    if rate < 1.0 {
        return String::new();
    }
    let remaining = total.saturating_sub(completed);
    let eta = (remaining as f32 / rate).ceil() as u64;
    let eta_str = if remaining > 0 && eta > 0 {
        format!("  ·  ETA {eta}s")
    } else {
        String::new()
    };
    format!("  ·  {rate:.0} files/s{eta_str}")
}

fn finish(state: &AppState, weak: &Weak<MainWindow>) {
    *state.busy.lock().unwrap() = false;
    set_busy(weak, false);
    ui::progress_async(weak, false, 0.0);
}

fn set_busy(weak: &Weak<MainWindow>, busy: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_busy(&ui, busy));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
