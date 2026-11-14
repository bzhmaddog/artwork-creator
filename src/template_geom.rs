//! Template geometry as editable shapes, for the template editor: a template's outlines
//! (from its SVG, see `template_svg`, or the DXF files of older templates) read as
//! circles, rectangles (rounded or not) and paths whose arcs stay curves, and shape
//! layers turned back into circles and outlines of lines, arcs and Beziers, which
//! `template_svg` writes.
//!
//! Items are in mm, y up, as DXF has them. The editor's page is in mm, y down.

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;

use dxf::entities::{EntityType, Spline};

use crate::document::{Layer, P2};
use crate::i18n::tr;
use crate::shape::{self, Shape, ShapeKind};

/// A point in DXF mm, y up.
pub type V = [f64; 2];

/// Ends closer than this (mm) are the same point.
const TOLERANCE: f64 = 0.01;

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: V, k: f64) -> V {
    [a[0] * k, a[1] * k]
}
fn len(a: V) -> f64 {
    a[0].hypot(a[1])
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: V, b: V) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn near(a: V, b: V, tol: f64) -> bool {
    len(sub(a, b)) <= tol
}
fn polar(c: V, r: f64, a: f64) -> V {
    [c[0] + r * a.cos(), c[1] + r * a.sin()]
}

/// A piece of an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line(V, V),
    /// Around `c`, from angle `a0` (radians), turning `sweep` (counter-clockwise when
    /// positive).
    Arc {
        c: V,
        r: f64,
        a0: f64,
        sweep: f64,
    },
    Cubic([V; 4]),
}

impl Seg {
    pub fn start(&self) -> V {
        match *self {
            Seg::Line(a, _) => a,
            Seg::Arc { c, r, a0, .. } => polar(c, r, a0),
            Seg::Cubic(p) => p[0],
        }
    }

    pub fn end(&self) -> V {
        match *self {
            Seg::Line(_, b) => b,
            Seg::Arc { c, r, a0, sweep } => polar(c, r, a0 + sweep),
            Seg::Cubic(p) => p[3],
        }
    }

    fn reversed(&self) -> Seg {
        match *self {
            Seg::Line(a, b) => Seg::Line(b, a),
            Seg::Arc { c, r, a0, sweep } => Seg::Arc {
                c,
                r,
                a0: a0 + sweep,
                sweep: -sweep,
            },
            Seg::Cubic([a, b, c, d]) => Seg::Cubic([d, c, b, a]),
        }
    }

    /// The arc from `a` to `b` of a polyline vertex's bulge (the tangent of a quarter of
    /// its angle, counter-clockwise when positive), or the line when it has none.
    pub fn bulged(a: V, b: V, bulge: f64) -> Seg {
        let chord = len(sub(b, a));
        if bulge.abs() < 1e-9 || chord < 1e-12 {
            return Seg::Line(a, b);
        }
        let sweep = 4.0 * bulge.atan();
        let r = chord / (2.0 * (sweep / 2.0).sin()).abs();
        // The center is on the chord's left for a counter-clockwise arc under a half turn.
        let mid = mul(add(a, b), 0.5);
        let d = mul(sub(b, a), 1.0 / chord);
        let off = (chord / 2.0) / (sweep / 2.0).tan();
        let c = add(mid, mul([-d[1], d[0]], off));
        let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
        Seg::Arc { c, r, a0, sweep }
    }

    /// The same curve as cubic Beziers: one for a line or a cubic, one per quarter turn
    /// of an arc.
    pub fn cubics(&self) -> Vec<[V; 4]> {
        match *self {
            Seg::Line(a, b) => vec![[a, a, b, b]],
            Seg::Cubic(p) => vec![p],
            Seg::Arc { c, r, a0, sweep } => {
                let n = (sweep.abs() / FRAC_PI_2 - 1e-9).ceil().max(1.0) as usize;
                let d = sweep / n as f64;
                let k = 4.0 / 3.0 * (d / 4.0).tan() * r;
                (0..n)
                    .map(|i| {
                        let (s, e) = (a0 + d * i as f64, a0 + d * (i + 1) as f64);
                        let (p0, p3) = (polar(c, r, s), polar(c, r, e));
                        [
                            p0,
                            add(p0, mul([-s.sin(), s.cos()], k)),
                            sub(p3, mul([-e.sin(), e.cos()], k)),
                            p3,
                        ]
                    })
                    .collect()
            }
        }
    }

    /// Points along it, for its bounds.
    fn samples(&self) -> Vec<V> {
        match *self {
            Seg::Line(a, b) => vec![a, b],
            Seg::Arc { c, r, a0, sweep } => (0..=16)
                .map(|i| polar(c, r, a0 + sweep * i as f64 / 16.0))
                .collect(),
            Seg::Cubic(p) => p.to_vec(),
        }
    }
}

