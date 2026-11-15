//! The artwork canvas: pan/zoom camera, page, layers, template elements and selection gizmo.

pub mod points;
pub mod text;

mod draw;
mod handles;
mod rulers;
mod snap;
mod tools;
pub use draw::element_colors;
use draw::paint;
use handles::{
    corners_of, edge_axis, edge_handles, group_box, group_rotate_handle, handles, layer_to_select,
    len, rotate_vec, topmost_hit,
};
pub use rulers::clear_guides;
use rulers::{
    guide_at, guide_cursor, guide_dialog, guide_line, guide_targets, paint_rulers, ruler_bands,
};
pub(crate) use snap::{Guides, guides, nearest_snap};
use snap::{snap_axis, snap_points};
pub use tools::{TOOLS, active_tool, can_paint, clear_paint, has_paint};
use tools::{
    canvas_frames, eyedropper_input, paint_brush, paint_dropper_sample, paint_input, tool_bar,
};

use egui::epaint::Vertex;
use egui::{
    Color32, CursorIcon, Event, Key, Mesh, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2,
    Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::document::{Affine, Document, Guide, Homography, Layer, Node, P2, Transform, is_convex};
use crate::i18n::tr;
use crate::render::{self, SceneNode};
use crate::render::{Cuts, PlacedElement};
use crate::template::ElementKind;
pub use crate::ui::canvas::points::{Drawing, PointEdit};
use crate::ui::layers;

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
/// Width of the rulers along the canvas' top and left edges, in screen points.
const RULER: f32 = 20.0;
/// Shortest distance between two numbered marks on a ruler, in screen points.
const RULER_LABEL_SPACING: f32 = 50.0;
/// How close (screen points) the pointer must come to a guide to grab it.
const GUIDE_GRAB: f32 = 4.0;
/// Color of the guides: cyan, apart from the template element colors and the accent.
const GUIDE_COLOR: Color32 = Color32::from_rgb(0, 180, 216);

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
    /// Turns the layer in 3D (Ctrl+drag on it): up and down tilt its top, left and right
    /// its sides.
    Tilt,
    /// Turns a group's layers around this point (page mm), its middle.
    RotateGroup(P2),
}

/// Degrees of 3D tilt per screen pixel dragged.
const TILT_DEG_PER_PX: f32 = 0.5;

/// Middles of the image edges in unit-square coordinates: top, right, bottom, left.
const EDGE_MIDS: [P2; 4] = [[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]];

/// A drag in progress on the canvas.
struct Drag {
    kind: DragKind,
    /// Layers being moved with their starting placement. Scale and rotate use only one.
    layers: Vec<(u64, Transform)>,
    start_mouse: P2,
    /// Moving a layer whose position is linked: it stays linked until released. Released
    /// within `LINK_HOLD_RADIUS` of where it started (`held`), it goes back; farther, it
    /// is unlinked.
    linked: bool,
    /// The linked layer is within `LINK_HOLD_RADIUS` of where it started.
    held: bool,
}

