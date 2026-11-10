//! Painted strokes: what the pencil and the brush leave in a closed shape, and how the
//! eraser cuts them.
//!
//! A stroke is a line through points, each with its own width: even for the pencil,
//! tapered at both ends for the round brush, following the pen's pressure when the
//! system reports it. Strokes are vectors: a stroke is drawn as one filled outline, made
//! of pieces (a disc at each point and the band joining each point to the next, or the
//! shapes of its [`Tip`]) all turning the same way so that the non-zero rule fills their
//! union once (no darker overlaps with a see-through color), on screen and in the PDF
//! alike.
//!
//! The brush comes in several kinds ([`BrushKind`]): round, calligraphy (a flat nib at an
//! angle), marker (square ends, see-through), spray (scattered dots) and bristle (thin
//! lines side by side, each a stroke of its own).

use serde::{Deserialize, Serialize};
use tiny_skia::{Path, PathBuilder};

use crate::document::P2;
use crate::i18n::tr_noop;

/// A painted stroke, in the frame of the shape it is painted in (mm).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaintStroke {
    /// Points along it: x, y and the stroke's width there, in mm.
    pub points: Vec<[f32; 3]>,
    /// Straight RGBA.
    pub color: [u8; 4],
    /// How it is drawn from its points.
    #[serde(default, skip_serializing_if = "Tip::is_round")]
    pub tip: Tip,
}

/// How a stroke is drawn from its points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Tip {
    /// Round ends and joins.
    #[default]
    Round,
    /// A flat nib held at `angle` (radians from the x axis, in the shape's frame), as
    /// long as the width: broad across it, thin along it.
    Nib { angle: f32 },
    /// Square ends, round joins.
    Square,
    /// A dot at each point, not joined.
    Dots,
}

impl Tip {
    fn is_round(&self) -> bool {
        *self == Tip::Round
    }
}

/// The kinds of brush, each leaving its own sort of stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrushKind {
    /// Thinning out at both ends.
    #[default]
    Round,
    /// A flat nib at an angle: thick or thin depending on the stroke's direction.
    Calligraphy,
    /// Even, with square ends and see-through.
    Marker,
    /// Dots scattered around the line, denser in the middle.
    Spray,
    /// Thin lines side by side with gaps between them, like a dry brush.
    Bristle,
}

impl BrushKind {
    /// In the order of the brush menu.
    pub const ALL: [BrushKind; 5] = [
        BrushKind::Round,
        BrushKind::Calligraphy,
        BrushKind::Marker,
        BrushKind::Spray,
        BrushKind::Bristle,
    ];

    /// Translation key of its name.
    pub fn name_key(self) -> &'static str {
        match self {
            BrushKind::Round => tr_noop!("BRUSH_ROUND"),
            BrushKind::Calligraphy => tr_noop!("BRUSH_CALLIGRAPHY"),
            BrushKind::Marker => tr_noop!("BRUSH_MARKER"),
            BrushKind::Spray => tr_noop!("BRUSH_SPRAY"),
            BrushKind::Bristle => tr_noop!("BRUSH_BRISTLE"),
        }
    }
}

/// What paints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    /// An even line.
    #[default]
    Pencil,
    /// A line thinning out at both ends.
    Brush,
    /// Cuts the strokes it passes over.
    Eraser,
}

impl Tool {
    fn index(self) -> usize {
        self as usize
    }
}

/// The paint tools' settings, each tool keeping its own width.
#[derive(Clone, Debug, PartialEq)]
pub struct Brushes {
    /// The tool in use; `None` when selecting and moving layers.
    pub tool: Option<Tool>,
    /// Straight RGBA, for the pencil and the brush.
    pub color: [u8; 4],
    /// Pencil, brush and eraser widths, in mm on the page.
    pub widths: [f32; 3],
    /// The brush's kind.
    pub brush: BrushKind,
    /// The calligraphy nib's angle on the page, in degrees from the horizontal,
    /// counterclockwise.
    pub nib_angle: f32,
    /// How thick the spray is, from 0 to 1.
    pub spray_density: f32,
}

impl Default for Brushes {
    fn default() -> Self {
        Self {
            tool: None,
            color: [0, 0, 0, 255],
            widths: [0.5, 3.0, 4.0],
            brush: BrushKind::Round,
            nib_angle: 45.0,
            spray_density: 0.5,
        }
    }
}

impl Brushes {
    pub fn width(&self, tool: Tool) -> f32 {
        self.widths[tool.index()]
    }

    pub fn width_mut(&mut self, tool: Tool) -> &mut f32 {
        &mut self.widths[tool.index()]
    }