/// What a template's file holds, as the editor makes shapes of it.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Circle {
        c: V,
        r: f64,
    },
    /// Pieces joined end to end, in order.
    Outline {
        segs: Vec<Seg>,
        closed: bool,
    },
    Point(V),
}

impl Item {
    fn samples(&self) -> Vec<V> {
        match self {
            Item::Circle { c, r } => vec![sub(*c, [*r, *r]), add(*c, [*r, *r])],
            Item::Outline { segs, .. } => segs.iter().flat_map(Seg::samples).collect(),
            Item::Point(p) => vec![*p],
        }
    }
}

/// The smallest and largest corners of `points`.
fn bounds(points: &[V]) -> Option<(V, V)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(lo, hi), p| {
        (
            [lo[0].min(p[0]), lo[1].min(p[1])],
            [hi[0].max(p[0]), hi[1].max(p[1])],
        )
    }))
}

/// Points along a B-spline of `degree` (de Boor's algorithm), 16 per span; rational
/// weights are left out.
pub fn spline_points(degree: usize, knots: &[f64], ctrl: &[V]) -> Vec<V> {
    let n = ctrl.len();
    if n == 0 || knots.len() < n + degree + 1 || degree == 0 {
        return ctrl.to_vec();
    }
    let (lo, hi) = (knots[degree], knots[n]);
    if hi <= lo {
        return ctrl.to_vec();
    }
    let at = |t: f64| -> V {
        // The span holding t.
        let mut k = degree;
        while k < n - 1 && knots[k + 1] <= t {
            k += 1;
        }
        let mut d: Vec<V> = (0..=degree).map(|j| ctrl[j + k - degree]).collect();
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let i = j + k - degree;
                let den = knots[i + degree + 1 - r] - knots[i];
                let a = if den.abs() < 1e-12 {
                    0.0
                } else {
                    (t - knots[i]) / den
                };
                d[j] = add(mul(d[j - 1], 1.0 - a), mul(d[j], a));
            }
        }
        d[degree]
    };
    let steps = 16 * (n - degree);
    (0..=steps)
        .map(|i| at(lo + (hi - lo) * i as f64 / steps as f64))
        .collect()
}

/// The control points of a spline that is one cubic Bezier (degree 3, four control
/// points, knots 0 0 0 0 1 1 1 1 or the like), as written for a curve.
fn spline_cubic(s: &Spline) -> Option<[V; 4]> {
    let k = &s.knot_values;
    (s.degree_of_curve == 3
        && s.control_points.len() == 4
        && k.len() == 8
        && k[..4].iter().all(|&v| v == k[0])
        && k[4..].iter().all(|&v| v == k[7])
        && k[7] > k[0])
        .then(|| {
            let p = |i: usize| [s.control_points[i].x, s.control_points[i].y];
            [p(0), p(1), p(2), p(3)]
        })
}

