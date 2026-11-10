//! Drawing lines, polylines and polygons point by point on the canvas, and editing their
//! points afterwards. Their segments can be Bezier curves.
//!
//! Drawing: after picking one of these kinds in the Shape menu, each click places a
//! sharp point; a press and drag places a smooth one, pulling out its handles. A double
//! click, Enter, Escape or a right click finishes; a click on the first point closes a
//! polyline into a polygon; a line finishes at its second point.
//!
//! Editing: a double click on such a shape (or Edit points in its properties) shows a
//! handle on each point, and the Bezier handles of the point picked last and the facing
//! ones of its neighbors. Dragging a point moves it; dragging a Bezier handle bends the
//! curve, the other handle of a smooth point turning with it unless Alt is held. A
//! double click on a point makes it smooth or sharp; on a segment, it adds a point there.
//! Points snap to the template's anchors. The keyboard deletes or nudges the point
//! picked last. The layer's own handles are hidden meanwhile; Escape, or selecting
//! something else, ends the editing.

use egui::{Color32, CursorIcon, Key, PointerButton, Pos2, Rect, Response, Shape, Stroke, vec2};

use crate::document::{Document, Layer, Node, P2};
use crate::i18n::translate;
use crate::layers_ui;
use crate::shape::{self, Mirror, ShapeKind};
use crate::view::{Camera, nearest_snap};

/// Radius of a point's handle, in screen points.
const POINT_RADIUS: f32 = 6.0;
/// How close to a point's handle a press must be to grab it.
const POINT_REACH: f32 = POINT_RADIUS * 1.8;
/// Radius of the dot at the end of a Bezier handle.
const HANDLE_RADIUS: f32 = 5.0;
/// How close to a segment a double click must be to add a point on it.
const SEGMENT_REACH: f32 = 6.0;
/// Pieces each curved segment is drawn and hit tested with.
const CURVE_STEPS: usize = 32;
/// A press dragged further than this while drawing pulls out handles.
const PULL_MIN: f32 = 4.0;

/// What is being dragged while editing points.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Point(usize),
    /// Handle `which` (0 in, 1 out) of point `i`, and whether the other handle turns
    /// with it.
    Handle {
        i: usize,
        which: usize,
        smooth: bool,
    },
}

/// Editing in progress of a shape's points.
#[derive(Clone, Debug, PartialEq)]
pub struct PointEdit {
    pub layer: u64,
    /// The point picked last, which the keyboard deletes or nudges, and whose Bezier
    /// handles are shown.
    pub selected: Option<usize>,
    dragging: Option<Grab>,
}

impl PointEdit {
    pub fn new(layer: u64) -> Self {
        Self {
            layer,
            selected: None,
            dragging: None,
        }
    }
}

/// A line, polyline or polygon being drawn point by point.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    pub kind: ShapeKind,
    /// A canvas to paint in: without fill or stroke, named so.
    pub canvas: bool,
    /// The layer, once the first point is placed.
    pub layer: Option<u64>,
    /// The point placed by the press being held: dragging pulls out its handles.
    pulling: Option<usize>,
    /// Time and place of the last press, to tell the second press of a double click.
    last_press: Option<(f64, Pos2)>,
    /// Done: the drawing ends once the press that finished it is released, so that
    /// release isn't a click for the canvas.
    done: bool,
}

impl Drawing {
    pub fn new(kind: ShapeKind) -> Self {
        Self {
            kind,
            canvas: false,
            layer: None,
            pulling: None,
            last_press: None,
            done: false,
        }
    }
}

/// The layer, if its points can be edited: a line, polyline or polygon, selected,
/// visible and unlocked.
fn editable(doc: &Document, selected: Option<u64>, id: u64) -> Option<&Layer> {
    doc.layer(id)
        .filter(|l| l.shape.as_ref().is_some_and(|s| s.kind.has_points()))
        .filter(|_| selected == Some(id))
        .filter(|_| !doc.effectively_locked(id) && doc.effectively_visible(id))
}

/// True if a layer's points can be edited on the canvas.
pub fn can_edit(doc: &Document, id: u64) -> bool {
    editable(doc, Some(id), id).is_some()
}

