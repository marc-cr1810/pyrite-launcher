//! Built-in instance icons. Each is a Phosphor icon-font glyph rendered inside
//! the monogram Avatar (the font is bundled in assets/fonts; no per-icon binary
//! assets). The same list is pushed to the UI for the picker grid, keeping a
//! single source of truth. The stable `id` is what gets stored in instance.toml,
//! so the glyph codepoints can change freely without breaking saved instances.
pub const BUILTIN: &[(&str, &str)] = &[
    ("diamond", "\u{e1ec}"),  // ph-diamond
    ("star", "\u{e46a}"),     // ph-star
    ("gear", "\u{e272}"),     // ph-gear-six
    ("block", "\u{e1da}"),    // ph-cube
    ("triangle", "\u{e4b0}"), // ph-triangle
    ("circle", "\u{e18a}"),   // ph-circle
    ("sparkle", "\u{e6a2}"),  // ph-sparkle
    ("hexagon", "\u{e2ae}"),  // ph-hexagon
    ("snow", "\u{e5aa}"),     // ph-snowflake
    ("sun", "\u{e472}"),      // ph-sun
    ("bolt", "\u{e2de}"),     // ph-lightning
    ("heart", "\u{e2a8}"),    // ph-heart
];

/// Glyph for a built-in icon id, or `None` if the id is unknown (e.g. "custom").
pub fn glyph_for(id: &str) -> Option<&'static str> {
    BUILTIN
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, glyph)| *glyph)
}
