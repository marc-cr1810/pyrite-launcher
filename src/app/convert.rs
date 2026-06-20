//! Build Slint UI models from core data structures.

use std::path::Path;

use std::io::Read;

use slint::{Color, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::app::avatars;
use crate::app::icons;
use crate::app::state::AppState;
use crate::core::assets::{AssetInfo, ScreenshotInfo, WorldInfo};
use crate::core::config::{AccountType, Config};
use crate::core::instance::{Instance, InstanceMod};
use crate::{
    AccountItem, AssetItem, BackupItem, FmtLine, FmtSpan, InstanceItem, LogLine, ModItem,
    ScreenshotItem, WorldItem,
};

/// Glyph to render in place of the monogram for a built-in instance icon.
/// Empty string means "no glyph" (use the monogram initial, or a custom image).
pub fn instance_icon_glyph(icon: &Option<String>) -> SharedString {
    match icon.as_deref() {
        Some("custom") | None => SharedString::new(),
        Some(id) => icons::glyph_for(id).unwrap_or("").into(),
    }
}

/// The custom `icon.png` for an instance as a Slint image, or an empty image
/// when the instance does not use a custom icon (or the file is missing).
pub fn instance_icon_image(instance_path: &Path, icon: &Option<String>) -> Image {
    if icon.as_deref() == Some("custom") {
        Image::load_from_path(&instance_path.join("icon.png")).unwrap_or_default()
    } else {
        Image::default()
    }
}

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

pub fn accounts_model(config: &Config, state: &AppState) -> ModelRc<AccountItem> {
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
            avatar: avatars::image_for(state, &a.uuid),
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
                jvm_args: inst.config.jvm_args.as_deref().unwrap_or(&[]).join(" ").into(),
                java_path: inst.config.java_path.clone().unwrap_or_default().into(),
                pre_launch: inst.config.pre_launch.clone().unwrap_or_default().into(),
                post_exit: inst.config.post_exit.clone().unwrap_or_default().into(),
                icon_id: inst.config.icon.clone().unwrap_or_default().into(),
                icon_glyph: instance_icon_glyph(&inst.config.icon),
                icon_image: instance_icon_image(&inst.path, &inst.config.icon),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Format a byte count as a short human-readable size (e.g. "4.2 MB").
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

/// Strip a trailing ".disabled" for display, leaving the user-facing name.
fn strip_disabled(filename: &str) -> &str {
    filename.strip_suffix(".disabled").unwrap_or(filename)
}

/// Default span color when no `§` color code is active (matches Theme.text-muted).
const DEFAULT_FMT_COLOR: (u8, u8, u8) = (0xa9, 0xa9, 0xc2);

/// Map a Minecraft `§` color code to RGB. Returns `None` for non-color codes.
fn mc_color(code: char) -> Option<(u8, u8, u8)> {
    Some(match code {
        '0' => (0x00, 0x00, 0x00),
        '1' => (0x00, 0x00, 0xaa),
        '2' => (0x00, 0xaa, 0x00),
        '3' => (0x00, 0xaa, 0xaa),
        '4' => (0xaa, 0x00, 0x00),
        '5' => (0xaa, 0x00, 0xaa),
        '6' => (0xff, 0xaa, 0x00),
        '7' => (0xaa, 0xaa, 0xaa),
        '8' => (0x55, 0x55, 0x55),
        '9' => (0x55, 0x55, 0xff),
        'a' => (0x55, 0xff, 0x55),
        'b' => (0x55, 0xff, 0xff),
        'c' => (0xff, 0x55, 0x55),
        'd' => (0xff, 0x55, 0xff),
        'e' => (0xff, 0xff, 0x55),
        'f' => (0xff, 0xff, 0xff),
        _ => return None,
    })
}

/// Parse a Minecraft `§`-formatted string into colored lines/spans for rendering.
/// Color codes set the color and reset bold/italic (MC semantics); `l`/`o` add
/// bold/italic; `r` resets all. Other codes (`k`/`m`/`n`) are ignored.
pub fn parse_formatted(s: &str) -> ModelRc<FmtLine> {
    let lines: Vec<FmtLine> = s
        .split('\n')
        .map(|line| {
            let mut spans: Vec<FmtSpan> = Vec::new();
            let mut buf = String::new();
            let mut color = DEFAULT_FMT_COLOR;
            let mut bold = false;
            let mut italic = false;

            let flush = |spans: &mut Vec<FmtSpan>, buf: &mut String, color, bold, italic| {
                if !buf.is_empty() {
                    let (r, g, b) = color;
                    spans.push(FmtSpan {
                        text: std::mem::take(buf).into(),
                        color: Color::from_rgb_u8(r, g, b),
                        bold,
                        italic,
                    });
                }
            };

            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '§' {
                    if let Some(code) = chars.next() {
                        flush(&mut spans, &mut buf, color, bold, italic);
                        let code = code.to_ascii_lowercase();
                        if let Some(rgb) = mc_color(code) {
                            color = rgb;
                            bold = false;
                            italic = false;
                        } else if code == 'l' {
                            bold = true;
                        } else if code == 'o' {
                            italic = true;
                        } else if code == 'r' {
                            color = DEFAULT_FMT_COLOR;
                            bold = false;
                            italic = false;
                        }
                    }
                } else {
                    buf.push(c);
                }
            }
            flush(&mut spans, &mut buf, color, bold, italic);
            FmtLine {
                spans: ModelRc::new(VecModel::from(spans)),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(lines))
}

/// Remove Minecraft formatting codes (`§` followed by a code char) so pack/mod
/// descriptions read as plain text instead of showing raw `§6`, `§l`, etc.
fn strip_mc_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next(); // drop the code character that follows
        } else {
            out.push(c);
        }
    }
    out
}