/// Screen positions of the layer's points.
fn screen_points(layer: &Layer, cam: Camera, rect: Rect) -> Vec<Pos2> {
    shape::page_points(layer)
        .into_iter()
        .map(|p| cam.to_screen(rect, p))
        .collect()
}

/// The point whose handle is under a screen position.
fn point_at(layer: &Layer, cam: Camera, rect: Rect, p: Pos2) -> Option<usize> {
    screen_points(layer, cam, rect)
        .into_iter()
        .enumerate()
        .map(|(i, h)| (i, h.distance(p)))
        .filter(|&(_, d)| d <= POINT_REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The Bezier handles shown while point `picked` is picked: its own, and those of its
/// neighbors that face it. Each as point, which (0 in, 1 out) and its end on screen.
fn shown_handles(
    layer: &Layer,
    picked: Option<usize>,
    cam: Camera,
    rect: Rect,
) -> Vec<(usize, usize, Pos2)> {
    let Some(i) = picked else {
        return Vec::new();
    };
    let Some(s) = layer.shape.as_deref() else {
        return Vec::new();
    };
    let n = s.points.len();
    if i >= n {
        return Vec::new();
    }
    let closed = s.kind.closed();
    let mut wanted = vec![(i, 0), (i, 1)];
    if i > 0 || closed {
        wanted.push(((i + n - 1) % n, 1));
    }
    if i + 1 < n || closed {
        wanted.push(((i + 1) % n, 0));
    }
    wanted.dedup();
    wanted
        .into_iter()
        .filter_map(|(k, which)| {
            let end = shape::page_handles(layer, k)[which]?;
            Some((k, which, cam.to_screen(rect, end)))
        })
        .collect()
}

/// The shown Bezier handle under a screen position: point and which.
fn handle_at(
    layer: &Layer,
    picked: Option<usize>,
    cam: Camera,
    rect: Rect,
    p: Pos2,
) -> Option<(usize, usize)> {
    shown_handles(layer, picked, cam, rect)
        .into_iter()
        .map(|(i, which, h)| (i, which, h.distance(p)))
        .filter(|&(_, _, d)| d <= POINT_REACH)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(i, which, _)| (i, which))
}

/// Each segment on screen, as a line through points along its curve.
fn screen_curves(layer: &Layer, cam: Camera, rect: Rect) -> Vec<Vec<Pos2>> {
    shape::page_curves(layer, CURVE_STEPS)
        .into_iter()
        .map(|c| c.into_iter().map(|p| cam.to_screen(rect, p)).collect())
        .collect()
}

/// The segment near a screen position: its number, and where along it (0 to 1).
fn segment_at(layer: &Layer, cam: Camera, rect: Rect, p: Pos2) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32, f32)> = None;
    for (k, line) in screen_curves(layer, cam, rect).into_iter().enumerate() {
        let pieces = line.len().saturating_sub(1).max(1) as f32;
        for (j, w) in line.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let u = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
            let d = (a + ab * u).distance(p);
            if d <= SEGMENT_REACH && best.is_none_or(|(_, _, bd)| d < bd) {
                best = Some((k, (j as f32 + u) / pieces, d));
            }
        }
    }
    best.map(|(k, t, _)| (k, t))
}

/// What the point drawing or editing did with the pointer this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pointer {
    /// A press, click or drag that was for them: not for the canvas.
    pub took: bool,
    /// The pointer is over a point's or a Bezier handle.
    pub over_handle: bool,
    /// The anchor the placed or dragged point snapped to.
    pub snapped: Option<P2>,
    /// The document changed.
    pub changed: bool,
}

/// Where a point goes for the pointer at `p`: Shift keeps its segment from `from` at 15
/// degree steps, then it snaps to the anchor within `snap_reach` mm.
fn place(
    p: P2,
    from: Option<P2>,
    shift: bool,
    anchors: &[P2],
    snap_reach: Option<f32>,
    snapped: &mut Option<P2>,
) -> P2 {
    let mut at = p;
    if shift && let Some(from) = from {
        at = at_angle_step(from, at, 15.0);
    }
    if let Some(reach) = snap_reach
        && let Some((_, a)) = nearest_snap(anchors, &[at], reach)
    {
        at = a;
        *snapped = Some(a);
    }
    at
}

