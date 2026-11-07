//! Layer list, its toolbar and the selected layer's or group's properties.

use egui::collapsing_header::CollapsingState;
use egui::{
    Button, Color32, DragAndDrop, DragValue, Pos2, Rect, Response, RichText, Sense, Shape, Slider,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::document::{Document, Group, Layer, Node, P2};
use crate::effects_ui;
use crate::i18n::{tr, tr_n, tr_noop, translate};
use crate::text::{self, FontLibrary, TextAlign};

/// Width of one nesting level in the layer list.
const INDENT: f32 = 14.0;
const THUMB: f32 = 28.0;
const ICON: f32 = 18.0;

/// State of the layers panel that lasts across frames.
#[derive(Default)]
pub struct LayersPanel {
    /// Width and height can be changed separately in Properties.
    pub free_aspect: bool,
    /// Corner handles on the canvas distort the layer instead of scaling it.
    pub distort: bool,
    /// What the Align buttons line up with.
    pub align_to: AlignTo,
    /// Put the keyboard in the selected text layer's text, with it all selected.
    pub focus_text: bool,
    /// Typed in the font list to narrow it.
    pub font_filter: String,
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

/// Gives a node a new id.
fn set_id(node: &mut Node, id: u64) {
    match node {
        Node::Group(g) => g.id = id,
        Node::Layer(l) => l.id = id,
    }
}

/// The node's opacity, layer or group.
fn opacity_mut(node: &mut Node) -> &mut f32 {
    match node {
        Node::Group(g) => &mut g.opacity,
        Node::Layer(l) => &mut l.opacity,
    }
}

/// The node's name, layer or group.
fn name_mut(node: &mut Node) -> &mut String {
    match node {
        Node::Group(g) => &mut g.name,
        Node::Layer(l) => &mut l.name,
    }
}

/// Where a new layer goes: at the top of a selected group, just above a selected layer,
/// or at the top of the stack.
pub fn insertion_path(doc: &mut Document, selected: Option<u64>) -> Vec<usize> {
    let Some(mut path) = selected.and_then(|id| doc.path_of(id)) else {
        return vec![doc.layers.len()];
    };
    if let Some(Node::Group(g)) = doc.node_at_mut(&path) {
        g.expanded = true;
        path.push(g.children.len());
    } else if let Some(last) = path.last_mut() {
        *last += 1;
    }
    path
}

/// Copies the selected node just above itself, moved 5 mm, with new ids, and selects the
/// copy.
pub fn duplicate(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    let Some(mut path) = selected.and_then(|id| doc.path_of(id)) else {
        return false;
    };
    let Some(mut copy) = doc.node_at(&path).cloned() else {
        return false;
    };
    copy.for_each_node_mut(&mut |n| set_id(n, doc.alloc_id()));
    copy.for_each_layer_mut(&mut |l| {
        l.transform.center[0] += 5.0;
        l.transform.center[1] += 5.0;
    });
    let name = name_mut(&mut copy);
    *name = tr!("DUPLICATE_LAYER_NAME", name = name);
    *selected = Some(copy.id());
    *path.last_mut().unwrap() += 1;
    doc.insert(&path, copy);
    true
}

/// Deletes the selected node and selects the one that took its place, the one below, or its
/// group.
pub fn delete(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    let Some((path, _)) = selected.and_then(|id| doc.remove(id)) else {
        return false;
    };
    let (&i, parent) = path.split_last().unwrap();
    let siblings = doc.list(parent).map(Vec::as_slice).unwrap_or_default();
    // Next selection: the node that took its place, the one below, or the parent group.
    *selected = siblings
        .get(i)
        .or(i.checked_sub(1).and_then(|j| siblings.get(j)))
        .map(Node::id)
        .or_else(|| doc.node_at(parent).map(Node::id));
    true
}

/// Moves the selected layer, or every unlocked layer of the selected group.
pub fn nudge(doc: &mut Document, selected: Option<u64>, d: P2) -> bool {
    let Some(id) = selected else { return false };
    let ids = movable_layers(doc, id);
    for &id in &ids {
        if let Some(l) = doc.layer_mut(id) {
            l.transform.center[0] += d[0];
            l.transform.center[1] += d[1];
        }
    }
    !ids.is_empty()
}

/// Page-mm bounds (min, max) of the given layers as drawn, rotation and distortion
/// included.
pub fn layers_bounds(doc: &Document, ids: &[u64]) -> Option<(P2, P2)> {
    ids.iter()
        .filter_map(|&id| doc.layer(id))
        .flat_map(|l| l.corners())
        .fold(None, |acc, p| {
            Some(match acc {
                None => (p, p),
                Some((lo, hi)) => (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                ),
            })
        })
}

/// Moves the selected layer, or the unlocked layers of the selected group as one block,
/// so that its bounding box lines up with `area` (min, max corners in page mm).
pub fn align(doc: &mut Document, selected: Option<u64>, how: Align, area: (P2, P2)) -> bool {
    let Some(id) = selected else { return false };
    let ids = movable_layers(doc, id);
    let Some((lo, hi)) = layers_bounds(doc, &ids) else {
        return false;
    };
    let (alo, ahi) = area;
    let mid = |a: f32, b: f32| (a + b) / 2.0;
    let d = match how {
        Align::Left => [alo[0] - lo[0], 0.0],
        Align::CenterX => [mid(alo[0], ahi[0]) - mid(lo[0], hi[0]), 0.0],
        Align::Right => [ahi[0] - hi[0], 0.0],
        Align::Top => [0.0, alo[1] - lo[1]],
        Align::CenterY => [0.0, mid(alo[1], ahi[1]) - mid(lo[1], hi[1])],
        Align::Bottom => [0.0, ahi[1] - hi[1]],
    };
    if d == [0.0, 0.0] {
        return false;
    }
    nudge(doc, selected, d)
}

/// Ids of the image layers that move with node `id`: itself or the unlocked layers it
/// holds.
pub fn movable_layers(doc: &Document, id: u64) -> Vec<u64> {
    let Some(path) = doc.path_of(id) else {
        return Vec::new();
    };
    if doc.effectively_locked(id) {
        return Vec::new();
    }
    doc.leaves()
        .into_iter()
        .filter(|leaf| leaf.path.starts_with(&path) && !leaf.locked)
        .map(|leaf| leaf.layer.id)
        .collect()
}

/// Wraps the selected node in a new group, or adds an empty group at the top.
pub fn group(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    let id = doc.alloc_id();
    fn count(nodes: &[Node]) -> usize {
        nodes
            .iter()
            .filter_map(Node::as_group)
            .map(|g| 1 + count(&g.children))
            .sum()
    }
    let count = count(&doc.layers);
    let mut group = Group {
        id,
        name: tr!("DEFAULT_GROUP_NAME", n = count + 1),
        visible: true,
        locked: false,
        opacity: 1.0,
        expanded: true,
        children: Vec::new(),
    };
    let path = match selected.and_then(|s| doc.remove(s)) {
        Some((path, node)) => {
            group.children.push(node);
            path
        }
        None => vec![doc.layers.len()],
    };
    doc.insert(&path, Node::Group(group));
    *selected = Some(id);
    true
}

/// Replaces the selected group by its children. Hidden, locked and opacity carry over
/// to the children so they look about the same.
pub fn ungroup(doc: &mut Document, selected: &mut Option<u64>) -> bool {
    if selected.and_then(|id| doc.node(id)?.as_group()).is_none() {
        return false;
    }
    let Some((mut path, Node::Group(g))) = selected.and_then(|id| doc.remove(id)) else {
        return false;
    };
    *selected = g.children.last().map(Node::id);
    for mut child in g.children {
        if !g.visible {
            match &mut child {
                Node::Group(c) => c.visible = false,
                Node::Layer(c) => c.visible = false,
            }
        }
        if g.locked {
            match &mut child {
                Node::Group(c) => c.locked = true,
                Node::Layer(c) => c.locked = true,
            }
        }
        *opacity_mut(&mut child) *= g.opacity;
        doc.insert(&path, child);
        *path.last_mut().unwrap() += 1;
    }
    true
}

/// Whether `move_step` can move the node up (`up`) or down.
pub fn can_step(doc: &Document, id: u64, up: bool) -> bool {
    let Some(path) = doc.path_of(id) else {
        return false;
    };
    let (&i, parent) = path.split_last().unwrap();
    let len = doc.list(parent).map_or(0, Vec::len);
    let at_end = if up { i + 1 == len } else { i == 0 };
    !at_end || !parent.is_empty()
}

/// Moves a node one step up or down the stack. It enters a neighboring group, and
/// leaves its own group when it reaches the group's end.
pub fn move_step(doc: &mut Document, id: u64, up: bool) -> bool {
    if !can_step(doc, id, up) {
        return false;
    }
    let path = doc.path_of(id).unwrap();
    let (&i, parent) = path.split_last().unwrap();
    let list = doc.list(parent).unwrap();
    let neighbor = if up { i + 1 } else { i.wrapping_sub(1) };
    let Some(next) = list.get(neighbor) else {
        // Leave the group: just above or below it in its parent.
        let (&gi, grand) = parent.split_last().unwrap();
        let (_, node) = doc.remove(id).unwrap();
        let mut to = grand.to_vec();
        to.push(if up { gi + 1 } else { gi });
        doc.insert(&to, node);
        return true;
    };
    match next {
        Node::Group(_) => {
            let gid = next.id();
            let (_, node) = doc.remove(id).unwrap();
            let gpath = doc.path_of(gid).unwrap();
            let Some(Node::Group(g)) = doc.node_at_mut(&gpath) else {
                unreachable!()
            };
            g.expanded = true;
            let at = if up { 0 } else { g.children.len() };
            g.children.insert(at, node);
        }
        Node::Layer(_) => {
            doc.list_mut(parent).unwrap().swap(i, neighbor);
        }
    }
    true
}

/// Moves a node to the top (`top`) or bottom of its own list.
pub fn move_to_end(doc: &mut Document, id: u64, top: bool) -> bool {
    let Some(path) = doc.path_of(id) else {
        return false;
    };
    let (&i, parent) = path.split_last().unwrap();
    let list = doc.list_mut(parent).unwrap();
    let to = if top { list.len() - 1 } else { 0 };
    if i == to {
        return false;
    }
    let node = list.remove(i);
    list.insert(to, node);
    true
}

/// Applies a drag-and-drop move. Refuses to drop a group into itself.
pub fn drop_node(doc: &mut Document, id: u64, target: DropTarget) -> bool {
    let target_id = match target {
        DropTarget::Above(t) | DropTarget::Below(t) | DropTarget::Into(t) => t,
    };
    if doc.is_within(target_id, id) || doc.path_of(target_id).is_none() {
        return false;
    }
    let Some((from, node)) = doc.remove(id) else {
        return false;
    };
    let mut to = doc.path_of(target_id).unwrap();
    match target {
        DropTarget::Above(_) => *to.last_mut().unwrap() += 1,
        DropTarget::Below(_) => {}
        DropTarget::Into(_) => {
            let Some(Node::Group(g)) = doc.node_at_mut(&to) else {
                doc.insert(&from, node);
                return false;
            };
            g.expanded = true;
            to.push(g.children.len());
        }
    }
    doc.insert(&to, node);
    true
}

// ---- Icons ----

/// Color of a toggle icon: follows hover, faded when off, dimmer when `dim`.
fn icon_color(ui: &Ui, r: &Response, on: bool, dim: bool) -> Color32 {
    let c = ui.style().interact(r).fg_stroke.color;
    let c = if on { c } else { c.gamma_multiply(0.35) };
    if dim { c.gamma_multiply(0.45) } else { c }
}

/// An eye that toggles `visible`. `dim` grays it out when a parent group is hidden.
fn eye_toggle(ui: &mut Ui, visible: &mut bool, dim: bool) -> Response {
    let (rect, mut r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    if r.clicked() {
        *visible = !*visible;
        r.mark_changed();
    }
    let c = icon_color(ui, &r, *visible, dim);
    let p = ui.painter();
    let (cx, cy) = (rect.center().x, rect.center().y);
    let (w, h) = (7.0, 5.5);
    let stroke = Stroke::new(1.3, c);
    let (left, right) = (pos2(cx - w, cy), pos2(cx + w, cy));
    let curve = |dy: f32| {
        Shape::QuadraticBezier(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [left, pos2(cx, cy + dy), right],
            false,
            Color32::TRANSPARENT,
            stroke,
        ))
    };
    if *visible {
        p.add(curve(-h * 1.5));
        p.add(curve(h * 1.5));
        p.circle_filled(rect.center(), 2.4, c);
    } else {
        // Closed eye: the lower lid with lashes.
        p.add(curve(h * 1.2));
        for (dx, lx) in [(-4.0, -1.5), (0.0, 0.0), (4.0, 1.5)] {
            let base = pos2(cx + dx, cy + h * 0.6 - dx.abs() * 0.12);
            p.line_segment([base, base + vec2(lx, 2.5)], stroke);
        }
    }
    r.on_hover_text(if *visible {
        tr!("HIDE_LAYER")
    } else {
        tr!("SHOW_LAYER")
    })
}

/// A padlock that toggles `locked`.
fn lock_toggle(ui: &mut Ui, locked: &mut bool, dim: bool) -> Response {
    let (rect, mut r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    if r.clicked() {
        *locked = !*locked;
        r.mark_changed();
    }
    let c = icon_color(ui, &r, *locked, dim);
    paint_lock(ui.painter(), rect, *locked, c);
    r.on_hover_text(if *locked {
        tr!("UNLOCK_LAYER")
    } else {
        tr!("LOCK_LAYER")
    })
}

/// A padlock centered in `rect`, closed when `locked`.
fn paint_lock(p: &egui::Painter, rect: Rect, locked: bool, c: Color32) {
    let body = Rect::from_center_size(rect.center() + vec2(0.0, 3.0), vec2(10.0, 7.0));
    if locked {
        p.rect_filled(body, 1.5, c);
    } else {
        p.rect_stroke(body, 1.5, Stroke::new(1.2, c), StrokeKind::Inside);
    }
    // Shackle: a half circle above the body, lifted and shifted when unlocked.
    let (lift, shift) = if locked { (0.0, 0.0) } else { (2.0, 2.5) };
    let center = pos2(body.center().x + shift, body.top() - 1.0 - lift);
    let pts: Vec<Pos2> = (0..=10)
        .map(|k| {
            let a = std::f32::consts::PI * (1.0 + k as f32 / 10.0);
            center + vec2(a.cos(), a.sin()) * 3.3
        })
        .collect();
    let mut shackle = vec![pts[0] + vec2(0.0, 1.0 + lift)];
    shackle.extend(pts.iter().copied());
    shackle.push(pts[10] + vec2(0.0, 1.0 + lift * 0.3));
    p.add(Shape::line(shackle, Stroke::new(1.3, c)));
}

/// Padlock button keeping a layer's width and height in proportion.
fn aspect_lock(ui: &mut Ui, free_aspect: &mut bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    if r.clicked() {
        *free_aspect = !*free_aspect;
    }
    let locked = !*free_aspect;
    if ui.is_rect_visible(rect) {
        if r.hovered() {
            let bg = ui.style().interact(&r).weak_bg_fill;
            ui.painter().rect_filled(rect, 3.0, bg);
        }
        // Full strength when locked, faint when width and height are free.
        let c = icon_color(ui, &r, locked, false);
        paint_lock(ui.painter(), rect, locked, c);
    }
    r.on_hover_text(if locked {
        tr!("ASPECT_RATIO_LOCKED")
    } else {
        tr!("ASPECT_RATIO_UNLOCKED")
    })
}

/// Square thumbnail of a layer: the middle of the image, cropped to fill the square.
/// `image_uv` is the part of the texture holding the image, without margins drawn by
/// effects.
fn thumbnail(ui: &mut Ui, tex: Option<egui::TextureId>, size_px: [u32; 2], image_uv: [f32; 4]) {
    let (rect, _) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::hover());
    let radius = 3.0;
    if let Some(tex) = tex {
        let [w, h] = size_px.map(|v| v.max(1) as f32);
        // Fraction of the image kept along each axis.
        let (kx, ky) = if w > h { (h / w, 1.0) } else { (1.0, w / h) };
        let [u0, v0, u1, v1] = image_uv;
        let uv = Rect::from_center_size(
            pos2((u0 + u1) / 2.0, (v0 + v1) / 2.0),
            vec2(kx * (u1 - u0), ky * (v1 - v0)),
        );
        egui::Image::new((tex, rect.size()))
            .uv(uv)
            .corner_radius(radius)
            .paint_at(ui, rect);
    }
    let border = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.painter()
        .rect_stroke(rect, radius, border, StrokeKind::Inside);
}

/// Folder icon shown in place of a group's thumbnail.
fn folder_icon(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::hover());
    let c = ui.visuals().widgets.inactive.fg_stroke.color;
    let body = Rect::from_center_size(rect.center() + vec2(0.0, 2.0), vec2(22.0, 15.0));
    let tab = Rect::from_min_size(body.min - vec2(0.0, 3.0), vec2(9.0, 4.0));
    let p = ui.painter();
    p.rect_filled(tab, 1.0, c.gamma_multiply(0.7));
    p.rect_filled(body, 2.0, c.gamma_multiply(0.7));
}

/// Triangle that opens or closes a group in the list.
fn expander(ui: &mut Ui, expanded: &mut bool) {
    let (rect, r) = ui.allocate_exact_size(vec2(ICON * 0.8, ICON), Sense::click());
    if r.clicked() {
        *expanded = !*expanded;
    }
    let c = ui.style().interact(&r).fg_stroke.color;
    let k = rect.center();
    let pts = if *expanded {
        vec![
            k + vec2(-4.0, -2.0),
            k + vec2(4.0, -2.0),
            k + vec2(0.0, 3.0),
        ]
    } else {
        vec![
            k + vec2(-2.0, -4.0),
            k + vec2(3.0, 0.0),
            k + vec2(-2.0, 4.0),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(pts, c, Stroke::NONE));
}

/// Icons of the toolbar under the layer list.
#[derive(Clone, Copy)]
pub enum Tool {
    Add,
    AddImage,
    AddText,
    Group,
    Ungroup,
    Duplicate,
    Delete,
    Top,
    Up,
    Down,
    Bottom,
    Align(Align),
}

const TOOL: f32 = 24.0;

/// A flat icon button of the layer toolbar; returns whether it was clicked.
pub fn tool_button(ui: &mut Ui, tool: Tool, enabled: bool, tip: &str) -> bool {
    let r = tool_icon(ui, tool, enabled, tip);
    enabled && r.clicked()
}

/// A flat icon button of the layer toolbar, e.g. to open a menu from it.
pub fn tool_icon(ui: &mut Ui, tool: Tool, enabled: bool, tip: &str) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(vec2(TOOL, TOOL), sense);
    let r = r.on_hover_text(tip);
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let visuals = ui.style().interact(&r);
    let p = ui.painter();
    if enabled && (r.hovered() || r.is_pointer_button_down_on()) {
        p.rect_filled(rect, 3.0, visuals.weak_bg_fill);
    }
    let c = if enabled {
        visuals.fg_stroke.color
    } else {
        ui.visuals()
            .widgets
            .noninteractive
            .fg_stroke
            .color
            .gamma_multiply(0.4)
    };
    let s = Stroke::new(1.4, c);
    let k = rect.center();
    let at = |x: f32, y: f32| k + vec2(x, y);
    let folder = |dx: f32| {
        let body = Rect::from_center_size(at(dx, 1.5), vec2(13.0, 9.0));
        let tab = Rect::from_min_size(body.min - vec2(0.0, 2.5), vec2(5.5, 3.0));
        p.rect_filled(tab, 1.0, c);
        p.rect_stroke(body, 1.5, s, StrokeKind::Inside);
    };
    let chevron = |dy: f32, up: bool| {
        let d = if up { -3.0 } else { 3.0 };
        p.add(Shape::line(
            vec![at(-5.0, dy - d), at(0.0, dy + d), at(5.0, dy - d)],
            s,
        ));
    };
    match tool {
        Tool::Add => {
            p.line_segment([at(-6.0, 0.0), at(6.0, 0.0)], s);
            p.line_segment([at(0.0, -6.0), at(0.0, 6.0)], s);
        }
        Tool::AddImage => {
            // A framed picture: hills under a sun.
            p.rect_stroke(
                Rect::from_center_size(k, vec2(15.0, 12.0)),
                1.5,
                s,
                StrokeKind::Inside,
            );
            p.add(Shape::line(
                vec![
                    at(-6.0, 4.5),
                    at(-2.0, 0.0),
                    at(1.0, 3.0),
                    at(3.0, 1.0),
                    at(6.0, 4.5),
                ],
                s,
            ));
            p.circle_filled(at(3.0, -2.5), 1.4, c);
        }
        Tool::AddText => {
            // A capital T next to a text cursor.
            let bold = Stroke::new(1.8, c);
            p.line_segment([at(-7.0, -5.5), at(2.0, -5.5)], bold);
            p.line_segment([at(-2.5, -5.5), at(-2.5, 6.0)], bold);
            p.line_segment([at(6.0, -6.0), at(6.0, 6.0)], s);
            p.line_segment([at(4.5, -6.0), at(7.5, -6.0)], s);
            p.line_segment([at(4.5, 6.0), at(7.5, 6.0)], s);
        }
        Tool::Group => {
            folder(-2.0);
            p.line_segment([at(4.0, 4.5), at(10.0, 4.5)], s);
            p.line_segment([at(7.0, 1.5), at(7.0, 7.5)], s);
        }
        Tool::Ungroup => {
            folder(-2.0);
            p.rect_filled(
                Rect::from_center_size(at(7.0, 4.5), vec2(8.0, 4.0)),
                0.0,
                ui.visuals().panel_fill,
            );
            p.line_segment([at(4.0, 4.5), at(10.0, 4.5)], s);
        }
        Tool::Duplicate => {
            p.rect_stroke(
                Rect::from_center_size(at(-2.0, -2.0), vec2(10.0, 10.0)),
                1.5,
                s,
                StrokeKind::Inside,
            );
            let front = Rect::from_center_size(at(2.5, 2.5), vec2(10.0, 10.0));
            p.rect_filled(front, 1.5, ui.visuals().panel_fill);
            p.rect_stroke(front, 1.5, s, StrokeKind::Inside);
        }
        Tool::Delete => {
            // Trash can: lid, handle and a tapered bin with ribs.
            p.line_segment([at(-6.5, -4.5), at(6.5, -4.5)], s);
            p.add(Shape::line(
                vec![at(-2.5, -4.5), at(-2.0, -7.0), at(2.0, -7.0), at(2.5, -4.5)],
                s,
            ));
            p.add(Shape::closed_line(
                vec![at(-5.0, -4.5), at(-4.0, 7.0), at(4.0, 7.0), at(5.0, -4.5)],
                s,
            ));
            for x in [-1.8, 1.8] {
                p.line_segment([at(x, -2.0), at(x, 4.5)], s);
            }
        }
        Tool::Up => chevron(0.0, true),
        Tool::Down => chevron(0.0, false),
        Tool::Top => {
            p.line_segment([at(-6.0, -5.5), at(6.0, -5.5)], s);
            chevron(1.5, true);
        }
        Tool::Bottom => {
            p.line_segment([at(-6.0, 5.5), at(6.0, 5.5)], s);
            chevron(-1.5, false);
        }
        Tool::Align(a) => {
            // A guide line and two bars of different lengths lined up against it.
            // Drawn for horizontal alignment, then swapped for vertical.
            let (vertical, pos) = match a {
                Align::Left => (false, -1.0),
                Align::CenterX => (false, 0.0),
                Align::Right => (false, 1.0),
                Align::Top => (true, -1.0),
                Align::CenterY => (true, 0.0),
                Align::Bottom => (true, 1.0),
            };
            let pt = |x: f32, y: f32| if vertical { at(y, x) } else { at(x, y) };
            let guide = pos * 8.0;
            p.line_segment([pt(guide, -8.0), pt(guide, 8.0)], s);
            for (y, len) in [(-3.0, 12.0), (3.0, 7.0)] {
                let x0 = match pos {
                    p if p < 0.0 => guide + 1.5,
                    p if p > 0.0 => guide - 1.5 - len,
                    _ => -len / 2.0,
                };
                let bar = if vertical {
                    Rect::from_min_size(at(y - 1.5, x0), vec2(3.0, len))
                } else {
                    Rect::from_min_size(at(x0, y - 1.5), vec2(len, 3.0))
                };
                p.rect_filled(bar, 1.0, c);
            }
        }
    }
    r
}

// ---- Property grids ----

/// Gap between the rows of the property grids.
const PROP_ROW_GAP: f32 = 6.0;

/// Labels on the left of the property grids: the label column is as wide as the longest,
/// so the controls line up from one section to the next, in every language.
const PROP_LABELS: [&str; 12] = [
    tr_noop!("LAYER_NAME"),
    tr_noop!("OPACITY"),
    tr_noop!("LAYER_POSITION"),
    tr_noop!("LAYER_SIZE"),
    tr_noop!("LAYER_ROTATION"),
    tr_noop!("TEXT_FONT"),
    tr_noop!("TEXT_STYLE"),
    tr_noop!("TEXT_LETTER_SPACING"),
    tr_noop!("TEXT_LINE_SPACING"),
    tr_noop!("TEXT_COLOR"),
    tr_noop!("TEXT_FONT_SIZE"),
    tr_noop!("TEXT_OUTLINE"),
];

/// A two-column grid of labels and controls, lined up with the other property grids.
fn prop_grid(ui: &Ui, id: &str) -> egui::Grid {
    let label_width = PROP_LABELS
        .iter()
        .map(|k| {
            egui::WidgetText::from(translate(k))
                .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, PROP_ROW_GAP])
        .min_col_width(label_width.ceil())
}

/// What a text style toggle shows.
#[derive(Clone, Copy)]
enum StyleIcon {
    Bold,
    Italic,
    Align(TextAlign),
}

/// A square toggle of the text style row, highlighted when `on`.
fn style_toggle(ui: &mut Ui, on: bool, icon: StyleIcon, tip: &str) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, r) = ui.allocate_exact_size(vec2(side + 2.0, side), Sense::click());
    let r = r.on_hover_text(tip);
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let visuals = ui.style().interact_selectable(&r, on);
    let p = ui.painter();
    if on || r.hovered() || r.is_pointer_button_down_on() {
        p.rect(
            rect,
            3.0,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
    }
    let c = visuals.fg_stroke.color;
    let k = rect.center();
    match icon {
        StyleIcon::Bold | StyleIcon::Italic => {
            let text = match icon {
                StyleIcon::Bold => RichText::new("B").strong(),
                _ => RichText::new("I").italics(),
            };
            let galley = egui::WidgetText::from(text).into_galley(
                ui,
                None,
                f32::INFINITY,
                egui::TextStyle::Button,
            );
            p.galley(k - galley.size() / 2.0, galley, c);
        }
        StyleIcon::Align(a) => {
            // Four lines of different lengths, lined up on one side or centered.
            let s = Stroke::new(1.4, c);
            for (y, len) in [(-4.5, 12.0), (-1.5, 8.0), (1.5, 12.0), (4.5, 6.0)] {
                let x0 = match a {
                    TextAlign::Left => -6.0,
                    TextAlign::Center => -len / 2.0,
                    TextAlign::Right => 6.0 - len,
                };
                p.line_segment([k + vec2(x0, y), k + vec2(x0 + len, y)], s);
            }
        }
    }
    r
}

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

