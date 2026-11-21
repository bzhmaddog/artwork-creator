//! Number fields linked to formulas, the formula editor, link buttons and the Variables
//! panel. The variables, formulas and links themselves are in `document::variables`.

use crate::document::Document;
use crate::document::variables::{
    Bindings, Field, FormulaError, Kind, Value, Variable, eval, inherited, name_problem, new_name,
    point_family, rename_in, use_count, uses_variables,
};
use crate::i18n::{tr, tr_n};
use egui::{DragValue, Id, Response, RichText, Ui};
use std::cell::RefCell;
use std::sync::Arc;

fn vars_id() -> Id {
    Id::unique("project_variables")
}

/// Makes the project's variables available to the fields drawn this frame.
pub fn publish(ctx: &egui::Context, doc: &Document) {
    let vars = Arc::new(doc.all_variables());
    ctx.data_mut(|d| d.insert_temp(vars_id(), vars));
}

pub fn published(ctx: &egui::Context) -> Arc<Vec<Variable>> {
    ctx.data(|d| d.get_temp(vars_id())).unwrap_or_default()
}

/// A number field that can be linked to a formula, with an "fx" button inside it, at
/// its right end, which opens the formula editor ([`formula_editor`]). It is the
/// `DragValue` made by `make`, which also takes a formula: one naming a variable links
/// the field. Linked, it shows the formula's result (the formula is in its tooltip and
/// the editor), its fx button filled with the link color; a number typed in or dragged
/// unlinks it. Linked to a variable that no longer exists (removed), it keeps its last value,
/// its fx button in the warning color: a value typed or dragged unlinks it, a variable
/// of that name links it again. The unit is left to the caller, after the field (see
/// `ui::layers::unit`). The response is the value's, changed when `value` was changed
/// here; [`apply`](crate::document::variables::apply) keeps linked fields up to date.
pub fn field<'a>(
    ui: &mut Ui,
    bindings: &mut Bindings,
    field: Field,
    value: &'a mut f32,
    make: impl FnOnce(DragValue<'a>) -> DragValue<'a>,
) -> Response {
    let vars = published(ui.ctx());
    let id = ui.auto_id_with(("formula", field));
    let mut formula = bindings.get(&field).cloned();
    let width = ui.spacing().interact_size.x * 1.6 + ui.spacing().interact_size.y;
    let response = formula_field(
        ui,
        id,
        &mut formula,
        field.kind(),
        &vars,
        value,
        make,
        width,
    );
    match formula {
        Some(f) => bindings.insert(field, f),
        None => bindings.remove(&field),
    };
    response
}

/// A [`field`] of `kind` linked to `formula` (none when unlinked), whose variables are
/// `vars`, its state kept under `formula_id`, `width` wide with the fx button.
#[allow(clippy::too_many_arguments)]
fn formula_field<'a>(
    ui: &mut Ui,
    formula_id: Id,
    formula: &mut Option<String>,
    kind: Kind,
    vars: &[Variable],
    value: &'a mut f32,
    make: impl FnOnce(DragValue<'a>) -> DragValue<'a>,
    width: f32,
) -> Response {
    let link = match formula.as_deref() {
        None => Link::None,
        Some(formula) => match eval(formula, vars, kind) {
            Ok(v) => Link::Linked(tr!(
                "LINKED_FIELD_HINT",
                formula = formula,
                value = format!("{v:.2}{}", kind.unit())
            )),
            Err(FormulaError::UnknownVariable(name)) => Link::Broken(tr!(
                "BROKEN_FIELD_LINK_HINT",
                formula = formula,
                name = name
            )),
            Err(e) => Link::Broken(format!("{formula}: {}", e.message())),
        },
    };
    let fx_width = ui.spacing().interact_size.y;
    // A number set in the formula editor last frame (the value is borrowed by the field
    // once drawn, so it is set now).
    let pending_id = formula_id.with("set");
    let pending = ui.data_mut(|d| d.remove_temp::<f32>(pending_id));
    if let Some(v) = pending {
        *value = v;
    }
    let current = *value;
    // The value, the formula's result when linked. A number typed in or dragged unlinks
    // it, a formula links it again.
    let (mut response, typed) = plain_field(ui, value, make, vars, kind, width);
    match typed {
        Some(typed) => *formula = Some(typed),
        None if response.changed() => *formula = None,
        None => {}
    }
    match &link {
        Link::Linked(hint) | Link::Broken(hint) => response = response.on_hover_text(hint),
        Link::None => {}
    }
    // The fx button, over the field's right end: added last, it takes the clicks there.
    let rect = response.rect;
    let fx_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - fx_width, rect.top()),
        rect.right_bottom(),
    )
    .shrink(2.0);
    let fx = ui.interact(fx_rect, formula_id.with("fx"), egui::Sense::click());
    let hint = match &link {
        Link::None => tr!("LINK_VARIABLE_HINT").to_owned(),
        Link::Linked(_) => tr!("CHANGE_VARIABLE_LINK_HINT").to_owned(),
        Link::Broken(hint) => hint.clone(),
    };
    let fx = fx.on_hover_text(hint);
    if ui.is_rect_visible(fx_rect) {
        let painter = ui.painter();
        // Linked, a filled badge in the link color (the warning color when broken), so
        // the fields using variables stand out; unlinked, a plain gray fx.
        let badge = match link {
            Link::None => None,
            Link::Linked(_) => Some(ui.visuals().hyperlink_color),
            Link::Broken(_) => Some(ui.visuals().warn_fg_color),
        };
        let color = match badge {
            Some(fill) => {
                let fill = if fx.hovered() {
                    fill.gamma_multiply(0.8)
                } else {
                    fill
                };
                painter.rect_filled(fx_rect, crate::ui::tokens::RADIUS, fill);
                ui.visuals().extreme_bg_color
            }
            None => {
                if fx.hovered() {
                    painter.rect_filled(
                        fx_rect,
                        crate::ui::tokens::RADIUS,
                        ui.visuals().widgets.hovered.weak_bg_fill,
                    );
                }
                ui.visuals().weak_text_color()
            }
        };
        let mut font = egui::TextStyle::Small.resolve(ui.style());
        font.size *= 1.1;
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "fx",
            0.0,
            egui::TextFormat {
                font_id: font,
                color,
                italics: true,
                ..Default::default()
            },
        );
        let galley = painter.layout_job(job);
        painter.galley(fx_rect.center() - galley.size() / 2.0, galley, color);
    }
    if let Some(v) = formula_editor(ui, &fx, formula, kind, current, vars) {
        ui.data_mut(|d| d.insert_temp(pending_id, v));
        ui.ctx().request_repaint();
    }
    if pending.is_some() {
        response.mark_changed();
    }
    response
}

