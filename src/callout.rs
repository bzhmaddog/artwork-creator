//! Callouts: the app's helper texts (explanations, empty states, tips, warnings) in a
//! rounded box tinted with their tone's color, with an accent bar along the left edge
//! and a drawn icon, so they stand apart from the settings around them. Data shown
//! for information (sizes, counts, paths) stays plain weak text.

use egui::{
    Color32, CornerRadius, Frame, Label, Margin, Painter, Rect, Response, RichText, Sense, Shape,
    Stroke, Ui, Visuals, pos2, vec2,
};

/// What a callout says, which sets its color and icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// An explanation or an empty state: blue, an "i" in a circle.
    Info,
    /// How to do something: green, a light bulb.
    Tip,
    /// Something to look out for: amber, a triangle with "!".
    Warning,
}

impl Tone {
    /// The tone's color, for the accent bar, the icon, the tint and the border.
    pub fn color(self, visuals: &Visuals) -> Color32 {
        match self {
            Tone::Info => visuals.hyperlink_color,
            Tone::Tip if visuals.dark_mode => Color32::from_rgb(110, 200, 140),
            Tone::Tip => Color32::from_rgb(30, 140, 80),
            Tone::Warning => visuals.warn_fg_color,
        }
    }
}

/// Corner radius of a callout, in points.
const RADIUS: u8 = 6;
/// Width of the accent bar along the left edge, in points.
const BAR: f32 = 3.0;

/// A callout as wide as the room it is given, its text wrapped.
pub fn callout(ui: &mut Ui, tone: Tone, text: &str) -> Response {
    show(ui, tone, RichText::new(text), false)
}

/// A small callout, as wide as its text, for tight places: a menu, a table cell, a bar
/// of buttons.
pub fn callout_small(ui: &mut Ui, tone: Tone, text: &str) -> Response {
    show(ui, tone, RichText::new(text).small(), true)
}

fn show(ui: &mut Ui, tone: Tone, text: RichText, compact: bool) -> Response {
    let color = tone.color(ui.visuals());
    let icon = if compact { 13.0 } else { 16.0 };
    let pad: i8 = if compact { 4 } else { 7 };
    let frame = Frame::new()
        .fill(color.gamma_multiply(if ui.visuals().dark_mode { 0.12 } else { 0.08 }))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(RADIUS)
        .inner_margin(Margin {
            left: BAR as i8 + pad + 2,
            right: pad + 2,
            top: pad,
            bottom: pad,
        });
    let response = frame
        .show(ui, |ui| {
            if !compact {
                ui.set_width(ui.available_width());
            }
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = if compact { 5.0 } else { 8.0 };
                // The icon sits on the first line of the text.
                let line = ui.text_style_height(&egui::TextStyle::Body);
                let height = if compact { icon } else { line.max(icon) };
                let (rect, _) = ui.allocate_exact_size(vec2(icon, height), Sense::hover());
                let rect = Rect::from_center_size(rect.center(), vec2(icon, icon));
                paint_icon(ui.painter(), rect, tone, color, ui.visuals());
                ui.add(Label::new(text.color(ui.visuals().text_color())).wrap());
            });
        })
        .response;
    let r = response.rect;
    let bar = Rect::from_min_max(r.min, pos2(r.left() + BAR, r.bottom()));
    let round = CornerRadius {
        nw: RADIUS,
        sw: RADIUS,
        ne: 0,
        se: 0,
    };
    ui.painter().rect_filled(bar, round, color);
    response
}

/// Draws `tone`'s icon in `rect`, in `color`, its marks cut out in the panel color.
fn paint_icon(painter: &Painter, rect: Rect, tone: Tone, color: Color32, visuals: &Visuals) {
    let c = rect.center();
    let s = rect.width();
    let cut = visuals.panel_fill;
    match tone {
        Tone::Info => {
            painter.circle_filled(c, s / 2.0, color);
            painter.circle_filled(c + vec2(0.0, -s * 0.24), s * 0.08, cut);
            painter.line_segment(
                [c + vec2(0.0, -s * 0.06), c + vec2(0.0, s * 0.28)],
                Stroke::new(s * 0.14, cut),
            );
        }
        Tone::Tip => {
            // The bulb, its base and its rays.
            let bulb = c + vec2(0.0, -s * 0.1);
            painter.circle_filled(bulb, s * 0.3, color);
            let base = Rect::from_center_size(c + vec2(0.0, s * 0.3), vec2(s * 0.3, s * 0.16));
            painter.rect_filled(base, 1.0, color);
            painter.line_segment(
                [c + vec2(-s * 0.13, s * 0.45), c + vec2(s * 0.13, s * 0.45)],
                Stroke::new(s * 0.08, color),
            );
            // A glint on the glass.
            painter.circle_filled(bulb + vec2(-s * 0.1, -s * 0.08), s * 0.06, cut);
        }
        Tone::Warning => paint_warning(painter, rect, color, cut),
    }
}

