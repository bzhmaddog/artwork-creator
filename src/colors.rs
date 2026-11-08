//! The project's colors, to reuse them: those in use, gathered from the document, and
//! the palette of colors saved with the project on purpose, each with its own name.
//!
//! Every color button of the app is [`button`]: its picker shows both lists as swatches,
//! and a click on one applies it. A click on a saved color also links the button's color
//! to it, by name, like a field linked to a variable: each frame, [`apply`] gives every
//! linked color its saved color's value, so changing a saved color changes all the
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
use crate::i18n::{tr, tr_n};

/// A color saved in the project's palette.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    /// Straight RGBA.
    pub color: [u8; 4],
    /// Unique in the palette; the colors linked to this one name it. Older projects
    /// may have none: [`name_palette`] gives them one.
    #[serde(default)]
    pub name: String,
}

/// A layer's color that can be linked to a saved color. An effect's color parameters
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

/// Names of the saved colors a layer's colors are linked to.
pub type Links = BTreeMap<ColorField, String>;

/// Side of a swatch in the pickers and the Colors section, in points.
const SWATCH: f32 = 18.0;
/// Swatches per row in a picker.
const PER_ROW: usize = 12;

/// Every color the project uses, once each, in stack order: the page background, then
/// for each layer from the bottom its text, outline, fill and stroke colors and those of
/// its effects. Colors switched off (no fill, no outline) don't count.
pub fn used(doc: &Document) -> Vec<[u8; 4]> {
    let [r, g, b] = doc.background;
    let mut colors = vec![[r, g, b, 255]];
    let mut add = |c: [u8; 4]| {
        if !colors.contains(&c) {
            colors.push(c);
        }
    };
    fn walk(nodes: &[Node], add: &mut dyn FnMut([u8; 4])) {
        for node in nodes {
            match node {
                Node::Group(g) => walk(&g.children, add),
                Node::Layer(l) => {
                    if let Some(t) = &l.text {
                        add(t.color);
                        if t.outline {
                            add(t.outline_color);
                        }
                    }
                    if let Some(s) = &l.shape {
                        if s.fill && s.kind.can_fill() {
                            add(s.fill_color);
                        }
                        if s.stroke {
                            add(s.stroke_color);
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

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn hex(c: [u8; 4]) -> String {
    let [r, g, b, a] = c;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// What the color pickers show this frame.
#[derive(Clone, Default)]
struct Swatches {
    saved: Vec<Swatch>,
    used: Vec<[u8; 4]>,
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
        saved: doc.palette.clone(),
        used: used(doc),
    });
    ctx.data_mut(|d| d.insert_temp(swatches_id(), swatches));
}

/// Colors saved from a picker's "Save this color" button since the last call.
pub fn take_saved(ctx: &egui::Context) -> Vec<Swatch> {
    ctx.data_mut(|d| d.remove_temp::<Vec<Swatch>>(saved_id()))
        .unwrap_or_default()
}

/// Adds `swatch` to the palette unless its color or its name is there already. Returns
/// true if it was added.
pub fn save(doc: &mut Document, swatch: Swatch) -> bool {
    if doc
        .palette
        .iter()
        .any(|s| s.color == swatch.color || s.name == swatch.name)
    {
        return false;
    }
    doc.palette.push(swatch);
    true
}

/// A name no saved color has: "Color 1", "Color 2"...
pub fn new_name(palette: &[Swatch]) -> String {
    (1..)
        .map(|n| tr!("NEW_SAVED_COLOR_NAME", n = n))
        .find(|n| palette.iter().all(|s| s.name != *n))
        .expect("an unused name")
}

/// Names the saved colors that have no name, or the name of one before them.
pub fn name_palette(palette: &mut [Swatch]) {
    for i in 0..palette.len() {
        let name = palette[i].name.trim();
        if name.is_empty() || palette[..i].iter().any(|s| s.name == name) {
            palette[i].name = new_name(palette);
        } else if name.len() != palette[i].name.len() {
            palette[i].name = name.to_owned();
        }
    }
}

/// Gives every linked color its saved color's value, and unlinks those whose saved
/// color is gone. Returns true if the document changed.
pub fn apply(doc: &mut Document) -> bool {
    let palette = std::mem::take(&mut doc.palette);
    let find = |name: &str| palette.iter().find(|s| s.name == name).map(|s| s.color);
    let mut changed = false;
    if let Some(name) = &doc.background_link {
        match find(name) {
            Some([r, g, b, _]) => {
                changed |= doc.background != [r, g, b];
                doc.background = [r, g, b];
            }
            None => {
                doc.background_link = None;
                changed = true;
            }
        }
    }
    doc.for_each_layer_mut(|layer| {
        let mut links = std::mem::take(&mut layer.color_links);
        links.retain(|&field, name| {
            let (Some(c), Some(slot)) = (find(name), slot(layer, field)) else {
                changed = true;
                return false;
            };
            changed |= *slot != c;
            *slot = c;
            true
        });
        layer.color_links = links;
        for fx in &mut layer.effects {
            let params = &mut fx.params;
            fx.links.retain(|param, name| {
                let Some(c) = find(name) else {
                    changed = true;
                    return false;
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
    doc.palette = palette;
    changed
}

/// Fits the color links of `node`, pasted from the clipboard, to `palette`, which may not
/// be the one it was copied from. A link stays when the palette has a saved color of
/// that name and value; it moves to the saved color of that value when there is one;
/// otherwise its color is saved under its name. A link whose name the palette gives to
/// another color is dropped, its color kept as it is.
pub fn adopt_links(palette: &mut Vec<Swatch>, node: &mut Node) {
    let mut fit = |name: &mut String, c: [u8; 4]| {
        if palette.iter().any(|s| s.name == *name && s.color == c) {
            return true;
        }
        if let Some(s) = palette.iter().find(|s| s.color == c) {
            *name = s.name.clone();
            return true;
        }
        if palette.iter().any(|s| s.name == *name) {
            return false;
        }
        palette.push(Swatch {
            color: c,
            name: name.clone(),
        });
        true
    };
    node.for_each_layer_mut(&mut |l| {
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

/// Every link to a saved color in the document: the page background's, the layers'
/// and their effects'.
fn for_each_link(doc: &mut Document, mut f: impl FnMut(&mut String) -> bool) {
    if let Some(name) = &mut doc.background_link
        && !f(name)
    {
        doc.background_link = None;
    }
    doc.for_each_layer_mut(|l| {
        l.color_links.retain(|_, name| f(name));
        for fx in &mut l.effects {
            fx.links.retain(|_, name| f(name));
        }
    });
}

/// How many colors are linked to the saved color `name`.
fn use_count(doc: &mut Document, name: &str) -> usize {
    let mut n = 0;
    for_each_link(doc, |link| {
        n += (link == name) as usize;
        true
    });
    n
}

/// A swatch of `color`, clickable. The selected one gets a thick frame.
pub fn swatch(ui: &mut Ui, color: [u8; 4], selected: bool) -> Response {
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
    }
    response
}

/// A grid of swatches, each a color, whether it is selected and its tooltip; returns
/// the index of the one clicked.
fn swatch_grid(ui: &mut Ui, swatches: &[([u8; 4], bool, String)]) -> Option<usize> {
    let mut picked = None;
    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
    for (r, row) in swatches.chunks(PER_ROW).enumerate() {
        ui.horizontal(|ui| {
            for (i, (c, selected, tip)) in row.iter().enumerate() {
                if swatch(ui, *c, *selected).on_hover_text(tip).clicked() {
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

/// A color button whose picker also shows the project's saved and used colors. `color`
/// is straight RGBA; with `alpha` false it stays opaque. `link` is the name of the
/// saved color it is linked to: a click on a saved color links it, any other change
/// unlinks it. A linked button has a colored frame.
pub fn button(
    ui: &mut Ui,
    color: &mut [u8; 4],
    alpha: bool,
    link: &mut Option<String>,
) -> Response {
    let popup_id = ui.auto_id_with("color_popup");
    let open = Popup::is_id_open(ui.ctx(), popup_id);
    let size = ui.spacing().interact_size;
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
        let stroke = if link.is_some() {
            egui::Stroke::new(2.0, ui.visuals().hyperlink_color)
        } else {
            visuals.bg_stroke
        };
        ui.painter()
            .rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    }
    if let Some(name) = link {
        response = response.on_hover_text(tr!("COLOR_LINKED_HINT", name = name));
    }

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

            let swatches = ui
                .data(|d| d.get_temp::<Arc<Swatches>>(swatches_id()))
                .unwrap_or_default();
            ui.separator();
            let mut picked = None;
            ui.label(RichText::new(tr!("PALETTE_COLORS")).small());
            if swatches.saved.is_empty() {
                ui.label(RichText::new(tr!("PALETTE_EMPTY")).small().weak());
            } else {
                let grid: Vec<_> = swatches
                    .saved
                    .iter()
                    .map(|s| {
                        let selected = match link {
                            Some(name) => s.name == *name,
                            None => s.color == *color,
                        };
                        (s.color, selected, tip(s))
                    })
                    .collect();
                if let Some(i) = swatch_grid(ui, &grid) {
                    let s = &swatches.saved[i];
                    picked = Some((s.color, Some(s.name.clone())));
                }
            }
            let saved = swatches.saved.iter().any(|s| s.color == *color);
            if ui
                .add_enabled(
                    !saved,
                    egui::Button::new(tr!("SAVE_COLOR_TO_PALETTE")).small(),
                )
                .on_hover_text(tr!("SAVE_COLOR_TO_PALETTE_HINT"))
                .clicked()
            {
                // Saved and linked to at once; the app adds it to the palette.
                let swatch = Swatch {
                    color: *color,
                    name: new_name(&swatches.saved),
                };
                *link = Some(swatch.name.clone());
                response.mark_changed();
                ui.data_mut(|d| {
                    d.get_temp_mut_or_default::<Vec<Swatch>>(saved_id())
                        .push(swatch)
                });
            }
            ui.label(RichText::new(tr!("COLORS_IN_USE")).small());
            let grid: Vec<_> = swatches
                .used
                .iter()
                .map(|&c| (c, link.is_none() && c == *color, hex(c)))
                .collect();
            if let Some(i) = swatch_grid(ui, &grid) {
                picked = Some((swatches.used[i], None));
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

/// An opaque color button for an RGB color, as [`button`].
pub fn button_rgb(ui: &mut Ui, color: &mut [u8; 3], link: &mut Option<String>) -> Response {
    let [r, g, b] = *color;
    let mut c = [r, g, b, 255];
    let response = button(ui, &mut c, false, link);
    *color = [c[0], c[1], c[2]];
    response
}

/// A saved swatch's tooltip: its name and its hex code.
fn tip(s: &Swatch) -> String {
    format!("{} ({})", s.name, hex(s.color))
}

/// The Colors section's body: the saved colors, each with its name and a remove
/// button, then the colors in use, a click on one saving it. Renaming a saved color
/// renames it in the links; removing one unlinks the colors using it, which keep their
/// value. Returns true if the document changed.
pub fn palette_ui(ui: &mut Ui, doc: &mut Document) -> bool {
    let mut changed = false;
    ui.label(tr!("PALETTE_COLORS"));
    if doc.palette.is_empty() {
        ui.label(RichText::new(tr!("PALETTE_EMPTY")).weak());
    }
    let mut remove = None;
    let mut rename = None;
    let names: Vec<String> = doc.palette.iter().map(|s| s.name.clone()).collect();
    let used_by: Vec<usize> = names.iter().map(|n| use_count(doc, n)).collect();
    for (i, s) in doc.palette.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui
                .color_edit_button_srgba_unmultiplied(&mut s.color)
                .on_hover_text(hex(s.color))
                .changed();
            // The name is edited in a draft, applied when the field is left, so the
            // links follow from one valid name to the next.
            let id = ui.scope_id().with(("saved_color_name", i));
            let mut draft = ui
                .data(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| s.name.clone());
            let name = draft.trim();
            let ok =
                !name.is_empty() && !names.iter().enumerate().any(|(j, m)| j != i && m == name);
            let width = ui.available_width() - ui.spacing().interact_size.y - 4.0;
            let mut edit = egui::TextEdit::singleline(&mut draft)
                .hint_text(tr!("SAVED_COLOR_NAME_HINT"))
                .desired_width(width.max(40.0));
            if !ok {
                edit = edit.text_color(ui.visuals().error_fg_color);
            }
            let r = ui.add(edit);
            let r = if ok {
                r.on_hover_text(tr_n!("SAVED_COLOR_USE_COUNT", used_by[i]))
            } else {
                r.on_hover_text(tr!("SAVED_COLOR_NAME_INVALID"))
            };
            if r.lost_focus() {
                let name = draft.trim().to_owned();
                if ok && name != s.name {
                    rename = Some((s.name.clone(), name.clone()));
                    s.name = name;
                    changed = true;
                }
                ui.data_mut(|d| d.remove::<String>(id));
            } else if r.has_focus() || r.changed() {
                ui.data_mut(|d| d.insert_temp(id, draft));
            }
            if ui
                .add(egui::Button::new("🗑").frame(false))
                .on_hover_text(tr!("REMOVE_PALETTE_COLOR"))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some((old, new)) = rename {
        for_each_link(doc, |link| {
            if *link == old {
                *link = new.clone();
            }
            true
        });
    }
    if let Some(i) = remove {
        let gone = doc.palette.remove(i).name;
        for_each_link(doc, |link| *link != gone);
        changed = true;
    }
    if ui.button(tr!("ADD_PALETTE_COLOR")).clicked() {
        let name = new_name(&doc.palette);
        doc.palette.push(Swatch {
            color: [255, 255, 255, 255],
            name,
        });
        changed = true;
    }

    ui.add_space(4.0);
    ui.label(tr!("COLORS_IN_USE"))
        .on_hover_text(tr!("COLORS_IN_USE_HINT"));
    let used = used(doc);
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for c in used {
            let saved = doc.palette.iter().any(|s| s.color == c);
            let tip = if saved {
                tr!("COLOR_IN_PALETTE", hex = hex(c))
            } else {
                tr!("SAVE_COLOR_IN_USE", hex = hex(c))
            };
            if swatch(ui, c, saved).on_hover_text(tip).clicked() {
                picked = Some(c);
            }
        }
    });
    if let Some(color) = picked {
        let name = new_name(&doc.palette);
        changed |= save(doc, Swatch { color, name });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Group, Layer};

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
            palette: vec![Swatch {
                color: [10, 200, 10, 255],
                name: "green".into(),
            }],
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
        assert_eq!(*link.borrow(), Some("green".into()), "a saved color links");
        assert!(changed.get());

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
            [Swatch {
                color: [9, 9, 9, 255],
                name: "Color 1".into(),
            }]
        );
        assert_eq!(*link.borrow(), Some("Color 1".into()), "and linked to");
        assert!(take_saved(&ctx).is_empty());
    }

    #[test]
    fn saving_keeps_one_swatch_per_color_and_name_and_hex_drops_full_alpha() {
        let mut doc = Document::default();
        let sw = |c, n: &str| Swatch {
            color: c,
            name: n.into(),
        };
        assert!(save(&mut doc, sw([1, 2, 3, 255], "a")));
        assert!(!save(&mut doc, sw([1, 2, 3, 255], "b")));
        assert!(!save(&mut doc, sw([4, 5, 6, 255], "a")));
        assert!(save(&mut doc, sw([1, 2, 3, 0], "b")));
        assert_eq!(doc.palette.len(), 2);
        assert_eq!(hex([1, 2, 3, 255]), "#010203");
        assert_eq!(hex([255, 0, 171, 128]), "#ff00ab80");
        let json = serde_json::to_string(&doc).unwrap();
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back.palette, doc.palette);
        assert!(
            !serde_json::to_string(&Document::default())
                .unwrap()
                .contains("palette"),
            "an empty palette isn't saved"
        );
    }

    #[test]
    fn pasted_links_fit_the_palette_of_the_project_they_land_in() {
        let sw = |c: [u8; 4], n: &str| Swatch {
            color: c,
            name: n.into(),
        };
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
    }

    #[test]
    fn older_palettes_get_a_unique_name_for_each_color() {
        let sw = |n: &str| Swatch {
            color: [0, 0, 0, 255],
            name: n.into(),
        };
        let mut palette = vec![sw(""), sw(" ink "), sw("ink"), sw("Color 1")];
        name_palette(&mut palette);
        let names: Vec<_> = palette.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Color 2", "ink", "Color 3", "Color 1"]);
    }

    #[test]
    fn linked_colors_follow_their_saved_color_and_unlink_when_it_is_gone() {
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
            palette: vec![
                Swatch {
                    color: [200, 0, 0, 255],
                    name: "brand".into(),
                },
                Swatch {
                    color: [0, 0, 50, 128],
                    name: "ink".into(),
                },
            ],
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

        // Renamed in the links, then removed: the colors stay as they are.
        for_each_link(&mut doc, |link| {
            if link == "brand" {
                *link = "red".into();
            }
            true
        });
        doc.palette[0].name = "red".into();
        doc.palette[0].color = [0, 200, 0, 255];
        apply(&mut doc);
        assert_eq!(doc.background, [0, 200, 0]);
        doc.palette.clear();
        apply(&mut doc);
        let l = layer(&doc);
        assert_eq!(doc.background_link, None);
        assert!(l.color_links.is_empty() && l.effects[0].links.is_empty());
        assert_eq!(l.shape.as_ref().unwrap().fill_color, [0, 200, 0, 255]);
    }
}
