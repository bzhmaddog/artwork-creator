//! The artwork canvas: pan/zoom camera, page, layers, template elements and selection gizmo.

use egui::epaint::Vertex;
use egui::{
    Color32, CursorIcon, Event, Key, Mesh, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2,
    Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::canvas_points;
pub use crate::canvas_points::{Drawing, PointEdit};
use crate::canvas_text;
use crate::document::{
    Affine, Document, Homography, Layer, Node, P2, Transform, UNIT_CORNERS, is_convex,
};
use crate::layers_ui;
use crate::render::{self, SceneNode};
use crate::render::{Cuts, PlacedElement};
use crate::template::{ElementKind, KINDS};

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
/// Color of the selection and of the anchor an image snapped to: crimson, apart from
/// the template elements' default colors (magenta, blue, orange, green).
const ACCENT: Color32 = Color32::from_rgb(220, 20, 60);
/// Color of the safe-area guide, apart from the template element colors.
const SAFE_AREA_COLOR: Color32 = Color32::from_rgb(130, 80, 220);
/// Longest side of the offscreen render of a semi-transparent group.
const MAX_GROUP_PREVIEW_SIDE: f32 = 2048.0;
/// How close (screen points) an image's center or corner must come to an anchor to snap.
const SNAP_RADIUS: f32 = 10.0;
/// Shortest straight side of the outline that layers snap to, in mm.
const MIN_GUIDE_SIDE_MM: f32 = 2.0;
/// Half the size of the cross marking an anchor, in screen points.
const ANCHOR_MARK: f32 = 5.0;

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

    /// Zooms and centers so the whole page fits, with a margin.
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

/// What the template cuts away, rendered over the page: paper covering it, and
/// hatching over that.
struct CutPreview {
    key: u64,
    texture: egui::TextureHandle,
    hatch: egui::TextureHandle,
}

/// The cut preview's hatching, in pixels of its texture: from one line to the next
/// along the diagonal, and a line's width. The texture follows the zoom in powers of
/// two, so on screen they are between one and two thirds of that.
const HATCH_PERIOD: f32 = 14.0;
const HATCH_WIDTH: f32 = 2.5;
/// The hatching's color, straight RGBA: shows over paper and over artwork alike.
const HATCH_COLOR: [u8; 4] = [120, 120, 120, 200];

/// Canvas state that lasts across frames: view, drag, display options and caches.
pub struct CanvasState {
    /// Current view.
    pub camera: Camera,
    /// Re-fit the page on the next frame.
    pub fit_pending: bool,
    /// Draw the template's lines at all.
    pub show_template: bool,
    /// Per element kind (`ElementKind::index`): lines shown, and their color.
    pub element_show: [bool; KINDS],
    /// Line color per element kind.
    pub element_color: [Color32; KINDS],
    /// Cover and hatch what is cut out of the panel: outside the outline, inside the
    /// holes.
    pub preview_cuts: bool,
    /// What is cut out of the panel, whatever the export prints there; set by the app.
    pub cuts: Cuts,
    /// The cut preview shows the bleed: the holes shrunk and the outline grown by it.
    /// Off, it follows the cut lines themselves. The export always uses the bleed.
    pub preview_bleed: bool,
    /// Show the safe area's dashed guide, when the project has one.
    pub preview_safe_area: bool,
    /// Opacity of the cut preview: below 1, what will be cut stays faintly visible.
    pub cut_opacity: f32,
    cut_preview: Option<CutPreview>,
    /// The safe area's width inside the cut lines, in mm; 0 shows no guide.
    pub safe_margin_mm: f32,
    /// The safe-area guide's rings in page mm, and the key they were made for.
    safe_area: Option<(u64, Vec<Vec<P2>>)>,
    /// Mouse position on the page, for the status bar.
    pub cursor_mm: Option<P2>,
    /// Corner handles distort the selected layer instead of scaling it. Holding Ctrl
    /// swaps the two.
    pub distort_mode: bool,
    /// Image centers, corners and edge middles snap to the template's anchors while
    /// dragged. Holding Alt
    /// turns it off for the drag.
    pub snap_to_anchors: bool,
    /// The anchor the dragged image snapped to this frame, in page mm, to highlight it.
    snapped: Option<P2>,
    /// Edges and centers of layers snap to the page's and the template outline's edges
    /// while moved or resized. Holding Alt turns it off for the drag.
    pub snap_to_edges: bool,
    /// The guides the dragged layer snapped to this frame: a vertical line's x and a
    /// horizontal line's y, in page mm, to show them.
    snapped_guides: [Option<f32>; 2],
    /// Where the canvas is on screen.
    pub(crate) rect: Rect,
    drag: Option<Drag>,
    /// A left drag that started on empty canvas pans the view.
    panning: bool,
    /// Typing in a text layer on the canvas, after a double click on it.
    pub typing: Option<canvas_text::Typing>,
    /// Editing the points of a line, polyline or polygon, after a double click on it.
    pub points: Option<PointEdit>,
    /// A line, polyline or polygon being drawn point by point, after picking it in the
    /// Shape menu.
    pub drawing: Option<Drawing>,
    /// The paint tool picked last and its settings; painting in the selected closed
    /// shape replaces selecting and moving. The tool is kept while no such shape is
    /// selected, and in hand again once one is (see [`active_tool`]).
    pub brushes: crate::paint::Brushes,
    /// Each command's shortcut as text, for the tool bar's tooltips; set by the app.
    pub keys: HashMap<crate::shortcuts::Command, String>,
    /// The stroke being painted: x, y on the page (mm) and the pen's pressure.
    painting: Vec<[f32; 3]>,
    /// The pen's pressure during the stroke, when the system reports one.
    pen_pressure: Option<f32>,
    /// The stroke being painted, drawn as it will be.
    paint_preview: Option<egui::TextureHandle>,
    group_previews: HashMap<u64, GroupPreview>,
    /// The page background's gradient drawn small, with what it was drawn from.
    background: Option<(u64, egui::TextureHandle)>,
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
            element_show: [true; KINDS],
            element_color: ElementKind::ALL.map(|k| {
                let [r, g, b] = k.default_color();
                Color32::from_rgb(r, g, b)
            }),
            preview_cuts: true,
            cuts: Cuts::default(),
            preview_bleed: true,
            preview_safe_area: true,
            cut_opacity: 0.8,
            cut_preview: None,
            safe_margin_mm: 0.0,
            safe_area: None,
            cursor_mm: None,
            distort_mode: false,
            snap_to_anchors: true,
            snapped: None,
            snap_to_edges: true,
            snapped_guides: [None; 2],
            rect: Rect::NOTHING,
            drag: None,
            panning: false,
            typing: None,
            points: None,
            drawing: None,
            group_previews: HashMap::new(),
            background: None,
            brushes: Default::default(),
            keys: HashMap::new(),
            painting: Vec::new(),
            paint_preview: None,
            pen_pressure: None,
        }
    }
}

/// The Page panel's display settings, kept with the app's preferences (not the
/// project): how the canvas previews the cuts, the bleed and the safe area.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Previews {
    cuts: bool,
    cut_opacity: f32,
    bleed: bool,
    safe_area: bool,
    safe_margin_mm: f32,
}

impl Default for Previews {
    fn default() -> Self {
        let s = CanvasState::default();
        Self {
            cuts: s.preview_cuts,
            cut_opacity: s.cut_opacity,
            bleed: s.preview_bleed,
            safe_area: s.preview_safe_area,
            safe_margin_mm: s.safe_margin_mm,
        }
    }
}

/// Where they are kept.
const PREVIEWS_KEY: &str = "canvas_previews";

impl CanvasState {
    /// The canvas with the display settings saved by a previous session.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let p: Previews = storage
            .and_then(|s| eframe::get_value(s, PREVIEWS_KEY))
            .unwrap_or_default();
        Self {
            preview_cuts: p.cuts,
            cut_opacity: p.cut_opacity.clamp(0.0, 1.0),
            preview_bleed: p.bleed,
            preview_safe_area: p.safe_area,
            safe_margin_mm: p.safe_margin_mm.clamp(0.0, crate::print::MAX_MARGIN_MM),
            ..Default::default()
        }
    }

    /// Keeps its display settings for the next session.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let p = Previews {
            cuts: self.preview_cuts,
            cut_opacity: self.cut_opacity,
            bleed: self.preview_bleed,
            safe_area: self.preview_safe_area,
            safe_margin_mm: self.safe_margin_mm,
        };
        eframe::set_value(storage, PREVIEWS_KEY, &p);
    }

    /// `p` (page mm) if it is well inside the visible part of the canvas, otherwise the
    /// middle of the view: where to put something new so that it can be seen.
    pub fn in_view(&self, p: P2) -> P2 {
        if !self.rect.is_positive() {
            return p;
        }
        let inner = self.rect.shrink2(self.rect.size() * 0.1);
        if inner.contains(self.camera.to_screen(self.rect, p)) {
            p
        } else {
            self.camera.to_mm(self.rect, self.rect.center())
        }
    }

    /// Starts typing in a text layer on the canvas, all of its text selected.
    pub fn type_in(&mut self, layer: &Layer) {
        let n = layer.text.as_ref().map_or(0, |t| t.content.chars().count());
        canvas_text::begin(&mut self.typing, layer.id, 0, n);
    }

    /// Zooms around the center of the canvas (toolbar buttons).
    pub fn zoom_centered(&mut self, factor: f32) {
        if self.rect.is_positive() {
            self.camera.zoom_at(self.rect, self.rect.center(), factor);
        }
    }

    /// Sets an absolute zoom around the center of the canvas.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom_centered(zoom / self.camera.zoom);
    }
}

/// Points of a layer that snap to anchors: its center, its drawn corners, then the
/// middles of its drawn edges. The
/// center is the middle of the drawn corners' bounds, as for Align: once the layer is
/// distorted, that is where it looks centered, not the center of its undistorted frame.
/// A shape's are those of its geometry, its stroke left out (see `shape::snap_points`).
fn snap_points(layer: &Layer) -> Vec<P2> {
    if layer.shape.is_some() {
        return crate::shape::snap_points(layer);
    }
    let c = layer.corners();
    let (mut lo, mut hi) = (c[0], c[0]);
    for p in &c[1..] {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let middle = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let mut points = vec![middle, c[0], c[1], c[2], c[3]];
    points.extend((0..4).map(|i| {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
    }));
    points
}

/// The anchor nearest to any of `points`, if within `reach` (mm): the offset that moves
/// that point onto it, and the anchor.
pub(crate) fn nearest_snap(anchors: &[P2], points: &[P2], reach: f32) -> Option<(P2, P2)> {
    let mut best: Option<(f32, P2, P2)> = None;
    for &p in points {
        for &a in anchors {
            let d = [a[0] - p[0], a[1] - p[1]];
            let dist = len(d);
            if dist <= reach && best.is_none_or(|(b, _, _)| dist < b) {
                best = Some((dist, d, a));
            }
        }
    }
    best.map(|(_, d, a)| (d, a))
}

/// Lines that the edges and middles of layers snap to, in page mm: vertical lines by
/// their x, horizontal lines by their y.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Guides {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

/// The guides layers snap to: the edges and middle of a `page` mm page, and those of
/// the outline's bounds, its straight vertical and horizontal sides, and the bleed line
/// `bleed` mm outside its bounds.
pub(crate) fn guides(elements: &[PlacedElement], bleed: f32, page: P2) -> Guides {
    let mut g = Guides {
        x: vec![0.0, page[0] / 2.0, page[0]],
        y: vec![0.0, page[1] / 2.0, page[1]],
    };
    let lines = elements
        .iter()
        .filter(|e| e.kind == ElementKind::Outline)
        .flat_map(|e| &e.polylines);
    let mut bounds: Option<(P2, P2)> = None;
    for line in lines.clone() {
        for p in &line.points {
            bounds = Some(match bounds {
                None => (*p, *p),
                Some((lo, hi)) => (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                ),
            });
        }
    }
    let Some((lo, hi)) = bounds else {
        return g.sorted();
    };
    for a in 0..2 {
        let axis = if a == 0 { &mut g.x } else { &mut g.y };
        axis.extend([lo[a], (lo[a] + hi[a]) / 2.0, hi[a]]);
        if bleed > 0.0 {
            axis.extend([lo[a] - bleed, hi[a] + bleed]);
        }
    }
    for line in lines {
        let n = line.points.len();
        let segments = if line.closed { n } else { n.saturating_sub(1) };
        for i in 0..segments {
            let (p, q) = (line.points[i], line.points[(i + 1) % n]);
            let (dx, dy) = ((q[0] - p[0]).abs(), (q[1] - p[1]).abs());
            if dx < 1e-3 && dy >= MIN_GUIDE_SIDE_MM {
                g.x.push(p[0]);
            } else if dy < 1e-3 && dx >= MIN_GUIDE_SIDE_MM {
                g.y.push(p[1]);
            }
        }
    }
    g.sorted()
}

impl Guides {
    /// Sorted, each line once.
    fn sorted(mut self) -> Self {
        for axis in [&mut self.x, &mut self.y] {
            axis.sort_by(f32::total_cmp);
            axis.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        }
        self
    }
}

/// The shift that brings the nearest of `values` onto one of `guides`, if within `reach`
/// (mm), and that guide.
fn snap_axis(guides: &[f32], values: &[f32], reach: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for &v in values {
        for &g in guides {
            let d = g - v;
            if d.abs() <= reach && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                best = Some((d, g));
            }
        }
    }
    best
}