/// What a DXF file draws, in mm (`units` is mm per drawing unit when `template.json`
/// sets it), its open pieces joined into outlines where their ends meet.
pub fn read(path: &Path, units: Option<f64>) -> Result<Vec<Item>, String> {
    let drawing = dxf::Drawing::load_file(path)
        .map_err(|e| tr!("CANNOT_READ_FILE", path = path.display(), e = e))?;
    let k = units.unwrap_or_else(|| crate::template::mm_per_unit(&drawing));
    let mut items = Vec::new();
    let mut pieces: Vec<Vec<Seg>> = Vec::new();
    let poly = |verts: &[(V, f64)], closed: bool| -> Vec<Seg> {
        let mut segs: Vec<Seg> = verts
            .windows(2)
            .map(|w| Seg::bulged(w[0].0, w[1].0, w[0].1))
            .collect();
        if closed
            && let (Some(first), Some(last)) = (verts.first(), verts.last())
            && verts.len() > 1
            && !near(first.0, last.0, 1e-9)
        {
            segs.push(Seg::bulged(last.0, first.0, last.1));
        }
        segs
    };
    for entity in drawing.entities() {
        match &entity.specific {
            EntityType::ModelPoint(p) => items.push(Item::Point([p.location.x, p.location.y])),
            EntityType::Circle(c) => items.push(Item::Circle {
                c: [c.center.x, c.center.y],
                r: c.radius,
            }),
            EntityType::Line(l) => pieces.push(vec![Seg::Line([l.p1.x, l.p1.y], [l.p2.x, l.p2.y])]),
            EntityType::Arc(a) => {
                let (s, mut e) = (a.start_angle.to_radians(), a.end_angle.to_radians());
                if e <= s {
                    e += TAU;
                }
                pieces.push(vec![Seg::Arc {
                    c: [a.center.x, a.center.y],
                    r: a.radius,
                    a0: s,
                    sweep: e - s,
                }]);
            }
            EntityType::LwPolyline(pl) => {
                let verts: Vec<_> = pl.vertices.iter().map(|v| ([v.x, v.y], v.bulge)).collect();
                let segs = poly(&verts, pl.is_closed());
                if pl.is_closed() {
                    items.push(Item::Outline { segs, closed: true });
                } else {
                    pieces.push(segs);
                }
            }
            EntityType::Polyline(pl) => {
                if pl.flags & (16 | 64) != 0 {
                    continue;
                }
                let verts: Vec<_> = pl
                    .vertices()
                    .map(|v| ([v.location.x, v.location.y], v.bulge))
                    .collect();
                let segs = poly(&verts, pl.is_closed());
                if pl.is_closed() {
                    items.push(Item::Outline { segs, closed: true });
                } else {
                    pieces.push(segs);
                }
            }
            EntityType::Spline(s) => match spline_cubic(s) {
                Some(c) => pieces.push(vec![Seg::Cubic(c)]),
                None => {
                    let ctrl: Vec<V> = s.control_points.iter().map(|p| [p.x, p.y]).collect();
                    let pts =
                        spline_points(s.degree_of_curve.max(1) as usize, &s.knot_values, &ctrl);
                    let segs: Vec<Seg> = pts.windows(2).map(|w| Seg::Line(w[0], w[1])).collect();
                    if s.is_closed() {
                        items.push(Item::Outline { segs, closed: true });
                    } else {
                        pieces.push(segs);
                    }
                }
            },
            _ => {}
        }
    }
    pieces.retain(|p| !p.is_empty());
    items.extend(join(pieces));
    // To mm.
    let scale = |p: &mut V| *p = mul(*p, k);
    for item in &mut items {
        match item {
            Item::Point(p) => scale(p),
            Item::Circle { c, r } => {
                scale(c);
                *r *= k;
            }
            Item::Outline { segs, .. } => {
                for s in segs {
                    match s {
                        Seg::Line(a, b) => {
                            scale(a);
                            scale(b);
                        }
                        Seg::Arc { c, r, .. } => {
                            scale(c);
                            *r *= k;
                        }
                        Seg::Cubic(p) => p.iter_mut().for_each(scale),
                    }
                }
            }
        }
    }
    Ok(items)
}

/// Joins pieces whose ends meet into outlines, closed when they come back to their
/// start.
pub fn join(mut open: Vec<Vec<Seg>>) -> Vec<Item> {
    let tol = TOLERANCE;
    let mut out = Vec::new();
    while let Some(mut path) = open.pop() {
        loop {
            let (start, end) = (path[0].start(), path[path.len() - 1].end());
            if near(start, end, tol) && (path.len() > 1 || !matches!(path[0], Seg::Line(..))) {
                out.push(Item::Outline {
                    segs: path,
                    closed: true,
                });
                break;
            }
            let found = open.iter().position(|q| {
                let (qs, qe) = (q[0].start(), q[q.len() - 1].end());
                near(end, qs, tol)
                    || near(end, qe, tol)
                    || near(start, qs, tol)
                    || near(start, qe, tol)
            });
            let Some(i) = found else {
                out.push(Item::Outline {
                    segs: path,
                    closed: false,
                });
                break;
            };
            let q = open.swap_remove(i);
            let rev = |q: Vec<Seg>| -> Vec<Seg> { q.iter().rev().map(Seg::reversed).collect() };
            let (qs, qe) = (q[0].start(), q[q.len() - 1].end());
            if near(end, qs, tol) {
                path.extend(q);
            } else if near(end, qe, tol) {
                path.extend(rev(q));
            } else if near(start, qe, tol) {
                let mut q = q;
                q.extend(path);
                path = q;
            } else {
                let mut q = rev(q);
                q.extend(path);
                path = q;
            }
        }
    }
    out
}

/// A shape as the editor makes it: what it is, and where, on the page (mm, y down).
#[derive(Clone, Debug, PartialEq)]
pub enum Made {
    /// A circle's center and diameter.
    Circle { center: P2, diameter: f32 },
    /// A rectangle's center, size, clockwise rotation (degrees) and corner radius.
    Rect {
        center: P2,
        size: P2,
        rotation: f32,
        radius: f32,
    },
    /// An outline: its shape (points in its frame) and where its frame's origin is.
    Path { shape: Box<Shape>, at: P2 },
}

/// How DXF mm (y up) are placed on the editor's page (y down): `[x, y]` goes to
/// `[x + dx, dy - y]`.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub dx: f64,
    pub dy: f64,
}

impl Frame {
    pub fn page(&self, p: V) -> P2 {
        [(p[0] + self.dx) as f32, (self.dy - p[1]) as f32]
    }