/// Modifier keys and snapping, as the canvas reads them.
#[derive(Clone, Copy)]
pub struct Keys<'a> {
    pub shift: bool,
    pub alt: bool,
    pub anchors: &'a [P2],
    /// Snapping distance in mm, when on.
    pub snap_reach: Option<f32>,
}

/// Before the canvas' own handling, while a shape is drawn: places its points, pulls
/// out their handles and finishes it. Takes every left click and drag meanwhile.
#[allow(clippy::too_many_arguments)]
pub fn draw_input(
    ui: &egui::Ui,
    doc: &mut Document,
    selected: &mut Option<u64>,
    slot: &mut Option<Drawing>,
    cam: Camera,
    rect: Rect,
    response: &Response,
    keys: Keys,
) -> Pointer {
    let Some(d) = slot else {
        return Pointer::default();
    };
    let mut out = Pointer {
        took: true,
        ..Default::default()
    };
    let (pressed, down, time) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.primary_down(),
            i.time,
        )
    });
    if d.done {
        if !down {
            *slot = None;
        }
        return out;
    }
    // Undone meanwhile.
    if d.layer.is_some_and(|id| doc.layer(id).is_none()) {
        *slot = None;
        return out;
    }
    let finish_keys = !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Enter));
    if finish_keys || response.clicked_by(PointerButton::Secondary) {
        out.changed = finish(doc, selected, d);
        *slot = None;
        return out;
    }
    let pointer = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|_| response.hovered() || response.is_pointer_button_down_on());
    if pointer.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }

    // Pulling out the handles of the point just placed.
    if let Some(i) = d.pulling {
        if !down {
            d.pulling = None;
            // A line is done at its second point.
            let count = d
                .layer
                .and_then(|id| doc.layer(id))
                .map_or(0, |l| shape::page_points(l).len());
            if d.kind == ShapeKind::Line && count >= 2 {
                out.changed |= finish(doc, selected, d);
                *slot = None;
            }
        } else if let Some(p) = pointer
            && let Some(l) = d.layer.and_then(|id| doc.layer_mut(id))
            && let Some(&at) = shape::page_points(l).get(i)
        {
            let start = cam.to_screen(rect, at);
            if start.distance(p) > PULL_MIN {
                let mut h = cam.to_mm(rect, p);
                if keys.shift {
                    h = at_angle_step(at, h, 15.0);
                }
                out.changed |= shape::set_handle(l, i, 1, h, Mirror::Full);
            }
        }
        return out;
    }

    let layer = d.layer.and_then(|id| doc.layer(id));
    let points = layer.map(shape::page_points).unwrap_or_default();
    let closing = layer.is_some_and(|l| closes(l, d.kind, cam, rect, pointer));
    // Where a click would place the next point, for the preview too.
    let mut snapped = None;
    let next = pointer.map(|p| {
        let m = cam.to_mm(rect, p);
        if closing {
            points[0]
        } else {
            place(
                m,
                points.last().copied(),
                keys.shift,
                keys.anchors,
                keys.snap_reach,
                &mut snapped,
            )
        }
    });
    out.snapped = snapped;

    if pressed && let (Some(at), Some(p)) = (next, pointer) {
        // The second press of a double click: the first one placed the last point.
        let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
        let double = d
            .last_press
            .is_some_and(|(t, q)| time - t < delay && q.distance(p) < SEGMENT_REACH);
        d.last_press = Some((time, p));
        if double || closing {
            if closing
                && let Some(l) = d.layer.and_then(|id| doc.layer_mut(id))
                && d.kind == ShapeKind::Polyline
            {
                set_kind_renamed(l, ShapeKind::Polygon);
            }
            finish(doc, selected, d);
            d.done = true;
            out.changed = true;
            // Pressed and released within the frame: nothing left to wait for.
            if !down {
                *slot = None;
            }
            return out;
        }
        match d.layer.and_then(|id| doc.layer_mut(id)) {
            Some(l) => out.changed |= shape::push_point(l, at),
            None => {
                let id = doc.alloc_id();
                let at_path = layers_ui::insertion_path(doc, *selected);
                let mut l = shape::new_drawn_layer(id, d.kind, at);
                if d.canvas {
                    shape::make_canvas(&mut l);
                }
                doc.insert(&at_path, Node::Layer(Box::new(l)));
                *selected = Some(id);
                d.layer = Some(id);
                out.changed = true;
            }
        }
        let count = d
            .layer
            .and_then(|id| doc.layer(id))
            .map_or(0, |l| shape::page_points(l).len());
        d.pulling = count.checked_sub(1).filter(|_| down);
        // A line is done at its second point, once its handles are pulled out.
        if d.pulling.is_none() && d.kind == ShapeKind::Line && count >= 2 {
            out.changed |= finish(doc, selected, d);
            *slot = None;
        }
    }
    out
}