/// Drawn corners of an image of `size_px` placed by `t`.
fn corners_of(t: Transform, size_px: [u32; 2]) -> [P2; 4] {
    let m = t.to_affine(size_px);
    let (w, h) = (size_px[0] as f32, size_px[1] as f32);
    t.unit_quad().map(|q| m.apply([q[0] * w, q[1] * h]))
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

/// The layer a click at a page point selects: the topmost unlocked one, or else the
/// topmost locked one. A locked layer can be selected (for the layer toolbar and
/// Properties) but does not hide the layers under it from clicks.
fn layer_to_select(doc: &Document, p: P2) -> Option<u64> {
    topmost_hit(doc, p).or_else(|| {
        doc.leaves()
            .into_iter()
            .rev()
            .find(|l| l.visible && l.layer.contains(p))
            .map(|l| l.layer.id)
    })
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
    // The cursor as the panels and windows drawn so far left it: the canvas' own only
    // show over the canvas itself (see the end).
    let cursor_before = ui.ctx().output(|o| o.cursor_icon);

    // Keep the view centered when the window resizes, and fit on demand.
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

    // Panning: left drag on empty canvas, middle or right drag, or Space + left drag
    // (except while typing, where Space is a space).
    let space = state.typing.is_none() && ui.input(|i| i.key_down(Key::Space));
    if !ui.input(|i| i.pointer.primary_down()) {
        state.panning = false;
    }
    let panning_now = response.dragged_by(PointerButton::Middle)
        || response.dragged_by(PointerButton::Secondary)
        || (response.dragged_by(PointerButton::Primary) && (space || state.panning));
    if panning_now {
        state.camera.pan += response.drag_delta();
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if space && hover.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }

    let cam = state.camera;
    let shift = ui.input(|i| i.modifiers.shift);
    // Ctrl+Alt on a resize handle: the paint in a shape does the other way than its
    // Stretch paint says, for that drag. Neither key then does what it does alone.
    let flip_paint = ui.input(|i| i.modifiers.command && i.modifiers.alt);
    let alt = ui.input(|i| i.modifiers.alt) && !flip_paint;
    // Ctrl swaps the corner handles between resizing and distorting while held.
    let distorting = state.distort_mode != (ui.input(|i| i.modifiers.command) && !flip_paint);
    // Snapping reach in mm, when on and the template has anchors.
    let anchors: Vec<P2> = elements.iter().flat_map(|e| &e.anchors).copied().collect();
    let snap_reach =
        (state.snap_to_anchors && !alt && !anchors.is_empty()).then_some(SNAP_RADIUS / cam.zoom);
    let mut snapped = None;
    // The page's and the outline's guides, and their reach in mm when snapping to them
    // is on.
    let guides = guides(elements, state.cuts.bleed_mm, doc.page_mm());
    let guide_reach = (state.snap_to_edges && !alt).then_some(SNAP_RADIUS / cam.zoom);
    let mut snapped_guides = [None; 2];

    // Which handle (if any) of the selected layer is under a screen point. A shape whose
    // points are being edited shows those instead.
    let editing_points = state.points.as_ref().map(|e| e.layer);
    let handle_at = |doc: &Document, sel: Option<u64>, p: Pos2| -> Option<(u64, DragKind)> {
        let id = sel?;
        let layer = doc.layer(id)?;
        if doc.effectively_locked(id) || !doc.effectively_visible(id) || editing_points == Some(id)
        {
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

    // A paint tool takes the left button: it paints or erases in the selected closed
    // shape instead of selecting. Esc goes back to selecting. Without one to paint in,
    // the tool waits for the next one.
    if active_tool(doc, *selected, &state.brushes).is_some()
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(Key::Escape))
    {
        state.brushes.tool = None;
    }
    let active = active_tool(doc, *selected, &state.brushes);
    if active.is_none() {
        state.painting.clear();
    }
    let paint_tool = active.filter(|_| !space);
    if let Some(tool) = paint_tool {
        state.typing = None;
        state.drawing = None;
        state.points = None;
        changed |= paint_input(ui, doc, *selected, state, cam, rect, &response, tool);
    }

    // Typing on the canvas takes the clicks and drags on its text.
    let on_handle = hover.is_some_and(|p| handle_at(doc, *selected, p).is_some());
    let typing = canvas_text::input(
        ui,
        doc,
        *selected,
        &mut state.typing,
        cam,
        rect,
        &response,
        on_handle,
    );

    // Drawing a shape takes the left clicks; its points are edited once it is done.
    let keys = canvas_points::Keys {
        shift,
        alt,
        anchors: &anchors,
        snap_reach,
    };
    let drawn = canvas_points::draw_input(
        ui,
        doc,
        selected,
        &mut state.drawing,
        cam,
        rect,
        &response,
        keys,
    );
    if state.drawing.is_some() {
        state.points = None;
    }
    let mut points = canvas_points::input(
        ui,
        doc,
        *selected,
        &mut state.points,
        cam,
        rect,
        &response,
        keys,
    );
    points.took |= drawn.took;
    points.changed |= drawn.changed;
    points.snapped = points.snapped.or(drawn.snapped);
    changed |= points.changed;
    snapped = snapped.or(points.snapped);

    if !space && !typing.took && !points.took && paint_tool.is_none() {
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
                        // A locked layer is selected but stays put: the drag pans.
                        (hit, _) => {
                            *selected = layer_to_select(doc, m);
                            (hit.into_iter().collect(), DragKind::Move)
                        }
                    },
                };
                // A text following a shape moves with it: the shape is moved.
                let mut ids = ids;
                if kind == DragKind::Move {
                    let shapes: Vec<u64> = ids
                        .iter()
                        .filter_map(|&id| doc.layer(id).and_then(crate::text::followed_shape))
                        .filter(|&s| !doc.effectively_locked(s))
                        .collect();
                    for s in shapes {
                        if !ids.contains(&s) {
                            ids.push(s);
                        }
                    }
                }
                let layers: Vec<(u64, Transform)> = ids
                    .into_iter()
                    .filter_map(|id| Some((id, doc.layer(id)?.transform)))
                    .collect();
                if flip_paint && matches!(kind, DragKind::Scale | DragKind::Stretch(_)) {
                    for (id, _) in &layers {
                        if let Some(l) = doc.layer_mut(*id) {
                            crate::shape::flip_paint_once(l);
                        }
                    }
                }
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
            *selected = layer_to_select(doc, cam.to_mm(rect, p));
        }
        // A double click on a text layer starts typing in it, where it was clicked. Text
        // on a path is typed in Properties.
        if response.double_clicked_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
            && let Some(layer) = selected.and_then(|id| doc.layer(id))
            && crate::text::is_straight(layer)
            && !doc.effectively_locked(layer.id)
        {
            let i = canvas_text::index_at(layer, cam.to_mm(rect, p));
            canvas_text::begin(&mut state.typing, layer.id, i, i);
        }
        // On a line, polyline or polygon, it shows its points to edit them.
        if response.double_clicked_by(PointerButton::Primary)
            && let Some(id) = *selected
            && state.points.is_none()
            && canvas_points::can_edit(doc, id)
        {
            state.points = Some(PointEdit::new(id));
        }
    }
    changed |= canvas_text::field(ui, doc, &mut state.typing, cam, rect);

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
            // The center or corner nearest an anchor lands on it; everything moves along.
            if let Some(reach) = snap_reach {
                let points: Vec<P2> = drag
                    .layers
                    .iter()
                    .filter_map(|(id, _)| doc.layer(*id))
                    .flat_map(snap_points)
                    .collect();
                if let Some((off, a)) = nearest_snap(&anchors, &points, reach) {
                    for (id, _) in &drag.layers {
                        if let Some(layer) = doc.layer_mut(*id) {
                            let c = layer.transform.center;
                            layer.transform.center = [c[0] + off[0], c[1] + off[1]];
                        }
                    }
                    snapped = Some(a);
                }
            }
            // Otherwise an edge or the middle of what is moved lands on a guide, on
            // each axis.
            let ids: Vec<u64> = drag.layers.iter().map(|(id, _)| *id).collect();
            if snapped.is_none()
                && let Some(reach) = guide_reach
                && let Some((lo, hi)) = layers_ui::layers_bounds(doc, &ids)
            {
                let mut off = [0.0; 2];
                for (a, axis) in [&guides.x, &guides.y].into_iter().enumerate() {
                    let values = [lo[a], (lo[a] + hi[a]) / 2.0, hi[a]];
                    if let Some((d, g)) = snap_axis(axis, &values, reach) {
                        off[a] = d;
                        snapped_guides[a] = Some(g);
                    }
                }
                for id in &ids {
                    if let Some(layer) = doc.layer_mut(*id) {
                        let c = layer.transform.center;
                        layer.transform.center = [c[0] + off[0], c[1] + off[1]];
                    }
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
                    // The opposite edge stays put; with Alt, both move (center fixed).
                    let (dir, extent) = edge_axis(s, layer.size_px, edge);
                    let axis = if edge % 2 == 0 { 1 } else { 0 };
                    let px = layer.size_px[axis].max(1) as f32;
                    if extent > 0.0 {
                        let d = [m[0] - drag.start_mouse[0], m[1] - drag.start_mouse[1]];
                        let pull = d[0] * dir[0] + d[1] * dir[1];
                        let both = if alt { 2.0 } else { 1.0 };
                        let mut new = (extent + pull * both).max(MIN_STRETCH_MM);
                        // An edge parallel to a guide lands on it. The edge stands
                        // `new / 2` from the center with Alt, `new - extent / 2` from
                        // the starting center without.
                        let a = (0..2).find(|&a| dir[a].abs() > 0.999);
                        if let Some(a) = a.filter(|_| !s.is_distorted())
                            && let Some(reach) = guide_reach
                        {
                            let axis = if a == 0 { &guides.x } else { &guides.y };
                            let from_center =
                                |new: f32| if alt { new / 2.0 } else { new - extent / 2.0 };
                            let edge = c[a] + dir[a] * from_center(new);
                            if let Some((_, g)) = snap_axis(axis, &[edge], reach) {
                                let e = dir[a] * (g - c[a]);
                                let snapped_new = if alt { 2.0 * e } else { e + extent / 2.0 };
                                if snapped_new >= MIN_STRETCH_MM {
                                    new = snapped_new;
                                    snapped_guides[a] = Some(g);
                                }
                            }
                        }
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
                    // The dragged corner snaps: scaled so it comes as close as it can to
                    // the anchor along its diagonal, the center staying put.
                    let start = corners_of(s, layer.size_px);
                    let i = (0..4)
                        .min_by(|&a, &b| {
                            let d = |k: usize| {
                                len([
                                    start[k][0] - drag.start_mouse[0],
                                    start[k][1] - drag.start_mouse[1],
                                ])
                            };
                            d(a).total_cmp(&d(b))
                        })
                        .unwrap_or(0);
                    if let Some(reach) = snap_reach {
                        let p = corners_of(*t, layer.size_px)[i];
                        if let Some((_, a)) = nearest_snap(&anchors, &[p], reach) {
                            let u = [p[0] - c[0], p[1] - c[1]];
                            let w = [a[0] - c[0], a[1] - c[1]];
                            let uu = u[0] * u[0] + u[1] * u[1];
                            let k = if uu > 1e-6 {
                                (w[0] * u[0] + w[1] * u[1]) / uu
                            } else {
                                0.0
                            };
                            if k > 1e-3 {
                                t.scale = [t.scale[0] * k, t.scale[1] * k];
                                snapped = Some(a);
                            }
                        }
                    }
                    // Otherwise the corner's x or y lands on a guide, whichever is
                    // nearer, the center staying put.
                    if snapped.is_none()
                        && let Some(reach) = guide_reach
                    {
                        let p = corners_of(*t, layer.size_px)[i];
                        let mut best: Option<(f32, usize, f32)> = None;
                        for (a, axis) in [&guides.x, &guides.y].into_iter().enumerate() {
                            if (p[a] - c[a]).abs() > 1e-3
                                && let Some((d, g)) = snap_axis(axis, &[p[a]], reach)
                                && best.is_none_or(|(b, _, _)| d.abs() < b.abs())
                            {
                                best = Some((d, a, g));
                            }
                        }
                        if let Some((_, a, g)) = best {
                            let k = (g - c[a]) / (p[a] - c[a]);
                            if k > 1e-3 {
                                t.scale = [t.scale[0] * k, t.scale[1] * k];
                                snapped_guides[a] = Some(g);
                            }
                        }
                    }
                }
                DragKind::Distort(i) => {
                    // Corner offsets live in the undistorted image frame, which this drag
                    // does not change.
                    if let (Some(a), Some(b)) =
                        (layer.page_to_unit(drag.start_mouse), layer.page_to_unit(m))
                    {
                        let mut d = s.distort;
                        d[i] = [d[i][0] + b[0] - a[0], d[i][1] + b[1] - a[1]];
                        // The corner lands on an anchor within reach.
                        let mut t2 = s;
                        t2.distort = d;
                        if let Some(reach) = snap_reach {
                            let p = corners_of(t2, layer.size_px)[i];
                            if let Some((_, anchor)) = nearest_snap(&anchors, &[p], reach)
                                && let Some(u) = layer.page_to_unit(anchor)
                            {
                                d[i] = [u[0] - UNIT_CORNERS[i][0], u[1] - UNIT_CORNERS[i][1]];
                                snapped = Some(anchor);
                            }
                        }
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
            snapped = None;
            snapped_guides = [None; 2];
        }
    } else if let Some(p) = hover
        && !space
        && !panning_now
        && !typing.over_text
        && !points.took
        && !points.over_handle
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

    state.snapped = snapped;
    state.snapped_guides = snapped_guides;
    paint(ui, doc, elements, *selected, state, rect, distorting);
    if can_paint(doc, *selected) && tool_bar(ui, state, rect, has_paint(doc, *selected)) {
        changed |= clear_paint(doc, *selected);
    }
    canvas_text::paint(ui, doc, &state.typing, cam, rect);
    let painter = ui.painter_at(rect);
    let look = canvas_points::Look {
        accent: ACCENT,
        background: ui.visuals().extreme_bg_color,
    };
    canvas_points::paint(&painter, doc, &state.points, cam, rect, &look);
    canvas_points::paint_drawing(&painter, doc, &state.drawing, cam, rect, hover, keys, &look);
    // Outside the canvas, or over its tool bar, a window or a popup, even during a drag
    // that started on it: the cursor of what is there.
    if !ui.rect_contains_pointer(rect) {
        ui.ctx().output_mut(|o| o.cursor_icon = cursor_before);
    }
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
    // The texture may reach past the image, where effects such as shadows draw.
    let m: Homography = l.homography().then_affine(cam.affine(rect));
    let [x0, y0, x1, y1] = l.shown_rect();
    let at = |u: f32, v: f32| {
        let p = m.apply([x0 + u * (x1 - x0), y0 + v * (y1 - y0)]);
        pos2(p[0], p[1])
    };
    if !l.transform.is_distorted() {
        let corners = [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)];
        paint_quad(painter, tex, corners, opacity);
    } else {
        paint_grid(painter, tex, WARP_GRID, at, opacity);
    }
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
                Node::Layer(l) if l.visible => match l.shown_texture() {
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
                Node::Layer(l) if l.visible && l.shown_texture().is_none() => {
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

/// Covers what is cut out of the panel (outside the outline, inside the button holes and
/// cutouts) with paper, whether the export prints there or not; with `cut_opacity` below
/// 1, what will be cut shows through faintly.
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
    // The bleed only when asked for: the cut lines themselves otherwise.
    let cuts = Cuts {
        bleed_mm: if state.preview_bleed {
            state.cuts.bleed_mm
        } else {
            0.0
        },
        ..state.cuts
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cuts.hash(&mut h);
    px_per_mm.to_bits().hash(&mut h);
    hash_elements(elements, &mut h);
    let key = h.finish();
    if state.cut_preview.as_ref().is_none_or(|c| c.key != key) {
        let Some(keep) = crate::render::keep_mask(elements, cuts, [0.0, 0.0], size, px_per_mm)
        else {
            state.cut_preview = None;
            return;
        };
        // Premultiplied white, opaque where everything is cut.
        let rgba: Vec<u8> = keep
            .iter()
            .flat_map(|&k| {
                let a = 255 - k;
                [a, a, a, a]
            })
            .collect();
        let [w, h] = size.map(|v| v as usize);
        let image = egui::ColorImage::from_rgba_premultiplied([w, h], &rgba);
        // Diagonal lines, where it is cut, smoothed at their edges.
        let mut lines = vec![0u8; w * h * 4];
        for (i, (px, &k)) in lines
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&keep)
            .enumerate()
        {
            if k == 255 {
                continue;
            }
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let d = (x + y).rem_euclid(HATCH_PERIOD);
            let off = d.min(HATCH_PERIOD - d) - HATCH_WIDTH / 2.0;
            let line = (0.5 - off).clamp(0.0, 1.0) * (255 - k) as f32 / 255.0;
            // Premultiplied.
            let a = HATCH_COLOR[3] as f32 / 255.0 * line;
            let [r, g, b, _] = HATCH_COLOR.map(|v| (v as f32 * a).round() as u8);
            px.copy_from_slice(&[r, g, b, (a * 255.0).round() as u8]);
        }
        let hatch = egui::ColorImage::from_rgba_premultiplied([w, h], &lines);
        let options = egui::TextureOptions::LINEAR;
        let (texture, hatch) = match state.cut_preview.take() {
            Some(mut c) => {
                c.texture.set(image, options);
                c.hatch.set(hatch, options);
                (c.texture, c.hatch)
            }
            None => (
                ctx.load_texture("template-cuts", image, options),
                ctx.load_texture("template-cuts-hatch", hatch, options),
            ),
        };
        state.cut_preview = Some(CutPreview {
            key,
            texture,
            hatch,
        });
    }
    if let Some(c) = &state.cut_preview {
        let corners =
            [[0.0, 0.0], [page[0], 0.0], page, [0.0, page[1]]].map(|p| cam.to_screen(rect, p));
        paint_quad(painter, c.texture.id(), corners, state.cut_opacity);
        // The hatching shows what is cut away at any opacity.
        paint_quad(painter, c.hatch.id(), corners, 1.0);
    }
}

/// Feeds the template elements' shapes to `h`.
fn hash_elements(elements: &[PlacedElement], h: &mut impl Hasher) {
    for e in elements {
        e.kind.hash(h);
        for poly in &e.polylines {
            poly.closed.hash(h);
            for p in &poly.points {
                p[0].to_bits().hash(h);
                p[1].to_bits().hash(h);
            }
        }
    }
}

/// Dashed line along the safe area's edge: `safe_margin_mm` inside the outline, and as
/// far around every button and cutout. Only a guide: it is never printed.
fn paint_safe_area(
    painter: &egui::Painter,
    elements: &[PlacedElement],
    state: &mut CanvasState,
    rect: Rect,
) {
    let margin = state.safe_margin_mm;
    if margin <= 0.0 || elements.is_empty() {
        state.safe_area = None;
        return;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    margin.to_bits().hash(&mut h);
    hash_elements(elements, &mut h);
    let key = h.finish();
    if state.safe_area.as_ref().is_none_or(|(k, _)| *k != key) {
        let rings = |keep: &dyn Fn(ElementKind) -> bool| -> Vec<Vec<P2>> {
            elements
                .iter()
                .filter(|e| keep(e.kind))
                .flat_map(|e| &e.polylines)
                .filter(|p| p.closed && p.points.len() > 2)
                .map(|p| p.points.clone())
                .collect()
        };
        let mut guide = crate::print::offset(&rings(&|k| k == ElementKind::Outline), -margin);
        guide.extend(crate::print::offset(
            &rings(&|k| matches!(k, ElementKind::Buttons | ElementKind::Cutouts)),
            margin,
        ));
        state.safe_area = Some((key, guide));
    }
    let Some((_, guide)) = &state.safe_area else {
        return;
    };
    let cam = state.camera;
    let stroke = Stroke::new(1.0, SAFE_AREA_COLOR);
    for ring in guide {
        let mut pts: Vec<Pos2> = ring.iter().map(|&p| cam.to_screen(rect, p)).collect();
        if let Some(&first) = pts.first() {
            pts.push(first);
        }
        painter.extend(Shape::dashed_line(&pts, stroke, 6.0, 4.0));
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
    // Surroundings of the page, matched to the light or dark theme.
    let dark = ui.visuals().dark_mode;
    let (surround, shadow, dim, border) = if dark {
        (45, 100, 160, 110)
    } else {
        (205, 60, 90, 150)
    };
    painter.rect_filled(rect, 0.0, Color32::from_gray(surround));

    let page = Rect::from_min_max(
        cam.to_screen(rect, [0.0, 0.0]),
        cam.to_screen(rect, doc.page_mm()),
    );
    painter.rect_filled(
        page.translate(vec2(5.0, 5.0)),
        0.0,
        Color32::from_black_alpha(shadow),
    );
    let [r, g, b] = doc.background;
    painter.rect_filled(page, 0.0, Color32::from_rgb(r, g, b));
    match &doc.background_gradient {
        Some(gradient) => {
            let texture = background_texture(ui.ctx(), state, doc, gradient);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture, page, uv, Color32::WHITE);
        }
        None => state.background = None,
    }

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
    let dim = Color32::from_black_alpha(dim);
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
        Stroke::new(1.0, Color32::from_gray(border)),
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
        // Anchors: small crosses.
        for &a in &e.anchors {
            let p = cam.to_screen(rect, a);
            let (h, v) = (vec2(ANCHOR_MARK, 0.0), vec2(0.0, ANCHOR_MARK));
            painter.line_segment([p - h, p + h], stroke);
            painter.line_segment([p - v, p + v], stroke);
        }
    }
    if state.show_template && state.preview_safe_area {
        paint_safe_area(&painter, elements, state, rect);
    }
    // The anchor the dragged image snapped to, shown even when anchors are hidden.
    if let Some(a) = state.snapped {
        let p = cam.to_screen(rect, a);
        painter.circle_stroke(p, ANCHOR_MARK * 1.6, Stroke::new(2.0, ACCENT));
    }
    canvas_frames(doc, &painter, state, rect);
    paint_brush(ui, &painter, doc, selected, state, rect);

    // The guides the dragged layer snapped to, across the whole canvas.
    let guide = Stroke::new(1.0, ACCENT);
    if let Some(x) = state.snapped_guides[0] {
        let x = cam.to_screen(rect, [x, 0.0]).x;
        let line = [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())];
        painter.extend(Shape::dashed_line(&line, guide, 6.0, 4.0));
    }
    if let Some(y) = state.snapped_guides[1] {
        let y = cam.to_screen(rect, [0.0, y]).y;
        let line = [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)];
        painter.extend(Shape::dashed_line(&line, guide, 6.0, 4.0));
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
            if locked {
                lock_badge(ui, &painter, b.center());
            }
        }
    } else if let Some(layer) = doc.layer(sel)
        // Its points are being edited or drawn: they are shown instead.
        && state.points.as_ref().is_none_or(|e| e.layer != sel)
        && state.drawing.as_ref().is_none_or(|d| d.layer != Some(sel))
    {
        let (corners, top_mid, rot) = handles(&cam, rect, layer);
        let stroke = Stroke::new(1.5, color);
        painter.add(Shape::closed_line(corners.to_vec(), stroke));
        if locked {
            lock_badge(ui, &painter, cam.to_screen(rect, layer.transform.center));
        }
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

/// The canvas' tools, with their commands: selecting and moving, then painting.
pub const TOOLS: [(crate::shortcuts::Command, Option<crate::paint::Tool>); 4] = [
    (crate::shortcuts::Command::SelectTool, None),
    (
        crate::shortcuts::Command::PencilTool,
        Some(crate::paint::Tool::Pencil),
    ),
    (
        crate::shortcuts::Command::BrushTool,
        Some(crate::paint::Tool::Brush),
    ),
    (
        crate::shortcuts::Command::EraserTool,
        Some(crate::paint::Tool::Eraser),
    ),
];

/// The tool bar at the canvas' top left, while a closed shape is selected: the tools,
/// the button clearing the shape's paint (`can_clear` says whether it has any), then,
/// for a paint tool, its color (not the eraser's) and width. Returns true if the
/// clear button was clicked.
fn tool_bar(ui: &egui::Ui, state: &mut CanvasState, rect: Rect, can_clear: bool) -> bool {
    use crate::layers_ui::{Tool as Icon, tool_icon};
    let mut clear = false;
    egui::Area::new(ui.scope_id().with("tool_bar"))
        .fixed_pos(rect.min + vec2(8.0, 8.0))
        .order(egui::Order::Middle)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (command, tool) in TOOLS {
                            let icon = match tool {
                                None => Icon::Select,
                                Some(crate::paint::Tool::Pencil) => Icon::Pencil,
                                Some(crate::paint::Tool::Brush) => Icon::Brush(state.brushes.brush),
                                Some(crate::paint::Tool::Eraser) => Icon::Eraser,
                            };
                            let mut tip = crate::shortcuts::with_key(
                                command.label(),
                                state.keys.get(&command),
                            );
                            if tool == Some(crate::paint::Tool::Brush) {
                                tip = format!("{tip}\n{}", crate::i18n::tr!("BRUSH_KIND_HINT"));
                            }
                            // The tool in use stands out.
                            let on = state.brushes.tool == tool;
                            let at = ui.cursor().min;
                            if on {
                                ui.painter().rect_filled(
                                    Rect::from_min_size(at, vec2(24.0, 24.0)),
                                    3.0,
                                    ui.visuals().selection.bg_fill,
                                );
                            }
                            let button = tool_icon(ui, icon, true, &tip);
                            if button.clicked() {
                                state.brushes.tool = tool;
                                state.painting.clear();
                            }
                            if tool == Some(crate::paint::Tool::Brush) {
                                brush_menu(ui, &button, &mut state.brushes);
                            }
                        }
                        // Emptying the shape of its paint, with any tool.
                        ui.add_space(4.0);
                        let c = crate::shortcuts::Command::ClearPaint;
                        let tip = crate::shortcuts::with_key(c.label(), state.keys.get(&c));
                        clear =
                            crate::layers_ui::tool_button(ui, Icon::ClearPaint, can_clear, &tip);
                        let Some(tool) = state.brushes.tool else {
                            return;
                        };
                        ui.separator();
                        if tool != crate::paint::Tool::Eraser {
                            let mut link = None;
                            crate::colors::button(ui, &mut state.brushes.color, true, &mut link)
                                .on_hover_text(crate::i18n::tr!("PAINT_COLOR_HINT"));
                        }
                        ui.add(
                            egui::DragValue::new(state.brushes.width_mut(tool))
                                .range(0.05..=100.0)
                                .speed(0.05)
                                .max_decimals(2)
                                .suffix(" mm"),
                        )
                        .on_hover_text(crate::i18n::tr!("PAINT_WIDTH_HINT"));
                        if tool == crate::paint::Tool::Brush {
                            brush_options(ui, &mut state.brushes);
                        }
                    });
                });
        });
    clear
}

