//! The artwork canvas: pan/zoom camera, page, layers, template elements and selection gizmo.

use egui::epaint::Vertex;
use egui::{
    Color32, CursorIcon, Event, Key, Mesh, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2,
    Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::document::{Affine, Document, Homography, Layer, Node, P2, Transform, is_convex};
use crate::layers_ui;
use crate::render::{self, SceneNode};
use crate::render::{Cuts, PlacedElement};
use crate::template::ElementKind;

/// Screen points per mm when the page is shown at real size on a 96 DPI screen.
pub const REAL_SIZE_ZOOM: f32 = 96.0 / 25.4;
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 80.0;
const HANDLE_RADIUS: f32 = 6.0;
const ROTATE_HANDLE_OFFSET: f32 = 26.0;
/// Smallest width or height an edge drag can squash a layer to.
const MIN_STRETCH_MM: f32 = 0.5;
/// Grid cells per side when drawing a distorted layer, so the preview is close to a
/// true perspective (a single textured quad bends along its diagonal).
const WARP_GRID: usize = 16;
const ACCENT: Color32 = Color32::from_rgb(0, 160, 255);
/// Longest side of the offscreen render of a semi-transparent group.
const MAX_GROUP_PREVIEW_SIDE: f32 = 2048.0;

/// Maps page mm to the screen: zoom and pan.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Screen points per mm.
    pub zoom: f32,
    /// Screen offset of the page's top-left corner from the canvas' top-left corner.
    pub pan: Vec2,
}

impl Camera {
    /// Page mm to a screen position.
    pub fn to_screen(self, rect: Rect, p: P2) -> Pos2 {
        rect.min + self.pan + vec2(p[0], p[1]) * self.zoom
    }

    /// Page mm to screen points, as an affine map.
    pub fn affine(self, rect: Rect) -> Affine {
        let o = rect.min + self.pan;
        Affine::scale(self.zoom, self.zoom).then(Affine::translate(o.x, o.y))
    }

    /// Screen position to page mm.
    pub fn to_mm(self, rect: Rect, s: Pos2) -> P2 {
        let v = (s - rect.min - self.pan) / self.zoom;
        [v.x, v.y]
    }

    /// Zooms by `factor`, keeping the point under `anchor` fixed.
    pub fn zoom_at(&mut self, rect: Rect, anchor: Pos2, factor: f32) {
        let mm = self.to_mm(rect, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = anchor - rect.min - vec2(mm[0], mm[1]) * self.zoom;
    }

    /// Zooms and centres so the whole page fits, with a margin.
    pub fn fit(&mut self, rect: Rect, page: P2) {
        let margin = 0.92;
        self.zoom = (rect.width() / page[0])
            .min(rect.height() / page[1])
            .clamp(MIN_ZOOM, MAX_ZOOM)
            * margin;
        self.pan = (rect.size() - vec2(page[0], page[1]) * self.zoom) / 2.0;
    }
}

/// What a drag on the canvas does.
#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    Scale,
    Rotate,
    /// Moves one corner on its own (perspective distortion).
    Distort(usize),
    /// Stretches along one image axis from the middle of an edge: top, right, bottom, left.
    Stretch(usize),
}

/// Middles of the image edges in unit-square coordinates: top, right, bottom, left.
const EDGE_MIDS: [P2; 4] = [[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]];

/// A drag in progress on the canvas.
struct Drag {
    kind: DragKind,
    /// Layers being moved with their starting placement. Scale and rotate use only one.
    layers: Vec<(u64, Transform)>,
    start_mouse: P2,
}

/// A semi-transparent group rendered offscreen, so that its children blend with each
/// other first and the group's opacity applies once, as in the export.
struct GroupPreview {
    key: u64,
    texture: egui::TextureHandle,
    /// Page area covered by the texture: min and max corners in mm.
    rect_mm: (P2, P2),
}

/// What the template cuts away, rendered over the page.
struct CutPreview {
    key: u64,
    texture: egui::TextureHandle,
}

/// Canvas state that lasts across frames: view, drag, display options and caches.
pub struct CanvasState {
    /// Current view.
    pub camera: Camera,
    /// Re-fit the page on the next frame.
    pub fit_pending: bool,
    /// Draw the template's lines at all.
    pub show_template: bool,
    /// Per element kind (`ElementKind::index`): lines shown, and their colour.
    pub element_show: [bool; 3],
    /// Line colour per element kind.
    pub element_color: [Color32; 3],
    /// Grey out what the template cuts away, as the export leaves it blank.
    pub preview_cuts: bool,
    /// What the export cuts; set by the app from the export settings.
    pub cuts: Cuts,
    cut_preview: Option<CutPreview>,
    /// Mouse position on the page, for the status bar.
    pub cursor_mm: Option<P2>,
    /// Corner handles distort the selected layer instead of scaling it (Ctrl does the
    /// same).
    pub distort_mode: bool,
    rect: Rect,
    drag: Option<Drag>,
    /// A left drag that started on empty canvas pans the view.
    panning: bool,
    group_previews: HashMap<u64, GroupPreview>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            camera: Camera {
                zoom: 2.0,
                pan: Vec2::ZERO,
            },
            fit_pending: true,
            show_template: true,
            element_show: [true; 3],
            element_color: ElementKind::ALL.map(|k| {
                let [r, g, b] = k.default_color();
                Color32::from_rgb(r, g, b)
            }),
            preview_cuts: true,
            cuts: Cuts::default(),
            cut_preview: None,
            cursor_mm: None,
            distort_mode: false,
            rect: Rect::NOTHING,
            drag: None,
            panning: false,
            group_previews: HashMap::new(),
        }
    }
}

