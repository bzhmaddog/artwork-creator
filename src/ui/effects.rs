//! The Effects section of a layer's properties: its effect stack, with controls built
//! from each effect's parameter list.

use egui::collapsing_header::CollapsingState;
use egui::{ComboBox, CursorIcon, DragAndDrop, RichText, Sense, Slider, Stroke, Ui, vec2};
use serde_json::Value;

use crate::i18n::{tr, tr_n, translate};
use crate::render::effects::plugin::{PluginInfo, Source};
use crate::render::effects::{
    Effect, EffectInstance, Generator, ParamKind, ParamSpec, Params, Role, color_to_hex,
    random_seed, registry,
};
use crate::ui::layers::{self, Tool};
use crate::ui::plugins::missing_name;

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
    layers::section(
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
    let button = layers::tool_icon(ui, Tool::Add, true, tr!("ADD_EFFECT"));
    egui::Popup::menu(&button).show(|ui| {
        let reg = registry();
        let mut add = |ui: &mut Ui, effect: &dyn Effect, plugin: Option<&PluginInfo>| {
            let mut r = ui.button(effect.name());
            let tip = entry_tip(effect.description(), plugin);
            if !tip.is_empty() {
                r = r.on_hover_text(tip);
            }
            if r.clicked() {
                added = Some(EffectInstance::new(effect).with_random_seeds());
                ui.close();
            }
        };
        for effect in reg.all().filter(|e| e.plugin().is_none()) {
            add(ui, effect, None);
        }
        // The plugins' effects, by source.
        let groups = by_source(|p| p.effects.iter().filter_map(|id| reg.get(id)).collect());
        for (source, plugins) in groups {
            source_header(ui, source);
            for (plugin, effects) in plugins {
                for effect in effects {
                    add(ui, effect, Some(plugin));
                }
            }
        }
        let errors = reg.errors();
        if !errors.is_empty() {
            ui.separator();
            let n = errors.len();
            let color = ui.visuals().warn_fg_color;
            let tip = errors.join("\n");
            ui.horizontal(|ui| {
                crate::ui::callout::warning_icon(ui, color).on_hover_text(&tip);
                ui.colored_label(color, tr_n!("PLUGIN_PROBLEM_COUNT", n))
                    .on_hover_text(&tip);
            });
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
        crate::ui::callout::callout(ui, crate::ui::callout::Tone::Info, tr!("EFFECTS_INTRO"));
        return changed;
    }
    if stack.len() > 1 {
        crate::ui::callout::callout(
            ui,
            crate::ui::callout::Tone::Info,
            tr!("EFFECTS_ORDER_HINT"),
        );
    }

    let mut remove = None;
    let mut drop = None;
    let dragged = DragAndDrop::payload::<DraggedEffect>(ui.ctx()).filter(|d| d.layer == layer_id);
    let full = ui.max_rect().x_range();
    // Area of each effect, header and controls, to find where a dragged one drops; in
    // the order shown.
    let mut blocks = Vec::with_capacity(stack.len());
    // As layers are: the last applied (the last of the stack) at the top.
    for (i, inst) in stack.iter_mut().enumerate().rev() {
        let effect = registry().get(&inst.effect);
        let id = ui.make_persistent_id(("effect", layer_id, i));
        let being_dragged = dragged.as_ref().is_some_and(|d| d.index == i);
        let block = ui.horizontal_top(|ui| {
            let handle = drag_handle(ui, being_dragged);
            handle.dnd_set_drag_payload(DraggedEffect {
                layer: layer_id,
                index: i,
            });
            let name = effect.map_or_else(|| missing_name(ui.ctx(), inst), |e| e.name().to_owned());
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
                        if effect.is_some() {
                            changed |= ui
                                .checkbox(&mut inst.enabled, "")
                                .on_hover_text(tr!("ENABLE_EFFECT"))
                                .changed();
                        } else {
                            // Shown off, but kept as saved for when the plugin is back.
                            ui.add_enabled(false, egui::Checkbox::new(&mut false, ""));
                        }
                        let (text, tip) = match effect {
                            Some(e) => {
                                let t = RichText::new(e.name());
                                let t = if inst.enabled { t } else { t.weak() };
                                (t, e.description().to_owned())
                            }
                            None => (
                                RichText::new(missing_name(ui.ctx(), inst))
                                    .color(ui.visuals().error_fg_color),
                                String::new(),
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
                                let old = std::mem::replace(inst, EffectInstance::new(e));
                                inst.enabled = old.enabled;
                                // The variation stays: only the settings go back.
                                for s in e.params().iter().filter(|s| s.seed_range().is_some()) {
                                    if let Some(v) = old.params.get(&s.id) {
                                        inst.params.insert(s.id.clone(), v.clone());
                                    }
                                }
                                changed = true;
                            }
                            if ui.button(tr!("REMOVE_EFFECT")).clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if layers::tool_button(ui, Tool::Delete, true, tr!("REMOVE_EFFECT")) {
                                remove = Some(i);
                            }
                            // A new random variation acts on the whole effect, so it
                            // goes with the effect's name, next to the trash.
                            if let Some(e) = effect {
                                let enabled = inst.enabled;
                                let tip = tr!("REGENERATE_EFFECT_HINT");
                                changed |= regenerate(ui, e.params(), inst, enabled, tip);
                            }
                        });
                    })
                    .body(|ui| {
                        if let Some(e) = effect {
                            ui.add_enabled_ui(inst.enabled, |ui| {
                                changed |= params_ui(ui, (layer_id, i), e, inst, px_mm);
                            });
                        } else {
                            let tone = crate::ui::callout::Tone::Error;
                            let mut note = None;
                            let text = tr!("MISSING_EFFECT");
                            crate::ui::callout::callout_with(ui, tone, text, |ui| {
                                if let Some(entry) = &inst.plugin {
                                    note = crate::ui::plugins::install_missing(
                                        ui,
                                        &inst.effect,
                                        entry,
                                    );
                                }
                            });
                            if let Some((tone, text)) = note {
                                crate::ui::callout::callout_small(ui, tone, &text);
                            }
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
        // The gap in the list shown, and where it is in the stack, shown reversed.
        let gap = blocks.iter().filter(|r| r.center().y < p.y).count();
        let to = blocks.len() - gap;
        // Next to itself nothing moves: no line.
        if to != d.index && to != d.index + 1 {
            let y = match gap {
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

    // A removal waits for confirmation (a misclick on the trash is quick): the effect's
    // index and id are kept here meanwhile, per layer.
    let confirm_id = egui::Id::unique(("confirm_remove_effect", layer_id));
    if let Some(i) = remove {
        let pending = (i, stack[i].effect.clone());
        ui.data_mut(|d| d.insert_temp(confirm_id, pending));
    }
    if let Some((i, effect_id)) = ui.data(|d| d.get_temp::<(usize, String)>(confirm_id)) {
        match stack.get(i).filter(|inst| inst.effect == effect_id) {
            Some(inst) => {
                let name = registry()
                    .get(&inst.effect)
                    .map_or_else(|| missing_name(ui.ctx(), inst), |e| e.name().to_owned());
                let (ok, cancel, close) = confirm_remove_dialog(ui.ctx(), &name);
                if ok || cancel || close {
                    ui.data_mut(|d| d.remove::<(usize, String)>(confirm_id));
                }
                if ok {
                    stack.remove(i);
                    changed = true;
                }
            }
            // The stack changed meanwhile (undo, another layer shown): nothing to ask.
            None => ui.data_mut(|d| d.remove::<(usize, String)>(confirm_id)),
        }
    }
    if let Some((from, to)) = drop {
        changed |= move_effect(stack, from, to);
    }
    changed
}

/// Asks whether to remove the effect `name`. Returns (remove, cancel, closed otherwise).
fn confirm_remove_dialog(ctx: &egui::Context, name: &str) -> (bool, bool, bool) {
    let mut answer = (false, false);
    let response = crate::app::dialog(ctx, "confirm_remove_effect").show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.strong(tr!("CONFIRM_REMOVE_EFFECT", name = name));
        ui.add_space(8.0);
        answer = crate::app::dialog_button_row(ui, |ui| {
            let ok = ui
                .add(crate::app::danger_button(tr!("REMOVE_EFFECT_OK")))
                .clicked();
            let cancel = ui.button(tr!("CANCEL")).clicked();
            (ok, cancel)
        });
    });
    (answer.0, answer.1, response.should_close())
}

/// Controls for an effect's parameters. Returns true if one changed.
fn params_ui(
    ui: &mut Ui,
    ui_key: (u64, usize),
    effect: &dyn Effect,
    inst: &mut EffectInstance,
    px_mm: Option<(f32, &str)>,
) -> bool {
    params_grid(
        ui,
        ui_key,
        effect.params(),
        effect.plugin().is_none(),
        inst,
        px_mm,
    )
}

/// Controls for the parameters `specs` of an effect or generator, `builtin` or from a
/// plugin, but for the seeds: their regenerate button is in the effect's header or next
/// to the generator menu (`regenerate`). Returns true if one changed.
fn params_grid(
    ui: &mut Ui,
    ui_key: (u64, usize),
    specs: &[ParamSpec],
    builtin: bool,
    inst: &mut EffectInstance,
    px_mm: Option<(f32, &str)>,
) -> bool {
    if specs.is_empty() {
        return false;
    }
    let shown: Vec<&ParamSpec> = specs.iter().filter(|s| s.seed_range().is_none()).collect();
    // The controls start at the same x in every grid, so they line up across groups: the
    // labels' column is as wide as the widest label, a grouped one counting its indent.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let indent = ui.spacing().indent;
    let label_w = shown
        .iter()
        .map(|s| {
            let text = label_text(builtin, &s.label).to_owned();
            let w = ui
                .painter()
                .layout_no_wrap(text, font.clone(), egui::Color32::PLACEHOLDER)
                .size()
                .x;
            if s.group.is_some() { w + indent } else { w }
        })
        .fold(0.0, f32::max);
    let mut changed = false;
    for (b, (group, rows)) in param_blocks(&shown).into_iter().enumerate() {
        let grid_key = (ui_key, b);
        match group {
            None => {
                changed |= param_rows(ui, grid_key, specs, &rows, builtin, inst, px_mm, label_w)
            }
            Some(name) => {
                let id = ui.make_persistent_id(("effect_group", ui_key, name));
                let w = label_w - indent;
                CollapsingState::load_with_default_open(ui.ctx(), id, true)
                    .show_header(ui, |ui| ui.weak(label_text(builtin, name)))
                    .body(|ui| {
                        changed |= param_rows(ui, grid_key, specs, &rows, builtin, inst, px_mm, w);
                    });
            }
        }
    }
    changed
}

/// The parameters `shown` in the order they come, as blocks: runs of ungrouped ones, and
/// each group, with all its parameters, where its first one is.
fn param_blocks<'a>(shown: &[&'a ParamSpec]) -> Vec<(Option<&'a str>, Vec<&'a ParamSpec>)> {
    let mut blocks: Vec<(Option<&str>, Vec<&ParamSpec>)> = Vec::new();
    for &s in shown {
        let group = s.group.as_deref();
        let i = match group {
            None => blocks
                .len()
                .checked_sub(1)
                .filter(|&i| blocks[i].0.is_none()),
            Some(_) => blocks.iter().position(|(g, _)| *g == group),
        };
        match i {
            Some(i) => blocks[i].1.push(s),
            None => blocks.push((group, vec![s])),
        }
    }
    blocks
}

/// A grid of the parameters `rows` of `specs`, its labels' column `label_w` wide.
/// Returns true if one changed.
#[allow(clippy::too_many_arguments)]
fn param_rows(
    ui: &mut Ui,
    grid_key: ((u64, usize), usize),
    specs: &[ParamSpec],
    rows: &[&ParamSpec],
    builtin: bool,
    inst: &mut EffectInstance,
    px_mm: Option<(f32, &str)>,
    label_w: f32,
) -> bool {
    let ui_key = grid_key.0;
    let mut changed = false;
    // Built-in labels are translated; plugins' are shown as they give them.
    egui::Grid::new(("effect_params", grid_key))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .min_col_width(label_w.max(0.0))
        .show(ui, |ui| {
            for &spec in rows {
                let params = Params::new(specs, &inst.params);
                ui.label(label_text(builtin, &spec.label));
                if let Some(v) =
                    param_ui(ui, ui_key, spec, builtin, &params, &mut inst.links, px_mm)
                {
                    inst.params.insert(spec.id.clone(), v);
                    changed = true;
                }
                ui.end_row();
            }
        });
    changed
}

/// The menu of generators, in a fill set to Generated, picking the one `inst` uses.
/// Returns true if it changed. `key` tells this fill from others.
pub fn generator_menu(ui: &mut Ui, key: u64, inst: &mut EffectInstance) -> bool {
    let reg = registry();
    let current = reg.generator(&inst.effect);
    let text = match current {
        Some(g) => RichText::new(g.name()),
        None => RichText::new(tr!("MISSING_GENERATOR")).color(ui.visuals().warn_fg_color),
    };
    let mut picked = None;
    let r = ComboBox::from_id_salt(("generator", key))
        .selected_text(text)
        .show_ui(ui, |ui| {
            let mut item = |ui: &mut Ui, g: &'static dyn Generator, plugin: Option<&PluginInfo>| {
                let mut r = ui.selectable_label(inst.effect == g.id(), g.name());
                let tip = entry_tip(g.description(), plugin);
                if !tip.is_empty() {
                    r = r.on_hover_text(tip);
                }
                if r.clicked() && inst.effect != g.id() {
                    picked = Some(g);
                }
            };
            for g in reg.generators().filter(|g| g.plugin().is_none()) {
                item(ui, g, None);
            }
            // The plugins' generators, by source.
            let groups = by_source(|p| {
                p.generators
                    .iter()
                    .filter_map(|id| reg.generator(id))
                    .collect()
            });
            for (source, plugins) in groups {
                source_header(ui, source);
                for (plugin, gens) in plugins {
                    for g in gens {
                        item(ui, g, Some(plugin));
                    }
                }
            }
        });
    if current.is_none() {
        r.response.on_hover_text(tr!(
            "MISSING_GENERATOR_HINT",
            name = missing_name(ui.ctx(), inst)
        ));
    }
    match picked {
        Some(g) => {
            *inst = EffectInstance::generator(g).with_random_seeds();
            true
        }
        None => false,
    }
}

/// A magic wand icon button with tooltip `tip`: a new random result, with the same
/// settings.
fn regenerate_icon(ui: &mut Ui, enabled: bool, tip: &str) -> egui::Response {
    let size = ui.spacing().interact_size.y;
    layers::flat_button(ui, vec2(size, size), enabled, tip, |p, rect, color| {
        let c = rect.center();
        let at = |x: f32, y: f32| c + size * vec2(x, y);
        let stroke = Stroke::new(crate::ui::tokens::STROKE, color);
        // The wand, from the bottom left, with a lighter tip under the star.
        p.line_segment([at(-0.3, 0.3), at(0.02, -0.02)], Stroke::new(2.4, color));
        p.line_segment([at(0.02, -0.02), at(0.1, -0.1)], stroke);
        // A four-pointed star at its tip...
        let star = at(0.2, -0.2);
        let (outer, inner) = (size * 0.15, size * 0.04);
        let points = (0..WAND_STAR_POINTS)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / WAND_STAR_POINTS as f32;
                let r = if i % 2 == 0 { outer } else { inner };
                star + r * vec2(a.sin(), -a.cos())
            })
            .collect();
        p.add(egui::Shape::closed_line(points, stroke));
        // ...and two sparkles.
        for (x, y, r) in [(-0.12, -0.28, 0.06), (0.3, 0.1, 0.05)] {
            let (s, r) = (at(x, y), size * r);
            p.line_segment([s - vec2(r, 0.0), s + vec2(r, 0.0)], stroke);
            p.line_segment([s - vec2(0.0, r), s + vec2(0.0, r)], stroke);
        }
    })
}