/// How far a layer whose position is linked can be dragged and released without
/// unlinking it, in screen points.
const LINK_HOLD_RADIUS: f32 = 40.0;

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
    /// Per template element, in the template's order: lines shown, and their color
    /// (the element's own, set when the template is loaded).
    pub element_show: Vec<bool>,
    pub element_color: Vec<Color32>,
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
    /// Image centers, corners and edge middles snap to the centers of the template's
    /// shapes while dragged. Holding Alt turns it off for the drag.
    pub snap_to_centers: bool,
    /// The center the dragged image snapped to this frame, in page mm, to highlight it.
    snapped: Option<P2>,
    /// A layer just dropped on a template point: the app offers to link its position
    /// to it, and takes this.
    pub link_offer: Option<LinkOffer>,
    /// Edges and centers of layers snap to the page's and the template outline's edges
    /// while moved or resized. Holding Alt turns it off for the drag.
    pub snap_to_edges: bool,
    /// The guides the dragged layer snapped to this frame: a vertical line's x and a
    /// horizontal line's y, in page mm, to show them.
    snapped_guides: [Option<f32>; 2],
    /// Show rulers along the canvas' top and left edges; guides are dragged out of them.
    pub show_rulers: bool,
    /// Show the project's guides. Hidden guides are neither snapped to nor moved.
    pub show_guides: bool,
    /// Edges and centers of layers snap to the guides while moved or resized, and a
    /// dragged guide snaps to the layers, the page and the outline. Holding Alt turns it
    /// off for the drag.
    pub snap_to_guides: bool,
    /// The guides can't be moved, deleted or edited on the canvas.
    pub lock_guides: bool,
    /// The guide being dragged: its index in `Document::guides`.
    guide_drag: Option<usize>,
    /// The guide under the pointer, to highlight it.
    guide_hover: Option<usize>,
    /// The guide whose position is being typed, after a double click on it.
    pub guide_edit: Option<GuideEdit>,
    /// Another version of the template, shown left of a divider to compare it with the
    /// project's (right of it); set by the app.
    pub compare: Option<Compare>,
    /// Where the divider stands, as a fraction of the canvas' width.
    compare_split: f32,
    /// The other version's cut preview and safe-area guide, kept apart from the project's.
    compare_cut_preview: Option<CutPreview>,
    compare_safe_area: Option<(u64, Vec<Vec<P2>>)>,
    /// Where the canvas is on screen, rulers left out.
    pub(crate) rect: Rect,
    /// Where the canvas is on screen, rulers included.
    full_rect: Rect,
    drag: Option<Drag>,
    /// A left drag that started on empty canvas pans the view.
    panning: bool,
    /// Typing in a text layer on the canvas, after a double click on it.
    pub typing: Option<text::Typing>,
    /// Editing the points of a line, polyline or polygon, after a double click on it.
    pub points: Option<PointEdit>,
    /// A line, polyline or polygon being drawn point by point, after picking it in the
    /// Shape menu.
    pub drawing: Option<Drawing>,
    /// The paint tool picked last and its settings; painting in the selected closed
    /// shape replaces selecting and moving. The tool is kept while no such shape is
    /// selected, and in hand again once one is (see [`active_tool`]).
    pub brushes: crate::document::paint::Brushes,
    /// The eyedropper is in hand: a click on the canvas takes the artwork's color there
    /// (see [`eyedropper_input`]). The paint tool picked last is kept.
    pub eyedropper: bool,
    /// The colors added with the eyedropper (Ctrl+click), last first: first among the
    /// colors in use, in the Colors in use panel and in every color picker.
    pub picked: Vec<[u8; 4]>,
    /// Where the eyedropper was on the page last, and the color there.
    dropper_sample: Option<(P2, Option<[u8; 4]>)>,
    /// Each command's shortcut as text, for the tool bar's tooltips; set by the app.
    pub keys: HashMap<crate::ui::shortcuts::Command, String>,
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
            element_show: Vec::new(),
            element_color: Vec::new(),
            preview_cuts: true,
            cuts: Cuts::default(),
            preview_bleed: true,
            preview_safe_area: true,
            cut_opacity: 0.8,
            cut_preview: None,
            safe_margin_mm: 0.0,
            safe_area: None,
            cursor_mm: None,
            snap_to_centers: true,
            snapped: None,
            link_offer: None,
            snap_to_edges: true,
            snapped_guides: [None; 2],
            show_rulers: true,
            show_guides: true,
            snap_to_guides: true,
            lock_guides: false,
            guide_drag: None,
            guide_hover: None,
            guide_edit: None,
            compare: None,
            compare_split: 0.5,
            compare_cut_preview: None,
            compare_safe_area: None,
            rect: Rect::NOTHING,
            full_rect: Rect::NOTHING,
            drag: None,
            panning: false,
            typing: None,
            points: None,
            drawing: None,
            group_previews: HashMap::new(),
            background: None,
            brushes: Default::default(),
            eyedropper: false,
            picked: Vec::new(),
            dropper_sample: None,
            keys: HashMap::new(),
            painting: Vec::new(),
            paint_preview: None,
            pen_pressure: None,
        }
    }
}

/// A layer dropped on a template shape's center, whose position can be linked to it.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkOffer {
    pub layer: u64,
    /// The shape's id in the template.
    pub point: String,
    /// The layer's center less the shape's, in mm.
    pub offset: P2,
}

