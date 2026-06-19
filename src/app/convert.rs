//! Build Slint UI models from core data structures.

use slint::{ModelRc, SharedString, VecModel};

use crate::core::config::{AccountType, Config};
use crate::core::instance::Instance;
use crate::{AccountItem, InstanceItem, LogLine};

pub fn accounts_model(config: &Config) -> ModelRc<AccountItem> {
    let active = config.active_account_uuid.clone();
    let items: Vec<AccountItem> = config
        .accounts
        .iter()
        .map(|a| AccountItem {
            uuid: a.uuid.clone().into(),
            username: a.username.clone().into(),
            kind: match a.account_type {
                AccountType::Microsoft => "Microsoft".into(),
                AccountType::Offline => "Offline".into(),
            },
            active: active.as_deref() == Some(a.uuid.as_str()),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn instances_model(config: &Config) -> ModelRc<InstanceItem> {
    let game_dir = &config.game_dir;
    let active = config.active_instance.clone();
    let items: Vec<InstanceItem> = Instance::load_all(game_dir)
        .into_iter()
        .map(|inst| {
            let (game_version, loader) = inst.get_game_version_and_loader(game_dir);
            let loader_label = match loader.as_deref() {
                Some("fabric") => "Fabric",
                Some("forge") => "Forge",
                Some("neoforge") => "NeoForge",
                Some(other) => other,
                None => "",
            };
            InstanceItem {
                id: inst.id.clone().into(),
                name: inst.config.name.clone().into(),
                version: game_version.into(),
                loader: SharedString::from(loader_label),
                active: active.as_deref() == Some(inst.id.as_str()),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn string_model(items: Vec<String>) -> ModelRc<SharedString> {
    let v: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(v))
}

pub fn loglines_model(lines: &[String]) -> ModelRc<LogLine> {
    let items: Vec<LogLine> = lines
        .iter()
        .map(|line| LogLine {
            level: log_level(line).into(),
            text: line.clone().into(),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Classify a log line by severity for coloring. Matches the log4j-style tags
/// Minecraft emits (e.g. "[12:00:00] [main/ERROR]: ...").
fn log_level(line: &str) -> &'static str {
    let upper = line.to_uppercase();
    if upper.contains("/ERROR]") || upper.contains("/FATAL]") || upper.contains("EXCEPTION") {
        "error"
    } else if upper.contains("/WARN]") {
        "warn"
    } else if line.starts_with("Launch command:") || line.starts_with('→') {
        "plain"
    } else {
        "info"
    }
}