    /// The frame putting the top-left of `bounds` at `margin` mm from the page's corner.
    pub fn for_bounds(bounds: (V, V), margin: f64) -> Self {
        let (lo, hi) = bounds;
        Self {
            dx: margin - lo[0],
            dy: margin + hi[1],
        }
    }
}

/// The bounds of all the items, in DXF mm; points count only without anything else.
pub fn items_bounds<'a>(items: impl IntoIterator<Item = &'a Item>) -> Option<(V, V)> {
    let items: Vec<&Item> = items.into_iter().collect();
    let lines: Vec<V> = items
        .iter()
        .filter(|i| !matches!(i, Item::Point(_)))
        .flat_map(|i| i.samples())
        .collect();
    bounds(&lines).or_else(|| bounds(&items.iter().flat_map(|i| i.samples()).collect::<Vec<_>>()))
}

/// What the editor makes of an item: a circle, a rectangle or a path. Points are left
/// out.
pub fn make(item: &Item, frame: Frame) -> Option<Made> {
    match item {
        Item::Point(_) => None,
        Item::Circle { c, r } => Some(Made::Circle {
            center: frame.page(*c),
            diameter: (2.0 * r) as f32,
        }),
        Item::Outline { segs, closed } => {
            if *closed && let Some((c, r)) = circle(segs) {
                return Some(Made::Circle {
                    center: frame.page(c),
                    diameter: (2.0 * r) as f32,
                });
            }
            if *closed && let Some(rect) = rectangle(segs, frame) {
                return Some(rect);
            }
            Some(path(segs, *closed, frame))
        }
    }
}

/// The circle a closed outline draws, if it is one: arcs of one center and radius
/// going once around (as CAD often draws a circle in two halves).
fn circle(segs: &[Seg]) -> Option<(V, f64)> {
    let Seg::Arc { c, r, .. } = segs.first()? else {
        return polygon_circle(segs);
    };
    let mut turn = 0.0;
    for s in segs {
        let Seg::Arc {
            c: c2,
            r: r2,
            sweep,
            ..
        } = s
        else {
            return None;
        };
        if !near(*c, *c2, 1e-3) || (r - r2).abs() > 1e-3 {
            return None;
        }
        turn += sweep;
    }
    ((turn.abs() - TAU).abs() < 1e-3).then_some((*c, *r))
}

/// The circle a closed outline of many short lines stands for, if all its corners are
/// as far from their middle (as some CAD exports write circles: 60 sides for the
/// Cosmox T16's buttons).
fn polygon_circle(segs: &[Seg]) -> Option<(V, f64)> {
    if segs.len() < 12 || !segs.iter().all(|s| matches!(s, Seg::Line(..))) {
        return None;
    }
    let pts: Vec<V> = segs.iter().map(Seg::start).collect();
    let c = mul(
        pts.iter().fold([0.0, 0.0], |a, p| add(a, *p)),
        1.0 / pts.len() as f64,
    );
    let radii: Vec<f64> = pts.iter().map(|p| len(sub(*p, c))).collect();
    let r = radii.iter().sum::<f64>() / radii.len() as f64;
    // Within 1% of the radius (and no more than 0.05 mm).
    let tol = (r * 0.01).min(0.05);
    (r > 0.0 && radii.iter().all(|x| (x - r).abs() <= tol)).then_some((c, r))
}

/// The rectangle a closed outline draws, if it is one: four sides at right angles, or
/// four sides joined by quarter-circle corners of one radius.
fn rectangle(segs: &[Seg], frame: Frame) -> Option<Made> {
    let lines_at: Vec<usize> = (0..segs.len())
        .filter(|&i| matches!(segs[i], Seg::Line(..)))
        .collect();
    let (sides, radius, centers): (Vec<Seg>, f64, Vec<V>) = match segs.len() {
        4 if lines_at.len() == 4 => (segs.to_vec(), 0.0, segs.iter().map(Seg::start).collect()),
        8 if lines_at.len() == 4 => {
            // Lines and arcs taking turns, from a line.
            let first = lines_at[0];
            let ring: Vec<Seg> = (0..8).map(|i| segs[(first + i) % 8]).collect();
            let mut r0 = None;
            let mut centers = Vec::new();
            for (i, s) in ring.iter().enumerate() {
                match (i % 2, s) {
                    (0, Seg::Line(..)) => {}
                    (1, Seg::Arc { c, r, sweep, .. }) => {
                        if (sweep.abs() - FRAC_PI_2).abs() > 1e-3 {
                            return None;
                        }
                        let r0 = *r0.get_or_insert(*r);
                        if (r - r0).abs() > 1e-3 {
                            return None;
                        }
                        centers.push(*c);
                    }
                    _ => return None,
                }
            }
            let sides = ring.iter().step_by(2).copied().collect();
            (sides, r0?, centers)
        }
        _ => return None,
    };
    let dirs: Vec<V> = sides.iter().map(|s| sub(s.end(), s.start())).collect();
    let lens: Vec<f64> = dirs.iter().map(|d| len(*d)).collect();
    let tol = 1e-3 * lens.iter().cloned().fold(1.0, f64::max);
    for i in 0..4 {
        let (a, b) = (dirs[i], dirs[(i + 1) % 4]);
        if lens[i] < 1e-9 && radius == 0.0 {
            return None;
        }
        if dot(a, b).abs() > tol * (lens[i] + lens[(i + 1) % 4]) {
            return None;
        }
    }
    if (lens[0] - lens[2]).abs() > tol || (lens[1] - lens[3]).abs() > tol {
        return None;
    }
    let center = mul(
        centers.iter().fold([0.0, 0.0], |acc, p| add(acc, *p)),
        1.0 / centers.len() as f64,
    );
    let angle = dirs[0][1].atan2(dirs[0][0]);
    Some(Made::Rect {
        center: frame.page(center),
        size: [
            (lens[0] + 2.0 * radius) as f32,
            (lens[1] + 2.0 * radius) as f32,
        ],
        // Counter-clockwise, y up, is clockwise on the page, y down, the other way.
        rotation: normalize_degrees(-angle.to_degrees()) as f32,
        radius: radius as f32,
    })
}