/// The brush button's corner mark, saying it has more kinds, and the menu of them
/// below it, opened by a click (which also takes the brush): each kind's icon and name.
fn brush_menu(ui: &egui::Ui, button: &egui::Response, brushes: &mut crate::paint::Brushes) {
    use crate::i18n::translate;
    use crate::layers_ui::{Tool as Icon, icon_entry};
    let r = button.rect;
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    ui.painter().add(Shape::convex_polygon(
        vec![
            r.right_bottom() - vec2(1.0, 1.0),
            r.right_bottom() - vec2(1.0, 5.0),
            r.right_bottom() - vec2(5.0, 1.0),
        ],
        color,
        Stroke::NONE,
    ));
    let labels = crate::paint::BrushKind::ALL.map(|k| translate(k.name_key()));
    egui::Popup::menu(button).show(|ui| {
        for kind in crate::paint::BrushKind::ALL {
            let on = brushes.brush == kind;
            let name = translate(kind.name_key());
            if icon_entry(ui, Icon::Brush(kind), name, &labels, on).clicked() {
                brushes.brush = kind;
                ui.close();
            }
        }
    });
}

/// A dial for an angle in degrees, counterclockwise from the right: a ring with a dot at
/// the angle and a faint line through its center, the calligraphy nib. Dragging or
/// clicking on it turns it there; a double click puts it back to the default.
fn angle_dial(ui: &mut egui::Ui, deg: &mut f32) -> egui::Response {
    const SIDE: f32 = 22.0;
    let (rect, mut r) = ui.allocate_exact_size(vec2(SIDE, SIDE), Sense::click_and_drag());
    let c = rect.center();
    if r.double_clicked() {
        *deg = crate::paint::Brushes::default().nib_angle;
        r.mark_changed();
    } else if (r.dragged() || r.clicked())
        && let Some(p) = r.interact_pointer_pos()
    {
        let v = p - c;
        if v.length() > 1.0 {
            // Up on screen is counterclockwise.
            *deg = (-v.y).atan2(v.x).to_degrees().round();
            r.mark_changed();
        }
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&r);
        let p = ui.painter();
        let radius = SIDE / 2.0 - 1.5;
        p.circle(c, radius, visuals.bg_fill, visuals.bg_stroke);
        let a = deg.to_radians();
        let dir = vec2(a.cos(), -a.sin());
        let faint = visuals.fg_stroke.color.gamma_multiply(0.4);
        p.line_segment(
            [c - dir * radius, c + dir * radius],
            Stroke::new(1.0, faint),
        );
        p.circle_filled(c + dir * (radius - 3.0), 2.5, visuals.fg_stroke.color);
    }
    r
}

