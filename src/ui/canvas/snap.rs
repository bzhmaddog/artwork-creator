//! Snapping: the template's guides (page, outline, bleed) and anchor points.

use super::*;

/// Points of a layer that snap to anchors: its center, its drawn corners, then the
/// middles of its drawn edges. The
/// center is the middle of the drawn corners' bounds, as for Align: once the layer is
/// distorted, that is where it looks centered, not the center of its undistorted frame.
/// A shape's are those of its geometry, its stroke left out (see `shape::snap_points`).
pub(super) fn snap_points(layer: &Layer) -> Vec<P2> {
    if layer.shape.is_some() {
        return crate::document::shape::snap_points(layer);
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
    pub(super) fn sorted(mut self) -> Self {
        for axis in [&mut self.x, &mut self.y] {
            axis.sort_by(f32::total_cmp);
            axis.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        }
        self
    }
}

/// The shift that brings the nearest of `values` onto one of `guides`, if within `reach`
/// (mm), and that guide.
pub(super) fn snap_axis(guides: &[f32], values: &[f32], reach: f32) -> Option<(f32, f32)> {
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
