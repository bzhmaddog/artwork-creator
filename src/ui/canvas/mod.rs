//! The artwork canvas: pan/zoom camera, page, layers, template elements and selection gizmo.

pub mod points;
pub mod text;

mod drag;
mod draw;
mod handles;
mod navigate;
mod rulers;
mod snap;
mod tools;
pub use draw::element_colors;
use draw::paint;
use handles::{
    Hot, corners_of, edge_axis, edge_handles, group_box, group_rotate_handle, handles,
    layer_to_select, len, rotate_vec, topmost_hit,
};
pub use rulers::clear_guides;
use rulers::{
    guide_at, guide_cursor, guide_dialog, guide_line, guide_targets, paint_rulers, ruler_input,
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
    /// The selection's handle under the pointer, or being dragged, drawn highlighted.
    hot: Option<Hot>,
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
            hot: None,
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

    navigate::fit(state, doc.page_mm(), full, rect);
    let on_divider = navigate::divider(ui, state, rect);

    // Only when no window, dialog or popup covers the canvas under the pointer.
    let hover = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|_| ui.rect_contains_pointer(rect));
    state.cursor_mm = hover.map(|p| state.camera.to_mm(rect, p));
    if let Some(pos) = hover {
        navigate::wheel(ui, &mut state.camera, rect, pos);
    }
    // Space pans, except while typing, where it is a space.
    let space = state.typing.is_none() && ui.input(|i| i.key_down(Key::Space));
    let panning_now = navigate::pan(ui, state, &response, space, hover);

    let f = drag::Frame::new(ui, doc, elements, state, rect);
    let cam = f.cam;
    let mut snapped = drag::Snapped::default();
    if state.show_rulers && state.guide_drag.is_none() {
        changed |= ruler_input(ui, doc, state, full, rect);
    }

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
    let on_handle = hover.is_some_and(|p| f.handle_at(doc, *selected, p).is_some());
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
        shift: f.shift,
        alt: f.alt,
        anchors: &f.anchors,
        snap_reach: f.snap_reach,
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
    snapped.anchor = points.snapped;

    if !space && !typing.took && !points.took && paint_tool.is_none() && !dropper {
        drag::press_input(ui, doc, selected, state, &response, &f);
    }
    changed |= text::field(ui, doc, &mut state.typing, cam, rect);
    changed |= drag::guide_input(ui, doc, state, &f, &mut snapped);
    // The guide under the pointer, or the one dragged, is highlighted.
    state.guide_hover = state.guide_drag.or_else(|| {
        hover
            .filter(|_| {
                state.drag.is_none() && paint_tool.is_none() && !dropper && state.drawing.is_none()
            })
            .filter(|&p| !space && !points.over_handle && f.handle_at(doc, *selected, p).is_none())
            .and_then(|p| f.guide_under(doc, p))
    });

    drag::hold_linked(ui, doc, state, &f);
    if let Some(d) = &state.drag {
        changed |= drag::update(ui, doc, d, &response, &f, &mut snapped);
        if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            drag::release(doc, state);
            snapped = drag::Snapped::default();
        }
    } else if let Some(p) = hover
        && !space
        && !dropper
        && !panning_now
        && !typing.over_text
        && !points.took
        && !points.over_handle
    {
        drag::hover_cursor(ui, doc, *selected, state.guide_hover, &f, p);
        state.hot = f.hot_at(doc, *selected, p);
    } else if state.drag.is_none() {
        state.hot = None;
    }

    state.snapped = snapped.anchor;
    state.snapped_guides = snapped.guides;
    paint(ui, doc, elements, *selected, state, rect, f.distorting);
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

/// Half the width of the zone where the comparison's divider is grabbed, in screen
/// points.
const DIVIDER_GRAB: f32 = 6.0;

/// Half the size of the square around the divider's handle (a circle of radius 12)
/// where a press grabs the divider, in screen points.
const HANDLE_DEAD_ZONE: f32 = 24.0;

#[cfg(test)]
mod tests;