    /// What `tool` paints with now, for a stroke painted through `samples` on the page:
    /// they also pick its pattern, so that it looks the same while it is painted and once
    /// it is added.
    pub fn pen(&self, tool: Tool, samples: &[[f32; 3]]) -> Pen {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(s) = samples.first() {
            s.map(f32::to_bits).hash(&mut h);
        }
        Pen {
            brush: self.brush,
            // Counterclockwise on the page, y going down.
            angle: -self.nib_angle.to_radians(),
            density: self.spray_density,
            pattern: h.finish(),
            ..Pen::new(tool, self.width(tool), self.color)
        }
    }
}

/// What a stroke is painted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
    pub tool: Tool,
    /// The brush's kind, when `tool` is the brush.
    pub brush: BrushKind,
    /// In mm.
    pub width: f32,
    /// Straight RGBA.
    pub color: [u8; 4],
    /// The calligraphy nib's angle, in radians from the x axis.
    pub angle: f32,
    /// How thick the spray is, from 0 to 1.
    pub density: f32,
    /// Which of the possible scatterings of the spray's dots and the bristles' lines it
    /// paints.
    pub pattern: u64,
}

impl Pen {
    /// `tool` with a round brush, a 45 degree nib and a medium spray.
    pub fn new(tool: Tool, width: f32, color: [u8; 4]) -> Self {
        Self {
            tool,
            brush: BrushKind::Round,
            width,
            color,
            angle: -std::f32::consts::FRAC_PI_4,
            density: 0.5,
            pattern: 1,
        }
    }

    /// The brush of `kind`.
    #[cfg(test)]
    pub fn brush(kind: BrushKind, width: f32, color: [u8; 4]) -> Self {
        Self {
            brush: kind,
            ..Self::new(Tool::Brush, width, color)
        }
    }

    /// The kind of brush it paints with; `None` for the pencil and the eraser.
    fn brush_kind(&self) -> Option<BrushKind> {
        (self.tool == Tool::Brush).then_some(self.brush)
    }
}

/// Points of a stroke closer than this to the line through their neighbors are left
/// out, in mm: well under what shows in print.
const TOLERANCE: f32 = 0.02;
/// Samples closer than this to the previous one are skipped, in mm.
const MIN_STEP: f32 = 0.05;
/// A brush stroke's ends thin out over this many widths, at most a quarter of its
/// length each, down to this fraction of its width.
const TAPER_WIDTHS: f32 = 4.0;
const TAPER_MIN: f32 = 0.15;
/// A calligraphy nib's thickness, as a fraction of its width: what is left of a stroke
/// along it.
const NIB_THICKNESS: f32 = 0.12;
/// A marker's opacity, as a fraction of its color's.
const MARKER_OPACITY: f32 = 0.5;
/// A spray's dots across, as a fraction of its width (at random from half to 1.5 times
/// that).
const SPRAY_DOT: f32 = 0.04;
/// The finest spray dot, in mm.
const SPRAY_DOT_MIN: f32 = 0.08;
/// A bristle brush's bristles: one per this many mm of its width, within these counts.
const BRISTLE_SPACING: f32 = 0.9;
const BRISTLE_MIN: usize = 3;
const BRISTLE_MAX: usize = 12;

impl PaintStroke {
    /// The strokes painted with `pen` through the samples taken while painting (x, y in
    /// mm, pen pressure from 0 to 1, 1 without a pen): smoothed and simplified, shaped
    /// as the pen's kind says. A bristle brush leaves one stroke per bristle; nothing
    /// when nothing was painted.
    pub fn paint(samples: &[[f32; 3]], pen: &Pen) -> Vec<Self> {
        let width = pen.width;
        let mut pts: Vec<[f32; 3]> = Vec::new();
        for &[x, y, p] in samples {
            if pts
                .last()
                .is_none_or(|l| dist([l[0], l[1]], [x, y]) >= MIN_STEP)
            {
                pts.push([x, y, width * p.clamp(0.05, 1.0)]);
            }
        }
        if pts.is_empty() {
            return Vec::new();
        }
        // A light smoothing of the hand's jitter, the ends kept where they are.
        if pts.len() > 2 {
            let raw = pts.clone();
            for i in 1..raw.len() - 1 {
                for k in 0..3 {
                    pts[i][k] = (raw[i - 1][k] + 2.0 * raw[i][k] + raw[i + 1][k]) / 4.0;
                }
            }
        }
        let stroke = |points: Vec<[f32; 3]>, color: [u8; 4], tip: Tip| Self { points, color, tip };
        match pen.brush_kind() {
            None => vec![stroke(simplify(&pts), pen.color, Tip::Round)],
            Some(BrushKind::Round) => {
                taper(&mut pts, width);
                vec![stroke(simplify(&pts), pen.color, Tip::Round)]
            }
            Some(BrushKind::Calligraphy) => {
                let tip = Tip::Nib { angle: pen.angle };
                vec![stroke(simplify(&pts), pen.color, tip)]
            }
            Some(BrushKind::Marker) => {
                let mut color = pen.color;
                color[3] = (color[3] as f32 * MARKER_OPACITY).round() as u8;
                vec![stroke(simplify(&pts), color, Tip::Square)]
            }
            Some(BrushKind::Spray) => vec![stroke(spray(&pts, pen), pen.color, Tip::Dots)],
            Some(BrushKind::Bristle) => bristles(&pts, pen)
                .into_iter()
                .map(|b| stroke(b, pen.color, Tip::Round))
                .collect(),
        }
    }

