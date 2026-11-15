//! Selection handles and picking the layer under the pointer.

use super::*;

/// Drawn corners of an image of `size_px` placed by `t`.
pub(super) fn corners_of(t: Transform, size_px: [u32; 2]) -> [P2; 4] {
    let m = t.to_affine(size_px);
    let (w, h) = (size_px[0] as f32, size_px[1] as f32);
    t.unit_quad(size_px).map(|q| m.apply([q[0] * w, q[1] * h]))
}

/// Screen positions of the edge-middle handles (top, right, bottom, left), on the drawn
/// (possibly distorted) edges.
pub(super) fn edge_handles(cam: &Camera, rect: Rect, layer: &Layer) -> [Pos2; 4] {
    let c = layer.corners().map(|p| cam.to_screen(rect, p));
    std::array::from_fn(|i| c[i] + (c[(i + 1) % 4] - c[i]) / 2.0)
}

/// Outward unit direction (page mm) of image edge `edge` and the image's full extent along
/// it, in the undistorted frame of `t`.
pub(super) fn edge_axis(t: Transform, size_px: [u32; 2], edge: usize) -> (P2, f32) {
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

/// The box around the visible layers of group `id` on screen, as drawn around it when
/// selected.
pub(super) fn group_box(doc: &Document, cam: &Camera, rect: Rect, id: u64) -> Option<Rect> {
    let path = doc.path_of(id)?;
    let pts: Vec<Pos2> = doc
        .leaves()
        .into_iter()
        .filter(|l| l.visible && l.path.starts_with(&path))
        .flat_map(|l| l.layer.corners().map(|p| cam.to_screen(rect, p)))
        .collect();
    (!pts.is_empty()).then(|| Rect::from_points(&pts).expand(4.0))
}

/// The middle of the top of a group's box on screen, and its rotation handle above.
pub(super) fn group_rotate_handle(b: Rect) -> (Pos2, Pos2) {
    let top_mid = b.center_top();
    (top_mid, top_mid - vec2(0.0, ROTATE_HANDLE_OFFSET))
}

/// Screen positions of the four corners, the middle of the top edge and the rotation
/// handle.
pub(super) fn handles(cam: &Camera, rect: Rect, layer: &Layer) -> ([Pos2; 4], Pos2, Pos2) {
    let c = layer.corners().map(|p| cam.to_screen(rect, p));
    let center = cam.to_screen(rect, layer.transform.center);
    let top_mid = c[0] + (c[1] - c[0]) / 2.0;
    let dir = (top_mid - center).normalized();
    let dir = if dir.is_finite() { dir } else { -Vec2::Y };
    (c, top_mid, top_mid + dir * ROTATE_HANDLE_OFFSET)
}

/// Of the layers under a click, topmost first, the one it selects: the topmost; in the
/// template editor, the smallest (on a tie, the topmost), so a click inside a button or
/// a cutout picks that hole rather than the outline around it.
pub(super) fn pick<'a>(doc: &Document, hits: impl Iterator<Item = &'a Layer>) -> Option<u64> {
    if doc.edited_template.is_none() {
        return hits.map(|l| l.id).next();
    }
    let area = |l: &Layer| {
        let c = l.corners();
        (0..4)
            .map(|i| {
                let (a, b) = (c[i], c[(i + 1) % 4]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f32>()
            .abs()
            / 2.0
    };
    hits.map(|l| (area(l), l.id))
        .fold(None, |best: Option<(f32, u64)>, (a, id)| match best {
            Some((b, _)) if b <= a => best,
            _ => Some((a, id)),
        })
        .map(|(_, id)| id)
}

/// The visible, unlocked layer under a page point a click selects (see `pick`).
pub(super) fn topmost_hit(doc: &Document, p: P2) -> Option<u64> {
    let leaves = doc.leaves();
    let hits = leaves
        .iter()
        .rev()
        .filter(|l| l.visible && !l.locked && l.layer.contains(p))
        .map(|l| l.layer);
    pick(doc, hits)
}

/// The layer a click at a page point selects: an unlocked one, or else a locked one. A
/// locked layer can be selected (for the layer toolbar and Properties) but does not
/// hide the layers under it from clicks.
pub(super) fn layer_to_select(doc: &Document, p: P2) -> Option<u64> {
    topmost_hit(doc, p).or_else(|| {
        let leaves = doc.leaves();
        let hits = leaves
            .iter()
            .rev()
            .filter(|l| l.visible && l.layer.contains(p))
            .map(|l| l.layer);
        pick(doc, hits)
    })
}

/// Rotates a vector by `deg` degrees, clockwise on screen.
pub(super) fn rotate_vec(v: P2, deg: f32) -> P2 {
    let (s, c) = deg.to_radians().sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1]]
}

/// Length of a vector.
pub(super) fn len(v: P2) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// The handle of the selection under the pointer, or being dragged: drawn highlighted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Hot {
    /// A corner: top left, top right, bottom right, bottom left.
    Corner(usize),
    /// The middle of an edge: top, right, bottom, left.
    Edge(usize),
    /// The rotation handle, of a layer or a group.
    Rotate,
}
