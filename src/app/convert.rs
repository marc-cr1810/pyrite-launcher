//! Build Slint UI models from core data structures.

use slint::{Color, ModelRc, SharedString, VecModel};

use crate::core::config::{AccountType, Config};
use crate::core::instance::Instance;
use crate::{AccountItem, InstanceItem, LogLine};

/// Uppercased first character of a name, for monogram avatars. "?" if empty.
pub fn initial(name: &str) -> SharedString {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
        .into()
}

/// A stable, pleasant color derived from a key (uuid / instance id), so each
/// account or instance keeps the same avatar tint across launches.
pub fn avatar_color(key: &str) -> Color {
    // A small curated palette that reads well on the dark surfaces.
    const PALETTE: [(u8, u8, u8); 8] = [
        (122, 162, 247), // blue
        (158, 206, 106), // green
        (224, 175, 104), // amber
        (247, 118, 142), // red
        (187, 154, 247), // purple
        (125, 207, 255), // cyan
        (255, 158, 100), // orange
        (115, 218, 202), // teal
    ];
    let hash = key.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = PALETTE[(hash as usize) % PALETTE.len()];
    Color::from_rgb_u8(r, g, b)
}

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
            initial: initial(&a.username),
            avatar_color: avatar_color(&a.uuid),
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
                initial: initial(&inst.config.name),
                avatar_color: avatar_color(&inst.id),
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