    /// Its outline, to fill under the non-zero rule.
    pub fn outline(&self) -> Option<Path> {
        let mut pb = PathBuilder::new();
        let pts = &self.points;
        match self.tip {
            Tip::Round => {
                for (i, p) in pts.iter().enumerate() {
                    disc(&mut pb, [p[0], p[1]], p[2] / 2.0);
                    if let Some(q) = pts.get(i + 1) {
                        band(&mut pb, *p, *q);
                    }
                }
            }
            Tip::Dots => {
                for p in pts {
                    disc(&mut pb, [p[0], p[1]], p[2] / 2.0);
                }
            }
            Tip::Nib { angle } => {
                let (sin, cos) = angle.sin_cos();
                // The nib's four corners at a point.
                let nib = |p: &[f32; 3]| -> [P2; 4] {
                    let h = [cos * p[2] / 2.0, sin * p[2] / 2.0];
                    let t = p[2] * NIB_THICKNESS / 2.0;
                    let n = [-sin * t, cos * t];
                    [
                        [p[0] - h[0] - n[0], p[1] - h[1] - n[1]],
                        [p[0] + h[0] - n[0], p[1] + h[1] - n[1]],
                        [p[0] + h[0] + n[0], p[1] + h[1] + n[1]],
                        [p[0] - h[0] + n[0], p[1] - h[1] + n[1]],
                    ]
                };
                if let [p] = pts.as_slice() {
                    polygon(&mut pb, &nib(p));
                }
                // Swept from each point to the next.
                for w in pts.windows(2) {
                    let mut corners = nib(&w[0]).to_vec();
                    corners.extend(nib(&w[1]));
                    polygon(&mut pb, &convex_hull(corners));
                }
            }
            Tip::Square => {
                if let [p] = pts.as_slice() {
                    let r = p[2] / 2.0;
                    polygon(
                        &mut pb,
                        &[
                            [p[0] - r, p[1] - r],
                            [p[0] + r, p[1] - r],
                            [p[0] + r, p[1] + r],
                            [p[0] - r, p[1] + r],
                        ],
                    );
                }
                // The ends pushed out by half the width, round joins in between.
                let mut line = pts.clone();
                let n = line.len();
                if n >= 2 {
                    for (end, next) in [(0, 1), (n - 1, n - 2)] {
                        let (e, q) = (line[end], line[next]);
                        let l = dist([e[0], e[1]], [q[0], q[1]]);
                        if l > 1e-6 {
                            let k = e[2] / 2.0 / l;
                            line[end][0] += (e[0] - q[0]) * k;
                            line[end][1] += (e[1] - q[1]) * k;
                        }
                    }
                }
                for (i, w) in line.windows(2).enumerate() {
                    band(&mut pb, w[0], w[1]);
                    if i > 0 {
                        disc(&mut pb, [w[0][0], w[0][1]], w[0][2] / 2.0);
                    }
                }
            }
        }
        pb.finish()
    }

    /// Smallest box holding it: min and max corners.
    pub fn bounds(&self) -> Option<(P2, P2)> {
        // A square end's corners reach further than half its width from its point.
        let reach = match self.tip {
            Tip::Square => std::f32::consts::SQRT_2 / 2.0,
            _ => 0.5,
        };
        let mut it = self.points.iter();
        let f = it.next()?;
        let r = f[2] * reach;
        let mut b = ([f[0] - r, f[1] - r], [f[0] + r, f[1] + r]);
        for p in it {
            let r = p[2] * reach;
            b.0 = [b.0[0].min(p[0] - r), b.0[1].min(p[1] - r)];
            b.1 = [b.1[0].max(p[0] + r), b.1[1].max(p[1] + r)];
        }
        Some(b)
    }

    /// Scales it by `k` around `about`; its width by the mean of both.
    pub fn scale(&mut self, k: P2, about: P2) {
        let kw = (k[0].abs() * k[1].abs()).sqrt();
        for p in &mut self.points {
            p[0] = about[0] + (p[0] - about[0]) * k[0];
            p[1] = about[1] + (p[1] - about[1]) * k[1];
            p[2] *= kw;
        }
        self.turn_nib(|v| [v[0] * k[0], v[1] * k[1]]);
    }

