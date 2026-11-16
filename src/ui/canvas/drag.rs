//! Selecting, moving, resizing, turning and distorting layers with the pointer, and
//! dragging guides.

use super::*;

/// What the pointer's gestures need this frame: the view, the keys held, and what
/// layers snap to.
pub(super) struct Frame {
    pub cam: Camera,
    pub rect: Rect,
    pub shift: bool,
    pub alt: bool,
    /// Ctrl makes the corner handles distort instead of resizing while held.
    pub distorting: bool,
    /// Ctrl+Alt on a resize handle: the paint in a shape does the other way than its
    /// Stretch paint says, for that drag. Neither key then does what it does alone.
    pub flip_paint: bool,
    /// The template's shape centers, and how close (mm) a point must come to snap to
    /// one, when snapping to them is on.
    pub anchors: Vec<P2>,
    pub snap_reach: Option<f32>,
    /// The page's and the outline's edges, which a dragged guide snaps to.
    pub edges: Guides,
    /// The lines layers snap to: those edges and the guides, each when snapping to them
    /// is on; and their reach in mm.
    pub guides: Guides,
    pub guide_reach: Option<f32>,
    /// The shape whose points are being edited: it shows those instead of its handles.
    editing_points: Option<u64>,
    /// The guides can be grabbed.
    grab_guides: bool,
}

impl Frame {
    pub(super) fn new(
        ui: &egui::Ui,
        doc: &Document,
        elements: &[PlacedElement],
        state: &CanvasState,
        rect: Rect,
    ) -> Self {
        let cam = state.camera;
        let flip_paint = ui.input(|i| i.modifiers.command && i.modifiers.alt);
        let alt = ui.input(|i| i.modifiers.alt) && !flip_paint;
        let anchors: Vec<P2> = elements.iter().flat_map(|e| &e.centers).copied().collect();
        let snap_reach = (state.snap_to_centers && !alt && !anchors.is_empty())
            .then_some(SNAP_RADIUS / cam.zoom);
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
        let guide_reach = (!alt && !(guides.x.is_empty() && guides.y.is_empty()))
            .then_some(SNAP_RADIUS / cam.zoom);
        Self {
            cam,
            rect,
            shift: ui.input(|i| i.modifiers.shift),
            alt,
            distorting: ui.input(|i| i.modifiers.command) && !flip_paint,
            flip_paint,
            anchors,
            snap_reach,
            edges,
            guides,
            guide_reach,
            editing_points: state.points.as_ref().map(|e| e.layer),
            grab_guides: state.show_guides && !state.lock_guides,
        }
    }

    fn to_mm(&self, p: Pos2) -> P2 {
        self.cam.to_mm(self.rect, p)
    }

    /// The guide under a screen point, when guides can be grabbed.
    pub(super) fn guide_under(&self, doc: &Document, p: Pos2) -> Option<usize> {
        self.grab_guides
            .then(|| guide_at(doc, self.cam, self.rect, p))
            .flatten()
    }

    /// Which handle (if any) of the selected layer `sel` is under a screen point.
    pub(super) fn handle_at(
        &self,
        doc: &Document,
        sel: Option<u64>,
        p: Pos2,
    ) -> Option<(u64, DragKind)> {
        let (cam, rect) = (&self.cam, self.rect);
        let id = sel?;
        if doc.effectively_locked(id)
            || !doc.effectively_visible(id)
            || self.editing_points == Some(id)
        {
            return None;
        }
        // A group: the rotation handle above its box (none in the template editor).
        if let Some(Node::Group(_)) = doc.node(id) {
            if doc.edited_template.is_some() {
                return None;
            }
            let (_, rot) = group_rotate_handle(group_box(doc, cam, rect, id)?);
            return (rot.distance(p) <= ROTATE_GRAB)
                .then(|| Some((id, DragKind::RotateGroup(layers::group_middle(doc, id)?))))
                .flatten();
        }
        let layer = doc
            .layer(id)
            .filter(|l| !crate::document::background::is_fixed(l))?;
        let (corners, _, rot) = handles(cam, rect, layer);
        if rot.distance(p) <= ROTATE_GRAB {
            Some((layer.id, DragKind::Rotate))
        } else if let Some(i) = corners
            .iter()
            .position(|c| c.distance(p) <= HANDLE_RADIUS * 1.5)
        {
            let kind = if self.distorting {
                DragKind::Distort(i)
            } else {
                DragKind::Scale
            };
            Some((layer.id, kind))
        } else {
            edge_handles(cam, rect, layer)
                .iter()
                .position(|c| c.distance(p) <= HANDLE_RADIUS * 1.5)
                .map(|i| (layer.id, DragKind::Stretch(i)))
        }
    }
}