/// `template_bounds` is the template's axis-aligned box on the page: (center, size).
pub fn show(
    ui: &mut Ui,
    doc: &mut Document,
    selected: &mut Option<u64>,
    state: &mut LayersPanel,
    template_bounds: Option<(P2, P2)>,
    fonts: &FontLibrary,
) -> LayersOutput {
    let mut out = LayersOutput::default();
    ui.heading(tr!("LAYERS_SECTION"));

    let sel = selected.filter(|&id| doc.path_of(id).is_some());
    let has = sel.is_some();
    let is_group = sel.and_then(|id| doc.node(id)?.as_group()).is_some();

    // Layer list, top of the stack first.
    let mut pending_drop = None;
    let mut pending_duplicate = None;
    egui::ScrollArea::vertical()
        .id_salt("layer_list")
        .max_height(ui.available_height() * 0.5)
        .show(ui, |ui| {
            if doc.layers.is_empty() {
                ui.weak(tr!("NO_LAYERS_YET"));
            }
            let full = ui.max_rect();
            for row in rows(doc) {
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
                    let (visible, locked) = match node {
                        Node::Group(g) => (&mut g.visible, &mut g.locked),
                        Node::Layer(l) => (&mut l.visible, &mut l.locked),
                    };
                    out.changed |= eye_toggle(ui, visible, row.parent_hidden).changed();
                    out.changed |= lock_toggle(ui, locked, row.parent_locked).changed();
                    match &*node {
                        Node::Group(_) => folder_icon(ui),
                        Node::Layer(l) => {
                            let tex = l.shown_texture().map(|t| t.id());
                            thumbnail(ui, tex, l.size_px, l.image_uv());
                        }
                    }
                    let broken = node.as_layer().filter(|l| l.broken);
                    let missing = broken.map(|l| l.source.display().to_string());
                    let name = name_mut(node);
                    {
                        let mut text = RichText::new(name.as_str());
                        if missing.is_some() {
                            text = text.color(ui.visuals().error_fg_color);
                        }
                        if row.parent_hidden {
                            text = text.weak();
                        }
                        let label = name.clone();
                        // The name takes the rest of the row, so short names are as easy to
                        // click (and drag) as long ones. No frame of its own: the whole row
                        // shows hover and selection. Renaming is done in Properties.
                        let size = vec2(ui.available_width(), ui.spacing().interact_size.y);
                        let (name_rect, r) = ui.allocate_exact_size(size, Sense::click_and_drag());
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
                        r.dnd_set_drag_payload(DraggedNode(row.id));
                        r.context_menu(|ui| {
                            if ui
                                .add(Button::new(tr!("DUPLICATE_LAYER")).shortcut_text("Ctrl+D"))
                                .clicked()
                            {
                                pending_duplicate = Some(row.id);
                            }
                        });
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
                    let accent = ui.visuals().selection.stroke.color;
                    let stroke = Stroke::new(2.0, accent);
                    let left = rect.left() + row.depth as f32 * INDENT;
                    let painter = ui.painter();
                    match target {
                        DropTarget::Above(_) => {
                            painter.hline(left..=rect.right(), rect.top(), stroke);
                        }
                        DropTarget::Below(_) => {
                            painter.hline(left..=rect.right(), rect.bottom(), stroke);
                        }
                        DropTarget::Into(_) => {
                            painter.rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
                        }
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        DragAndDrop::clear_payload(ui.ctx());
                        pending_drop = Some((dragged.0, target));
                    }
                }
            }
        });
    if let Some((id, target)) = pending_drop {
        out.changed |= drop_node(doc, id, target);
    }
    if let Some(id) = pending_duplicate {
        *selected = Some(id);
        out.changed |= duplicate(doc, selected);
    }

    // Toolbar under the list, as in most layer panels.
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let id = sel.unwrap_or(0);
        // One button per kind of layer, so each says what it adds.
        if tool_button(ui, Tool::AddImage, true, tr!("ADD_IMAGE_LAYERS")) {
            out.add_images = true;
        }
        if tool_button(ui, Tool::AddText, true, tr!("ADD_TEXT_LAYER")) {
            out.add_text = true;
        }
        ui.add_space(6.0);
        if tool_button(ui, Tool::Group, true, tr!("GROUP_LAYERS")) {
            out.changed |= group(doc, selected);
        }
        if tool_button(ui, Tool::Ungroup, is_group, tr!("UNGROUP_LAYERS")) {
            out.changed |= ungroup(doc, selected);
        }
        if tool_button(ui, Tool::Duplicate, has, tr!("DUPLICATE_LAYER_HINT")) {
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if tool_button(ui, Tool::Delete, has, tr!("DELETE_LAYER")) {
                out.changed |= delete(doc, selected);
            }
        });
    });

    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("layer_props")
        .show(ui, |ui| {
            properties(ui, doc, sel, state, template_bounds, fonts, &mut out)
        });
    out
}