/// True if a click at the pointer would close the shape being drawn: on its first
/// point, with at least three placed, for a polyline or polygon.
fn closes(layer: &Layer, kind: ShapeKind, cam: Camera, rect: Rect, pointer: Option<Pos2>) -> bool {
    let points = screen_points(layer, cam, rect);
    kind != ShapeKind::Line
        && points.len() >= 3
        && pointer.is_some_and(|p| points[0].distance(p) <= POINT_REACH)
}

/// Turns a shape into another kind, and its name with it if it was the kind's name.
fn set_kind_renamed(layer: &mut Layer, kind: ShapeKind) {
    let Some(old) = layer.shape.as_ref().map(|s| s.kind) else {
        return;
    };
    if layer.name == translate(old.name_key()) {
        layer.name = translate(kind.name_key()).to_owned();
    }
    shape::set_kind(layer, kind);
}

/// Finishes the shape being drawn. One with too few points is removed, or becomes a
/// polyline if it was to be a polygon with two. Returns true if the document changed.
fn finish(doc: &mut Document, selected: &mut Option<u64>, d: &Drawing) -> bool {
    let Some(id) = d.layer else {
        return false;
    };
    let Some(l) = doc.layer_mut(id) else {
        return false;
    };
    let n = shape::page_points(l).len();
    if n < 2 {
        doc.remove(id);
        if *selected == Some(id) {
            *selected = None;
        }
        return true;
    }
    if n < 3
        && l.shape
            .as_ref()
            .is_some_and(|s| s.kind == ShapeKind::Polygon)
    {
        set_kind_renamed(l, ShapeKind::Polyline);
        return true;
    }
    false
}

