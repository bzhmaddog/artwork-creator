//! The Effects section of a layer's properties: its effect stack, with controls built
//! from each effect's parameter list.

use egui::collapsing_header::CollapsingState;
use egui::{ComboBox, CursorIcon, DragAndDrop, RichText, Sense, Slider, Stroke, Ui, vec2};
use serde_json::Value;

use crate::effects::{
    Effect, EffectInstance, ParamKind, ParamSpec, Params, color_to_hex, registry,
};
use crate::i18n::{tr, tr_n, translate};
use crate::layers_ui::{self, Tool};

/// Drag-and-drop payload: the index of the dragged effect in the layer's stack.
struct DraggedEffect {
    layer: u64,
    index: usize,
}

/// Moves the effect at `from` so it lands at insertion position `to` (0 to len), as
/// counted before the move.
fn move_effect(stack: &mut Vec<EffectInstance>, from: usize, to: usize) -> bool {
    let to = if to > from { to - 1 } else { to };
    if from == to || from >= stack.len() {
        return false;
    }
    let e = stack.remove(from);
    stack.insert(to.min(stack.len()), e);
    true
}

/// How far above or below the list a dragged effect can still be dropped.
const DROP_MARGIN: f32 = 16.0;

/// The grip on the left of an effect, by which it is dragged to reorder.
fn drag_handle(ui: &mut Ui, active: bool) -> egui::Response {
    let size = vec2(12.0, ui.spacing().interact_size.y);
    let (rect, r) = ui.allocate_exact_size(size, Sense::drag());
    let r = r.on_hover_text(tr!("DRAG_TO_REORDER"));
    if r.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if r.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&r);
        let color = if active || r.hovered() {
            visuals.fg_stroke.color
        } else {
            visuals.fg_stroke.color.gamma_multiply(0.6)
        };
        // Two columns of three dots.
        let c = rect.center();
        for dx in [-2.5, 2.5] {
            for dy in [-4.5, 0.0, 4.5] {
                ui.painter().circle_filled(c + vec2(dx, dy), 1.3, color);
            }
        }
    }
    r
}

/// Shows the effect stack of a layer. Returns true if it changed. With `px_mm`, the
/// length of a layer pixel on the page and a tooltip saying how it changes, sizes in
/// pixels are shown in mm: for text and shape layers, whose pixels are a fraction of the
/// font size (see `text::EM`) or of a mm (see `shape::UNITS_PER_MM`).
pub fn show(
    ui: &mut Ui,
    layer_id: u64,
    stack: &mut Vec<EffectInstance>,
    px_mm: Option<(f32, &str)>,
) -> bool {
    let mut added = None;
    let mut changed = false;
    let title = match stack.len() {
        0 => tr!("EFFECTS_SECTION").to_owned(),
        n => tr!("EFFECTS_SECTION_WITH_COUNT", n = n),
    };
    layers_ui::section(
        ui,
        "effects",
        RichText::new(title).strong(),
        |ui| {
            added = add_menu(ui);
            added.is_some()
        },
        |ui| changed = stack_ui(ui, layer_id, stack, px_mm),
    );
    // Added after the list was shown: it appears on the next frame.
    if let Some(e) = added {
        stack.push(e);
        changed = true;
    }
    changed
}