impl CanvasState {
    /// Zooms around the centre of the canvas (toolbar buttons).
    pub fn zoom_centered(&mut self, factor: f32) {
        if self.rect.is_positive() {
            self.camera.zoom_at(self.rect, self.rect.center(), factor);
        }
    }

    /// Sets an absolute zoom around the centre of the canvas.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom_centered(zoom / self.camera.zoom);
    }
}

/// Screen positions of the edge-middle handles (top, right, bottom, left), on the drawn
/// (possibly distorted) edges.
fn edge_handles(cam: &Camera, rect: Rect, layer: &Layer) -> [Pos2; 4] {
    let c = layer.corners().map(|p| cam.to_screen(rect, p));
    std::array::from_fn(|i| c[i] + (c[(i + 1) % 4] - c[i]) / 2.0)
}

/// Outward unit direction (page mm) of image edge `edge` and the image's full extent along
/// it, in the undistorted frame of `t`.
fn edge_axis(t: Transform, size_px: [u32; 2], edge: usize) -> (P2, f32) {
    let m = t.to_affine(size_px);
    let (w, h) = (size_px[0] as f32, size_px[1] as f32);
    let mid = m.apply([EDGE_MIDS[edge][0] * w, EDGE_MIDS[edge][1] * h]);
    let v = [mid[0] - t.center[0], mid[1] - t.center[1]];
    let half = len(v);
    if half < 1e-6 {
        return ([0.0, 0.0], 0.0);
    }
    ([v[0] / half, v[1] / half], half * 2.0)
}

/// Screen positions of the four corners, the middle of the top edge and the rotation
/// handle.
fn handles(cam: &Camera, rect: Rect, layer: &Layer) -> ([Pos2; 4], Pos2, Pos2) {
    let c = layer.corners().map(|p| cam.to_screen(rect, p));
    let center = cam.to_screen(rect, layer.transform.center);
    let top_mid = c[0] + (c[1] - c[0]) / 2.0;
    let dir = (top_mid - center).normalized();
    let dir = if dir.is_finite() { dir } else { -Vec2::Y };
    (c, top_mid, top_mid + dir * ROTATE_HANDLE_OFFSET)
}

/// The topmost visible, unlocked layer under a page point.
fn topmost_hit(doc: &Document, p: P2) -> Option<u64> {
    doc.leaves()
        .into_iter()
        .rev()
        .find(|l| l.visible && !l.locked && l.layer.contains(p))
        .map(|l| l.layer.id)
}

/// Rotates a vector by `deg` degrees, clockwise on screen.
fn rotate_vec(v: P2, deg: f32) -> P2 {
    let (s, c) = deg.to_radians().sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1]]
}

