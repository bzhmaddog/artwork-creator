//! The project's colors, to reuse them: those in use, gathered from the document, and
//! its color variables (see `variables`), each with its own name.
//!
//! Every color button of the app is [`button`]: its picker shows both lists as swatches,
//! and a click on one applies it. A click on a color variable also links the button's
//! color to it, by name, like a field linked to a variable: each frame, [`apply`] gives
//! every linked color its variable's value, so changing a color variable changes all the
//! colors using it. A color changed some other way in the picker is unlinked. The app
//! publishes the lists at the start of each frame ([`publish`]), so the buttons need no
//! extra arguments; a color saved from a picker is handed back the same way
//! ([`take_saved`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::color_picker::{Alpha, color_picker_hsva_2d, show_color_at};
use egui::ecolor::Hsva;
use egui::{Color32, Id, Popup, PopupCloseBehavior, Response, RichText, Sense, Ui, vec2};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Node};
use crate::effects::{ParamKind, Params, color_from_hex, color_to_hex, registry};
use crate::i18n::tr;
use crate::variables::{self, Kind, Value, Variable};

/// A saved color of the first projects that had them, before color variables: read,
/// then made a color variable ([`migrate_palette`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OldSwatch {
    /// Straight RGBA.
    pub color: [u8; 4],
    #[serde(default)]
    pub name: String,
}

/// A layer's color that can be linked to a color variable. An effect's color parameters
/// are linked in the effect itself (`EffectInstance::links`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorField {
    /// A text's color.
    Text,
    /// A text's outline color.
    TextOutline,
    /// A shape's fill.
    Fill,
    /// A shape's stroke.
    Stroke,
}

/// Names of the color variables a layer's colors are linked to.
pub type Links = BTreeMap<ColorField, String>;

/// Side of a swatch in the pickers and the Colors section, in points.
const SWATCH: f32 = 18.0;
/// Swatches per row in a picker.
const PER_ROW: usize = 12;

/// Every color the project uses, once each, in stack order: the page background, then
/// for each layer from the bottom its text, outline, fill and stroke colors and those of
/// its effects. A gradient counts with the colors of its stops. Colors switched off (no
/// fill, no outline) don't count.
pub fn used(doc: &Document) -> Vec<[u8; 4]> {
    let mut colors = Vec::new();
    let mut add = |c: [u8; 4]| {
        if !colors.contains(&c) {
            colors.push(c);
        }
    };
    // One color, or a gradient's stops.
    fn either(add: &mut dyn FnMut([u8; 4]), c: [u8; 4], g: Option<&crate::gradient::Gradient>) {
        match g {
            Some(g) => g.sorted().iter().for_each(|s| add(s.color)),
            None => add(c),
        }
    }
    let [r, g, b] = doc.background;
    // The page is opaque, and so are its gradient's colors.
    let bg = doc.background_gradient.as_ref().map(|g| {
        let mut g = g.clone();
        g.stops.iter_mut().for_each(|s| s.color[3] = 255);
        g
    });
    either(&mut add, [r, g, b, 255], bg.as_ref());
    fn walk(nodes: &[Node], add: &mut dyn FnMut([u8; 4])) {
        for node in nodes {
            match node {
                Node::Group(g) => walk(&g.children, add),
                Node::Layer(l) => {
                    if let Some(t) = &l.text {
                        either(add, t.color, t.gradient.as_ref());
                        if t.outline {
                            add(t.outline_color);
                        }
                    }
                    if let Some(s) = &l.shape {
                        if s.fill && s.kind.can_fill() {
                            either(add, s.fill_color, s.fill_gradient.as_ref());
                        }
                        if s.stroke {
                            either(add, s.stroke_color, s.stroke_gradient.as_ref());
                        }
                    }
                    for fx in &l.effects {
                        let Some(effect) = registry().get(&fx.effect) else {
                            continue;
                        };
                        let params = Params::new(effect.params(), &fx.params);
                        for spec in effect.params() {
                            if let ParamKind::Color { .. } = spec.kind {
                                add(params.color(&spec.id));
                            }
                        }
                    }
                }
            }
        }
    }
    walk(&doc.layers, &mut add);
    colors
}

/// A color for people to read: `#RRGGBB`, then its opacity (`#000032, 50% opacity`).
pub fn describe(c: [u8; 4]) -> String {
    let [r, g, b, a] = c;
    format!(
        "#{r:02X}{g:02X}{b:02X}, {}",
        tr!("OPACITY_PERCENT", n = opacity_percent(a))
    )
}

/// An alpha as a percentage of opacity, rounded.
fn opacity_percent(a: u8) -> u8 {
    (a as f32 / 2.55).round() as u8
}

/// What the color pickers show this frame.
#[derive(Clone, Default)]
struct Swatches {
    /// All the variables: the colors to show, the names to avoid for a new one.
    vars: Vec<Variable>,
    used: Vec<[u8; 4]>,
}

impl Swatches {
    /// The color variables' names and colors.
    fn saved(&self) -> impl Iterator<Item = (&str, [u8; 4])> {
        self.vars
            .iter()
            .filter_map(|v| Some((v.name.as_str(), v.value.color()?)))
    }
}

fn swatches_id() -> Id {
    Id::unique("project_colors")
}

fn saved_id() -> Id {
    Id::unique("project_colors_saved")
}

/// Makes the project's colors available to the color buttons drawn this frame.
pub fn publish(ctx: &egui::Context, doc: &Document) {
    let swatches = Arc::new(Swatches {
        vars: doc.variables.clone(),
        used: used(doc),
    });
    ctx.data_mut(|d| d.insert_temp(swatches_id(), swatches));
}

/// Color variables made from a picker's "Save this color" button since the last call.
pub fn take_saved(ctx: &egui::Context) -> Vec<Variable> {
    ctx.data_mut(|d| d.remove_temp::<Vec<Variable>>(saved_id()))
        .unwrap_or_default()
}

/// Adds the color variable `var` unless a color variable has its color or a variable
/// its name already. Returns true if it was added.
pub fn save(doc: &mut Document, var: Variable) -> bool {
    let color = var.value.color();
    if color.is_none()
        || doc
            .variables
            .iter()
            .any(|v| v.name == var.name || (v.value.color().is_some() && v.value.color() == color))
    {
        return false;
    }
    doc.variables.push(var);
    true
}

/// The color variable `name`'s color, if there is one.
fn find(vars: &[Variable], name: &str) -> Option<[u8; 4]> {
    vars.iter().find(|v| v.name == name)?.value.color()
}

