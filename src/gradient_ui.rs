//! Editing a fill that can be one color or a gradient: a shape's fill and stroke, a
//! text's color, the page background.
//!
//! [`inline`] goes where the color button was: a Solid / Linear / Radial switch, then
//! the color button, or a swatch of the gradient. [`rows`] goes under it for a gradient:
//! its angle, or its center and radius, then the bar of its color stops. A click on the
//! bar adds a stop there, a stop is dragged along it, and right-clicked to remove it; the
//! stop picked last shows its color button (linked to color variables like any color),
//! its position and a button removing it.

use egui::{Color32, ComboBox, DragValue, Id, Mesh, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::gradient::{Gradient, GradientKind, MAX_STOPS, Stop};
use crate::i18n::tr;

/// A color, or a gradient in its place, with the color's variable link.
pub struct Fill<'a> {
    /// Straight RGBA; kept when a gradient is set, for going back to one color.
    pub color: &'a mut [u8; 4],
    pub link: &'a mut Option<String>,
    pub gradient: &'a mut Option<Gradient>,
    /// Colors may be see-through; the page background may not.
    pub alpha: bool,
}

/// Height of the stops bar, and of the stops' markers under it.
const BAR: f32 = 18.0;
const MARKER: f32 = 12.0;

/// The switch between one color and a gradient, then the color button or a swatch of
/// the gradient. Returns whether anything changed. `salt` tells this fill from others.
pub fn inline(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, f: &mut Fill) -> bool {
    let id = ui.make_persistent_id(("fill", salt));
    let mut changed = false;
    let kind = f.gradient.as_ref().map(|g| g.kind);
    let name = |k: Option<GradientKind>| match k {
        None => tr!("FILL_SOLID"),
        Some(GradientKind::Linear) => tr!("FILL_LINEAR_GRADIENT"),
        Some(GradientKind::Radial) => tr!("FILL_RADIAL_GRADIENT"),
    };
    let mut picked = kind;
    ComboBox::from_id_salt(id.with("kind"))
        .selected_text(name(kind))
        .width(90.0)
        .show_ui(ui, |ui| {
            for k in [None, Some(GradientKind::Linear), Some(GradientKind::Radial)] {
                ui.selectable_value(&mut picked, k, name(k));
            }
        })
        .response
        .on_hover_text(tr!("FILL_KIND_HINT"));
    if picked != kind {
        changed = true;
        *f.gradient = picked.map(|k| {
            // From the color, or the gradient as it was with the other spread.
            let mut g = f
                .gradient
                .take()
                .unwrap_or_else(|| Gradient::from_color(*f.color));
            g.kind = k;
            g
        });
    }
    match f.gradient {
        None => changed |= crate::colors::button(ui, f.color, f.alpha, f.link).changed(),
        Some(g) => {
            let (rect, _) = ui.allocate_exact_size(
                vec2(
                    ui.spacing().interact_size.y * 2.0,
                    ui.spacing().interact_size.y,
                ),
                Sense::hover(),
            );
            paint_strip(ui, rect, g);
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
pub fn rows(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, f: &mut Fill) -> bool {
    let Some(g) = f.gradient.as_mut() else {
        return false;
    };
    let id = ui.make_persistent_id(("fill", salt));
    let mut changed = false;
    ui.vertical(|ui| {
        ui.horizontal(|ui| match g.kind {
            GradientKind::Linear => {
                ui.label(tr!("GRADIENT_ANGLE"));
                changed |= ui
                    .add(
                        DragValue::new(&mut g.angle)
                            .range(-180.0..=180.0)
                            .speed(1.0)
                            .fixed_decimals(0)
                            .suffix("°"),
                    )
                    .on_hover_text(tr!("GRADIENT_ANGLE_HINT"))
                    .changed();
            }
            GradientKind::Radial => {
                let mut percent = |ui: &mut Ui, v: &mut f32, max: f32, prefix: &str, hint: &str| {
                    let mut p = *v * 100.0;
                    let r = ui
                        .add(
                            DragValue::new(&mut p)
                                .prefix(prefix)
                                .range(0.0..=max)
                                .speed(1.0)
                                .fixed_decimals(0)
                                .suffix("%"),
                        )
                        .on_hover_text(hint);
                    if r.changed() {
                        *v = p / 100.0;
                        changed = true;
                    }
                };
                ui.label(tr!("GRADIENT_CENTER"));
                percent(
                    ui,
                    &mut g.center[0],
                    100.0,
                    "x ",
                    tr!("GRADIENT_CENTER_X_HINT"),
                );
                percent(
                    ui,
                    &mut g.center[1],
                    100.0,
                    "y ",
                    tr!("GRADIENT_CENTER_Y_HINT"),
                );
                ui.label(tr!("GRADIENT_RADIUS"));
                percent(ui, &mut g.radius, 300.0, "", tr!("GRADIENT_RADIUS_HINT"));
            }
        });
        let selected_id = id.with("selected");
        let mut selected = ui
            .data(|d| d.get_temp::<usize>(selected_id))
            .unwrap_or(0)
            .min(g.stops.len().saturating_sub(1));
        changed |= stops_bar(ui, id, g, &mut selected, f.alpha);
        // The stop picked last: its color, where it is, and removing it.
        ui.horizontal(|ui| {
            let can_remove = g.stops.len() > 2;
            let Some(stop) = g.stops.get_mut(selected) else {
                return;
            };
            ui.label(tr!("GRADIENT_STOP"));
            changed |=
                crate::colors::button(ui, &mut stop.color, f.alpha, &mut stop.link).changed();
            let mut p = stop.pos * 100.0;
            if ui
                .add(
                    DragValue::new(&mut p)
                        .range(0.0..=100.0)
                        .speed(0.5)
                        .fixed_decimals(0)
                        .suffix("%"),
                )
                .on_hover_text(tr!("GRADIENT_STOP_POSITION_HINT"))
                .changed()
            {
                stop.pos = p / 100.0;
                changed = true;
            }
            if crate::layers_ui::tool_button(
                ui,
                crate::layers_ui::Tool::Delete,
                can_remove,
                tr!("REMOVE_GRADIENT_STOP"),
            ) {
                g.stops.remove(selected);
                selected = selected.saturating_sub(1);
                changed = true;
            }
        });
        ui.data_mut(|d| d.insert_temp(selected_id, selected));
    });
    changed
}

/// The gradient as a bar, its stops as markers under it. A click on the bar adds a stop
/// of the color there; a marker is picked by a click, moved by a drag and removed by a
/// right click (two stops always stay).
fn stops_bar(ui: &mut Ui, id: Id, g: &mut Gradient, selected: &mut usize, alpha: bool) -> bool {
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
    changed
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
                    alpha: true,
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
                    alpha: true,
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

    /// The editor for a linear and a radial gradient, to look at it:
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
        crate::preview::render(&path, vec2(380.0, 330.0), |ui| {
            let mut state = state.borrow_mut();
            egui::Frame::NONE.inner_margin(10.0).show(ui, |ui| {
                for (i, (color, link, gradient)) in state.iter_mut().enumerate() {
                    let mut f = Fill {
                        color,
                        link,
                        gradient,
                        alpha: true,
                    };
                    ui.horizontal(|ui| {
                        ui.label("Fill");
                        inline(ui, i, &mut f);
                    });
                    rows(ui, i, &mut f);
                    ui.separator();
                }
            });
        });
    }
}