/// The settings of the brush kinds that have some: the calligraphy nib's angle, the
/// spray's density.
fn brush_options(ui: &mut egui::Ui, brushes: &mut crate::paint::Brushes) {
    use crate::i18n::tr;
    use crate::paint::BrushKind;
    match brushes.brush {
        BrushKind::Calligraphy => {
            angle_dial(ui, &mut brushes.nib_angle).on_hover_text(tr!("NIB_ANGLE_HINT"));
        }
        BrushKind::Spray => {
            let mut percent = brushes.spray_density * 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .range(1.0..=100.0)
                        .speed(1.0)
                        .max_decimals(0)
                        .suffix(" %"),
                )
                .on_hover_text(tr!("SPRAY_DENSITY_HINT"))
                .changed()
            {
                brushes.spray_density = percent / 100.0;
            }
        }
        _ => {}
    }
}

/// The selected shape can be painted in and has paint to clear.
pub fn has_paint(doc: &Document, selected: Option<u64>) -> bool {
    can_paint(doc, selected)
        && selected
            .and_then(|id| doc.layer(id))
            .and_then(|l| l.shape.as_deref())
            .is_some_and(|s| !s.strokes.is_empty())
}

/// Empties the selected shape of its paint, if it can be painted in. Returns true if
/// anything was cleared.
pub fn clear_paint(doc: &mut Document, selected: Option<u64>) -> bool {
    has_paint(doc, selected)
        && selected
            .and_then(|id| doc.layer_mut(id))
            .is_some_and(crate::shape::clear_paint)
}

/// The paint tool in hand: the one picked last, while a shape that can be painted in is
/// selected.
pub fn active_tool(
    doc: &Document,
    selected: Option<u64>,
    brushes: &crate::paint::Brushes,
) -> Option<crate::paint::Tool> {
    brushes.tool.filter(|_| can_paint(doc, selected))
}

/// The selected layer, if it is a shape that can be painted in: closed, visible and
/// unlocked.
pub fn can_paint(doc: &Document, selected: Option<u64>) -> bool {
    selected.is_some_and(|id| {
        doc.layer(id).is_some_and(crate::shape::paintable)
            && !doc.effectively_locked(id)
            && doc.effectively_visible(id)
    })
}

/// Painting with the pencil or the brush, or erasing, in the selected closed shape: a
/// drag on the page paints a stroke, added when the button is released (what goes past
/// the shape's outline doesn't show); a click, a dot. The eraser cuts its strokes
/// as it goes. Returns true if the document changed.
#[allow(clippy::too_many_arguments)]
fn paint_input(
    ui: &egui::Ui,
    doc: &mut Document,
    selected: Option<u64>,
    state: &mut CanvasState,
    cam: Camera,
    rect: Rect,
    response: &egui::Response,
    tool: crate::paint::Tool,
) -> bool {
    use crate::paint::Tool;
    let Some(id) = selected.filter(|_| can_paint(doc, selected)) else {
        return false;
    };
    // A pen's pressure comes with touch events.
    let force = ui.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    });
    if force.is_some() {
        state.pen_pressure = force;
    }
    let pressure = state.pen_pressure.unwrap_or(1.0);
    let mut changed = false;
    let at = |p: Pos2| {
        let m = cam.to_mm(rect, p);
        [m[0], m[1], pressure]
    };
    let eraser = state.brushes.width(Tool::Eraser) / 2.0;
    let erase = |doc: &mut Document, path: &[P2]| {
        doc.layer_mut(id)
            .is_some_and(|l| crate::shape::erase(l, path, eraser))
    };
    if response.drag_started_by(PointerButton::Primary)
        && let Some(p) = ui
            .input(|i| i.pointer.press_origin())
            .or(response.interact_pointer_pos())
    {
        state.painting = vec![at(p)];
    }
    let pointer = response.interact_pointer_pos();
    if response.dragged_by(PointerButton::Primary)
        && let Some(p) = pointer
        && !state.painting.is_empty()
    {
        let s = at(p);
        let last = *state.painting.last().unwrap_or(&s);
        if last[0] != s[0] || last[1] != s[1] {
            state.painting.push(s);
            if tool == Tool::Eraser {
                changed |= erase(doc, &[[last[0], last[1]], [s[0], s[1]]]);
            }
        }
        ui.ctx().request_repaint();
    }
    let done = response.drag_stopped_by(PointerButton::Primary)
        || (response.clicked_by(PointerButton::Primary) && state.painting.is_empty());
    if done {
        let mut samples = std::mem::take(&mut state.painting);
        if samples.is_empty()
            && let Some(p) = pointer
        {
            samples.push(at(p));
        }
        if !samples.is_empty() {
            changed |= match tool {
                Tool::Eraser => {
                    let path: Vec<P2> = samples.iter().map(|s| [s[0], s[1]]).collect();
                    erase(doc, &path)
                }
                _ => {
                    let pen = state.brushes.pen(tool, &samples);
                    doc.layer_mut(id)
                        .is_some_and(|l| crate::shape::add_stroke(l, &samples, &pen))
                }
            };
        }
        state.pen_pressure = None;
    }
    changed
}

/// The outlines of the visible closed shapes with neither fill nor stroke, faint and
/// dashed so that one added to paint in shows (not printed).
fn canvas_frames(doc: &Document, painter: &egui::Painter, state: &CanvasState, rect: Rect) {
    let cam = state.camera;
    let frame = Stroke::new(1.0, Color32::from_gray(140));
    for leaf in doc.leaves() {
        let unseen = leaf
            .layer
            .shape
            .as_deref()
            .is_some_and(|s| s.kind.closed() && !s.fill && !(s.stroke && s.stroke_width > 0.0));
        if !leaf.visible || !unseen {
            continue;
        }
        let Some((outline, _)) = crate::shape::page_outline(leaf.layer) else {
            continue;
        };
        let mut pts: Vec<Pos2> = outline.iter().map(|&p| cam.to_screen(rect, p)).collect();
        pts.extend(pts.first().copied());
        painter.extend(Shape::dashed_line(&pts, frame, 4.0, 4.0));
    }
}

/// The stroke being painted, as it goes, and the tool's size around the pointer.
fn paint_brush(
    ui: &egui::Ui,
    painter: &egui::Painter,
    doc: &Document,
    selected: Option<u64>,
    state: &mut CanvasState,
    rect: Rect,
) {
    use crate::paint::{BrushKind, Tool};
    let Some(tool) = active_tool(doc, selected, &state.brushes) else {
        return;
    };
    let cam = state.camera;
    let width = state.brushes.width(tool) * cam.zoom;
    if tool != Tool::Eraser && !state.painting.is_empty() {
        // Cut off at the canvas' box on screen, as the stroke will be.
        let clip = selected
            .and_then(|id| doc.layer(id))
            .and_then(crate::shape::page_outline)
            .map(|(o, _)| {
                Rect::from_points(
                    &o.iter()
                        .map(|&p| cam.to_screen(rect, p))
                        .collect::<Vec<_>>(),
                )
            });
        let painter = match clip {
            Some(c) => painter.with_clip_rect(c.intersect(painter.clip_rect())),
            None => painter.clone(),
        };
        let pen = state.brushes.pen(tool, &state.painting);
        let strokes = crate::paint::PaintStroke::paint(&state.painting, &pen);
        stroke_preview(
            ui.ctx(),
            &painter,
            &mut state.paint_preview,
            &strokes,
            cam,
            rect,
        );
    }
    // Only over the bare canvas, not the tool bar, a window or a popup over it.
    if let Some(p) = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|_| ui.rect_contains_pointer(rect))
    {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        // The tool's reach, in black and white so it shows on anything: the calligraphy
        // nib as a line at its angle.
        if tool == Tool::Brush && state.brushes.brush == BrushKind::Calligraphy {
            let a = state.brushes.nib_angle.to_radians();
            let h = vec2(a.cos(), -a.sin()) * (width / 2.0).max(4.0);
            painter.line_segment([p - h, p + h], Stroke::new(3.0, Color32::WHITE));
            painter.line_segment([p - h, p + h], Stroke::new(1.0, Color32::BLACK));
        } else {
            let r = (width / 2.0).max(2.0);
            painter.circle_stroke(p, r + 1.0, Stroke::new(1.0, Color32::WHITE));
            painter.circle_stroke(p, r, Stroke::new(1.0, Color32::BLACK));
        }
    }
}

/// Draws `strokes` (page mm) as they will be filled, over the part of the screen they
/// cover within the painter's clip, through `texture`.
fn stroke_preview(
    ctx: &egui::Context,
    painter: &egui::Painter,
    texture: &mut Option<egui::TextureHandle>,
    strokes: &[crate::paint::PaintStroke],
    cam: Camera,
    rect: Rect,
) {
    let Some((lo, hi)) = strokes
        .iter()
        .filter_map(crate::paint::PaintStroke::bounds)
        .reduce(|a, b| {
            (
                [a.0[0].min(b.0[0]), a.0[1].min(b.0[1])],
                [a.1[0].max(b.1[0]), a.1[1].max(b.1[1])],
            )
        })
    else {
        return;
    };
    let area = Rect::from_min_max(cam.to_screen(rect, lo), cam.to_screen(rect, hi))
        .expand(1.0)
        .intersect(painter.clip_rect());
    if !area.is_positive() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let size = [area.width(), area.height()].map(|v| (v * ppp).ceil().clamp(1.0, 8192.0) as u32);
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size[0], size[1]) else {
        return;
    };
    // From page mm to the pixmap's pixels.
    let o = cam.to_screen(rect, [0.0; 2]);
    let k = cam.zoom * ppp;
    let ts = tiny_skia::Transform::from_scale(k, k)
        .post_translate((o.x - area.min.x) * ppp, (o.y - area.min.y) * ppp);
    for s in strokes {
        if let Some(path) = s.outline() {
            let ink = crate::gradient::Ink::Solid(s.color);
            pixmap.fill_path(&path, &ink.paint(), tiny_skia::FillRule::Winding, ts, None);
        }
    }
    let image = egui::ColorImage::from_rgba_premultiplied(
        [size[0] as usize, size[1] as usize],
        pixmap.data(),
    );
    let options = egui::TextureOptions::LINEAR;
    let id = match texture {
        Some(t) => {
            t.set(image, options);
            t.id()
        }
        None => texture
            .insert(ctx.load_texture("paint-preview", image, options))
            .id(),
    };
    let shown = Rect::from_min_size(area.min, vec2(size[0] as f32, size[1] as f32) / ppp);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    painter.image(id, shown, uv, Color32::WHITE);
}