/// Draws the warning triangle in `rect`, in `color`, its "!" cut out in `cut`.
fn paint_warning(painter: &Painter, rect: Rect, color: Color32, cut: Color32) {
    let c = rect.center();
    let s = rect.width();
    let top = pos2(c.x, rect.top() + s * 0.04);
    let left = pos2(rect.left(), rect.bottom() - s * 0.06);
    let right = pos2(rect.right(), rect.bottom() - s * 0.06);
    painter.add(Shape::convex_polygon(
        vec![top, right, left],
        color,
        Stroke::NONE,
    ));
    painter.line_segment(
        [c + vec2(0.0, -s * 0.12), c + vec2(0.0, s * 0.16)],
        Stroke::new(s * 0.12, cut),
    );
    painter.circle_filled(c + vec2(0.0, s * 0.3), s * 0.07, cut);
}

/// A warning triangle in `color` at the cursor, as tall as a line of text: drawn, so it
/// shows whatever the font has.
pub fn warning_icon(ui: &mut Ui, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, 16.0), Sense::hover());
    let rect = Rect::from_center_size(rect.center(), egui::Vec2::splat(13.0));
    paint_warning(ui.painter(), rect, color, ui.visuals().panel_fill);
    response
}

/// A warning triangle in `color`, as tall as a line of text, to put in a row or a
/// button: drawn, so it shows whatever the font has.
pub fn warning_mark(color: Color32) -> egui::Atom<'static> {
    egui::Atom::paint(egui::Vec2::splat(14.0), move |ui, args| {
        let rect = Rect::from_center_size(args.rect.center(), egui::Vec2::splat(13.0));
        paint_warning(ui.painter(), rect, color, ui.visuals().panel_fill);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_callout_fills_its_width_and_a_small_one_fits_its_text() {
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(300.0);
            rects.push(callout(ui, Tone::Info, "Short").rect);
            rects.push(callout_small(ui, Tone::Tip, "Short").rect);
            rects.push(callout(ui, Tone::Warning, &"A long warning. ".repeat(20)).rect);
        });
        out.textures_delta.clear();
        let [full, small, long] = [rects[0], rects[1], rects[2]];
        assert!((full.width() - 300.0).abs() < 1.0, "{full:?}");
        assert!(small.width() < 120.0, "{small:?}");
        assert!(
            long.width() <= 301.0 && long.height() > 3.0 * small.height(),
            "wrapped"
        );
        // Each tone has its own color, in both themes.
        for v in [Visuals::dark(), Visuals::light()] {
            let c = [Tone::Info, Tone::Tip, Tone::Warning].map(|t| t.color(&v));
            assert!(c[0] != c[1] && c[1] != c[2] && c[0] != c[2]);
        }
    }

    /// Draws sample callouts in both themes to the PNG named by `CALLOUT_PREVIEW`, to
    /// look at them: `CALLOUT_PREVIEW=out.png cargo test callout_preview -- --ignored`.
    #[test]
    #[ignore]
    fn callout_preview() {
        let Ok(path) = std::env::var("CALLOUT_PREVIEW") else {
            return;
        };
        crate::preview::render(&path, vec2(340.0, 460.0), |ui| {
            ui.heading("Variables");
            callout(
                ui,
                Tone::Info,
                "No variables yet. Add one, then use it with a field's fx button, a color picker or a text's link button.",
            );
            ui.add_space(6.0);
            let _ = ui.button("Add a variable");
            ui.add_space(10.0);
            ui.heading("Effects");
            callout(
                ui,
                Tone::Tip,
                "Applied top to bottom. Drag the dots to reorder.",
            );
            ui.add_space(10.0);
            ui.heading("Image");
            callout(ui, Tone::Warning, "Printed at 96 DPI: it may look blurry.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Shape layer");
                callout_small(ui, Tone::Info, "Add a shape layer first");
            });
            ui.add_space(6.0);
            callout_small(ui, Tone::Tip, "Ctrl+click to select several files");
        });
    }
}
