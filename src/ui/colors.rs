//! Color buttons and their pickers, swatches and the Colors in use list. The colors in
//! use and the links to color variables are in `document::colors`.

use crate::document::Document;
use crate::document::colors::in_use;
use crate::document::variables::{self, Kind, Value, Variable};
use crate::i18n::tr;
use crate::render::effects::color_from_hex;
use crate::ui::variables as variables_ui;
use egui::color_picker::{Alpha, color_picker_hsva_2d, show_color_at};
use egui::ecolor::Hsva;
use egui::{Color32, Id, Popup, PopupCloseBehavior, Response, RichText, Sense, Ui, vec2};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Side of a swatch in the pickers and the Colors section, in points.
const SWATCH: f32 = 18.0;

/// Swatches per row in a picker.
const PER_ROW: usize = 12;

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
    /// The colors added with the eyedropper, then the other colors in use.
    picked: Vec<[u8; 4]>,
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

/// Makes the project's colors available to the color buttons drawn this frame, with
/// `picked`, the colors added with the eyedropper (last first), first among the colors
/// in use.
pub fn publish(ctx: &egui::Context, doc: &Document, picked: &[[u8; 4]]) {
    let swatches = Arc::new(Swatches {
        vars: doc.variables.clone(),
        picked: picked.to_vec(),
        used: in_use(doc, picked),
    });
    ctx.data_mut(|d| d.insert_temp(swatches_id(), swatches));
}

/// Color variables made from a picker's "Save this color" button since the last call.
pub fn take_saved(ctx: &egui::Context) -> Vec<Variable> {
    ctx.data_mut(|d| d.remove_temp::<Vec<Variable>>(saved_id()))
        .unwrap_or_default()
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
    variables_ui::paint_link_stroke(painter, icon, egui::Stroke::new(3.0, halo), broken);
    variables_ui::paint_link_stroke(painter, icon, egui::Stroke::new(1.4, ink), broken);
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
    let response = button(ui, color, alpha, Some(&mut link));
    if let Some(name) = link {
        links.insert(key, name);
    }
    response
}