fn normalize_degrees(mut d: f64) -> f64 {
    while d > 180.0 {
        d -= 360.0;
    }
    while d <= -180.0 {
        d += 360.0;
    }
    // -0 shows as -0.0.
    if d.abs() < 1e-9 { 0.0 } else { d }
}

/// A polygon (closed) or polyline (open) through the outline's points, its curves
/// as Bezier handles; the frame's origin in the middle of its bounds.
fn path(segs: &[Seg], closed: bool, frame: Frame) -> Made {
    let cubics: Vec<[P2; 4]> = segs
        .iter()
        .flat_map(Seg::cubics)
        .map(|c| c.map(|p| frame.page(p)))
        .collect();
    let all: Vec<V> = cubics
        .iter()
        .flatten()
        .map(|p| [p[0] as f64, p[1] as f64])
        .collect();
    let (lo, hi) = bounds(&all).unwrap_or(([0.0; 2], [0.0; 2]));
    let at = [
        ((lo[0] + hi[0]) / 2.0) as f32,
        ((lo[1] + hi[1]) / 2.0) as f32,
    ];
    let rel = |p: P2| [p[0] - at[0], p[1] - at[1]];
    let mut points: Vec<P2> = cubics.iter().map(|c| rel(c[0])).collect();
    let n = cubics.len();
    let mut handles = vec![[[0.0f32; 2]; 2]; n + 1];
    for (i, c) in cubics.iter().enumerate() {
        handles[i][1] = [c[1][0] - c[0][0], c[1][1] - c[0][1]];
        handles[(i + 1) % if closed { n } else { n + 1 }][0] =
            [c[2][0] - c[3][0], c[2][1] - c[3][1]];
    }
    if closed {
        handles.truncate(n);
    } else if let Some(last) = cubics.last() {
        points.push(rel(last[3]));
    }
    let mut shape = Shape::new(if closed {
        ShapeKind::Polygon
    } else {
        ShapeKind::Polyline
    });
    shape.points = points;
    shape.handles = if handles.iter().flatten().all(|h| *h == [0.0, 0.0]) {
        Vec::new()
    } else {
        handles
    };
    Made::Path {
        shape: Box::new(shape),
        at,
    }
}

// ---- Writing ----

/// A page point (mm, y down) as an item's (y up).
fn item_point(p: P2) -> V {
    [p[0] as f64, -(p[1] as f64)]
}

/// The arc a cubic Bezier draws, if it is one (as written for an arc): its center,
/// radius, start angle and sweep.
fn cubic_arc(c: [V; 4]) -> Option<Seg> {
    let (t0, t1) = (sub(c[1], c[0]), sub(c[3], c[2]));
    let (l0, l1) = (len(t0), len(t1));
    if l0 < 1e-9 || l1 < 1e-9 {
        return None;
    }
    let theta = cross(t0, t1).atan2(dot(t0, t1));
    if theta.abs() < 1e-6 || theta.abs() > PI / 2.0 + 1e-6 {
        return None;
    }
    let chord = len(sub(c[3], c[0]));
    let r = chord / (2.0 * (theta.abs() / 2.0).sin());
    let k = 4.0 / 3.0 * (theta.abs() / 4.0).tan() * r;
    let tol = 1e-3 * r.max(1.0);
    if (l0 - k).abs() > tol || (l1 - k).abs() > tol {
        return None;
    }
    // The center is on the left of the start's tangent for a counter-clockwise turn.
    let d = mul(t0, 1.0 / l0);
    let n = if theta > 0.0 {
        [-d[1], d[0]]
    } else {
        [d[1], -d[0]]
    };
    let center = add(c[0], mul(n, r));
    if (len(sub(c[3], center)) - r).abs() > tol {
        return None;
    }
    let a0 = (c[0][1] - center[1]).atan2(c[0][0] - center[0]);
    Some(Seg::Arc {
        c: center,
        r,
        a0,
        sweep: theta,
    })
}