    /// Turns a calligraphy nib as `f` turns a direction, when its points are mapped by a
    /// linear map `f` is the matrix of.
    pub fn turn_nib(&mut self, f: impl Fn(P2) -> P2) {
        if let Tip::Nib { angle } = &mut self.tip {
            let v = f([angle.cos(), angle.sin()]);
            if v[0] != 0.0 || v[1] != 0.0 {
                *angle = v[1].atan2(v[0]);
            }
        }
    }
}

/// Cuts `strokes` where the eraser, a disc of `radius` mm, passed along `path` (mm):
/// what it covers goes, a stroke crossed in the middle becomes two. Returns true if
/// anything changed.
pub fn erase(strokes: &mut Vec<PaintStroke>, path: &[P2], radius: f32) -> bool {
    if path.is_empty() {
        return false;
    }
    let mut changed = false;
    let mut out = Vec::with_capacity(strokes.len());
    // The box the eraser swept.
    let lo = path
        .iter()
        .fold([f32::INFINITY; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
    let hi = path.iter().fold([f32::NEG_INFINITY; 2], |m, p| {
        [m[0].max(p[0]), m[1].max(p[1])]
    });
    for stroke in strokes.drain(..) {
        // Far from the eraser: kept as it is.
        let touched = stroke.bounds().is_some_and(|(a, b)| {
            a[0] < hi[0] + radius
                && b[0] > lo[0] - radius
                && a[1] < hi[1] + radius
                && b[1] > lo[1] - radius
        });
        if !touched {
            out.push(stroke);
            continue;
        }
        // Separate dots go one by one.
        if stroke.tip == Tip::Dots {
            let n = stroke.points.len();
            let points: Vec<[f32; 3]> = stroke
                .points
                .into_iter()
                .filter(|p| distance_to_path([p[0], p[1]], path) >= radius + p[2] / 2.0)
                .collect();
            changed |= points.len() != n;
            if !points.is_empty() {
                out.push(PaintStroke { points, ..stroke });
            }
            continue;
        }
        // Points close enough together that a cut falls where the eraser passed.
        let step = (radius / 3.0).max(MIN_STEP);
        let dense = resample(&stroke.points, step);
        let gone = |p: &[f32; 3]| distance_to_path([p[0], p[1]], path) < radius + p[2] / 4.0;
        if !dense.iter().any(gone) {
            out.push(stroke);
            continue;
        }
        changed = true;
        let mut run: Vec<[f32; 3]> = Vec::new();
        let mut flush = |run: &mut Vec<[f32; 3]>| {
            // A sliver shorter than its own width is not kept.
            let length: f32 = run
                .windows(2)
                .map(|w| dist([w[0][0], w[0][1]], [w[1][0], w[1][1]]))
                .sum();
            let width = run.iter().map(|p| p[2]).fold(0.0, f32::max);
            if !run.is_empty() && length >= width {
                out.push(PaintStroke {
                    points: simplify(run),
                    color: stroke.color,
                    tip: stroke.tip,
                });
            }
            run.clear();
        };
        for p in &dense {
            if gone(p) {
                flush(&mut run);
            } else {
                run.push(*p);
            }
        }
        flush(&mut run);
    }
    *strokes = out;
    changed
}

fn dist(a: P2, b: P2) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Distance from `p` to the line through `path`'s points.
fn distance_to_path(p: P2, path: &[P2]) -> f32 {
    if path.len() == 1 {
        return dist(p, path[0]);
    }
    path.windows(2)
        .map(|w| distance_to_segment(p, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

fn distance_to_segment(p: P2, a: P2, b: P2) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let dd = d[0] * d[0] + d[1] * d[1];
    let t = if dd > 1e-12 {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / dd).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(p, [a[0] + d[0] * t, a[1] + d[1] * t])
}

/// The stroke's points, with more in between so none is further than `step` from the
/// next, widths following along.
fn resample(points: &[[f32; 3]], step: f32) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for (i, p) in points.iter().enumerate() {
        out.push(*p);
        if let Some(q) = points.get(i + 1) {
            let n = (dist([p[0], p[1]], [q[0], q[1]]) / step).ceil() as usize;
            for k in 1..n {
                let t = k as f32 / n as f32;
                out.push(std::array::from_fn(|c| p[c] + (q[c] - p[c]) * t));
            }
        }
    }
    out
}

/// Thins out both ends of a brush stroke of `width`.
fn taper(pts: &mut [[f32; 3]], width: f32) {
    let mut along = vec![0.0f32];
    for w in pts.windows(2) {
        along.push(along.last().unwrap() + dist([w[0][0], w[0][1]], [w[1][0], w[1][1]]));
    }
    let total = *along.last().unwrap();
    let reach = (TAPER_WIDTHS * width).min(total / 4.0).max(1e-6);
    for (p, &d) in pts.iter_mut().zip(&along) {
        let end = d.min(total - d) / reach;
        let x = end.clamp(0.0, 1.0);
        let ease = x * x * (3.0 - 2.0 * x);
        p[2] *= TAPER_MIN + (1.0 - TAPER_MIN) * ease;
    }
}

/// Leaves out the points the stroke does not need: those within `TOLERANCE` of the
/// line through their neighbors, widths counted like distances (Douglas-Peucker).
fn simplify(pts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut todo = vec![(0, pts.len() - 1)];
    while let Some((a, b)) = todo.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let mut far = (0.0, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            // Distance in x, y and width from the straight line between the ends.
            let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let dd: f32 = d.iter().map(|v| v * v).sum();
            let s = if dd > 1e-12 {
                ((0..3).map(|k| (p[k] - pa[k]) * d[k]).sum::<f32>() / dd).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let e: f32 = (0..3)
                .map(|k| (p[k] - (pa[k] + d[k] * s)).powi(2))
                .sum::<f32>()
                .sqrt();
            if e > far.0 {
                far = (e, i);
            }
        }
        if far.0 > TOLERANCE {
            keep[far.1] = true;
            todo.push((a, far.1));
            todo.push((far.1, b));
        }
    }
    pts.iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// The spray's dots along `pts` (x, y and the spray's width there): scattered across
/// its width, denser in the middle, as many as `pen.density` says.
fn spray(pts: &[[f32; 3]], pen: &Pen) -> Vec<[f32; 3]> {
    let mut rng = Scatter::new(pen.pattern);
    // Cover up to this share of the spray's area at full density.
    let cover = 0.35 * pen.density.clamp(0.0, 1.0);
    let step = (pen.width / 4.0).max(MIN_STEP);
    let line = resample(pts, step);
    let mut dots = Vec::new();
    for (i, p) in line.iter().enumerate() {
        let next = line.get(i + 1).copied();
        // The stretch of line up to the next point; a click sprays a dab.
        let len = match next {
            Some(q) => dist([p[0], p[1]], [q[0], q[1]]),
            None if line.len() == 1 => p[2] / 2.0,
            None => 0.0,
        };
        let dot = (p[2] * SPRAY_DOT).max(SPRAY_DOT_MIN);
        let per_mm2 = cover / (std::f32::consts::PI * dot * dot / 4.0);
        let count = per_mm2 * p[2] * len + rng.next();
        for _ in 0..count as usize {
            if dots.len() >= MAX_SPRAY_DOTS {
                return dots;
            }
            let q = next.unwrap_or(*p);
            let t = rng.next();
            let at = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
            // Uniform in distance from the line: denser near it.
            let r = p[2] / 2.0 * rng.next();
            let a = std::f32::consts::TAU * rng.next();
            let size = dot * (0.5 + rng.next());
            dots.push([at[0] + r * a.cos(), at[1] + r * a.sin(), size]);
        }
    }
    dots
}

/// The bristle brush's lines along `pts` (x, y and the brush's width there): side by
/// side across its width, each a little thinner than its share of it and swelling and
/// thinning along the way, starting and ending a little apart, some running dry for a
/// stretch, thinning out at their ends.
fn bristles(pts: &[[f32; 3]], pen: &Pen) -> Vec<Vec<[f32; 3]>> {
    let mut rng = Scatter::new(pen.pattern);
    let n = ((pen.width / BRISTLE_SPACING).round() as usize).clamp(BRISTLE_MIN, BRISTLE_MAX);
    let last = pts.len() - 1;
    // The unit normal at each point.
    let normals: Vec<P2> = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(last)]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if l > 1e-6 {
                [-d[1] / l, d[0] / l]
            } else {
                [0.0, 1.0]
            }
        })
        .collect();
    let mut along = vec![0.0f32];
    for w in pts.windows(2) {
        along.push(along[along.len() - 1] + dist([w[0][0], w[0][1]], [w[1][0], w[1][1]]));
    }
    let total = along[last];
    let mut out = Vec::new();
    for i in 0..n {
        let share = 1.0 / n as f32;
        // Across the width, from -1/2 to 1/2, a little off even spacing.
        let offset = (i as f32 + 0.5) * share - 0.5 + (rng.next() - 0.5) * 0.3 * share;
        let thickness = share * (0.4 + 0.5 * rng.next());
        let from = rng.next() * 0.12 * total;
        let to = total - rng.next() * 0.12 * total;
        // Swelling and thinning along it, over a few mm.
        let period = pen.width * (1.0 + 2.0 * rng.next());
        let phase = std::f32::consts::TAU * rng.next();
        // A stretch where it runs dry, for some.
        let dry = if rng.next() < 0.4 {
            let at = from + (to - from) * rng.next();
            at..at + total * (0.05 + 0.1 * rng.next())
        } else {
            0.0..0.0
        };
        let mut line: Vec<[f32; 3]> = Vec::new();
        let mut runs = Vec::new();
        for ((p, n), &d) in pts.iter().zip(&normals).zip(&along) {
            if d < from || d > to {
                continue;
            }
            if dry.contains(&d) {
                runs.push(std::mem::take(&mut line));
                continue;
            }
            let swell = 0.7 + 0.3 * (phase + d / period * std::f32::consts::TAU).sin();
            line.push([
                p[0] + n[0] * offset * p[2],
                p[1] + n[1] * offset * p[2],
                p[2] * thickness * swell,
            ]);
        }
        runs.push(line);
        for mut run in runs.into_iter().filter(|r| !r.is_empty()) {
            taper(&mut run, pen.width * thickness);
            out.push(simplify(&run));
        }
    }
    out
}

/// The most dots one spray stroke lays.
const MAX_SPRAY_DOTS: usize = 20_000;

/// Numbers from 0 to 1 that look random, the same ones each time for the same pattern
/// (SplitMix64).
struct Scatter(u64);

impl Scatter {
    fn new(pattern: u64) -> Self {
        Self(pattern)
    }

    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// The convex hull of `pts`, its corners in turn.
fn convex_hull(mut pts: Vec<P2>) -> Vec<P2> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    // Andrew's monotone chain: the lower then the upper side.
    let mut hull: Vec<P2> = Vec::with_capacity(pts.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let it: Box<dyn Iterator<Item = &P2>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in it {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// A polygon, turned to go clockwise on screen (y down) like [`disc`].
fn polygon(pb: &mut PathBuilder, pts: &[P2]) {
    let n = pts.len();
    if n < 3 {
        return;
    }
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area.abs() < 1e-12 {
        return;
    }
    let mut order: Vec<P2> = pts.to_vec();
    if area < 0.0 {
        order.reverse();
    }
    pb.move_to(order[0][0], order[0][1]);
    for p in &order[1..] {
        pb.line_to(p[0], p[1]);
    }
    pb.close();
}

/// Control points of a quarter circle drawn as a cubic curve, as a fraction of its radius.
const KAPPA: f32 = 0.552_284_8;

/// A disc, turning clockwise on screen (y down), like [`band`].
fn disc(pb: &mut PathBuilder, c: P2, r: f32) {
    if r <= 0.0 {
        return;
    }
    let k = r * KAPPA;
    let [x, y] = c;
    pb.move_to(x + r, y);
    pb.cubic_to(x + r, y + k, x + k, y + r, x, y + r);
    pb.cubic_to(x - k, y + r, x - r, y + k, x - r, y);
    pb.cubic_to(x - r, y - k, x - k, y - r, x, y - r);
    pb.cubic_to(x + k, y - r, x + r, y - k, x + r, y);
    pb.close();
}

/// The band from `a` to `b`, as wide as each end's width there, turning clockwise on
/// screen like [`disc`].
fn band(pb: &mut PathBuilder, a: [f32; 3], b: [f32; 3]) {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if l < 1e-6 {
        return;
    }
    // The normal to the right of the direction, on screen.
    let n = [-d[1] / l, d[0] / l];
    let (ra, rb) = (a[2] / 2.0, b[2] / 2.0);
    let c = [
        [a[0] - n[0] * ra, a[1] - n[1] * ra],
        [b[0] - n[0] * rb, b[1] - n[1] * rb],
        [b[0] + n[0] * rb, b[1] + n[1] * rb],
        [a[0] + n[0] * ra, a[1] + n[1] * ra],
    ];
    pb.move_to(c[0][0], c[0][1]);
    for p in &c[1..] {
        pb.line_to(p[0], p[1]);
    }
    pb.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed area of a closed polygon, positive turning clockwise on screen (y down).
    fn area(pts: &[P2]) -> f32 {
        let n = pts.len();
        (0..n)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f32>()
            / 2.0
    }

    #[test]
    fn discs_and_bands_turn_the_same_way() {
        // The disc's quarter points, in drawing order.
        let disc = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
        let mut pb = PathBuilder::new();
        band(&mut pb, [0.0, 0.0, 2.0], [10.0, 3.0, 1.0]);
        let path = pb.finish().unwrap();
        let pts: Vec<P2> = path.points().iter().map(|p| [p.x, p.y]).collect();
        assert!(area(&disc) > 0.0 && area(&pts[..4]) > 0.0);
        // Either way it goes.
        let mut pb = PathBuilder::new();
        band(&mut pb, [10.0, 3.0, 1.0], [0.0, 0.0, 2.0]);
        let path = pb.finish().unwrap();
        let pts: Vec<P2> = path.points().iter().map(|p| [p.x, p.y]).collect();
        assert!(area(&pts[..4]) > 0.0);
    }

    /// The one stroke `pen` paints through `samples`.
    fn one(samples: &[[f32; 3]], pen: Pen) -> PaintStroke {
        let mut s = PaintStroke::paint(samples, &pen);
        assert_eq!(s.len(), 1);
        s.remove(0)
    }

    fn line(n: usize) -> Vec<[f32; 3]> {
        (0..n).map(|i| [i as f32 * 0.5, 0.0, 1.0]).collect()
    }

    #[test]
    fn a_straight_line_keeps_its_ends_only_and_the_brush_tapers() {
        let s = one(&line(41), Pen::new(Tool::Pencil, 2.0, [0, 0, 0, 255]));
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.points[0], [0.0, 0.0, 2.0]);
        assert_eq!(s.points[1], [20.0, 0.0, 2.0]);
        let b = one(&line(41), Pen::new(Tool::Brush, 2.0, [0, 0, 0, 255]));
        assert!(b.points.len() > 2, "its widths change at the ends");
        let w = |s: &PaintStroke, x: f32| {
            s.points
                .iter()
                .min_by(|a, b| (a[0] - x).abs().total_cmp(&(b[0] - x).abs()))
                .unwrap()[2]
        };
        assert!(w(&b, 0.0) < 0.5 && w(&b, 20.0) < 0.5);
        assert!((w(&b, 10.0) - 2.0).abs() < 0.01);
        // Pressure thins it.
        let soft: Vec<[f32; 3]> = line(41).iter().map(|p| [p[0], p[1], 0.5]).collect();
        let p = one(&soft, Pen::new(Tool::Pencil, 2.0, [0; 4]));
        assert!((p.points[0][2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn the_eraser_cuts_a_stroke_in_two() {
        let mut strokes = vec![one(&line(41), Pen::new(Tool::Pencil, 1.0, [0, 0, 0, 255]))];
        // Across the middle, 2 mm wide.
        assert!(erase(&mut strokes, &[[10.0, -5.0], [10.0, 5.0]], 1.0));
        assert_eq!(strokes.len(), 2);
        let (a, b) = (strokes[0].bounds().unwrap(), strokes[1].bounds().unwrap());
        assert!(a.1[0] < 9.5 && a.0[0] < 0.0, "{a:?}");
        assert!(b.0[0] > 10.5 && b.1[0] > 20.0, "{b:?}");
        // Away from them: nothing.
        assert!(!erase(&mut strokes, &[[10.0, 30.0]], 1.0));
        // Over a whole stroke: gone.
        assert!(erase(&mut strokes, &[[-1.0, 0.0], [9.0, 0.0]], 1.0));
        assert_eq!(strokes.len(), 1);
    }

    #[test]
    fn overlapping_pieces_are_filled_once() {
        // A see-through stroke turning back on itself: where it crosses, it is no
        // darker than elsewhere.
        let pts = vec![
            [0.0, 0.0, 4.0],
            [20.0, 0.0, 4.0],
            [10.0, 10.0, 4.0],
            [10.0, -10.0, 4.0],
        ];
        let s = PaintStroke {
            points: pts,
            color: [0, 0, 0, 128],
            tip: Tip::Round,
        };
        let mut pm = tiny_skia::Pixmap::new(40, 40).unwrap();
        let paint = crate::gradient::Ink::Solid(s.color).paint();
        let ts = tiny_skia::Transform::from_translate(10.0, 15.0);
        pm.fill_path(
            &s.outline().unwrap(),
            &paint,
            tiny_skia::FillRule::Winding,
            ts,
            None,
        );
        let a = |x, y| pm.pixel(x, y).unwrap().alpha();
        // The crossing (10, 0) and a plain part (4, 0), on the pixmap.
        assert_eq!(a(20, 15), a(14, 15));
        assert!(a(20, 15) > 100 && a(20, 15) < 160);
    }

    /// Filled pixels of `s` drawn 4 pixels per mm, moved by `at` mm.
    fn ink(s: &PaintStroke, at: P2, size: u32) -> tiny_skia::Pixmap {
        let mut pm = tiny_skia::Pixmap::new(size, size).unwrap();
        let paint = crate::gradient::Ink::Solid(s.color).paint();
        let ts = tiny_skia::Transform::from_translate(at[0], at[1]).post_scale(4.0, 4.0);
        pm.fill_path(
            &s.outline().unwrap(),
            &paint,
            tiny_skia::FillRule::Winding,
            ts,
            None,
        );
        pm
    }

    #[test]
    fn a_calligraphy_nib_is_broad_across_and_thin_along() {
        // Flat nib: a horizontal line is thin, a vertical one broad.
        let pen = Pen {
            angle: 0.0,
            ..Pen::brush(BrushKind::Calligraphy, 4.0, [0, 0, 0, 255])
        };
        let across: Vec<[f32; 3]> = (0..41).map(|i| [10.0, i as f32 * 0.5, 1.0]).collect();
        let along: Vec<[f32; 3]> = (0..41).map(|i| [i as f32 * 0.5, 10.0, 1.0]).collect();
        let (a, b) = (one(&across, pen), one(&along, pen));
        assert_eq!(a.tip, Tip::Nib { angle: 0.0 });
        let ab = a.bounds().unwrap();
        assert!((ab.1[0] - ab.0[0] - 4.0).abs() < 0.01, "{ab:?}");
        // What is filled across the horizontal line: the nib's thickness only.
        let pm = ink(&b, [0.0, 0.0], 100);
        let filled = (0..100)
            .filter(|&y| pm.pixel(40, y).unwrap().alpha() > 128)
            .count();
        assert!(filled as f32 <= 4.0 * 4.0 * NIB_THICKNESS + 2.0, "{filled}");
        let pm = ink(&a, [0.0, 0.0], 100);
        let filled = (0..100)
            .filter(|&x| pm.pixel(x, 40).unwrap().alpha() > 128)
            .count();
        assert!(filled >= 15, "{filled}");
        // Turned with the stroke when it is stretched: a quarter turn stays one.
        let mut c = a.clone();
        c.turn_nib(|v| [-v[1], v[0]]);
        let Tip::Nib { angle } = c.tip else { panic!() };
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn a_marker_is_see_through_with_square_ends() {
        let m = one(
            &line(41),
            Pen::brush(BrushKind::Marker, 2.0, [0, 0, 0, 200]),
        );
        assert_eq!((m.tip, m.color[3]), (Tip::Square, 100));
        // Its ends reach half its width past the line's, square.
        let pm = ink(&m, [4.0, 4.0], 120);
        let a = |x: f32, y: f32| {
            pm.pixel((x * 4.0) as u32, (y * 4.0) as u32)
                .unwrap()
                .alpha()
        };
        assert!(a(4.0 - 0.8, 4.0 - 0.8) > 0, "the corner of the square end");
        assert_eq!(a(4.0 - 1.2, 4.0), 0);
        assert!(a(24.0 + 0.8, 4.0 + 0.8) > 0);
    }

    #[test]
    fn a_spray_scatters_dots_the_same_way_each_time_and_the_eraser_takes_some() {
        let pen = Pen::brush(BrushKind::Spray, 4.0, [0, 0, 0, 255]);
        let s = one(&line(41), pen);
        assert_eq!(s.tip, Tip::Dots);
        assert!(s.points.len() > 100, "{}", s.points.len());
        assert!(s.points.iter().all(|p| p[1].abs() <= 2.0 && p[2] > 0.0));
        // Denser near the line.
        let near = s.points.iter().filter(|p| p[1].abs() < 0.5).count();
        let far = s.points.iter().filter(|p| p[1].abs() > 1.5).count();
        assert!(near > far, "{near} {far}");
        assert_eq!(s, one(&line(41), pen), "the same pattern");
        let thin = one(
            &line(41),
            Pen {
                density: 0.1,
                ..pen
            },
        );
        assert!(thin.points.len() * 3 < s.points.len());
        // A click sprays a dab.
        assert!(!one(&line(1), pen).points.is_empty());
        // The eraser takes the dots it passes over, one by one.
        let mut strokes = vec![s.clone()];
        assert!(erase(&mut strokes, &[[10.0, -5.0], [10.0, 5.0]], 1.0));
        assert_eq!(strokes.len(), 1);
        let left = &strokes[0].points;
        assert!(left.len() < s.points.len() && left.iter().all(|p| (p[0] - 10.0).abs() > 1.0));
    }

    #[test]
    fn a_bristle_brush_paints_thin_lines_side_by_side() {
        let pen = Pen::brush(BrushKind::Bristle, 4.0, [0, 0, 0, 255]);
        let lines = PaintStroke::paint(&line(41), &pen);
        assert!(lines.len() >= 4, "{}", lines.len());
        for l in &lines {
            assert_eq!(l.tip, Tip::Round);
            let (lo, hi) = l.bounds().unwrap();
            assert!(lo[1] > -2.5 && hi[1] < 2.5, "{lo:?} {hi:?}");
            assert!(l.points.iter().all(|p| p[2] < 4.0 / 3.0));
        }
        // Spread across the width.
        let ys: Vec<f32> = lines.iter().map(|l| l.points[0][1]).collect();
        let spread = ys.iter().cloned().fold(f32::MIN, f32::max)
            - ys.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 2.0, "{ys:?}");
    }

    #[test]
    fn a_round_stroke_is_saved_as_before() {
        let s = one(&line(3), Pen::new(Tool::Pencil, 1.0, [0, 0, 0, 255]));
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("tip"), "{json}");
        let n = one(&line(3), Pen::brush(BrushKind::Calligraphy, 1.0, [0; 4]));
        let back: PaintStroke = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(back, n);
    }
}