/// A color button whose picker also shows the project's color variables and used
/// colors. `color` is straight RGBA; with `alpha` false it stays opaque. `link` is the
/// name of the color variable it is linked to: a click on a color variable links it,
/// any other change unlinks it. Without `link`, the color can't be linked: a color
/// variable clicked gives its color, and there is no "Save this color". The button is a square swatch; linked, a chain link is
/// drawn inside it (the variable's name is in its tooltip); linked to a variable that
/// no longer exists, a broken chain.
pub fn button(
    ui: &mut Ui,
    color: &mut [u8; 4],
    alpha: bool,
    link: Option<&mut Option<String>>,
) -> Response {
    let linkable = link.is_some();
    let mut unlinked = None;
    let link = link.unwrap_or(&mut unlinked);
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
        // Always outlined, so that a color like the panel's (white on white) shows; the
        // widget's own outline when hovered or open.
        let stroke = if open || response.hovered() {
            visuals.bg_stroke
        } else {
            egui::Stroke::new(1.0, ui.visuals().weak_text_color())
        };
        ui.painter()
            .rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
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
            // As wide as the picker: the texts under it wrap.
            ui.set_max_width(275.0);
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
            // The colors in use, right below the sliders: those added with the
            // eyedropper, a thin line, then the others.
            ui.add_space(4.0);
            for (n, colors) in [&swatches.picked, &swatches.used].into_iter().enumerate() {
                if colors.is_empty() {
                    continue;
                }
                if n == 1 && !swatches.picked.is_empty() {
                    ui.separator();
                }
                let grid: Vec<_> = colors
                    .iter()
                    .map(|&c| (c, link.is_none() && c == *color, describe(c)))
                    .collect();
                if let Some(i) = swatch_grid(ui, &grid, false) {
                    picked = Some((colors[i], None));
                }
            }
            ui.separator();
            ui.label(RichText::new(tr!("COLOR_VARIABLES")).small());
            let saved: Vec<(&str, [u8; 4])> = swatches.saved().collect();
            if saved.is_empty() {
                crate::ui::callout::callout_small(
                    ui,
                    crate::ui::callout::Tone::Info,
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
            // Always there, so the picker keeps its size; grayed out when there is nothing
            // to save: a color a variable has already. At the right, under the picker.
            let is_saved = saved.iter().any(|&(_, c)| c == *color);
            let save = linkable
                && ui
                    // As wide as the picker above: the popup doesn't widen.
                    .allocate_ui_with_layout(
                        vec2(ui.min_rect().width(), 0.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            // A button as in the panels: a popup's are drawn flat.
                            ui.style_mut().visuals.widgets =
                                ui.ctx().global_style().visuals.widgets.clone();
                            ui.add_enabled(
                                !is_saved,
                                egui::Button::new(tr!("SAVE_COLOR_AS_VARIABLE")).frame(true),
                            )
                            .on_hover_text(tr!("SAVE_COLOR_AS_VARIABLE_HINT"))
                            .clicked()
                        },
                    )
                    .inner;
            if save {
                // Saved and linked to at once; the app adds the variable.
                let var = Variable {
                    formula: None,
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
pub fn tip(name: &str, c: [u8; 4]) -> String {
    format!("{name} ({})", describe(c))
}

/// The picker's Hex field (`#RRGGBB`, applied when left or on Enter; `#RRGGBBAA` sets
/// the opacity too) and, with `alpha`, its Opacity field in %. Returns true if `color`
/// changed.
fn code_fields(ui: &mut Ui, popup_id: Id, color: &mut [u8; 4], alpha: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        crate::ui::layers::field_frames(ui);
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
        // In capitals, as typed too.
        if r.changed() {
            draft = draft.to_uppercase();
        }
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
    });
    if alpha {
        // On its own row, the slider ending under the picker's.
        let picker = ui.spacing().slider_width;
        ui.horizontal(|ui| {
            crate::ui::layers::field_frames(ui);
            let label = ui.label(tr!("OPACITY")).rect.width();
            let sp = ui.spacing().item_spacing.x;
            // The value box and the unit after it.
            let value = ui.spacing().interact_size.x + 2.0 * sp + 14.0;
            ui.spacing_mut().slider_width = (picker - label - value - sp).max(60.0);
            let mut pct = opacity_percent(color[3]);
            if ui.add(egui::Slider::new(&mut pct, 0..=100)).changed() {
                color[3] = (pct as f32 * 2.55).round() as u8;
                changed = true;
            }
            crate::ui::layers::unit(ui, "%");
        });
    }
    changed
}

/// A color as the Colors in use section lists it, in a monospace font: `#RRGGBB`,
/// then its opacity, padded so the percentages line up (`#000032  50%`).
fn in_use_text(c: [u8; 4]) -> String {
    let [r, g, b, a] = c;
    format!("#{r:02X}{g:02X}{b:02X} {:>3}%", opacity_percent(a))
}

/// The Colors in use section's body: the colors in use, one per row, each with its
/// swatch and its code: `picked` (the eyedropper's), a thin line, then the others. A
/// double click on one adds a color variable of that color, even if one already has it.
/// Returns true if the document changed.
pub fn in_use_ui(ui: &mut Ui, doc: &mut Document, picked: &[[u8; 4]]) -> bool {
    let mut saved = None;
    // Monospace, so the columns line up, and small enough for a whole row to fit the
    // panel: every row has as many characters as the widest possible one.
    let mut font = egui::TextStyle::Monospace.resolve(ui.style());
    let room = ui.available_width() - SWATCH - 16.0;
    let chars = in_use_text([255; 4]).chars().count() as f32;
    let advance = ui.fonts_mut(|f| f.glyph_width(&egui::FontId::monospace(1.0), '0'));
    if advance > 0.0 {
        font.size = font.size.min(room / (chars * advance)).max(7.0);
    }
    let used = in_use(doc, picked);
    for (i, &c) in picked.iter().chain(&used).enumerate() {
        if i == picked.len() && i > 0 {
            ui.separator();
        }
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
            saved = Some(c);
        }
    }
    let Some(c) = saved else {
        return false;
    };
    doc.variables.push(Variable {
        formula: None,
        name: variables::new_name(&doc.variables, Kind::Color),
        value: Value::Color(c),
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Layer, Node};

    #[test]
    fn the_colors_picked_come_first_among_the_colors_in_use() {
        let mut text = Layer::new(1, "t".into(), "".into());
        text.text = Some(Box::new(crate::document::text::Text {
            color: [10, 20, 30, 255],
            outline: false,
            ..Default::default()
        }));
        let doc = Document {
            layers: vec![Node::Layer(Box::new(text))],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let used = |picked: &[[u8; 4]]| {
            publish(&ctx, &doc, picked);
            let s = ctx
                .data(|d| d.get_temp::<Arc<Swatches>>(swatches_id()))
                .unwrap();
            (s.picked.clone(), s.used.clone())
        };
        let (_, before) = used(&[]);
        let picked = [[1, 2, 3, 255], [4, 5, 6, 255]];
        assert_eq!(used(&picked), (picked.to_vec(), before.clone()));
        // A color already in use is shown with the picked ones only.
        let (first, rest) = used(&[[10, 20, 30, 255]]);
        assert_eq!(first, [[10, 20, 30, 255]]);
        assert!(!rest.contains(&[10, 20, 30, 255]));
        assert_eq!(rest.len() + 1, before.len());
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
                    formula: None,
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
                publish(ui.ctx(), doc, &[]);
                let mut link = link.borrow_mut();
                changed.set(changed.get() | button(ui, color, true, Some(&mut link)).changed());
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
        // is. "Save this color" stays, grayed out, for a color a variable has: the
        // picker keeps its size.
        let shapes = frame(Vec::new(), &mut color, &doc);
        assert!(shapes.iter().any(|s| matches!(&s.shape,
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
            let mut out = ctx.run_ui(input, |ui| changed = in_use_ui(ui, doc, &[]));
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
                publish(ui.ctx(), &doc, &[]);
                side = ui.spacing().interact_size.y;
                rect = button(ui, &mut c, true, Some(&mut link)).rect;
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
        crate::ui::preview::render(&path, vec2(260.0, 140.0), |ui| {
            publish(ui.ctx(), &doc, &[]);
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
                    button(ui, &mut c, true, Some(&mut link));
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
        crate::ui::preview::render(&path, vec2(300.0, 60.0), |ui| {
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
        shape.shape = Some(Box::new(crate::document::shape::Shape {
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
        crate::ui::preview::render(&path, vec2(230.0, 200.0), |ui| {
            in_use_ui(ui, &mut doc.borrow_mut(), &[]);
        });
    }

    /// A color variable.
    fn cv(name: &str, c: [u8; 4]) -> Variable {
        Variable {
            formula: None,
            name: name.into(),
            value: Value::Color(c),
        }
    }
}