/// A titled section of a side panel, with a line above it, folded by clicking its arrow
/// or title. `extra`
/// adds widgets at the right of the header and returns true to open the section (e.g.
/// after adding something to it). The open state is shared by all layers and kept
/// between runs.
pub fn section(
    ui: &mut Ui,
    id_salt: &str,
    title: RichText,
    extra: impl FnOnce(&mut Ui) -> bool,
    body: impl FnOnce(&mut Ui),
) {
    let id = ui.make_persistent_id(("section", id_salt));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
    // A line above each section, so sections stand apart, folded or not.
    ui.add_space(2.0);
    ui.separator();
    ui.horizontal(|ui| {
        state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
        let title = ui.add(egui::Label::new(title).sense(Sense::click()));
        if title.clicked() {
            state.toggle(ui);
        }
        let open = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), extra)
            .inner;
        if open {
            state.set_open(true);
        }
    });
    state.show_body_unindented(ui, body);
}

/// The part of Properties for a text layer: what it says and how it looks. Returns true
/// if it changed.
fn text_section(
    ui: &mut Ui,
    layer: &mut Layer,
    state: &mut LayersPanel,
    fonts: &FontLibrary,
) -> bool {
    let mut changed = false;
    // A new text layer opens the section, so its text can be typed right away.
    let open = state.focus_text;
    section(
        ui,
        "text_props",
        RichText::new(tr!("TEXT_SECTION")),
        move |_| open,
        |ui| {
            let Some(t) = layer.text.as_deref_mut() else {
                return;
            };
            // The name follows the text until it is renamed.
            let auto = layer.name == text::auto_name(&t.content);
            let id = ui.make_persistent_id("text_content");
            let r = ui.add(
                egui::TextEdit::multiline(&mut t.content)
                    .id(id)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text(tr!("TEXT_PLACEHOLDER")),
            );
            if std::mem::take(&mut state.focus_text) {
                r.request_focus();
                r.scroll_to_me(Some(egui::Align::Center));
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
                    st.cursor
                        .set_char_range(Some(egui::text::CCursorRange::select_all_str(&t.content)));
                    st.store(ui.ctx(), id);
                }
            }
            if r.changed() {
                changed = true;
                if auto {
                    layer.name = text::auto_name(&t.content);
                }
            }
            ui.add_space(4.0);

            let mut font_changed = false;
            prop_grid(ui, "text_props").show(ui, |ui| {
                ui.label(tr!("TEXT_FONT"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    font_changed |= font_menu(ui, &mut t.family, state, fonts);
                    let missing = layer.broken;
                    if missing {
                        ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                            .on_hover_text(tr!("TEXT_FONT_NOT_INSTALLED"));
                    }
                });
                ui.end_row();

                ui.label(tr!("TEXT_STYLE"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if style_toggle(ui, t.bold, StyleIcon::Bold, tr!("TEXT_BOLD")).clicked() {
                        t.bold ^= true;
                        font_changed = true;
                    }
                    let italic = style_toggle(ui, t.italic, StyleIcon::Italic, tr!("TEXT_ITALIC"));
                    if italic.clicked() {
                        t.italic ^= true;
                        font_changed = true;
                    }
                    ui.add_space(8.0);
                    for (a, tip) in [
                        (TextAlign::Left, tr!("TEXT_ALIGN_LEFT")),
                        (TextAlign::Center, tr!("TEXT_ALIGN_CENTER")),
                        (TextAlign::Right, tr!("TEXT_ALIGN_RIGHT")),
                    ] {
                        if style_toggle(ui, t.align == a, StyleIcon::Align(a), tip).clicked() {
                            t.align = a;
                            changed = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(tr!("TEXT_LETTER_SPACING"));
                changed |= ui
                    .add(
                        DragValue::new(&mut t.letter_spacing)
                            .range(-500.0..=2000.0)
                            .speed(2.0)
                            .fixed_decimals(0),
                    )
                    .on_hover_text(tr!("TEXT_LETTER_SPACING_HINT"))
                    .changed();
                ui.end_row();

                ui.label(tr!("TEXT_LINE_SPACING"));
                changed |= ui
                    .add(
                        DragValue::new(&mut t.line_height)
                            .range(0.5..=5.0)
                            .speed(0.01)
                            .fixed_decimals(2)
                            .suffix(" x"),
                    )
                    .on_hover_text(tr!("TEXT_LINE_SPACING_HINT"))
                    .changed();
                ui.end_row();

                ui.label(tr!("TEXT_COLOR"));
                changed |= ui
                    .color_edit_button_srgba_unmultiplied(&mut t.color)
                    .changed();
                ui.end_row();
            });
            if font_changed {
                text::font_changed(layer);
                changed = true;
            }

            // Size and outline width are in mm on the page, so they need the whole layer.
            // Same gap as between the rows above.
            ui.add_space(PROP_ROW_GAP - ui.spacing().item_spacing.y);
            prop_grid(ui, "text_size").show(ui, |ui| {
                ui.label(tr!("TEXT_FONT_SIZE"));
                let mut pt = text::size_pt(layer);
                if ui
                    .add(
                        DragValue::new(&mut pt)
                            .range(1.0..=2000.0)
                            .speed(0.2)
                            .fixed_decimals(1)
                            .suffix(" pt"),
                    )
                    .on_hover_text(tr!("TEXT_FONT_SIZE_HINT"))
                    .changed()
                {
                    text::set_size_pt(layer, pt);
                    changed = true;
                }
                ui.end_row();

                ui.label(tr!("TEXT_OUTLINE"));
                // The outline's thickness is kept in ems, shown in mm.
                let em_mm = text::em_to_mm(layer, 1.0).max(1e-6);
                let Some(t) = layer.text.as_deref_mut() else {
                    return;
                };
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut t.outline, "").changed();
                    ui.add_enabled_ui(t.outline, |ui| {
                        let mut mm = t.outline_width * em_mm;
                        if ui
                            .add(
                                DragValue::new(&mut mm)
                                    .range(0.01..=50.0)
                                    .speed(0.02)
                                    .fixed_decimals(2)
                                    .suffix(" mm"),
                            )
                            .on_hover_text(tr!("TEXT_OUTLINE_HINT"))
                            .changed()
                        {
                            t.outline_width = mm / em_mm;
                            changed = true;
                        }
                        changed |= ui
                            .color_edit_button_srgba_unmultiplied(&mut t.outline_color)
                            .changed();
                    });
                });
                ui.end_row();
            });
        },
    );
    changed
}