/// The points of the wand's star, around it: its tips and the corners between them.
pub(crate) const WAND_STAR_POINTS: usize = 8;

/// A magic wand button giving `inst` a new random seed, for generators that have
/// one. Returns true when clicked.
pub fn regenerate_button(ui: &mut Ui, inst: &mut EffectInstance) -> bool {
    let Some(g) = registry().generator(&inst.effect) else {
        return false;
    };
    regenerate(ui, g.params(), inst, true, tr!("REGENERATE_FILL_HINT"))
}

/// A magic wand button giving each seed of `specs` (see [`ParamSpec::seed_range`])
/// a new random value in `inst`, with tooltip `tip`; nothing when there is none.
/// Returns true when clicked.
fn regenerate(
    ui: &mut Ui,
    specs: &[ParamSpec],
    inst: &mut EffectInstance,
    enabled: bool,
    tip: &str,
) -> bool {
    let seeds: Vec<_> = specs
        .iter()
        .filter_map(|s| Some((s, s.seed_range()?)))
        .collect();
    if seeds.is_empty() || !regenerate_icon(ui, enabled, tip).clicked() {
        return false;
    }
    for (spec, (min, max)) in seeds {
        let old = Params::new(specs, &inst.params).i32(&spec.id);
        let new = random_seed(min, max, Some(old));
        inst.params.insert(spec.id.clone(), Value::from(new));
    }
    true
}