/// Longest side of the page background's gradient as drawn on screen, in pixels: a
/// gradient is smooth, so a small texture stretched over the page shows it well.
const BACKGROUND_SIDE: f32 = 512.0;

/// The page background's gradient as a texture the page's shape, drawn again when the
/// gradient, the color or the page changes.
fn background_texture(
    ctx: &egui::Context,
    state: &mut CanvasState,
    doc: &Document,
    gradient: &crate::gradient::Gradient,
) -> egui::TextureId {
    let page = doc.page_mm();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(gradient)
            .unwrap_or_default()
            .hash(&mut h);
        doc.background.hash(&mut h);
        page.map(f32::to_bits).hash(&mut h);
        h.finish()
    };
    if let Some((k, t)) = &state.background
        && *k == key
    {
        return t.id();
    }
    let k = BACKGROUND_SIDE / page[0].max(page[1]).max(1.0);
    let (w, h) = (
        (page[0] * k).round().max(1.0) as u32,
        (page[1] * k).round().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("a small pixmap");
    crate::render::fill_background(&mut pixmap, doc.background, Some(gradient));
    let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    let texture = ctx.load_texture("page-background", image, egui::TextureOptions::LINEAR);
    let id = texture.id();
    state.background = Some((key, texture));
    id
}

/// Radius of the badge showing that the selected layer is locked.
const LOCK_BADGE: f32 = 18.0;