/// Makes the saved colors of a project from before color variables color variables.
/// A name that can't be a variable's (such as "Color 1") or is taken becomes one that
/// can ("Color_1"), in the links too.
pub fn migrate_palette(doc: &mut Document) {
    let old = std::mem::take(&mut doc.old_palette);
    if old.is_empty() {
        return;
    }
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    for swatch in old {
        let mut base: String = swatch
            .name
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if !base
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        {
            base.insert(0, '_');
        }
        let free = |n: &str| doc.variables.iter().all(|v| v.name != n);
        let name = if base.chars().all(|c| c == '_') {
            variables::new_name(&doc.variables, Kind::Color)
        } else if free(&base) {
            base
        } else {
            (2..)
                .map(|n| format!("{base}_{n}"))
                .find(|n| free(n))
                .expect("an unused name")
        };
        if !swatch.name.is_empty() && !renamed.contains_key(&swatch.name) {
            renamed.insert(swatch.name, name.clone());
        }
        doc.variables.push(Variable {
            name,
            value: Value::Color(swatch.color),
        });
    }
    for_each_link(doc, |link| match renamed.get(link.as_str()) {
        Some(new) => {
            *link = new.clone();
            true
        }
        None => false,
    });
}

/// Gives every linked color its color variable's value. A color whose variable is gone
/// (or no longer a color) keeps its link, broken, and its last value: its button shows
/// a broken link icon, and a color variable of that name links it again. Links to a
/// color the layer doesn't have are dropped. Returns true if the document changed.
pub fn apply(doc: &mut Document) -> bool {
    let vars = std::mem::take(&mut doc.variables);
    let find = |name: &str| find(&vars, name);
    let mut changed = false;
    if let Some([r, g, b, _]) = doc.background_link.as_deref().and_then(find) {
        changed |= doc.background != [r, g, b];
        doc.background = [r, g, b];
    }
    // A gradient's stops, each linked on its own.
    let mut stops = |g: &mut crate::gradient::Gradient| {
        for stop in &mut g.stops {
            if let Some(c) = stop.link.as_deref().and_then(find) {
                changed |= stop.color != c;
                stop.color = c;
            }
        }
    };
    if let Some(g) = &mut doc.background_gradient {
        stops(g);
    }
    doc.for_each_layer_mut(|layer| layer.gradients_mut().into_iter().for_each(&mut stops));
    doc.for_each_layer_mut(|layer| {
        let mut links = std::mem::take(&mut layer.color_links);
        links.retain(|&field, name| {
            let Some(slot) = slot(layer, field) else {
                changed = true;
                return false;
            };
            if let Some(c) = find(name) {
                changed |= *slot != c;
                *slot = c;
            }
            true
        });
        layer.color_links = links;
        for fx in &mut layer.effects {
            let params = &mut fx.params;
            fx.links.retain(|param, name| {
                let Some(c) = find(name) else {
                    return true;
                };
                let now = params
                    .get(param)
                    .and_then(|v| v.as_str())
                    .and_then(color_from_hex);
                if now != Some(c) {
                    params.insert(param.clone(), color_to_hex(c).into());
                    changed = true;
                }
                true
            });
        }
    });
    doc.variables = vars;
    changed
}

/// Fits the color links of `node`, pasted from the clipboard, to the variables `vars`,
/// which may not be those of the project it was copied from. A link stays when there
/// is a color variable of that name and value; it moves to the color variable of that
/// value when there is one; otherwise its color is saved as a variable of its name. A
/// link whose name another variable has is dropped, its color kept as it is.
pub fn adopt_links(vars: &mut Vec<Variable>, node: &mut Node) {
    let mut fit = |name: &mut String, c: [u8; 4]| {
        if find(vars, name) == Some(c) {
            return true;
        }
        if let Some(v) = vars.iter().find(|v| v.value.color() == Some(c)) {
            *name = v.name.clone();
            return true;
        }
        if vars.iter().any(|v| v.name == *name) {
            return false;
        }
        vars.push(Variable {
            name: name.clone(),
            value: Value::Color(c),
        });
        true
    };
    node.for_each_layer_mut(&mut |l| {
        for g in l.gradients_mut() {
            for stop in &mut g.stops {
                if let Some(name) = &mut stop.link
                    && !fit(name, stop.color)
                {
                    stop.link = None;
                }
            }
        }
        let mut links = std::mem::take(&mut l.color_links);
        links.retain(|&field, name| match slot(l, field) {
            Some(c) => {
                let c = *c;
                fit(name, c)
            }
            None => false,
        });
        l.color_links = links;
        for fx in &mut l.effects {
            let params = &fx.params;
            fx.links.retain(|param, name| {
                match params
                    .get(param)
                    .and_then(|v| v.as_str())
                    .and_then(color_from_hex)
                {
                    Some(c) => fit(name, c),
                    None => false,
                }
            });
        }
    });
}

/// Where a layer keeps the color `field`, if it has it.
fn slot(layer: &mut crate::document::Layer, field: ColorField) -> Option<&mut [u8; 4]> {
    match field {
        ColorField::Text => layer.text.as_deref_mut().map(|t| &mut t.color),
        ColorField::TextOutline => layer.text.as_deref_mut().map(|t| &mut t.outline_color),
        ColorField::Fill => layer.shape.as_deref_mut().map(|s| &mut s.fill_color),
        ColorField::Stroke => layer.shape.as_deref_mut().map(|s| &mut s.stroke_color),
    }
}

/// Every link to a color variable in the document: the page background's, the layers'
/// and their effects', and their gradients' stops'. A link `f` returns false for is
/// removed.
fn for_each_link(doc: &mut Document, mut f: impl FnMut(&mut String) -> bool) {
    if let Some(name) = &mut doc.background_link
        && !f(name)
    {
        doc.background_link = None;
    }
    let stops = |g: &mut crate::gradient::Gradient, f: &mut dyn FnMut(&mut String) -> bool| {
        for stop in &mut g.stops {
            if let Some(name) = &mut stop.link
                && !f(name)
            {
                stop.link = None;
            }
        }
    };
    if let Some(g) = &mut doc.background_gradient {
        stops(g, &mut f);
    }
    doc.for_each_layer_mut(|l| {
        for g in l.gradients_mut() {
            stops(g, &mut f);
        }
        l.color_links.retain(|_, name| f(name));
        for fx in &mut l.effects {
            fx.links.retain(|_, name| f(name));
        }
    });
}