/// The "Add effect" menu. Returns the effect picked.
fn add_menu(ui: &mut Ui) -> Option<EffectInstance> {
    let mut added = None;
    let button = layers_ui::tool_icon(ui, Tool::Add, true, tr!("ADD_EFFECT"));
    egui::Popup::menu(&button).show(|ui| {
        let reg = registry();
        let mut add = |ui: &mut Ui, effect: &dyn Effect| {
            let mut r = ui.button(effect.name());
            if !effect.description().is_empty() {
                r = r.on_hover_text(effect.description());
            }
            if r.clicked() {
                added = Some(EffectInstance::new(effect));
                ui.close();
            }
        };
        for effect in reg.all().filter(|e| e.plugin().is_none()) {
            add(ui, effect);
        }
        // Each plugin's effects under its name.
        for plugin in &reg.plugins {
            let effects: Vec<_> = plugin.effects.iter().filter_map(|id| reg.get(id)).collect();
            if effects.is_empty() {
                continue;
            }
            ui.separator();
            let title = format!("{} {}", plugin.name, plugin.version);
            ui.weak(title.trim_end())
                .on_hover_text(tr!("PLUGIN_LOADED_FROM", path = plugin.path.display()));
            for effect in effects {
                add(ui, effect);
            }
        }
        if !reg.errors.is_empty() {
            ui.separator();
            let n = reg.errors.len();
            ui.label(
                RichText::new(format!("⚠ {}", tr_n!("PLUGIN_PROBLEM_COUNT", n)))
                    .color(ui.visuals().warn_fg_color),
            )
            .on_hover_text(reg.errors.join("\n"));
        }
    });
    added
}

/// The effects of the stack, with their controls. Returns true if it changed.
fn stack_ui(
    ui: &mut Ui,
    layer_id: u64,
    stack: &mut Vec<EffectInstance>,
    px_mm: Option<(f32, &str)>,
) -> bool {
    let mut changed = false;
    if stack.is_empty() {
        crate::callout::callout(ui, crate::callout::Tone::Info, tr!("EFFECTS_INTRO"));
        return changed;
    }
    if stack.len() > 1 {
        crate::callout::callout(ui, crate::callout::Tone::Tip, tr!("EFFECTS_ORDER_HINT"));
    }

    let mut remove = None;
    let mut drop = None;
    let dragged = DragAndDrop::payload::<DraggedEffect>(ui.ctx()).filter(|d| d.layer == layer_id);
    let full = ui.max_rect().x_range();
    // Area of each effect, header and controls, to find where a dragged one drops.
    let mut blocks = Vec::with_capacity(stack.len());
    for (i, inst) in stack.iter_mut().enumerate() {
        let effect = registry().get(&inst.effect);
        let id = ui.make_persistent_id(("effect", layer_id, i));
        let being_dragged = dragged.as_ref().is_some_and(|d| d.index == i);
        let block = ui.horizontal_top(|ui| {
            let handle = drag_handle(ui, being_dragged);
            handle.dnd_set_drag_payload(DraggedEffect {
                layer: layer_id,
                index: i,
            });
            let name = effect
                .map_or(tr!("MISSING_EFFECT"), |e| e.name())
                .to_owned();
            if handle.dragged()
                && let Some(p) = ui.ctx().pointer_interact_pos()
            {
                // The name follows the pointer while dragging.
                egui::Area::new(egui::Id::unique(("effect-drag", layer_id)))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .fixed_pos(p + vec2(12.0, 4.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
                    });
            }
            ui.vertical(|ui| {
                CollapsingState::load_with_default_open(ui.ctx(), id, true)
                    .show_header(ui, |ui| {
                        changed |= ui
                            .checkbox(&mut inst.enabled, "")
                            .on_hover_text(tr!("ENABLE_EFFECT"))
                            .changed();
                        let (text, tip) = match effect {
                            Some(e) => {
                                let t = RichText::new(e.name());
                                let t = if inst.enabled { t } else { t.weak() };
                                (t, e.description().to_owned())
                            }
                            None => (
                                RichText::new(tr!("MISSING_EFFECT"))
                                    .color(ui.visuals().warn_fg_color),
                                tr!("MISSING_EFFECT_HINT", id = inst.effect),
                            ),
                        };
                        let mut r = ui.add(egui::Label::new(text).sense(Sense::click()));
                        if !tip.is_empty() {
                            r = r.on_hover_text(tip);
                        }
                        r.context_menu(|ui| {
                            if let Some(e) = effect
                                && !e.params().is_empty()
                                && ui.button(tr!("RESET_EFFECT_PARAMS")).clicked()
                            {
                                let enabled = inst.enabled;
                                *inst = EffectInstance::new(e);
                                inst.enabled = enabled;
                                changed = true;
                            }
                            if ui.button(tr!("REMOVE_EFFECT")).clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if layers_ui::tool_button(ui, Tool::Delete, true, tr!("REMOVE_EFFECT"))
                            {
                                remove = Some(i);
                            }
                        });
                    })
                    .body(|ui| {
                        if let Some(e) = effect {
                            ui.add_enabled_ui(inst.enabled, |ui| {
                                changed |= params_ui(ui, (layer_id, i), e, inst, px_mm);
                            });
                        }
                    });
            });
        });
        blocks.push(block.response.rect);
    }

    // Drop target: the gap nearest the pointer, anywhere over the list or just around it.
    if let Some(d) = &dragged
        && let (Some(first), Some(last)) = (blocks.first(), blocks.last())
        && let Some(p) = ui.ctx().pointer_hover_pos()
        && full.contains(p.x)
        && (first.top() - DROP_MARGIN..=last.bottom() + DROP_MARGIN).contains(&p.y)
    {
        let to = blocks.iter().filter(|r| r.center().y < p.y).count();
        // Next to itself nothing moves: no line.
        if to != d.index && to != d.index + 1 {
            let y = match to {
                0 => first.top(),
                t if t == blocks.len() => last.bottom(),
                t => (blocks[t - 1].bottom() + blocks[t].top()) / 2.0,
            };
            let stroke = Stroke::new(2.0, ui.visuals().selection.stroke.color);
            ui.painter().hline(full, y, stroke);
        }
        if ui.input(|i| i.pointer.any_released()) {
            DragAndDrop::clear_payload(ui.ctx());
            drop = Some((d.index, to));
        }
    }

    if let Some(i) = remove {
        stack.remove(i);
        changed = true;
    }
    if let Some((from, to)) = drop {
        changed |= move_effect(stack, from, to);
    }
    changed
}

