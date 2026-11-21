//! Editing a fill that can be one color, a gradient, a pattern or generated: a shape's
//! fill and stroke, a text's color, the page background (no pattern for a stroke, and
//! only a shape's fill is generated).
//!
//! [`inline`] goes where the color button was: a Solid / Linear / Radial / Pattern
//! switch, then the color button, or a swatch of the gradient. [`rows`] goes under it
//! for a pattern: its image, tile width, turn and offset; for a generated fill: the
//! generator's settings; for a gradient: its angle, or
//! its center and radius, then the bar of its color stops. A click on the
//! bar adds a stop there, a stop is dragged along it, and right-clicked to remove it; the
//! stop picked last shows its color button (linked to color variables like any color),
//! its position and a button removing it.

use egui::{
    Color32, ComboBox, DragValue, Id, Mesh, Pos2, Rect, Sense, Slider, Stroke, Ui, pos2, vec2,
};

use crate::document::gradient::{Gradient, GradientKind, MAX_STOPS, Stop};
use crate::document::pattern::Pattern;
use crate::i18n::tr;
use crate::render::effects::{EffectInstance, registry};

/// A color, or a gradient or a pattern in its place, with the color's variable link.
pub struct Fill<'a> {
    /// Straight RGBA; kept when a gradient is set, for going back to one color.
    pub color: &'a mut [u8; 4],
    pub link: &'a mut Option<String>,
    pub gradient: &'a mut Option<Gradient>,
    /// The pattern in place of both, for fills that can have one.
    pub pattern: Option<&'a mut Option<Pattern>>,
    /// The generator in place of the color and gradient, for fills that can have one;
    /// the pattern comes first.
    pub generator: Option<&'a mut Option<EffectInstance>>,
    /// Pattern sizes are in the fill's units: mm times this.
    pub units_per_mm: f32,
    /// Colors may be see-through; the page background may not.
    pub alpha: bool,
    /// Set by [`inline`] and [`rows`] when the pattern's image is to be chosen: the
    /// caller opens the file dialog.
    pub pick_pattern: bool,
}

impl Fill<'_> {
    /// It has settings for [`rows`]: a gradient, a pattern or a generator.
    pub fn has_rows(&self) -> bool {
        self.gradient.is_some() || self.patterned() || self.generated()
    }

    fn patterned(&self) -> bool {
        self.pattern.as_ref().is_some_and(|p| p.is_some())
    }

    /// Generated, with no pattern over it.
    fn generated(&self) -> bool {
        !self.patterned() && self.generator.as_ref().is_some_and(|g| g.is_some())
    }
}

/// What fills: the choices of the switch.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Solid,
    Gradient(GradientKind),
    Pattern,
    Generated,
}