/// Before the canvas' own handling, while a shape's points are edited: ends the editing
/// when it should, and takes presses and drags on the points and Bezier handles, and
/// double clicks on points and segments.
#[allow(clippy::too_many_arguments)]
pub fn input(
    ui: &egui::Ui,
    doc: &mut Document,
    selected: Option<u64>,
    slot: &mut Option<PointEdit>,
    cam: Camera,
    rect: Rect,
    response: &Response,
    keys: Keys,
) -> Pointer {
    let Some(e) = slot else {
        return Pointer::default();
    };
    let escape = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(Key::Escape));
    let Some(layer) = editable(doc, selected, e.layer).filter(|_| !escape) else {
        *slot = None;
        return Pointer::default();
    };
    let mut out = Pointer::default();
    let pointer = ui
        .input(|i| i.pointer.interact_pos())
        .filter(|_| response.hovered() || response.is_pointer_button_down_on());
    let (pressed, down) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.primary_down(),
        )
    });
    // Bezier handles first: one can lie over a point.
    let hit = pointer.and_then(|p| {
        handle_at(layer, e.selected, cam, rect, p)
            .map(|(i, which)| Grab::Handle {
                i,
                which,
                smooth: layer.shape.as_ref().is_some_and(|s| s.is_smooth(i)),
            })
            .or_else(|| point_at(layer, cam, rect, p).map(Grab::Point))
    });
    out.over_handle = hit.is_some();

    if pressed && let Some(g) = hit {
        if let Grab::Point(i) = g {
            e.selected = Some(i);
        }
        e.dragging = Some(g);
    } else if response.double_clicked_by(PointerButton::Primary)
        && let Some(p) = pointer
    {
        let l = doc.layer_mut(e.layer);
        match (hit, l) {
            (Some(Grab::Point(i)), Some(l)) => {
                out.changed |= shape::toggle_smooth(l, i);
                out.took = true;
            }
            (None, Some(l)) => {
                let layer = &*l;
                if let Some((k, t)) = segment_at(layer, cam, rect, p)
                    && shape::split_segment(l, k, t)
                {
                    e.selected = Some(k + 1);
                    out.changed = true;
                    out.took = true;
                }
            }
            _ => {}
        }
    }

    if let Some(g) = e.dragging {
        // Down to the release of that press: not a drag or a click for the canvas.
        out.took = true;
        if !down {
            e.dragging = None;
        } else if let Some(p) = pointer
            && let Some(layer) = doc.layer_mut(e.layer)
        {
            let m = cam.to_mm(rect, p);
            match g {
                Grab::Point(i) => {
                    let from = neighbor(layer, i);
                    let at = place(
                        m,
                        from,
                        keys.shift,
                        keys.anchors,
                        keys.snap_reach,
                        &mut out.snapped,
                    );
                    out.changed |= shape::move_point(layer, i, at);
                }
                Grab::Handle { i, which, smooth } => {
                    let mut at = m;
                    if keys.shift
                        && let Some(&from) = shape::page_points(layer).get(i)
                    {
                        at = at_angle_step(from, at, 15.0);
                    }
                    // Alt parts a smooth point's handles.
                    let mirror = if smooth && !keys.alt {
                        Mirror::Turn
                    } else {
                        Mirror::None
                    };
                    out.changed |= shape::set_handle(layer, i, which, at, mirror);
                }
            }
        }
    }

    match (e.dragging, hit) {
        (Some(_), _) => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
        (None, Some(_)) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
        _ => {}
    }
    out
}

/// The point before point `i` on the page, or after it for the first point: the other
/// end of the segment Shift keeps at an angle.
fn neighbor(layer: &Layer, i: usize) -> Option<P2> {
    let points = shape::page_points(layer);
    let k = if i > 0 { i - 1 } else { 1 };
    points.get(k).copied()
}