/// Width of the formula editor's contents, in points.
const FORMULA_EDITOR_WIDTH: f32 = 260.0;

/// Height of the formula editor's variable list, in points: longer lists scroll.
const VARIABLE_LIST_HEIGHT: f32 = 240.0;

/// The formula editor of a [`field`], opened by its fx button `fx`: the formula, its
/// result (or what is wrong with it), the variables the field takes (a click inserts
/// one at the cursor), then Unlink (the field keeps its value), Cancel and OK. OK with a
/// formula naming variables links the field to it; with a plain number, it unlinks the
/// field and returns the number, for the field to take. Enter is OK, a click outside
/// cancels. Unlinked, the formula starts as the field's `value`.
fn formula_editor(
    ui: &mut Ui,
    fx: &Response,
    formula: &mut Option<String>,
    kind: Kind,
    value: f32,
    vars: &[Variable],
) -> Option<f32> {
    let popup_id = fx.id.with("editor");
    let draft_id = popup_id.with("draft");
    let edit_id = popup_id.with("edit");
    let filter_id = popup_id.with("filter");
    if fx.clicked() {
        let draft = formula
            .clone()
            .unwrap_or_else(|| format!("{}", (value * 1000.0).round() / 1000.0));
        ui.data_mut(|d| {
            d.insert_temp(draft_id, draft);
            d.insert_temp(filter_id, String::new());
            // Ready to type once the editor shows.
            d.insert_temp(popup_id.with("focus"), true);
        });
    }
    let mut set = None;
    let mut close = false;
    egui::Popup::menu(fx)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            // A fixed width: the formula box would otherwise take all the room it gets.
            ui.set_width(FORMULA_EDITOR_WIDTH);
            let mut draft: String = ui.data(|d| d.get_temp(draft_id)).unwrap_or_default();
            ui.label(RichText::new(tr!("FORMULA_EDITOR_TITLE")).strong());
            // Sized as a whole: a desired width leaves out the box's margins, which then
            // stick out of the editor.
            let size = [ui.available_width(), ui.spacing().interact_size.y];
            let edit = ui
                .scope(|ui| {
                    crate::ui::layers::field_frames(ui);
                    ui.add_sized(
                        size,
                        egui::TextEdit::singleline(&mut draft)
                            .id(edit_id)
                            .align(egui::Align2::LEFT_CENTER),
                    )
                })
                .inner;
            if ui.data_mut(|d| d.remove_temp::<bool>(popup_id.with("focus"))) == Some(true) {
                edit.request_focus();
            }
            let result = eval(&draft, vars, kind);
            match &result {
                Ok(v) => ui.label(tr!(
                    "FORMULA_RESULT",
                    value = format!("{v:.2}{}", kind.unit())
                )),
                Err(e) => ui.colored_label(ui.visuals().error_fg_color, e.message()),
            };
            let compatible: Vec<&Variable> = vars
                .iter()
                .filter(|v| matches!(v.value.kind(), k if k == kind || k == Kind::Number))
                .collect();
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("FORMULA_VARIABLES")).small().weak());
            if compatible.is_empty() {
                crate::ui::callout::callout(
                    ui,
                    crate::ui::callout::Tone::Info,
                    tr!("NO_COMPATIBLE_VARIABLES"),
                );
            }
            if !compatible.is_empty() {
                let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
                ui.scope(|ui| {
                    crate::ui::layers::field_frames(ui);
                    ui.add_sized(
                        size,
                        egui::TextEdit::singleline(&mut filter)
                            .hint_text(tr!("FILTER_VARIABLES_HINT")),
                    );
                });
                let needle = filter.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(VARIABLE_LIST_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let row = |ui: &mut Ui, v: &Variable| {
                            let text = format!("{}   {}", v.name, v.value.short_text());
                            ui.horizontal(|ui| {
                                kind_icon(ui, v.value.kind());
                                ui.selectable_label(false, text).clicked()
                            })
                            .inner
                            .then(|| v.name.clone())
                        };
                        let mut picked = None;
                        if needle.is_empty() {
                            // Template points folded by family, plain variables first.
                            let mut families: Vec<(&str, Vec<&Variable>)> = Vec::new();
                            for &v in &compatible {
                                match point_family(&v.name) {
                                    Some(f) => match families.iter_mut().find(|(g, _)| *g == f) {
                                        Some((_, list)) => list.push(v),
                                        None => families.push((f, vec![v])),
                                    },
                                    None => {
                                        if let Some(n) = row(ui, v) {
                                            picked = Some(n);
                                        }
                                    }
                                }
                            }
                            for (family, list) in families {
                                let points = list.len().div_ceil(2);
                                egui::CollapsingHeader::new(format!("{family} ({points})"))
                                    .id_salt(popup_id.with(family))
                                    .show(ui, |ui| {
                                        for v in list {
                                            if let Some(n) = row(ui, v) {
                                                picked = Some(n);
                                            }
                                        }
                                    });
                            }
                        } else {
                            for &v in compatible
                                .iter()
                                .filter(|v| v.name.to_lowercase().contains(&needle))
                            {
                                if let Some(n) = row(ui, v) {
                                    picked = Some(n);
                                }
                            }
                        }
                        if let Some(name) = picked {
                            insert_at_cursor(ui.ctx(), edit_id, &mut draft, &name);
                        }
                    });
                ui.data_mut(|d| d.insert_temp(filter_id, filter));
            }
            ui.add_space(4.0);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            // As in the app's dialogs: OK at the right edge, Cancel on its left; Unlink,
            // when linked, apart at the left edge.
            crate::app::dialog_button_row(ui, |ui| {
                let ok = ui
                    .add_enabled(result.is_ok(), egui::Button::new(tr!("OK")))
                    .clicked();
                let cancel = ui.button(tr!("CANCEL")).clicked();
                if formula.is_some() {
                    crate::app::at_left_edge(ui, |ui| {
                        if ui.button(tr!("UNLINK_VARIABLE")).clicked() {
                            *formula = None;
                            close = true;
                        }
                    });
                }
                close |= cancel;
                if (ok || enter)
                    && let Ok(v) = result
                {
                    let text = draft.trim().to_owned();
                    if uses_variables(&text) {
                        *formula = Some(text);
                    } else {
                        *formula = None;
                        set = Some(v as f32);
                    }
                    close = true;
                }
            });
            ui.data_mut(|d| d.insert_temp(draft_id, draft));
        });
    if close {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    set
}