/// A pattern's settings: its image (a click chooses another), the tile's width, turn
/// and offset in mm. Returns whether they changed, and whether another image is to be
/// chosen.
fn pattern_rows(ui: &mut Ui, p: &mut Pattern, units_per_mm: f32) -> (bool, bool) {
    let mut changed = false;
    let mut pick = false;
    let k = units_per_mm.max(1e-6);
    // A length in the fill's units, shown in mm.
    let mm = |ui: &mut Ui, v: &mut f32, prefix: &str, min: f32, hint: &str| {
        let mut x = *v / k;
        let r = crate::ui::layers::prefixed(ui, prefix, |ui| {
            ui.add(
                DragValue::new(&mut x)
                    .range(min..=f32::MAX)
                    .speed(0.1)
                    .fixed_decimals(1),
            )
        })
        .on_hover_text(hint);
        if r.changed() {
            *v = x * k;
        }
        r.changed()
    };
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let name = p
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let missing = p.tile.is_none();
            let mut text = egui::RichText::new(name);
            if missing {
                text = text.color(ui.visuals().error_fg_color);
            }
            let r = ui.add(egui::Button::new(text).truncate());
            let r = if missing {
                r.on_hover_text(tr!("PATTERN_IMAGE_MISSING"))
            } else {
                r.on_hover_text(tr!("PATTERN_IMAGE_HINT"))
            };
            pick = r.clicked();
        });
        ui.horizontal(|ui| {
            ui.label(tr!("PATTERN_TILE"));
            changed |= mm(ui, &mut p.size, "", 0.1, tr!("PATTERN_TILE_HINT"));
            ui.label("mm");
            ui.label(tr!("PATTERN_ANGLE"));
            changed |= ui
                .add(
                    DragValue::new(&mut p.angle_deg)
                        .range(-180.0..=180.0)
                        .speed(0.0)
                        .fixed_decimals(0),
                )
                .on_hover_cursor(egui::CursorIcon::Text)
                .on_hover_text(tr!("PATTERN_ANGLE_HINT"))
                .changed();
            crate::ui::layers::unit(ui, "°");
            changed |= crate::ui::dial::angle_dial(
                ui,
                &mut p.angle_deg,
                crate::ui::dial::Turn::FromTop,
                0.0,
            )
            .on_hover_text(tr!("PATTERN_ANGLE_HINT"))
            .changed();
        });
        ui.horizontal(|ui| {
            ui.label(tr!("PATTERN_OFFSET"));
            let [x, y] = &mut p.offset;
            changed |= mm(ui, x, "x ", f32::MIN, tr!("PATTERN_OFFSET_HINT"));
            changed |= mm(ui, y, "y ", f32::MIN, tr!("PATTERN_OFFSET_HINT"));
            ui.label("mm");
        });
    });
    (changed, pick)
}

/// Height of the stops bar, and of the stops' markers under it.
const BAR: f32 = 18.0;
const MARKER: f32 = 12.0;
/// A new linear gradient's angle as kept (from the right, counterclockwise): 0° as shown,
/// its colors running upward.
const NEW_ANGLE: f32 = 90.0;
/// How far the selected color's box points up at its marker.
const NOTCH: f32 = 5.0;

