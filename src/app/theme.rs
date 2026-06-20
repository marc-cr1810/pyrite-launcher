//! JSON-based theming. The built-in "Default Dark" palette is embedded so the
//! app is fully usable with zero theme files on disk; additional palettes are
//! discovered from `<config_dir>/pyrite-launcher/themes/*.json` at startup.
use slint::ComponentHandle;

use serde::Deserialize;
use slint::Color;
use std::path::PathBuf;

use crate::Theme as UiTheme;
use crate::MainWindow;

const DEFAULT_JSON: &str = include_str!("../../themes/default.json");
const CATPPUCCIN_MOCHA_JSON: &str = include_str!("../../themes/catppuccin_mocha.json");
const DRACULA_JSON: &str = include_str!("../../themes/dracula.json");
const NORD_JSON: &str = include_str!("../../themes/nord.json");
const GRUVBOX_DARK_JSON: &str = include_str!("../../themes/gruvbox_dark.json");
const LIGHT_JSON: &str = include_str!("../../themes/light.json");

/// A palette. All colors optional so a partial user theme inherits the rest
/// from the default.
#[derive(Deserialize, Clone, Default)]
pub struct ThemePalette {
    pub name: Option<String>,
    pub window_bg: Option<String>,
    pub sidebar_bg: Option<String>,
    pub surface: Option<String>,
    pub surface_alt: Option<String>,
    pub surface_hover: Option<String>,
    pub text: Option<String>,
    pub text_muted: Option<String>,
    pub text_dim: Option<String>,
    pub accent: Option<String>,
    pub accent_hover: Option<String>,
    pub accent_text: Option<String>,
    pub border: Option<String>,
    pub success: Option<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

/// Discovered themes; index 0 is always the built-in default.
pub struct ThemeStore {
    pub themes: Vec<(String, ThemePalette)>,
}

impl ThemeStore {
    pub fn themes_dir() -> Option<PathBuf> {
        crate::core::config::Config::config_path()
            .and_then(|p| p.parent().map(|d| d.join("themes")))
    }

    /// Parse a bundled JSON string into a named palette entry.
    fn parse_builtin(json: &str, fallback_name: &str) -> (String, ThemePalette) {
        let palette: ThemePalette = serde_json::from_str(json)
            .unwrap_or_else(|_| panic!("embedded theme '{fallback_name}' is valid JSON"));
        let name = palette.name.clone().unwrap_or_else(|| fallback_name.to_string());
        (name, palette)
    }

    pub fn load() -> Self {
        // Embedded themes — always available.
        let mut themes = vec![
            Self::parse_builtin(DEFAULT_JSON, "Default Dark"),
            Self::parse_builtin(CATPPUCCIN_MOCHA_JSON, "Catppuccin Mocha"),
            Self::parse_builtin(DRACULA_JSON, "Dracula"),
            Self::parse_builtin(NORD_JSON, "Nord"),
            Self::parse_builtin(GRUVBOX_DARK_JSON, "Gruvbox Dark"),
            Self::parse_builtin(LIGHT_JSON, "Light"),
        ];
        let builtin_names: Vec<String> = themes.iter().map(|(n, _)| n.clone()).collect();

        // User themes from the config dir (additive, won't override builtins).
        if let Some(dir) = Self::themes_dir()
            && let Ok(entries) = std::fs::read_dir(&dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path)
                    && let Ok(palette) = serde_json::from_str::<ThemePalette>(&content)
                {
                    let name = palette.name.clone().unwrap_or_else(|| {
                        path.file_stem().unwrap_or_default().to_string_lossy().to_string()
                    });
                    if !builtin_names.contains(&name) {
                        themes.push((name, palette));
                    }
                }
            }
        }
        Self { themes }
    }

    pub fn names(&self) -> Vec<String> {
        self.themes.iter().map(|(n, _)| n.clone()).collect()
    }

    fn default_palette(&self) -> &ThemePalette {
        &self.themes[0].1
    }

    /// Apply the named theme (falling back to default) to the UI's Theme global.
    pub fn apply(&self, window: &MainWindow, name: &str) {
        let default = self.default_palette().clone();
        let palette = self
            .themes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| default.clone());

        let theme = window.global::<UiTheme>();
        let pick = |val: &Option<String>, fallback: &Option<String>| -> Color {
            parse_hex(val.as_deref().or(fallback.as_deref()).unwrap_or("#000000"))
        };

        theme.set_window_bg(pick(&palette.window_bg, &default.window_bg));
        theme.set_sidebar_bg(pick(&palette.sidebar_bg, &default.sidebar_bg));
        theme.set_surface(pick(&palette.surface, &default.surface));
        theme.set_surface_alt(pick(&palette.surface_alt, &default.surface_alt));
        theme.set_surface_hover(pick(&palette.surface_hover, &default.surface_hover));
        theme.set_text(pick(&palette.text, &default.text));
        theme.set_text_muted(pick(&palette.text_muted, &default.text_muted));
        theme.set_text_dim(pick(&palette.text_dim, &default.text_dim));
        theme.set_accent(pick(&palette.accent, &default.accent));
        theme.set_accent_hover(pick(&palette.accent_hover, &default.accent_hover));
        theme.set_accent_text(pick(&palette.accent_text, &default.accent_text));
        theme.set_border(pick(&palette.border, &default.border));
        theme.set_success(pick(&palette.success, &default.success));
        theme.set_warning(pick(&palette.warning, &default.warning));
        theme.set_error(pick(&palette.error, &default.error));
    }
}

/// Parse "#rrggbb" or "#aarrggbb" into a Slint Color. Invalid input → opaque black.
fn parse_hex(s: &str) -> Color {
    let h = s.trim().trim_start_matches('#');
    let parse = |slice: &str| u8::from_str_radix(slice, 16).unwrap_or(0);
    match h.len() {
        6 => Color::from_rgb_u8(parse(&h[0..2]), parse(&h[2..4]), parse(&h[4..6])),
        8 => Color::from_argb_u8(
            parse(&h[0..2]),
            parse(&h[2..4]),
            parse(&h[4..6]),
            parse(&h[6..8]),
        ),
        _ => Color::from_rgb_u8(0, 0, 0),
    }
}