/// How many colors are linked to the variable `name`.
pub fn use_count(doc: &mut Document, name: &str) -> usize {
    let mut n = 0;
    for_each_link(doc, |link| {
        n += (link == name) as usize;
        true
    });
    n
}

/// Links to the variable `old` go to `new`.
pub fn rename_links(doc: &mut Document, old: &str, new: &str) {
    for_each_link(doc, |link| {
        if link == old {
            *link = new.to_owned();
        }
        true
    });
}

/// Draws a chain inside the swatch `rect` of `color`, black or white, whichever stands
/// out from the color, over a halo of the other so it shows on any color; `broken`, a
/// broken chain.
fn paint_chain(painter: &egui::Painter, rect: egui::Rect, color: [u8; 4], broken: bool) {
    let [r, g, b, a] = color;
    let over = Color32::from_rgba_unmultiplied(r, g, b, a);
    let (ink, halo) = if over.intensity() < 0.55 && a > 96 {
        (Color32::WHITE, Color32::from_black_alpha(140))
    } else {
        (Color32::BLACK, Color32::from_white_alpha(140))
    };
    let icon = rect.shrink(1.0);
    variables::paint_link_stroke(painter, icon, egui::Stroke::new(3.0, halo), broken);
    variables::paint_link_stroke(painter, icon, egui::Stroke::new(1.4, ink), broken);
}

/// A swatch of `color`, clickable. The selected one gets a thick frame; the one a color
/// is `linked` to, a chain inside too.
pub fn swatch(ui: &mut Ui, color: [u8; 4], selected: bool, linked: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
    if ui.is_rect_visible(rect) {
        let [r, g, b, a] = color;
        show_color_at(
            ui.painter(),
            Color32::from_rgba_unmultiplied(r, g, b, a),
            rect,
        );
        let visuals = ui.style().interact_selectable(&response, selected);
        let width = if selected { 2.0 } else { 1.0 };
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(width, visuals.fg_stroke.color),
            egui::StrokeKind::Inside,
        );
        if linked {
            paint_chain(ui.painter(), rect, color, false);
        }
    }
    response
}

/// A grid of swatches, each a color, whether it is selected (the one linked to, for
/// color variables, which shows a chain) and its tooltip; returns the index of the one
/// clicked.
fn swatch_grid(ui: &mut Ui, swatches: &[([u8; 4], bool, String)], links: bool) -> Option<usize> {
    let mut picked = None;
    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
    for (r, row) in swatches.chunks(PER_ROW).enumerate() {
        ui.horizontal(|ui| {
            for (i, (c, selected, tip)) in row.iter().enumerate() {
                let linked = links && *selected;
                if swatch(ui, *c, *selected, linked)
                    .on_hover_text(tip)
                    .clicked()
                {
                    picked = Some(r * PER_ROW + i);
                }
            }
        });
    }
    picked
}

/// [`button`] for a color whose link is kept in `links` under `key`.
pub fn linked_button<K: Ord>(
    ui: &mut Ui,
    links: &mut BTreeMap<K, String>,
    key: K,
    color: &mut [u8; 4],
    alpha: bool,
) -> Response {
    let mut link = links.remove(&key);
    let response = button(ui, color, alpha, &mut link);
    if let Some(name) = link {
        links.insert(key, name);
    }
    response
}

/// A color button whose picker also shows the project's color variables and used
/// colors. `color` is straight RGBA; with `alpha` false it stays opaque. `link` is the
/// name of the color variable it is linked to: a click on a color variable links it,
/// any other change unlinks it. The button is a square swatch; linked, a chain link is
/// drawn inside it (the variable's name is in its tooltip); linked to a variable that
/// no longer exists, a broken chain.
pub fn button(
    ui: &mut Ui,
    color: &mut [u8; 4],
    alpha: bool,
    link: &mut Option<String>,
) -> Response {
    let popup_id = ui.auto_id_with("color_popup");
    let open = Popup::is_id_open(ui.ctx(), popup_id);
    // Square, as tall as a field.
    let side = ui.spacing().interact_size.y;
    let size = vec2(side, side);
    let broken = link.as_deref().is_some_and(|name| {
        let swatches = ui.data(|d| d.get_temp::<Arc<Swatches>>(swatches_id()));
        !swatches.is_some_and(|s| s.saved().any(|(n, _)| n == name))
    });
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = if open {
            ui.visuals().widgets.open
        } else {
            *ui.style().interact(&response)
        };
        let rect = rect.expand(visuals.expansion);
        let [r, g, b, a] = *color;
        show_color_at(
            ui.painter(),
            Color32::from_rgba_unmultiplied(r, g, b, a),
            rect,
        );
        let radius = visuals.corner_radius.at_most(2);
        ui.painter()
            .rect_stroke(rect, radius, visuals.bg_stroke, egui::StrokeKind::Inside);
        if link.is_some() {
            paint_chain(ui.painter(), rect, *color, broken);
        }
    }
    // The color, with its opacity, then its link if any.
    response = response.on_hover_text(match link {
        Some(name) if broken => {
            format!(
                "{}\n{}",
                describe(*color),
                tr!("BROKEN_COLOR_LINK_HINT", name = name)
            )
        }
        Some(name) => format!(
            "{}\n{}",
            describe(*color),
            tr!("COLOR_LINKED_HINT", name = name)
        ),
        None => describe(*color),
    });

    // The picker's hue and saturation, kept so they survive a gray or black color.
    let hsva_id = popup_id.with("hsva");
    Popup::menu(&response)
        .id(popup_id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = 275.0;
            let cached = ui
                .data(|d| d.get_temp::<([u8; 4], Hsva)>(hsva_id))
                .filter(|(c, _)| c == color);
            let mut hsva = cached.map_or_else(
                || {
                    let [r, g, b, a] = *color;
                    Hsva::from_srgba_unmultiplied([r, g, b, a])
                },
                |(_, h)| h,
            );
            let mode = if alpha {
                Alpha::OnlyBlend
            } else {
                Alpha::Opaque
            };
            if color_picker_hsva_2d(ui, &mut hsva, mode) {
                *color = hsva.to_srgba_unmultiplied();
                *link = None;
                response.mark_changed();
            }
            // The values the sliders don't show: the code, to type or copy, and the
            // opacity in %.
            if code_fields(ui, popup_id, color, alpha) {
                hsva = Hsva::from_srgba_unmultiplied(*color);
                *link = None;
                response.mark_changed();
            }

            let swatches = ui
                .data(|d| d.get_temp::<Arc<Swatches>>(swatches_id()))
                .unwrap_or_default();
            let mut picked = None;
            // The colors in use, right below the sliders.
            ui.add_space(4.0);
            let grid: Vec<_> = swatches
                .used
                .iter()
                .map(|&c| (c, link.is_none() && c == *color, describe(c)))
                .collect();
            if let Some(i) = swatch_grid(ui, &grid, false) {
                picked = Some((swatches.used[i], None));
            }
            ui.separator();
            ui.label(RichText::new(tr!("COLOR_VARIABLES")).small());
            let saved: Vec<(&str, [u8; 4])> = swatches.saved().collect();
            if saved.is_empty() {
                crate::callout::callout_small(
                    ui,
                    crate::callout::Tone::Info,
                    tr!("COLOR_VARIABLES_EMPTY"),
                );
            } else {
                // Only the one linked to is selected, with a chain inside; a click on
                // it again unlinks the color, which keeps its value.
                let grid: Vec<_> = saved
                    .iter()
                    .map(|&(name, c)| {
                        let linked = link.as_deref() == Some(name);
                        let tip = if linked {
                            format!("{}\n{}", tip(name, c), tr!("CLICK_TO_UNLINK_COLOR"))
                        } else {
                            tip(name, c)
                        };
                        (c, linked, tip)
                    })
                    .collect();
                if let Some(i) = swatch_grid(ui, &grid, true) {
                    let (name, c) = saved[i];
                    if link.as_deref() == Some(name) {
                        *link = None;
                        response.mark_changed();
                    } else {
                        picked = Some((c, Some(name.to_owned())));
                    }
                }
            }
            // Only when there is something to save: a color no variable has.
            let is_saved = saved.iter().any(|&(_, c)| c == *color);
            if !is_saved
                && ui
                    .add(egui::Button::new(tr!("SAVE_COLOR_AS_VARIABLE")).small())
                    .on_hover_text(tr!("SAVE_COLOR_AS_VARIABLE_HINT"))
                    .clicked()
            {
                // Saved and linked to at once; the app adds the variable.
                let var = Variable {
                    name: variables::new_name(&swatches.vars, Kind::Color),
                    value: Value::Color(*color),
                };
                *link = Some(var.name.clone());
                response.mark_changed();
                ui.data_mut(|d| {
                    d.get_temp_mut_or_default::<Vec<Variable>>(saved_id())
                        .push(var)
                });
            }
            if let Some((mut c, name)) = picked {
                if !alpha {
                    c[3] = 255;
                }
                *color = c;
                *link = name;
                hsva = Hsva::from_srgba_unmultiplied(c);
                response.mark_changed();
            }
            ui.data_mut(|d| d.insert_temp(hsva_id, (*color, hsva)));
        });
    response
}

