//! Layer list, its toolbar and the selected layer's or group's properties.

mod icons;
mod ops;
mod properties;
mod shape_properties;
mod text_properties;
use icons::{
    TOOL, aspect_lock, clip_mark, error_mark, expander, eye_toggle, folder_icon, lock_toggle,
    thumbnail,
};
pub use icons::{Tool, icon_entry, tool_button, tool_icon};
use ops::name_mut;
pub use ops::{
    align, can_clip, can_step, delete, drop_allowed, drop_node, duplicate, flip_group,
    give_fresh_ids, group, group_middle, insertion_path, layers_bounds, linked_axes,
    movable_layers, move_step, move_to_end, name_copy, nudge, random_name, rotate_group,
    step_selection, toggle_clip, turned_about, ungroup,
};
use properties::{PROP_ROW_GAP, StyleIcon, choice, mm_value, prop_grid, style_toggle};
pub use properties::{field_frames, properties_panel, section, unit};
use shape_properties::{shape_section, shape_size};
use text_properties::text_section;

use egui::collapsing_header::CollapsingState;
use egui::{
    Button, Color32, DragAndDrop, DragValue, Pos2, Rect, Response, RichText, Sense, Shape, Slider,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

use std::collections::HashMap;

use crate::document::colors::ColorField;
use crate::document::shape::{self, Cap, DashStyle, Join, ShapeKind};
use crate::document::text::{self, FontLibrary, PathKind, TextAlign};
use crate::document::variables::{self, Field};
use crate::document::{BlendMode, Document, Group, Layer, Node, P2};
use crate::i18n::{tr, tr_noop, translate};
use crate::template::ElementKind;
use crate::ui::effects as effects_ui;
use crate::ui::shortcuts::Command;
use crate::ui::variables as variables_ui;

/// Width of one nesting level in the layer list.
const INDENT: f32 = 14.0;
/// How near the line between two rows of the layer list Alt+click clips, in points.
const CLIP_BAND: f32 = 4.0;
const THUMB: f32 = 28.0;
const ICON: f32 = 18.0;

/// State of the layers panel that lasts across frames.
#[derive(Default)]
pub struct LayersPanel {
    /// The layer whose pattern fill is to get an image: the app opens the file dialog.
    pub pick_pattern: Option<u64>,
    /// The angle in a group's "Rotate by" field, in degrees clockwise.
    pub group_turn_deg: f32,
    /// Width and height can be changed separately in Properties.
    pub free_aspect: bool,
    /// What the Align buttons line up with.
    pub align_to: AlignTo,
    /// Open the selected text layer's sections, for a new text layer.
    pub show_text: bool,
    /// Typed in the font list to narrow it.
    pub font_filter: String,
    /// Kind of shape added last, added again by the shortcut.
    pub shape_kind: ShapeKind,
    /// Kind of canvas (a closed shape to paint in) added last, added again by the
    /// shortcut.
    pub canvas_kind: ShapeKind,
    /// The shape layer whose points are being edited on the canvas.
    pub edit_points: Option<u64>,
    /// Image layers printed below this resolution (DPI) are flagged; set by the app from
    /// the export settings.
    pub min_dpi: f32,
    /// Something was cut or copied, for the rows' Paste entry; set by the app.
    pub can_paste: bool,
    /// Each command's shortcut as text, for tooltips and menus; set by the app.
    pub keys: HashMap<Command, String>,
    /// A template is edited: only shapes are added, into its elements.
    pub template_mode: bool,
    /// The layer list was clicked last: Up and Down step through its rows.
    pub list_focused: bool,
    /// Scroll the list to the selected row, chosen with the keyboard.
    pub scroll_to_selected: bool,
}

impl LayersPanel {
    /// A tooltip followed by its command's shortcut.
    fn hint(&self, text: &str, command: Command) -> String {
        crate::ui::shortcuts::with_key(text, self.keys.get(&command))
    }
}

/// What the Align buttons line the selection up with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlignTo {
    #[default]
    Page,
    Template,
}