/// Length of a vector.
fn len(v: P2) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Draws the canvas and handles its input. Returns true if the document changed.
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    elements: &[PlacedElement],
    selected: &mut Option<u64>,
    state: &mut CanvasState,
) -> bool {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let mut changed = false;

    // Keep the view centred when the window resizes, and fit on demand.
    if state.fit_pending || !state.rect.is_positive() {
        state.camera.fit(rect, doc.page_mm());
        state.fit_pending = false;
    } else if state.rect.size() != rect.size() {
        state.camera.pan += (rect.size() - state.rect.size()) / 2.0;
    }
    state.rect = rect;

    // Only when no window, dialog or popup covers the canvas under the pointer.
    let hover = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|_| ui.rect_contains_pointer(rect));
    state.cursor_mm = hover.map(|p| state.camera.to_mm(rect, p));

    // Wheel: a mouse wheel or Ctrl+scroll zooms, a trackpad scroll pans, a pinch zooms.
    if let Some(pos) = hover {
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            match ev {
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    source,
                    ..
                } => {
                    let is_wheel = match source {
                        MouseWheelSource::Wheel => true,
                        MouseWheelSource::Unknown => unit != MouseWheelUnit::Point,
                        _ => false,
                    };
                    if modifiers.command || modifiers.ctrl || is_wheel {
                        let steps = match unit {
                            MouseWheelUnit::Point => delta.y / 40.0,
                            MouseWheelUnit::Line => delta.y,
                            MouseWheelUnit::Page => delta.y * 5.0,
                        };
                        state.camera.zoom_at(rect, pos, 1.15f32.powf(steps));
                    } else {
                        state.camera.pan += delta;
                    }
                }
                Event::Zoom(factor) => state.camera.zoom_at(rect, pos, factor),
                _ => {}
            }
        }
    }

    // Panning: left drag on empty canvas, middle or right drag, or Space + left drag.
    let space = ui.input(|i| i.key_down(Key::Space));
    if !ui.input(|i| i.pointer.primary_down()) {
        state.panning = false;
    }
    if response.dragged_by(PointerButton::Middle)
        || response.dragged_by(PointerButton::Secondary)
        || (response.dragged_by(PointerButton::Primary) && (space || state.panning))
    {
        state.camera.pan += response.drag_delta();
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if space && hover.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }

    let cam = state.camera;
    let shift = ui.input(|i| i.modifiers.shift);
    let alt = ui.input(|i| i.modifiers.alt);
    let distorting = state.distort_mode || ui.input(|i| i.modifiers.command);

    // Which handle (if any) of the selected layer is under a screen point.
    let handle_at = |doc: &Document, sel: Option<u64>, p: Pos2| -> Option<(u64, DragKind)> {
        let id = sel?;
        let layer = doc.layer(id)?;
        if doc.effectively_locked(id) || !doc.effectively_visible(id) {
            return None;
        }
        let (corners, _, rot) = handles(&cam, rect, layer);
        if rot.distance(p) <= HANDLE_RADIUS * 1.5 {
            Some((layer.id, DragKind::Rotate))
        } else if let Some(i) = corners
            .iter()
            .position(|c| c.distance(p) <= HANDLE_RADIUS * 1.5)
        {
            let kind = if distorting {
                DragKind::Distort(i)
            } else {
                DragKind::Scale
            };
            Some((layer.id, kind))
        } else {
            edge_handles(&cam, rect, layer)
                .iter()
                .position(|c| c.distance(p) <= HANDLE_RADIUS * 1.5)
                .map(|i| (layer.id, DragKind::Stretch(i)))
        }
    };

    if !space {
        if response.drag_started_by(PointerButton::Primary) {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos());
            if let Some(origin) = origin {
                let m = cam.to_mm(rect, origin);
                let (ids, kind) = match handle_at(doc, *selected, origin) {
                    Some((id, kind)) => (vec![id], kind),
                    None => match (topmost_hit(doc, m), *selected) {
                        // Dragging a member of the selected group moves the whole group.
                        (Some(hit), Some(sel)) if hit != sel && doc.is_within(hit, sel) => {
                            (layers_ui::movable_layers(doc, sel), DragKind::Move)
                        }
                        (hit, _) => {
                            *selected = hit;
                            (hit.into_iter().collect(), DragKind::Move)
                        }
                    },
                };
                let layers: Vec<(u64, Transform)> = ids
                    .into_iter()
                    .filter_map(|id| Some((id, doc.layer(id)?.transform)))
                    .collect();
                state.drag = (!layers.is_empty()).then_some(Drag {
                    kind,
                    layers,
                    start_mouse: m,
                });
                // Nothing grabbed: this drag pans the view.
                state.panning = state.drag.is_none();
            }
        } else if response.clicked_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
            && handle_at(doc, *selected, p).is_none()
        {
            *selected = topmost_hit(doc, cam.to_mm(rect, p));
        }
    }

    if let Some(drag) = &state.drag {
        if response.dragged_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
            && drag.kind == DragKind::Move
        {
            let m = cam.to_mm(rect, p);
            let d = [m[0] - drag.start_mouse[0], m[1] - drag.start_mouse[1]];
            for (id, start) in &drag.layers {
                if let Some(layer) = doc.layer_mut(*id) {
                    layer.transform.center = [start.center[0] + d[0], start.center[1] + d[1]];
                }
            }
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            changed = true;
        } else if response.dragged_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
            && let Some(&(id, s)) = drag.layers.first()
            && let Some(layer) = doc.layer_mut(id)
        {
            let m = cam.to_mm(rect, p);
            let c = s.center;
            let v0 = [drag.start_mouse[0] - c[0], drag.start_mouse[1] - c[1]];
            let v = [m[0] - c[0], m[1] - c[1]];
            let t = &mut layer.transform;
            match drag.kind {
                DragKind::Move => {}
                DragKind::Stretch(edge) => {
                    // The opposite edge stays put; with Alt, both move (centre fixed).
                    let (dir, extent) = edge_axis(s, layer.size_px, edge);
                    let axis = if edge % 2 == 0 { 1 } else { 0 };
                    let px = layer.size_px[axis].max(1) as f32;
                    if extent > 0.0 {
                        let d = [m[0] - drag.start_mouse[0], m[1] - drag.start_mouse[1]];
                        let pull = d[0] * dir[0] + d[1] * dir[1];
                        let both = if alt { 2.0 } else { 1.0 };
                        let new = (extent + pull * both).max(MIN_STRETCH_MM);
                        t.scale[axis] = new / px;
                        let shift_c = if alt { 0.0 } else { (new - extent) / 2.0 };
                        t.center = [
                            s.center[0] + dir[0] * shift_c,
                            s.center[1] + dir[1] * shift_c,
                        ];
                    }
                }
                DragKind::Scale if shift => {
                    // Free scale along the image's own axes.
                    let (l0, l) = (
                        rotate_vec(v0, -s.rotation_deg),
                        rotate_vec(v, -s.rotation_deg),
                    );
                    let ratio = |a: f32, b: f32| if a.abs() > 1e-3 { (b / a).abs() } else { 1.0 };
                    t.scale = [
                        (s.scale[0] * ratio(l0[0], l[0])).max(1e-4),
                        (s.scale[1] * ratio(l0[1], l[1])).max(1e-4),
                    ];
                }
                DragKind::Scale => {
                    let f = if len(v0) > 1e-3 {
                        len(v) / len(v0)
                    } else {
                        1.0
                    };
                    t.scale = [(s.scale[0] * f).max(1e-4), (s.scale[1] * f).max(1e-4)];
                }
                DragKind::Distort(i) => {
                    // Corner offsets live in the undistorted image frame, which this drag
                    // does not change.
                    if let (Some(a), Some(b)) =
                        (layer.page_to_unit(drag.start_mouse), layer.page_to_unit(m))
                    {
                        let mut d = s.distort;
                        d[i] = [d[i][0] + b[0] - a[0], d[i][1] + b[1] - a[1]];
                        let mut t2 = s;
                        t2.distort = d;
                        // A corner dragged across the others would fold the image.
                        if is_convex(t2.unit_quad()) {
                            layer.transform.distort = d;
                        }
                    }
                }
                DragKind::Rotate => {
                    let delta = (v[1].atan2(v[0]) - v0[1].atan2(v0[0])).to_degrees();
                    let mut r = s.rotation_deg + delta;
                    if shift {
                        r = (r / 15.0).round() * 15.0;
                    }
                    t.rotation_deg = (r + 180.0).rem_euclid(360.0) - 180.0;
                }
            }
            changed = true;
        }
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            state.drag = None;
        }
    } else if let Some(p) = hover
        && !space
    {
        match handle_at(doc, *selected, p) {
            Some((_, DragKind::Rotate)) => ui.ctx().set_cursor_icon(CursorIcon::Alias),
            Some((id, DragKind::Stretch(edge))) => {
                // Resize cursor along the edge's direction on screen.
                let horizontal = doc.layer(id).is_some_and(|l| {
                    let (dir, _) = edge_axis(l.transform, l.size_px, edge);
                    dir[0].abs() >= dir[1].abs()
                });
                ui.ctx().set_cursor_icon(if horizontal {
                    CursorIcon::ResizeHorizontal
                } else {
                    CursorIcon::ResizeVertical
                });
            }
            Some(_) => ui.ctx().set_cursor_icon(CursorIcon::Crosshair),
            None if topmost_hit(doc, cam.to_mm(rect, p)).is_some() => {
                ui.ctx().set_cursor_icon(CursorIcon::Move)
            }
            None => ui.ctx().set_cursor_icon(CursorIcon::Grab),
        }
    }

    paint(ui, doc, elements, *selected, state, rect, distorting);
    changed
}