/// `to`, turned around `from` to the nearest multiple of `step` degrees.
fn at_angle_step(from: P2, to: P2, step: f32) -> P2 {
    let d = [to[0] - from[0], to[1] - from[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let a = d[1].atan2(d[0]).to_degrees();
    let a = ((a / step).round() * step).to_radians();
    [from[0] + len * a.cos(), from[1] + len * a.sin()]
}

/// Deletes the point picked last, if the shape keeps enough. Returns true if it did.
pub fn delete_selected(slot: &mut Option<PointEdit>, doc: &mut Document) -> bool {
    let Some(e) = slot else { return false };
    let Some(i) = e.selected else { return false };
    let Some(layer) = doc.layer_mut(e.layer) else {
        return false;
    };
    if !shape::remove_point(layer, i) {
        return false;
    }
    e.selected = None;
    true
}

/// Moves the point picked last by `d` mm on the page. Returns true if it moved.
pub fn nudge_selected(slot: &Option<PointEdit>, doc: &mut Document, d: P2) -> bool {
    let Some(e) = slot else { return false };
    let Some(i) = e.selected else { return false };
    let Some(layer) = doc.layer_mut(e.layer) else {
        return false;
    };
    let Some(&p) = shape::page_points(layer).get(i) else {
        return false;
    };
    shape::move_point(layer, i, [p[0] + d[0], p[1] + d[1]])
}

/// Colors of the point handles.
pub struct Look {
    pub accent: Color32,
    pub background: Color32,
}

/// Square handles on the points, the picked one filled.
fn paint_points(painter: &egui::Painter, points: &[Pos2], picked: Option<usize>, look: &Look) {
    for (i, &c) in points.iter().enumerate() {
        let r = Rect::from_center_size(c, vec2(POINT_RADIUS, POINT_RADIUS) * 2.0);
        let fill = if picked == Some(i) {
            look.accent
        } else {
            look.background
        };
        painter.rect(
            r,
            1.0,
            fill,
            Stroke::new(1.5, look.accent),
            egui::StrokeKind::Middle,
        );
    }
}

/// Bezier handles: a line from the point to a dot.
fn paint_handles(
    painter: &egui::Painter,
    points: &[Pos2],
    handles: &[(usize, usize, Pos2)],
    look: &Look,
) {
    let line = Stroke::new(1.0, look.accent.gamma_multiply(0.8));
    for &(i, _, end) in handles {
        if let Some(&p) = points.get(i) {
            painter.line_segment([p, end], line);
        }
        painter.circle(
            end,
            HANDLE_RADIUS,
            look.background,
            Stroke::new(1.5, look.accent),
        );
    }
}

/// The shape's segments as thin lines along their curves.
fn paint_curves(painter: &egui::Painter, curves: Vec<Vec<Pos2>>, look: &Look) {
    let line = Stroke::new(1.0, look.accent);
    for c in curves {
        painter.add(Shape::line(c, line));
    }
}

/// Draws the shape's path through its points and their handles while they are edited.
pub fn paint(
    painter: &egui::Painter,
    doc: &Document,
    slot: &Option<PointEdit>,
    cam: Camera,
    rect: Rect,
    look: &Look,
) {
    let Some(e) = slot else { return };
    let Some(layer) = doc.layer(e.layer) else {
        return;
    };
    let points = screen_points(layer, cam, rect);
    paint_curves(painter, screen_curves(layer, cam, rect), look);
    let handles = shown_handles(layer, e.selected, cam, rect);
    paint_handles(painter, &points, &handles, look);
    paint_points(painter, &points, e.selected, look);
}

/// Draws the shape being drawn: its points, the handles of the last one, and the
/// segment the next click would add. The first point is filled when a click would
/// close the shape.
#[allow(clippy::too_many_arguments)]
pub fn paint_drawing(
    painter: &egui::Painter,
    doc: &Document,
    slot: &Option<Drawing>,
    cam: Camera,
    rect: Rect,
    pointer: Option<Pos2>,
    keys: Keys,
    look: &Look,
) {
    let Some(d) = slot.as_ref().filter(|d| !d.done) else {
        return;
    };
    let layer = d.layer.and_then(|id| doc.layer(id));
    let points = layer.map_or(Vec::new(), |l| screen_points(l, cam, rect));
    let closing = layer.is_some_and(|l| closes(l, d.kind, cam, rect, pointer));
    let line = Stroke::new(1.0, look.accent);
    if let (Some(l), Some(p)) = (layer, pointer)
        && !points.is_empty()
        && d.pulling.is_none()
    {
        let page = shape::page_points(l);
        let next = if closing {
            page[0]
        } else {
            place(
                cam.to_mm(rect, p),
                page.last().copied(),
                keys.shift,
                keys.anchors,
                keys.snap_reach,
                &mut None,
            )
        };
        // The next segment, curved by the last point's handle.
        let from = page[page.len() - 1];
        let out = shape::page_handles(l, page.len() - 1)[1].unwrap_or(from);
        let to = cam.to_screen(rect, next);
        let curve: Vec<Pos2> = (0..=CURVE_STEPS)
            .map(|j| {
                let c = [from, out, next, next];
                cam.to_screen(rect, shape::cubic_at(c, j as f32 / CURVE_STEPS as f32))
            })
            .collect();
        painter.add(Shape::line(curve, line));
        // A polygon's closing side.
        if d.kind == ShapeKind::Polygon && points.len() >= 2 && !closing {
            painter.extend(Shape::dashed_line(&[to, points[0]], line, 4.0, 4.0));
        }
    }
    if let Some(l) = layer {
        let last = points.len().checked_sub(1);
        paint_handles(painter, &points, &shown_handles(l, last, cam, rect), look);
    }
    paint_points(painter, &points, closing.then_some(0), look);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_and_segments_under_the_pointer() {
        let mut doc = Document::default();
        let l = shape::new_layer(1, ShapeKind::Polygon, [100.0, 100.0]);
        doc.layers.push(Node::Layer(Box::new(l)));
        let cam = Camera {
            zoom: 4.0,
            pan: egui::Vec2::ZERO,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(2000.0, 2000.0));
        let layer = doc.layer(1).unwrap();
        let points = screen_points(layer, cam, rect);
        assert_eq!(points.len(), 3);
        assert_eq!(
            point_at(layer, cam, rect, points[1] + vec2(3.0, 3.0)),
            Some(1)
        );
        // The middle of the closing side, back to the first point: a new point 3.
        let mid = points[2] + (points[0] - points[2]) / 2.0;
        let (k, t) = segment_at(layer, cam, rect, mid + vec2(0.0, 2.0)).unwrap();
        assert_eq!(k, 2);
        assert!((t - 0.5).abs() < 0.02, "{t}");
        assert!(segment_at(layer, cam, rect, mid + vec2(0.0, 40.0)).is_none());
        assert!(can_edit(&doc, 1));
        doc.layer_mut(1).unwrap().locked = true;
        assert!(!can_edit(&doc, 1));

        // Keyboard: nudge, then delete down to the fewest points.
        doc.layer_mut(1).unwrap().locked = false;
        let mut slot = Some(PointEdit {
            selected: Some(0),
            ..PointEdit::new(1)
        });
        let before = shape::page_points(doc.layer(1).unwrap())[0];
        assert!(nudge_selected(&slot, &mut doc, [1.0, 0.0]));
        let after = shape::page_points(doc.layer(1).unwrap())[0];
        assert!((after[0] - before[0] - 1.0).abs() < 1e-3 && (after[1] - before[1]).abs() < 1e-3);
        assert!(!delete_selected(&mut slot, &mut doc), "a triangle keeps 3");
        let l = doc.layer_mut(1).unwrap();
        shape::split_segment(l, 2, 0.5);
        assert!(delete_selected(&mut slot, &mut doc));
        assert_eq!(slot.as_ref().unwrap().selected, None);
        let s = doc.layer(1).unwrap().shape.as_ref().unwrap();
        assert_eq!(s.points.len(), 3);
    }

    #[test]
    fn finishing_drops_shapes_with_too_few_points() {
        let mut doc = Document::default();
        let mut sel = None;
        // A single point: nothing to keep.
        doc.layers.push(Node::Layer(Box::new(shape::new_drawn_layer(
            1,
            ShapeKind::Polyline,
            [50.0, 50.0],
        ))));
        let d = Drawing {
            layer: Some(1),
            ..Drawing::new(ShapeKind::Polyline)
        };
        assert!(finish(&mut doc, &mut sel, &d));
        assert!(doc.layer(1).is_none());
        // A polygon of two points becomes a polyline.
        let mut l = shape::new_drawn_layer(2, ShapeKind::Polygon, [50.0, 50.0]);
        assert!(shape::push_point(&mut l, [70.0, 60.0]));
        doc.layers.push(Node::Layer(Box::new(l)));
        let d = Drawing {
            layer: Some(2),
            ..Drawing::new(ShapeKind::Polygon)
        };
        assert!(finish(&mut doc, &mut sel, &d));
        let l = doc.layer(2).unwrap();
        assert_eq!(l.shape.as_ref().unwrap().kind, ShapeKind::Polyline);
        assert_eq!(l.name, "Polyline");
    }

    #[test]
    fn shift_keeps_steps_of_15_degrees() {
        let p = at_angle_step([0.0, 0.0], [10.0, 1.0], 15.0);
        assert!((p[1]).abs() < 1e-4 && (p[0] - 101f32.sqrt()).abs() < 1e-4);
        let p = at_angle_step([0.0, 0.0], [10.0, 9.0], 15.0);
        assert!((p[0] - p[1]).abs() < 1e-4, "45 degrees");
    }
}