/// Puts `name` into `text` at the cursor of the text field `id` (at the end when it has
/// none), with the cursor after it, and gives the field the keyboard back. A text that
/// is just a number (the field's value, which the editor starts with) is replaced.
fn insert_at_cursor(ctx: &egui::Context, id: Id, text: &mut String, name: &str) {
    if text.trim().is_empty() || text.trim().parse::<f64>().is_ok() {
        text.clear();
    }
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let chars = text.chars().count();
    let at = state
        .cursor
        .char_range()
        .map_or(chars, |r| usize::from(r.primary.index).min(chars));
    let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
    text.insert_str(byte, name);
    let after = egui::text::CCursor::new(at + name.chars().count());
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(after)));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Where a field stands with its variables.
pub enum Link {
    None,
    /// Linked; the hint says to what.
    Linked(String),
    /// Linked to a variable that no longer exists; the hint says which.
    Broken(String),
}

/// What was done with a [`link_button`].
pub enum LinkAction {
    None,
    /// Link to the variable of this name.
    Pick(String),
    /// Unlink, keeping the value.
    Unlink,
}

/// A link button: a chain link, gray when unlinked, in the link color when linked and
/// broken in the warning color when its variable is gone. Its menu lists `candidates`,
/// `current` selected, then Unlink.
pub fn link_button(
    ui: &mut Ui,
    link: &Link,
    current: Option<&str>,
    candidates: &[&Variable],
) -> LinkAction {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let hint = match link {
        Link::None => tr!("LINK_VARIABLE_HINT").to_owned(),
        Link::Linked(_) => tr!("CHANGE_VARIABLE_LINK_HINT").to_owned(),
        Link::Broken(hint) => hint.clone(),
    };
    let response = response.on_hover_text(hint);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                crate::ui::tokens::RADIUS,
                ui.visuals().widgets.hovered.weak_bg_fill,
            );
        }
        let color = match link {
            Link::None => ui.visuals().weak_text_color(),
            Link::Linked(_) => ui.visuals().hyperlink_color,
            Link::Broken(_) => ui.visuals().warn_fg_color,
        };
        paint_link(ui.painter(), rect, color, matches!(link, Link::Broken(_)));
    }
    let mut action = LinkAction::None;
    egui::Popup::menu(&response).show(|ui| {
        if candidates.is_empty() {
            crate::ui::callout::callout_small(
                ui,
                crate::ui::callout::Tone::Info,
                tr!("NO_COMPATIBLE_VARIABLES"),
            );
        }
        for v in candidates {
            let selected = current == Some(v.name.as_str());
            let text = format!("{}   {}", v.name, v.value.short_text());
            let clicked = ui
                .horizontal(|ui| {
                    kind_icon(ui, v.value.kind());
                    ui.selectable_label(selected, text).clicked()
                })
                .inner;
            if clicked {
                action = LinkAction::Pick(v.name.clone());
            }
        }
        ui.separator();
        if ui
            .add_enabled(current.is_some(), egui::Button::new(tr!("UNLINK_VARIABLE")))
            .clicked()
        {
            action = LinkAction::Unlink;
        }
    });
    action
}

/// Paints a chain link in `rect`: two interlocked links, or, `broken`, two links apart
/// with a crack between them.
fn paint_link(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, broken: bool) {
    paint_link_stroke(
        painter,
        rect,
        egui::Stroke::new(crate::ui::tokens::STROKE, color),
        broken,
    );
}

/// [`paint_link`] with `stroke`.
pub fn paint_link_stroke(
    painter: &egui::Painter,
    rect: egui::Rect,
    stroke: egui::Stroke,
    broken: bool,
) {
    let side = rect.width().min(rect.height());
    let c = rect.center();
    // Along the diagonal, from bottom left to top right, as chain icons usually are.
    let axis = egui::vec2(1.0, -1.0).normalized();
    let across = egui::vec2(axis.y, -axis.x);
    let (len, off) = if broken {
        (side * 0.46, side * 0.29)
    } else {
        (side * 0.6, side * 0.2)
    };
    let radius = side * 0.13;
    for sign in [-1.0, 1.0] {
        capsule(painter, c + axis * sign * off, axis, len, radius, stroke);
    }
    if broken {
        // Sparks in the gap: a short stroke on each side of it, across the chain.
        for sign in [-1.0, 1.0] {
            let from = c + across * sign * side * 0.12;
            let to = c + across * sign * side * 0.3;
            painter.line_segment([from, to], stroke);
        }
    }
}