/// The switch between one color and a gradient, then the color button or a swatch of
/// the gradient. Returns whether anything changed. `ui_key` tells this fill from others.
pub fn inline(ui: &mut Ui, ui_key: impl std::hash::Hash + std::fmt::Debug, f: &mut Fill) -> bool {
    let id = ui.make_persistent_id(("fill", ui_key));
    let mut changed = false;
    let patterned = f.patterned();
    let generated = f.generated();
    let kind = match (patterned, generated, &f.gradient) {
        (true, _, _) => Kind::Pattern,
        (_, true, _) => Kind::Generated,
        (_, _, Some(g)) => Kind::Gradient(g.kind),
        (_, _, None) => Kind::Solid,
    };
    let name = |k: Kind| match k {
        Kind::Solid => tr!("FILL_SOLID"),
        Kind::Gradient(GradientKind::Linear) => tr!("FILL_LINEAR_GRADIENT"),
        Kind::Gradient(GradientKind::Radial) => tr!("FILL_RADIAL_GRADIENT"),
        Kind::Pattern => tr!("FILL_PATTERN"),
        Kind::Generated => tr!("FILL_GENERATED"),
    };
    let mut kinds = vec![
        Kind::Solid,
        Kind::Gradient(GradientKind::Linear),
        Kind::Gradient(GradientKind::Radial),
    ];
    if f.pattern.is_some() {
        kinds.push(Kind::Pattern);
    }
    if f.generator.is_some() {
        kinds.push(Kind::Generated);
    }
    let mut picked = kind;
    ComboBox::from_id_salt(id.with("kind"))
        .selected_text(name(kind))
        .width(90.0)
        .show_ui(ui, |ui| {
            for k in kinds {
                ui.selectable_value(&mut picked, k, name(k));
            }
        })
        .response
        .on_hover_text(tr!("FILL_KIND_HINT"));
    if picked != kind {
        match picked {
            // Once its image is chosen.
            Kind::Pattern => f.pick_pattern = true,
            Kind::Generated => {
                // The generator it had under a pattern, or the first one.
                if let Some(g) = f.generator.as_deref_mut()
                    && g.is_none()
                {
                    *g = registry()
                        .generators()
                        .next()
                        .map(EffectInstance::generator);
                }
                changed = true;
            }
            Kind::Solid => {
                changed = true;
                *f.gradient = None;
            }
            Kind::Gradient(k) => {
                changed = true;
                // From the color, or the gradient as it was with the other spread.
                let mut g = f.gradient.take().unwrap_or_else(|| Gradient {
                    angle: NEW_ANGLE,
                    ..Gradient::from_color(*f.color)
                });
                g.kind = k;
                *f.gradient = Some(g);
            }
        }
        if picked != Kind::Pattern
            && let Some(p) = f.pattern.as_deref_mut()
        {
            *p = None;
        }
        if !matches!(picked, Kind::Pattern | Kind::Generated)
            && let Some(g) = f.generator.as_deref_mut()
        {
            *g = None;
        }
    }
    if patterned && picked == Kind::Pattern {
        return changed;
    }
    if picked == Kind::Generated {
        if let Some(Some(g)) = f.generator.as_deref_mut() {
            changed |= crate::ui::effects::generator_menu(ui, id.value(), g);
            changed |= crate::ui::effects::regenerate_button(ui, g);
        }
        return changed;
    }
    match f.gradient {
        None => changed |= crate::ui::colors::button(ui, f.color, f.alpha, Some(f.link)).changed(),
        Some(g) => {
            let (rect, _) = ui.allocate_exact_size(
                vec2(
                    ui.spacing().interact_size.y * 2.0,
                    ui.spacing().interact_size.y,
                ),
                Sense::hover(),
            );
            paint_preview(ui, rect, g);
            ui.painter().rect_stroke(
                rect,
                2.0,
                ui.visuals().widgets.inactive.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
    changed
}

/// A gradient's settings, under [`inline`]; nothing for one color. Returns whether
/// anything changed.
pub fn rows(ui: &mut Ui, ui_key: impl std::hash::Hash + std::fmt::Debug, f: &mut Fill) -> bool {
    // Under a header that folds them away, open at first.
    let id = ui.make_persistent_id(("fill_rows", &ui_key));
    let mut changed = false;
    ui.vertical(|ui| {
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| ui.weak(tr!("FILL_SETTINGS")))
            .body(|ui| changed = settings_rows(ui, &ui_key, f));
    });
    changed
}

/// The settings [`rows`] shows, when unfolded.
fn settings_rows(
    ui: &mut Ui,
    ui_key: impl std::hash::Hash + std::fmt::Debug,
    f: &mut Fill,
) -> bool {
    let units_per_mm = f.units_per_mm;
    if let Some(Some(p)) = f.pattern.as_deref_mut() {
        let (changed, pick) = pattern_rows(ui, p, units_per_mm);
        f.pick_pattern |= pick;
        return changed;
    }
    if let Some(Some(g)) = f.generator.as_deref_mut() {
        let key = ui.make_persistent_id(("fill", &ui_key)).value();
        // In a column of its own, as a gradient's settings, so the rows below make room.
        return ui
            .vertical(|ui| crate::ui::effects::generator_params(ui, key, g))
            .inner;
    }
    let Some(g) = f.gradient.as_mut() else {
        return false;
    };
    let id = ui.make_persistent_id(("fill", ui_key));
    let mut changed = false;
    ui.vertical(|ui| {
        // Labels in a column, the controls lined up after them.
        egui::Grid::new(id.with("grid"))
            .num_columns(2)
            .show(ui, |ui| match g.kind {
                GradientKind::Linear => {
                    ui.label(tr!("GRADIENT_ANGLE"));
                    ui.horizontal(|ui| {
                        // Shown from the top, clockwise (0° upward, 90° rightward), as
                        // the dial's dot points where the colors run; kept from the
                        // right, counterclockwise.
                        let mut shown = from_top(g.angle);
                        if ui
                            .add(
                                DragValue::new(&mut shown)
                                    .range(-180.0..=180.0)
                                    .speed(0.0)
                                    .fixed_decimals(0),
                            )
                            .on_hover_cursor(egui::CursorIcon::Text)
                            .on_hover_text(tr!("GRADIENT_ANGLE_HINT"))
                            .changed()
                        {
                            g.angle = from_top(shown);
                            changed = true;
                        }
                        crate::ui::layers::unit(ui, "°");
                        changed |= crate::ui::dial::angle_dial(
                            ui,
                            &mut g.angle,
                            crate::ui::dial::Turn::FromRight,
                            NEW_ANGLE,
                        )
                        .on_hover_text(tr!("GRADIENT_ANGLE_HINT"))
                        .changed();
                    });
                    ui.end_row();
                }
                GradientKind::Radial => {
                    // A fraction shown in percent: a slider up to 100 %, a field beyond.
                    let mut percent =
                        |ui: &mut Ui, v: &mut f32, max: f32, prefix: &str, hint: &str| {
                            ui.horizontal(|ui| {
                                if !prefix.is_empty() {
                                    ui.label(prefix);
                                }
                                let mut p = *v * 100.0;
                                let r = if max <= 100.0 {
                                    ui.add(Slider::new(&mut p, 0.0..=max).fixed_decimals(0))
                                } else {
                                    ui.add(
                                        DragValue::new(&mut p)
                                            .range(0.0..=max)
                                            .speed(1.0)
                                            .fixed_decimals(0),
                                    )
                                }
                                .on_hover_text(hint);
                                crate::ui::layers::unit(ui, "%");
                                if r.changed() {
                                    *v = p / 100.0;
                                    changed = true;
                                }
                            });
                        };
                    ui.label(tr!("GRADIENT_CENTER"));
                    percent(
                        ui,
                        &mut g.center[0],
                        100.0,
                        "x",
                        tr!("GRADIENT_CENTER_X_HINT"),
                    );
                    ui.end_row();
                    ui.label("");
                    percent(
                        ui,
                        &mut g.center[1],
                        100.0,
                        "y",
                        tr!("GRADIENT_CENTER_Y_HINT"),
                    );
                    ui.end_row();
                    ui.label(tr!("GRADIENT_RADIUS"));
                    percent(ui, &mut g.radius, 300.0, "", tr!("GRADIENT_RADIUS_HINT"));
                    ui.end_row();
                }
            });
        let selected_id = id.with("selected");
        let mut selected = ui
            .data(|d| d.get_temp::<usize>(selected_id))
            .unwrap_or(0)
            .min(g.stops.len().saturating_sub(1));
        let (bar_changed, bar) = stops_bar(ui, id, g, &mut selected, f.alpha);
        changed |= bar_changed;
        // The stop picked last, in a box pointing at its marker: its color, where it
        // is, and removing it.
        let Some(at) = g
            .stops
            .get(selected)
            .map(|s| bar.left() + s.pos * bar.width())
        else {
            return;
        };
        ui.add_space(NOTCH);
        let visuals = ui.visuals().clone();
        // Outlined like the selected marker.
        let stroke = Stroke::new(1.0, visuals.selection.stroke.color);
        let frame = egui::Frame::NONE
            .fill(visuals.faint_bg_color)
            .stroke(stroke)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::same(6));
        let width = bar.width() + MARKER;
        let shown = frame.show(ui, |ui| {
            ui.set_width(width - 12.0);
            ui.horizontal(|ui| {
                let can_remove = g.stops.len() > 2;
                let Some(stop) = g.stops.get_mut(selected) else {
                    return;
                };
                ui.label(tr!("GRADIENT_STOP"));
                changed |=
                    crate::ui::colors::button(ui, &mut stop.color, f.alpha, Some(&mut stop.link))
                        .changed();
                // Where it is, to type it exactly: its marker is dragged on the bar.
                let mut p = stop.pos * 100.0;
                if ui
                    .add(
                        DragValue::new(&mut p)
                            .range(0.0..=100.0)
                            .speed(0.5)
                            .fixed_decimals(0),
                    )
                    .on_hover_text(tr!("GRADIENT_STOP_POSITION_HINT"))
                    .changed()
                {
                    stop.pos = p / 100.0;
                    changed = true;
                }
                crate::ui::layers::unit(ui, "%");
                // Removing it, at the box's right end.
                let remove = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::ui::layers::tool_button(
                            ui,
                            crate::ui::layers::Tool::Delete,
                            can_remove,
                            tr!("REMOVE_GRADIENT_STOP"),
                        )
                    })
                    .inner;
                if remove {
                    g.stops.remove(selected);
                    selected = selected.saturating_sub(1);
                    changed = true;
                }
            });
        });
        // The notch on the box's top edge, under the marker.
        let top = shown.response.rect.top();
        let notch = [
            pos2(at - NOTCH, top + 0.5),
            pos2(at, top - NOTCH),
            pos2(at + NOTCH, top + 0.5),
        ];
        let painter = ui.painter();
        painter.add(egui::Shape::convex_polygon(
            notch.to_vec(),
            visuals.faint_bg_color,
            Stroke::NONE,
        ));
        painter.add(egui::Shape::line(notch.to_vec(), stroke));
        ui.data_mut(|d| d.insert_temp(selected_id, selected));
    });
    changed
}

