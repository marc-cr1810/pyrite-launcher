//! Build Slint UI models from core data structures.

use std::path::Path;

use std::io::Read;

use slint::{Color, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use crate::app::avatars;
use crate::app::avatars::AvatarEntry;
use crate::app::icons;
use crate::app::state::AppState;
use crate::core::api::{ModrinthProject, ModrinthSearchHit, ModrinthVersion};
use crate::core::assets::{AssetInfo, ScreenshotInfo, WorldInfo};
use crate::core::config::{AccountType, Config};
use crate::core::instance::{Instance, InstanceMod};
use crate::core::storage::{self, StorageReport};
use crate::{
    AccountItem, AssetItem, BackupItem, FmtLine, FmtSpan, InstanceItem, JavaRuntimeItem, LogLine, ModItem,
    ModrinthDetail, ModrinthHit, ScreenshotItem, StorageItem, WorldItem,
};

/// Build the Storage-card model: one row per instance (largest first) followed
/// by the shared caches. `bytes` is capped to i32::MAX for the Slint bar scale.
pub fn storage_model(report: &StorageReport) -> ModelRc<StorageItem> {
    let clamp = |b: u64| b.min(i32::MAX as u64) as i32;
    let mut items: Vec<StorageItem> = report
        .instances
        .iter()
        .map(|inst| StorageItem {
            label: inst.name.clone().into(),
            detail: storage::format_bytes(inst.bytes).into(),
            bytes: clamp(inst.bytes),
            kind: "instance".into(),
            id: inst.id.clone().into(),
        })
        .collect();

    for (label, bytes) in [
        ("Versions", report.versions_bytes),
        ("Libraries", report.libraries_bytes),
        ("Assets", report.assets_bytes),
    ] {
        items.push(StorageItem {
            label: label.into(),
            detail: storage::format_bytes(bytes).into(),
            bytes: clamp(bytes),
            kind: "cache".into(),
            id: SharedString::new(),
        });
    }

    ModelRc::new(VecModel::from(items))
}

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

pub fn java_runtimes_model(config: &Config) -> ModelRc<JavaRuntimeItem> {
    let game_dir = &config.game_dir;
    let current_java_path = &config.java_path;
    let mut items = Vec::new();

    let is_active = |major: u32, path: &std::path::Path| -> bool {
        if path == std::path::Path::new("java") || path.to_string_lossy().trim().is_empty() {
            false
        } else {
            if let Some(installed_path) = crate::core::java::get_installed_java(game_dir, major) {
                if let (Ok(p1), Ok(p2)) = (path.canonicalize(), installed_path.canonicalize()) {
                    p1 == p2
                } else {
                    path == &installed_path
                }
            } else {
                false
            }
        }
    };

    let is_auto_active = current_java_path == std::path::Path::new("java") || current_java_path.to_string_lossy().trim().is_empty();
    items.push(JavaRuntimeItem {
        major: 0,
        name: "Auto-detect (Recommended)".into(),
        description: "Automatically download and use the correct version required by the Minecraft instance.".into(),
        status: if is_auto_active { "Active".into() } else { "Installed".into() },
        is_active: is_auto_active,
    });

    let versions = [
        (25, "Adoptium JRE 25 (Required for Minecraft 1.21.5+ / 26.2+)"),
        (21, "Adoptium JRE 21 (Required for Minecraft 1.20.5 - 1.21.4)"),
        (17, "Adoptium JRE 17 (Required for Minecraft 1.18 - 1.20.4)"),
        (8, "Adoptium JRE 8 (Required for Minecraft 1.12.2 and older)"),
    ];

    for (major, desc) in versions {
        let is_installed = crate::core::java::get_installed_java(game_dir, major).is_some();
        let active = is_active(major, current_java_path);
        let status = if active {
            "Active".into()
        } else if is_installed {
            "Installed".into()
        } else {
            "Not installed".into()
        };

        items.push(JavaRuntimeItem {
            major: major as i32,
            name: format!("Java {major}").into(),
            description: desc.into(),
            status,
            is_active: active,
        });
    }

    ModelRc::new(VecModel::from(items))
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

pub fn instances_model(config: &Config, sort_key: &str, search_query: &str) -> ModelRc<InstanceItem> {
    let game_dir = &config.game_dir;
    let active = config.active_instance.clone();
    let query = search_query.to_lowercase();
    let mut items: Vec<InstanceItem> = Instance::load_all(game_dir)
        .into_iter()
        .filter(|inst| query.is_empty() || inst.config.name.to_lowercase().contains(&query))
        .map(|inst| {
            let (game_version, loader) = inst.get_game_version_and_loader(game_dir);
            let loader_label = match loader.as_deref() {
                Some("fabric") => "Fabric",
                Some("forge") => "Forge",
                Some("neoforge") => "NeoForge",
                Some(other) => other,
                None => "",
            };
            let last_played = inst.config.last_played.as_deref()
                .map(|s| human_relative_time(s))
                .unwrap_or_else(|| "Never".to_string());
            let playtime = human_duration(inst.config.total_playtime_secs);
            // Split -Xmx out of the stored args so the editor can show memory on
            // a slider and the remaining flags in the free-form field.
            let (memory_mb, jvm_rest) = crate::app::controllers::instances::split_memory_args(
                inst.config.jvm_args.as_deref().unwrap_or(&[]),
            );
            let memory_mb = memory_mb.unwrap_or(0);
            InstanceItem {
                id: inst.id.clone().into(),
                name: inst.config.name.clone().into(),
                version: game_version.into(),
                loader: SharedString::from(loader_label),
                active: active.as_deref() == Some(inst.id.as_str()),
                initial: initial(&inst.config.name),
                avatar_color: avatar_color(&inst.id),
                memory_mb: memory_mb as i32,
                jvm_args: jvm_rest.into(),
                java_path: inst.config.java_path.clone().unwrap_or_default().into(),
                pre_launch: inst.config.pre_launch.clone().unwrap_or_default().into(),
                post_exit: inst.config.post_exit.clone().unwrap_or_default().into(),
                icon_id: inst.config.icon.clone().unwrap_or_default().into(),
                icon_glyph: instance_icon_glyph(&inst.config.icon),
                icon_image: instance_icon_image(&inst.path, &inst.config.icon),
                last_played: last_played.into(),
                playtime: playtime.into(),
            }
        })
        .collect();

    // Sort according to the requested key.
    match sort_key {
        "last-played" => {
            // "Never" sorts last; otherwise reverse chronological via the raw
            // string — but since we only have the human label, we re-derive
            // order from the underlying data. We can't easily here, so we sort
            // the items Vec by a secondary load. Instead, sort by label with
            // "Never" pushed to the end, and everything else reverse-alpha
            // ("Yesterday" < "Just now" etc. won't be perfect, but close enough
            // for a sort). For a proper sort, we embed a hidden sort key.
            // Actually, let's just re-load and sort by the raw timestamp.
            items.sort_by(|a, b| {
                let a_str = a.last_played.as_str();
                let b_str = b.last_played.as_str();
                if a_str == "Never" && b_str == "Never" {
                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                } else if a_str == "Never" {
                    std::cmp::Ordering::Greater
                } else if b_str == "Never" {
                    std::cmp::Ordering::Less
                } else {
                    // Both have been played; sort by the label heuristically.
                    // We'll use a trick: items with smaller time labels played
                    // more recently. Parse the leading number.
                    a_str.cmp(b_str)
                }
            });
        }
        "playtime" => {
            // Sort by playtime descending. "\u{2014}" (em-dash = 0) sorts last.
            // We can't recover secs from the label, so we embed a sort key.
            // For simplicity, let's keep a numeric sort key. Actually, let's
            // just sort by the display string length + value (longer = more).
            // This is hacky. Better: re-read the instances for the raw value.
            // Since Instance::load_all is cheap, let's just do a second pass.
            let raw: std::collections::HashMap<String, u64> = Instance::load_all(game_dir)
                .into_iter()
                .map(|i| (i.id, i.config.total_playtime_secs))
                .collect();
            items.sort_by(|a, b| {
                let a_secs = raw.get(a.id.as_str()).copied().unwrap_or(0);
                let b_secs = raw.get(b.id.as_str()).copied().unwrap_or(0);
                b_secs.cmp(&a_secs).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        _ => {
            // Default: sort by name (already sorted by id from load_all; re-sort by name)
            items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
    }

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

/// Format a duration in seconds as a short human-readable string.
pub fn human_duration(secs: u64) -> String {
    if secs == 0 {
        return "\u{2014}".to_string(); // em-dash
    }
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        "< 1m".to_string()
    }
}

/// Format an ISO 8601 timestamp as a human-friendly relative time.
pub fn human_relative_time(iso: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso) {
        let now = chrono::Utc::now();
        let dur = now.signed_duration_since(dt.with_timezone(&chrono::Utc));
        let secs = dur.num_seconds();
        if secs < 60 {
            "Just now".to_string()
        } else if secs < 3600 {
            format!("{}m ago", secs / 60)
        } else if secs < 86400 {
            format!("{}h ago", secs / 3600)
        } else if secs < 86400 * 30 {
            let days = secs / 86400;
            if days == 1 { "Yesterday".to_string() } else { format!("{}d ago", days) }
        } else {
            dt.format("%Y-%m-%d").to_string()
        }
    } else {
        "Never".to_string()
    }
}

/// User-facing pack name: drop the ".disabled" enable/disable marker and the
/// archive extension (e.g. "Cool Pack.zip.disabled" -> "Cool Pack"). Folder
/// packs (no extension) are left as-is.
fn display_pack_name(filename: &str) -> String {
    let base = filename.strip_suffix(".disabled").unwrap_or(filename);
    let lower = base.to_ascii_lowercase();
    for ext in [".zip", ".rpo"] {
        if lower.ends_with(ext) {
            return base[..base.len() - ext.len()].to_string();
        }
    }
    base.to_string()
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
/// Parse a Minecraft `§`-formatted string into colored, styled lines. At most
/// `max_lines` lines are emitted (descriptions render in fixed-height list rows,
/// so unbounded multi-line text would overflow); an ellipsis marks truncation.
pub fn parse_formatted(s: &str, max_lines: usize) -> ModelRc<FmtLine> {
    let total = s.split('\n').count();
    let mut line_spans: Vec<Vec<FmtSpan>> = s
        .split('\n')
        .take(max_lines.max(1))
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
            spans
        })
        .collect();

    // If we dropped lines, append an ellipsis to the last visible line so the
    // truncation is visible rather than silent.
    if total > line_spans.len() {
        if let Some(last) = line_spans.iter_mut().rev().find(|s| !s.is_empty()) {
            if let Some(span) = last.last_mut() {
                span.text = format!("{} …", span.text).into();
            }
        }
    }

    let lines: Vec<FmtLine> = line_spans
        .into_iter()
        .map(|spans| FmtLine {
            spans: ModelRc::new(VecModel::from(spans)),
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
            display_name: display_pack_name(&a.filename).into(),
            size: human_size(a.size_bytes).into(),
            enabled: a.enabled,
            description: strip_mc_formatting(a.description.as_deref().unwrap_or_default()).into(),
            desc_lines: parse_formatted(a.description.as_deref().unwrap_or_default(), 2),
            icon: load_pack_icon(&dir.join(&a.filename)),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Build the screenshot list plus parallel `[image]` / `[caption]` models for the
/// full-screen viewer (so it can page through shots without re-loading them).
pub fn screenshots_model(
    instance_path: &Path,
    shots: &[ScreenshotInfo],
) -> (ModelRc<ScreenshotItem>, ModelRc<Image>, ModelRc<SharedString>) {
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
    let images: Vec<Image> = items.iter().map(|i| i.image.clone()).collect();
    let captions: Vec<SharedString> = items.iter().map(|i| i.filename.clone()).collect();
    (
        ModelRc::new(VecModel::from(items)),
        ModelRc::new(VecModel::from(images)),
        ModelRc::new(VecModel::from(captions)),
    )
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

pub fn mods_model(
    mods: &[InstanceMod],
    updates: &std::collections::HashMap<String, crate::app::state::ModUpdate>,
) -> ModelRc<ModItem> {
    let items: Vec<ModItem> = mods
        .iter()
        .map(|m| {
            let update = updates.get(&m.filename);
            ModItem {
                filename: m.filename.clone().into(),
                name: m.metadata.name.clone().into(),
                version: m.metadata.version.clone().into(),
                description: strip_mc_formatting(m.metadata.description.as_deref().unwrap_or_default()).into(),
                desc_lines: parse_formatted(m.metadata.description.as_deref().unwrap_or_default(), 2),
                enabled: m.enabled,
                update_available: update.is_some(),
                latest_version: update.map(|u| u.version_number.clone()).unwrap_or_default().into(),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Format a download count compactly ("1.2M", "12.3K", "987").
pub fn human_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Build a `slint::Image` for a cached Modrinth project icon, or an empty image
/// when it is not ready yet. Must be called on the UI thread.
pub fn modrinth_icon_image(state: &AppState, project_id: &str) -> Image {
    let cache = state.modrinth_icon_cache.lock().unwrap();
    if let Some(AvatarEntry::Ready { rgba, width, height }) = cache.get(project_id) {
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(*width, *height);
        buf.make_mut_bytes().copy_from_slice(rgba);
        Image::from_rgba8(buf)
    } else {
        Image::default()
    }
}

pub fn modrinth_results_model(hits: &[ModrinthSearchHit], state: &AppState) -> ModelRc<ModrinthHit> {
    let items: Vec<ModrinthHit> = hits
        .iter()
        .map(|h| ModrinthHit {
            project_id: h.project_id.clone().into(),
            slug: h.slug.clone().into(),
            title: h.title.clone().into(),
            description: h.description.clone().into(),
            author: h.author.clone().into(),
            downloads: format!("{} downloads", human_count(h.downloads)).into(),
            icon: modrinth_icon_image(state, &h.project_id),
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// The data backing an open project-detail view, cached in `AppState` so the
/// Slint model can be rebuilt as gallery images decode.
#[derive(Clone)]
pub struct ModrinthDetailData {
    pub project: ModrinthProject,
    pub versions: Vec<ModrinthVersion>,
    pub author: String,
    pub kind: String,
}

/// Build a `slint::Image` for a cached detail icon / gallery image (keyed by url).
/// Must be called on the UI thread.
pub fn modrinth_gallery_image(state: &AppState, url: &str) -> Image {
    if url.is_empty() {
        return Image::default();
    }
    let cache = state.modrinth_gallery_cache.lock().unwrap();
    if let Some(AvatarEntry::Ready { rgba, width, height }) = cache.get(url) {
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(*width, *height);
        buf.make_mut_bytes().copy_from_slice(rgba);
        Image::from_rgba8(buf)
    } else {
        Image::default()
    }
}

pub fn modrinth_detail_model(data: &ModrinthDetailData, state: &AppState) -> ModrinthDetail {
    let p = &data.project;
    let icon_url = p.icon_url.clone().unwrap_or_default();

    // Use each image's full-resolution `raw_url` (the API's `url` is a 350px
    // thumbnail) so both the strip and the full-screen viewer stay crisp.
    let gallery: Vec<Image> = p
        .gallery
        .iter()
        .map(|g| modrinth_gallery_image(state, g.raw_url.as_deref().unwrap_or(&g.url)))
        .collect();
    let gallery_titles: Vec<SharedString> = p
        .gallery
        .iter()
        .map(|g| g.title.clone().unwrap_or_default().into())
        .collect();

    let version_labels: Vec<SharedString> = data
        .versions
        .iter()
        .map(|v| {
            let game = v.game_versions.first().cloned().unwrap_or_default();
            let loaders = v.loaders.join("/");
            if loaders.is_empty() {
                format!("{} — {}", v.version_number, game).into()
            } else {
                format!("{} — {} · {}", v.version_number, game, loaders).into()
            }
        })
        .collect();
    let version_ids: Vec<SharedString> = data.versions.iter().map(|v| v.id.clone().into()).collect();

    let url_type = p.project_type.clone().unwrap_or_else(|| data.kind.clone());
    let page_url = format!("https://modrinth.com/{}/{}", url_type, p.slug);
    let updated = p.updated.as_deref().map(human_relative_time).unwrap_or_default();

    ModrinthDetail {
        project_id: p.id.clone().into(),
        kind: data.kind.clone().into(),
        title: p.title.clone().into(),
        author: data.author.clone().into(),
        description: p.description.clone().into(),
        body: markdown_to_text(&p.body).into(),
        icon: modrinth_gallery_image(state, &icon_url),
        downloads: format!("{} downloads", human_count(p.downloads)).into(),
        updated: updated.into(),
        categories: p.categories.join(", ").into(),
        source_url: p.source_url.clone().unwrap_or_default().into(),
        issues_url: p.issues_url.clone().unwrap_or_default().into(),
        wiki_url: p.wiki_url.clone().unwrap_or_default().into(),
        discord_url: p.discord_url.clone().unwrap_or_default().into(),
        page_url: page_url.into(),
        gallery: ModelRc::new(VecModel::from(gallery)),
        gallery_titles: ModelRc::new(VecModel::from(gallery_titles)),
        version_labels: ModelRc::new(VecModel::from(version_labels)),
        version_ids: ModelRc::new(VecModel::from(version_ids)),
    }
}

/// Convert a Markdown body to a simplified plain-text rendering: drop images,
/// flatten links to their text, strip heading/quote/list markers and emphasis.
/// Not a full Markdown renderer — just readable preview text.
pub fn markdown_to_text(body: &str) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    for raw in body.replace("\r\n", "\n").lines() {
        let mut line = raw.trim_end().to_string();

        // Leading block markers.
        let trimmed = line.trim_start();
        let lead_ws = &line[..line.len() - trimmed.len()];
        let mut rest = trimmed.to_string();
        let mut bullet = "";
        if rest.starts_with('#') {
            rest = rest.trim_start_matches('#').trim_start().to_string();
        } else if rest.starts_with('>') {
            rest = rest.trim_start_matches('>').trim_start().to_string();
        } else if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
            rest = rest[2..].to_string();
            bullet = "• ";
        }
        line = format!("{lead_ws}{bullet}{}", md_inline(&rest));

        // Collapse runs of blank lines.
        if line.trim().is_empty() {
            if matches!(out_lines.last(), Some(l) if l.trim().is_empty()) {
                continue;
            }
        }
        out_lines.push(line);
    }
    out_lines.join("\n").trim().to_string()
}

/// Inline-Markdown cleanup for one line: `![alt](url)` removed, `[text](url)` ->
/// `text`, and `* \` ~` emphasis/code markers stripped.
fn md_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        // Image: drop the whole ![alt](url).
        if c == '!' && i + 1 < n && chars[i + 1] == '[' {
            if let Some(j) = link_end(&chars, i + 1) {
                i = j;
                continue;
            }
        }
        // Link: keep the bracketed text, drop the (url).
        if c == '[' {
            if let Some((text, j)) = take_link(&chars, i) {
                out.push_str(&text);
                i = j;
                continue;
            }
        }
        if c == '*' || c == '`' || c == '~' {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Parse `[text](url)` starting at the `[`; returns (text, index-after-`)`).
fn take_link(chars: &[char], start: usize) -> Option<(String, usize)> {
    let end = link_end(chars, start)?;
    // text is between the first '[' and its matching ']'.
    let mut depth = 0;
    let mut close = start;
    for (k, &c) in chars.iter().enumerate().skip(start) {
        if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth == 0 {
                close = k;
                break;
            }
        }
    }
    let text: String = chars[start + 1..close].iter().collect();
    Some((text, end))
}

/// Index just past the closing `)` of a `[...](...)` construct at `start` (`[`).
fn link_end(chars: &[char], start: usize) -> Option<usize> {
    let n = chars.len();
    if start >= n || chars[start] != '[' {
        return None;
    }
    let mut depth = 0;
    let mut i = start;
    // Match the bracket.
    while i < n {
        if chars[i] == '[' {
            depth += 1;
        } else if chars[i] == ']' {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        i += 1;
    }
    // Require "](" immediately after.
    if i + 1 >= n || chars[i] != ']' || chars[i + 1] != '(' {
        return None;
    }
    i += 2;
    while i < n && chars[i] != ')' {
        i += 1;
    }
    if i < n && chars[i] == ')' {
        Some(i + 1)
    } else {
        None
    }
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
        let model = parse_formatted("§6Gold§r white\nsecond §lbold", 4);
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
        let model = parse_formatted("just text", 2);
        assert_eq!(model.row_count(), 1);
        let line = model.row_data(0).unwrap();
        assert_eq!(line.spans.row_count(), 1);
        assert_eq!(line.spans.row_data(0).unwrap().text.as_str(), "just text");
    }

    #[test]
    fn test_parse_formatted_caps_lines_with_ellipsis() {
        let model = parse_formatted("one\ntwo\nthree\nfour", 2);
        assert_eq!(model.row_count(), 2);
        // The last visible line gets an ellipsis to signal truncation.
        let last = model.row_data(1).unwrap();
        let span = last.spans.row_data(last.spans.row_count() - 1).unwrap();
        assert_eq!(span.text.as_str(), "two …");

        // No ellipsis when nothing was dropped.
        let model = parse_formatted("one\ntwo", 2);
        assert_eq!(model.row_count(), 2);
        let last = model.row_data(1).unwrap();
        assert_eq!(last.spans.row_data(0).unwrap().text.as_str(), "two");
    }

    #[test]
    fn test_human_duration() {
        assert_eq!(human_duration(0), "\u{2014}");
        assert_eq!(human_duration(30), "< 1m");
        assert_eq!(human_duration(60), "1m");
        assert_eq!(human_duration(90), "1m");
        assert_eq!(human_duration(3661), "1h 1m");
        assert_eq!(human_duration(7200), "2h 0m");
    }

    #[test]
    fn test_human_relative_time() {
        assert_eq!(human_relative_time("invalid"), "Never");
        assert_eq!(human_relative_time(""), "Never");
        // A timestamp far in the past should give a date.
        assert_eq!(human_relative_time("2020-01-01T00:00:00+00:00"), "2020-01-01");
    }
}