/// A capsule's outline: a link of a chain, `len` long from end to end along `axis`,
/// `radius` round at both ends, centered on `center`.
fn capsule(
    painter: &egui::Painter,
    center: egui::Pos2,
    axis: egui::Vec2,
    len: f32,
    radius: f32,
    stroke: egui::Stroke,
) {
    let half = (len / 2.0 - radius).max(0.0);
    let angle = axis.y.atan2(axis.x);
    let mut points = Vec::with_capacity(20);
    for (end, from) in [
        (center + axis * half, -90.0_f32),
        (center - axis * half, 90.0),
    ] {
        for k in 0..=8 {
            let a = angle + (from + 180.0 * k as f32 / 8.0).to_radians();
            points.push(end + radius * egui::vec2(a.cos(), a.sin()));
        }
    }
    painter.add(egui::Shape::closed_line(points, stroke));
}

/// A broken chain in the warning color, for a layer list row whose layer (or group)
/// is linked to variables that no longer exist, named in its tooltip.
pub fn broken_links_badge(ui: &mut Ui, gone: &[String]) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_link(ui.painter(), rect, ui.visuals().warn_fg_color, true);
    }
    response.on_hover_text(tr_n!(
        "LAYER_BROKEN_LINKS",
        gone.len(),
        names = gone.join(", ")
    ))
}

/// An unlinked [`field`]: the `DragValue` made by `make`, which also takes a formula.
/// Returns its response and the formula typed in, when it names a variable.
fn plain_field<'a>(
    ui: &mut Ui,
    value: &'a mut f32,
    make: impl FnOnce(DragValue<'a>) -> DragValue<'a>,
    vars: &[Variable],
    kind: Kind,
    width: f32,
) -> (Response, Option<String>) {
    let typed: RefCell<Option<String>> = RefCell::new(None);
    let drag = make(DragValue::new(value)).custom_parser(|s| {
        let v = eval(s, vars, kind).ok()?;
        if uses_variables(s) {
            *typed.borrow_mut() = Some(s.trim().to_owned());
        }
        Some(v)
    });
    let response = ui.add_sized([width, ui.spacing().interact_size.y], drag);
    (response, typed.into_inner())
}