/// Decode PNG bytes into a `slint::Image`, or an empty image on failure.
fn image_from_png_bytes(bytes: &[u8]) -> Image {
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
            buf.make_mut_bytes().copy_from_slice(&rgba.into_raw());
            Image::from_rgba8(buf)
        }
        Err(_) => Image::default(),
    }
}

/// The `pack.png` thumbnail for a resource/shader pack, or an empty image when
/// none exists. Handles both folder packs (`<pack>/pack.png`) and zip packs
/// (works even with a trailing `.disabled` rename).
fn load_pack_icon(asset_path: &std::path::Path) -> Image {
    if asset_path.is_dir() {
        return Image::load_from_path(&asset_path.join("pack.png")).unwrap_or_default();
    }
    let Ok(file) = std::fs::File::open(asset_path) else {
        return Image::default();
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return Image::default();
    };
    let Ok(mut entry) = archive.by_name("pack.png") else {
        return Image::default();
    };
    let mut bytes = Vec::new();
    if entry.read_to_end(&mut bytes).is_err() {
        return Image::default();
    }
    image_from_png_bytes(&bytes)
}

pub fn worlds_model(instance_path: &Path, worlds: &[WorldInfo]) -> ModelRc<WorldItem> {
    let saves = instance_path.join("saves");
    let items: Vec<WorldItem> = worlds
        .iter()
        .map(|w| WorldItem {
            folder_name: w.folder_name.clone().into(),
            last_played: w.last_played.clone().into(),
            size: human_size(w.size_bytes).into(),
            icon: Image::load_from_path(&saves.join(&w.folder_name).join("icon.png"))
                .unwrap_or_default(),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn assets_model(dir: &Path, assets: &[AssetInfo]) -> ModelRc<AssetItem> {
    let items: Vec<AssetItem> = assets
        .iter()
        .map(|a| AssetItem {
            filename: a.filename.clone().into(),
            display_name: strip_disabled(&a.filename).into(),
            size: human_size(a.size_bytes).into(),
            enabled: a.enabled,
            description: strip_mc_formatting(a.description.as_deref().unwrap_or_default()).into(),
            desc_lines: parse_formatted(a.description.as_deref().unwrap_or_default()),
            icon: load_pack_icon(&dir.join(&a.filename)),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn screenshots_model(instance_path: &Path, shots: &[ScreenshotInfo]) -> ModelRc<ScreenshotItem> {
    let dir = instance_path.join("screenshots");
    let items: Vec<ScreenshotItem> = shots
        .iter()
        .map(|s| ScreenshotItem {
            filename: s.filename.clone().into(),
            created: s.created.clone().into(),
            size: human_size(s.size_bytes).into(),
            image: Image::load_from_path(&dir.join(&s.filename)).unwrap_or_default(),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Parse the `backup_YYYYMMDD_HHMMSS.zip` timestamp into a friendly date, or
/// `None` if the name doesn't match.
fn parse_backup_timestamp(filename: &str) -> Option<String> {
    let stamp = filename.strip_prefix("backup_")?.strip_suffix(".zip")?;
    let dt = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d_%H%M%S").ok()?;
    Some(dt.format("%Y-%m-%d %H:%M").to_string())
}

pub fn backups_model(instance_path: &Path, backups: &[String]) -> ModelRc<BackupItem> {
    let dir = instance_path.join("backups");
    let items: Vec<BackupItem> = backups
        .iter()
        .map(|name| {
            let size = std::fs::metadata(dir.join(name)).map(|m| m.len()).unwrap_or(0);
            BackupItem {
                filename: name.clone().into(),
                created: parse_backup_timestamp(name).unwrap_or_else(|| name.clone()).into(),
                size: human_size(size).into(),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn mods_model(mods: &[InstanceMod]) -> ModelRc<ModItem> {
    let items: Vec<ModItem> = mods
        .iter()
        .map(|m| ModItem {
            filename: m.filename.clone().into(),
            name: m.metadata.name.clone().into(),
            version: m.metadata.version.clone().into(),
            description: strip_mc_formatting(m.metadata.description.as_deref().unwrap_or_default()).into(),
            desc_lines: parse_formatted(m.metadata.description.as_deref().unwrap_or_default()),
            enabled: m.enabled,
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

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn test_human_size() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1024 * 1024 * 3), "3.0 MB");
    }

    #[test]
    fn test_strip_mc_formatting() {
        assert_eq!(strip_mc_formatting("§6Gold §lBold§r plain"), "Gold Bold plain");
        assert_eq!(strip_mc_formatting("no codes"), "no codes");
    }

    #[test]
    fn test_parse_formatted_colors_and_lines() {
        let model = parse_formatted("§6Gold§r white\nsecond §lbold");
        assert_eq!(model.row_count(), 2);

        // Line 1: "Gold" (gold) + " white" (default after reset).
        let line0 = model.row_data(0).unwrap();
        assert_eq!(line0.spans.row_count(), 2);
        let gold = line0.spans.row_data(0).unwrap();
        assert_eq!(gold.text.as_str(), "Gold");
        assert_eq!(gold.color, Color::from_rgb_u8(0xff, 0xaa, 0x00));
        assert!(!gold.bold);
        let white = line0.spans.row_data(1).unwrap();
        assert_eq!(white.text.as_str(), " white");

        // Line 2: "second " (plain) + "bold" (bold).
        let line1 = model.row_data(1).unwrap();
        assert_eq!(line1.spans.row_count(), 2);
        assert!(line1.spans.row_data(1).unwrap().bold);
    }

    #[test]
    fn test_parse_formatted_plain() {
        let model = parse_formatted("just text");
        assert_eq!(model.row_count(), 1);
        let line = model.row_data(0).unwrap();
        assert_eq!(line.spans.row_count(), 1);
        assert_eq!(line.spans.row_data(0).unwrap().text.as_str(), "just text");
    }
}