/// A cubic Bezier as a segment: a line when straight, an arc when it is one.
pub fn cubic_seg(c: [V; 4]) -> Seg {
    if near(c[1], c[0], 1e-9) && near(c[2], c[3], 1e-9) {
        Seg::Line(c[0], c[3])
    } else {
        cubic_arc(c).unwrap_or(Seg::Cubic(c))
    }
}

/// A segment from a cubic on the page.
fn page_seg(c: [P2; 4]) -> Seg {
    cubic_seg(c.map(item_point))
}

/// A closed outline of `segs`.
fn closed(segs: Vec<Seg>) -> Vec<Item> {
    vec![Item::Outline { segs, closed: true }]
}

/// A shape layer as items, where it is on the page: a circle, or outlines of lines, arcs
/// and Beziers.
pub fn layer_items(layer: &Layer) -> Vec<Item> {
    let Some(s) = layer.shape.as_deref() else {
        return Vec::new();
    };
    let distorted = layer.transform.distort != [[0.0; 2]; 4];
    let at = |p: P2| shape::to_page(layer, p);
    if distorted {
        // Not a similarity any more: its curves, cut in short lines.
        return flattened(layer);
    }
    match s.kind {
        ShapeKind::Ellipse => {
            let (Some(c), Some(x), Some(y)) = (
                at([0.0, 0.0]),
                at([s.size[0] / 2.0, 0.0]),
                at([0.0, s.size[1] / 2.0]),
            ) else {
                return Vec::new();
            };
            let (c, x, y) = (item_point(c), item_point(x), item_point(y));
            let (rx, ry) = (len(sub(x, c)), len(sub(y, c)));
            // Equal within what f32 page points keep.
            if (rx - ry).abs() < 1e-4 * rx.max(1.0) {
                return vec![Item::Circle { c, r: rx }];
            }
            // An ellipse: four quarter arcs as Beziers, written as splines.
            let k = 4.0 / 3.0 * (PI / 8.0).tan();
            let (a, b) = (sub(x, c), sub(y, c));
            let p = |u: f64, v: f64| add(c, add(mul(a, u), mul(b, v)));
            let quads = [
                [p(1.0, 0.0), p(1.0, k), p(k, 1.0), p(0.0, 1.0)],
                [p(0.0, 1.0), p(-k, 1.0), p(-1.0, k), p(-1.0, 0.0)],
                [p(-1.0, 0.0), p(-1.0, -k), p(-k, -1.0), p(0.0, -1.0)],
                [p(0.0, -1.0), p(k, -1.0), p(1.0, -k), p(1.0, 0.0)],
            ];
            let segs: Vec<Seg> = quads.into_iter().map(Seg::Cubic).collect();
            closed(segs)
        }
        ShapeKind::Rectangle => {
            let [w, h] = [s.size[0] / 2.0, s.size[1] / 2.0];
            let r = s.corner_radius.clamp(0.0, w.min(h));
            let corners = [[-w, -h], [w, -h], [w, h], [-w, h]];
            let page: Option<Vec<V>> = corners.iter().map(|&p| at(p).map(item_point)).collect();
            let Some(page) = page else {
                return Vec::new();
            };
            if r <= 0.0 {
                let segs: Vec<Seg> = (0..4)
                    .map(|i| Seg::Line(page[i], page[(i + 1) % 4]))
                    .collect();
                return closed(segs);
            }
            // Sides shortened by the radius, joined by quarter arcs (as cubics, read
            // back as arcs).
            let mut segs = Vec::new();
            let pts: Option<Vec<V>> = [
                [-w + r, -h],
                [w - r, -h],
                [w, -h + r],
                [w, h - r],
                [w - r, h],
                [-w + r, h],
                [-w, h - r],
                [-w, -h + r],
            ]
            .iter()
            .map(|&p| at(p).map(item_point))
            .collect();
            let Some(pts) = pts else {
                return Vec::new();
            };
            let k = 4.0 / 3.0 * (PI / 8.0).tan() as f32 * r;
            let handle = |p: P2, d: P2| at([p[0] + d[0], p[1] + d[1]]).map(item_point);
            let frame_pts = [
                [-w + r, -h],
                [w - r, -h],
                [w, -h + r],
                [w, h - r],
                [w - r, h],
                [-w + r, h],
                [-w, h - r],
                [-w, -h + r],
            ];
            // Out of each side's end, along the side; into the next side's start.
            let out_dir = [[k, 0.0], [0.0, k], [-k, 0.0], [0.0, -k]];
            for i in 0..4 {
                let (a, b) = (2 * i, 2 * i + 1);
                segs.push(Seg::Line(pts[a], pts[b]));
                let next = (b + 1) % 8;
                let d = out_dir[i];
                let nd = out_dir[(i + 1) % 4];
                let (Some(h1), Some(h2)) = (
                    handle(frame_pts[b], d),
                    handle(frame_pts[next], [-nd[0], -nd[1]]),
                ) else {
                    return Vec::new();
                };
                let c = [pts[b], h1, h2, pts[next]];
                segs.push(cubic_arc(c).unwrap_or(Seg::Cubic(c)));
            }
            closed(segs)
        }
        k if k.has_points() => {
            let mut out = Vec::new();
            let mut seg = 0;
            for (range, closed) in s.contours() {
                let count = if closed {
                    range.len()
                } else {
                    range.len().saturating_sub(1)
                };
                let segs: Option<Vec<Seg>> = (seg..seg + count)
                    .map(|i| {
                        let c = s.segment(i)?;
                        let c: Option<Vec<P2>> = c.iter().map(|&p| at(p)).collect();
                        let c = c?;
                        Some(page_seg([c[0], c[1], c[2], c[3]]))
                    })
                    .collect();
                seg += count;
                if let Some(segs) = segs {
                    out.push(Item::Outline { segs, closed });
                }
            }
            out
        }
        // Regular polygons and stars: their corners.
        _ => flattened(layer),
    }
}