/// The part of Properties for an image layer: its print resolution and source size.
fn image_section(ui: &mut Ui, layer: &Layer) {
    section(
        ui,
        "image_props",
        RichText::new(tr!("IMAGE_SECTION")),
        |_| false,
        |ui| {
            // Print resolution of this layer, from its pixel count and printed size.
            let [w_mm, h_mm] = layer.size_mm();
            let dpi = (layer.size_px[0] as f32 / (w_mm / 25.4))
                .min(layer.size_px[1] as f32 / (h_mm / 25.4));
            let shown = format!("{dpi:.0}");
            if dpi < 150.0 {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    tr!("PRINT_RESOLUTION_LOW", dpi = shown),
                );
            } else {
                ui.weak(tr!("PRINT_RESOLUTION", dpi = shown));
            }
            ui.weak(tr!(
                "IMAGE_SOURCE_SIZE",
                w = layer.size_px[0],
                h = layer.size_px[1]
            ));
        },
    );
}

/// The font family list, with a search field. Returns true if another was picked.
fn font_menu(
    ui: &mut Ui,
    family: &mut String,
    state: &mut LayersPanel,
    fonts: &FontLibrary,
) -> bool {
    let mut picked = None;
    egui::ComboBox::from_id_salt("text_font")
        .width(170.0)
        .selected_text(family.as_str())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut state.font_filter)
                    .hint_text(tr!("SEARCH_FONTS"))
                    .desired_width(f32::INFINITY),
            );
            if !search.has_focus() && state.font_filter.is_empty() {
                search.request_focus();
            }
            let filter = state.font_filter.to_lowercase();
            let list = fonts.with_families(|all| {
                all.iter()
                    .filter(|f| f.to_lowercase().contains(&filter))
                    .cloned()
                    .collect::<Vec<_>>()
            });
            let Some(list) = list else {
                ui.weak(tr!("LOADING_FONTS"));
                ui.ctx().request_repaint();
                return;
            };
            let row = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical().max_height(300.0).show_rows(
                ui,
                row,
                list.len(),
                |ui, range| {
                    for f in &list[range] {
                        if ui.selectable_label(f == family, f.as_str()).clicked() {
                            picked = Some(f.clone());
                            ui.close();
                        }
                    }
                },
            );
        });
    match picked {
        Some(f) if f != *family => {
            *family = f;
            true
        }
        _ => false,
    }
}

