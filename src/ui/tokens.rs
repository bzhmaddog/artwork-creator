//! The interface's design tokens: the colors, sizes, radii, spacing and strokes that
//! widgets share, so they look alike. Colors that follow the theme come from egui's
//! `Visuals` (see [`theme_colors`]); the fixed ones are here. `docs/tokens.png` shows
//! them all (`cargo test ui_tokens -- --ignored` draws it again); `docs/widgets.png`,
//! the widgets made with them.

use egui::{Color32, Visuals};

// Colors that stay the same in both themes.

/// Fill of a button that deletes or discards something, under white text.
pub const DANGER: Color32 = Color32::from_rgb(200, 40, 40);
/// Selection and handles on the canvas, and the export sheets' outlines.
pub const ACCENT: Color32 = Color32::from_rgb(220, 20, 60);
/// Guides on the canvas.
pub const GUIDE: Color32 = Color32::from_rgb(0, 180, 216);
/// The safe area's outline on the canvas.
pub const SAFE_AREA: Color32 = Color32::from_rgb(130, 80, 220);
/// Tip callouts, on a dark and a light background (see [`tip`]).
pub const TIP_DARK: Color32 = Color32::from_rgb(110, 200, 140);
pub const TIP_LIGHT: Color32 = Color32::from_rgb(30, 140, 80);

/// The tip color for the theme of `visuals`.
pub fn tip(visuals: &Visuals) -> Color32 {
    if visuals.dark_mode {
        TIP_DARK
    } else {
        TIP_LIGHT
    }
}

/// The theme's colors the widgets use, by name, as `docs/tokens.png` shows them.
#[cfg(test)]
pub fn theme_colors(v: &Visuals) -> [(&'static str, Color32); 10] {
    [
        ("text", v.text_color()),
        ("weak text", v.weak_text_color()),
        ("panel", v.panel_fill),
        ("field", v.extreme_bg_color),
        ("widget", v.widgets.inactive.weak_bg_fill),
        ("hover", v.widgets.hovered.weak_bg_fill),
        ("selection", v.selection.bg_fill),
        ("info / link", v.hyperlink_color),
        ("warning", v.warn_fg_color),
        ("error", v.error_fg_color),
    ]
}

// Sizes, in points.

/// Side of a toolbar icon button (layer toolbar, shortcuts window).
pub const ICON_TOOL: f32 = 24.0;
/// Side of an icon button in a panel's title bar.
pub const ICON_PANEL: f32 = 20.0;
/// Side of an icon in a list row (eye, lock) and of a color swatch.
pub const ICON_ROW: f32 = 18.0;
/// Side of an angle dial.
pub const DIAL: f32 = 22.0;
/// Side of a layer thumbnail.
pub const THUMB: f32 = 28.0;

// Corner radii, in points.

/// Buttons, fields and hover fills of icon buttons.
pub const RADIUS: f32 = 3.0;
/// Callouts.
pub const RADIUS_CALLOUT: u8 = 6;

// Spacing, in points.

/// Room around a dialog's or window's contents (egui's is 6).
pub const DIALOG_MARGIN: i8 = 14;
/// Between the label and control columns of a settings grid.
pub const COLUMN_GAP: f32 = 8.0;
/// Between the rows of the Properties grids.
pub const PROP_ROW_GAP: f32 = 6.0;
/// Between the rows of the Page, Template and Print panels' grids.
pub const PANEL_ROW_GAP: f32 = 4.0;
/// Indent of a folding section's contents inside another, and of a fill's settings.
pub const INDENT: f32 = 8.0;
/// Indent of a layer inside a group, in the layer list.
pub const LAYER_INDENT: f32 = 14.0;

// Strokes, in points.

/// Lines of the drawn icons, and the outline that highlights something.
pub const STROKE: f32 = 1.5;