/// An edge or center line to align to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
}

impl Align {
    pub const ALL: [Align; 6] = [
        Align::Left,
        Align::CenterX,
        Align::Right,
        Align::Top,
        Align::CenterY,
        Align::Bottom,
    ];

    /// Tooltip of its button.
    fn tip(self) -> &'static str {
        match self {
            Align::Left => tr!("ALIGN_LEFT"),
            Align::CenterX => tr!("ALIGN_CENTER_HORIZONTALLY"),
            Align::Right => tr!("ALIGN_RIGHT"),
            Align::Top => tr!("ALIGN_TOP"),
            Align::CenterY => tr!("ALIGN_CENTER_VERTICALLY"),
            Align::Bottom => tr!("ALIGN_BOTTOM"),
        }
    }
}

/// What happened in the panel this frame, for the app to act on.
#[derive(Default)]
pub struct LayersOutput {
    /// The document was modified.
    pub changed: bool,
    /// The Add button was clicked: open the image dialog.
    pub add_images: bool,
    /// The Text button was clicked: add a text layer.
    pub add_text: bool,
    /// A kind was picked from the Shape button's menu: add a shape layer.
    pub add_shape: Option<ShapeKind>,
    /// A kind was picked from the Canvas button's menu: add a closed shape to paint in.
    pub add_canvas: Option<ShapeKind>,
    /// Background was picked from the Shape button's menu: add a background layer.
    pub add_background: bool,
    /// Cut, Copy or Paste was picked in a row's menu, that row now selected.
    pub edit: Option<Edit>,
    /// A kind was picked from the Element button's menu: add a template element.
    pub add_element: Option<ElementKind>,
    /// The Reference button was clicked: choose the template's reference file.
    pub choose_reference: bool,
    /// The height the layer list and its toolbar would take without a limit.
    pub natural_height: f32,
}

/// Clipboard commands, which the app carries out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit {
    Cut,
    Copy,
    Paste,
}

/// Drag-and-drop payload in the layer list: the id of the dragged node.
struct DraggedNode(u64);

/// Where a dragged node goes, relative to a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropTarget {
    /// Just above the row in the list (higher in the stack).
    Above(u64),
    /// Just below the row in the list.
    Below(u64),
    /// Top of the group's children.
    Into(u64),
}

// ---- Layer operations, shared with keyboard shortcuts ----

// ---- Icons ----

// ---- Property grids ----

// ---- Panel ----

/// One row of the layer list.
struct Row {
    path: Vec<usize>,
    id: u64,
    depth: usize,
    /// A parent group is hidden / locked.
    parent_hidden: bool,
    parent_locked: bool,
}