/// Alignment, properties and effects of the selected layer or group.
fn properties(
    ui: &mut Ui,
    doc: &mut Document,
    sel: Option<u64>,
    state: &mut LayersPanel,
    template_bounds: Option<(P2, P2)>,
    fonts: &FontLibrary,
    out: &mut LayersOutput,
) {
    let Some(id) = sel else {
        ui.weak(tr!("NO_LAYER_SELECTED"));
        return;
    };
    let page_center = doc.page_center();
    let page = doc.page_mm();
    let locked = doc.effectively_locked(id);
    // Align the selection with the page or the template outline.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.label(tr!("ALIGN_TO"));
        if template_bounds.is_none() {
            state.align_to = AlignTo::Page;
        }
        egui::ComboBox::from_id_salt("align_to")
            .width(80.0)
            .selected_text(match state.align_to {
                AlignTo::Page => tr!("ALIGN_TO_PAGE"),
                AlignTo::Template => tr!("ALIGN_TO_TEMPLATE"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.align_to, AlignTo::Page, tr!("ALIGN_TO_PAGE"));
                ui.add_enabled_ui(template_bounds.is_some(), |ui| {
                    ui.selectable_value(
                        &mut state.align_to,
                        AlignTo::Template,
                        tr!("ALIGN_TO_TEMPLATE"),
                    );
                });
            });
        ui.add_space(4.0);
        let area = match (state.align_to, template_bounds) {
            (AlignTo::Template, Some((c, s))) => (
                [c[0] - s[0] / 2.0, c[1] - s[1] / 2.0],
                [c[0] + s[0] / 2.0, c[1] + s[1] / 2.0],
            ),
            _ => ([0.0, 0.0], page),
        };
        for (k, a) in Align::ALL.into_iter().enumerate() {
            if k == 3 {
                ui.add_space(4.0);
            }
            if tool_button(ui, Tool::Align(a), !locked, a.tip()) {
                out.changed |= align(doc, sel, a, area);
            }
        }
    });
    ui.add_space(4.0);

    let layer_count = {
        let mut n = 0;
        if let Some(node) = doc.node(id) {
            node.for_each_layer(&mut |_| n += 1);
        }
        n
    };
    match doc.node_mut(id) {
        Some(Node::Group(g)) => {
            // Common properties first, then what only groups have.
            section(
                ui,
                "layer_props",
                RichText::new(tr!("PROPERTIES_SECTION")).strong(),
                |_| false,
                |ui| {
                    prop_grid(ui, "group_props").show(ui, |ui| {
                        ui.label(tr!("LAYER_NAME"));
                        out.changed |= ui.text_edit_singleline(&mut g.name).changed();
                        ui.end_row();

                        ui.label(tr!("OPACITY"));
                        out.changed |= ui
                            .add(Slider::new(&mut g.opacity, 0.0..=1.0).fixed_decimals(2))
                            .on_hover_text(tr!("GROUP_OPACITY_HINT"))
                            .changed();
                        ui.end_row();
                    });
                    section(
                        ui,
                        "group_props",
                        RichText::new(tr!("GROUP_SECTION")),
                        |_| false,
                        |ui| {
                            ui.weak(tr_n!("LAYER_COUNT", layer_count));
                            ui.weak(tr!("MOVE_GROUP_HINT"));
                        },
                    );
                },
            );
        }
        Some(Node::Layer(layer)) => {
            // Common properties first, then what only this kind of layer has.
            let open = state.focus_text;
            section(
                ui,
                "layer_props",
                RichText::new(tr!("PROPERTIES_SECTION")).strong(),
                move |_| open,
                |ui| {
                    prop_grid(ui, "layer_props").show(ui, |ui| {
                        ui.label(tr!("LAYER_NAME"));
                        out.changed |= ui.text_edit_singleline(&mut layer.name).changed();
                        ui.end_row();

                        ui.label(tr!("OPACITY"));
                        out.changed |= ui
                            .add(Slider::new(&mut layer.opacity, 0.0..=1.0).fixed_decimals(2))
                            .changed();
                        ui.end_row();

                        ui.add_enabled_ui(!locked, |ui| ui.label(tr!("LAYER_POSITION")));
                        ui.add_enabled_ui(!locked, |ui| {
                            ui.horizontal(|ui| {
                                let t = &mut layer.transform;
                                out.changed |= ui
                                    .add(
                                        DragValue::new(&mut t.center[0])
                                            .fixed_decimals(1)
                                            .speed(0.5)
                                            .prefix("x ")
                                            .suffix(" mm"),
                                    )
                                    .changed();
                                out.changed |= ui
                                    .add(
                                        DragValue::new(&mut t.center[1])
                                            .fixed_decimals(1)
                                            .speed(0.5)
                                            .prefix("y ")
                                            .suffix(" mm"),
                                    )
                                    .changed();
                            });
                        });
                        ui.end_row();

                        ui.label(tr!("LAYER_SIZE"));
                        ui.add_enabled_ui(!locked, |ui| {
                            ui.horizontal(|ui| {
                                let [px_w, px_h] = layer.size_px.map(|v| v.max(1) as f32);
                                let [mut w, mut h] = layer.size_mm();
                                let rw = ui.add(
                                    DragValue::new(&mut w)
                                        .fixed_decimals(1)
                                        .speed(0.5)
                                        .range(0.01..=5000.0)
                                        .prefix(format!("{} ", tr!("LAYER_WIDTH_PREFIX")))
                                        .suffix(" mm"),
                                );
                                let rh = ui.add(
                                    DragValue::new(&mut h)
                                        .fixed_decimals(1)
                                        .speed(0.5)
                                        .range(0.01..=5000.0)
                                        .prefix(format!("{} ", tr!("LAYER_HEIGHT_PREFIX")))
                                        .suffix(" mm"),
                                );
                                aspect_lock(ui, &mut state.free_aspect);
                                let s = &mut layer.transform.scale;
                                if rw.changed() {
                                    let k = w / px_w / s[0];
                                    s[0] = w / px_w;
                                    if !state.free_aspect {
                                        s[1] *= k;
                                    }
                                    out.changed = true;
                                }
                                if rh.changed() {
                                    let k = h / px_h / s[1];
                                    s[1] = h / px_h;
                                    if !state.free_aspect {
                                        s[0] *= k;
                                    }
                                    out.changed = true;
                                }
                            });
                        });
                        ui.end_row();

                        ui.label(tr!("LAYER_ROTATION"));
                        ui.add_enabled_ui(!locked, |ui| {
                            out.changed |= ui
                                .add(
                                    Slider::new(&mut layer.transform.rotation_deg, -180.0..=180.0)
                                        .suffix("°")
                                        .fixed_decimals(1),
                                )
                                .changed();
                        });
                        ui.end_row();
                    });

                    ui.add_space(4.0);
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            if ui
                                .button(tr!("FIT_LAYER_TO_PAGE"))
                                .on_hover_text(tr!("FIT_LAYER_TO_PAGE_HINT"))
                                .clicked()
                            {
                                layer.fit_to(page_center, page, true);
                                out.changed = true;
                            }
                            if ui
                                .add_enabled(
                                    template_bounds.is_some(),
                                    Button::new(tr!("FIT_LAYER_TO_TEMPLATE")),
                                )
                                .on_hover_text(tr!("FIT_LAYER_TO_TEMPLATE_HINT"))
                                .clicked()
                            {
                                let (c, s) = template_bounds.unwrap_or((page_center, page));
                                layer.fit_to(c, s, true);
                                out.changed = true;
                            }
                            if ui
                                .button(tr!("RESET_LAYER_TRANSFORM"))
                                .on_hover_text(tr!("RESET_LAYER_TRANSFORM_HINT"))
                                .clicked()
                            {
                                layer.fit_to(page_center, page, false);
                                layer.transform.flip_x = false;
                                layer.transform.flip_y = false;
                                layer.transform.distort = Default::default();
                                out.changed = true;
                            }
                            if ui.button(tr!("FLIP_LAYER_HORIZONTALLY")).clicked() {
                                layer.transform.flip_x ^= true;
                                out.changed = true;
                            }
                            if ui.button(tr!("FLIP_LAYER_VERTICALLY")).clicked() {
                                layer.transform.flip_y ^= true;
                                out.changed = true;
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            // Shows what the corner handles do right now: Ctrl swaps the mode
                            // while held.
                            let ctrl = ui.input(|i| i.modifiers.command);
                            if ui
                                .selectable_label(state.distort != ctrl, tr!("DISTORT_LAYER"))
                                .on_hover_text(tr!("DISTORT_LAYER_HINT"))
                                .clicked()
                            {
                                state.distort = !state.distort;
                            }
                            if ui
                                .add_enabled(
                                    layer.transform.is_distorted(),
                                    Button::new(tr!("RESET_DISTORTION")),
                                )
                                .clicked()
                            {
                                layer.transform.distort = Default::default();
                                out.changed = true;
                            }
                        });
                    });

                    if layer.text.is_some() {
                        out.changed |= text_section(ui, layer, state, fonts);
                    } else {
                        image_section(ui, layer);
                    }
                },
            );
            out.changed |= effects_ui::show(
                ui,
                layer.id,
                &mut layer.effects,
                layer.text.is_some().then(|| layer.transform.scale[1].abs()),
            );
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Layer;

    fn layer(id: u64) -> Node {
        Node::Layer(Box::new(Layer::new(id, format!("l{id}"), "x.png".into())))
    }

    /// Ids in the tree, as "id" or "id[children]", bottom first.
    fn shape(nodes: &[Node]) -> String {
        nodes
            .iter()
            .map(|n| match n {
                Node::Layer(l) => l.id.to_string(),
                Node::Group(g) => format!("{}[{}]", g.id, shape(&g.children)),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn doc_with(nodes: Vec<Node>) -> Document {
        Document {
            layers: nodes,
            next_id: 100,
            ..Default::default()
        }
    }

    /// Drives the real panel with synthetic pointer input.
    struct Harness {
        ctx: egui::Context,
        doc: Document,
        selected: Option<u64>,
        state: LayersPanel,
        time: f64,
        /// Centers of the dots drawn in the last frame (effect drag handles).
        dots: Vec<Pos2>,
        /// Filled rectangles drawn in the last frame, with their color.
        fills: Vec<(Rect, egui::Color32)>,
        /// The mouse cursor asked for in the last frame.
        cursor: egui::CursorIcon,
    }

    impl Harness {
        fn new(doc: Document) -> Self {
            let ctx = egui::Context::default();
            crate::app::set_interaction(&ctx);
            let mut h = Self {
                ctx,
                doc,
                selected: None,
                state: LayersPanel::default(),
                time: 0.0,
                dots: Vec::new(),
                fills: Vec::new(),
                cursor: egui::CursorIcon::Default,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        /// Runs one frame; returns where each text label was drawn.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, Pos2)> {
            self.time += 0.02;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = self.ctx.run_ui(input, |ui| {
                show(
                    ui,
                    &mut self.doc,
                    &mut self.selected,
                    &mut self.state,
                    None,
                    &FontLibrary::builtin_only(),
                );
            });
            out.textures_delta.clear();
            self.cursor = out.platform_output.cursor_icon;
            self.dots = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Circle(d) => Some(d.center),
                    _ => None,
                })
                .collect();
            self.fills = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) => Some((r.rect, r.fill)),
                    _ => None,
                })
                .collect();
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos)),
                    _ => None,
                })
                .collect()
        }

        /// The "+" at the right of the Effects header.
        fn add_effect_button(&mut self) -> Pos2 {
            let labels = self.frame(vec![]);
            let (_, p) = labels
                .into_iter()
                .find(|(t, _)| t.starts_with("Effects"))
                .expect("Effects header");
            Pos2::new(388.0, p.y + 6.0)
        }

        fn label(&mut self, text: &str) -> Pos2 {
            let labels = self.frame(vec![]);
            let (_, p) = labels.into_iter().find(|(t, _)| t == text).expect(text);
            p + vec2(4.0, 6.0)
        }

        fn pointer(&mut self, p: Pos2, pressed: Option<bool>) {
            let mut events = vec![egui::Event::PointerMoved(p)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            self.frame(events);
        }

        fn click(&mut self, p: Pos2) {
            self.pointer(p, None);
            self.pointer(p, Some(true));
            self.pointer(p, Some(false));
        }

        /// Middle of the drag handle left of an effect's name.
        fn handle(&mut self, name: &str) -> Pos2 {
            let labels = self.frame(vec![]);
            let (_, t) = labels.into_iter().find(|(t, _)| t == name).expect(name);
            let dots: Vec<Pos2> = self
                .dots
                .iter()
                .copied()
                .filter(|d| d.x < t.x && (t.y - 6.0..t.y + 22.0).contains(&d.y))
                .collect();
            assert_eq!(dots.len(), 6, "handle of {name}");
            let sum = dots.iter().fold(vec2(0.0, 0.0), |a, d| a + d.to_vec2());
            (sum / 6.0).to_pos2()
        }

        fn secondary_click(&mut self, p: Pos2) {
            self.pointer(p, None);
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(p),
                    egui::Event::PointerButton {
                        pos: p,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            self.frame(vec![]);
        }

        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.pointer(from, None);
            self.pointer(from, Some(true));
            for k in 1..=5 {
                self.pointer(from + (to - from) * (k as f32 / 5.0), None);
            }
            self.pointer(to, Some(false));
            self.frame(vec![]);
        }
    }

    #[test]
    fn name_field_gives_up_focus_on_a_press_elsewhere() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        let p = h.label("alpha");
        // Double-clicking the name in the list doesn't edit it there.
        h.click(p);
        h.click(p);
        h.frame(vec![]);
        assert!(h.ctx.memory(|m| m.focused().is_none()));

        // The Name field of the properties.
        let field = h.label("Name") + vec2(150.0, 0.0);
        h.click(field);
        assert!(h.ctx.memory(|m| m.focused().is_some()));
        // Press and drag on empty space below the panel, as when grabbing the canvas.
        let empty = Pos2::new(200.0, 780.0);
        h.drag(empty, empty + vec2(40.0, -20.0));
        assert!(h.ctx.memory(|m| m.focused().is_none()));
    }

    fn named(id: u64, name: &str) -> Node {
        Node::Layer(Box::new(Layer::new(id, name.into(), "x.png".into())))
    }

    #[test]
    fn properties_and_effects_fold_from_their_titles() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        h.selected = Some(1);
        let shown = |h: &mut Harness, text: &str| h.frame(vec![]).iter().any(|(t, _)| t == text);
        assert!(shown(&mut h, "Opacity") && shown(&mut h, "Effects"));

        let p = h.label("Properties");
        h.click(p);
        // Let the folding animation finish.
        for _ in 0..30 {
            h.frame(vec![]);
        }
        assert!(!shown(&mut h, "Opacity"), "Properties folded");
        assert!(
            shown(
                &mut h,
                "Effects change how the image looks without changing its file."
            ),
            "Effects still open"
        );

        let p = h.label("Effects");
        h.click(p);
        for _ in 0..30 {
            h.frame(vec![]);
        }
        assert!(!shown(
            &mut h,
            "Effects change how the image looks without changing its file."
        ));
        // The + stays in the header, so effects can be added while folded; that opens it.
        let p = h.add_effect_button();
        h.click(p);
        let p = h.label("Invert");
        h.click(p);
        for _ in 0..30 {
            h.frame(vec![]);
        }
        assert_eq!(h.doc.layer(1).unwrap().effects.len(), 1);
        assert!(shown(&mut h, "Effects (1)"));
        assert!(h.handle("Invert").x > 0.0, "Effects opened on adding");

        let p = h.label("Properties");
        h.click(p);
        for _ in 0..30 {
            h.frame(vec![]);
        }
        assert!(shown(&mut h, "Opacity"), "Properties open again");
    }

    #[test]
    fn text_cursor_only_over_what_can_be_edited() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        h.selected = Some(1);
        for (label, want) in [
            ("Opacity", egui::CursorIcon::Default),
            ("Properties", egui::CursorIcon::Default),
        ] {
            let p = h.label(label);
            h.pointer(p, None);
            h.frame(vec![]);
            assert_eq!(h.cursor, want, "over {label:?}");
        }
        // The name field, right of its label.
        let p = h.label("Name") + vec2(120.0, 0.0);
        h.pointer(p, None);
        h.frame(vec![]);
        assert_eq!(h.cursor, egui::CursorIcon::Text, "over the name field");
    }

    #[test]
    fn padlock_right_of_the_size_toggles_the_aspect_ratio() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        h.selected = Some(1);
        let labels = h.frame(vec![]);
        let (_, hpos) = labels
            .iter()
            .find(|(t, _)| t == "h ")
            .unwrap_or_else(|| panic!("height field: {labels:?}"));
        // The closed padlock's body: a filled 10 x 7 rectangle right of the height, on
        // the same line.
        let body = h
            .fills
            .iter()
            .map(|(r, _)| *r)
            .find(|r| {
                (r.width() - 10.0).abs() < 0.1
                    && (r.height() - 7.0).abs() < 0.1
                    && r.left() > hpos.x
                    && (r.center().y - hpos.y).abs() < 16.0
            })
            .expect("padlock");
        assert!(!h.state.free_aspect);
        h.click(body.center());
        assert!(h.state.free_aspect, "unlocked");
        h.click(body.center());
        assert!(!h.state.free_aspect, "locked again");
    }

    #[test]
    fn hovered_row_is_highlighted_across_its_width() {
        let mut h = Harness::new(doc_with(vec![named(1, "a"), named(2, "b")]));
        let b = h.label("b");
        h.pointer(Pos2::new(380.0, b.y), None);
        h.frame(vec![]);
        let fill = h.ctx.global_style().visuals.widgets.hovered.weak_bg_fill;
        let rows: Vec<Rect> = h
            .fills
            .iter()
            .filter(|(r, c)| *c == fill && r.left() < 20.0 && r.width() > 300.0)
            .map(|(r, _)| *r)
            .collect();
        assert_eq!(rows.len(), 1, "{:?}", h.fills);
        assert!(rows[0].y_range().contains(b.y));
    }

    #[test]
    fn clicking_right_of_a_short_name_selects_the_layer() {
        let mut h = Harness::new(doc_with(vec![named(1, "a"), named(2, "b")]));
        let p = h.label("b");
        h.click(Pos2::new(380.0, p.y));
        assert_eq!(h.selected, Some(2));
    }

    #[test]
    fn selected_row_is_highlighted_across_its_width() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha"), named(2, "beta")]));
        let alpha = h.label("alpha");
        h.click(alpha);
        assert_eq!(h.selected, Some(1));
        let beta = h.label("beta");
        let fill = h.ctx.global_style().visuals.selection.bg_fill;
        let rows: Vec<Rect> = h
            .fills
            .iter()
            .filter(|(r, c)| *c == fill && r.left() < 20.0 && r.width() > 300.0)
            .map(|(r, _)| *r)
            .collect();
        assert_eq!(rows.len(), 1, "one full-row highlight: {:?}", h.fills);
        let row = rows[0];
        // From the eye icon on the left to the name and beyond.
        assert!(
            row.left() < 20.0 && row.y_range().contains(alpha.y),
            "{row:?}"
        );
        assert!(!row.y_range().contains(beta.y), "{row:?}");
    }

    #[test]
    fn panel_click_selects_and_drag_drops_into_group() {
        let mut d = doc_with(vec![named(1, "alpha"), named(2, "beta")]);
        group(&mut d, &mut Some(2)); // alpha 100[beta]
        let mut h = Harness::new(d);

        let g = h.label("Group 1");
        h.click(g);
        assert_eq!(h.selected, Some(100), "clicking a group row selects it");

        // Drop alpha onto the middle of the group row.
        let from = h.label("alpha");
        let to = h.label("Group 1");
        h.drag(from, to);
        assert_eq!(shape(&h.doc.layers), "100[2 1]");

        // Drop alpha below beta (lower half of beta's row): bottom of the group.
        let from = h.label("alpha");
        let to = h.label("beta") + vec2(0.0, 7.0);
        h.drag(from, to);
        assert_eq!(shape(&h.doc.layers), "100[1 2]");
    }

    #[test]
    fn panel_adds_and_removes_effects() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        h.selected = Some(1);
        let p = h.add_effect_button();
        h.click(p);
        let p = h.label("Invert");
        h.click(p);
        let effects = |h: &Harness| h.doc.layer(1).unwrap().effects.clone();
        assert_eq!(effects(&h).len(), 1);
        let labels = h.frame(vec![]);
        assert!(
            !labels.iter().any(|(t, _)| t == "Sepia"),
            "menu closed after picking"
        );
        assert_eq!(effects(&h)[0].effect, "builtin.invert");

        // Parameters get controls built from their specs.
        let p = h.add_effect_button();
        h.click(p);
        let p = h.label("Gaussian blur");
        h.click(p);
        h.label("Radius");
        assert_eq!(effects(&h)[1].effect, "builtin.blur");

        let ids =
            |h: &Harness| -> Vec<String> { effects(h).into_iter().map(|e| e.effect).collect() };
        // Dragging the name does nothing: only the handle moves effects.
        let from = h.label("Invert");
        let to = h.label("Radius");
        h.drag(from, to);
        assert_eq!(ids(&h), ["builtin.invert", "builtin.blur"]);

        // Drag Invert's handle onto the lower half of the blur (its Radius row).
        let from = h.handle("Invert");
        let to = h.label("Radius");
        h.drag(from, to);
        assert_eq!(ids(&h), ["builtin.blur", "builtin.invert"]);
        // And back, released in the space just above the list.
        let from = h.handle("Invert");
        let to = h.handle("Gaussian blur") - vec2(0.0, 14.0);
        h.drag(from, to);
        assert_eq!(ids(&h), ["builtin.invert", "builtin.blur"]);

        // Remove from the right-click menu; the blur stays.
        let p = h.label("Invert");
        h.secondary_click(p);
        let p = h.label("Remove");
        h.click(p);
        assert_eq!(ids(&h), ["builtin.blur"]);
    }

    #[test]
    fn right_click_duplicates_a_layer_with_its_effects() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        h.doc
            .layer_mut(1)
            .unwrap()
            .effects
            .push(crate::effects::EffectInstance::new(
                crate::effects::registry().get("builtin.invert").unwrap(),
            ));
        let p = h.label("alpha");
        h.secondary_click(p);
        let p = h.label("Duplicate");
        h.click(p);
        let copy = h
            .doc
            .node(h.selected.unwrap())
            .and_then(Node::as_layer)
            .unwrap();
        assert_eq!(copy.name, "alpha copy");
        assert_eq!(copy.effects, h.doc.layer(1).unwrap().effects);
        assert_ne!(copy.id, 1);
    }

    #[test]
    fn text_layer_shows_its_text_settings() {
        let mut doc = Document::default();
        let mut layer = text::new_layer(1, "Hello", text::BUILTIN_FAMILY.into(), [50.0, 50.0]);
        // Laid out, as the app does before showing the panel.
        let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        layer.size_px = text::layout(layer.text.as_ref().unwrap(), &font).size;
        doc.layers.push(Node::Layer(Box::new(layer)));
        let mut h = Harness::new(doc);
        h.selected = Some(1);
        let labels = h.frame(vec![]);
        for want in [
            "Font",
            "Font size",
            "Letter spacing",
            "Outline",
            "Properties",
        ] {
            assert!(labels.iter().any(|(t, _)| t == want), "no {want:?}");
        }
        assert!(
            !labels
                .iter()
                .any(|(t, _)| t.starts_with("Print resolution")),
            "no DPI for text"
        );
        // The font size field shows the default size.
        assert!(labels.iter().any(|(t, _)| t == "24.0"), "{labels:?}");
        // The text settings are a part of Properties, below the common ones.
        let y = |want: &str| labels.iter().find(|(t, _)| t == want).unwrap().1.y;
        assert!(y("Properties") < y("Opacity") && y("Opacity") < y("Text"));
        assert!(y("Text") < y("Font"));
    }

    #[test]
    fn panel_eye_toggles_visibility() {
        let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
        let name = h.label("alpha");
        // The eye sits left of the lock and the thumbnail space.
        let eye = name - vec2(3.0 + THUMB + 3.0 + ICON + 3.0 + ICON / 2.0 + 4.0, 0.0);
        h.click(eye);
        assert!(!h.doc.layer(1).unwrap().visible);
        h.click(eye);
        assert!(h.doc.layer(1).unwrap().visible);
    }

    #[test]
    fn group_and_ungroup() {
        let mut d = doc_with(vec![layer(1), layer(2)]);
        let mut sel = Some(1);
        assert!(group(&mut d, &mut sel));
        assert_eq!(shape(&d.layers), "100[1] 2");
        assert_eq!(sel, Some(100));
        if let Some(Node::Group(g)) = d.node_mut(100) {
            g.opacity = 0.5;
            g.visible = false;
        }
        assert!(ungroup(&mut d, &mut sel));
        assert_eq!(shape(&d.layers), "1 2");
        let l = d.layer(1).unwrap();
        assert_eq!((l.opacity, l.visible), (0.5, false));
    }

    #[test]
    fn steps_enter_and_leave_groups() {
        let mut d = doc_with(vec![layer(1), layer(2)]);
        group(&mut d, &mut Some(2)); // 1 100[2]
        assert!(move_step(&mut d, 1, true));
        assert_eq!(shape(&d.layers), "100[1 2]");
        assert!(move_step(&mut d, 1, true));
        assert_eq!(shape(&d.layers), "100[2 1]");
        assert!(move_step(&mut d, 1, true));
        assert_eq!(shape(&d.layers), "100[2] 1");
        assert!(!can_step(&d, 1, true));
        assert!(move_step(&mut d, 1, false));
        assert_eq!(shape(&d.layers), "100[2 1]");
    }

    #[test]
    fn drag_and_drop_moves() {
        let mut d = doc_with(vec![layer(1), layer(2), layer(3)]);
        group(&mut d, &mut Some(3)); // 1 2 100[3]
        assert!(drop_node(&mut d, 1, DropTarget::Into(100)));
        assert_eq!(shape(&d.layers), "2 100[3 1]");
        assert!(drop_node(&mut d, 2, DropTarget::Above(3)));
        assert_eq!(shape(&d.layers), "100[3 2 1]");
        assert!(drop_node(&mut d, 1, DropTarget::Below(100)));
        assert_eq!(shape(&d.layers), "1 100[3 2]");
        // A group cannot go inside itself.
        let mut sel = Some(100);
        group(&mut d, &mut sel); // 1 101[100[3 2]]
        assert!(!drop_node(&mut d, 101, DropTarget::Into(100)));
        assert_eq!(shape(&d.layers), "1 101[100[3 2]]");
    }

    #[test]
    fn duplicate_group_gets_fresh_ids_and_delete_selects_neighbor() {
        let mut d = doc_with(vec![layer(1), layer(2)]);
        let mut sel = Some(2);
        group(&mut d, &mut sel); // 1 100[2]
        assert!(duplicate(&mut d, &mut sel));
        assert_eq!(shape(&d.layers), "1 100[2] 101[102]");
        assert_eq!(sel, Some(101));
        assert!(delete(&mut d, &mut sel));
        assert_eq!(sel, Some(100));
        let mut sel = Some(2);
        delete(&mut d, &mut sel);
        assert_eq!(sel, Some(100), "falls back to the parent group");
    }

    #[test]
    fn align_uses_drawn_bounds_and_moves_groups_as_a_block() {
        let sized = |id: u64, center: P2| {
            let mut l = crate::document::Layer::new(id, format!("l{id}"), "x.png".into());
            l.size_px = [20, 10];
            l.transform.center = center;
            Node::Layer(Box::new(l))
        };
        let page = ([0.0, 0.0], [297.0, 210.0]);
        let mut d = doc_with(vec![sized(1, [50.0, 50.0])]);
        assert!(align(&mut d, Some(1), Align::Left, page));
        assert_eq!(d.layer(1).unwrap().transform.center, [10.0, 50.0]);
        assert!(align(&mut d, Some(1), Align::Bottom, page));
        assert_eq!(d.layer(1).unwrap().transform.center, [10.0, 205.0]);
        assert!(align(&mut d, Some(1), Align::CenterX, page));
        assert!(align(&mut d, Some(1), Align::CenterY, page));
        assert_eq!(d.layer(1).unwrap().transform.center, [148.5, 105.0]);
        assert!(
            !align(&mut d, Some(1), Align::CenterY, page),
            "already there"
        );

        // Rotated a quarter turn, the 20 x 10 image is 10 wide on the page.
        d.layer_mut(1).unwrap().transform.rotation_deg = 90.0;
        assert!(align(&mut d, Some(1), Align::Right, page));
        assert!((d.layer(1).unwrap().transform.center[0] - 292.0).abs() < 1e-3);

        // A group keeps its members' spacing.
        let mut d = doc_with(vec![sized(1, [50.0, 50.0]), sized(2, [100.0, 80.0])]);
        group(&mut d, &mut Some(1));
        let g = d.layers.iter().find_map(|n| n.as_group()).unwrap().id;
        // Group holds layer 1 only; put 2 in as well.
        drop_node(&mut d, 2, DropTarget::Into(g));
        assert!(align(&mut d, Some(g), Align::Top, page));
        assert_eq!(d.layer(1).unwrap().transform.center, [50.0, 5.0]);
        assert_eq!(d.layer(2).unwrap().transform.center, [100.0, 35.0]);
        assert!(align(&mut d, Some(g), Align::Right, page));
        assert_eq!(d.layer(1).unwrap().transform.center, [237.0, 5.0]);
        assert_eq!(d.layer(2).unwrap().transform.center, [287.0, 35.0]);

        // Locked layers stay put.
        d.layer_mut(1).unwrap().locked = true;
        assert!(!align(&mut d, Some(1), Align::Left, page));
    }

    #[test]
    fn nudge_moves_unlocked_group_members() {
        let mut d = doc_with(vec![layer(1), layer(2)]);
        group(&mut d, &mut Some(1));
        drop_node(&mut d, 2, DropTarget::Into(100));
        d.layer_mut(2).unwrap().locked = true;
        assert!(nudge(&mut d, Some(100), [1.0, 0.0]));
        assert_eq!(d.layer(1).unwrap().transform.center, [1.0, 0.0]);
        assert_eq!(d.layer(2).unwrap().transform.center, [0.0, 0.0]);
        if let Some(Node::Group(g)) = d.node_mut(100) {
            g.locked = true;
        }
        assert!(!nudge(&mut d, Some(1), [1.0, 0.0]));
    }
}