/// Controls for an effect's parameters. Returns true if one changed.
fn params_ui(
    ui: &mut Ui,
    salt: (u64, usize),
    effect: &dyn Effect,
    inst: &mut EffectInstance,
    px_mm: Option<(f32, &str)>,
) -> bool {
    let specs = effect.params();
    if specs.is_empty() {
        return false;
    }
    let mut changed = false;
    // Built-in labels are translated; plugins' are shown as they give them.
    let builtin = effect.plugin().is_none();
    egui::Grid::new(("effect_params", salt))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for spec in specs {
                ui.label(label_text(builtin, &spec.label));
                let params = Params::new(specs, &inst.params);
                if let Some(v) = param_ui(ui, salt, spec, builtin, &params, &mut inst.links, px_mm)
                {
                    inst.params.insert(spec.id.clone(), v);
                    changed = true;
                }
                ui.end_row();
            }
        });
    changed
}

/// A parameter or option label, translated for a built-in effect.
fn label_text(builtin: bool, text: &str) -> &str {
    if builtin { translate(text) } else { text }
}

/// One parameter's control, a color one linked through `links`. Returns the new value
/// if it changed.
fn param_ui(
    ui: &mut Ui,
    salt: (u64, usize),
    spec: &ParamSpec,
    builtin: bool,
    p: &Params,
    links: &mut std::collections::BTreeMap<String, String>,
    px_mm: Option<(f32, &str)>,
) -> Option<Value> {
    let id = spec.id.as_str();
    // Pixel sizes shown in mm on the page, when asked.
    let in_mm = |unit: &str| px_mm.filter(|_| unit.trim() == "px");
    match &spec.kind {
        ParamKind::Float {
            min,
            max,
            unit,
            decimals,
            ..
        } => {
            let mut v = p.f32(id);
            // Sizes in pixels span a wide range where small values matter most:
            // logarithmic, the slider moves in fine steps there.
            let slider = Slider::new(&mut v, *min..=*max).logarithmic(unit.trim() == "px");
            let r = ui
                .horizontal(|ui| match in_mm(unit) {
                    Some((k, hint)) => {
                        let k = f64::from(k.max(1e-6));
                        let r = ui
                            .add(
                                slider
                                    .custom_formatter(move |n, _| format!("{:.2}", n * k))
                                    .custom_parser(move |s| {
                                        s.trim().parse::<f64>().ok().map(|mm| mm / k)
                                    }),
                            )
                            .on_hover_text(hint);
                        layers_ui::unit(ui, "mm");
                        r
                    }
                    None => {
                        let r = ui.add(slider.fixed_decimals(*decimals));
                        layers_ui::unit(ui, unit);
                        r
                    }
                })
                .inner;
            r.changed().then(|| Value::from(v))
        }
        ParamKind::Int { min, max, unit, .. } => {
            let mut v = p.i32(id);
            ui.horizontal(|ui| {
                let r = ui.add(Slider::new(&mut v, *min..=*max));
                layers_ui::unit(ui, unit);
                r
            })
            .inner
            .changed()
            .then(|| Value::from(v))
        }
        ParamKind::Bool { .. } => {
            let mut v = p.bool(id);
            ui.checkbox(&mut v, "").changed().then(|| Value::from(v))
        }
        ParamKind::Color { .. } => {
            let mut v = p.color(id);
            crate::colors::linked_button(ui, links, spec.id.clone(), &mut v, true)
                .changed()
                .then(|| Value::from(color_to_hex(v)))
        }
        ParamKind::Choice { options, .. } => {
            let current = p.choice(id).to_owned();
            let label = options
                .iter()
                .find(|(o, _)| *o == current)
                .map_or(current.as_str(), |(_, l)| label_text(builtin, l));
            let mut picked = None;
            ComboBox::from_id_salt(("effect_choice", salt, id))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (o, l) in options {
                        if ui
                            .selectable_label(*o == current, label_text(builtin, l))
                            .clicked()
                            && *o != current
                        {
                            picked = Some(Value::from(o.clone()));
                        }
                    }
                });
            picked
        }
        ParamKind::Curve { .. } => {
            let mut c = p.curve(id);
            curve_ui(ui, (salt, id), &mut c).then(|| crate::effects::curve_value(c))
        }
    }
}