/// The settings of the generator `inst` uses, under its menu. Returns true if one
/// changed.
pub fn generator_params(ui: &mut Ui, key: u64, inst: &mut EffectInstance) -> bool {
    let Some(g) = registry().generator(&inst.effect) else {
        // Its plugin's Install button, as a missing effect's.
        if let Some(entry) = &inst.plugin
            && let Some((tone, text)) = crate::ui::plugins::install_missing(ui, &inst.effect, entry)
        {
            crate::ui::callout::callout_small(ui, tone, &text);
        }
        return false;
    };
    params_grid(
        ui,
        (key, usize::MAX),
        g.params(),
        g.plugin().is_none(),
        inst,
        None,
    )
}

/// A parameter or option label, translated for a built-in effect.
fn label_text(builtin: bool, text: &str) -> &str {
    if builtin { translate(text) } else { text }
}

/// One parameter's control, a color one linked through `links`. Returns the new value
/// if it changed.
fn param_ui(
    ui: &mut Ui,
    ui_key: (u64, usize),
    spec: &ParamSpec,
    builtin: bool,
    p: &Params,
    links: &mut std::collections::BTreeMap<String, String>,
    px_mm: Option<(f32, &str)>,
) -> Option<Value> {
    let id = spec.id.as_str();
    let role = spec.role();
    // Sizes shown in mm on the page, when asked.
    let in_mm = px_mm.filter(|_| role == Some(Role::Size));
    match &spec.kind {
        ParamKind::Float {
            min,
            max,
            default,
            unit,
            decimals,
        } => {
            let mut v = p.f32(id);
            // Sizes span a wide range where small values matter most: logarithmic, the
            // slider moves in fine steps there.
            let log = role == Some(Role::Size);
            let r = ui
                .horizontal(|ui| match in_mm {
                    Some((k, hint)) => {
                        let k = f64::from(k.max(1e-6));
                        let r = ui
                            .add(
                                Slider::new(&mut v, *min..=*max)
                                    .logarithmic(log)
                                    .custom_formatter(move |n, _| format!("{:.2}", n * k))
                                    .custom_parser(move |s| {
                                        s.trim().parse::<f64>().ok().map(|mm| mm / k)
                                    }),
                            )
                            .on_hover_text(hint);
                        layers::unit(ui, "mm");
                        r
                    }
                    // An angle: its value, and a dial to turn it, counterclockwise as
                    // effects take angles.
                    None if role == Some(Role::Angle) => {
                        let r = ui
                            .add(
                                egui::DragValue::new(&mut v)
                                    .range(*min..=*max)
                                    .speed(0.0)
                                    .fixed_decimals(*decimals),
                            )
                            .on_hover_cursor(egui::CursorIcon::Text);
                        layers::unit(ui, if unit.is_empty() { "°" } else { unit });
                        let mut a = v;
                        let dial = crate::ui::dial::angle_dial(
                            ui,
                            &mut a,
                            crate::ui::dial::Turn::FromRight,
                            *default,
                        );
                        if dial.changed() {
                            v = a.clamp(*min, *max);
                        }
                        r | dial
                    }
                    None => {
                        let r = ui.add(
                            Slider::new(&mut v, *min..=*max)
                                .logarithmic(log)
                                .fixed_decimals(*decimals),
                        );
                        layers::unit(ui, unit);
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
                layers::unit(ui, unit);
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
            crate::ui::colors::linked_button(ui, links, spec.id.clone(), &mut v, true)
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
            ComboBox::from_id_salt(("effect_choice", ui_key, id))
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
            curve_ui(ui, (ui_key, id), &mut c).then(|| crate::render::effects::curve_value(c))
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
    ui_key: impl std::hash::Hash + std::fmt::Debug + Copy,
    c: &mut [f32; 4],
) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        // The usual curves, each a thumbnail of itself, named in its tooltip.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (_, key, preset) in crate::render::effects::CURVE_PRESETS {
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
                painter.rect(
                    rect,
                    crate::ui::tokens::RADIUS,
                    fill,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                let ink = if on {
                    visuals.selection.stroke.color
                } else {
                    visuals.text_color()
                };
                let b = rect.shrink2(vec2(7.0, 5.0));
                let points: Vec<egui::Pos2> = (0..=16)
                    .map(|i| {
                        let x = i as f32 / 16.0;
                        let y = crate::render::effects::ease(preset, x);
                        b.left_top() + vec2(x * b.width(), y * b.height())
                    })
                    .collect();
                painter.add(egui::Shape::line(
                    points,
                    Stroke::new(crate::ui::tokens::STROKE, ink),
                ));
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
                ui.make_persistent_id((ui_key, "curve_handle", k)),
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
            painter.circle(p, 5.0, fill, Stroke::new(crate::ui::tokens::STROKE, accent));
        }
        bg.on_hover_text(tr!("FADE_CURVE_HINT"));
    });
    changed
}

/// What `items` keeps of each plugin loaded, those with something kept only, grouped by
/// source: the plugins put in a plugins folder by hand first, then each repository's, in
/// loading order.
#[allow(clippy::type_complexity)]
fn by_source<T>(
    items: impl Fn(&'static PluginInfo) -> Vec<T>,
) -> Vec<(Option<&'static Source>, Vec<(&'static PluginInfo, Vec<T>)>)> {
    let mut groups: Vec<(Option<&Source>, Vec<_>)> = Vec::new();
    for plugin in registry().plugins() {
        let kept = items(plugin);
        if kept.is_empty() {
            continue;
        }
        let source = plugin.source.as_ref();
        let origin = source.map(|s| &s.origin);
        match groups
            .iter_mut()
            .find(|(s, _)| s.map(|s| &s.origin) == origin)
        {
            Some((_, plugins)) => plugins.push((plugin, kept)),
            None => groups.push((source, vec![(plugin, kept)])),
        }
    }
    groups.sort_by_key(|(s, _)| s.is_some());
    groups
}

/// The title of a group of plugins of a menu: their repository's name (its address in
/// the tooltip), or "Local plugins".
fn source_header(ui: &mut Ui, source: Option<&Source>) {
    ui.separator();
    match source {
        Some(s) => {
            let r = if s.name.is_empty() {
                ui.strong(tr!("UNKNOWN_REPOSITORY"))
            } else {
                ui.strong(&s.name)
            };
            if !s.url.is_empty() {
                r.on_hover_text(&s.url);
            }
        }
        None => {
            ui.strong(tr!("LOCAL_PLUGINS"))
                .on_hover_text(tr!("LOCAL_PLUGINS_HINT"));
        }
    }
}

/// The tooltip of an effect's or generator's menu entry: its description, then the
/// name and version of its plugin, if any.
fn entry_tip(description: &str, plugin: Option<&PluginInfo>) -> String {
    let plugin = plugin.map(|p| format!("{} {}", p.name, p.version));
    let lines = [Some(description.to_owned()), plugin];
    let lines: Vec<_> = lines
        .into_iter()
        .flatten()
        .filter(|l| !l.trim().is_empty())
        .collect();
    lines.join("\n").trim_end().to_owned()
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
                plugin: None,
                name: None,
            })
            .collect()
    }

    fn ids(s: &[EffectInstance]) -> Vec<&str> {
        s.iter().map(|e| e.effect.as_str()).collect()
    }

    #[test]
    fn dragging_a_curve_handle_and_picking_a_preset() {
        let ctx = egui::Context::default();
        let mut c = crate::render::effects::CURVE_PRESETS[3].2;
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
        assert_eq!(c, crate::render::effects::CURVE_PRESETS[0].2);
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
            crate::render::effects::curve_value([0.1, 0.6, 0.3, 1.0]),
        );
        let inst = std::cell::RefCell::new(inst);
        crate::ui::preview::render(&path, vec2(420.0, 300.0), |ui| {
            egui::Frame::NONE.inner_margin(12.0).show(ui, |ui| {
                params_ui(ui, (1, 0), effect, &mut inst.borrow_mut(), None);
            });
        });
    }

    #[test]
    fn an_effect_seed_is_a_regenerate_button_not_a_setting() {
        let specs = [
            ParamSpec::int("amount", "Amount", 0, 100, 20),
            ParamSpec::int("variation", "Variation", 0, 9999, 0).with_role(Role::Seed),
        ];
        let mut inst = stack(&["plugin.grain"]).remove(0);
        let ctx = egui::Context::default();
        let mut time = 0.0;
        // One frame; returns its texts, the arrow's center and whether a setting changed.
        let mut frame = |inst: &mut EffectInstance, events: Vec<egui::Event>| {
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
            let mut out = ctx.run_ui(input, |ui| {
                changed = params_grid(ui, (1, 0), &specs, false, inst, None);
                changed |= regenerate(ui, &specs, inst, true, "");
            });
            out.textures_delta.clear();
            let texts: Vec<String> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
            let arrow = out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Path(p) if p.closed && p.points.len() == WAND_STAR_POINTS => {
                    Some(egui::Rect::from_points(&p.points).center())
                }
                _ => None,
            });
            (texts, arrow, changed)
        };
        let (texts, arrow, _) = frame(&mut inst, vec![]);
        assert!(texts.iter().any(|t| t == "Amount"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Variation"), "{texts:?}");
        let arrow = arrow.expect("the magic wand");
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let seed = |inst: &EffectInstance| inst.params.get("variation").cloned();
        let mut click = |inst: &mut EffectInstance| {
            frame(inst, vec![egui::Event::PointerMoved(arrow)]);
            frame(inst, vec![button(arrow, true)]);
            frame(inst, vec![button(arrow, false)]).2
        };
        let before = seed(&inst);
        assert!(click(&mut inst));
        let after = seed(&inst);
        assert_ne!(before, after);
        assert!(click(&mut inst));
        assert_ne!(seed(&inst), after);
    }

    /// `spec` in the group `name`.
    fn grouped(spec: ParamSpec, name: &str) -> ParamSpec {
        ParamSpec {
            group: Some(name.into()),
            ..spec
        }
    }

    #[test]
    fn grouped_params_are_together_where_the_first_is() {
        let specs = [
            ParamSpec::int("a", "A", 0, 9, 0),
            grouped(ParamSpec::int("x", "X", 0, 9, 0), "G"),
            ParamSpec::int("b", "B", 0, 9, 0),
            grouped(ParamSpec::int("y", "Y", 0, 9, 0), "G"),
            ParamSpec::int("c", "C", 0, 9, 0),
            grouped(ParamSpec::int("z", "Z", 0, 9, 0), "H"),
        ];
        let shown: Vec<_> = specs.iter().collect();
        let blocks: Vec<(Option<&str>, Vec<&str>)> = param_blocks(&shown)
            .into_iter()
            .map(|(g, rows)| (g, rows.iter().map(|s| s.id.as_str()).collect()))
            .collect();
        assert_eq!(
            blocks,
            [
                (None, vec!["a"]),
                (Some("G"), vec!["x", "y"]),
                (None, vec!["b", "c"]),
                (Some("H"), vec!["z"]),
            ]
        );
    }

    #[test]
    fn a_group_folds_its_params_away() {
        let specs = [
            ParamSpec::int("amount", "Amount", 0, 100, 20),
            grouped(ParamSpec::int("size", "Grain size", 0, 9, 1), "Grain"),
        ];
        let mut inst = stack(&["plugin.grain"]).remove(0);
        let ctx = egui::Context::default();
        let mut time = 0.0;
        // One frame; returns its texts and where "Grain" is.
        let mut frame = |inst: &mut EffectInstance, events: Vec<egui::Event>| {
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
            let mut out = ctx.run_ui(input, |ui| {
                params_grid(ui, (1, 0), &specs, false, inst, None);
            });
            out.textures_delta.clear();
            let mut header = None;
            let texts: Vec<String> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => {
                        let text = t.galley.text().to_owned();
                        if text == "Grain" {
                            header = Some(t.visual_bounding_rect().center());
                        }
                        Some(text)
                    }
                    _ => None,
                })
                .collect();
            (texts, header)
        };
        let (texts, header) = frame(&mut inst, vec![]);
        assert!(
            texts.iter().any(|t| t == "Grain size"),
            "open at first: {texts:?}"
        );
        // The fold arrow is at the left of the title.
        let arrow = header.expect("the group's title") - vec2(30.0, 0.0);
        let arrow = egui::pos2(arrow.x.max(6.0), arrow.y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut inst, vec![egui::Event::PointerMoved(arrow)]);
        frame(&mut inst, vec![button(arrow, true)]);
        frame(&mut inst, vec![button(arrow, false)]);
        // Let the folding animation end.
        for _ in 0..10 {
            frame(&mut inst, vec![]);
        }
        let (texts, _) = frame(&mut inst, vec![]);
        assert!(texts.iter().any(|t| t == "Amount"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "Grain"), "{texts:?}");
        assert!(
            !texts.iter().any(|t| t == "Grain size"),
            "folded: {texts:?}"
        );
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