/// The gradient as a bar, its stops as markers under it. A click on the bar adds a stop
/// of the color there; a marker is picked by a click, moved by a drag and removed by a
/// right click (two stops always stay).
fn stops_bar(
    ui: &mut Ui,
    id: Id,
    g: &mut Gradient,
    selected: &mut usize,
    alpha: bool,
) -> (bool, Rect) {
    let mut changed = false;
    let width = ui.available_width().clamp(150.0, 280.0);
    let (rect, bar_response) = ui.allocate_exact_size(vec2(width, BAR + MARKER), Sense::click());
    // Room at both ends for half a marker.
    let bar = Rect::from_min_max(
        rect.min + vec2(MARKER / 2.0, 0.0),
        pos2(rect.max.x - MARKER / 2.0, rect.min.y + BAR),
    );
    let x_of = |pos: f32| bar.left() + pos * bar.width();
    let pos_of = |x: f32| ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);

    // The markers first, so the bar under them takes only the clicks they leave.
    let mut remove = None;
    for (i, stop) in g.stops.iter_mut().enumerate() {
        let r = ui.interact(
            marker_rect(x_of(stop.pos), bar.bottom()),
            id.with(("stop", i)),
            Sense::click_and_drag(),
        );
        if r.clicked() || r.drag_started() {
            *selected = i;
        }
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            stop.pos = pos_of(p.x);
            changed = true;
        }
        if r.secondary_clicked() {
            remove = Some(i);
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
    if let Some(i) = remove.filter(|_| g.stops.len() > 2) {
        g.stops.remove(i);
        if *selected >= i {
            *selected = selected.saturating_sub(1);
        }
        changed = true;
    }
    let bar_response = bar_response.on_hover_text(tr!("GRADIENT_BAR_HINT"));
    if bar_response.clicked()
        && g.stops.len() < MAX_STOPS
        && let Some(p) = bar_response.interact_pointer_pos()
    {
        let pos = pos_of(p.x);
        g.stops.push(Stop::new(pos, g.color_at(pos)));
        *selected = g.stops.len() - 1;
        changed = true;
    }

    if ui.is_rect_visible(rect) {
        if alpha {
            checkers(ui, bar);
        }
        paint_strip(ui, bar, g);
        let visuals = ui.visuals();
        ui.painter().rect_stroke(
            bar,
            2.0,
            visuals.widgets.inactive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        for (i, stop) in g.stops.iter().enumerate() {
            let x = x_of(stop.pos);
            let r = marker_rect(x, bar.bottom());
            let on = i == *selected;
            let stroke = if on {
                Stroke::new(2.0, visuals.selection.stroke.color)
            } else {
                Stroke::new(1.0, visuals.strong_text_color())
            };
            // A tip pointing at its place on the bar, then a swatch of its color.
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    pos2(x, bar.bottom() - 3.0),
                    pos2(r.right(), r.top() + 3.0),
                    pos2(r.right(), r.bottom()),
                    pos2(r.left(), r.bottom()),
                    pos2(r.left(), r.top() + 3.0),
                ],
                color32(stop.color),
                stroke,
            ));
        }
    }
    (changed, bar)
}