/// How close (screen points) the pointer must come to a rotation handle to grab it:
/// more than its size, being small and alone.
const ROTATE_GRAB: f32 = HANDLE_RADIUS * 2.2;

impl Frame {
    /// Which handle of the selected layer `sel` is under a screen point, to highlight it.
    pub(super) fn hot_at(&self, doc: &Document, sel: Option<u64>, p: Pos2) -> Option<Hot> {
        let (id, kind) = self.handle_at(doc, sel, p)?;
        Some(match kind {
            DragKind::Rotate | DragKind::RotateGroup(_) => Hot::Rotate,
            DragKind::Stretch(i) => Hot::Edge(i),
            DragKind::Distort(i) => Hot::Corner(i),
            // The corner nearest the pointer.
            _ => {
                let (corners, _, _) = handles(&self.cam, self.rect, doc.layer(id)?);
                let i = (0..4)
                    .min_by(|&a, &b| corners[a].distance(p).total_cmp(&corners[b].distance(p)))?;
                Hot::Corner(i)
            }
        })
    }
}

/// The pointer while turning something: drawn, as the system has no such cursor, a
/// curved arrow at `at`.
pub(super) fn rotate_cursor(ui: &egui::Ui, at: Pos2) {
    ui.ctx().set_cursor_icon(CursorIcon::None);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("rotate_cursor"),
    ));
    let r = 7.0;
    let arc: Vec<Pos2> = (0..=16)
        .map(|k| {
            let a = (-150.0 + 240.0 * k as f32 / 16.0f32).to_radians();
            at + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    let end = arc[arc.len() - 1];
    let tangent = (end - arc[arc.len() - 2]).normalized();
    let side = tangent.rot90();
    let head = vec![end + tangent * 4.0, end - side * 4.0, end + side * 4.0];
    // White under black: seen on dark and light alike.
    for (width, color) in [(4.0, Color32::WHITE), (1.8, Color32::BLACK)] {
        painter.add(Shape::line(arc.clone(), Stroke::new(width, color)));
        painter.add(Shape::convex_polygon(
            head.clone(),
            color,
            Stroke::new(width - 1.8, Color32::WHITE),
        ));
    }
}

/// What a drag snapped to this frame: a template point (page mm), and a vertical and
/// a horizontal guide (their x and y).
#[derive(Default)]
pub(super) struct Snapped {
    pub anchor: Option<P2>,
    pub guides: [Option<f32>; 2],
}

/// Left presses, clicks and double clicks when no tool takes them: grabbing a guide or
/// a layer's handle, selecting, starting a drag, and starting to edit a guide, a text or
/// a shape's points.
pub(super) fn press_input(
    ui: &egui::Ui,
    doc: &mut Document,
    selected: &mut Option<u64>,
    state: &mut CanvasState,
    response: &egui::Response,
    f: &Frame,
) {
    if response.drag_started_by(PointerButton::Primary) {
        let origin = ui
            .input(|i| i.pointer.press_origin())
            .or(response.interact_pointer_pos());
        // A guide is grabbed unless a handle of the selected layer is there.
        let on_guide = origin
            .filter(|&o| f.handle_at(doc, *selected, o).is_none())
            .and_then(|o| f.guide_under(doc, o));
        if let Some(i) = on_guide {
            state.guide_drag = Some(i);
            state.panning = false;
        } else if let Some(origin) = origin {
            start(ui, doc, selected, state, f, origin);
        }
    } else if response.clicked_by(PointerButton::Primary)
        && let Some(p) = response.interact_pointer_pos()
        && f.handle_at(doc, *selected, p).is_none()
        && f.guide_under(doc, p).is_none()
    {
        *selected = layer_to_select(doc, f.to_mm(p));
    }
    // A double click on a guide asks for its position.
    let guide_clicked = response
        .double_clicked_by(PointerButton::Primary)
        .then(|| response.interact_pointer_pos())
        .flatten()
        .filter(|&p| f.handle_at(doc, *selected, p).is_none())
        .and_then(|p| f.guide_under(doc, p));
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
        let i = text::index_at(layer, f.to_mm(p));
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

/// A left drag starting at `origin` (screen), not on a guide: what it grabs, or else
/// it pans the view.
fn start(
    ui: &egui::Ui,
    doc: &mut Document,
    selected: &mut Option<u64>,
    state: &mut CanvasState,
    f: &Frame,
    origin: Pos2,
) {
    let m = f.to_mm(origin);
    // Ctrl+drag on the selected layer turns it in 3D (not the template's reference,
    // whose tilt isn't saved).
    let tilt = ui.input(|i| i.modifiers.command && !i.modifiers.alt);
    let tiltable = |id: u64| {
        doc.layer(id)
            .is_some_and(|l| !crate::document::background::is_fixed(l))
            && !doc.effectively_locked(id)
            && !crate::template::edit::is_reference(doc, id)
    };
    let (mut ids, kind) = match f.handle_at(doc, *selected, origin) {
        Some((id, kind @ DragKind::RotateGroup(_))) => (layers::movable_layers(doc, id), kind),
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
            // On empty canvas, the drag pans and the selection stays.
            (hit, _) => {
                if let Some(id) = layer_to_select(doc, m) {
                    *selected = Some(id);
                }
                let hit = hit.filter(|&id| {
                    doc.layer(id)
                        .is_none_or(|l| !crate::document::background::is_fixed(l))
                });
                (hit.into_iter().collect(), DragKind::Move)
            }
        },
    };
    // A text following a shape moves with it: the shape is moved.
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
    if f.flip_paint && matches!(kind, DragKind::Scale | DragKind::Stretch(_)) {
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

/// A guide being dragged follows the pointer; let go off the canvas, it is deleted.
/// Returns true if the guides changed.
pub(super) fn guide_input(
    ui: &egui::Ui,
    doc: &mut Document,
    state: &mut CanvasState,
    f: &Frame,
    snapped: &mut Snapped,
) -> bool {
    let Some(i) = state.guide_drag else {
        return false;
    };
    let rect = f.rect;
    let pointer = ui.input(|i| i.pointer.latest_pos());
    if i >= doc.guides.len() {
        state.guide_drag = None;
    } else if ui.input(|i| i.pointer.primary_down()) {
        let a = if doc.guides[i].vertical { 0 } else { 1 };
        if let Some(p) = pointer {
            let mut at = f.to_mm(p)[a];
            // On the canvas, it snaps to the layers, the page and the outline.
            let reach = (state.snap_to_guides && !f.alt && rect.contains(p))
                .then_some(SNAP_RADIUS / f.cam.zoom);
            let target = reach.and_then(|r| snap_axis(&guide_targets(doc, &f.edges, a), &[at], r));
            match target {
                Some((_, t)) => {
                    at = t;
                    snapped.guides[a] = Some(t);
                }
                None => at = (at * 10.0).round() / 10.0,
            }
            if doc.guides[i].at != at {
                doc.guides[i].at = at;
                return true;
            }
        }
    } else {
        state.guide_drag = None;
        if pointer.is_none_or(|p| !rect.contains(p)) {
            doc.guides.remove(i);
            return true;
        }
    }
    false
}

/// A linked position near where it started: a release there puts it back, with a
/// warning meanwhile; farther, a release unlinks it.
pub(super) fn hold_linked(ui: &egui::Ui, doc: &Document, state: &mut CanvasState, f: &Frame) {
    let zoom = f.cam.zoom;
    if let Some(drag) = &mut state.drag
        && drag.linked
    {
        drag.held = drag.layers.iter().all(|(id, start)| {
            doc.layer(*id).is_none_or(|l| {
                let c = l.transform.center;
                let d = [c[0] - start.center[0], c[1] - start.center[1]];
                len(d) * zoom <= LINK_HOLD_RADIUS
            })
        });
        if drag.held
            && let Some(p) = ui.ctx().pointer_hover_pos()
        {
            linked_drag_hint(ui, p);
        }
    }
}

/// The drag in progress follows the pointer. Returns true if the document changed.
pub(super) fn update(
    ui: &egui::Ui,
    doc: &mut Document,
    drag: &Drag,
    response: &egui::Response,
    f: &Frame,
    snapped: &mut Snapped,
) -> bool {
    if !response.dragged_by(PointerButton::Primary) {
        return false;
    }
    let Some(p) = response.interact_pointer_pos() else {
        return false;
    };
    let m = f.to_mm(p);
    match drag.kind {
        DragKind::Move => {
            move_layers(doc, drag, m, f, snapped);
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
        DragKind::RotateGroup(pivot) => {
            // Turned by the angle the pointer went around the middle; Shift: 15° steps.
            let (v0, v) = (
                [
                    drag.start_mouse[0] - pivot[0],
                    drag.start_mouse[1] - pivot[1],
                ],
                [m[0] - pivot[0], m[1] - pivot[1]],
            );
            let mut delta = (v[1].atan2(v[0]) - v0[1].atan2(v0[0])).to_degrees();
            if f.shift {
                delta = (delta / 15.0).round() * 15.0;
            }
            for (id, start) in &drag.layers {
                if let Some(layer) = doc.layer_mut(*id) {
                    layer.transform = layers::turned_about(*start, pivot, delta);
                }
            }
            rotate_cursor(ui, p);
        }
        _ => {
            let Some(&(id, s)) = drag.layers.first() else {
                return false;
            };
            let Some(layer) = doc.layer_mut(id) else {
                return false;
            };
            match drag.kind {
                // Handled above, for all its layers.
                DragKind::Move | DragKind::RotateGroup(_) => {}
                DragKind::Tilt => {
                    tilt(layer, s, drag.start_mouse, m, f);
                    ui.ctx().set_cursor_icon(CursorIcon::AllScroll);
                }
                DragKind::Stretch(edge) => stretch(layer, s, edge, drag.start_mouse, m, f, snapped),
                DragKind::Scale => scale(layer, s, drag.start_mouse, m, f, snapped),
                DragKind::Distort(i) => distort(layer, s, i, drag.start_mouse, m, f, snapped),
                DragKind::Rotate => {
                    let c = s.center;
                    let v0 = [drag.start_mouse[0] - c[0], drag.start_mouse[1] - c[1]];
                    let v = [m[0] - c[0], m[1] - c[1]];
                    let delta = (v[1].atan2(v[0]) - v0[1].atan2(v0[0])).to_degrees();
                    let mut r = s.rotation_deg + delta;
                    if f.shift {
                        r = (r / 15.0).round() * 15.0;
                    }
                    layer.transform.rotation_deg = (r + 180.0).rem_euclid(360.0) - 180.0;
                    rotate_cursor(ui, p);
                }
            }
        }
    }
    true
}

/// Moves the dragged layers with the pointer, now at `m` (page mm).
fn move_layers(doc: &mut Document, drag: &Drag, m: P2, f: &Frame, snapped: &mut Snapped) {
    let shift_all = |doc: &mut Document, off: P2| {
        for (id, _) in &drag.layers {
            if let Some(layer) = doc.layer_mut(*id) {
                let c = layer.transform.center;
                layer.transform.center = [c[0] + off[0], c[1] + off[1]];
            }
        }
    };
    let d = [m[0] - drag.start_mouse[0], m[1] - drag.start_mouse[1]];
    for (id, start) in &drag.layers {
        if let Some(layer) = doc.layer_mut(*id) {
            layer.transform.center = [start.center[0] + d[0], start.center[1] + d[1]];
        }
    }
    // The center or corner nearest an anchor lands on it; everything moves along.
    if let Some(reach) = f.snap_reach {
        let points: Vec<P2> = drag
            .layers
            .iter()
            .filter_map(|(id, _)| doc.layer(*id))
            .flat_map(snap_points)
            .collect();
        if let Some((off, a)) = nearest_snap(&f.anchors, &points, reach) {
            shift_all(doc, off);
            snapped.anchor = Some(a);
        }
    }
    // Otherwise an edge or the middle of what is moved lands on a guide, on each axis.
    let ids: Vec<u64> = drag.layers.iter().map(|(id, _)| *id).collect();
    if snapped.anchor.is_none()
        && let Some(reach) = f.guide_reach
        && let Some((lo, hi)) = layers::layers_bounds(doc, &ids)
    {
        let mut off = [0.0; 2];
        for (a, axis) in [&f.guides.x, &f.guides.y].into_iter().enumerate() {
            let values = [lo[a], (lo[a] + hi[a]) / 2.0, hi[a]];
            if let Some((d, g)) = snap_axis(axis, &values, reach) {
                off[a] = d;
                snapped.guides[a] = Some(g);
            }
        }
        shift_all(doc, off);
    }
}

/// Turns the layer in 3D, from `s` as the pointer went from `from` to `m` (page mm).
fn tilt(layer: &mut Layer, s: Transform, from: P2, m: P2, f: &Frame) {
    // Screen pixels dragged, in the layer's own frame: up is toward its top, whatever
    // its rotation and flips.
    let d = [(m[0] - from[0]) * f.cam.zoom, (m[1] - from[1]) * f.cam.zoom];
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
    if f.shift {
        x = (x / 15.0).round() * 15.0;
        y = (y / 15.0).round() * 15.0;
    }
    layer.transform.tilt_x_deg = crate::document::variables::clamp_tilt(x);
    layer.transform.tilt_y_deg = crate::document::variables::clamp_tilt(y);
}

/// Stretches the layer from the middle of `edge`. The opposite edge stays put; with
/// Alt, both move (center fixed).
fn stretch(
    layer: &mut Layer,
    s: Transform,
    edge: usize,
    from: P2,
    m: P2,
    f: &Frame,
    snapped: &mut Snapped,
) {
    let (dir, extent) = edge_axis(s, layer.size_px, edge);
    let axis = if edge.is_multiple_of(2) { 1 } else { 0 };
    let px = layer.size_px[axis].max(1) as f32;
    if extent <= 0.0 {
        return;
    }
    let c = s.center;
    let d = [m[0] - from[0], m[1] - from[1]];
    let pull = d[0] * dir[0] + d[1] * dir[1];
    let both = if f.alt { 2.0 } else { 1.0 };
    let mut new = (extent + pull * both).max(MIN_STRETCH_MM);
    // An edge parallel to a guide lands on it. The edge stands `new / 2` from the center
    // with Alt, `new - extent / 2` from the starting center without.
    let a = (0..2).find(|&a| dir[a].abs() > 0.999);
    if let Some(a) = a.filter(|_| !s.is_distorted())
        && let Some(reach) = f.guide_reach
    {
        let lines = if a == 0 { &f.guides.x } else { &f.guides.y };
        let from_center = |new: f32| if f.alt { new / 2.0 } else { new - extent / 2.0 };
        let edge = c[a] + dir[a] * from_center(new);
        if let Some((_, g)) = snap_axis(lines, &[edge], reach) {
            let e = dir[a] * (g - c[a]);
            let snapped_new = if f.alt { 2.0 * e } else { e + extent / 2.0 };
            if snapped_new >= MIN_STRETCH_MM {
                new = snapped_new;
                snapped.guides[a] = Some(g);
            }
        }
    }
    let t = &mut layer.transform;
    t.scale[axis] = new / px;
    let shift_c = if f.alt { 0.0 } else { (new - extent) / 2.0 };
    t.center = [
        s.center[0] + dir[0] * shift_c,
        s.center[1] + dir[1] * shift_c,
    ];
}

/// Resizes the layer from a corner, the center staying put; with Shift, freely along
/// its own axes.
fn scale(layer: &mut Layer, s: Transform, from: P2, m: P2, f: &Frame, snapped: &mut Snapped) {
    let c = s.center;
    let v0 = [from[0] - c[0], from[1] - c[1]];
    let v = [m[0] - c[0], m[1] - c[1]];
    let size = layer.size_px;
    let t = &mut layer.transform;
    if f.shift {
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
        return;
    }
    let k = if len(v0) > 1e-3 {
        len(v) / len(v0)
    } else {
        1.0
    };
    t.scale = [(s.scale[0] * k).max(1e-4), (s.scale[1] * k).max(1e-4)];
    // The dragged corner snaps: scaled so it comes as close as it can to the anchor
    // along its diagonal, the center staying put.
    let start = corners_of(s, size);
    let i = (0..4)
        .min_by(|&a, &b| {
            let d = |k: usize| len([start[k][0] - from[0], start[k][1] - from[1]]);
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(0);
    if let Some(reach) = f.snap_reach {
        let p = corners_of(*t, size)[i];
        if let Some((_, a)) = nearest_snap(&f.anchors, &[p], reach) {
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
                snapped.anchor = Some(a);
            }
        }
    }
    // Otherwise the corner's x or y lands on a guide, whichever is nearer, the center
    // staying put.
    if snapped.anchor.is_none()
        && let Some(reach) = f.guide_reach
    {
        let p = corners_of(*t, size)[i];
        let mut best: Option<(f32, usize, f32)> = None;
        for (a, axis) in [&f.guides.x, &f.guides.y].into_iter().enumerate() {
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
                snapped.guides[a] = Some(g);
            }
        }
    }
}

/// Moves corner `i` on its own (perspective distortion).
fn distort(
    layer: &mut Layer,
    s: Transform,
    i: usize,
    from: P2,
    m: P2,
    f: &Frame,
    snapped: &mut Snapped,
) {
    // Corner offsets live in the undistorted image frame, which this drag does not
    // change.
    let (Some(a), Some(b)) = (layer.page_to_unit(from), layer.page_to_unit(m)) else {
        return;
    };
    let mut d = s.distort;
    d[i] = [d[i][0] + b[0] - a[0], d[i][1] + b[1] - a[1]];
    // The corner lands on an anchor within reach.
    let mut t2 = s;
    t2.distort = d;
    if let Some(reach) = f.snap_reach {
        let p = corners_of(t2, layer.size_px)[i];
        if let Some((_, anchor)) = nearest_snap(&f.anchors, &[p], reach)
            && let Some(u) = layer.page_to_unit(anchor)
        {
            let base = s.tilted_corners(layer.size_px)[i];
            d[i] = [u[0] - base[0], u[1] - base[1]];
            snapped.anchor = Some(anchor);
        }
    }
    t2.distort = d;
    // A corner dragged across the others would fold the image.
    if is_convex(t2.unit_quad(layer.size_px)) {
        layer.transform.distort = d;
    }
}

/// The drag is over. Released near where it started, a linked layer goes back, still
/// linked; farther, it stays, and is unlinked once the drag is over (see
/// `held_layers`). A single layer moved onto a template point is offered a link to it.
pub(super) fn release(doc: &mut Document, state: &mut CanvasState) {
    let Some(drag) = state.drag.take() else {
        return;
    };
    if drag.held {
        for (id, start) in &drag.layers {
            if let Some(layer) = doc.layer_mut(*id) {
                layer.transform = *start;
            }
        }
    }
    let offer = match (drag.kind, drag.layers.as_slice(), state.snapped) {
        _ if drag.held => None,
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
}

/// The cursor over the canvas at `p` when nothing is dragged: what a press there would
/// grab.
pub(super) fn hover_cursor(
    ui: &egui::Ui,
    doc: &Document,
    selected: Option<u64>,
    hovered_guide: Option<usize>,
    f: &Frame,
    p: Pos2,
) {
    let hovered_guide = hovered_guide.and_then(|i| doc.guides.get(i));
    let icon = match f.handle_at(doc, selected, p) {
        Some((_, DragKind::Rotate | DragKind::RotateGroup(_))) => return rotate_cursor(ui, p),
        Some((id, DragKind::Stretch(edge))) => {
            // Resize cursor along the edge's direction on screen.
            let horizontal = doc.layer(id).is_some_and(|l| {
                let (dir, _) = edge_axis(l.transform, l.size_px, edge);
                dir[0].abs() >= dir[1].abs()
            });
            if horizontal {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::ResizeVertical
            }
        }
        Some(_) => CursorIcon::Crosshair,
        None if let Some(g) = hovered_guide => guide_cursor(g.vertical),
        // Ctrl over the selected layer: a drag turns it in 3D.
        None if ui.input(|i| i.modifiers.command && !i.modifiers.alt)
            && selected.is_some()
            && topmost_hit(doc, f.to_mm(p)) == selected
            && selected.is_some_and(|id| doc.layer(id).is_some()) =>
        {
            CursorIcon::AllScroll
        }
        None if topmost_hit(doc, f.to_mm(p)).is_some() => CursorIcon::Move,
        None => CursorIcon::Grab,
    };
    ui.ctx().set_cursor_icon(icon);
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
