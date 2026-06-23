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

/// Open the crash-report file (or its folder) in the system default handler.
pub fn open_crash_report(path: String) {
    if path.is_empty() {
        return;
    }
    let _ = open::that(&path);
}

/// Copy a formatted summary of the crash analysis to the system clipboard.
pub fn copy_crash_details(
    title: String,
    description: String,
    solutions: Vec<String>,
    excerpt: Vec<String>,
) {
    let mut text = format!("{title}\n\n{description}\n\nWhat to try:\n");
    for s in solutions {
        text.push_str(&format!("- {s}\n"));
    }
    if !excerpt.is_empty() {
        text.push_str("\nLog excerpt:\n");
        for e in excerpt {
            text.push_str(&format!("{e}\n"));
        }
    }
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

/// Force-quit the running game, if any. The launcher publishes the live PID into
/// `state.running_pid` while the process is alive; killing it makes `child.wait()`
/// return, which tears down the launch flow normally (clearing the slot).
pub fn stop_game(state: &AppState) {
    let pid = *state.running_pid.lock().unwrap();
    if let Some(pid) = pid {
        let spid = sysinfo::Pid::from_u32(pid);
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
        if let Some(proc) = sys.process(spid) {
            proc.kill();
        }
    }
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
        // Clear any crash panel from a previous launch.
        let _ = weak.upgrade_in_event_loop(|ui| ui.global::<Logic>().set_crash_visible(false));

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

        // Refresh the Microsoft session if its token has expired, so launches
        // don't silently fail with stale credentials.
        status(&weak, "Checking account session…");
        let account = match crate::app::controllers::accounts::refresh_if_needed(&state, &account, false)
            .await
        {
            Ok(a) => a,
            Err(e) => {
                status(&weak, format!("Session expired — please sign in again. ({e})"));
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
        let weak_run = weak.clone();
        let consumer = std::thread::spawn(move || {
            let mut launched = false;
            while let Some(line) = log_rx.blocking_recv() {
                // The launcher emits this exactly when the game process spawns;
                // switch from the "preparing" UI to a "running" state so the
                // progress bar stops and the header reflects gameplay.
                if !launched && line.starts_with("Launch command:") {
                    launched = true;
                    let _ = weak_run.upgrade_in_event_loop(|ui| {
                        let logic = ui.global::<Logic>();
                        logic.set_progress_active(false);
                        logic.set_progress_indeterminate(false);
                        logic.set_game_running(true);
                        ui::set_status(&ui, "Minecraft running.");
                    });
                }
                log_buf.lock().unwrap().push(line);
                log_dirty.store(true, Ordering::SeqCst);
            }
        });

        let play_start = std::time::Instant::now();
        let launcher = Launcher::new(config.clone());
        let result = launcher
            .launch_with_logs(&instance, &account, Some(log_tx), Some(state.running_pid.clone()))
            .await;

        let _ = consumer.join();

        // Record playtime and last-played timestamp.
        let elapsed_secs = play_start.elapsed().as_secs();
        if elapsed_secs > 0 {
            let game_dir = state.game_dir();
            let inst_path = game_dir.join("instances").join(&instance.id);
            if let Ok(mut inst) = crate::core::instance::Instance::load(&instance.id, inst_path) {
                inst.config.last_played = Some(chrono::Utc::now().to_rfc3339());
                inst.config.total_playtime_secs += elapsed_secs;
                let _ = inst.save();
            }
        }

        match result {
            Ok(()) => status(&weak, "Minecraft exited."),
            Err(e) => {
                status(&weak, format!("Launch error: {e}"));

                let game_dir = state.game_dir();
                let inst_path = game_dir.join("instances").join(&instance.id);
                let log_path = inst_path.join("logs").join("latest.log");

                // Analyze the launcher's own error, the on-disk log, AND this
                // run's captured output. The launcher rejects some failures (e.g.
                // an incompatible Java version) before the process spawns, so the
                // cause lives only in `e`; early JVM failures print to stderr
                // before Minecraft writes latest.log, so they live only in the
                // in-memory buffer; and a stale latest.log could mask either.
                let file_log = std::fs::read_to_string(&log_path).unwrap_or_default();
                let buf_log = state.log_buf.lock().unwrap().join("\n");
                let log_content = format!("{e}\n{file_log}\n{buf_log}");

                if let Some(analysis) = crate::core::crash_analyzer::analyze_crash(&inst_path, &log_content) {
                    // Resolve the one-click remedy (if any) against this instance.
                    let system_ram = crate::core::config::system_ram_mb()
                        .unwrap_or(8192)
                        .min(i32::MAX as u64) as i32;
                    let (fix_kind, fix_label, fix_arg) =
                        crate::app::controllers::crashfix::describe(&analysis.fix, &instance, system_ram);

                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        let logic = ui.global::<Logic>();
                        logic.set_crash_title(analysis.title.into());
                        logic.set_crash_description(analysis.description.into());
                        logic.set_crash_category(analysis.category.into());
                        logic.set_crash_report_path(analysis.report_path.unwrap_or_default().into());
                        logic.set_crash_fix_kind(fix_kind.into());
                        logic.set_crash_fix_label(fix_label.into());
                        logic.set_crash_fix_arg(fix_arg.into());
                        logic.set_crash_fixing(false);

                        let solutions: Vec<slint::SharedString> =
                            analysis.possible_solutions.into_iter().map(|s| s.into()).collect();
                        logic.set_crash_solutions(slint::ModelRc::new(slint::VecModel::from(solutions)));

                        let excerpt: Vec<slint::SharedString> =
                            analysis.excerpt.into_iter().map(|s| s.into()).collect();
                        logic.set_crash_excerpt(slint::ModelRc::new(slint::VecModel::from(excerpt)));

                        logic.set_crash_visible(true);
                    });
                }
            }
        }
        finish(&state, &weak);
        // Refresh the instance list so the new playtime stats show up.
        let st = state.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| ui::refresh_instances(&ui, &st.config.lock().unwrap()));
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
    let downloader = Downloader::with_concurrency(tx, config.download_concurrency);
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
    let _ = weak.upgrade_in_event_loop(|ui| ui.global::<Logic>().set_game_running(false));
}

fn set_busy(weak: &Weak<MainWindow>, busy: bool) {
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_busy(&ui, busy));
}

fn status(weak: &Weak<MainWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = weak.upgrade_in_event_loop(move |ui| ui::set_status(&ui, text));
}