/// Draws a textured quad with the given corners on screen.
fn paint_quad(painter: &egui::Painter, tex: egui::TextureId, corners: [Pos2; 4], opacity: f32) {
    let uv = |u: f32, v: f32| {
        let top = corners[0] + (corners[1] - corners[0]) * u;
        let bottom = corners[3] + (corners[2] - corners[3]) * u;
        top + (bottom - top) * v
    };
    paint_grid(painter, tex, 1, uv, opacity);
}

/// Draws a texture through a map from texture coordinates (0..1) to screen, as an
/// `n` x `n` grid of quads.
fn paint_grid(
    painter: &egui::Painter,
    tex: egui::TextureId,
    n: usize,
    map: impl Fn(f32, f32) -> Pos2,
    opacity: f32,
) {
    let tint = Color32::WHITE.gamma_multiply(opacity.clamp(0.0, 1.0));
    let mut mesh = Mesh::with_texture(tex);
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            mesh.vertices.push(Vertex {
                pos: map(u, v),
                uv: pos2(u, v),
                color: tint,
            });
        }
    }
    let row = n as u32 + 1;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let k = j * row + i;
            mesh.add_triangle(k, k + 1, k + row + 1);
            mesh.add_triangle(k, k + row + 1, k + row);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws a layer's texture, in perspective if it is distorted.
fn paint_layer(
    painter: &egui::Painter,
    cam: Camera,
    rect: Rect,
    l: &Layer,
    tex: egui::TextureId,
    opacity: f32,
) {
    let corners = l.corners().map(|p| cam.to_screen(rect, p));
    if !l.transform.is_distorted() {
        paint_quad(painter, tex, corners, opacity);
        return;
    }
    let m: Homography = l.homography().then_affine(cam.affine(rect));
    let (w, h) = (l.size_px[0] as f32, l.size_px[1] as f32);
    paint_grid(
        painter,
        tex,
        WARP_GRID,
        |u, v| {
            let p = m.apply([u * w, v * h]);
            pos2(p[0], p[1])
        },
        opacity,
    );
}

/// Missing image: its footprint crossed out.
fn paint_missing(painter: &egui::Painter, corners: [Pos2; 4]) {
    let stroke = Stroke::new(1.5, Color32::from_rgb(220, 60, 60));
    painter.add(Shape::closed_line(corners.to_vec(), stroke));
    painter.line_segment([corners[0], corners[2]], stroke);
    painter.line_segment([corners[1], corners[3]], stroke);
}

/// Hashes what a group preview depends on, to know when to render it again.
fn hash_scene(nodes: &[SceneNode], h: &mut impl Hasher) {
    for n in nodes {
        match n {
            SceneNode::Image(l) => {
                0u8.hash(h);
                (std::sync::Arc::as_ptr(&l.pixmap) as usize).hash(h);
                for v in l.map.m {
                    v.to_bits().hash(h);
                }
                l.opacity.to_bits().hash(h);
            }
            SceneNode::Group { opacity, children } => {
                1u8.hash(h);
                opacity.to_bits().hash(h);
                children.len().hash(h);
                hash_scene(children, h);
            }
        }
    }
}

