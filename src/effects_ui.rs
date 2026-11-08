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
        // Centered, every wrapped line too.
        ui.vertical_centered(|ui| {
            ui.weak(tr!("EFFECTS_INTRO"));
        });
        return changed;
    }
    if stack.len() > 1 {
        ui.weak(tr!("EFFECTS_ORDER_HINT"));
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
            let r = match in_mm(unit) {
                Some((k, hint)) => {
                    let k = f64::from(k.max(1e-6));
                    ui.add(
                        slider
                            .custom_formatter(move |n, _| format!("{:.2}", n * k))
                            .custom_parser(move |s| s.trim().parse::<f64>().ok().map(|mm| mm / k))
                            .suffix(" mm"),
                    )
                    .on_hover_text(hint)
                }
                None => ui.add(slider.suffix(unit).fixed_decimals(*decimals)),
            };
            r.changed().then(|| Value::from(v))
        }
        ParamKind::Int { min, max, unit, .. } => {
            let mut v = p.i32(id);
            ui.add(Slider::new(&mut v, *min..=*max).suffix(unit))
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
    }
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