/// Where a stop's marker is, its tip at `x` on the bar's bottom edge.
fn marker_rect(x: f32, bar_bottom: f32) -> Rect {
    Rect::from_min_max(
        pos2(x - MARKER / 2.0, bar_bottom),
        pos2(x + MARKER / 2.0, bar_bottom + MARKER),
    )
}

fn color32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// A linear gradient's angle counted from the top, clockwise, from one counted from the
/// right, counterclockwise, or back: the same turn both ways, between -180 and 180.
fn from_top(deg: f32) -> f32 {
    let a = (90.0 - deg).rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

/// The gradient over `rect` as it fills a box: at its angle, or around its center.
fn paint_preview(ui: &Ui, rect: Rect, g: &Gradient) {
    use crate::document::gradient::Geometry;
    const STEPS: usize = 24;
    let Some(frame) =
        tiny_skia::Rect::from_ltrb(rect.left(), rect.top(), rect.right(), rect.bottom())
    else {
        return;
    };
    // Where a point is along the gradient, 0 to 1.
    let geometry = g.geometry(frame);
    let t_at = |p: Pos2| match geometry {
        Geometry::Linear { from, to } => {
            let d = [to[0] - from[0], to[1] - from[1]];
            let dd = (d[0] * d[0] + d[1] * d[1]).max(1e-6);
            ((p.x - from[0]) * d[0] + (p.y - from[1]) * d[1]) / dd
        }
        Geometry::Radial { ellipse: e } => {
            let (x, y) = ((p.x - e.tx) / e.sx, (p.y - e.ty) / e.sy);
            (x * x + y * y).sqrt()
        }
    };
    let mut mesh = Mesh::default();
    for j in 0..=STEPS {
        for i in 0..=STEPS {
            let p = pos2(
                rect.left() + rect.width() * i as f32 / STEPS as f32,
                rect.top() + rect.height() * j as f32 / STEPS as f32,
            );
            mesh.colored_vertex(p, color32(g.color_at(t_at(p))));
            if i > 0 && j > 0 {
                let k = (j * (STEPS + 1) + i) as u32;
                let row = (STEPS + 1) as u32;
                mesh.add_triangle(k - row - 1, k - row, k);
                mesh.add_triangle(k - row - 1, k - 1, k);
            }
        }
    }
    ui.painter().add(mesh);
}

/// The gradient's colors from left to right across `rect`, whatever its kind.
fn paint_strip(ui: &Ui, rect: Rect, g: &Gradient) {
    const STEPS: usize = 48;
    let mut mesh = Mesh::default();
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let x = rect.left() + t * rect.width();
        let c = color32(g.color_at(t));
        mesh.colored_vertex(Pos2::new(x, rect.top()), c);
        mesh.colored_vertex(Pos2::new(x, rect.bottom()), c);
        if i > 0 {
            let k = (2 * i) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    ui.painter().add(mesh);
}

/// A checkerboard under see-through colors.
fn checkers(ui: &Ui, rect: Rect) {
    let side = rect.height() / 2.0;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(200));
    let cols = (rect.width() / side).ceil() as usize;
    for i in 0..cols {
        for j in 0..2 {
            if (i + j) % 2 == 0 {
                let min = rect.min + vec2(i as f32 * side, j as f32 * side);
                painter.rect_filled(
                    Rect::from_min_size(min, vec2(side, side)),
                    0.0,
                    Color32::from_gray(140),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::from_top;

    #[test]
    fn angle_is_shown_from_the_top_clockwise() {
        // Stored from the right, counterclockwise: 0 rightward, 90 upward.
        assert_eq!(from_top(90.0), 0.0);
        assert_eq!(from_top(0.0), 90.0);
        assert_eq!(from_top(-90.0), 180.0);
        assert_eq!(from_top(180.0), -90.0);
        // The same turn back.
        for a in [-170.0, -45.0, 0.0, 30.0, 135.0, 180.0] {
            assert_eq!(from_top(from_top(a)), a);
        }
    }

    use super::*;

    /// One frame of a fill's editor; returns where each stop marker was drawn (their
    /// tips) and whether it changed.
    struct Harness {
        ctx: egui::Context,
        time: f64,
        color: [u8; 4],
        link: Option<String>,
        gradient: Option<Gradient>,
    }

    impl Harness {
        fn new(gradient: Option<Gradient>) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                time: 0.0,
                color: [255, 0, 0, 255],
                link: None,
                gradient,
            };
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<(String, Rect)>, bool) {
            self.time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let (color, link, gradient) = (&mut self.color, &mut self.link, &mut self.gradient);
            let mut out = self.ctx.run_ui(input, |ui| {
                let mut f = Fill {
                    color,
                    link,
                    gradient,
                    pattern: None,
                    generator: None,
                    units_per_mm: 1.0,
                    alpha: true,
                    pick_pattern: false,
                };
                ui.horizontal(|ui| changed |= inline(ui, "t", &mut f));
                changed |= rows(ui, "t", &mut f);
            });
            out.textures_delta.clear();
            let texts = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        Rect::from_min_size(t.pos, t.galley.size()),
                    )),
                    _ => None,
                })
                .collect();
            (texts, changed)
        }

        fn click(&mut self, p: Pos2, button: egui::PointerButton) -> bool {
            let press = |pressed| egui::Event::PointerButton {
                pos: p,
                button,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(p)]);
            let a = self.frame(vec![press(true)]).1;
            let b = self.frame(vec![press(false)]).1;
            a || b
        }

        fn g(&self) -> &Gradient {
            self.gradient.as_ref().unwrap()
        }

        /// The stops bar's left and right ends and its middle height, measured from the
        /// markers of the stops at 0% and 100%: their tips touch the bar's bottom edge.
        fn bar(&mut self) -> (f32, f32, f32) {
            self.time += 0.1;
            let (color, link, gradient) = (&mut self.color, &mut self.link, &mut self.gradient);
            let mut out = self.ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut f = Fill {
                    color,
                    link,
                    gradient,
                    pattern: None,
                    generator: None,
                    units_per_mm: 1.0,
                    alpha: true,
                    pick_pattern: false,
                };
                ui.horizontal(|ui| inline(ui, "t", &mut f));
                rows(ui, "t", &mut f);
            });
            out.textures_delta.clear();
            let tips: Vec<Pos2> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Path(p) if p.points.len() == 5 => Some(p.points[0]),
                    _ => None,
                })
                .collect();
            assert_eq!(tips.len(), 2, "two stops' markers");
            let (l, r) = (tips[0].x.min(tips[1].x), tips[0].x.max(tips[1].x));
            (l, r, tips[0].y + 3.0 - BAR / 2.0)
        }
    }

    #[test]
    fn switching_to_a_gradient_and_back() {
        let mut h = Harness::new(None);
        let (texts, _) = h.frame(vec![]);
        let combo = texts.iter().find(|(t, _)| t == "Solid").unwrap().1.center();
        h.click(combo, egui::PointerButton::Primary);
        let (texts, _) = h.frame(vec![]);
        let radial = texts
            .iter()
            .find(|(t, _)| t == "Radial gradient")
            .unwrap()
            .1
            .center();
        assert!(h.click(radial, egui::PointerButton::Primary));
        // From the color to white, spread around the center.
        assert_eq!(h.g().kind, GradientKind::Radial);
        assert_eq!(h.g().stops[0].color, [255, 0, 0, 255]);
        assert_eq!(
            h.color,
            [255, 0, 0, 255],
            "the color is kept for going back"
        );
    }

    #[test]
    fn the_bar_adds_moves_and_removes_stops() {
        let mut h = Harness::new(Some(Gradient::from_color([255, 0, 0, 255])));
        let (left, right, y) = h.bar();
        let at = |t: f32| pos2(left + t * (right - left), y);
        // A click in the middle of the bar adds a stop there, of the color there.
        assert!(h.click(at(0.5), egui::PointerButton::Primary));
        assert_eq!(h.g().stops.len(), 3);
        let added = &h.g().stops[2];
        assert!((added.pos - 0.5).abs() < 0.02, "{}", added.pos);
        assert!(
            added.color[1] > 100 && added.color[1] < 160,
            "{:?}",
            added.color
        );
        // Its marker, under the bar, is dragged to 75%.
        let marker = pos2(at(0.5).x, y + BAR / 2.0 + MARKER / 2.0);
        let to = pos2(at(0.75).x, marker.y);
        let press = |p, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(vec![egui::Event::PointerMoved(marker)]);
        h.frame(vec![press(marker, true)]);
        for k in 1..=4 {
            let p = marker + (to - marker) * (k as f32 / 4.0);
            h.frame(vec![egui::Event::PointerMoved(p)]);
        }
        h.frame(vec![press(to, false)]);
        assert!(
            (h.g().stops[2].pos - 0.75).abs() < 0.02,
            "{}",
            h.g().stops[2].pos
        );
        // Right-clicked, it goes; the last two stay.
        assert!(h.click(to, egui::PointerButton::Secondary));
        assert_eq!(h.g().stops.len(), 2);
        let first = pos2(at(0.0).x, to.y);
        h.click(first, egui::PointerButton::Secondary);
        assert_eq!(h.g().stops.len(), 2, "a gradient keeps two colors");
    }

    /// The editor for a linear and a radial gradient, one color and a pattern, to look
    /// at it:
    /// `GRADIENT_PREVIEW=out.png cargo test gradient_preview -- --ignored`.
    #[test]
    #[ignore]
    fn gradient_preview() {
        let Ok(path) = std::env::var("GRADIENT_PREVIEW") else {
            return;
        };
        let mut linear = Gradient::from_color([220, 40, 40, 255]);
        linear.stops.push(Stop::new(0.4, [250, 210, 60, 255]));
        linear.stops[1].color = [40, 80, 220, 120];
        linear.angle = 30.0;
        let mut radial = linear.clone();
        radial.kind = GradientKind::Radial;
        let state = std::cell::RefCell::new([
            ([0u8; 4], None::<String>, Some(linear)),
            ([0u8; 4], None, Some(radial)),
            ([70, 130, 180, 255], None, None),
        ]);
        let pattern = std::cell::RefCell::new(Some(crate::document::pattern::Pattern {
            source: "/somewhere/carbon-fiber.png".into(),
            size: 12.5,
            angle_deg: 45.0,
            offset: [0.0, 2.0],
            data: None,
            tile: Some(std::sync::Arc::new(tiny_skia::Pixmap::new(4, 4).unwrap())),
        }));
        crate::ui::preview::render(&path, vec2(380.0, 440.0), |ui| {
            let mut state = state.borrow_mut();
            egui::Frame::NONE.inner_margin(10.0).show(ui, |ui| {
                for (i, (color, link, gradient)) in state.iter_mut().enumerate() {
                    let mut f = Fill {
                        color,
                        link,
                        gradient,
                        pattern: None,
                        generator: None,
                        units_per_mm: 1.0,
                        alpha: true,
                        pick_pattern: false,
                    };
                    ui.horizontal(|ui| {
                        ui.label("Fill");
                        inline(ui, i, &mut f);
                    });
                    rows(ui, i, &mut f);
                    ui.separator();
                }
                // A pattern, its kind open to show the choices.
                let mut pattern = pattern.borrow_mut();
                let (mut color, mut link, mut gradient) = ([0u8; 4], None, None);
                let mut f = Fill {
                    color: &mut color,
                    link: &mut link,
                    gradient: &mut gradient,
                    pattern: Some(&mut pattern),
                    generator: None,
                    units_per_mm: 1.0,
                    alpha: true,
                    pick_pattern: false,
                };
                ui.horizontal(|ui| {
                    ui.label("Fill");
                    inline(ui, "pattern", &mut f);
                });
                rows(ui, "pattern", &mut f);
            });
        });
    }
}