/// Rows of the layer list, top of the stack first, skipping collapsed groups' children.
fn rows(doc: &Document) -> Vec<Row> {
    fn walk(nodes: &[Node], path: &mut Vec<usize>, hidden: bool, locked: bool, out: &mut Vec<Row>) {
        for (i, n) in nodes.iter().enumerate().rev() {
            path.push(i);
            out.push(Row {
                path: path.clone(),
                id: n.id(),
                depth: path.len() - 1,
                parent_hidden: hidden,
                parent_locked: locked,
            });
            if let Node::Group(g) = n
                && g.expanded
            {
                walk(
                    &g.children,
                    path,
                    hidden || !g.visible,
                    locked || g.locked,
                    out,
                );
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(&doc.layers, &mut Vec::new(), false, false, &mut out);
    out
}

/// The Layers panel: the list of layers and groups, and its toolbar.
pub fn show(
    ui: &mut Ui,
    doc: &mut Document,
    selected: &mut Option<u64>,
    state: &mut LayersPanel,
) -> LayersOutput {
    let mut out = LayersOutput::default();
    let sel = selected.filter(|&id| doc.path_of(id).is_some());
    let has = sel.is_some();
    let is_group = sel.and_then(|id| doc.node(id)?.as_group()).is_some();

    // Toolbar above the list, always in sight however long the list.
    let top = ui.cursor().top();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let id = sel.unwrap_or(0);
        // One button per kind of layer, so each says what it adds.
        // A template is made of shapes, in its elements.
        if state.template_mode {
            let button = tool_icon(ui, Tool::AddElement, true, tr!("ADD_ELEMENT_HINT"));
            // A template has one outline.
            let has_outline = doc.layers.iter().any(|n| {
                n.as_group()
                    .and_then(|g| g.element)
                    .is_some_and(|t| t.kind == ElementKind::Outline)
            });
            egui::Popup::menu(&button).show(|ui| {
                for kind in ElementKind::ALL {
                    let taken = kind == ElementKind::Outline && has_outline;
                    let r = ui
                        .add_enabled(!taken, egui::Button::new(translate(kind.label())))
                        .on_disabled_hover_text(tr!("TEMPLATE_HAS_OUTLINE"));
                    if r.clicked() {
                        out.add_element = Some(kind);
                        ui.close();
                    }
                }
            });
            let hint = state.hint(
                tr!("CHOOSE_TEMPLATE_REFERENCE_HINT"),
                Command::ChooseReference,
            );
            if tool_button(ui, Tool::AddImage, true, &hint) {
                out.choose_reference = true;
            }
        } else {
            let hint = state.hint(tr!("ADD_IMAGE_LAYERS"), Command::AddImages);
            if tool_button(ui, Tool::AddImage, true, &hint) {
                out.add_images = true;
            }
            let hint = state.hint(tr!("ADD_TEXT_LAYER"), Command::AddText);
            if tool_button(ui, Tool::AddText, true, &hint) {
                out.add_text = true;
            }
        }
        let hint = match (state.keys.get(&Command::AddShape), state.template_mode) {
            (Some(s), false) => tr!("ADD_SHAPE_LAYER_WITH_SHORTCUT", shortcut = s),
            (None, false) => tr!("ADD_SHAPE_LAYER").to_owned(),
            (Some(s), true) => tr!("ADD_TEMPLATE_SHAPE_WITH_SHORTCUT", shortcut = s),
            (None, true) => tr!("ADD_TEMPLATE_SHAPE").to_owned(),
        };
        // In a template, shapes go into an element: one must be selected (or a shape of
        // it).
        let can_add_shape = !state.template_mode
            || sel.is_some_and(|id| crate::template::edit::element_of(doc, id).is_some());
        let hint = if can_add_shape {
            hint
        } else {
            tr!("ADD_SHAPE_SELECT_ELEMENT").to_owned()
        };
        let button = tool_icon(ui, Tool::AddShape, can_add_shape, &hint);
        egui::Popup::menu(&button).show(|ui| {
            for kind in ShapeKind::ALL {
                let mut b = Button::new(translate(kind.name_key()));
                // The shortcut adds the kind added last.
                if kind == state.shape_kind
                    && let Some(s) = state.keys.get(&Command::AddShape)
                {
                    b = b.shortcut_text(s.as_str());
                }
                if ui.add(b).clicked() {
                    out.add_shape = Some(kind);
                    ui.close();
                }
            }
            // Paths come from SVG files: a dialog asks for them.
            ui.separator();
            let mut b = Button::new(tr!("ADD_PATH_FROM_SVG"));
            if state.shape_kind == ShapeKind::Path
                && let Some(s) = state.keys.get(&Command::AddShape)
            {
                b = b.shortcut_text(s.as_str());
            }
            if ui
                .add(b)
                .on_hover_text(tr!("ADD_PATH_FROM_SVG_HINT"))
                .clicked()
            {
                out.add_shape = Some(ShapeKind::Path);
                ui.close();
            }
            // A template has no outline of its own to fill.
            if !state.template_mode {
                let mut b = Button::new(tr!("ADD_BACKGROUND_LAYER"));
                if let Some(s) = state.keys.get(&Command::AddBackground) {
                    b = b.shortcut_text(s.as_str());
                }
                if ui
                    .add(b)
                    .on_hover_text(tr!("ADD_BACKGROUND_LAYER_HINT"))
                    .clicked()
                {
                    out.add_background = true;
                    ui.close();
                }
            }
        });
        // A template has no canvases, groups or stacking order: only shapes in its
        // elements.
        if !state.template_mode {
            let hint = match state.keys.get(&Command::AddCanvas) {
                Some(s) => tr!("ADD_CANVAS_LAYER_WITH_SHORTCUT", shortcut = s),
                None => tr!("ADD_CANVAS_LAYER").to_owned(),
            };
            let button = tool_icon(ui, Tool::AddCanvas, true, &hint);
            egui::Popup::menu(&button).show(|ui| {
                // Only a closed shape has an inside to paint in.
                for kind in ShapeKind::ALL.into_iter().filter(|k| k.closed()) {
                    let mut b = Button::new(translate(kind.name_key()));
                    if kind == state.canvas_kind
                        && let Some(s) = state.keys.get(&Command::AddCanvas)
                    {
                        b = b.shortcut_text(s.as_str());
                    }
                    if ui.add(b).clicked() {
                        out.add_canvas = Some(kind);
                        ui.close();
                    }
                }
            });
            ui.add_space(6.0);
            let hint = state.hint(tr!("GROUP_LAYERS"), Command::Group);
            if tool_button(ui, Tool::Group, true, &hint) {
                out.changed |= group(doc, selected);
            }
            let hint = state.hint(tr!("UNGROUP_LAYERS"), Command::Ungroup);
            if tool_button(ui, Tool::Ungroup, is_group, &hint) {
                out.changed |= ungroup(doc, selected);
            }
            let hint = state.hint(tr!("DUPLICATE_LAYER"), Command::Duplicate);
            if tool_button(ui, Tool::Duplicate, has, &hint) {
                out.changed |= duplicate(doc, selected);
            }
            ui.add_space(6.0);
            if tool_button(ui, Tool::Top, has, tr!("MOVE_LAYER_TO_TOP")) {
                out.changed |= move_to_end(doc, id, true);
            }
            if tool_button(
                ui,
                Tool::Up,
                has && can_step(doc, id, true),
                tr!("MOVE_LAYER_UP"),
            ) {
                out.changed |= move_step(doc, id, true);
            }
            if tool_button(
                ui,
                Tool::Down,
                has && can_step(doc, id, false),
                tr!("MOVE_LAYER_DOWN"),
            ) {
                out.changed |= move_step(doc, id, false);
            }
            if tool_button(ui, Tool::Bottom, has, tr!("MOVE_LAYER_TO_BOTTOM")) {
                out.changed |= move_to_end(doc, id, false);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let hint = state.hint(tr!("DELETE_LAYER"), Command::Delete);
            if tool_button(ui, Tool::Delete, has, &hint) {
                out.changed |= delete(doc, selected);
            }
        });
    });
    ui.separator();
    let toolbar = ui.cursor().top() - top;

    // Layer list, top of the stack first.
    let mut pending_drop = None;
    let mut pending_duplicate = None;
    let mut pending_edit = None;
    let mut pending_clip = None;
    // The list takes the rest of the panel's height.
    let list = egui::ScrollArea::vertical()
        .id_salt("layer_list")
        .max_height(ui.available_height().max(TOOL))
        .show(ui, |ui| {
            if doc.layers.is_empty() {
                let text = if state.template_mode {
                    tr!("NO_ELEMENTS_YET")
                } else {
                    tr!("NO_LAYERS_YET")
                };
                crate::ui::callout::callout(ui, crate::ui::callout::Tone::Info, text);
            }
            let full = ui.max_rect();
            let broken_links = variables::broken_links_by_node(doc);
            let all_rows = rows(doc);
            for (k, row) in all_rows.iter().enumerate() {
                // The row below, when it is the node right under this one in the same
                // list: Alt+click between them clips this one to it.
                let sibling_below = all_rows.get(k + 1).filter(|b| {
                    b.path.len() == row.path.len()
                        && b.path[..b.path.len() - 1] == row.path[..row.path.len() - 1]
                        && b.path.last().map(|i| i + 1) == row.path.last().copied()
                });
                // The template's reference: at the bottom, apart from the elements.
                let reference = crate::template::edit::is_reference(doc, row.id);
                let can_clip_row = !state.template_mode && row.path.last().is_some_and(|&i| i > 0);
                let clipped = doc.node_at(&row.path).is_some_and(Node::clip);
                if reference {
                    ui.separator();
                }
                let Some(node) = doc.node_at_mut(&row.path) else {
                    continue;
                };
                // Room for the selection background, painted once the row's size is known.
                let background = ui.painter().add(egui::Shape::Noop);
                let r = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.add_space(row.depth as f32 * INDENT);
                    match node {
                        Node::Group(g) => expander(ui, &mut g.expanded),
                        Node::Layer(_) => {
                            ui.add_space(ICON * 0.8);
                        }
                    }
                    if node.clip() && row.path.last().is_some_and(|&i| i > 0) {
                        clip_mark(ui);
                    }
                    let (visible, locked) = match node {
                        Node::Group(g) => (&mut g.visible, &mut g.locked),
                        Node::Layer(l) => (&mut l.visible, &mut l.locked),
                    };
                    out.changed |= eye_toggle(ui, visible, row.parent_hidden).changed();
                    out.changed |= lock_toggle(ui, locked, row.parent_locked).changed();
                    // In a template, a shape in no element: an error, until moved into one.
                    let orphan = state.template_mode
                        && !reference
                        && row.depth == 0
                        && match &*node {
                            Node::Group(g) => g.element.is_none(),
                            Node::Layer(_) => true,
                        };
                    if orphan {
                        error_mark(ui, tr!("TEMPLATE_ORPHAN_SHAPE"));
                    }
                    match &*node {
                        Node::Group(_) => folder_icon(ui),
                        Node::Layer(l) => {
                            let tex = l.shown_texture().map(|t| t.id());
                            thumbnail(ui, tex, l.size_px, l.image_uv());
                        }
                    }
                    // Visible image layers that will print below the chosen resolution.
                    let low_dpi = node
                        .as_layer()
                        .filter(|l| l.visible && !row.parent_hidden)
                        .and_then(Layer::print_dpi)
                        .filter(|&d| d < state.min_dpi);
                    if let Some(dpi) = low_dpi {
                        ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                            .on_hover_text(tr!(
                                "LAYER_LOW_RESOLUTION",
                                dpi = format!("{dpi:.0}"),
                                min = format!("{:.0}", state.min_dpi)
                            ));
                    }
                    let broken = node.as_layer().filter(|l| l.broken);
                    let missing = broken.map(|l| l.source.display().to_string());
                    let name = name_mut(node);
                    {
                        let mut text = RichText::new(name.as_str());
                        if missing.is_some() || orphan {
                            text = text.color(ui.visuals().error_fg_color);
                        }
                        if row.parent_hidden {
                            text = text.weak();
                        }
                        let label = name.clone();
                        // The name takes the rest of the row, so short names are as easy to
                        // click (and drag) as long ones. No frame of its own: the whole row
                        // shows hover and selection. Renaming is done in Properties.
                        // Linked to variables that no longer exist (a group: inside it): a
                        // badge at the right end of the row, after the name.
                        let gone = broken_links.get(&row.id);
                        let badge = if gone.is_some() {
                            ui.spacing().interact_size.y + ui.spacing().item_spacing.x
                        } else {
                            0.0
                        };
                        let size = vec2(ui.available_width() - badge, ui.spacing().interact_size.y);
                        let (name_rect, r) = ui.allocate_exact_size(size, Sense::click_and_drag());
                        if let Some(gone) = gone {
                            variables_ui::broken_links_badge(ui, gone);
                        }
                        if ui.is_rect_visible(name_rect) {
                            let color = if *selected == Some(row.id) {
                                ui.visuals().selection.stroke.color
                            } else {
                                ui.visuals().text_color()
                            };
                            let galley = egui::WidgetText::from(text).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                name_rect.width(),
                                egui::TextStyle::Button,
                            );
                            let pos = egui::pos2(
                                name_rect.left(),
                                name_rect.center().y - galley.size().y / 2.0,
                            );
                            ui.painter().galley(pos, galley, color);
                        }
                        // The reference stays where it is, and is no layer to copy.
                        if !reference {
                            r.dnd_set_drag_payload(DraggedNode(row.id));
                        }
                        if !reference {
                            r.context_menu(|ui| {
                                let entry = |key: &str, c: Command| {
                                    let b = Button::new(translate(key));
                                    match state.keys.get(&c) {
                                        Some(s) => b.shortcut_text(s.as_str()),
                                        None => b,
                                    }
                                };
                                for (edit, key, c, enabled) in [
                                    (Edit::Cut, tr_noop!("CUT_LAYER"), Command::Cut, true),
                                    (Edit::Copy, tr_noop!("COPY_LAYER"), Command::Copy, true),
                                    (
                                        Edit::Paste,
                                        tr_noop!("PASTE_LAYER"),
                                        Command::Paste,
                                        state.can_paste,
                                    ),
                                ] {
                                    if ui.add_enabled(enabled, entry(key, c)).clicked() {
                                        pending_edit = Some((row.id, edit));
                                    }
                                }
                                if ui
                                    .add(entry(tr_noop!("DUPLICATE_LAYER"), Command::Duplicate))
                                    .clicked()
                                {
                                    pending_duplicate = Some(row.id);
                                }
                                // A template has no stacking order to clip with.
                                if !state.template_mode {
                                    let clip =
                                        entry(tr_noop!("CLIP_TO_LAYER_BELOW"), Command::Clip)
                                            .selected(clipped);
                                    if ui
                                        .add_enabled(clipped || can_clip_row, clip)
                                        .on_hover_text(tr!("CLIP_TO_LAYER_BELOW_HINT"))
                                        .clicked()
                                    {
                                        pending_clip = Some(row.id);
                                    }
                                }
                            });
                        }
                        if r.dragged()
                            && let Some(p) = ui.ctx().pointer_interact_pos()
                        {
                            // The name follows the pointer while dragging.
                            egui::Area::new(egui::Id::unique(("layer-drag", row.id)))
                                .order(egui::Order::Tooltip)
                                .interactable(false)
                                .fixed_pos(p + vec2(12.0, 4.0))
                                .show(ui.ctx(), |ui| {
                                    egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(label));
                                });
                        }
                        let r = match &missing {
                            Some(path) => r.on_hover_text(tr!("LAYER_FILE_MISSING", path = path)),
                            None => r,
                        };
                        if r.clicked() {
                            *selected = Some(row.id);
                        }
                    }
                });

                // Drop target: upper/lower part of the row, or the middle of a group row.
                let rect = Rect::from_x_y_ranges(full.x_range(), r.response.rect.y_range());
                if state.scroll_to_selected && *selected == Some(row.id) {
                    ui.scroll_to_rect(rect, None);
                    state.scroll_to_selected = false;
                }
                // The whole row, not only the name, shows the selection and the hover.
                let fill = if *selected == Some(row.id) {
                    Some(ui.visuals().selection.bg_fill)
                } else if ui.rect_contains_pointer(rect)
                    && DragAndDrop::payload::<DraggedNode>(ui.ctx()).is_none()
                {
                    Some(ui.visuals().widgets.hovered.weak_bg_fill)
                } else {
                    None
                };
                if let Some(fill) = fill {
                    ui.painter().set(
                        background,
                        egui::epaint::RectShape::filled(rect.expand2(vec2(0.0, 1.0)), 2.0, fill),
                    );
                }
                // Alt over the line between this row and the layer right under it: a mark
                // on the line, and a click clips this one to that layer (or unclips it).
                if !state.template_mode
                    && sibling_below.is_some()
                    && let Some(p) = ui.ctx().pointer_hover_pos()
                    && ui.input(|i| i.modifiers.alt)
                    && (p.y - rect.bottom()).abs() <= CLIP_BAND
                    && rect.x_range().contains(p.x)
                    && DragAndDrop::payload::<DraggedNode>(ui.ctx()).is_none()
                {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Alias);
                    let left = rect.left() + row.depth as f32 * INDENT;
                    let stroke = Stroke::new(1.0, ui.visuals().selection.stroke.color);
                    ui.painter().add(egui::Shape::dashed_line(
                        &[
                            egui::pos2(left, rect.bottom()),
                            egui::pos2(rect.right(), rect.bottom()),
                        ],
                        stroke,
                        4.0,
                        3.0,
                    ));
                    if ui.input(|i| i.pointer.primary_clicked()) {
                        pending_clip = Some(row.id);
                    }
                }
                let hover = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p));
                if let (Some(p), Some(dragged)) =
                    (hover, DragAndDrop::payload::<DraggedNode>(ui.ctx()))
                    && dragged.0 != row.id
                    && !doc.is_within(row.id, dragged.0)
                {
                    let t = (p.y - rect.top()) / rect.height().max(1.0);
                    let target = match doc.node_at(&row.path) {
                        Some(Node::Group(g)) => {
                            if t < 0.25 {
                                DropTarget::Above(row.id)
                            } else if t > 0.75 && !g.expanded {
                                DropTarget::Below(row.id)
                            } else {
                                DropTarget::Into(row.id)
                            }
                        }
                        _ if t < 0.5 => DropTarget::Above(row.id),
                        _ => DropTarget::Below(row.id),
                    };
                    // Nowhere to go: no mark, nothing dropped.
                    let target = Some(target)
                        .filter(|&t| drop_allowed(doc, dragged.0, t, state.template_mode));
                    let accent = ui.visuals().selection.stroke.color;
                    let stroke = Stroke::new(2.0, accent);
                    let left = rect.left() + row.depth as f32 * INDENT;
                    let painter = ui.painter();
                    match target {
                        Some(DropTarget::Above(_)) => {
                            painter.hline(left..=rect.right(), rect.top(), stroke);
                        }
                        Some(DropTarget::Below(_)) => {
                            painter.hline(left..=rect.right(), rect.bottom(), stroke);
                        }
                        Some(DropTarget::Into(_)) => {
                            painter.rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
                        }
                        None => {}
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        DragAndDrop::clear_payload(ui.ctx());
                        pending_drop = target.map(|t| (dragged.0, t));
                    }
                }
            }
        });
    out.natural_height = list.content_size.y + toolbar;
    // A press in the list gives it the arrow keys, a press anywhere else takes them back.
    if let Some(p) = ui.input(|i| i.pointer.any_pressed().then(|| i.pointer.press_origin())) {
        state.list_focused = p.is_some_and(|p| list.inner_rect.contains(p));
    }
    if let Some((id, target)) = pending_drop {
        out.changed |= drop_node(doc, id, target);
    }
    if let Some(id) = pending_duplicate {
        *selected = Some(id);
        out.changed |= duplicate(doc, selected);
    }
    if let Some(id) = pending_clip {
        out.changed |= toggle_clip(doc, Some(id));
    }
    if let Some((id, edit)) = pending_edit {
        *selected = Some(id);
        out.edit = Some(edit);
    }

    out
}

#[cfg(test)]
mod tests;