/// Draws the layer tree on the canvas, reusing cached group previews.
struct Painting<'a> {
    painter: egui::Painter,
    ctx: egui::Context,
    cam: Camera,
    rect: Rect,
    page: P2,
    /// Previews of the previous frame; the ones still in use move to `used`.
    cache: &'a mut HashMap<u64, GroupPreview>,
    used: HashMap<u64, GroupPreview>,
}

impl Painting<'_> {
    /// Draws nodes under an inherited opacity. Semi-transparent groups with overlapping
    /// children go through an offscreen render.
    fn nodes(&mut self, nodes: &[Node], opacity: f32) {
        for node in nodes {
            match node {
                Node::Layer(l) if l.visible => match &l.texture {
                    Some(tex) => paint_layer(
                        &self.painter,
                        self.cam,
                        self.rect,
                        l,
                        tex.id(),
                        l.opacity * opacity,
                    ),
                    None => paint_missing(
                        &self.painter,
                        l.corners().map(|p| self.cam.to_screen(self.rect, p)),
                    ),
                },
                Node::Group(g) if g.visible && g.opacity > 0.0 => {
                    if g.opacity >= 1.0 {
                        self.nodes(&g.children, opacity);
                        continue;
                    }
                    let scene = render::scene_nodes(&g.children, true);
                    if scene.len() <= 1 || !self.group(g.id, &scene, opacity * g.opacity) {
                        // Nothing overlaps within the group: per-layer opacity is exact.
                        self.nodes(&g.children, opacity * g.opacity);
                    } else {
                        self.missing(&g.children);
                    }
                }
                _ => {}
            }
        }
    }

    /// Footprints of visible layers whose image is missing.
    fn missing(&self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Layer(l) if l.visible && l.texture.is_none() => {
                    paint_missing(
                        &self.painter,
                        l.corners().map(|p| self.cam.to_screen(self.rect, p)),
                    );
                }
                Node::Group(g) if g.visible => self.missing(&g.children),
                _ => {}
            }
        }
    }

    /// Draws a group composited offscreen. Returns false if it could not be rendered.
    fn group(&mut self, id: u64, scene: &[SceneNode], opacity: f32) -> bool {
        let Some((lo, hi)) = render::bounds(scene, Affine::IDENTITY) else {
            return true;
        };
        // Content far off the page is not worth the pixels.
        let [pw, ph] = self.page;
        let lo = [lo[0].max(-pw), lo[1].max(-ph)];
        let hi = [hi[0].min(2.0 * pw), hi[1].min(2.0 * ph)];
        if hi[0] <= lo[0] || hi[1] <= lo[1] {
            return true;
        }
        // Resolution follows the zoom in powers of two, so zooming re-renders rarely.
        let screen_px = self.cam.zoom * self.ctx.pixels_per_point();
        let mut px_per_mm = 2f32.powf(screen_px.max(1e-3).log2().ceil());
        let longest = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        px_per_mm = px_per_mm.min(MAX_GROUP_PREVIEW_SIDE / longest);
        let size = [
            ((hi[0] - lo[0]) * px_per_mm).ceil().max(1.0) as u32,
            ((hi[1] - lo[1]) * px_per_mm).ceil().max(1.0) as u32,
        ];
        let mut h = std::collections::hash_map::DefaultHasher::new();
        hash_scene(scene, &mut h);
        px_per_mm.to_bits().hash(&mut h);
        for v in [lo[0], lo[1], hi[0], hi[1]] {
            v.to_bits().hash(&mut h);
        }
        let key = h.finish();

        let preview = match self.cache.remove(&id) {
            Some(p) if p.key == key => p,
            old => {
                let pixmap = match render::compose_region(scene, lo, size, px_per_mm) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("group preview: {e}");
                        return false;
                    }
                };
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [size[0] as usize, size[1] as usize],
                    pixmap.data(),
                );
                let texture = match old {
                    Some(mut p) => {
                        p.texture.set(image, egui::TextureOptions::LINEAR);
                        p.texture
                    }
                    None => self.ctx.load_texture(
                        format!("group-{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                };
                GroupPreview {
                    key,
                    texture,
                    rect_mm: (lo, hi),
                }
            }
        };
        let (a, b) = preview.rect_mm;
        let corners = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]
            .map(|p| self.cam.to_screen(self.rect, p));
        paint_quad(&self.painter, preview.texture.id(), corners, opacity);
        self.used.insert(id, preview);
        true
    }
}