/// The layer's outline cut in short lines.
fn flattened(layer: &Layer) -> Vec<Item> {
    let Some(s) = layer.shape.as_deref() else {
        return Vec::new();
    };
    let closed = s.kind.closed();
    let curves: Vec<Vec<P2>> = if s.kind.has_points() {
        // One polyline per contour, its segments' curves end to end.
        let mut out = Vec::new();
        let mut seg = 0;
        let curves = shape::page_curves(layer, 16);
        for (range, closed) in s.contours() {
            let count = if closed {
                range.len()
            } else {
                range.len().saturating_sub(1)
            };
            let mut pts: Vec<P2> = Vec::new();
            for c in curves.iter().skip(seg).take(count) {
                let skip = usize::from(!pts.is_empty());
                pts.extend(c.iter().skip(skip));
            }
            seg += count;
            out.push(pts);
        }
        out
    } else {
        shape::page_outline(layer)
            .map(|(pts, _)| vec![pts])
            .unwrap_or_default()
    };
    curves
        .into_iter()
        .filter(|pts| pts.len() > 1)
        .map(|pts| {
            let pts: Vec<V> = pts.into_iter().map(item_point).collect();
            let mut segs: Vec<Seg> = pts.windows(2).map(|w| Seg::Line(w[0], w[1])).collect();
            if closed
                && let (Some(a), Some(b)) = (pts.last(), pts.first())
                && !near(*a, *b, 1e-9)
            {
                segs.push(Seg::Line(*a, *b));
            }
            Item::Outline { segs, closed }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame { dx: 0.0, dy: 0.0 }
    }

    #[test]
    fn bulge_makes_the_arc_it_says() {
        // A half circle counter-clockwise from (1, 0) to (-1, 0) around the origin.
        let s = Seg::bulged([1.0, 0.0], [-1.0, 0.0], 1.0);
        let Seg::Arc { c, r, sweep, .. } = s else {
            panic!("{s:?}");
        };
        assert!(near(c, [0.0, 0.0], 1e-9) && (r - 1.0).abs() < 1e-9);
        assert!((sweep - PI).abs() < 1e-9);
        assert!(near(s.end(), [-1.0, 0.0], 1e-9));
    }

    #[test]
    fn circles_drawn_in_halves_are_circles() {
        // A polyline of two vertices, each bulging a half turn.
        let segs = vec![
            Seg::bulged([10.0, 0.0], [-10.0, 0.0], 1.0),
            Seg::bulged([-10.0, 0.0], [10.0, 0.0], 1.0),
        ];
        let made = make(&Item::Outline { segs, closed: true }, frame());
        let Some(Made::Circle { center, diameter }) = made else {
            panic!("{made:?}");
        };
        assert!(center[0].abs() < 1e-6 && center[1].abs() < 1e-6);
        assert_eq!(diameter, 20.0);
    }

    #[test]
    fn many_sided_polygons_on_a_circle_are_circles() {
        let n = 60;
        let p = |i: usize| polar([5.0, 7.0], 12.0, TAU * i as f64 / n as f64);
        let segs: Vec<Seg> = (0..n).map(|i| Seg::Line(p(i), p((i + 1) % n))).collect();
        let (c, r) = circle(&segs).unwrap();
        assert!(near(c, [5.0, 7.0], 1e-9) && (r - 12.0).abs() < 1e-9);
        // A square is no circle.
        let sq: Vec<Seg> = (0..4)
            .map(|i| {
                let q = |i: usize| polar([0.0, 0.0], 1.0, TAU * i as f64 / 4.0);
                Seg::Line(q(i), q((i + 1) % 4))
            })
            .collect();
        assert_eq!(circle(&sq), None);
    }

    #[test]
    fn rectangles_are_recognized_rounded_or_not() {
        let sq = |a: V, b: V| Seg::Line(a, b);
        let plain = vec![
            sq([0.0, 0.0], [40.0, 0.0]),
            sq([40.0, 0.0], [40.0, 20.0]),
            sq([40.0, 20.0], [0.0, 20.0]),
            sq([0.0, 20.0], [0.0, 0.0]),
        ];
        let Some(Made::Rect {
            size,
            radius,
            rotation,
            center,
        }) = rectangle(&plain, frame())
        else {
            panic!();
        };
        assert_eq!((size, radius, rotation), ([40.0, 20.0], 0.0, 0.0));
        assert_eq!(center, [20.0, -10.0]);
        // Rounded by 5, from a polyline with bulges as CAD writes it.
        let b = (FRAC_PI_2 / 4.0).tan();
        let verts = [
            ([5.0, 0.0], 0.0),
            ([35.0, 0.0], b),
            ([40.0, 5.0], 0.0),
            ([40.0, 15.0], b),
            ([35.0, 20.0], 0.0),
            ([5.0, 20.0], b),
            ([0.0, 15.0], 0.0),
            ([0.0, 5.0], b),
        ];
        let segs: Vec<Seg> = (0..8)
            .map(|i| Seg::bulged(verts[i].0, verts[(i + 1) % 8].0, verts[i].1))
            .collect();
        let Some(Made::Rect { size, radius, .. }) = rectangle(&segs, frame()) else {
            panic!();
        };
        assert_eq!(size, [40.0, 20.0]);
        assert!((radius - 5.0).abs() < 1e-4);
    }

    #[test]
    fn arcs_written_as_cubics_are_read_back_as_arcs() {
        let arc = Seg::Arc {
            c: [3.0, 4.0],
            r: 12.0,
            a0: 0.3,
            sweep: -1.2,
        };
        let c = arc.cubics();
        assert_eq!(c.len(), 1);
        let Some(Seg::Arc {
            c: center,
            r,
            sweep,
            ..
        }) = cubic_arc(c[0])
        else {
            panic!();
        };
        assert!(near(center, [3.0, 4.0], 1e-6), "{center:?}");
        assert!((r - 12.0).abs() < 1e-6 && (sweep + 1.2).abs() < 1e-6);
        // A cubic that is no arc stays one.
        assert_eq!(
            cubic_arc([[0.0, 0.0], [1.0, 3.0], [2.0, -3.0], [3.0, 0.0]]),
            None
        );
    }

    #[test]
    fn pieces_join_whichever_way_they_run() {
        // A square drawn as four lines, two of them the other way round.
        let items = join(vec![
            vec![Seg::Line([0.0, 0.0], [10.0, 0.0])],
            vec![Seg::Line([10.0, 10.0], [10.0, 0.0])],
            vec![Seg::Line([10.0, 10.0], [0.0, 10.0])],
            vec![Seg::Line([0.0, 0.0], [0.0, 10.0])],
        ]);
        assert_eq!(items.len(), 1);
        let Item::Outline { segs, closed } = &items[0] else {
            panic!();
        };
        assert!(*closed);
        assert_eq!(segs.len(), 4);
        for w in segs.windows(2) {
            assert!(near(w[0].end(), w[1].start(), 1e-9));
        }
        assert!(matches!(rectangle(segs, frame()), Some(Made::Rect { .. })));
    }

    #[test]
    fn splines_are_evaluated() {
        // A degree-1 spline is its control polygon.
        let pts = spline_points(1, &[0.0, 0.0, 1.0, 1.0], &[[0.0, 0.0], [4.0, 2.0]]);
        assert!(near(pts[0], [0.0, 0.0], 1e-9) && near(*pts.last().unwrap(), [4.0, 2.0], 1e-9));
        // A Bezier's ends are its first and last control points.
        let pts = spline_points(
            3,
            &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            &[[0.0, 0.0], [1.0, 2.0], [3.0, 2.0], [4.0, 0.0]],
        );
        assert!(near(*pts.last().unwrap(), [4.0, 0.0], 1e-9));
        assert!(near(pts[8], [2.0, 1.5], 1e-9), "{:?}", pts[8]);
    }
}