/// Another version of the template, compared with the project's on the canvas.
pub struct Compare {
    /// Its elements, placed on the page.
    pub elements: Vec<PlacedElement>,
    /// Their line colors, set in that template (see `element_colors`).
    pub colors: Vec<Color32>,
    /// What each side shows, written at the top of the canvas: left, then right.
    pub labels: [String; 2],
    /// It is shown left of the divider (else right): the older version left, the newer
    /// right.
    pub on_left: bool,
}

/// The guide whose position is being typed: its index in `Document::guides`, and the
/// value typed so far, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideEdit {
    pub index: usize,
    pub at: f32,
    /// Give the field the keyboard focus on the next frame.
    focus: bool,
}

/// The View panel's display settings, kept with the app's preferences (not the
/// project): whether the canvas shows the template's lines, how it previews the cuts,
/// the bleed and the safe area, and whether it shows the rulers and the guides.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Previews {
    template: bool,
    cuts: bool,
    cut_opacity: f32,
    bleed: bool,
    safe_area: bool,
    safe_margin_mm: f32,
    rulers: bool,
    guides: bool,
}

impl Default for Previews {
    fn default() -> Self {
        let s = CanvasState::default();
        Self {
            template: s.show_template,
            cuts: s.preview_cuts,
            cut_opacity: s.cut_opacity,
            bleed: s.preview_bleed,
            safe_area: s.preview_safe_area,
            safe_margin_mm: s.safe_margin_mm,
            rulers: s.show_rulers,
            guides: s.show_guides,
        }
    }
}

/// Where they are kept.
const PREVIEWS_KEY: &str = "canvas_previews";

impl CanvasState {
    /// The layers whose linked position a drag holds until it is released:
    /// [`crate::document::variables::apply`] leaves them alone.
    pub fn held_layers(&self) -> Vec<u64> {
        match &self.drag {
            Some(d) if d.linked => d.layers.iter().map(|(id, _)| *id).collect(),
            _ => Vec::new(),
        }
    }