/// The Variables section's body: the variables grouped by kind, each with its kind's
/// icon, its name, its value, its unit and a remove button, then an Add menu.
/// Renaming a variable renames it in
/// the formulas and color links. Removing one in use asks first; the fields and colors
/// using it keep their link and their value, and show a broken link icon (see
/// [`field`]). Only `kinds` can be added. Returns true if the document changed.
pub fn variables_ui(ui: &mut Ui, doc: &mut Document, kinds: &[Kind]) -> bool {
    let mut changed = false;
    if doc.variables.is_empty() && doc.template_variables.is_empty() {
        crate::ui::callout::callout(ui, crate::ui::callout::Tone::Info, tr!("VARIABLES_EMPTY"));
        ui.add_space(4.0);
    }
    // Names the template's variables have: not given to the project's.
    let template_names: Vec<String> = doc
        .template_variables
        .iter()
        .map(|v| v.name.clone())
        .collect();
    // With a template that has variables: two sections, the template's first.
    let sections = template_variables_ui(ui, doc);
    if sections {
        ui.add_space(4.0);
        ui.label(RichText::new(tr!("PROJECT_VARIABLES")).strong());
        if doc.variables.is_empty() {
            ui.weak(tr!("NO_PROJECT_VARIABLES"));
        }
    }
    let mut remove = None;
    let mut rename = None;
    let names: Vec<String> = doc.variables.iter().map(|v| v.name.clone()).collect();
    let used: Vec<usize> = names.iter().map(|n| use_count(doc, n)).collect();
    // What the project's lengths, angles, sizes and numbers can inherit from.
    let template_vars = doc.template_variables.clone();
    for kind in Kind::ALL {
        let of_kind: Vec<usize> = (0..doc.variables.len())
            .filter(|&i| doc.variables[i].value.kind() == kind)
            .collect();
        if of_kind.is_empty() {
            continue;
        }
        for i in of_kind {
            let var = &mut doc.variables[i];
            // Inheriting from a template variable that is gone (the template replaced),
            // or with another error.
            // What is wrong with the name being typed (or the name itself).
            let mut name_error = None;
            let error = match inherited(var, &template_vars) {
                Some(Err(e)) => Some(e),
                _ => None,
            };
            ui.horizontal(|ui| {
                kind_icon(ui, kind);
                // From the right edge: the remove button, the value, then the name in the
                // room left, so the values and buttons line up from row to row.
                let value_width = ui.spacing().interact_size.x * 1.6;
                let row_height = ui.spacing().interact_size.y;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::ui::layers::tool_button(
                        ui,
                        crate::ui::layers::Tool::Delete,
                        true,
                        tr!("REMOVE_VARIABLE"),
                    ) {
                        remove = Some(i);
                    }
                    // The unit, in a column of its own so the values line up, empty for
                    // numbers and colors.
                    ui.allocate_ui_with_layout(
                        egui::vec2(row_height * 1.2, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| crate::ui::layers::unit(ui, kind.unit()),
                    );
                    if let Value::Text(t) = &mut var.value {
                        changed |= text_value_button(ui, t, [value_width, row_height]);
                    } else if let Value::Color(c) = &mut var.value {
                        changed |= crate::ui::colors::button(ui, c, true, None).changed();
                    } else if !template_vars.is_empty() || var.formula.is_some() {
                        // With an fx button, to inherit from the template's variables.
                        let id = ui.scope_id().with(("variable_formula", i));
                        let before = var.formula.clone();
                        if let Some(v) = var.value.number_mut() {
                            changed |= formula_field(
                                ui,
                                id,
                                &mut var.formula,
                                kind,
                                &template_vars,
                                v,
                                |d| d.speed(0.1).max_decimals(3),
                                value_width,
                            )
                            .changed();
                        }
                        changed |= var.formula != before;
                    } else if let Some(v) = var.value.number_mut() {
                        changed |= ui
                            .add_sized(
                                [value_width, row_height],
                                DragValue::new(v).speed(0.1).max_decimals(3),
                            )
                            .changed();
                    }
                    // The name is edited in a draft, applied when the field is left, so the
                    // formulas and links follow from one valid name to the next.
                    let id = ui.scope_id().with(("variable_name", i));
                    let mut draft = ui
                        .data(|d| d.get_temp::<String>(id))
                        .unwrap_or_else(|| var.name.clone());
                    let taken = |n: &str| {
                        names.iter().enumerate().any(|(j, m)| j != i && m == n)
                            || template_names.iter().any(|m| m == n)
                    };
                    let problem = name_problem(&draft, taken(&draft));
                    let ok = problem.is_none();
                    let mut edit = egui::TextEdit::singleline(&mut draft)
                        .desired_width(ui.available_width())
                        .hint_text(tr!("VARIABLE_NAME_HINT"));
                    if !ok || error.is_some() {
                        edit = edit.text_color(ui.visuals().error_fg_color);
                    }
                    let r = ui.add(edit);
                    // The use count; a problem is told in the warning under the row.
                    let r = if ok {
                        r.on_hover_text(tr_n!("VARIABLE_USE_COUNT", used[i]))
                    } else {
                        r
                    };
                    if r.lost_focus() {
                        if ok && draft != var.name {
                            rename = Some((var.name.clone(), draft.clone()));
                            var.name = draft;
                            changed = true;
                        }
                        ui.data_mut(|d| d.remove::<String>(id));
                    } else if r.has_focus() || r.changed() {
                        ui.data_mut(|d| d.insert_temp(id, draft));
                    }
                    name_error = problem;
                });
            });
            if let Some(text) = &name_error {
                crate::ui::callout::callout(ui, crate::ui::callout::Tone::Warning, text);
                ui.add_space(2.0);
            }
            if let (Some(e), Some(formula)) = (&error, &var.formula) {
                let text = match e {
                    FormulaError::UnknownVariable(source) => tr!(
                        "INHERITED_VARIABLE_MISSING",
                        name = var.name,
                        source = source
                    ),
                    e => format!("{} = {formula}: {}", var.name, e.message()),
                };
                crate::ui::callout::callout(ui, crate::ui::callout::Tone::Warning, &text);
                ui.add_space(2.0);
            }
        }
    }
    if let Some((old, new)) = rename {
        doc.for_each_layer_mut(|l| {
            for f in l.bindings.values_mut() {
                *f = rename_in(f, &old, &new);
            }
        });
        crate::document::colors::rename_links(doc, &old, &new);
        crate::document::text_links::rename_links(doc, &old, &new);
    }
    // A variable in use is removed once confirmed; its name waits here meanwhile.
    let confirm_id = Id::unique("confirm_remove_variable");
    if let Some(i) = remove {
        if used[i] == 0 {
            doc.variables.remove(i);
            changed = true;
        } else {
            ui.data_mut(|d| d.insert_temp(confirm_id, names[i].clone()));
        }
    }
    if let Some(name) = ui.data(|d| d.get_temp::<String>(confirm_id)) {
        let n = names.iter().position(|m| *m == name).map_or(0, |i| used[i]);
        let mut answer = (false, false);
        let response =
            crate::app::dialog(ui.ctx(), "confirm_remove_variable").show(ui.ctx(), |ui| {
                ui.set_max_width(420.0);
                ui.strong(tr!("CONFIRM_REMOVE_VARIABLE", name = name));
                ui.add_space(4.0);
                crate::ui::callout::callout(
                    ui,
                    crate::ui::callout::Tone::Warning,
                    &tr_n!("REMOVED_VARIABLE_USERS", n),
                );
                ui.add_space(8.0);
                answer = crate::app::dialog_buttons(ui, tr!("REMOVE_VARIABLE_OK"), true);
            });
        let (remove, cancel) = answer;
        if remove || cancel || response.should_close() {
            ui.data_mut(|d| d.remove::<String>(confirm_id));
        }
        if remove {
            // The fields and colors using it keep their link, broken, and their value.
            doc.variables.retain(|v| v.name != name);
            changed = true;
        }
    }
    ui.add_space(6.0);
    // At the panel's right edge.
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.menu_button(tr!("ADD_VARIABLE"), |ui| {
                for &kind in kinds {
                    let clicked = ui
                        .horizontal(|ui| {
                            kind_icon(ui, kind);
                            ui.button(kind.label()).clicked()
                        })
                        .inner;
                    if clicked {
                        let name = new_name(&doc.all_variables(), kind);
                        doc.variables.push(Variable {
                            formula: None,
                            name,
                            value: kind.new_value(),
                        });
                        changed = true;
                    }
                }
            });
        },
    );
    ui.add_space(6.0);
    changed
}