/// Covers what the template cuts away (outside the outline, inside holes) with paper,
/// slightly see-through so the artwork underneath can still be lined up.
fn paint_cuts(
    ctx: &egui::Context,
    painter: &egui::Painter,
    page: P2,
    elements: &[PlacedElement],
    state: &mut CanvasState,
    rect: Rect,
) {
    if !state.preview_cuts || elements.is_empty() {
        state.cut_preview = None;
        return;
    }
    let cam = state.camera;
    // Resolution follows the zoom in powers of two, like group previews.
    let screen_px = cam.zoom * ctx.pixels_per_point();
    let longest = page[0].max(page[1]);
    let px_per_mm = 2f32
        .powf(screen_px.max(1e-3).log2().ceil())
        .min(MAX_GROUP_PREVIEW_SIDE * 2.0 / longest);
    let size = [
        (page[0] * px_per_mm).ceil().max(1.0) as u32,
        (page[1] * px_per_mm).ceil().max(1.0) as u32,
    ];
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state.cuts.hash(&mut h);
    px_per_mm.to_bits().hash(&mut h);
    for e in elements {
        e.kind.hash(&mut h);
        for poly in &e.polylines {
            poly.closed.hash(&mut h);
            for p in &poly.points {
                p[0].to_bits().hash(&mut h);
                p[1].to_bits().hash(&mut h);
            }
        }
    }
    let key = h.finish();
    if state.cut_preview.as_ref().is_none_or(|c| c.key != key) {
        let Some(keep) =
            crate::render::keep_mask(elements, state.cuts, [0.0, 0.0], size, px_per_mm)
        else {
            state.cut_preview = None;
            return;
        };
        // Premultiplied white, opaque where everything is cut.
        let rgba: Vec<u8> = keep
            .iter()
            .flat_map(|&k| {
                let a = ((255 - k) as f32 * 0.88) as u8;
                [a, a, a, a]
            })
            .collect();
        let image =
            egui::ColorImage::from_rgba_premultiplied([size[0] as usize, size[1] as usize], &rgba);
        let texture = match state.cut_preview.take() {
            Some(mut c) => {
                c.texture.set(image, egui::TextureOptions::LINEAR);
                c.texture
            }
            None => ctx.load_texture("template-cuts", image, egui::TextureOptions::LINEAR),
        };
        state.cut_preview = Some(CutPreview { key, texture });
    }
    if let Some(c) = &state.cut_preview {
        let corners =
            [[0.0, 0.0], [page[0], 0.0], page, [0.0, page[1]]].map(|p| cam.to_screen(rect, p));
        paint_quad(painter, c.texture.id(), corners, 1.0);
    }
}

