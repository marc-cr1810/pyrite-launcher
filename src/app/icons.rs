//! Built-in instance icons. Each is a glyph rendered inside the monogram
//! Avatar (so no binary assets are bundled). The same list is pushed to the UI
//! for the picker grid, keeping a single source of truth.

/// (id stored in instance.toml, glyph shown in the Avatar).
pub const BUILTIN: &[(&str, &str)] = &[
    ("diamond", "◆"),
    ("star", "★"),
    ("gear", "⚙"),
    ("block", "■"),
    ("triangle", "▲"),
    ("circle", "●"),
    ("sparkle", "✦"),
    ("hexagon", "⬡"),
    ("snow", "❄"),
    ("sun", "☀"),
    ("bolt", "⚡"),
    ("heart", "♥"),
];

/// Glyph for a built-in icon id, or `None` if the id is unknown (e.g. "custom").
pub fn glyph_for(id: &str) -> Option<&'static str> {
    BUILTIN
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, glyph)| *glyph)
}