/// The project template's variables, under a Template heading, in rows tinted in the
/// accent color: read-only (they are the template's, changed in the template editor),
/// usable in formulas like the others. Those a project variable names are left out: the
/// project's is used. Returns true if it showed any.
fn template_variables_ui(ui: &mut Ui, doc: &mut Document) -> bool {
    let shown: Vec<Variable> = doc
        .template_variables
        .iter()
        .filter(|t| doc.variables.iter().all(|v| v.name != t.name))
        .cloned()
        .collect();
    if shown.is_empty() {
        return false;
    }
    ui.label(RichText::new(tr!("TEMPLATE_VARIABLES")).strong());
    let tint = ui.visuals().selection.bg_fill.gamma_multiply(0.18);
    for var in &shown {
        let kind = var.value.kind();
        let used = use_count(doc, &var.name);
        let response = egui::Frame::NONE
            .fill(tint)
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    kind_icon(ui, kind);
                    // At the right edge, the unit then the value, the name in the room
                    // left.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::ui::layers::unit(ui, kind.unit());
                        // As the project's value fields show it: one decimal at least.
                        let value = match var.value.number() {
                            Some(v) if v.fract() == 0.0 => format!("{v:.1}"),
                            Some(v) => format!("{v}"),
                            None => var.value.short_text(),
                        };
                        ui.label(value);
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(ui.spacing().button_padding.x);
                            ui.add(egui::Label::new(&var.name).truncate());
                        });
                    });
                });
            })
            .response;
        response.on_hover_text(format!(
            "{}\n{}",
            tr!("TEMPLATE_VARIABLE_HINT"),
            tr_n!("VARIABLE_USE_COUNT", used)
        ));
        ui.add_space(2.0);
    }
    true
}

/// A text variable's value in the Variables section: a button of `size` showing the
/// start of the text, which opens a box to edit it, as a color button opens a picker.
/// Returns true if the text changed.
fn text_value_button(ui: &mut Ui, text: &mut String, size: [f32; 2]) -> bool {
    let first_line = text.lines().next().unwrap_or_default();
    let button = ui
        .add_sized(size, egui::Button::new(first_line).truncate())
        .on_hover_text(if text.is_empty() {
            tr!("EDIT_TEXT_VARIABLE_HINT").to_owned()
        } else {
            format!("{text}\n\n{}", tr!("EDIT_TEXT_VARIABLE_HINT"))
        });
    let mut changed = false;
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::ui::layers::field_frames(ui);
            let edit = ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(4)
                    .desired_width(260.0),
            );
            // Ready to type as soon as it opens.
            if edit.ctx.memory(|m| m.focused().is_none()) {
                edit.request_focus();
            }
            changed |= edit.changed();
        });
    changed
}