/// A white padlock on a dark disc at `c` (the middle of the selection on screen), seen on
/// any artwork, with a tooltip saying why the layer does not move.
fn lock_badge(ui: &egui::Ui, painter: &egui::Painter, c: Pos2) {
    // A soft shadow, then the disc and its white ring.
    painter.circle_filled(
        c + vec2(0.0, 1.5),
        LOCK_BADGE + 1.5,
        Color32::from_black_alpha(60),
    );
    painter.circle(
        c,
        LOCK_BADGE,
        Color32::from_black_alpha(185),
        Stroke::new(2.0, Color32::WHITE),
    );
    let ink = Color32::WHITE;
    // Body in the lower half; the shackle, an arch, above it.
    let body = Rect::from_min_max(c + vec2(-8.0, -1.0), c + vec2(8.0, 10.0));
    painter.rect_filled(body, 2.5, ink);
    // The keyhole.
    let dark = Color32::from_black_alpha(185);
    painter.circle_filled(c + vec2(0.0, 3.5), 1.8, dark);
    painter.rect_filled(
        Rect::from_center_size(c + vec2(0.0, 6.0), vec2(1.6, 3.5)),
        0.5,
        dark,
    );
    let r = 5.0;
    let top = c + vec2(0.0, -6.0);
    let mut shackle = vec![c + vec2(-r, -1.0)];
    shackle.extend((0..=16).map(|i| {
        let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
        top + vec2(r * a.cos(), r * a.sin())
    }));
    shackle.push(c + vec2(r, -1.0));
    painter.add(Shape::line(shackle, Stroke::new(2.6, ink)));
    let badge = Rect::from_center_size(c, Vec2::splat(LOCK_BADGE * 2.0));
    ui.interact(
        badge,
        ui.scope_id().with("lock_badge"),
        egui::Sense::hover(),
    )
    .on_hover_text(crate::i18n::tr!("LAYER_LOCKED_HINT"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_area_guide_is_inside_the_outline_and_around_holes() {
        use crate::template::Polyline;
        let rect_poly = |lo: P2, hi: P2| Polyline {
            points: vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
            closed: true,
        };
        let element = |kind, poly| PlacedElement {
            kind,
            polylines: vec![poly],
            anchors: Vec::new(),
        };
        let elements = [
            element(ElementKind::Outline, rect_poly([10.0, 10.0], [90.0, 40.0])),
            element(ElementKind::Buttons, rect_poly([20.0, 20.0], [30.0, 30.0])),
        ];
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let run = |state: &mut CanvasState| {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                paint_safe_area(ui.painter(), &elements, state, rect);
            });
            out.textures_delta.clear();
        };
        run(&mut state);
        assert!(state.safe_area.is_none(), "no guide without a margin");
        state.safe_margin_mm = 2.0;
        run(&mut state);
        let (_, rings) = state.safe_area.as_ref().unwrap();
        assert_eq!(rings.len(), 2);
        let b = |r: &Vec<P2>| {
            crate::print::bounds(&[Polyline {
                points: r.clone(),
                closed: true,
            }])
            .unwrap()
        };
        let mut boxes: Vec<_> = rings.iter().map(b).collect();
        boxes.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        let near = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        assert!(near(boxes[0].0, [12.0, 12.0]) && near(boxes[0].1, [88.0, 38.0]));
        assert!(near(boxes[1].0, [18.0, 18.0]) && near(boxes[1].1, [32.0, 32.0]));
    }

    #[test]
    fn display_settings_come_back_next_session() {
        #[derive(Default)]
        struct Memory(HashMap<String, String>);
        impl eframe::Storage for Memory {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.to_owned(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let mut storage = Memory::default();
        let fresh = CanvasState::load(Some(&storage));
        assert!(fresh.preview_cuts && fresh.preview_bleed && fresh.preview_safe_area);
        let state = CanvasState {
            preview_cuts: false,
            cut_opacity: 0.4,
            preview_bleed: false,
            preview_safe_area: false,
            safe_margin_mm: 3.0,
            ..Default::default()
        };
        state.save(&mut storage);
        let back = CanvasState::load(Some(&storage));
        assert!(!back.preview_cuts && !back.preview_bleed && !back.preview_safe_area);
        assert_eq!((back.cut_opacity, back.safe_margin_mm), (0.4, 3.0));
    }

    #[test]
    fn the_safe_area_preview_can_be_turned_off() {
        use crate::template::Polyline;
        let elements = [PlacedElement {
            kind: ElementKind::Outline,
            polylines: vec![Polyline {
                points: vec![[10.0, 10.0], [90.0, 10.0], [90.0, 40.0], [10.0, 40.0]],
                closed: true,
            }],
            anchors: Vec::new(),
        }];
        let ctx = egui::Context::default();
        let mut doc = Document::default();
        let mut selected = None;
        let mut frame = |state: &mut CanvasState| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, &mut doc, &elements, &mut selected, state);
            });
            out.textures_delta.clear();
        };
        let mut state = CanvasState {
            safe_margin_mm: 2.0,
            preview_safe_area: false,
            ..Default::default()
        };
        frame(&mut state);
        assert!(state.safe_area.is_none(), "hidden: not drawn");
        state.preview_safe_area = true;
        frame(&mut state);
        assert!(state.safe_area.is_some(), "shown");
    }

    #[test]
    fn dragging_a_member_of_the_selected_group_moves_the_group() {
        use crate::document::Group;
        let square = |id: u64, x: f32| {
            let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
            l.size_px = [10, 10];
            l.transform.center = [x, 50.0];
            Node::Layer(Box::new(l))
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
    fn a_canvas_is_placed_by_a_drag_then_painted_in_and_erased() {
        use crate::paint::Tool;
        let mut doc = Document::default();
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut selected = None;
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
        let (cam, rect) = (state.camera, state.rect);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut drag = |doc: &mut Document,
                        sel: &mut Option<u64>,
                        state: &mut CanvasState,
                        from: P2,
                        to: P2| {
            let (a, b) = (cam.to_screen(rect, from), cam.to_screen(rect, to));
            frame(doc, sel, state, vec![Event::PointerMoved(a)]);
            frame(doc, sel, state, vec![button(a, true)]);
            for k in 1..=10 {
                let p = a + (b - a) * (k as f32 / 10.0);
                frame(doc, sel, state, vec![Event::PointerMoved(p)]);
            }
            frame(doc, sel, state, vec![button(b, false)]);
            frame(doc, sel, state, vec![]);
        };
        let strokes = |doc: &Document, id: u64| {
            doc.layer(id)
                .unwrap()
                .shape
                .as_ref()
                .unwrap()
                .strokes
                .clone()
        };

        // No Canvas selected: the pencil waits, a click paints nothing (a drag on the
        // empty page would pan it).
        state.brushes.tool = Some(Tool::Pencil);
        drag(
            &mut doc,
            &mut selected,
            &mut state,
            [50.0, 100.0],
            [50.0, 100.0],
        );
        assert_eq!(
            state.brushes.tool,
            Some(Tool::Pencil),
            "kept for the next Canvas"
        );
        assert!(doc.layers.is_empty());

        // A Canvas selected, as the Canvas button leaves it.
        doc.layers
            .push(Node::Layer(Box::new(crate::shape::new_canvas_layer(
                7,
                crate::shape::ShapeKind::Rectangle,
                [40.0, 80.0],
                [160.0, 140.0],
            ))));
        // The pencil picked before is in hand.
        selected = Some(7);
        let id = 7;
        let size = doc.layer(id).unwrap().shape.as_ref().unwrap().size;

        drag(
            &mut doc,
            &mut selected,
            &mut state,
            [50.0, 100.0],
            [150.0, 100.0],
        );
        assert_eq!(strokes(&doc, id).len(), 1);
        let s = &strokes(&doc, id)[0];
        assert_eq!(s.points.len(), 2, "a straight line: its two ends");
        // Painted where the pointer went: its ends on the page.
        let ends: Vec<P2> = s
            .points
            .iter()
            .map(|p| crate::shape::to_page(doc.layer(id).unwrap(), [p[0], p[1]]).unwrap())
            .collect();
        assert!(
            (ends[0][0] - 50.0).abs() < 0.5 && (ends[1][0] - 150.0).abs() < 0.5,
            "{ends:?}"
        );

        // The next stroke goes into it too; the canvas keeps its size.
        drag(
            &mut doc,
            &mut selected,
            &mut state,
            [50.0, 120.0],
            [150.0, 120.0],
        );
        assert_eq!(selected, Some(id));
        assert_eq!(strokes(&doc, id).len(), 2);
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layer(id).unwrap().shape.as_ref().unwrap().size, size);
        // A stroke all outside it is left out.
        drag(
            &mut doc,
            &mut selected,
            &mut state,
            [50.0, 200.0],
            [150.0, 200.0],
        );
        assert_eq!(strokes(&doc, id).len(), 2);

        // The eraser across the first stroke, between both: it is cut in two.
        state.brushes.tool = Some(Tool::Eraser);
        drag(
            &mut doc,
            &mut selected,
            &mut state,
            [100.0, 90.0],
            [100.0, 110.0],
        );
        assert_eq!(strokes(&doc, id).len(), 3);
    }

    #[test]
    fn locked_layers_are_selected_by_a_click_but_not_moved() {
        let square = |id: u64, x: f32, size: u32, locked: bool| {
            let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
            l.size_px = [size, size];
            l.transform.center = [x, 50.0];
            l.locked = locked;
            Node::Layer(Box::new(l))
        };
        // A locked background under an unlocked square, and a locked square on top of it.
        let mut doc = Document {
            layers: vec![
                square(1, 100.0, 150, true),
                square(2, 60.0, 10, false),
                square(3, 60.0, 4, true),
            ],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut selected = None;
        let mut time = 0.0;
        let mut frame = |doc: &mut Document, sel: &mut Option<u64>, events: Vec<Event>| {
            time += 0.5;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], sel, &mut state);
            });
            out.textures_delta.clear();
            (state.camera, state.rect)
        };
        let (cam, rect) = frame(&mut doc, &mut selected, vec![]);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut click = |doc: &mut Document, sel: &mut Option<u64>, at: P2| {
            let p = cam.to_screen(rect, at);
            frame(doc, sel, vec![Event::PointerMoved(p)]);
            frame(doc, sel, vec![button(p, true)]);
            frame(doc, sel, vec![button(p, false)]);
        };
        // Only the locked background there: selected.
        click(&mut doc, &mut selected, [140.0, 50.0]);
        assert_eq!(selected, Some(1));
        // The locked square over the unlocked one lets the click through to it.
        click(&mut doc, &mut selected, [60.0, 50.0]);
        assert_eq!(selected, Some(2));

        // Dragging the locked background selects it, and it stays put.
        let (from, to) = (
            cam.to_screen(rect, [140.0, 50.0]),
            cam.to_screen(rect, [150.0, 60.0]),
        );
        frame(&mut doc, &mut selected, vec![Event::PointerMoved(from)]);
        frame(&mut doc, &mut selected, vec![button(from, true)]);
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            frame(&mut doc, &mut selected, vec![Event::PointerMoved(p)]);
        }
        frame(&mut doc, &mut selected, vec![button(to, false)]);
        assert_eq!(selected, Some(1));
        assert_eq!(doc.layer(1).unwrap().transform.center, [100.0, 50.0]);
    }

    #[test]
    fn a_pen_pressing_lightly_paints_thinner() {
        use crate::paint::Tool;
        let mut doc = Document::default();
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        state.brushes.tool = Some(Tool::Pencil);
        *state.brushes.width_mut(Tool::Pencil) = 2.0;
        doc.layers
            .push(Node::Layer(Box::new(crate::shape::new_canvas_layer(
                1,
                crate::shape::ShapeKind::Rectangle,
                [0.0, 50.0],
                [200.0, 150.0],
            ))));
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
        let (cam, rect) = (state.camera, state.rect);
        let (a, b) = (
            cam.to_screen(rect, [50.0, 100.0]),
            cam.to_screen(rect, [150.0, 100.0]),
        );
        // A pen: touch events with its pressure, alongside the pointer it moves.
        let touch = |pos, phase| Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: Some(0.5),
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut doc, &mut state, vec![Event::PointerMoved(a)]);
        frame(
            &mut doc,
            &mut state,
            vec![touch(a, egui::TouchPhase::Start), button(a, true)],
        );
        for k in 1..=10 {
            let p = a + (b - a) * (k as f32 / 10.0);
            frame(
                &mut doc,
                &mut state,
                vec![touch(p, egui::TouchPhase::Move), Event::PointerMoved(p)],
            );
        }
        frame(
            &mut doc,
            &mut state,
            vec![touch(b, egui::TouchPhase::End), button(b, false)],
        );
        let layer = doc.leaves()[0].layer;
        let s = &layer.shape.as_ref().unwrap().strokes[0];
        assert!(
            s.points.iter().all(|p| (p[2] - 1.0).abs() < 0.01),
            "{:?}",
            s.points
        );
    }

    #[test]
    fn clear_paint_empties_the_selected_shape_only() {
        use crate::paint::{Pen, Tool};
        let mut l = crate::shape::new_layer(1, crate::shape::ShapeKind::Rectangle, [50.0, 50.0]);
        let line = [[45.0, 50.0, 1.0], [55.0, 50.0, 1.0]];
        assert!(crate::shape::add_stroke(
            &mut l,
            &line,
            &Pen::new(Tool::Pencil, 1.0, [0; 4])
        ));
        let mut doc = Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        };
        assert!(!clear_paint(&mut doc, None));
        assert!(has_paint(&doc, Some(1)));
        assert!(clear_paint(&mut doc, Some(1)));
        assert!(
            doc.layer(1)
                .unwrap()
                .shape
                .as_ref()
                .unwrap()
                .strokes
                .is_empty()
        );
        // Nothing left: nothing changes.
        assert!(!has_paint(&doc, Some(1)) && !clear_paint(&mut doc, Some(1)));
    }

    /// A stroke of the pencil and of each kind of brush painted in an ellipse, one cut
    /// by the eraser, the strokes cut off at its outline, under its stroke, with the
    /// tool bar and the calligraphy brush in use, to look at them:
    /// `PAINT_PREVIEW=out.png cargo test paint_preview -- --ignored`.
    #[test]
    #[ignore]
    fn paint_preview() {
        use crate::paint::{BrushKind, Pen, Tool};
        let Ok(path) = std::env::var("PAINT_PREVIEW") else {
            return;
        };
        let mut layer = crate::shape::new_canvas_layer(
            1,
            crate::shape::ShapeKind::Rectangle,
            [40.0, 40.0],
            [200.0, 150.0],
        );
        // Its background and border: the paint goes between them.
        crate::shape::set_kind(&mut layer, crate::shape::ShapeKind::Ellipse);
        if let Some(s) = layer.shape.as_deref_mut() {
            (s.fill, s.fill_color) = (true, [250, 240, 200, 255]);
            (s.stroke, s.stroke_width) = (true, 2.0);
        }
        let wave = |y0: f32, amp: f32| -> Vec<[f32; 3]> {
            (0..=120)
                .map(|i| {
                    let x = 40.0 + i as f32 * 1.4;
                    [x, y0 + amp * (x / 18.0).sin(), 1.0]
                })
                .collect()
        };
        let strokes = [
            (Pen::new(Tool::Pencil, 0.8, [20, 20, 20, 255]), 55.0),
            (Pen::brush(BrushKind::Round, 8.0, [200, 40, 60, 255]), 70.0),
            (
                Pen::brush(BrushKind::Calligraphy, 7.0, [20, 20, 90, 255]),
                85.0,
            ),
            (
                Pen::brush(BrushKind::Marker, 8.0, [240, 200, 0, 255]),
                100.0,
            ),
            (Pen::brush(BrushKind::Spray, 9.0, [30, 140, 60, 255]), 115.0),
            (
                Pen::brush(BrushKind::Bristle, 8.0, [120, 60, 20, 255]),
                132.0,
            ),
        ];
        for (pen, y) in strokes {
            assert!(crate::shape::add_stroke(&mut layer, &wave(y, 6.0), &pen));
        }
        crate::shape::erase(&mut layer, &[[120.0, 40.0], [125.0, 80.0]], 2.0);
        let doc = std::cell::RefCell::new(Document {
            layers: vec![Node::Layer(Box::new(layer))],
            ..Default::default()
        });
        let mut st = CanvasState::default();
        st.brushes.tool = Some(Tool::Brush);
        st.brushes.brush = BrushKind::Calligraphy;
        st.brushes.color = [200, 40, 60, 255];
        let state = std::cell::RefCell::new(st);
        // The preview draws each theme with a context of its own: the layer's texture
        // is made again for each.
        let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
        crate::preview::render(&path, vec2(760.0, 560.0), |ui| {
            let mut doc = doc.borrow_mut();
            if last.borrow().as_ref() != Some(ui.ctx()) {
                doc.for_each_layer_mut(|l| l.pixmap = None);
                *last.borrow_mut() = Some(ui.ctx().clone());
            }
            crate::shape::sync(ui.ctx(), &mut doc, true);
            let mut sel = Some(1);
            show(ui, &mut doc, &[], &mut sel, &mut state.borrow_mut());
        });
    }

    /// A rectangle under a square outline with a square button hole, the cut preview
    /// covering and hatching what is cut away, to look at it:
    /// `CUTS_PREVIEW=out.png cargo test cuts_preview -- --ignored`.
    #[test]
    #[ignore]
    fn cuts_preview() {
        use crate::template::Polyline;
        let Ok(path) = std::env::var("CUTS_PREVIEW") else {
            return;
        };
        let mut l = crate::shape::new_layer(1, crate::shape::ShapeKind::Rectangle, [100.0, 80.0]);
        crate::shape::set_geometry_size(&mut l, [180.0, 140.0]);
        let doc = std::cell::RefCell::new(Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        });
        let square = |c: P2, r: f32| Polyline {
            points: vec![
                [c[0] - r, c[1] - r],
                [c[0] + r, c[1] - r],
                [c[0] + r, c[1] + r],
                [c[0] - r, c[1] + r],
            ],
            closed: true,
        };
        let element = |kind, poly| PlacedElement {
            kind,
            polylines: vec![poly],
            anchors: Vec::new(),
        };
        let elements = vec![
            element(ElementKind::Outline, square([100.0, 80.0], 50.0)),
            element(ElementKind::Buttons, square([100.0, 80.0], 15.0)),
        ];
        let state = std::cell::RefCell::new(CanvasState::default());
        // Each theme is drawn with a context of its own: textures are made again.
        let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
        crate::preview::render(&path, vec2(500.0, 400.0), |ui| {
            let mut doc = doc.borrow_mut();
            if last.borrow().as_ref() != Some(ui.ctx()) {
                doc.for_each_layer_mut(|l| l.pixmap = None);
                *state.borrow_mut() = CanvasState::default();
                *last.borrow_mut() = Some(ui.ctx().clone());
            }
            crate::shape::sync(ui.ctx(), &mut doc, true);
            let mut sel = None;
            show(ui, &mut doc, &elements, &mut sel, &mut state.borrow_mut());
        });
    }

    /// A selected locked layer, with its padlock badge, to look at it:
    /// `LOCK_PREVIEW=out.png cargo test lock_preview -- --ignored`.
    #[test]
    #[ignore]
    fn lock_preview() {
        let Ok(path) = std::env::var("LOCK_PREVIEW") else {
            return;
        };
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [80, 50];
        l.transform.center = [100.0, 80.0];
        l.transform.rotation_deg = 10.0;
        l.locked = true;
        let doc = std::cell::RefCell::new(Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        });
        let state = std::cell::RefCell::new(CanvasState::default());
        crate::preview::render(&path, vec2(500.0, 360.0), |ui| {
            let mut sel = Some(1);
            show(
                ui,
                &mut doc.borrow_mut(),
                &[],
                &mut sel,
                &mut state.borrow_mut(),
            );
        });
    }

    #[test]
    fn distort_mode_drags_one_corner() {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [100.0, 100.0];
        l.transform.rotation_deg = 30.0;
        let mut doc = Document {
            layers: vec![Node::Layer(Box::new(l))],
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
        canvas_drag_in(doc, from, to, modifiers, false, &[]);
    }

    /// `canvas_drag` with the Distort button on or off.
    fn canvas_drag_in(
        doc: &mut Document,
        from: P2,
        to: P2,
        modifiers: egui::Modifiers,
        distort_mode: bool,
        elements: &[PlacedElement],
    ) {
        let ctx = egui::Context::default();
        let mut state = CanvasState {
            distort_mode,
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
                show(ui, doc, elements, &mut selected, state);
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

    /// A 100 x 50 mm image centered at (100, 100): corners (50, 75) to (150, 125).
    fn snap_doc() -> Document {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [100.0, 100.0];
        Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        }
    }

    /// Template elements made of the given anchors only.
    fn anchors(points: &[P2]) -> Vec<PlacedElement> {
        vec![PlacedElement {
            kind: ElementKind::Anchors,
            polylines: Vec::new(),
            anchors: points.to_vec(),
        }]
    }

    #[test]
    fn distorted_layer_snaps_by_the_middle_of_what_is_drawn() {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [50.0, 25.0];
        assert_eq!(snap_points(&l)[0], [50.0, 25.0], "undistorted: its center");
        l.transform.rotation_deg = 30.0;
        let m = snap_points(&l)[0];
        assert!(
            (m[0] - 50.0).abs() < 1e-3 && (m[1] - 25.0).abs() < 1e-3,
            "rotated too"
        );
        // The right corners pulled further right: the drawn shape spans x 0..140.
        l.transform.rotation_deg = 0.0;
        l.transform.distort = [[0.0, 0.0], [0.4, 0.0], [0.4, 0.0], [0.0, 0.0]];
        let m = snap_points(&l)[0];
        assert!(
            (m[0] - 70.0).abs() < 1e-3 && (m[1] - 25.0).abs() < 1e-3,
            "{m:?}"
        );
    }

    #[test]
    fn editing_a_polyline_s_points_on_the_canvas() {
        use crate::shape::{self, ShapeKind};
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        let mut doc = Document {
            layers: vec![Node::Layer(Box::new(shape::new_layer(
                1,
                ShapeKind::Polyline,
                [100.0, 100.0],
            )))],
            ..Default::default()
        };
        let elements = anchors(&[[70.5, 120.5]]);
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut selected = Some(1);
        let mut time = 0.0;
        let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
            time += 0.02;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &elements, &mut selected, state);
                shape::sync(ui.ctx(), doc, !ui.input(|i| i.pointer.any_down()));
            });
            out.textures_delta.clear();
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut doc, &mut state, vec![]);
        let (cam, rect) = (state.camera, state.rect);
        let screen = |p: P2| cam.to_screen(rect, p);
        let drag =
            |doc: &mut Document,
             state: &mut CanvasState,
             from: P2,
             to: P2,
             frame: &mut dyn FnMut(&mut Document, &mut CanvasState, Vec<Event>)| {
                let (from, to) = (screen(from), screen(to));
                frame(doc, state, vec![Event::PointerMoved(from)]);
                frame(doc, state, vec![button(from, true)]);
                for k in 1..=4 {
                    frame(
                        doc,
                        state,
                        vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))],
                    );
                }
                frame(doc, state, vec![button(to, false)]);
                frame(doc, state, vec![]);
            };

        // A double click on the shape shows its points.
        let c = screen([100.0, 100.0]);
        frame(&mut doc, &mut state, vec![Event::PointerMoved(c)]);
        for pressed in [true, false, true, false] {
            frame(&mut doc, &mut state, vec![button(c, pressed)]);
        }
        assert_eq!(state.points.as_ref().map(|e| e.layer), Some(1));

        // The first point dragged near the anchor lands on it; the others stay.
        let before = shape::page_points(doc.layer(1).unwrap());
        drag(&mut doc, &mut state, before[0], [72.0, 119.0], &mut frame);
        let after = shape::page_points(doc.layer(1).unwrap());
        assert!(close(after[0], [70.5, 120.5]), "{:?}", after[0]);
        for k in 1..4 {
            assert!(
                close(after[k], before[k]),
                "point {k}: {:?} {:?}",
                after[k],
                before[k]
            );
        }
        assert_eq!(state.points.as_ref().unwrap().selected, Some(0));

        // A double click on the first segment adds a point there.
        let mid = [
            (after[0][0] + after[1][0]) / 2.0,
            (after[0][1] + after[1][1]) / 2.0,
        ];
        let m = screen(mid);
        // Long enough after the last clicks not to make a triple click with them.
        for _ in 0..20 {
            frame(&mut doc, &mut state, vec![Event::PointerMoved(m)]);
        }
        for pressed in [true, false, true, false] {
            frame(&mut doc, &mut state, vec![button(m, pressed)]);
        }
        frame(&mut doc, &mut state, vec![]);
        let now = shape::page_points(doc.layer(1).unwrap());
        assert_eq!(now.len(), 5);
        assert!(close(now[1], mid), "{:?} {mid:?}", now[1]);
        assert!(close(now[0], after[0]));
        assert_eq!(state.points.as_ref().unwrap().selected, Some(1));

        // Moving points again and again never adds any.
        for round in 0..3 {
            for i in [0usize, 2, 3, 4, 1] {
                let p = shape::page_points(doc.layer(1).unwrap())[i];
                let to = [p[0] + 3.0 + round as f32, p[1] - 2.0];
                drag(&mut doc, &mut state, p, to, &mut frame);
                let pts = shape::page_points(doc.layer(1).unwrap());
                assert_eq!(pts.len(), 5, "round {round}, point {i}");
                assert!(
                    close(pts[i], to),
                    "round {round}, point {i}: {:?} {to:?}",
                    pts[i]
                );
            }
        }

        // Real mice: a move and the press in one frame, a click first, a pause.
        for i in [0usize, 2, 3] {
            let p = shape::page_points(doc.layer(1).unwrap())[i];
            let sp = screen(p);
            frame(
                &mut doc,
                &mut state,
                vec![Event::PointerMoved(sp), button(sp, true)],
            );
            frame(&mut doc, &mut state, vec![button(sp, false)]);
            frame(&mut doc, &mut state, vec![]);
            let to = screen([p[0] + 4.0, p[1] + 4.0]);
            frame(
                &mut doc,
                &mut state,
                vec![
                    Event::PointerMoved(sp + vec2(1.0, 1.0)),
                    button(sp + vec2(1.0, 1.0), true),
                ],
            );
            frame(&mut doc, &mut state, vec![Event::PointerMoved(to)]);
            frame(
                &mut doc,
                &mut state,
                vec![Event::PointerMoved(to), button(to, false)],
            );
            frame(&mut doc, &mut state, vec![]);
            assert_eq!(
                shape::page_points(doc.layer(1).unwrap()).len(),
                5,
                "point {i}"
            );
        }

        // Escape stops editing.
        frame(
            &mut doc,
            &mut state,
            vec![Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(state.points.is_none());
    }

    #[test]
    fn drawing_shapes_point_by_point() {
        use crate::shape::{self, ShapeKind};
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        let elements = anchors(&[[150.5, 60.5]]);
        let ctx = egui::Context::default();
        let mut doc = Document::default();
        let mut state = CanvasState::default();
        let mut selected = None;
        let mut time = 0.0;
        let mut frame = |doc: &mut Document,
                         state: &mut CanvasState,
                         selected: &mut Option<u64>,
                         events: Vec<Event>| {
            // Apart enough that two clicks are not a double click.
            time += 0.5;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &elements, selected, state);
                shape::sync(ui.ctx(), doc, !ui.input(|i| i.pointer.any_down()));
            });
            out.textures_delta.clear();
        };
        frame(&mut doc, &mut state, &mut selected, vec![]);
        let (cam, rect) = (state.camera, state.rect);
        let click = |p: P2| {
            let s = cam.to_screen(rect, p);
            let b = |pressed| Event::PointerButton {
                pos: s,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            vec![Event::PointerMoved(s), b(true), b(false)]
        };

        // A polyline closed by a click on its first point becomes a polygon.
        state.drawing = Some(Drawing::new(ShapeKind::Polyline));
        for p in [
            [100.0, 100.0],
            [150.0, 61.0],
            [180.0, 120.0],
            [100.2, 100.2],
        ] {
            frame(&mut doc, &mut state, &mut selected, click(p));
        }
        assert!(state.drawing.is_none(), "done");
        let id = selected.expect("the new shape is selected");
        let l = doc.layer(id).unwrap();
        let s = l.shape.as_ref().unwrap();
        assert_eq!((s.kind, l.name.as_str()), (ShapeKind::Polygon, "Polygon"));
        assert!(s.fill, "closed: filled");
        let pts = shape::page_points(l);
        assert_eq!(pts.len(), 3);
        assert!(close(pts[0], [100.0, 100.0]));
        assert!(close(pts[1], [150.5, 60.5]), "snapped: {:?}", pts[1]);

        // A polygon canvas from the Canvas menu: drawn the same, to paint in.
        let mut d = Drawing::new(ShapeKind::Polygon);
        d.canvas = true;
        state.drawing = Some(d);
        for p in [[20.0, 20.0], [60.0, 20.0], [40.0, 50.0], [20.1, 20.1]] {
            frame(&mut doc, &mut state, &mut selected, click(p));
        }
        let l = doc.layer(selected.unwrap()).unwrap();
        let s = l.shape.as_ref().unwrap();
        assert_eq!((s.kind, l.name.as_str()), (ShapeKind::Polygon, "Canvas"));
        assert!(!s.fill && !s.stroke, "neither background nor border yet");
        assert!(can_paint(&doc, selected));

        // Enter finishes an open polyline.
        state.drawing = Some(Drawing::new(ShapeKind::Polyline));
        for p in [[50.0, 50.0], [60.0, 70.0], [70.0, 50.0]] {
            frame(&mut doc, &mut state, &mut selected, click(p));
        }
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(state.drawing.is_none());
        let l = doc.layer(selected.unwrap()).unwrap();
        let s = l.shape.as_ref().unwrap();
        assert_eq!(
            (s.kind, s.points.len(), s.fill),
            (ShapeKind::Polyline, 3, false)
        );

        // A line is done at its second click.
        state.drawing = Some(Drawing::new(ShapeKind::Line));
        for p in [[20.0, 150.0], [80.0, 150.0]] {
            frame(&mut doc, &mut state, &mut selected, click(p));
        }
        assert!(state.drawing.is_none());
        let l = doc.layer(selected.unwrap()).unwrap();
        assert_eq!(shape::page_points(l).len(), 2);
        assert_eq!(doc.leaves().len(), 4);
    }

    /// Runs one canvas frame with these events, in the tests that need helpers.
    type Frame<'a> = dyn FnMut(&mut Document, &mut CanvasState, &mut Option<u64>, Vec<Event>) + 'a;

    #[test]
    fn bezier_curves_on_the_canvas() {
        use crate::shape::{self, ShapeKind};
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.1 && (a[1] - b[1]).abs() < 0.1;
        let ctx = egui::Context::default();
        let mut doc = Document::default();
        let mut state = CanvasState::default();
        let mut selected = None;
        let mut time = 0.0;
        let mut frame = |doc: &mut Document,
                         state: &mut CanvasState,
                         selected: &mut Option<u64>,
                         events: Vec<Event>| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], selected, state);
                shape::sync(ui.ctx(), doc, !ui.input(|i| i.pointer.any_down()));
            });
            out.textures_delta.clear();
        };
        frame(&mut doc, &mut state, &mut selected, vec![]);
        let (cam, rect) = (state.camera, state.rect);
        let s = |p: P2| cam.to_screen(rect, p);
        let b = |p: P2, pressed, modifiers| Event::PointerButton {
            pos: s(p),
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        let none = egui::Modifiers::NONE;
        let idle = |frame: &mut Frame,
                    doc: &mut Document,
                    state: &mut CanvasState,
                    selected: &mut Option<u64>| {
            for _ in 0..10 {
                frame(doc, state, selected, vec![]);
            }
        };

        // Drawing: a click, a press dragged (a smooth point), a click, then Enter.
        state.drawing = Some(Drawing::new(ShapeKind::Polyline));
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![
                Event::PointerMoved(s([100.0, 100.0])),
                b([100.0, 100.0], true, none),
                b([100.0, 100.0], false, none),
            ],
        );
        idle(&mut frame, &mut doc, &mut state, &mut selected);
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![
                Event::PointerMoved(s([140.0, 100.0])),
                b([140.0, 100.0], true, none),
            ],
        );
        for x in [142.0, 146.0, 150.0] {
            frame(
                &mut doc,
                &mut state,
                &mut selected,
                vec![Event::PointerMoved(s([x, 90.0]))],
            );
        }
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![b([150.0, 90.0], false, none)],
        );
        idle(&mut frame, &mut doc, &mut state, &mut selected);
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![
                Event::PointerMoved(s([180.0, 100.0])),
                b([180.0, 100.0], true, none),
                b([180.0, 100.0], false, none),
            ],
        );
        let enter = Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: none,
        };
        frame(&mut doc, &mut state, &mut selected, vec![enter]);
        assert!(state.drawing.is_none());
        let id = selected.unwrap();
        let l = doc.layer(id).unwrap();
        let sh = l.shape.as_ref().unwrap();
        assert_eq!(sh.points.len(), 3);
        assert!(sh.is_smooth(1), "{:?}", sh.handles);
        let [hin, hout] = shape::page_handles(l, 1);
        assert!(close(hout.unwrap(), [150.0, 90.0]), "{hout:?}");
        assert!(close(hin.unwrap(), [130.0, 110.0]), "mirrored: {hin:?}");
        assert!(
            sh.handle(0) == [[0.0; 2]; 2] && sh.handle(2) == [[0.0; 2]; 2],
            "clicks: sharp"
        );

        // Editing: dragging the out handle turns the in handle with it...
        state.points = Some(PointEdit::new(id));
        let p1 = shape::page_points(l)[1];
        let click = |p: P2| {
            vec![
                Event::PointerMoved(s(p)),
                b(p, true, none),
                b(p, false, none),
            ]
        };
        frame(&mut doc, &mut state, &mut selected, click(p1));
        idle(&mut frame, &mut doc, &mut state, &mut selected);
        let drag = |frame: &mut Frame,
                    doc: &mut Document,
                    state: &mut CanvasState,
                    selected: &mut Option<u64>,
                    from: P2,
                    to: P2,
                    m: egui::Modifiers| {
            frame(
                doc,
                state,
                selected,
                vec![
                    Event::ModifiersChanged(m),
                    Event::PointerMoved(s(from)),
                    b(from, true, m),
                ],
            );
            for k in 1..=4 {
                let t = k as f32 / 4.0;
                let p = [
                    from[0] + (to[0] - from[0]) * t,
                    from[1] + (to[1] - from[1]) * t,
                ];
                frame(doc, state, selected, vec![Event::PointerMoved(s(p))]);
            }
            frame(
                doc,
                state,
                selected,
                vec![
                    b(to, false, m),
                    Event::ModifiersChanged(egui::Modifiers::NONE),
                ],
            );
            for _ in 0..10 {
                frame(doc, state, selected, vec![]);
            }
        };
        drag(
            &mut frame,
            &mut doc,
            &mut state,
            &mut selected,
            [150.0, 90.0],
            [140.0, 85.0],
            none,
        );
        let l = doc.layer(id).unwrap();
        let [hin, hout] = shape::page_handles(l, 1);
        assert!(close(hout.unwrap(), [140.0, 85.0]), "{hout:?}");
        assert!(l.shape.as_ref().unwrap().is_smooth(1));
        assert_eq!(shape::page_points(l).len(), 3, "no point added");
        // ...unless Alt is held.
        drag(
            &mut frame,
            &mut doc,
            &mut state,
            &mut selected,
            [140.0, 85.0],
            [150.0, 100.0],
            egui::Modifiers::ALT,
        );
        let l = doc.layer(id).unwrap();
        assert!(!l.shape.as_ref().unwrap().is_smooth(1));
        let still = shape::page_handles(l, 1)[0].unwrap();
        assert!(close(still, hin.unwrap()), "the in handle stayed");

        // A double click on the point makes it sharp: no handles left at all.
        let p1 = shape::page_points(l)[1];
        for pressed in [true, false, true, false] {
            frame(
                &mut doc,
                &mut state,
                &mut selected,
                vec![Event::PointerMoved(s(p1)), b(p1, pressed, none)],
            );
        }
        let sh = doc.layer(id).unwrap().shape.as_ref().unwrap();
        assert!(sh.handles.is_empty(), "{:?}", sh.handles);
        assert!(state.points.is_some(), "still editing");
    }

    /// Template elements made of a rectangular outline from `lo` to `hi`.
    fn outline(lo: P2, hi: P2) -> Vec<PlacedElement> {
        vec![PlacedElement {
            kind: ElementKind::Outline,
            polylines: vec![crate::template::Polyline {
                points: vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
                closed: true,
            }],
            anchors: Vec::new(),
        }]
    }

    #[test]
    fn guides_are_the_page_and_the_outline_s_bounds_middle_straight_sides_and_bleed_line() {
        let mut els = outline([20.0, 20.0], [250.0, 180.0]);
        // A notch with a short vertical side, too short to count, and a long one.
        els[0].polylines.push(crate::template::Polyline {
            points: vec![[60.0, 40.0], [60.0, 41.0], [90.0, 41.0], [90.0, 70.0]],
            closed: false,
        });
        let g = guides(&els, 3.0, [297.0, 210.0]);
        assert_eq!(
            g.x,
            [0.0, 17.0, 20.0, 90.0, 135.0, 148.5, 250.0, 253.0, 297.0]
        );
        assert_eq!(
            g.y,
            [0.0, 17.0, 20.0, 41.0, 100.0, 105.0, 180.0, 183.0, 210.0]
        );
        let page_only = guides(&anchors(&[[1.0, 1.0]]), 3.0, [297.0, 210.0]);
        assert_eq!(page_only.x, [0.0, 148.5, 297.0]);
        assert_eq!(page_only.y, [0.0, 105.0, 210.0]);
    }

    #[test]
    fn moving_and_resizing_snap_edges_to_the_outline() {
        let close = |a: f32, b: f32| (a - b).abs() < 0.05;
        let none = egui::Modifiers::NONE;
        // Guides x 40, 110, 180 and y 20, 100, 180. The image: (50, 75) to (150, 125).
        let els = outline([40.0, 20.0], [180.0, 180.0]);
        let drag =
            |doc: &mut Document, from: P2, to: P2, m| canvas_drag_in(doc, from, to, m, false, &els);

        // The left edge comes within reach of the outline's and lands on it.
        let mut doc = snap_doc();
        drag(&mut doc, [100.0, 100.0], [91.5, 100.0], none);
        let c = doc.layer(1).unwrap().transform.center;
        assert!(close(c[0], 90.0) && close(c[1], 100.0), "{c:?}");
        // Alt drags freely.
        let mut doc = snap_doc();
        drag(
            &mut doc,
            [100.0, 100.0],
            [91.5, 100.0],
            egui::Modifiers::ALT,
        );
        assert!(close(doc.layer(1).unwrap().transform.center[0], 91.5));

        // The bottom edge stretched near the outline's bottom ends on it, the top staying.
        let mut doc = snap_doc();
        drag(&mut doc, [100.0, 125.0], [100.0, 177.5], none);
        let l = doc.layer(1).unwrap();
        let (lo, hi) = layers_ui::layers_bounds(&doc, &[1]).unwrap();
        assert!(close(lo[1], 75.0) && close(hi[1], 180.0), "{lo:?} {hi:?}");
        assert!(close(l.transform.scale[0], 1.0));

        // Without a template, the page's edges: the right one, 297.
        let mut doc = snap_doc();
        canvas_drag_in(&mut doc, [100.0, 100.0], [245.5, 100.0], none, false, &[]);
        assert!(close(doc.layer(1).unwrap().transform.center[0], 247.0));

        // A corner scaled near the outline's right side ends on it.
        let mut doc = snap_doc();
        drag(&mut doc, [150.0, 125.0], [178.0, 139.0], none);
        let (_, hi) = layers_ui::layers_bounds(&doc, &[1]).unwrap();
        assert!(close(hi[0], 180.0), "{hi:?}");
    }

    #[test]
    fn moving_snaps_center_or_corner_to_anchors() {
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        let (none, alt) = (egui::Modifiers::NONE, egui::Modifiers::ALT);
        // The center comes within reach of an anchor and lands on it.
        let mut doc = snap_doc();
        let near_center = anchors(&[[104.0, 102.0]]);
        canvas_drag_in(
            &mut doc,
            [100.0, 100.0],
            [103.0, 101.0],
            none,
            false,
            &near_center,
        );
        let c = doc.layer(1).unwrap().transform.center;
        assert!(close(c, [104.0, 102.0]), "{c:?}");
        // Alt drags freely.
        let mut doc = snap_doc();
        canvas_drag_in(
            &mut doc,
            [100.0, 100.0],
            [103.0, 101.0],
            alt,
            false,
            &near_center,
        );
        let c = doc.layer(1).unwrap().transform.center;
        assert!(close(c, [103.0, 101.0]), "{c:?}");
        // A corner snaps too, moving the whole image with it.
        let mut doc = snap_doc();
        let near_corner = anchors(&[[155.0, 76.0]]);
        canvas_drag_in(
            &mut doc,
            [100.0, 100.0],
            [104.0, 100.0],
            none,
            false,
            &near_corner,
        );
        let l = doc.layer(1).unwrap();
        assert!(close(l.corners()[1], [155.0, 76.0]), "{:?}", l.corners());
        assert!(close(l.transform.center, [105.0, 101.0]));
        // So does the middle of an edge: the right one, at (150, 100).
        let mut doc = snap_doc();
        let near_edge = anchors(&[[156.0, 99.0]]);
        canvas_drag_in(
            &mut doc,
            [100.0, 100.0],
            [104.0, 100.0],
            none,
            false,
            &near_edge,
        );
        let c = doc.layer(1).unwrap().transform.center;
        assert!(close(c, [106.0, 99.0]), "{c:?}");
        // Out of reach, nothing snaps.
        let mut doc = snap_doc();
        let far = anchors(&[[130.0, 100.0]]);
        canvas_drag_in(&mut doc, [100.0, 100.0], [103.0, 101.0], none, false, &far);
        let c = doc.layer(1).unwrap().transform.center;
        assert!(close(c, [103.0, 101.0]), "{c:?}");
    }

    #[test]
    fn resizing_and_distorting_snap_the_dragged_corner() {
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        let none = egui::Modifiers::NONE;
        // Scaling around the center: the corner lands on an anchor on its diagonal.
        let mut doc = snap_doc();
        let on_diagonal = anchors(&[[160.0, 130.0]]);
        canvas_drag_in(
            &mut doc,
            [150.0, 125.0],
            [158.0, 129.0],
            none,
            false,
            &on_diagonal,
        );
        let l = doc.layer(1).unwrap();
        assert!(close(l.corners()[2], [160.0, 130.0]), "{:?}", l.corners());
        assert!(
            close(l.transform.scale, [1.2, 1.2]),
            "{:?}",
            l.transform.scale
        );
        // Distorting: the corner lands exactly on the anchor, the others stay.
        let mut doc = snap_doc();
        let anchor = anchors(&[[160.0, 81.0]]);
        canvas_drag_in(&mut doc, [150.0, 75.0], [158.0, 80.0], none, true, &anchor);
        let c = doc.layer(1).unwrap().corners();
        assert!(close(c[1], [160.0, 81.0]), "{c:?}");
        assert!(
            close(c[0], [50.0, 75.0]) && close(c[2], [150.0, 125.0]),
            "{c:?}"
        );
    }

    #[test]
    fn edge_handles_stretch_one_axis() {
        let layer = |rotation: f32| {
            let mut l = Layer::new(1, "l".into(), "x.png".into());
            l.size_px = [100, 50];
            l.transform.center = [100.0, 100.0];
            l.transform.rotation_deg = rotation;
            Document {
                layers: vec![Node::Layer(Box::new(l))],
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

        // Alt stretches both sides around the center.
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
    fn ctrl_swaps_resize_and_distort_on_corners() {
        let doc = || {
            let mut l = Layer::new(1, "l".into(), "x.png".into());
            l.size_px = [100, 50];
            l.transform.center = [100.0, 100.0];
            Document {
                layers: vec![Node::Layer(Box::new(l))],
                ..Default::default()
            }
        };
        let ctrl = egui::Modifiers::COMMAND;
        let none = egui::Modifiers::NONE;
        // Drag the bottom-right corner (150, 125) outwards.
        let (from, to) = ([150.0, 125.0], [160.0, 135.0]);
        let distorted = |d: &Document| d.layer(1).unwrap().transform.is_distorted();
        let scaled = |d: &Document| d.layer(1).unwrap().transform.scale != [1.0, 1.0];

        for (mods, mode, want_distort) in [
            (none, false, false), // plain drag resizes
            (ctrl, false, true),  // Ctrl distorts
            (none, true, true),   // Distort on: plain drag distorts
            (ctrl, true, false),  // Distort on: Ctrl goes back to resizing
        ] {
            let mut d = doc();
            canvas_drag_in(&mut d, from, to, mods, mode, &[]);
            assert_eq!(
                distorted(&d),
                want_distort,
                "ctrl {} mode {mode}",
                mods.command
            );
            assert_eq!(
                scaled(&d),
                !want_distort,
                "ctrl {} mode {mode}",
                mods.command
            );
        }
    }

    #[test]
    fn ctrl_alt_on_a_handle_resizes_a_shape_without_stretching_its_paint() {
        use crate::shape::ShapeKind;
        let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
        let ctx = egui::Context::default();
        let mut doc = Document::default();
        let mut l =
            crate::shape::new_canvas_layer(1, ShapeKind::Rectangle, [50.0, 75.0], [150.0, 125.0]);
        let line = [[60.0, 100.0, 1.0], [90.0, 100.0, 1.0]];
        let pencil = crate::paint::Tool::Pencil;
        assert!(crate::shape::add_stroke(
            &mut l,
            &line,
            &crate::paint::Pen::new(pencil, 2.0, [0; 4])
        ));
        doc.layers.push(Node::Layer(Box::new(l)));
        crate::shape::sync(&ctx, &mut doc, true);
        let ends = |doc: &Document| -> Vec<P2> {
            let l = doc.layer(1).unwrap();
            l.shape.as_ref().unwrap().strokes[0]
                .points
                .iter()
                .map(|p| crate::shape::to_page(l, [p[0], p[1]]).unwrap())
                .collect()
        };
        let before = ends(&doc);
        let ctrl_alt = egui::Modifiers::COMMAND | egui::Modifiers::ALT;

        // The right edge's handle dragged out with Ctrl+Alt: wider, the paint in place.
        canvas_drag_in(
            &mut doc,
            [150.0, 100.0],
            [200.0, 100.0],
            ctrl_alt,
            false,
            &[],
        );
        crate::shape::sync(&ctx, &mut doc, true);
        let size = |doc: &Document| doc.layer(1).unwrap().shape.as_ref().unwrap().size;
        assert!(close(size(&doc), [150.0, 50.0]), "{:?}", size(&doc));
        for (a, b) in before.iter().zip(&ends(&doc)) {
            assert!(close(*a, *b), "{before:?} {:?}", ends(&doc));
        }
        // Only for that drag: the next one stretches it, the shape's setting.
        let shape = doc.layer(1).unwrap().shape.as_ref().unwrap();
        assert!(shape.stretch_paint);
        canvas_drag_in(
            &mut doc,
            [200.0, 100.0],
            [350.0, 100.0],
            egui::Modifiers::NONE,
            false,
            &[],
        );
        crate::shape::sync(&ctx, &mut doc, true);
        let after = ends(&doc);
        assert!(close(size(&doc), [300.0, 50.0]), "{:?}", size(&doc));
        let len = |e: &[P2]| e[e.len() - 1][0] - e[0][0];
        assert!((len(&after) - 2.0 * len(&before)).abs() < 0.1, "{after:?}");
    }

    #[test]
    fn the_paint_cursor_leaves_the_tool_bar_alone() {
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        state.brushes.tool = Some(crate::paint::Tool::Brush);
        let mut doc = Document {
            layers: vec![Node::Layer(Box::new(crate::shape::new_layer(
                1,
                crate::shape::ShapeKind::Rectangle,
                [100.0, 100.0],
            )))],
            ..Default::default()
        };
        let mut selected = Some(1);
        let mut frame = |state: &mut CanvasState, time: f64, at: Pos2| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events: vec![Event::PointerMoved(at)],
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                crate::shape::sync(ui.ctx(), &mut doc, true);
                show(ui, &mut doc, &[], &mut selected, state);
            });
            out.textures_delta.clear();
            out.platform_output.cursor_icon
        };
        // The tool bar needs a few frames to size and show itself.
        for k in 0..4 {
            frame(&mut state, k as f64 * 0.02, pos2(600.0, 500.0));
        }
        assert_eq!(
            frame(&mut state, 0.1, pos2(600.0, 500.0)),
            CursorIcon::Crosshair
        );
        // Over its pencil, at the canvas' top left.
        assert_ne!(
            frame(&mut state, 0.2, pos2(48.0, 20.0)),
            CursorIcon::Crosshair
        );
    }

    #[test]
    fn canvas_cursors_stay_on_the_canvas() {
        let ctx = egui::Context::default();
        let mut state = CanvasState::default();
        let mut doc = Document::default();
        let mut selected = None;
        let mut text = String::from("text");
        let mut frame = |state: &mut CanvasState, time: f64, events: Vec<Event>| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                // A panel with a text field at the left, drawn first as in the app.
                egui::Panel::left("side").exact_size(200.0).show(ui, |ui| {
                    ui.add_sized(vec2(180.0, 40.0), egui::TextEdit::singleline(&mut text))
                });
                egui::CentralPanel::default().show(ui, |ui| {
                    show(ui, &mut doc, &[], &mut selected, state);
                });
            });
            out.textures_delta.clear();
            out.platform_output.cursor_icon
        };
        let moved = |p| vec![Event::PointerMoved(p)];
        let button = |p, pressed| Event::PointerButton {
            pos: p,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let canvas = pos2(600.0, 400.0);
        let field = pos2(60.0, 25.0);
        for k in 0..3 {
            frame(&mut state, k as f64 * 0.02, moved(canvas));
        }
        // A drag on the empty canvas pans: a closed hand, while on the canvas.
        frame(&mut state, 0.1, vec![button(canvas, true)]);
        frame(&mut state, 0.15, moved(canvas + vec2(15.0, 0.0)));
        let pan = frame(&mut state, 0.2, moved(canvas + vec2(30.0, 0.0)));
        assert_eq!(pan, CursorIcon::Grabbing);
        // Carried over the text field: not the canvas' hand any more.
        assert_eq!(frame(&mut state, 0.3, moved(field)), CursorIcon::Default);
        // Once let go: the text field's own.
        frame(&mut state, 0.4, vec![button(field, false)]);
        assert_eq!(frame(&mut state, 0.5, moved(field)), CursorIcon::Text);
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
    fn new_things_go_in_view() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
        let mut state = CanvasState {
            rect,
            ..Default::default()
        };
        state.camera.fit(rect, [297.0, 210.0]);
        let center = [148.5, 105.0];
        assert_eq!(state.in_view(center), center, "the whole page shows");
        // Zoomed in on the top left corner: the page's center is out of sight.
        state
            .camera
            .zoom_at(rect, state.camera.to_screen(rect, [20.0, 20.0]), 8.0);
        let p = state.in_view(center);
        let middle = state.camera.to_mm(rect, rect.center());
        assert!((p[0] - middle[0]).abs() < 1e-3 && (p[1] - middle[1]).abs() < 1e-3);
        assert!(p[0] < 60.0 && p[1] < 60.0, "{p:?}");
        // Before the canvas was ever shown, nothing changes.
        assert_eq!(CanvasState::default().in_view(center), center);
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
    fn fit_centers_page() {
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