/// Size of the curve's graph, and the margin left around it for the handles.
const CURVE_SIZE: egui::Vec2 = vec2(190.0, 120.0);
const CURVE_MARGIN: f32 = 8.0;

/// An easing curve, as in animation tools: buttons for the usual ones, then its graph,
/// whose two handles are dragged to shape it. It is drawn as the opacity across the
/// fade: opaque at the top left (where the fade starts), transparent at the bottom right.
/// Returns whether it changed.
fn curve_ui(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    c: &mut [f32; 4],
) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        // The usual curves, each a thumbnail of itself, named in its tooltip.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (_, key, preset) in crate::effects::CURVE_PRESETS {
                let on = *c == preset;
                let (rect, r) = ui.allocate_exact_size(vec2(36.0, 24.0), Sense::click());
                let r = r.on_hover_text(translate(key));
                if r.clicked() && !on {
                    *c = preset;
                    changed = true;
                }
                let visuals = ui.visuals();
                let (fill, stroke) = if on {
                    (visuals.selection.bg_fill, visuals.selection.stroke)
                } else if r.hovered() {
                    (
                        visuals.widgets.hovered.weak_bg_fill,
                        visuals.widgets.hovered.bg_stroke,
                    )
                } else {
                    (
                        visuals.widgets.inactive.weak_bg_fill,
                        visuals.widgets.inactive.bg_stroke,
                    )
                };
                let painter = ui.painter();
                painter.rect(rect, 3.0, fill, stroke, egui::StrokeKind::Inside);
                let ink = if on {
                    visuals.selection.stroke.color
                } else {
                    visuals.text_color()
                };
                let b = rect.shrink2(vec2(7.0, 5.0));
                let points: Vec<egui::Pos2> = (0..=16)
                    .map(|i| {
                        let x = i as f32 / 16.0;
                        let y = crate::effects::ease(preset, x);
                        b.left_top() + vec2(x * b.width(), y * b.height())
                    })
                    .collect();
                painter.add(egui::Shape::line(points, Stroke::new(1.5, ink)));
            }
        });
        let (rect, bg) = ui.allocate_exact_size(CURVE_SIZE, Sense::hover());
        let inner = rect.shrink(CURVE_MARGIN);
        let to_screen =
            |x: f32, y: f32| inner.left_top() + vec2(x * inner.width(), y * inner.height());
        // The handles first, so the drawing below shows where they are now.
        let mut active = [false; 2];
        for (k, (ix, iy)) in [(0, 1), (2, 3)].into_iter().enumerate() {
            let at = to_screen(c[ix], c[iy]);
            let r = ui.interact(
                egui::Rect::from_center_size(at, egui::Vec2::splat(16.0)),
                ui.make_persistent_id((salt, "curve_handle", k)),
                Sense::drag(),
            );
            if r.dragged()
                && let Some(p) = r.interact_pointer_pos()
            {
                c[ix] = ((p.x - inner.left()) / inner.width()).clamp(0.0, 1.0);
                c[iy] = ((p.y - inner.top()) / inner.height()).clamp(0.0, 1.0);
                changed = true;
            }
            active[k] = r.hovered() || r.dragged();
            if active[k] {
                ui.ctx().set_cursor_icon(if r.dragged() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            }
        }
        if !ui.is_rect_visible(rect) {
            return;
        }
        let visuals = ui.visuals();
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);
        // Quarters, and the straight line a linear fade follows.
        let faint = Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color);
        for q in [0.25, 0.5, 0.75] {
            painter.vline(to_screen(q, 0.0).x, inner.y_range(), faint);
            painter.hline(inner.x_range(), to_screen(0.0, q).y, faint);
        }
        painter.rect_stroke(inner, 0.0, faint, egui::StrokeKind::Middle);
        painter.line_segment([to_screen(0.0, 0.0), to_screen(1.0, 1.0)], faint);
        // The curve, from its two ends through the handles' pull.
        let (p0, p1, p2, p3) = (
            to_screen(0.0, 0.0),
            to_screen(c[0], c[1]),
            to_screen(c[2], c[3]),
            to_screen(1.0, 1.0),
        );
        let accent = visuals.selection.stroke.color;
        let arm = Stroke::new(1.0, visuals.weak_text_color());
        painter.line_segment([p0, p1], arm);
        painter.line_segment([p3, p2], arm);
        let curve: Vec<egui::Pos2> = (0..=48)
            .map(|i| {
                let s = i as f32 / 48.0;
                let r = 1.0 - s;
                let w = [r * r * r, 3.0 * r * r * s, 3.0 * r * s * s, s * s * s];
                let x = w[0] * p0.x + w[1] * p1.x + w[2] * p2.x + w[3] * p3.x;
                let y = w[0] * p0.y + w[1] * p1.y + w[2] * p2.y + w[3] * p3.y;
                egui::pos2(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(curve, Stroke::new(2.0, accent)));
        for (k, p) in [p1, p2].into_iter().enumerate() {
            let fill = if active[k] {
                accent
            } else {
                visuals.window_fill
            };
            painter.circle(p, 5.0, fill, Stroke::new(1.5, accent));
        }
        bg.on_hover_text(tr!("FADE_CURVE_HINT"));
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(ids: &[&str]) -> Vec<EffectInstance> {
        ids.iter()
            .map(|id| EffectInstance {
                effect: (*id).into(),
                enabled: true,
                params: Default::default(),
                links: Default::default(),
            })
            .collect()
    }

    fn ids(s: &[EffectInstance]) -> Vec<&str> {
        s.iter().map(|e| e.effect.as_str()).collect()
    }

    #[test]
    fn dragging_a_curve_handle_and_picking_a_preset() {
        let ctx = egui::Context::default();
        let mut c = crate::effects::CURVE_PRESETS[3].2;
        let mut time = 0.0;
        // One frame; returns the handles' centers (the curve's small circles).
        let mut frame = |c: &mut [f32; 4], events: Vec<egui::Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(400.0, 300.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let mut out = ctx.run_ui(input, |ui| changed = curve_ui(ui, 1, c));
            out.textures_delta.clear();
            let handles: Vec<egui::Pos2> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Circle(k) if k.radius == 5.0 => Some(k.center),
                    _ => None,
                })
                .collect();
            (handles, changed)
        };
        let (handles, _) = frame(&mut c, vec![]);
        assert_eq!(handles.len(), 2);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        // Drag the first handle down by 30 points: its y grows, x stays.
        let (from, to) = (handles[0], handles[0] + vec2(0.0, 30.0));
        frame(&mut c, vec![egui::Event::PointerMoved(from)]);
        frame(&mut c, vec![button(from, true)]);
        let mut any = false;
        for k in 1..=3 {
            let p = from + (to - from) * (k as f32 / 3.0);
            any |= frame(&mut c, vec![egui::Event::PointerMoved(p)]).1;
        }
        frame(&mut c, vec![button(to, false)]);
        assert!(any, "reported as changed");
        assert!((c[0] - 0.42).abs() < 0.01, "{c:?}");
        let h = CURVE_SIZE.y - 2.0 * CURVE_MARGIN;
        assert!((c[1] - 30.0 / h).abs() < 0.02, "{c:?}");
        assert_eq!([c[2], c[3]], [0.58, 1.0]);
        // The first thumbnail sets the linear curve.
        let thumb = egui::pos2(18.0, 12.0);
        frame(&mut c, vec![egui::Event::PointerMoved(thumb)]);
        frame(&mut c, vec![button(thumb, true)]);
        frame(&mut c, vec![button(thumb, false)]);
        assert_eq!(c, crate::effects::CURVE_PRESETS[0].2);
    }

    /// The linear fade's parameters with a custom curve, to look at the curve editor:
    /// `CURVE_PREVIEW=out.png cargo test curve_preview -- --ignored`.
    #[test]
    #[ignore]
    fn curve_preview() {
        let Ok(path) = std::env::var("CURVE_PREVIEW") else {
            return;
        };
        let effect = registry().get("builtin.fade_linear").unwrap();
        let mut inst = EffectInstance::new(effect);
        inst.params.insert(
            "curve".into(),
            crate::effects::curve_value([0.1, 0.6, 0.3, 1.0]),
        );
        let inst = std::cell::RefCell::new(inst);
        crate::preview::render(&path, vec2(420.0, 300.0), |ui| {
            egui::Frame::NONE.inner_margin(12.0).show(ui, |ui| {
                params_ui(ui, (1, 0), effect, &mut inst.borrow_mut(), None);
            });
        });
    }

    #[test]
    fn move_effect_to_insertion_points() {
        let mut s = stack(&["a", "b", "c"]);
        assert!(move_effect(&mut s, 0, 3));
        assert_eq!(ids(&s), ["b", "c", "a"]);
        assert!(move_effect(&mut s, 2, 0));
        assert_eq!(ids(&s), ["a", "b", "c"]);
        assert!(move_effect(&mut s, 0, 2));
        assert_eq!(ids(&s), ["b", "a", "c"]);
        // Next to itself: nothing moves.
        assert!(!move_effect(&mut s, 1, 1) && !move_effect(&mut s, 1, 2));
        assert_eq!(ids(&s), ["b", "a", "c"]);
    }
}