    /// The canvas with the display settings saved by a previous session.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let p: Previews = storage
            .and_then(|s| eframe::get_value(s, PREVIEWS_KEY))
            .unwrap_or_default();
        Self {
            show_template: p.template,
            preview_cuts: p.cuts,
            cut_opacity: p.cut_opacity.clamp(0.0, 1.0),
            preview_bleed: p.bleed,
            preview_safe_area: p.safe_area,
            safe_margin_mm: p
                .safe_margin_mm
                .clamp(0.0, crate::export::marks::MAX_MARGIN_MM),
            show_rulers: p.rulers,
            show_guides: p.guides,
            ..Default::default()
        }
    }

    /// Keeps its display settings for the next session.
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let p = Previews {
            template: self.show_template,
            cuts: self.preview_cuts,
            cut_opacity: self.cut_opacity,
            bleed: self.preview_bleed,
            safe_area: self.preview_safe_area,
            safe_margin_mm: self.safe_margin_mm,
            rulers: self.show_rulers,
            guides: self.show_guides,
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
        text::begin(&mut self.typing, layer.id, 0, n);
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

/// Draws the canvas and handles its input. Returns true if the document changed.
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    elements: &[PlacedElement],
    selected: &mut Option<u64>,
    state: &mut CanvasState,
) -> bool {
    // The rulers take a band along the top and left edges.
    let full = ui.available_rect_before_wrap();
    let rect = if state.show_rulers {
        Rect::from_min_max(full.min + vec2(RULER, RULER), full.max)
    } else {
        full
    };
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let mut changed = false;
    // The cursor as the panels and windows drawn so far left it: the canvas' own only
    // show over the canvas itself (see the end).
    let cursor_before = ui.ctx().output(|o| o.cursor_icon);

    // Keep the view centered when the window resizes, and fit on demand.
    if state.fit_pending || !state.rect.is_positive() {
        state.camera.fit(rect, doc.page_mm());
        state.fit_pending = false;
    } else if full == state.full_rect && rect != state.rect {
        // The rulers were shown or hidden: the page stays where it is on screen.
        state.camera.pan -= rect.min - state.rect.min;
    } else if state.rect.size() != rect.size() {
        state.camera.pan += (rect.size() - state.rect.size()) / 2.0;
    }
    state.rect = rect;
    state.full_rect = full;

    // The comparison's divider, dragged across the canvas.
    let mut on_divider = false;
    if state.compare.is_some() {
        let x = rect.left() + rect.width() * state.compare_split;
        // Along the line, and a dead area around the handle wider than it: there, the
        // divider takes the press (click or drag), never what is drawn under it.
        let line = Rect::from_x_y_ranges(x - DIVIDER_GRAB..=x + DIVIDER_GRAB, rect.y_range());
        let handle = Rect::from_center_size(
            pos2(x, rect.center().y),
            Vec2::splat(2.0 * HANDLE_DEAD_ZONE),
        );
        let id = ui.scope_id().with("compare_divider");
        let r = ui.interact(line, id, Sense::click_and_drag())
            | ui.interact(handle, id.with("handle"), Sense::click_and_drag());
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            state.compare_split = ((p.x - rect.left()) / rect.width()).clamp(0.02, 0.98);
        }
        if r.hovered() || r.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            on_divider = true;
        }
    } else {
        state.compare_cut_preview = None;
        state.compare_safe_area = None;
    }

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
    // Ctrl makes the corner handles distort instead of resizing while held.
    let distorting = ui.input(|i| i.modifiers.command) && !flip_paint;
    // Snapping reach in mm, when on and the template has shapes with a center.
    let anchors: Vec<P2> = elements.iter().flat_map(|e| &e.centers).copied().collect();
    let snap_reach =
        (state.snap_to_centers && !alt && !anchors.is_empty()).then_some(SNAP_RADIUS / cam.zoom);
    let mut snapped = None;
    // The lines layers snap to: the page's and the outline's, and the guides, each when
    // snapping to them is on; and their reach in mm.
    let edges = guides(elements, state.cuts.bleed_mm, doc.page_mm());
    let mut lines = if state.snap_to_edges {
        edges.clone()
    } else {
        Guides::default()
    };
    if state.snap_to_guides && state.show_guides {
        for g in &doc.guides {
            let axis = if g.vertical {
                &mut lines.x
            } else {
                &mut lines.y
            };
            axis.push(g.at);
        }
    }
    let guides = lines.sorted();
    let guide_reach =
        (!alt && !(guides.x.is_empty() && guides.y.is_empty())).then_some(SNAP_RADIUS / cam.zoom);
    let mut snapped_guides = [None; 2];

    // A drag out of a ruler adds a guide: a horizontal one from the top ruler, a vertical
    // one from the left ruler. Hidden guides are shown again.
    if state.show_rulers && state.guide_drag.is_none() {
        let (top, left) = ruler_bands(full, rect);
        for (band, vertical) in [(top, false), (left, true)] {
            let r = ui.interact(band, ui.scope_id().with(("ruler", vertical)), Sense::drag());
            if r.drag_started_by(PointerButton::Primary)
                && let Some(p) = r.interact_pointer_pos()
            {
                let at = cam.to_mm(rect, p)[if vertical { 0 } else { 1 }];
                doc.guides.push(Guide { vertical, at });
                state.guide_drag = Some(doc.guides.len() - 1);
                state.show_guides = true;
                changed = true;
            }
        }
    }
    // The guide under a screen point, when guides can be grabbed.
    let grab_guides = state.show_guides && !state.lock_guides;
    let guide_under =
        |doc: &Document, p: Pos2| grab_guides.then(|| guide_at(doc, cam, rect, p)).flatten();

    // Which handle (if any) of the selected layer is under a screen point. A shape whose
    // points are being edited shows those instead.
    let editing_points = state.points.as_ref().map(|e| e.layer);
    let handle_at = |doc: &Document, sel: Option<u64>, p: Pos2| -> Option<(u64, DragKind)> {
        let id = sel?;
        if doc.effectively_locked(id) || !doc.effectively_visible(id) || editing_points == Some(id)
        {
            return None;
        }
        // A group: the rotation handle above its box (none in the template editor).
        if let Some(Node::Group(_)) = doc.node(id) {
            if doc.edited_template.is_some() {
                return None;
            }
            let (_, rot) = group_rotate_handle(group_box(doc, &cam, rect, id)?);
            return (rot.distance(p) <= HANDLE_RADIUS * 1.5)
                .then(|| Some((id, DragKind::RotateGroup(layers::group_middle(doc, id)?))))
                .flatten();
        }
        let layer = doc
            .layer(id)
            .filter(|l| !crate::document::background::is_fixed(l))?;
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
    if (state.eyedropper || active_tool(doc, *selected, &state.brushes).is_some())
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(Key::Escape))
    {
        state.brushes.tool = None;
        state.eyedropper = false;
    }
    let active = active_tool(doc, *selected, &state.brushes).filter(|_| !state.eyedropper);
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
    // The eyedropper takes the left button too.
    let dropper = state.eyedropper && !space && doc.edited_template.is_none();
    if dropper {
        state.typing = None;
        state.drawing = None;
        state.points = None;
        eyedropper_input(ui, doc, state, cam, rect, &response);
    } else {
        state.dropper_sample = None;
    }

    // Typing on the canvas takes the clicks and drags on its text.
    let on_handle = hover.is_some_and(|p| handle_at(doc, *selected, p).is_some());
    let typing = text::input(
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
    let keys = points::Keys {
        shift,
        alt,
        anchors: &anchors,
        snap_reach,
    };
    let drawn = points::draw_input(
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
    let mut points = points::input(
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

    if !space && !typing.took && !points.took && paint_tool.is_none() && !dropper {
        if response.drag_started_by(PointerButton::Primary) {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos());
            // A guide is grabbed unless a handle of the selected layer is there.
            let on_guide = origin
                .filter(|&o| handle_at(doc, *selected, o).is_none())
                .and_then(|o| guide_under(doc, o));
            if let Some(i) = on_guide {
                state.guide_drag = Some(i);
                state.panning = false;
            } else if let Some(origin) = origin {
                let m = cam.to_mm(rect, origin);
                // Ctrl+drag on the selected layer turns it in 3D (not the template's
                // reference, whose tilt isn't saved).
                let tilt = ui.input(|i| i.modifiers.command && !i.modifiers.alt);
                let tiltable = |id: u64| {
                    doc.layer(id)
                        .is_some_and(|l| !crate::document::background::is_fixed(l))
                        && !doc.effectively_locked(id)
                        && !crate::template::edit::is_reference(doc, id)
                };
                let (ids, kind) = match handle_at(doc, *selected, origin) {
                    Some((id, kind @ DragKind::RotateGroup(_))) => {
                        (layers::movable_layers(doc, id), kind)
                    }
                    Some((id, kind)) => (vec![id], kind),
                    None => match (topmost_hit(doc, m), *selected) {
                        (Some(hit), Some(sel)) if tilt && hit == sel && tiltable(sel) => {
                            (vec![sel], DragKind::Tilt)
                        }
                        // Dragging a member of the selected group moves the whole group.
                        (Some(hit), Some(sel)) if hit != sel && doc.is_within(hit, sel) => {
                            (layers::movable_layers(doc, sel), DragKind::Move)
                        }
                        // A locked layer is selected but stays put: the drag pans.
                        // A background stays on the outline: the drag pans.
                        (hit, _) => {
                            *selected = layer_to_select(doc, m);
                            let hit = hit.filter(|&id| {
                                doc.layer(id)
                                    .is_none_or(|l| !crate::document::background::is_fixed(l))
                            });
                            (hit.into_iter().collect(), DragKind::Move)
                        }
                    },
                };
                // A text following a shape moves with it: the shape is moved.
                let mut ids = ids;
                if kind == DragKind::Move {
                    let shapes: Vec<u64> = ids
                        .iter()
                        .filter_map(|&id| {
                            doc.layer(id)
                                .and_then(crate::document::text::followed_shape)
                        })
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
                            crate::document::shape::flip_paint_once(l);
                        }
                    }
                }
                // A linked position is held a moment before the drag unlinks it.
                let linked = kind == DragKind::Move
                    && layers.iter().any(|(id, _)| {
                        doc.layer(*id).is_some_and(|l| {
                            l.bindings
                                .contains_key(&crate::document::variables::Field::X)
                                || l.bindings
                                    .contains_key(&crate::document::variables::Field::Y)
                        })
                    });

                state.drag = (!layers.is_empty()).then_some(Drag {
                    kind,
                    layers,
                    start_mouse: m,
                    linked,
                    held: linked,
                });
                // Nothing grabbed: this drag pans the view.
                state.panning = state.drag.is_none();
            }
        } else if response.clicked_by(PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
            && handle_at(doc, *selected, p).is_none()
            && guide_under(doc, p).is_none()
        {
            *selected = layer_to_select(doc, cam.to_mm(rect, p));
        }
        // A double click on a guide asks for its position.
        let guide_clicked = response
            .double_clicked_by(PointerButton::Primary)
            .then(|| response.interact_pointer_pos())
            .flatten()
            .filter(|&p| handle_at(doc, *selected, p).is_none())
            .and_then(|p| guide_under(doc, p));
        if let Some(i) = guide_clicked {
            state.guide_edit = Some(GuideEdit {
                index: i,
                at: doc.guides[i].at,
                focus: true,
            });
        }
        // A double click on a text layer starts typing in it, where it was clicked. Text
        // on a path is typed in Properties.
        if response.double_clicked_by(PointerButton::Primary)
            && guide_clicked.is_none()
            && let Some(p) = response.interact_pointer_pos()
            && let Some(layer) = selected.and_then(|id| doc.layer(id))
            && crate::document::text::is_straight(layer)
            && !doc.effectively_locked(layer.id)
        {
            let i = text::index_at(layer, cam.to_mm(rect, p));
            text::begin(&mut state.typing, layer.id, i, i);
        }
        // On a line, polyline or polygon, it shows its points to edit them.
        if response.double_clicked_by(PointerButton::Primary)
            && guide_clicked.is_none()
            && let Some(id) = *selected
            && state.points.is_none()
            && points::can_edit(doc, id)
        {
            state.points = Some(PointEdit::new(id));
        }
    }
    changed |= text::field(ui, doc, &mut state.typing, cam, rect);

    // A guide being dragged follows the pointer; let go off the canvas, it is deleted.
    if let Some(i) = state.guide_drag {
        let pointer = ui.input(|i| i.pointer.latest_pos());
        if i >= doc.guides.len() {
            state.guide_drag = None;
        } else if ui.input(|i| i.pointer.primary_down()) {
            let a = if doc.guides[i].vertical { 0 } else { 1 };
            if let Some(p) = pointer {
                let mut at = cam.to_mm(rect, p)[a];
                // On the canvas, it snaps to the layers, the page and the outline.
                let reach = (state.snap_to_guides && !alt && rect.contains(p))
                    .then_some(SNAP_RADIUS / cam.zoom);
                let target =
                    reach.and_then(|r| snap_axis(&guide_targets(doc, &edges, a), &[at], r));
                match target {
                    Some((_, t)) => {
                        at = t;
                        snapped_guides[a] = Some(t);
                    }
                    None => at = (at * 10.0).round() / 10.0,
                }
                if doc.guides[i].at != at {
                    doc.guides[i].at = at;
                    changed = true;
                }
            }
        } else {
            if pointer.is_none_or(|p| !rect.contains(p)) {
                doc.guides.remove(i);
                changed = true;
            }
            state.guide_drag = None;
        }
    }
    // The guide under the pointer, or the one dragged, is highlighted.
    state.guide_hover = state.guide_drag.or_else(|| {
        hover
            .filter(|_| {
                state.drag.is_none() && paint_tool.is_none() && !dropper && state.drawing.is_none()
            })
            .filter(|&p| !space && !points.over_handle && handle_at(doc, *selected, p).is_none())
            .and_then(|p| guide_under(doc, p))
    });

    // A linked position near where it started: a release there puts it back, with a
    // warning meanwhile; farther, a release unlinks it.
    if let Some(drag) = &mut state.drag
        && drag.linked
    {
        drag.held = drag.layers.iter().all(|(id, start)| {
            doc.layer(*id).is_none_or(|l| {
                let c = l.transform.center;
                let d = [c[0] - start.center[0], c[1] - start.center[1]];
                len(d) * cam.zoom <= LINK_HOLD_RADIUS
            })
        });
        if drag.held
            && let Some(p) = ui.ctx().pointer_hover_pos()
        {
            linked_drag_hint(ui, p);
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
                && let Some((lo, hi)) = layers::layers_bounds(doc, &ids)
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
            && let DragKind::RotateGroup(pivot) = drag.kind
        {
            // Turned by the angle the pointer went around the middle; Shift: 15° steps.
            let m = cam.to_mm(rect, p);
            let (v0, v) = (
                [
                    drag.start_mouse[0] - pivot[0],
                    drag.start_mouse[1] - pivot[1],
                ],
                [m[0] - pivot[0], m[1] - pivot[1]],
            );
            let mut delta = (v[1].atan2(v[0]) - v0[1].atan2(v0[0])).to_degrees();
            if shift {
                delta = (delta / 15.0).round() * 15.0;
            }
            for (id, start) in &drag.layers {
                if let Some(layer) = doc.layer_mut(*id) {
                    layer.transform = layers::turned_about(*start, pivot, delta);
                }
            }
            ui.ctx().set_cursor_icon(CursorIcon::Alias);
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
                // Handled above, for all its layers.
                DragKind::Move | DragKind::RotateGroup(_) => {}
                DragKind::Tilt => {
                    // Screen pixels dragged, in the layer's own frame: up is toward its
                    // top, whatever its rotation and flips.
                    let d = [
                        (m[0] - drag.start_mouse[0]) * cam.zoom,
                        (m[1] - drag.start_mouse[1]) * cam.zoom,
                    ];
                    let r = -s.rotation_deg.to_radians();
                    let mut dx = d[0] * r.cos() - d[1] * r.sin();
                    let mut dy = d[0] * r.sin() + d[1] * r.cos();
                    if s.flip_x {
                        dx = -dx;
                    }
                    if s.flip_y {
                        dy = -dy;
                    }
                    // Dragged up, the top goes away; right, the right side.
                    let mut x = s.tilt_x_deg - dy * TILT_DEG_PER_PX;
                    let mut y = s.tilt_y_deg + dx * TILT_DEG_PER_PX;
                    if shift {
                        x = (x / 15.0).round() * 15.0;
                        y = (y / 15.0).round() * 15.0;
                    }
                    t.tilt_x_deg = crate::document::variables::clamp_tilt(x);
                    t.tilt_y_deg = crate::document::variables::clamp_tilt(y);
                    ui.ctx().set_cursor_icon(CursorIcon::AllScroll);
                }
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
                                let base = s.tilted_corners(layer.size_px)[i];
                                d[i] = [u[0] - base[0], u[1] - base[1]];
                                snapped = Some(anchor);
                            }
                        }
                        t2.distort = d;
                        // A corner dragged across the others would fold the image.
                        if is_convex(t2.unit_quad(layer.size_px)) {
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
            // Released near where it started: it goes back, still linked. Farther, it
            // stays, and is unlinked once the drag is over (see `held_layers`).
            if drag.held {
                for (id, start) in &drag.layers {
                    if let Some(layer) = doc.layer_mut(*id) {
                        layer.transform = *start;
                    }
                }
            }
            // A single layer moved onto a template point: offer to link it there.
            let held = drag.held;
            let offer = match (drag.kind, drag.layers.as_slice(), state.snapped) {
                _ if held => None,
                (DragKind::Move, [(id, _)], Some(a)) => doc
                    .template_points
                    .iter()
                    .find(|(_, p)| (p[0] - a[0]).abs() < 1e-3 && (p[1] - a[1]).abs() < 1e-3)
                    .zip(doc.layer(*id))
                    .map(|((point, p), layer)| LinkOffer {
                        layer: *id,
                        point: point.clone(),
                        offset: [
                            layer.transform.center[0] - p[0],
                            layer.transform.center[1] - p[1],
                        ],
                    }),
                _ => None,
            };
            state.link_offer = offer.or(state.link_offer.take());
            state.drag = None;
            snapped = None;
            snapped_guides = [None; 2];
        }
    } else if let Some(p) = hover
        && !space
        && !dropper
        && !panning_now
        && !typing.over_text
        && !points.took
        && !points.over_handle
    {
        let hovered_guide = state.guide_hover.and_then(|i| doc.guides.get(i));
        match handle_at(doc, *selected, p) {
            Some((_, DragKind::Rotate | DragKind::RotateGroup(_))) => {
                ui.ctx().set_cursor_icon(CursorIcon::Alias)
            }
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
            None if let Some(g) = hovered_guide => {
                ui.ctx().set_cursor_icon(guide_cursor(g.vertical))
            }
            // Ctrl over the selected layer: a drag turns it in 3D.
            None if ui.input(|i| i.modifiers.command && !i.modifiers.alt)
                && selected.is_some()
                && topmost_hit(doc, cam.to_mm(rect, p)) == *selected
                && selected.is_some_and(|id| doc.layer(id).is_some()) =>
            {
                ui.ctx().set_cursor_icon(CursorIcon::AllScroll)
            }
            None if topmost_hit(doc, cam.to_mm(rect, p)).is_some() => {
                ui.ctx().set_cursor_icon(CursorIcon::Move)
            }
            None => ui.ctx().set_cursor_icon(CursorIcon::Grab),
        }
    }

    state.snapped = snapped;
    state.snapped_guides = snapped_guides;
    paint(ui, doc, elements, *selected, state, rect, distorting);
    // Not in the template editor, which has nothing to paint or pick colors for.
    let paintable = can_paint(doc, *selected);
    if (paintable || doc.edited_template.is_none())
        && tool_bar(ui, state, rect, paintable, has_paint(doc, *selected))
        && paintable
    {
        changed |= clear_paint(doc, *selected);
    }
    if dropper {
        paint_dropper_sample(ui, state, rect, hover);
    }
    text::paint(ui, doc, &state.typing, cam, rect);
    let painter = ui.painter_at(rect);
    let look = points::Look {
        accent: ACCENT,
        background: ui.visuals().extreme_bg_color,
    };
    points::paint(&painter, doc, &state.points, cam, rect, &look);
    points::paint_drawing(&painter, doc, &state.drawing, cam, rect, hover, keys, &look);
    // Outside the canvas, or over its tool bar, a window or a popup, even during a drag
    // that started on it: the cursor of what is there.
    if !ui.rect_contains_pointer(rect) {
        ui.ctx().output_mut(|o| o.cursor_icon = cursor_before);
    }
    if let Some(g) = state.guide_drag.and_then(|i| doc.guides.get(i)) {
        ui.ctx().set_cursor_icon(guide_cursor(g.vertical));
    }
    if on_divider {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if state.show_rulers {
        paint_rulers(ui, state, doc.page_mm(), full, rect);
    }
    changed |= guide_dialog(ui.ctx(), doc, state);
    changed
}

/// Next to the pointer, while a layer whose position is linked is dragged: says that
/// going on unlinks it and that releasing puts it back.
fn linked_drag_hint(ui: &egui::Ui, at: Pos2) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("linked_drag_hint"),
    ));
    let visuals = ui.visuals();
    let galley = painter.layout(
        tr!("LINKED_POSITION_DRAG_HINT").to_owned(),
        egui::FontId::proportional(13.0),
        visuals.text_color(),
        260.0,
    );
    let size = galley.size() + vec2(16.0, 10.0);
    let chip = Rect::from_min_size(at + vec2(16.0, 16.0), size);
    painter.rect(
        chip,
        4.0,
        visuals.extreme_bg_color,
        Stroke::new(1.5, visuals.warn_fg_color),
        StrokeKind::Inside,
    );
    painter.galley(chip.min + vec2(8.0, 5.0), galley, visuals.text_color());
}

/// Half the width of the zone where the comparison's divider is grabbed, in screen
/// points.
const DIVIDER_GRAB: f32 = 6.0;

/// Half the size of the square around the divider's handle (a circle of radius 12)
/// where a press grabs the divider, in screen points.
const HANDLE_DEAD_ZONE: f32 = 24.0;

#[cfg(test)]
mod tests;