/// Draws the whole canvas: page, layers, cut preview, template lines and the selection
/// handles.
fn paint(
    ui: &egui::Ui,
    doc: &Document,
    elements: &[PlacedElement],
    selected: Option<u64>,
    state: &mut CanvasState,
    rect: Rect,
    distorting: bool,
) {
    let cam = state.camera;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(45));

    let page = Rect::from_min_max(
        cam.to_screen(rect, [0.0, 0.0]),
        cam.to_screen(rect, doc.page_mm()),
    );
    painter.rect_filled(
        page.translate(vec2(5.0, 5.0)),
        0.0,
        Color32::from_black_alpha(100),
    );
    let [r, g, b] = doc.background;
    painter.rect_filled(page, 0.0, Color32::from_rgb(r, g, b));

    let mut painting = Painting {
        painter: painter.clone(),
        ctx: ui.ctx().clone(),
        cam,
        rect,
        page: doc.page_mm(),
        cache: &mut state.group_previews,
        used: HashMap::new(),
    };
    painting.nodes(&doc.layers, 1.0);
    // Previews not drawn this frame are dropped with their textures.
    state.group_previews = painting.used;

    paint_cuts(ui.ctx(), &painter, doc.page_mm(), elements, state, rect);

    // Darken whatever falls outside the page: it will not be printed.
    let dim = Color32::from_black_alpha(160);
    for r in [
        Rect::from_min_max(rect.min, pos2(rect.max.x, page.min.y)),
        Rect::from_min_max(pos2(rect.min.x, page.max.y), rect.max),
        Rect::from_min_max(pos2(rect.min.x, page.min.y), pos2(page.min.x, page.max.y)),
        Rect::from_min_max(pos2(page.max.x, page.min.y), pos2(rect.max.x, page.max.y)),
    ] {
        if r.is_positive() {
            painter.rect_filled(r, 0.0, dim);
        }
    }
    painter.rect_stroke(
        page,
        0.0,
        Stroke::new(1.0, Color32::from_gray(110)),
        StrokeKind::Outside,
    );

    for e in elements.iter().filter(|_| state.show_template) {
        let k = e.kind.index();
        if !state.element_show[k] {
            continue;
        }
        let stroke = Stroke::new(1.2, state.element_color[k]);
        for poly in &e.polylines {
            let pts: Vec<Pos2> = poly
                .points
                .iter()
                .map(|&p| cam.to_screen(rect, p))
                .collect();
            painter.add(if poly.closed {
                Shape::closed_line(pts, stroke)
            } else {
                Shape::line(pts, stroke)
            });
        }
    }

    let Some(sel) = selected.filter(|&id| doc.effectively_visible(id)) else {
        return;
    };
    let locked = doc.effectively_locked(sel);
    let color = if locked { Color32::GRAY } else { ACCENT };
    if let Some(Node::Group(_)) = doc.node(sel) {
        // Group: outline its visible layers and a dashed box around them all.
        let path = doc.path_of(sel).unwrap_or_default();
        let mut pts = Vec::new();
        for leaf in doc.leaves() {
            if leaf.visible && leaf.path.starts_with(&path) {
                let c = leaf.layer.corners().map(|p| cam.to_screen(rect, p));
                painter.add(Shape::closed_line(
                    c.to_vec(),
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                ));
                pts.extend(c);
            }
        }
        if !pts.is_empty() {
            let b = Rect::from_points(&pts).expand(4.0);
            let line = [
                b.left_top(),
                b.right_top(),
                b.right_bottom(),
                b.left_bottom(),
                b.left_top(),
            ];
            painter.extend(Shape::dashed_line(&line, Stroke::new(1.5, color), 6.0, 4.0));
        }
    } else if let Some(layer) = doc.layer(sel) {
        let (corners, top_mid, rot) = handles(&cam, rect, layer);
        let stroke = Stroke::new(1.5, color);
        painter.add(Shape::closed_line(corners.to_vec(), stroke));
        if !locked {
            painter.line_segment([top_mid, rot], stroke);
            for c in corners {
                if distorting {
                    // Diamonds: each corner moves on its own.
                    let r = HANDLE_RADIUS * 1.3;
                    painter.add(Shape::convex_polygon(
                        vec![
                            c - vec2(0.0, r),
                            c + vec2(r, 0.0),
                            c + vec2(0.0, r),
                            c - vec2(r, 0.0),
                        ],
                        Color32::WHITE,
                        stroke,
                    ));
                } else {
                    painter.rect(
                        Rect::from_center_size(c, Vec2::splat(HANDLE_RADIUS * 2.0)),
                        1.0,
                        Color32::WHITE,
                        stroke,
                        StrokeKind::Middle,
                    );
                }
            }
            // Edge middles: smaller bars along the edge.
            for (i, c) in edge_handles(&cam, rect, layer).into_iter().enumerate() {
                let along = (corners[(i + 1) % 4] - corners[i]).normalized();
                let along = if along.is_finite() { along } else { Vec2::X };
                let across = along.rot90();
                let (a, b) = (along * HANDLE_RADIUS * 0.9, across * HANDLE_RADIUS * 0.55);
                painter.add(Shape::convex_polygon(
                    vec![c - a - b, c + a - b, c + a + b, c - a + b],
                    Color32::WHITE,
                    stroke,
                ));
            }
            painter.circle(rot, HANDLE_RADIUS, Color32::WHITE, stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_a_member_of_the_selected_group_moves_the_group() {
        use crate::document::Group;
        let square = |id: u64, x: f32| {
            let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
            l.size_px = [10, 10];
            l.transform.center = [x, 50.0];
            Node::Layer(l)
        };
        let mut doc = Document {
            layers: vec![
                Node::Group(Group {
                    id: 1,
                    name: "g".into(),
                    visible: true,
                    locked: false,
                    opacity: 0.5,
                    expanded: true,
                    children: vec![square(2, 50.0), square(3, 100.0)],
                }),
                square(4, 200.0),
            ],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut selected = Some(1);
        let mut time = 0.0;
        let mut frame = |doc: &mut Document,
                         sel: &mut Option<u64>,
                         state: &mut CanvasState,
                         events: Vec<Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], sel, state);
            });
            out.textures_delta.clear();
        };
        frame(&mut doc, &mut selected, &mut state, vec![]);
        let cam = state.camera;
        let rect = state.rect;
        let from = cam.to_screen(rect, [50.0, 50.0]);
        let to = cam.to_screen(rect, [60.0, 55.0]);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut doc,
            &mut selected,
            &mut state,
            vec![Event::PointerMoved(from)],
        );
        frame(
            &mut doc,
            &mut selected,
            &mut state,
            vec![button(from, true)],
        );
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            frame(
                &mut doc,
                &mut selected,
                &mut state,
                vec![Event::PointerMoved(p)],
            );
        }
        frame(&mut doc, &mut selected, &mut state, vec![button(to, false)]);
        assert_eq!(selected, Some(1), "group stays selected");
        let c = |id| doc.layer(id).unwrap().transform.center;
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01;
        assert!(close(c(2), [60.0, 55.0]), "{:?}", c(2));
        assert!(close(c(3), [110.0, 55.0]), "{:?}", c(3));
        assert!(close(c(4), [200.0, 50.0]), "outside the group stays put");
    }

    #[test]
    fn distort_mode_drags_one_corner() {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [100.0, 100.0];
        l.transform.rotation_deg = 30.0;
        let mut doc = Document {
            layers: vec![Node::Layer(l)],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut state = CanvasState {
            distort_mode: true,
            ..Default::default()
        };
        let mut selected = Some(1);
        let mut time = 0.0;
        let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], &mut selected, state);
            });
            out.textures_delta.clear();
        };
        frame(&mut doc, &mut state, vec![]);
        let before = doc.layer(1).unwrap().corners();
        let (cam, rect) = (state.camera, state.rect);
        let target = [before[1][0] - 10.0, before[1][1] + 8.0];
        let from = cam.to_screen(rect, before[1]);
        let to = cam.to_screen(rect, target);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut doc, &mut state, vec![Event::PointerMoved(from)]);
        frame(&mut doc, &mut state, vec![button(from, true)]);
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            frame(&mut doc, &mut state, vec![Event::PointerMoved(p)]);
        }
        frame(&mut doc, &mut state, vec![button(to, false)]);

        let after = doc.layer(1).unwrap().corners();
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        assert!(close(after[1], target), "{:?} vs {:?}", after[1], target);
        for i in [0, 2, 3] {
            assert!(close(after[i], before[i]), "corner {i} stays put");
        }
        assert_eq!(doc.layer(1).unwrap().transform.scale, [1.0, 1.0]);
    }

    /// Drags on the canvas from one page point to another, with the layer selected.
    fn canvas_drag(doc: &mut Document, from: P2, to: P2, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut selected = Some(1);
        let mut time = 0.0;
        let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], &mut selected, state);
            });
            out.textures_delta.clear();
        };
        frame(doc, &mut state, vec![Event::ModifiersChanged(modifiers)]);
        let (cam, rect) = (state.camera, state.rect);
        let (from, to) = (cam.to_screen(rect, from), cam.to_screen(rect, to));
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(doc, &mut state, vec![Event::PointerMoved(from)]);
        frame(doc, &mut state, vec![button(from, true)]);
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            frame(doc, &mut state, vec![Event::PointerMoved(p)]);
        }
        frame(doc, &mut state, vec![button(to, false)]);
    }

    #[test]
    fn edge_handles_stretch_one_axis() {
        let layer = |rotation: f32| {
            let mut l = Layer::new(1, "l".into(), "x.png".into());
            l.size_px = [100, 50];
            l.transform.center = [100.0, 100.0];
            l.transform.rotation_deg = rotation;
            Document {
                layers: vec![Node::Layer(l)],
                ..Default::default()
            }
        };
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        let none = egui::Modifiers::NONE;

        // Bottom edge up by 20 mm: height 50 -> 30, top edge stays.
        let mut doc = layer(0.0);
        canvas_drag(&mut doc, [100.0, 125.0], [103.0, 105.0], none);
        let t = doc.layer(1).unwrap().transform;
        assert!(close(t.scale, [1.0, 0.6]), "{:?}", t.scale);
        assert!(close(t.center, [100.0, 90.0]), "{:?}", t.center);

        // Top edge: its handle sits under the rotation handle's stem, and still stretches.
        let mut doc = layer(0.0);
        canvas_drag(&mut doc, [100.0, 75.0], [100.0, 85.0], none);
        let t = doc.layer(1).unwrap().transform;
        assert!(close(t.scale, [1.0, 0.8]) && t.rotation_deg == 0.0, "{t:?}");

        // Rotated a quarter turn, the image's right edge points down the page.
        let mut doc = layer(90.0);
        canvas_drag(&mut doc, [100.0, 150.0], [100.0, 170.0], none);
        let t = doc.layer(1).unwrap().transform;
        assert!(close(t.scale, [1.2, 1.0]), "{:?}", t.scale);
        assert!(close(t.center, [100.0, 110.0]), "{:?}", t.center);

        // Alt stretches both sides around the centre.
        let mut doc = layer(0.0);
        canvas_drag(
            &mut doc,
            [150.0, 100.0],
            [160.0, 100.0],
            egui::Modifiers::ALT,
        );
        let t = doc.layer(1).unwrap().transform;
        assert!(
            close(t.scale, [1.2, 1.0]) && close(t.center, [100.0, 100.0]),
            "{t:?}"
        );

        // Squashing stops at a minimum instead of flipping the image.
        let mut doc = layer(0.0);
        canvas_drag(&mut doc, [100.0, 125.0], [100.0, 40.0], none);
        let t = doc.layer(1).unwrap().transform;
        assert!((t.scale[1] * 50.0 - MIN_STRETCH_MM).abs() < 1e-4, "{t:?}");
    }

    #[test]
    fn wheel_over_a_window_does_not_zoom_the_canvas() {
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut doc = Document::default();
        let mut selected = None;
        let mut frame = |state: &mut CanvasState, time: f64, events: Vec<Event>, window: bool| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, &mut doc, &[], &mut selected, state);
                if window {
                    egui::Window::new("over")
                        .fixed_pos(pos2(100.0, 100.0))
                        .fixed_size(vec2(300.0, 300.0))
                        .show(ui.ctx(), |ui| ui.allocate_space(vec2(300.0, 300.0)));
                }
            });
            out.textures_delta.clear();
        };
        let wheel = |p| {
            vec![
                Event::PointerMoved(p),
                Event::MouseWheel {
                    unit: MouseWheelUnit::Line,
                    delta: vec2(0.0, 1.0),
                    modifiers: Default::default(),
                    phase: egui::TouchPhase::Move,
                    source: MouseWheelSource::Wheel,
                },
            ]
        };
        // Windows need a few frames to size and show themselves.
        for k in 0..4 {
            let hover = vec![Event::PointerMoved(pos2(200.0, 200.0))];
            frame(&mut state, k as f64 * 0.02, hover, true);
        }
        let zoom = state.camera.zoom;
        frame(&mut state, 0.2, wheel(pos2(200.0, 200.0)), true);
        assert_eq!(state.camera.zoom, zoom, "window covers the canvas");
        frame(&mut state, 0.3, wheel(pos2(800.0, 600.0)), true);
        assert!(state.camera.zoom > zoom, "bare canvas still zooms");
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        let mut cam = Camera {
            zoom: 2.0,
            pan: vec2(30.0, 40.0),
        };
        let anchor = pos2(400.0, 300.0);
        let before = cam.to_mm(rect, anchor);
        cam.zoom_at(rect, anchor, 1.7);
        let after = cam.to_mm(rect, anchor);
        assert!((before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3);
    }

    #[test]
    fn fit_centres_page() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
        let mut cam = Camera {
            zoom: 1.0,
            pan: Vec2::ZERO,
        };
        cam.fit(rect, [297.0, 210.0]);
        let c = cam.to_screen(rect, [148.5, 105.0]);
        assert!((c.x - 500.0).abs() < 1e-2 && (c.y - 250.0).abs() < 1e-2);
    }
}