/// A color variable's tooltip: its name and its hex code.
fn tip(name: &str, c: [u8; 4]) -> String {
    format!("{name} ({})", describe(c))
}

/// The picker's Hex field (`#RRGGBB`, applied when left or on Enter; `#RRGGBBAA` sets
/// the opacity too) and, with `alpha`, its Opacity field in %. Returns true if `color`
/// changed.
fn code_fields(ui: &mut Ui, popup_id: Id, color: &mut [u8; 4], alpha: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        crate::layers_ui::field_frames(ui);
        ui.label(tr!("COLOR_HEX"));
        // Typed into a draft, shown as the color's code otherwise.
        let id = popup_id.with("hex");
        let [r, g, b, a] = *color;
        let code = format!("#{r:02X}{g:02X}{b:02X}");
        let mut draft: String = ui
            .data(|d| d.get_temp(id))
            .filter(|_| ui.memory(|m| m.has_focus(id.with("edit"))))
            .unwrap_or(code);
        let r = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .id(id.with("edit"))
                .desired_width(70.0)
                // In line with the label: the text sits at the top of its box otherwise.
                .align(egui::Align2::LEFT_CENTER),
        );
        if r.lost_focus() {
            let text = draft.trim();
            let text = if text.starts_with('#') {
                text.to_owned()
            } else {
                format!("#{text}")
            };
            if let Some(mut c) = color_from_hex(&text) {
                // Six digits keep the opacity; none without alpha.
                if text.len() == 7 || !alpha {
                    c[3] = if alpha { a } else { 255 };
                }
                if c != *color {
                    *color = c;
                    changed = true;
                }
            }
            ui.data_mut(|d| d.remove::<String>(id));
        } else if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, draft));
        }
        if alpha {
            ui.add_space(8.0);
            ui.label(tr!("OPACITY"));
            let mut pct = opacity_percent(color[3]);
            if ui
                .add(egui::DragValue::new(&mut pct).range(0..=100).speed(1.0))
                .changed()
            {
                color[3] = (pct as f32 * 2.55).round() as u8;
                changed = true;
            }
            crate::layers_ui::unit(ui, "%");
        }
    });
    changed
}

/// A color as the Colors in use section lists it, in a monospace font: `#RRGGBB`,
/// then its opacity, padded so the percentages line up (`#000032  50%`).
fn in_use_text(c: [u8; 4]) -> String {
    let [r, g, b, a] = c;
    format!("#{r:02X}{g:02X}{b:02X} {:>3}%", opacity_percent(a))
}