/// Draws the kind's icon, which stands for it in the Variables section: a dimension
/// line for lengths, an arc between two rays for angles, "Tt" for text sizes, "#" for
/// numbers and three color dots for colors. Hovering it shows [`Kind::label`].
pub fn kind_icon(ui: &mut Ui, kind: Kind) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let color = ui.visuals().weak_text_color();
        let stroke = egui::Stroke::new(crate::ui::tokens::STROKE, color);
        let r = rect.shrink(side * 0.2);
        match kind {
            Kind::Length => {
                let y = r.center().y;
                painter.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], stroke);
                for x in [r.left(), r.right()] {
                    let h = r.height() * 0.3;
                    painter.line_segment([egui::pos2(x, y - h), egui::pos2(x, y + h)], stroke);
                }
            }
            Kind::Angle => {
                let o = r.left_bottom();
                let len = r.width();
                let a = 50f32.to_radians();
                let tip = o + len * egui::vec2(a.cos(), -a.sin());
                painter.line_segment([o, r.right_bottom()], stroke);
                painter.line_segment([o, tip], stroke);
                let arc: Vec<_> = (0..=8)
                    .map(|i| {
                        let t = a * i as f32 / 8.0;
                        o + len * 0.55 * egui::vec2(t.cos(), -t.sin())
                    })
                    .collect();
                painter.add(egui::Shape::line(arc, stroke));
            }
            Kind::Size | Kind::Number | Kind::Text => {
                let text = match kind {
                    Kind::Size => "Tt",
                    Kind::Number => "#",
                    _ => "abc",
                };
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(side * 0.7),
                    color,
                );
            }
            Kind::Color => {
                let d = r.width() * 0.22;
                let c = r.center();
                for (k, rgb) in [[230, 70, 70], [70, 180, 90], [70, 120, 230]]
                    .into_iter()
                    .enumerate()
                {
                    let t = (90.0 + 120.0 * k as f32).to_radians();
                    let [red, green, blue] = rgb;
                    painter.circle_filled(
                        c + d * egui::vec2(t.cos(), -t.sin()),
                        r.width() * 0.3,
                        egui::Color32::from_rgba_unmultiplied(red, green, blue, 200),
                    );
                }
            }
        }
    }
    response.on_hover_text(kind.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::variables::{Applied, apply};
    use crate::document::{Layer, Node};

    /// [`type_in`], clicking the field `x` points from the left.
    fn type_in_at(
        ctx: &egui::Context,
        bindings: &mut Bindings,
        value: &mut f32,
        text: &str,
        x: f32,
    ) -> bool {
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut changed = false;
        let p = egui::pos2(x, 10.0);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let steps = vec![
            vec![egui::Event::PointerMoved(p)],
            vec![button(true)],
            vec![button(false)],
            vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
            vec![egui::Event::Text(text.into())],
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            vec![],
        ];
        let start = ctx.input(|i| i.time);
        for (i, events) in steps.into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                time: Some(start + 0.1 * (i + 1) as f64),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                publish(ui.ctx(), &doc);
                changed |= field(ui, bindings, Field::X, value, |d| d).changed();
            });
            out.textures_delta.clear();
        }
        changed
    }

    #[test]
    fn a_field_linked_to_a_removed_variable_keeps_its_value_until_replaced() {
        let ctx = egui::Context::default();
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut bindings = Bindings::from([(Field::X, "gone + 2".to_owned())]);
        let mut x = 12.0;
        let texts = texts_of(&ctx, |ui| {
            publish(ui.ctx(), &doc);
            field(ui, &mut bindings, Field::X, &mut x, |d| d);
        });
        assert!(
            !texts.iter().any(|t| t.contains("gone")),
            "the value, not the formula: {texts:?}"
        );
        assert!(texts.iter().any(|t| t.starts_with("12")), "{texts:?}");
        assert_eq!(bindings[&Field::X], "gone + 2", "still linked, broken");

        // Past the broken link icon: a variable links it again, a number unlinks it.
        assert!(type_in_at(&ctx, &mut bindings, &mut x, "button", 40.0));
        assert_eq!((x, bindings[&Field::X].as_str()), (30.0, "button"));
        bindings.insert(Field::X, "gone".into());
        assert!(type_in_at(&ctx, &mut bindings, &mut x, "7", 40.0));
        assert_eq!(x, 7.0);
        assert!(bindings.is_empty());
    }

    #[test]
    fn removing_a_variable_in_use_asks_first_and_leaves_its_links_broken() {
        let mut l = Layer::new(1, "img".into(), "x.png".into());
        l.bindings.insert(Field::X, "button + 2".into());
        let mut doc = doc_with(l);
        let ctx = egui::Context::default();
        let confirm_id = Id::unique("confirm_remove_variable");
        ctx.data_mut(|d| d.insert_temp(confirm_id, "button".to_owned()));
        let frame = |events: Vec<egui::Event>, doc: &mut Document| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                variables_ui(ui, doc, &Kind::ALL);
            });
            out.textures_delta.clear();
            out.shapes
        };
        // The dialog sizes itself on its first frame, and is drawn on the next.
        frame(Vec::new(), &mut doc);
        let shapes = frame(Vec::new(), &mut doc);
        let find = |text: &str| {
            shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.size() / 2.0)
                }
                _ => None,
            })
        };
        assert!(find("Remove the variable button?").is_some());
        assert!(
            find("It is used by 1 field or color, which keeps its current value and shows a broken link icon.")
                .is_some()
        );
        let ok = find("Remove").expect("the Remove button");
        for events in [
            vec![egui::Event::PointerMoved(ok)],
            vec![egui::Event::PointerButton {
                pos: ok,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: ok,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            frame(events, &mut doc);
        }
        assert!(doc.variables.iter().all(|v| v.name != "button"));
        assert_eq!(
            doc.layer(1).unwrap().bindings[&Field::X],
            "button + 2",
            "the field keeps its link, broken"
        );
        assert!(ctx.data(|d| d.get_temp::<String>(confirm_id)).is_none());
    }

    #[test]
    fn the_fx_button_edits_the_formula_with_the_variables_the_field_takes() {
        let ctx = egui::Context::default();
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut bindings = Bindings::new();
        let mut x = 1.0;
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, bindings: &mut Bindings, x: &mut f32| {
            time.set(time.get() + 0.1);
            let input = egui::RawInput {
                time: Some(time.get()),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut fx = egui::Pos2::ZERO;
            let mut out = ctx.run_ui(input, |ui| {
                publish(ui.ctx(), &doc);
                let r = field(ui, bindings, Field::X, x, |d| d);
                // The fx button is inside the field, at its right end.
                fx = egui::pos2(r.rect.right() - r.rect.height() / 2.0, r.rect.center().y);
            });
            out.textures_delta.clear();
            // Settle what the frame changed, as the app does.
            let mut d = doc.clone();
            let mut l = Layer::new(1, "l".into(), "".into());
            l.bindings = bindings.clone();
            l.transform.center[0] = *x;
            d.layers = vec![Node::Layer(Box::new(l))];
            apply(&mut d, &mut Applied::default(), &[]);
            *x = d.layer(1).unwrap().transform.center[0];
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
            (fx, texts)
        };
        let click = |p: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            [
                vec![egui::Event::PointerMoved(p)],
                vec![press(true)],
                vec![press(false)],
            ]
        };
        let mut run = |events: Vec<egui::Event>| frame(events, &mut bindings, &mut x);
        let open =
            |run: &mut dyn FnMut(Vec<egui::Event>) -> (egui::Pos2, Vec<(String, egui::Pos2)>)| {
                let (fx, _) = run(Vec::new());
                for events in click(fx) {
                    run(events);
                }
                for _ in 0..5 {
                    run(Vec::new());
                }
                run(Vec::new()).1
            };
        let find = |texts: &[(String, egui::Pos2)], t: &str| {
            texts
                .iter()
                .find(|(s, _)| s == t)
                .unwrap_or_else(|| panic!("{t} in {texts:?}"))
                .1
        };
        let texts = open(&mut run);
        let names: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            names.contains(&"Result: 1.00 mm"),
            "starts from the value: {names:?}"
        );
        // OK at the right edge, Cancel on its left, the editor as wide as its contents.
        let (ok, cancel) = (find(&texts, "OK"), find(&texts, "Cancel"));
        let title = find(&texts, "Formula");
        assert!(ok.x > cancel.x, "OK right of Cancel");
        assert!(
            ok.x - title.x < FORMULA_EDITOR_WIDTH + 10.0,
            "{ok:?} {title:?}"
        );
        assert!(
            !names.contains(&"Unlink, keeping the value"),
            "not linked yet"
        );
        assert!(names.contains(&"marge_é   2.5"), "numbers too: {names:?}");
        assert!(
            !names.iter().any(|t| t.starts_with("tilt")),
            "not angles: {names:?}"
        );
        // A click on a variable replaces the value with it.
        for events in click(find(&texts, "button   30 mm")) {
            run(events);
        }
        let texts = run(Vec::new()).1;
        assert!(
            texts.iter().any(|(t, _)| t == "Result: 30.00 mm"),
            "{texts:?}"
        );
        for events in click(find(&texts, "OK")) {
            run(events);
        }
        run(Vec::new());
        let texts = run(Vec::new()).1;
        let shown: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
        assert!(shown.contains(&"30"), "the result: {shown:?}");
        assert!(!shown.contains(&"button"), "not the formula: {shown:?}");
        assert_eq!(bindings[&Field::X], "button");
        assert_eq!(x, 30.0);

        let mut run = |events: Vec<egui::Event>| frame(events, &mut bindings, &mut x);
        let texts = open(&mut run);
        for events in click(find(&texts, "Unlink, keeping the value")) {
            run(events);
        }
        run(Vec::new());
        assert!(bindings.is_empty());
        assert_eq!(x, 30.0, "the value stays");

        // A plain number, with Enter: the field takes it.
        let mut run = |events: Vec<egui::Event>| frame(events, &mut bindings, &mut x);
        open(&mut run);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        run(vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
        run(vec![egui::Event::Text("4 * 3".into())]);
        run(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        run(Vec::new());
        assert!(bindings.is_empty());
        assert_eq!(x, 12.0);
    }

    #[test]
    fn a_text_variable_shows_its_name_and_is_edited_in_a_box() {
        let mut doc = Document {
            variables: vec![var("title", Value::Text("Hello\nWorld".into()))],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, doc: &mut Document| {
            time.set(time.get() + 0.1);
            let input = egui::RawInput {
                time: Some(time.get()),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                variables_ui(ui, doc, &Kind::ALL);
            });
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.pos + t.galley.size() / 2.0))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let texts = frame(Vec::new(), &mut doc);
        let name = texts.iter().find(|(t, _)| t == "title").expect("the name");
        let value = texts
            .iter()
            .find(|(t, _)| t == "Hello")
            .expect("the first line");
        assert!(name.1.x < value.1.x, "name, then value: {texts:?}");
        assert!(!texts.iter().any(|(t, _)| t.contains("World")), "{texts:?}");

        let at = value.1;
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![press(true)],
            vec![press(false)],
        ] {
            frame(events, &mut doc);
        }
        for _ in 0..5 {
            frame(Vec::new(), &mut doc);
        }
        let texts = frame(Vec::new(), &mut doc);
        assert!(
            texts.iter().any(|(t, _)| t == "Hello\nWorld"),
            "the box: {texts:?}"
        );
        frame(vec![egui::Event::Text("!".into())], &mut doc);
        let Value::Text(t) = &doc.variables[0].value else {
            unreachable!()
        };
        assert!(t.contains('!') && t.len() == "Hello\nWorld!".len(), "{t:?}");
    }

    #[test]
    fn the_section_lists_variables_by_kind_then_the_add_menu() {
        let mut doc = Document {
            variables: vec![
                var("ink", Value::Color([1, 2, 3, 255])),
                var("button", Value::Length(30.0)),
                var("count", Value::Number(3.0)),
            ],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            variables_ui(ui, &mut doc, &Kind::ALL);
        });
        out.textures_delta.clear();
        let texts: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        let at = |t: &str| texts.iter().position(|x| x == t);
        let order = ["button", "#", "count", "ink", "Add a variable"]
            .map(|t| at(t).unwrap_or_else(|| panic!("{t} in {texts:?}")));
        assert!(order.is_sorted(), "{texts:?}");
        assert!(
            at("Length (mm)").is_none() && at("Number").is_none() && at("Color").is_none(),
            "icons, not headings: {texts:?}"
        );
        let value = at("30.0").expect("the length's value");
        assert_eq!(
            texts[value - 1],
            "mm",
            "its unit, outside the field: {texts:?}"
        );
    }

    /// Draws the chain icons, whole and broken, at a few sizes and colors, to the PNG
    /// named by `LINK_ICON_PREVIEW`.
    #[test]
    #[ignore]
    fn link_icon_preview() {
        let Ok(path) = std::env::var("LINK_ICON_PREVIEW") else {
            return;
        };
        crate::ui::preview::render(&path, egui::vec2(260.0, 90.0), |ui| {
            for broken in [false, true] {
                ui.horizontal(|ui| {
                    for side in [14.0, 18.0, 24.0, 32.0] {
                        for color in [ui.visuals().text_color(), ui.visuals().warn_fg_color] {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                            paint_link(ui.painter(), rect, color, broken);
                        }
                    }
                });
            }
        });
    }

    fn var(name: &str, value: Value) -> Variable {
        Variable {
            formula: None,
            name: name.into(),
            value,
        }
    }

    fn vars() -> Vec<Variable> {
        vec![
            var("button", Value::Length(30.0)),
            var("marge_é", Value::Number(2.5)),
            var("tilt", Value::Angle(30.0)),
            var("ink", Value::Color([1, 2, 3, 255])),
        ]
    }

    fn doc_with(layer: Layer) -> Document {
        Document {
            layers: vec![Node::Layer(Box::new(layer))],
            variables: vars(),
            ..Default::default()
        }
    }

    /// Clicks the field, selects its text, types `text` and presses Enter, a frame per
    /// step. Returns whether the field reported a change.
    fn type_in(ctx: &egui::Context, bindings: &mut Bindings, value: &mut f32, text: &str) -> bool {
        type_in_at(ctx, bindings, value, text, 20.0)
    }

    /// The texts drawn by a frame of `ui`.
    fn texts_of(ctx: &egui::Context, ui: impl FnMut(&mut Ui)) -> Vec<String> {
        let mut ui = ui;
        let mut out = ctx.run_ui(egui::RawInput::default(), |u| ui(u));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn typing_a_formula_links_the_field_and_a_number_unlinks_it() {
        let ctx = egui::Context::default();
        let mut bindings = Bindings::new();
        let mut x = 1.0;
        assert!(type_in(&ctx, &mut bindings, &mut x, "button + 2"));
        assert_eq!(x, 32.0);
        assert_eq!(bindings[&Field::X], "button + 2");

        // A formula without variables is just a number.
        let mut other = Bindings::new();
        let mut y = 1.0;
        type_in(&egui::Context::default(), &mut other, &mut y, "3 * 4");
        assert_eq!(y, 12.0);
        assert!(other.is_empty());

        // Linked, the field holds the formula; a number unlinks it.
        assert!(type_in(&ctx, &mut bindings, &mut x, "7.5"));
        assert_eq!(x, 7.5);
        assert!(bindings.is_empty());
    }
}