/// The Colors in use section's body: the colors in use, one per row, each with its
/// swatch and its code. A double click on one adds a color variable of that color,
/// even if one already has it. Returns true if the document changed.
pub fn in_use_ui(ui: &mut Ui, doc: &mut Document) -> bool {
    let mut picked = None;
    // Monospace, so the columns line up, and small enough for a whole row to fit the
    // panel: every row has as many characters as the widest possible one.
    let mut font = egui::TextStyle::Monospace.resolve(ui.style());
    let room = ui.available_width() - SWATCH - 16.0;
    let chars = in_use_text([255; 4]).chars().count() as f32;
    let advance = ui.fonts_mut(|f| f.glyph_width(&egui::FontId::monospace(1.0), '0'));
    if advance > 0.0 {
        font.size = font.size.min(room / (chars * advance)).max(7.0);
    }
    for c in used(doc) {
        let height = ui.spacing().interact_size.y;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            let sw = egui::Rect::from_min_size(
                rect.left_center() - vec2(-2.0, SWATCH / 2.0),
                vec2(SWATCH, SWATCH),
            );
            let [r, g, b, a] = c;
            show_color_at(painter, Color32::from_rgba_unmultiplied(r, g, b, a), sw);
            painter.rect_stroke(
                sw,
                2.0,
                ui.visuals().widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Inside,
            );
            let text_at = egui::pos2(sw.right() + 6.0, rect.center().y);
            painter.text(
                text_at,
                egui::Align2::LEFT_CENTER,
                in_use_text(c),
                font.clone(),
                ui.visuals().text_color(),
            );
        }
        if response
            .on_hover_text(tr!("SAVE_COLOR_IN_USE"))
            .double_clicked()
        {
            picked = Some(c);
        }
    }
    let Some(c) = picked else {
        return false;
    };
    doc.variables.push(Variable {
        name: variables::new_name(&doc.variables, Kind::Color),
        value: Value::Color(c),
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Group, Layer};

    /// A color variable.
    fn cv(name: &str, c: [u8; 4]) -> Variable {
        Variable {
            name: name.into(),
            value: Value::Color(c),
        }
    }

    #[test]
    fn gradient_stops_are_colors_in_use_and_follow_their_variables() {
        use crate::gradient::Gradient;
        let mut doc = Document::default();
        doc.variables.push(cv("brand", [200, 0, 0, 255]));
        let mut layer = crate::shape::new_layer(1, crate::shape::ShapeKind::Rectangle, [0.0; 2]);
        let mut g = Gradient::from_color([1, 2, 3, 255]);
        g.stops[0].link = Some("brand".into());
        layer.shape.as_mut().unwrap().fill_gradient = Some(g);
        doc.layers.push(Node::Layer(Box::new(layer)));
        let mut bg = Gradient::from_color([9, 9, 9, 255]);
        bg.stops[1].link = Some("brand".into());
        doc.background_gradient = Some(bg);

        // Linked stops take their variable's color.
        assert!(apply(&mut doc));
        let stop = |doc: &Document| {
            doc.layer(1)
                .unwrap()
                .shape
                .as_ref()
                .unwrap()
                .fill_gradient
                .as_ref()
                .unwrap()
                .stops[0]
                .clone()
        };
        assert_eq!(stop(&doc).color, [200, 0, 0, 255]);
        assert_eq!(
            doc.background_gradient.as_ref().unwrap().stops[1].color,
            [200, 0, 0, 255]
        );
        // The stops are colors in use, the shape's single fill color is not.
        let used = used(&doc);
        assert!(used.contains(&[200, 0, 0, 255]) && used.contains(&[9, 9, 9, 255]));
        assert!(!used.contains(&[70, 130, 180, 255]), "{used:?}");
        // Counted, renamed, and broken like other links.
        assert_eq!(use_count(&mut doc, "brand"), 2);
        rename_links(&mut doc, "brand", "red");
        assert_eq!(stop(&doc).link.as_deref(), Some("red"));
        assert_eq!(
            crate::variables::broken_links(doc.layer(1).unwrap(), &doc.variables),
            ["red"]
        );
        // Pasted into a project without the variable: saved as one there.
        let mut vars = Vec::new();
        let mut node = doc.layers[0].clone();
        adopt_links(&mut vars, &mut node);
        assert_eq!(vars, [cv("red", [200, 0, 0, 255])]);
    }

    #[test]
    fn used_colors_come_once_each_in_stack_order_without_switched_off_ones() {
        let mut text = Layer::new(1, "t".into(), "".into());
        let mut t = crate::text::Text {
            color: [10, 20, 30, 255],
            outline: false,
            outline_color: [1, 1, 1, 255],
            ..Default::default()
        };
        text.text = Some(Box::new(t.clone()));
        let mut shape = Layer::new(2, "s".into(), "".into());
        shape.shape = Some(Box::new(crate::shape::Shape {
            fill: true,
            fill_color: [10, 20, 30, 255],
            stroke: true,
            stroke_color: [0, 0, 0, 128],
            ..Default::default()
        }));
        let mut shadow = EffectInstanceBuilder::new("builtin.drop_shadow");
        shadow.set_color("color", [5, 6, 7, 255]);
        shape.effects.push(shadow.0);
        t.outline = true;
        let mut outlined = Layer::new(3, "o".into(), "".into());
        outlined.text = Some(Box::new(t));
        let group = Node::Group(Group {
            id: 4,
            name: "g".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            expanded: true,
            children: vec![Node::Layer(Box::new(outlined))],
        });
        let doc = Document {
            background: [255, 255, 255],
            layers: vec![
                Node::Layer(Box::new(text)),
                Node::Layer(Box::new(shape)),
                group,
            ],
            ..Default::default()
        };
        assert_eq!(
            used(&doc),
            [
                [255, 255, 255, 255],
                [10, 20, 30, 255],
                [0, 0, 0, 128],
                [5, 6, 7, 255],
                [1, 1, 1, 255],
            ]
        );
    }

    /// An effect instance with some parameters set.
    struct EffectInstanceBuilder(crate::effects::EffectInstance);

    impl EffectInstanceBuilder {
        fn new(id: &str) -> Self {
            Self(crate::effects::EffectInstance::new(
                registry().get(id).expect("built-in effect"),
            ))
        }

        fn set_color(&mut self, param: &str, c: [u8; 4]) {
            self.0.params.insert(
                param.into(),
                serde_json::Value::from(crate::effects::color_to_hex(c)),
            );
        }
    }

    /// Centers of the opaque rectangles painted in `color`, in paint order.
    fn painted(shapes: &[egui::epaint::ClippedShape], color: [u8; 4]) -> Vec<egui::Pos2> {
        let [r, g, b, a] = color;
        let c = Color32::from_rgba_unmultiplied(r, g, b, a);
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(rect) if rect.fill == c => Some(rect.rect.center()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_swatch_in_the_picker_applies_its_color_and_save_hands_it_to_the_app() {
        let ctx = egui::Context::default();
        let doc = Document {
            background: [200, 10, 10],
            variables: vec![
                cv("green", [10, 200, 10, 255]),
                Variable {
                    name: "color1".into(),
                    value: Value::Length(1.0),
                },
            ],
            ..Default::default()
        };
        let mut color = [1, 2, 3, 255];
        let link = std::cell::RefCell::new(None::<String>);
        let changed = std::cell::Cell::new(false);
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, color: &mut [u8; 4], doc: &Document| {
            time.set(time.get() + 0.1);
            let input = egui::RawInput {
                time: Some(time.get()),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(800.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                publish(ui.ctx(), doc);
                let mut link = link.borrow_mut();
                changed.set(changed.get() | button(ui, color, true, &mut link).changed());
            });
            out.textures_delta.clear();
            out.shapes
        };
        // Move, press and release, one frame each, as egui needs.
        let click = |p: egui::Pos2| {
            let button = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            [
                vec![egui::Event::PointerMoved(p)],
                vec![button(true)],
                vec![button(false)],
            ]
        };
        let shapes = frame(Vec::new(), &mut color, &doc);
        let at = painted(&shapes, color)[0];
        for events in click(at) {
            frame(events, &mut color, &doc);
        }
        // A tenth of a second per frame (`frame`), and some frames for the popup's fade-in.
        for _ in 0..10 {
            frame(Vec::new(), &mut color, &doc);
        }
        let shapes = frame(Vec::new(), &mut color, &doc);
        // The picker is open: the saved green and the background red are shown.
        let green = painted(&shapes, [10, 200, 10, 255]);
        assert_eq!(painted(&shapes, [200, 10, 10, 255]).len(), 1);
        for events in click(green[0]) {
            frame(events, &mut color, &doc);
        }
        assert_eq!(color, [10, 200, 10, 255]);
        assert_eq!(
            *link.borrow(),
            Some("green".into()),
            "a color variable links"
        );
        assert!(changed.get());

        // A click on the variable it is linked to unlinks the color, which stays as it
        // is. "Save this color" isn't offered for a color a variable has.
        let shapes = frame(Vec::new(), &mut color, &doc);
        assert!(!shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Text(t) if t.galley.text() == "Save this color")));
        for events in click(green[0]) {
            frame(events, &mut color, &doc);
        }
        assert_eq!(*link.borrow(), None, "unlinked");
        assert_eq!(color, [10, 200, 10, 255], "the color kept");

        // A color in use is applied unlinked.
        let shapes = frame(Vec::new(), &mut color, &doc);
        let red = painted(&shapes, [200, 10, 10, 255]);
        for events in click(red[0]) {
            frame(events, &mut color, &doc);
        }
        assert_eq!(color, [200, 10, 10, 255]);
        assert_eq!(*link.borrow(), None);

        // "Save this color" is off for a saved color; on for another.
        color = [9, 9, 9, 255];
        let shapes = frame(Vec::new(), &mut color, &doc);
        let save_button = shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "Save this color" => {
                    Some(t.pos + t.galley.size() / 2.0)
                }
                _ => None,
            })
            .expect("the save button");
        for events in click(save_button) {
            frame(events, &mut color, &doc);
        }
        let saved = take_saved(&ctx);
        assert_eq!(
            saved,
            [cv("color2", [9, 9, 9, 255])],
            "named after no variable"
        );
        assert_eq!(*link.borrow(), Some("color2".into()), "and linked to");
        assert!(take_saved(&ctx).is_empty());
    }

    #[test]
    fn saving_keeps_one_variable_per_color_and_name() {
        let mut doc = Document::default();
        assert!(save(&mut doc, cv("a", [1, 2, 3, 255])));
        assert!(!save(&mut doc, cv("b", [1, 2, 3, 255])));
        assert!(!save(&mut doc, cv("a", [4, 5, 6, 255])));
        assert!(save(&mut doc, cv("b", [1, 2, 3, 0])));
        doc.variables.push(Variable {
            name: "c".into(),
            value: Value::Number(1.0),
        });
        assert!(
            !save(&mut doc, cv("c", [7, 7, 7, 255])),
            "a number has the name"
        );
        assert_eq!(doc.variables.len(), 3);
    }

    #[test]
    fn saved_colors_of_older_projects_become_color_variables() {
        let mut json = serde_json::to_value(Document::default()).unwrap();
        let fields = json.as_object_mut().unwrap();
        fields.insert("background_link".into(), "Color 1".into());
        fields.insert(
            "variables".into(),
            serde_json::json!([{"name": "ink", "value": 2}]),
        );
        fields.insert(
            "palette".into(),
            serde_json::json!([
                {"color": [1, 2, 3, 255], "name": "Color 1"},
                {"color": [4, 5, 6, 255], "name": "ink"},
                {"color": [7, 8, 9, 255], "name": ""},
                {"color": [0, 0, 0, 255], "name": "2 tone"},
            ]),
        );
        let json = json.to_string();
        let doc = Document::from_json(&json).unwrap();
        let vars: Vec<_> = doc
            .variables
            .iter()
            .map(|v| (v.name.as_str(), v.value.clone()))
            .collect();
        assert_eq!(
            vars,
            [
                ("ink", Value::Number(2.0)),
                ("Color_1", Value::Color([1, 2, 3, 255])),
                ("ink_2", Value::Color([4, 5, 6, 255])),
                ("color1", Value::Color([7, 8, 9, 255])),
                ("_2_tone", Value::Color([0, 0, 0, 255])),
            ]
        );
        assert_eq!(doc.background_link.as_deref(), Some("Color_1"));
        assert!(doc.old_palette.is_empty());
        let saved = serde_json::to_string(&doc).unwrap();
        assert!(!saved.contains("palette"), "{saved}");
    }

    #[test]
    fn pasted_links_fit_the_variables_of_the_project_they_land_in() {
        let sw = |c: [u8; 4], n: &str| cv(n, c);
        let red = [200, 0, 0, 255];
        let blue = [0, 0, 200, 255];
        let mut text = Layer::new(1, "t".into(), "".into());
        text.text = Some(Box::new(crate::text::Text {
            color: red,
            outline_color: blue,
            ..Default::default()
        }));
        text.color_links.insert(ColorField::Text, "Color 1".into());
        text.color_links
            .insert(ColorField::TextOutline, "ink".into());
        text.color_links.insert(ColorField::Fill, "Color 1".into());
        let mut shadow = EffectInstanceBuilder::new("builtin.drop_shadow");
        shadow.set_color("color", [9, 9, 9, 255]);
        shadow.0.links.insert("color".into(), "shadow".into());
        text.effects.push(shadow.0);
        let node = Node::Layer(Box::new(text));
        let links = |node: &Node| match node {
            Node::Layer(l) => (l.color_links.clone(), l.effects[0].links.clone()),
            _ => unreachable!(),
        };

        // "Color 1" is another color there, blue is saved as "navy", "shadow" is free.
        let mut palette = vec![sw([0, 255, 0, 255], "Color 1"), sw(blue, "navy")];
        let mut pasted = node.clone();
        adopt_links(&mut palette, &mut pasted);
        let (layer, fx) = links(&pasted);
        assert_eq!(
            layer.into_iter().collect::<Vec<_>>(),
            [(ColorField::TextOutline, "navy".to_owned())],
            "the clashing name and the missing fill are dropped"
        );
        assert_eq!(fx["color"], "shadow");
        assert_eq!(palette[2], sw([9, 9, 9, 255], "shadow"));

        // In the project it was copied from, nothing changes.
        let mut palette = vec![
            sw(red, "Color 1"),
            sw(blue, "ink"),
            sw([9, 9, 9, 255], "shadow"),
        ];
        let before = palette.clone();
        let mut pasted = node.clone();
        adopt_links(&mut palette, &mut pasted);
        assert_eq!(palette, before);
        assert_eq!(links(&pasted).0.len(), 2);

        // A variable of another kind has the name: that link is dropped.
        let number = Variable {
            name: "shadow".into(),
            value: Value::Number(1.0),
        };
        let mut vars = vec![number.clone()];
        let mut pasted = node.clone();
        adopt_links(&mut vars, &mut pasted);
        assert!(links(&pasted).1.is_empty());
        assert_eq!(vars, [number, sw(red, "Color 1"), sw(blue, "ink")]);
    }

    #[test]
    fn colors_in_use_are_listed_by_code_and_saved_by_double_click() {
        assert_eq!(in_use_text([255, 8, 0, 255]), "#FF0800 100%");
        assert_eq!(in_use_text([0, 0, 50, 128]), "#000032  50%");
        assert_eq!(describe([0, 0, 50, 128]), "#000032, 50% opacity");
        assert_eq!(describe([255, 8, 0, 255]), "#FF0800, 100% opacity");
        let ctx = egui::Context::default();
        let mut doc = Document {
            background: [200, 10, 10],
            ..Default::default()
        };
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, doc: &mut Document| {
            time.set(time.get() + 0.05);
            let input = egui::RawInput {
                time: Some(time.get()),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(400.0, 300.0),
                )),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let mut out = ctx.run_ui(input, |ui| changed = in_use_ui(ui, doc));
            out.textures_delta.clear();
            let row = out.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == "#C80A0A 100%" => {
                    Some(t.pos + t.galley.size() / 2.0)
                }
                _ => None,
            });
            (changed, row)
        };
        let at = frame(Vec::new(), &mut doc).1.expect("the background's row");
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at)], &mut doc);
        let clicks = |n: usize, doc: &mut Document| {
            let mut changed = false;
            for _ in 0..n {
                changed |= frame(vec![press(true)], doc).0;
                changed |= frame(vec![press(false)], doc).0;
            }
            changed
        };
        assert!(!clicks(1, &mut doc), "a click saves nothing");
        assert!(doc.variables.is_empty());
        // Far enough from the first click not to make a double click with it.
        time.set(time.get() + 1.0);
        assert!(clicks(2, &mut doc));
        assert_eq!(doc.variables, [cv("color1", [200, 10, 10, 255])]);
        time.set(time.get() + 1.0);
        assert!(clicks(2, &mut doc), "again, as another variable");
        assert_eq!(
            doc.variables,
            [
                cv("color1", [200, 10, 10, 255]),
                cv("color2", [200, 10, 10, 255])
            ]
        );
    }

    #[test]
    fn the_hex_and_opacity_fields_set_the_color() {
        let ctx = egui::Context::default();
        let mut color = [255, 8, 0, 128];
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, color: &mut [u8; 4]| {
            time.set(time.get() + 0.1);
            let input = egui::RawInput {
                time: Some(time.get()),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let mut out = ctx.run_ui(input, |ui| {
                changed = code_fields(ui, Id::unique("p"), color, true);
            });
            out.textures_delta.clear();
            let texts: Vec<(String, egui::Pos2)> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.pos + t.galley.size() / 2.0))
                    }
                    _ => None,
                })
                .collect();
            (changed, texts)
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // Clicks the field showing `shown`, types `text` over it and presses Enter.
        let type_in = |shown: &str, text: &str, color: &mut [u8; 4]| {
            let texts = frame(Vec::new(), color).1;
            let at = texts
                .iter()
                .find(|(t, _)| t == shown)
                .unwrap_or_else(|| panic!("{shown} in {texts:?}"))
                .1;
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut changed = false;
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![press(true)],
                vec![press(false)],
                vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
                vec![egui::Event::Text(text.into())],
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
                vec![],
            ] {
                changed |= frame(events, color).0;
            }
            changed
        };
        let texts = frame(Vec::new(), &mut color).1;
        assert!(texts.iter().any(|(t, _)| t == "#FF0800"), "{texts:?}");
        assert!(
            texts.iter().any(|(t, _)| t == "50"),
            "the opacity: {texts:?}"
        );
        // Six digits keep the opacity; eight set it.
        assert!(type_in("#FF0800", "00ff00", &mut color));
        assert_eq!(color, [0, 255, 0, 128]);
        assert!(type_in("#00FF00", "#11223380", &mut color));
        assert_eq!(color, [0x11, 0x22, 0x33, 0x80]);
        assert!(
            !type_in("#112233", "nonsense", &mut color),
            "not a color: unchanged"
        );
        assert!(type_in("50", "100", &mut color));
        assert_eq!(color[3], 255);
    }

    #[test]
    fn a_color_button_is_square_and_a_link_shows_as_a_chain_inside_it() {
        let ctx = egui::Context::default();
        let doc = Document {
            variables: vec![cv("ink", [1, 2, 3, 255])],
            ..Default::default()
        };
        // The button's rect, the colors of the rect strokes and the number of lines
        // drawn (the crack of a broken chain).
        let draw = |link: Option<&str>, color: [u8; 4]| {
            let mut c = color;
            let mut link = link.map(str::to_owned);
            let mut rect = egui::Rect::NOTHING;
            let mut side = 0.0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                publish(ui.ctx(), &doc);
                side = ui.spacing().interact_size.y;
                rect = button(ui, &mut c, true, &mut link).rect;
            });
            out.textures_delta.clear();
            assert_eq!(c, color, "it keeps its color");
            let strokes: Vec<Color32> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r) => Some(r.stroke.color),
                    egui::Shape::Path(p) => match p.stroke.color {
                        egui::epaint::ColorMode::Solid(c) => Some(c),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            let lines = out
                .shapes
                .iter()
                .filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. }))
                .count();
            assert!((rect.width() - side).abs() < 0.5 && (rect.height() - side).abs() < 0.5);
            (strokes, lines)
        };
        let (strokes, _) = draw(None, [1, 2, 3, 255]);
        assert!(!strokes.contains(&Color32::WHITE), "no link, no chain");
        let (strokes, lines) = draw(Some("ink"), [1, 2, 3, 255]);
        assert!(
            strokes.contains(&Color32::WHITE),
            "a white chain on a dark color"
        );
        assert_eq!(lines, 0);
        let (strokes, _) = draw(Some("ink"), [250, 250, 200, 255]);
        assert!(
            strokes.contains(&Color32::BLACK),
            "a black chain on a light color"
        );
        let (_, lines) = draw(Some("gone"), [1, 2, 3, 255]);
        assert!(lines > 0, "a broken chain, cracked");
    }

    #[test]
    fn linked_colors_follow_their_saved_color_and_keep_their_value_when_it_is_gone() {
        let mut shape = Layer::new(1, "s".into(), "".into());
        shape.shape = Some(Box::default());
        shape.color_links.insert(ColorField::Fill, "brand".into());
        shape.color_links.insert(ColorField::Text, "brand".into());
        let mut shadow = EffectInstanceBuilder::new("builtin.drop_shadow");
        shadow.0.links.insert("color".into(), "ink".into());
        shape.effects.push(shadow.0);
        let mut doc = Document {
            background_link: Some("brand".into()),
            layers: vec![Node::Layer(Box::new(shape))],
            variables: vec![cv("brand", [200, 0, 0, 255]), cv("ink", [0, 0, 50, 128])],
            ..Default::default()
        };
        assert!(apply(&mut doc));
        assert!(!apply(&mut doc), "nothing more to do");
        let layer = |doc: &Document| match &doc.layers[0] {
            Node::Layer(l) => l.clone(),
            _ => unreachable!(),
        };
        let l = layer(&doc);
        assert_eq!(doc.background, [200, 0, 0]);
        assert_eq!(l.shape.as_ref().unwrap().fill_color, [200, 0, 0, 255]);
        assert_eq!(
            l.color_links.keys().collect::<Vec<_>>(),
            [&ColorField::Fill],
            "a shape has no text color"
        );
        let fx = &l.effects[0];
        let params = Params::new(registry().get(&fx.effect).unwrap().params(), &fx.params);
        assert_eq!(params.color("color"), [0, 0, 50, 128]);
        assert_eq!(use_count(&mut doc, "brand"), 2);

        // Renamed in the links, then removed: the colors stay as they are, their links
        // kept, broken.
        rename_links(&mut doc, "brand", "red");
        doc.variables[0] = cv("red", [0, 200, 0, 255]);
        apply(&mut doc);
        assert_eq!(doc.background, [0, 200, 0]);
        // A variable of that name that isn't a color is no link either.
        doc.variables[0].value = Value::Number(1.0);
        doc.variables.remove(1);
        assert!(!apply(&mut doc), "nothing changes");
        let l = layer(&doc);
        assert_eq!(doc.background_link.as_deref(), Some("red"));
        assert_eq!(l.color_links[&ColorField::Fill], "red");
        assert_eq!(l.effects[0].links["color"], "ink");
        assert_eq!(l.shape.as_ref().unwrap().fill_color, [0, 200, 0, 255]);

        // A color variable of that name links them again.
        doc.variables.push(cv("ink", [9, 9, 9, 255]));
        assert!(apply(&mut doc));
        let fx = &layer(&doc).effects[0];
        let params = Params::new(registry().get(&fx.effect).unwrap().params(), &fx.params);
        assert_eq!(params.color("color"), [9, 9, 9, 255]);
    }

    /// Draws color buttons, unlinked, linked and broken, in both themes, to the PNG
    /// named by `COLOR_BUTTON_PREVIEW`: `cargo test color_button_preview -- --ignored`.
    #[test]
    #[ignore]
    fn color_button_preview() {
        let Ok(path) = std::env::var("COLOR_BUTTON_PREVIEW") else {
            return;
        };
        let doc = Document {
            variables: vec![
                cv("ink", [40, 120, 220, 255]),
                cv("paper", [245, 240, 200, 255]),
            ],
            ..Default::default()
        };
        crate::preview::render(&path, vec2(260.0, 140.0), |ui| {
            publish(ui.ctx(), &doc);
            egui::Grid::new("g").show(ui, |ui| {
                for (label, link, c) in [
                    ("Fill", None, [200, 60, 60, 255]),
                    ("Stroke", Some("ink"), [40, 120, 220, 255]),
                    ("Outline", Some("paper"), [245, 240, 200, 255]),
                    ("Text", Some("gone"), [90, 90, 90, 255]),
                ] {
                    ui.label(label);
                    let mut c = c;
                    let mut link = link.map(str::to_owned);
                    button(ui, &mut c, true, &mut link);
                    ui.end_row();
                }
            });
        });
    }

    /// Draws the picker's Hex and Opacity row in a popup's style, as egui draws its
    /// popup menus, to the PNG named by `COLOR_CODE_PREVIEW`.
    #[test]
    #[ignore]
    fn color_code_preview() {
        let Ok(path) = std::env::var("COLOR_CODE_PREVIEW") else {
            return;
        };
        crate::preview::render(&path, vec2(300.0, 60.0), |ui| {
            egui::containers::menu::menu_style(ui.style_mut());
            let mut c = [0, 194, 240, 255];
            code_fields(ui, Id::unique("preview"), &mut c, true);
        });
    }

    /// Draws the Colors in use section to the PNG named by `COLORS_IN_USE_PREVIEW`.
    #[test]
    #[ignore]
    fn colors_in_use_preview() {
        let Ok(path) = std::env::var("COLORS_IN_USE_PREVIEW") else {
            return;
        };
        let mut shape = Layer::new(1, "s".into(), "".into());
        shape.shape = Some(Box::new(crate::shape::Shape {
            fill_color: [0, 108, 234, 255],
            stroke_color: [0, 0, 50, 128],
            ..Default::default()
        }));
        let doc = std::cell::RefCell::new(Document {
            background: [255, 255, 255],
            layers: vec![Node::Layer(Box::new(shape))],
            ..Default::default()
        });
        // As narrow as the left panel.
        crate::preview::render(&path, vec2(230.0, 200.0), |ui| {
            in_use_ui(ui, &mut doc.borrow_mut());
        });
    }
}
