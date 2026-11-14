//! Shape layers: rectangles, ellipses, lines, polylines, polygons, regular polygons,
//! stars and paths (several outlines in one, such as a letter with a hole, from an SVG
//! file), with a stroke and a fill, and turning them into pixels.
//!
//! A shape layer is an image layer whose pixels are drawn from its shape, as a text
//! layer's are from its text: moving, rotating, distorting, snapping, effects and export
//! work as for images. Its `size_px` is the box around the shape and its stroke, in
//! `UNITS_PER_MM` units per mm.
//!
//! The shape's geometry is kept in mm, in its own frame (centered on the origin for the
//! kinds drawn in a box). Scaling the layer on the canvas stretches its pixels while the
//! drag lasts; once it is over, the scale goes into the geometry and the layer's scale
//! back to one unit per `1 / UNITS_PER_MM` mm. So the stroke keeps its width in mm, the
//! corners their radius, and the sizes shown in Properties are those of the shape.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Mask, Path, PathBuilder, Pixmap, Rect, Stroke, StrokeDash,
    Transform,
};

use crate::document::{Affine, Document, Layer, P2};
use crate::gradient::Ink;
use crate::i18n::{tr_noop, translate};
use crate::render::Outlines;
use crate::text;

/// Shape units per mm: a shape layer's `size_px` and its effects are measured in these.
/// Coarse enough that the effects' pixel sizes, made for images, span useful lengths
/// (a 20 px shadow distance is 2 mm), fine enough for the box around the shape.
pub const UNITS_PER_MM: f32 = 10.0;
/// Fineness of curves and strokes turned into shapes, per mm: within a few microns.
const CURVE_RES: f32 = 50.0;
/// Longest a corner join can reach, in stroke widths, before it is cut off (bevelled).
const MITER_LIMIT: f32 = 4.0;
/// Smallest width or height of a shape drawn in a box, in mm.
const MIN_SIZE_MM: f32 = 0.1;

/// What a shape layer draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShapeKind {
    /// Also what Canvas and Drawing layers, painted in before any closed shape could
    /// be, load as.
    #[default]
    #[serde(alias = "Canvas", alias = "Drawing")]
    Rectangle,
    Ellipse,
    Line,
    Polyline,
    Polygon,
    RegularPolygon,
    Star,
    /// Points like a polygon's, in several contours, open or closed (see
    /// `Shape::contours`): what an imported SVG's outlines become. Not drawn from the
    /// Shape menu.
    Path,
}

impl ShapeKind {
    /// In the order of the Shape menu.
    pub const ALL: [ShapeKind; 7] = [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Polyline,
        ShapeKind::Polygon,
        ShapeKind::RegularPolygon,
        ShapeKind::Star,
    ];

    /// Translation key of its name, also the name of a new layer of this kind.
    pub fn name_key(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => tr_noop!("SHAPE_RECTANGLE"),
            ShapeKind::Ellipse => tr_noop!("SHAPE_ELLIPSE"),
            ShapeKind::Line => tr_noop!("SHAPE_LINE"),
            ShapeKind::Polyline => tr_noop!("SHAPE_POLYLINE"),
            ShapeKind::Polygon => tr_noop!("SHAPE_POLYGON"),
            ShapeKind::RegularPolygon => tr_noop!("SHAPE_REGULAR_POLYGON"),
            ShapeKind::Star => tr_noop!("SHAPE_STAR"),
            ShapeKind::Path => tr_noop!("SHAPE_PATH"),
        }
    }

    /// Its geometry is a list of points (`Shape::points`), rather than a box
    /// (`Shape::size`).
    pub fn has_points(self) -> bool {
        matches!(
            self,
            ShapeKind::Line | ShapeKind::Polyline | ShapeKind::Polygon | ShapeKind::Path
        )
    }

    /// It can be filled, and painted in: its outline is closed (a path's open contours
    /// are filled as if closed).
    pub fn can_fill(self) -> bool {
        self.closed()
    }

    /// Its outline is closed, so line caps only show on dashes.
    pub fn closed(self) -> bool {
        !matches!(self, ShapeKind::Line | ShapeKind::Polyline)
    }

    /// Fewest points it needs.
    pub fn min_points(self) -> usize {
        match self {
            ShapeKind::Polygon => 3,
            _ => 2,
        }
    }
}

/// The pattern of a stroke. Lengths follow the stroke's width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DashStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
}

impl DashStyle {
    pub const ALL: [DashStyle; 4] = [
        DashStyle::Solid,
        DashStyle::Dashed,
        DashStyle::Dotted,
        DashStyle::DashDot,
    ];

    pub fn name_key(self) -> &'static str {
        match self {
            DashStyle::Solid => tr_noop!("STROKE_DASH_SOLID"),
            DashStyle::Dashed => tr_noop!("STROKE_DASH_DASHED"),
            DashStyle::Dotted => tr_noop!("STROKE_DASH_DOTTED"),
            DashStyle::DashDot => tr_noop!("STROKE_DASH_DASH_DOT"),
        }
    }

    /// Visible lengths of the dashes and gaps in stroke widths, caps included; `None`
    /// for a solid line.
    fn pattern(self) -> Option<&'static [f32]> {
        match self {
            DashStyle::Solid => None,
            DashStyle::Dashed => Some(&[3.0, 2.0]),
            DashStyle::Dotted => Some(&[1.0, 1.0]),
            DashStyle::DashDot => Some(&[3.0, 1.5, 1.0, 1.5]),
        }
    }
}

/// How the ends of an open line or of dashes look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

impl Cap {
    pub const ALL: [Cap; 3] = [Cap::Butt, Cap::Round, Cap::Square];

    pub fn name_key(self) -> &'static str {
        match self {
            Cap::Butt => tr_noop!("STROKE_CAP_BUTT"),
            Cap::Round => tr_noop!("STROKE_CAP_ROUND"),
            Cap::Square => tr_noop!("STROKE_CAP_SQUARE"),
        }
    }

    fn skia(self) -> LineCap {
        match self {
            Cap::Butt => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        }
    }
}

/// How a stroke turns at corners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

impl Join {
    pub const ALL: [Join; 3] = [Join::Miter, Join::Round, Join::Bevel];

    pub fn name_key(self) -> &'static str {
        match self {
            Join::Miter => tr_noop!("STROKE_JOIN_MITER"),
            Join::Round => tr_noop!("STROKE_JOIN_ROUND"),
            Join::Bevel => tr_noop!("STROKE_JOIN_BEVEL"),
        }
    }

    fn skia(self) -> LineJoin {
        match self {
            Join::Miter => LineJoin::Miter,
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
        }
    }
}

/// One of a path's outlines: the next `len` points, and whether it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Contour {
    pub len: usize,
    pub closed: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What a shape layer draws and how. Lengths are in mm on the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shape {
    pub kind: ShapeKind,
    /// Width and height, for the kinds drawn in a box (all but lines, polylines and
    /// polygons), centered on the shape's origin.
    pub size: P2,
    /// Corners, for lines, polylines and polygons, in the shape's frame (y down).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<P2>,
    /// Bezier handles of the points, one pair per point: where the curve comes in from
    /// and goes out to, relative to the point, in mm. A zero handle is none: a segment
    /// between two points without handles on its side is straight. Empty when no point
    /// has any.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub handles: Vec<[P2; 2]>,
    /// A path's contours, splitting `points` in order; empty for the other kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contours: Vec<Contour>,
    /// A path's inside is where its contours go round an odd number of times, so a
    /// contour inside another is a hole whichever way it turns; otherwise where they go
    /// round more one way than the other (non-zero), as for every other kind.
    #[serde(default, skip_serializing_if = "is_false")]
    pub even_odd: bool,
    /// Radius of a rectangle's rounded corners.
    pub corner_radius: f32,
    /// Corners of a regular polygon, or points of a star.
    pub sides: u32,
    /// A star's inner corners' distance from its center, as a fraction of the outer
    /// ones'.
    pub inner_ratio: f32,
    /// Fill the inside.
    pub fill: bool,
    /// Straight RGBA.
    pub fill_color: [u8; 4],
    /// Fills it instead of `fill_color`, when set; `fill_color` is kept for going back to
    /// one color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_gradient: Option<crate::gradient::Gradient>,
    /// Draw a line along the outline, centered on it.
    pub stroke: bool,
    pub stroke_width: f32,
    /// Straight RGBA.
    pub stroke_color: [u8; 4],
    /// Paints the stroke instead of `stroke_color`, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_gradient: Option<crate::gradient::Gradient>,
    pub dash: DashStyle,
    pub cap: Cap,
    pub join: Join,
    /// Strokes painted inside it, bottom first, in its frame, cut off at its outline;
    /// only a closed shape shows them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<crate::paint::PaintStroke>,
    /// Resizing the shape stretches the strokes painted in it; otherwise they stay where
    /// they are on the page, as when its points are moved.
    pub stretch_paint: bool,
}

impl Default for Shape {
    fn default() -> Self {
        Self::new(ShapeKind::Rectangle)
    }
}

impl Shape {
    /// A shape of `kind` at its default size and look.
    pub fn new(kind: ShapeKind) -> Self {
        let open = !kind.can_fill() || kind == ShapeKind::Polyline;
        Self {
            kind,
            size: match kind {
                ShapeKind::Rectangle => [40.0, 25.0],
                _ => [30.0, 30.0],
            },
            points: match kind {
                ShapeKind::Line => vec![[-20.0, 0.0], [20.0, 0.0]],
                ShapeKind::Polyline => {
                    vec![[-20.0, 8.0], [-7.0, -8.0], [7.0, 8.0], [20.0, -8.0]]
                }
                ShapeKind::Polygon | ShapeKind::Path => {
                    vec![[0.0, -12.0], [15.0, 12.0], [-15.0, 12.0]]
                }
                _ => Vec::new(),
            },
            handles: Vec::new(),
            contours: if kind == ShapeKind::Path {
                vec![Contour {
                    len: 3,
                    closed: true,
                }]
            } else {
                Vec::new()
            },
            even_odd: false,
            corner_radius: 0.0,
            sides: if kind == ShapeKind::Star { 5 } else { 6 },
            inner_ratio: 0.5,
            fill: !open,
            fill_color: [70, 130, 180, 255],
            fill_gradient: None,
            stroke: true,
            stroke_width: if open { 1.0 } else { 0.5 },
            stroke_color: [0, 0, 0, 255],
            stroke_gradient: None,
            dash: DashStyle::Solid,
            cap: if open { Cap::Round } else { Cap::Butt },
            join: if open { Join::Round } else { Join::Miter },
            strokes: Vec::new(),
            stretch_paint: true,
        }
    }

    /// Corners of a regular polygon or star, fitted to its box. The first one is at the
    /// top.
    fn star_points(&self) -> Vec<P2> {
        let star = self.kind == ShapeKind::Star;
        let n = self.sides.clamp(3, 100) as usize;
        let count = if star { 2 * n } else { n };
        let raw: Vec<P2> = (0..count)
            .map(|i| {
                let a =
                    -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / count as f32;
                let r = if star && i % 2 == 1 {
                    self.inner_ratio.clamp(0.01, 1.0)
                } else {
                    1.0
                };
                [r * a.cos(), r * a.sin()]
            })
            .collect();
        let (lo, hi) = bbox(&raw).unwrap_or(([-1.0; 2], [1.0; 2]));
        let span = [(hi[0] - lo[0]).max(1e-6), (hi[1] - lo[1]).max(1e-6)];
        raw.iter()
            .map(|p| std::array::from_fn(|k| ((p[k] - lo[k]) / span[k] - 0.5) * self.size[k]))
            .collect()
    }

    /// The points of its outline and their handles, as a line, polyline or polygon
    /// would have them: the corners, or an ellipse's four sides with the handles that
    /// draw it. Used when it becomes one of those.
    fn outline(&self) -> (Vec<P2>, Vec<[P2; 2]>) {
        let [w, h] = self.size.map(|v| v / 2.0);
        match self.kind {
            ShapeKind::Path => {
                let r = self.contours().first().map_or(0..0, |c| c.0.clone());
                let handles = if self.handles.is_empty() {
                    Vec::new()
                } else {
                    r.clone().map(|i| self.handle(i)).collect()
                };
                (self.points[r].to_vec(), handles)
            }
            k if k.has_points() => (self.points.clone(), self.handles.clone()),
            ShapeKind::Rectangle => (vec![[-w, -h], [w, -h], [w, h], [-w, h]], Vec::new()),
            ShapeKind::Ellipse => {
                let (kw, kh) = (KAPPA * w, KAPPA * h);
                (
                    vec![[w, 0.0], [0.0, h], [-w, 0.0], [0.0, -h]],
                    vec![
                        [[0.0, -kh], [0.0, kh]],
                        [[kw, 0.0], [-kw, 0.0]],
                        [[0.0, kh], [0.0, -kh]],
                        [[-kw, 0.0], [kw, 0.0]],
                    ],
                )
            }
            _ => (self.star_points(), Vec::new()),
        }
    }

    /// The handles of point `i`: in, out. Zero for none.
    pub fn handle(&self, i: usize) -> [P2; 2] {
        self.handles.get(i).copied().unwrap_or([[0.0; 2]; 2])
    }

    /// The handles, one pair per point, to change them.
    fn handles_mut(&mut self) -> &mut Vec<[P2; 2]> {
        self.handles.resize(self.points.len(), [[0.0; 2]; 2]);
        &mut self.handles
    }

    /// Forgets the handles when none is left, so a straight shape saves as before.
    fn tidy_handles(&mut self) {
        if self.handles.iter().flatten().all(|h| *h == [0.0; 2]) {
            self.handles.clear();
        } else {
            self.handles.resize(self.points.len(), [[0.0; 2]; 2]);
        }
    }

    /// The runs of points drawn as one outline, and whether each closes: a path's
    /// contours, or all the points of a line, polyline or polygon. Empty for the kinds
    /// drawn in a box.
    pub fn contours(&self) -> Vec<(std::ops::Range<usize>, bool)> {
        let n = self.points.len();
        match self.kind {
            ShapeKind::Path => {
                let mut start = 0;
                self.contours
                    .iter()
                    .filter_map(|c| {
                        let end = (start + c.len).min(n);
                        let r = start..end;
                        start = end;
                        (!r.is_empty()).then_some((r, c.closed))
                    })
                    .collect()
            }
            ShapeKind::Polygon => vec![(0..n, true)],
            k if k.has_points() => vec![(0..n, false)],
            _ => Vec::new(),
        }
    }

    /// Has an open outline, whose ends show the line caps.
    pub fn has_open_ends(&self) -> bool {
        self.contours().iter().any(|(_, closed)| !closed)
    }

    /// The points before and after point `i` along its outline; `None` past the ends of
    /// an open one.
    pub fn neighbors(&self, i: usize) -> [Option<usize>; 2] {
        let Some((r, closed)) = self.contours().into_iter().find(|(r, _)| r.contains(&i)) else {
            return [None, None];
        };
        let prev = if i > r.start {
            Some(i - 1)
        } else {
            closed.then_some(r.end - 1)
        };
        let next = if i + 1 < r.end {
            Some(i + 1)
        } else {
            closed.then_some(r.start)
        };
        [prev, next].map(|p| p.filter(|&p| p != i))
    }

    /// Number of segments along a run of `len` points.
    fn run_segments(len: usize, closed: bool) -> usize {
        if closed && len >= 2 {
            len
        } else {
            len.saturating_sub(1)
        }
    }

    /// Number of segments between the points: a polygon's last closes it, and so do
    /// those of a path's closed contours.
    pub fn segment_count(&self) -> usize {
        self.contours()
            .into_iter()
            .map(|(r, closed)| Self::run_segments(r.len(), closed))
            .sum()
    }

    /// The points at the ends of segment `k`.
    pub fn segment_ends(&self, k: usize) -> Option<(usize, usize)> {
        let mut k = k;
        for (r, closed) in self.contours() {
            let count = Self::run_segments(r.len(), closed);
            if k < count {
                return Some((r.start + k, r.start + (k + 1) % r.len()));
            }
            k -= count;
        }
        None
    }

    /// The path contour holding point `i`, as an index in `contours`.
    fn contour_index(&self, i: usize) -> Option<usize> {
        let mut start = 0;
        self.contours.iter().position(|c| {
            start += c.len;
            i < start
        })
    }

    /// Segment `k` as a cubic Bezier curve: its two ends and two control points.
    pub fn segment(&self, k: usize) -> Option<[P2; 4]> {
        let (a, b) = self.segment_ends(k)?;
        Some(self.curve(a, b))
    }

    /// The curve from point `a` to point `b`, by their handles.
    fn curve(&self, a: usize, b: usize) -> [P2; 4] {
        let (pa, pb) = (self.points[a], self.points[b]);
        let (out, inn) = (self.handle(a)[1], self.handle(b)[0]);
        [
            pa,
            [pa[0] + out[0], pa[1] + out[1]],
            [pb[0] + inn[0], pb[1] + inn[1]],
            pb,
        ]
    }

    /// How its outline's inside is found when it is filled (see `even_odd`).
    pub fn fill_rule(&self) -> FillRule {
        if self.kind == ShapeKind::Path && self.even_odd {
            FillRule::EvenOdd
        } else {
            FillRule::Winding
        }
    }

    /// True if point `i` has both handles, lined up on either side of it: dragging one
    /// turns the other.
    pub fn is_smooth(&self, i: usize) -> bool {
        let [a, b] = self.handle(i);
        let (la, lb) = (len(a), len(b));
        la > 1e-6 && lb > 1e-6 && (a[0] * b[0] + a[1] * b[1]) / (la * lb) < -0.999
    }

    /// Its outline in mm, in its own frame; `None` if it has too few points.
    pub fn path(&self) -> Option<Path> {
        let mut pb = PathBuilder::new();
        let [w, h] = self.size.map(|v| v.max(MIN_SIZE_MM));
        match self.kind {
            ShapeKind::Rectangle => {
                let r = self.corner_radius.clamp(0.0, w.min(h) / 2.0);
                rounded_rect(&mut pb, [-w / 2.0, -h / 2.0, w / 2.0, h / 2.0], r);
            }
            ShapeKind::Ellipse => {
                pb.push_oval(Rect::from_xywh(-w / 2.0, -h / 2.0, w, h)?);
            }
            ShapeKind::RegularPolygon | ShapeKind::Star => {
                push_points(&mut pb, &self.star_points(), true);
            }
            k => {
                if self.points.len() < k.min_points() {
                    return None;
                }
                for (r, closed) in self.contours() {
                    let first = self.points[r.start];
                    pb.move_to(first[0], first[1]);
                    for j in 0..Self::run_segments(r.len(), closed) {
                        let [a, c1, c2, b] = self.curve(r.start + j, r.start + (j + 1) % r.len());
                        if c1 == a && c2 == b {
                            pb.line_to(b[0], b[1]);
                        } else {
                            pb.cubic_to(c1[0], c1[1], c2[0], c2[1], b[0], b[1]);
                        }
                    }
                    if closed {
                        pb.close();
                    }
                }
            }
        }
        pb.finish()
    }

    /// The stroke's style in mm, without its dashes (`dashed` does those).
    fn stroke_style(&self) -> Stroke {
        Stroke {
            width: self.stroke_width,
            miter_limit: MITER_LIMIT,
            line_cap: self.cap.skia(),
            line_join: self.join.skia(),
            dash: None,
        }
    }

    /// The outline cut into dashes, if the stroke has some.
    fn dashed(&self, path: &Path) -> Option<Path> {
        let w = self.stroke_width;
        let pattern = self.dash.pattern()?;
        // Caps reach half the width past each end of a dash: take them off the dashes
        // and add them to the gaps, so the pattern looks the same with any cap.
        let cap = match self.cap {
            Cap::Butt => 0.0,
            Cap::Round | Cap::Square => w,
        };
        let array = pattern
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                if i % 2 == 0 {
                    (v * w - cap).max(w * 0.001)
                } else {
                    v * w + cap
                }
            })
            .collect();
        path.dash(&StrokeDash::new(array, 0.0)?, CURVE_RES)
    }

    /// Has a stroke that draws something.
    fn stroked(&self) -> bool {
        self.stroke && self.stroke_width > 0.0
    }

    fn filled(&self) -> bool {
        self.fill && self.kind.can_fill()
    }

    /// The filled shapes that draw it, in mm in its own frame, bottom first: the fill,
    /// the strokes painted inside it, then its own stroke turned into a shape, so that
    /// paint never covers its border. Each is filled with its color or gradient under
    /// its rule (non-zero but for a path's fill, see `fill_rule`), and is true when it is
    /// cut off at the outline (the painted strokes). Gradients span the shape's box,
    /// stroke included.
    pub fn coats(&self) -> Vec<(Path, Ink, bool, FillRule)> {
        let Some(path) = self.path() else {
            return Vec::new();
        };
        let frame = self
            .bounds()
            .and_then(|(lo, hi)| Rect::from_ltrb(lo[0], lo[1], hi[0], hi[1]));
        let mut out = Vec::new();
        if self.stroked() {
            let line = match self.dash {
                DashStyle::Solid => Some(path.clone()),
                _ => self.dashed(&path),
            };
            if let Some(stroke) = line.and_then(|l| l.stroke(&self.stroke_style(), CURVE_RES)) {
                let ink = Ink::of(self.stroke_color, self.stroke_gradient.as_ref(), frame);
                out.push((stroke, ink, false, FillRule::Winding));
            }
        }
        if self.kind.closed() {
            let paint = self
                .strokes
                .iter()
                .filter_map(|s| Some((s.outline()?, Ink::Solid(s.color), true, FillRule::Winding)));
            out.splice(0..0, paint);
        }
        if self.filled() {
            let ink = Ink::of(self.fill_color, self.fill_gradient.as_ref(), frame);
            out.insert(0, (path, ink, false, self.fill_rule()));
        }
        out
    }

    /// Area it covers, stroke included, in mm in its own frame: min and max corners.
    /// Dashes are left out, so the box doesn't change with them.
    pub fn bounds(&self) -> Option<(P2, P2)> {
        let path = self.path()?;
        let mut r = path.compute_tight_bounds()?;
        if self.stroked()
            && let Some(s) = path.stroke(&self.stroke_style(), CURVE_RES)
            && let Some(sb) = s.compute_tight_bounds()
        {
            r = Rect::from_ltrb(
                r.left().min(sb.left()),
                r.top().min(sb.top()),
                r.right().max(sb.right()),
                r.bottom().max(sb.bottom()),
            )?;
        }
        Some(([r.left(), r.top()], [r.right(), r.bottom()]))
    }

    /// Box of a line, polyline or polygon's geometry, curves included: min and max
    /// corners.
    fn points_bounds(&self) -> Option<(P2, P2)> {
        if !self.handles.is_empty()
            && let Some(r) = self.path().and_then(|p| p.compute_tight_bounds())
        {
            return Some(([r.left(), r.top()], [r.right(), r.bottom()]));
        }
        bbox(&self.points)
    }

    /// Width and height of the geometry itself, stroke left out.
    pub fn geometry_size(&self) -> P2 {
        if self.kind.has_points() {
            self.points_bounds()
                .map_or([0.0; 2], |(lo, hi)| [hi[0] - lo[0], hi[1] - lo[1]])
        } else {
            self.size
        }
    }

    /// Center of the geometry's box, in its own frame.
    fn geometry_center(&self) -> P2 {
        if self.kind.has_points() {
            self.points_bounds().map_or([0.0; 2], |(lo, hi)| {
                [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
            })
        } else {
            [0.0; 2]
        }
    }

    /// Scales the geometry by `k` along its own axes, around `about`. Lengths that
    /// aren't along an axis (stroke width, corner radius) stay as they are.
    /// The strokes painted inside stretch with it if `stretch_paint`.
    fn scale(&mut self, k: P2, about: P2, stretch_paint: bool) {
        let mut k = k;
        if self.kind.has_points() {
            for p in &mut self.points {
                *p = std::array::from_fn(|i| about[i] + (p[i] - about[i]) * k[i]);
            }
            for h in self.handles.iter_mut().flatten() {
                *h = [h[0] * k[0], h[1] * k[1]];
            }
        } else {
            let before = self.size;
            self.size = std::array::from_fn(|i| (before[i] * k[i]).max(MIN_SIZE_MM));
            // The box's true ratio, past its smallest size.
            k = std::array::from_fn(|i| self.size[i] / before[i].max(1e-6));
        }
        if stretch_paint {
            for s in &mut self.strokes {
                s.scale(k, about);
            }
        }
    }
}

/// Control points of a quarter circle drawn as a cubic curve, as a fraction of its radius.
const KAPPA: f32 = 0.552_284_8;

fn len(v: P2) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// A point on a cubic Bezier curve, at `t` from 0 to 1.
pub fn cubic_at(c: [P2; 4], t: f32) -> P2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    std::array::from_fn(|k| (0..4).map(|i| w[i] * c[i][k]).sum())
}

/// Smallest box holding all `points`: min and max corners.
fn bbox(points: &[P2]) -> Option<(P2, P2)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(lo, hi), p| {
        (
            [lo[0].min(p[0]), lo[1].min(p[1])],
            [hi[0].max(p[0]), hi[1].max(p[1])],
        )
    }))
}

fn push_points(pb: &mut PathBuilder, points: &[P2], closed: bool) {
    let Some((first, rest)) = points.split_first() else {
        return;
    };
    pb.move_to(first[0], first[1]);
    for p in rest {
        pb.line_to(p[0], p[1]);
    }
    if closed {
        pb.close();
    }
}

/// A rectangle (left, top, right, bottom) with corners rounded to radius `r`.
fn rounded_rect(pb: &mut PathBuilder, [x0, y0, x1, y1]: [f32; 4], r: f32) {
    if r <= 0.0 {
        if let Some(rect) = Rect::from_ltrb(x0, y0, x1, y1) {
            pb.push_rect(rect);
        }
        return;
    }
    // Control points of a quarter circle drawn as a cubic curve.
    let k = r * (1.0 - KAPPA);
    pb.move_to(x0 + r, y0);
    pb.line_to(x1 - r, y0);
    pb.cubic_to(x1 - k, y0, x1, y0 + k, x1, y0 + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - k, x1 - k, y1, x1 - r, y1);
    pb.line_to(x0 + r, y1);
    pb.cubic_to(x0 + k, y1, x0, y1 - k, x0, y1 - r);
    pb.line_to(x0, y0 + r);
    pb.cubic_to(x0, y0 + k, x0 + k, y0, x0 + r, y0);
    pb.close();
}

/// Where a shape layer's drawing stands. Not saved.
#[derive(Clone, Debug, Default)]
pub struct ShapeState {
    /// The point of the shape's frame at the top-left corner of the layer's box, in mm,
    /// from the last layout.
    origin: Option<P2>,
    /// A point of the shape's frame (after an edit) and where on the page it must stay
    /// when the box is worked out again. By default, the frame's origin stays in place.
    pin: Option<(P2, P2)>,
    /// Identifies what the layer's box was last worked out from.
    layout_key: u64,
    /// Identifies what the current pixmap was drawn from.
    key: u64,
    /// The map from the shape's frame to the page when it was last unscaled, for the
    /// strokes painted in it to stay in place while it is resized (see `paint_map`).
    settled: Option<Affine>,
    /// The resize going on does the other way than `Shape::stretch_paint` says (Ctrl+Alt
    /// on a handle); cleared once it settles.
    flip_paint: bool,
}

/// A new shape layer of `kind` at its default size, centered on `center`.
pub fn new_layer(id: u64, kind: ShapeKind, center: P2) -> Layer {
    let mut layer = Layer::new(id, translate(kind.name_key()).to_owned(), PathBuf::new());
    layer.shape = Some(Box::new(Shape::new(kind)));
    layer.transform.scale = [1.0 / UNITS_PER_MM; 2];
    layer.transform.center = center;
    layout(&mut layer, false);
    layer
}

/// A shape layer named `name` drawing `shape`, the origin of its frame at `at` on the
/// page.
pub fn new_shape_layer(id: u64, name: String, shape: Shape, at: P2) -> Layer {
    let mut layer = Layer::new(id, name, PathBuf::new());
    // The layer's box is centered on the shape's.
    let c = shape.bounds().map_or([0.0; 2], |(lo, hi)| {
        [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
    });
    layer.shape = Some(Box::new(shape));
    layer.transform.scale = [1.0 / UNITS_PER_MM; 2];
    layer.transform.center = [at[0] + c[0], at[1] + c[1]];
    layout(&mut layer, false);
    layer
}

/// A line, polyline or polygon to draw point by point, its first point at `at` on the
/// page. It draws nothing until it has a second point (see `push_point`).
pub fn new_drawn_layer(id: u64, kind: ShapeKind, at: P2) -> Layer {
    let mut layer = Layer::new(id, translate(kind.name_key()).to_owned(), PathBuf::new());
    let mut shape = Shape::new(kind);
    shape.points = vec![[0.0, 0.0]];
    layer.shape = Some(Box::new(shape));
    layer.transform.scale = [1.0 / UNITS_PER_MM; 2];
    layer.transform.center = at;
    layout(&mut layer, false);
    layer
}

/// A new shape of a kind drawn in a box (not a polygon) to paint in, without fill or
/// stroke, filling the page's box from `lo` to `hi` (mm); strokes are painted in with
/// [`add_stroke`].
pub fn new_canvas_layer(id: u64, kind: ShapeKind, lo: P2, hi: P2) -> Layer {
    let mut layer = Layer::new(id, String::new(), PathBuf::new());
    let mut shape = Shape::new(kind);
    shape.size = [hi[0] - lo[0], hi[1] - lo[1]].map(|v| v.abs().max(MIN_SIZE_MM));
    layer.shape = Some(Box::new(shape));
    make_canvas(&mut layer);
    layer.transform.scale = [1.0 / UNITS_PER_MM; 2];
    layer.transform.center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    layout(&mut layer, false);
    layer
}

/// Makes a new shape layer a canvas to paint in: no fill or stroke (its outline shows
/// dashed on screen), named so.
pub fn make_canvas(layer: &mut Layer) {
    layer.name = translate(tr_noop!("DEFAULT_CANVAS_LAYER_NAME")).to_owned();
    if let Some(s) = layer.shape.as_deref_mut() {
        s.fill = false;
        s.stroke = false;
    }
}

/// How many mm of the layer's frame make one mm on the page: 1 unless it was scaled and
/// the scale not yet put into its geometry.
fn frame_mm_per_page_mm(layer: &Layer) -> f32 {
    let s = layer.transform.scale;
    let k = (s[0].abs() * s[1].abs()).sqrt() * UNITS_PER_MM;
    if k > 1e-6 { 1.0 / k } else { 1.0 }
}

/// Adds what `pen` paints inside a closed shape: `samples` are x, y on the page (mm) and
/// the pen's pressure, its width is in mm and its nib's angle on the page. What is
/// outside the shape doesn't show; a stroke all outside its box isn't added. Returns
/// true if anything was added.
pub fn add_stroke(layer: &mut Layer, samples: &[[f32; 3]], pen: &crate::paint::Pen) -> bool {
    if layer.shape.as_deref().is_none_or(|s| !s.kind.closed()) {
        return false;
    }
    let k = frame_mm_per_page_mm(layer);
    let local: Vec<[f32; 3]> = samples
        .iter()
        .filter_map(|&[x, y, p]| from_page(layer, [x, y]).map(|q| [q[0], q[1], p]))
        .collect();
    let mut pen = crate::paint::Pen {
        width: pen.width * k,
        ..*pen
    };
    // The nib keeps its angle on the page, however the shape is turned.
    if let Some(&[x, y, _]) = samples.first() {
        let (sin, cos) = pen.angle.sin_cos();
        if let (Some(a), Some(b)) = (
            from_page(layer, [x, y]),
            from_page(layer, [x + cos, y + sin]),
        ) {
            pen.angle = (b[1] - a[1]).atan2(b[0] - a[0]);
        }
    }
    let strokes = crate::paint::PaintStroke::paint(&local, &pen);
    let Some(shape) = layer.shape.as_deref_mut() else {
        return false;
    };
    let area = shape.path().and_then(|p| p.compute_tight_bounds());
    let mut added = false;
    for stroke in strokes {
        let inside = stroke.bounds().zip(area).is_some_and(|((lo, hi), a)| {
            lo[0] < a.right() && hi[0] > a.left() && lo[1] < a.bottom() && hi[1] > a.top()
        });
        if inside {
            shape.strokes.push(stroke);
            added = true;
        }
    }
    added
}

/// Cuts the strokes painted inside a shape where an eraser of `radius` mm passed along `path` on
/// the page. Returns true if anything was cut.
pub fn erase(layer: &mut Layer, path: &[P2], radius: f32) -> bool {
    if layer.shape.as_deref().is_none_or(|s| !s.kind.closed()) {
        return false;
    }
    let k = frame_mm_per_page_mm(layer);
    let local: Vec<P2> = path.iter().filter_map(|&p| from_page(layer, p)).collect();
    layer
        .shape
        .as_deref_mut()
        .is_some_and(|s| crate::paint::erase(&mut s.strokes, &local, radius * k))
}

/// Takes out all the strokes painted inside a shape. Returns true if there were any.
pub fn clear_paint(layer: &mut Layer) -> bool {
    layer
        .shape
        .as_deref_mut()
        .is_some_and(|s| !std::mem::take(&mut s.strokes).is_empty())
}

/// A closed shape: it can be painted in.
pub fn paintable(layer: &Layer) -> bool {
    layer.shape.as_deref().is_some_and(|s| s.kind.closed())
}

/// Adds a point at the end of a line, polyline or polygon being drawn. Returns true if
/// it was added; a line takes two.
pub fn push_point(layer: &mut Layer, page: P2) -> bool {
    let Some(p) = from_page(layer, page) else {
        return false;
    };
    let Some(shape) = layer.shape.as_deref_mut().filter(|s| s.kind.has_points()) else {
        return false;
    };
    if shape.kind == ShapeKind::Line && shape.points.len() >= 2 {
        return false;
    }
    shape.points.push(p);
    shape.tidy_handles();
    true
}

/// Where a point of the shape's frame (mm) is on the page, from the last layout.
fn page_of(layer: &Layer, p: P2) -> Option<P2> {
    let o = layer.shape_state.origin?;
    Some(
        layer
            .affine()
            .apply([(p[0] - o[0]) * UNITS_PER_MM, (p[1] - o[1]) * UNITS_PER_MM]),
    )
}

/// The map from the shape's frame (mm) to the page, distortion left out.
fn frame_map(layer: &Layer) -> Option<Affine> {
    let o = layer.shape_state.origin?;
    Some(
        Affine::translate(-o[0], -o[1])
            .then(Affine::scale(UNITS_PER_MM, UNITS_PER_MM))
            .then(layer.affine()),
    )
}

/// For the resize about to start: the paint does the other way than the shape's Stretch
/// paint says, until the resize settles.
pub fn flip_paint_once(layer: &mut Layer) {
    layer.shape_state.flip_paint = true;
}

/// Resizing the shape stretches its paint now: its setting, unless flipped for the
/// resize going on.
fn stretches_paint(layer: &Layer) -> bool {
    layer
        .shape
        .as_deref()
        .is_some_and(|s| s.stretch_paint != layer.shape_state.flip_paint)
}

/// The layer's scale isn't yet in its geometry: it is being resized.
fn is_scaled(layer: &Layer) -> bool {
    layer
        .transform
        .scale
        .iter()
        .any(|v| (v.abs() * UNITS_PER_MM - 1.0).abs() > 1e-4)
}

/// While a shape whose paint doesn't stretch is being resized: where its strokes are in
/// its frame now, from where they are in its frame unscaled, so that they stay in place
/// on the page.
fn paint_map(layer: &Layer) -> Option<Affine> {
    let shape = layer.shape.as_deref()?;
    if stretches_paint(layer) || shape.strokes.is_empty() || !is_scaled(layer) {
        return None;
    }
    Some(
        layer
            .shape_state
            .settled?
            .then(frame_map(layer)?.inverse()?),
    )
}

/// Where a point of the shape's frame (mm) is drawn on the page, distortion included.
pub fn to_page(layer: &Layer, p: P2) -> Option<P2> {
    let o = layer.shape_state.origin?;
    Some(
        layer
            .homography()
            .apply([(p[0] - o[0]) * UNITS_PER_MM, (p[1] - o[1]) * UNITS_PER_MM]),
    )
}

/// The point of the shape's frame (mm) drawn at a page position.
pub fn from_page(layer: &Layer, page: P2) -> Option<P2> {
    let o = layer.shape_state.origin?;
    let u = layer.homography().inverse()?.apply(page);
    Some([u[0] / UNITS_PER_MM + o[0], u[1] / UNITS_PER_MM + o[1]])
}

/// The points of a line, polyline or polygon on the page; empty for other kinds.
pub fn page_points(layer: &Layer) -> Vec<P2> {
    match layer.shape.as_deref() {
        Some(s) if s.kind.has_points() => {
            s.points.iter().filter_map(|&p| to_page(layer, p)).collect()
        }
        _ => Vec::new(),
    }
}

/// The shape's outline on the page, as a line through points along it, and whether it
/// is closed: the path a text can follow. Curves are cut in short straight pieces; only
/// the first contour counts.
pub fn page_outline(layer: &Layer) -> Option<(Vec<P2>, bool)> {
    use tiny_skia::PathSegment;
    let shape = layer.shape.as_deref()?;
    let path = shape.path()?;
    let mut frame: Vec<P2> = Vec::new();
    let mut last = [0.0f32; 2];
    let sample = |out: &mut Vec<P2>, c: [P2; 4]| {
        for j in 1..=32 {
            out.push(cubic_at(c, j as f32 / 32.0));
        }
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                if !frame.is_empty() {
                    break;
                }
                last = [p.x, p.y];
                frame.push(last);
            }
            PathSegment::LineTo(p) => {
                last = [p.x, p.y];
                frame.push(last);
            }
            PathSegment::QuadTo(c, p) => {
                // As a cubic: control points two thirds of the way to the quad's.
                let c = [c.x, c.y];
                let e = [p.x, p.y];
                let k = |a: P2| {
                    [
                        a[0] + (c[0] - a[0]) * 2.0 / 3.0,
                        a[1] + (c[1] - a[1]) * 2.0 / 3.0,
                    ]
                };
                sample(&mut frame, [last, k(last), k(e), e]);
                last = e;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let e = [p.x, p.y];
                sample(&mut frame, [last, [c1.x, c1.y], [c2.x, c2.y], e]);
                last = e;
            }
            PathSegment::Close => break,
        }
    }
    let page: Vec<P2> = frame
        .into_iter()
        .filter_map(|p| to_page(layer, p))
        .collect();
    (page.len() >= 2).then_some((page, shape.kind.closed()))
}

/// Points of a shape layer that snap to anchors when it is moved: the center of its
/// geometry, then its points, or the corners of its box and the middles of its sides.
/// The stroke is left out, so a line's end, not its stroke's edge, lands on the anchor.
pub fn snap_points(layer: &Layer) -> Vec<P2> {
    let Some(s) = layer.shape.as_deref() else {
        return Vec::new();
    };
    let mut frame = vec![s.geometry_center()];
    if s.kind.has_points() {
        frame.extend(&s.points);
    } else {
        let [w, h] = s.size.map(|v| v / 2.0);
        frame.extend([[-w, -h], [w, -h], [w, h], [-w, h]]);
        frame.extend([[0.0, -h], [w, 0.0], [0.0, h], [-w, 0.0]]);
    }
    frame
        .into_iter()
        .filter_map(|p| to_page(layer, p))
        .collect()
}

/// Moves point `i` of a line, polyline or polygon to a page position. Returns true if it
/// moved.
pub fn move_point(layer: &mut Layer, i: usize, page: P2) -> bool {
    let Some(p) = from_page(layer, page) else {
        return false;
    };
    let Some(shape) = layer.shape.as_deref_mut().filter(|s| s.kind.has_points()) else {
        return false;
    };
    match shape.points.get_mut(i) {
        // Round trips through the page leave tiny differences: those aren't moves.
        Some(q) if (q[0] - p[0]).abs() + (q[1] - p[1]).abs() > 1e-4 => {
            *q = p;
            true
        }
        _ => false,
    }
}

/// Adds a point on segment `k` at `t` (0 to 1 along it), after its first end. The curve
/// is split there, so the shape stays the same. Returns true if it was added; a line
/// keeps its two points.
pub fn split_segment(layer: &mut Layer, k: usize, t: f32) -> bool {
    let Some(shape) = layer.shape.as_deref_mut().filter(|s| {
        matches!(
            s.kind,
            ShapeKind::Polyline | ShapeKind::Polygon | ShapeKind::Path
        )
    }) else {
        return false;
    };
    let (Some([p0, p1, p2, p3]), Some((a, b))) = (shape.segment(k), shape.segment_ends(k)) else {
        return false;
    };
    let t = t.clamp(0.0, 1.0);
    let mix = |a: P2, b: P2| -> P2 { [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t] };
    let (q0, q1, q2) = (mix(p0, p1), mix(p1, p2), mix(p2, p3));
    let (r0, r1) = (mix(q0, q1), mix(q1, q2));
    let m = mix(r0, r1);
    let n = shape.points.len();
    let straight = p1 == p0 && p2 == p3;
    if let Some(c) = shape.contour_index(a) {
        shape.contours[c].len += 1;
    }
    shape.points.insert(a + 1, m);
    if straight {
        if !shape.handles.is_empty() {
            shape.handles.insert(a + 1, [[0.0; 2]; 2]);
        }
        return true;
    }
    let off = |p: P2, from: P2| [p[0] - from[0], p[1] - from[1]];
    let hs = &mut shape.handles;
    hs.resize(n, [[0.0; 2]; 2]);
    hs[a][1] = off(q0, p0);
    hs[b][0] = off(q2, p3);
    hs.insert(a + 1, [off(r0, m), off(r1, m)]);
    true
}

/// Removes point `i`, unless the shape would have too few. Returns true if it was
/// removed.
pub fn remove_point(layer: &mut Layer, i: usize) -> bool {
    let Some(shape) = layer.shape.as_deref_mut().filter(|s| s.kind.has_points()) else {
        return false;
    };
    if i >= shape.points.len() || shape.points.len() <= shape.kind.min_points() {
        return false;
    }
    // Each of a path's contours keeps two points.
    if shape.kind == ShapeKind::Path {
        match shape.contour_index(i) {
            Some(c) if shape.contours[c].len > 2 => shape.contours[c].len -= 1,
            _ => return false,
        }
    }
    shape.points.remove(i);
    if i < shape.handles.len() {
        shape.handles.remove(i);
    }
    shape.tidy_handles();
    true
}

/// Where the handles of point `i` end on the page: in, out; `None` for a missing one.
pub fn page_handles(layer: &Layer, i: usize) -> [Option<P2>; 2] {
    let Some(s) = layer.shape.as_deref().filter(|s| i < s.points.len()) else {
        return [None, None];
    };
    let p = s.points[i];
    s.handle(i).map(|h| {
        (h != [0.0; 2])
            .then(|| to_page(layer, [p[0] + h[0], p[1] + h[1]]))
            .flatten()
    })
}

/// What happens to a point's other handle when one is moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    /// It stays as it is.
    None,
    /// It turns to stay lined up, keeping its length (or taking the same if it had
    /// none): a smooth point.
    Turn,
    /// It becomes the moved one's opposite, same length: pulling out a new point's
    /// handles.
    Full,
}

/// Moves handle `which` (0 in, 1 out) of point `i` to a page position, and the other
/// as `mirror` says. Returns true if it changed.
pub fn set_handle(layer: &mut Layer, i: usize, which: usize, page: P2, mirror: Mirror) -> bool {
    let Some(q) = from_page(layer, page) else {
        return false;
    };
    let Some(shape) = layer
        .shape
        .as_deref_mut()
        .filter(|s| s.kind.has_points() && i < s.points.len())
    else {
        return false;
    };
    let p = shape.points[i];
    let h = [q[0] - p[0], q[1] - p[1]];
    let old = shape.handle(i);
    let mut new = old;
    new[which] = h;
    if mirror != Mirror::None {
        let l = len(h);
        let other = match len(old[1 - which]) {
            o if o > 1e-6 && mirror == Mirror::Turn => o,
            _ => l,
        };
        new[1 - which] = if l > 1e-6 {
            [-h[0] / l * other, -h[1] / l * other]
        } else {
            [0.0; 2]
        };
    }
    let moved = (0..2).any(|k| len([new[k][0] - old[k][0], new[k][1] - old[k][1]]) > 1e-4);
    if moved {
        shape.handles_mut()[i] = new;
        shape.tidy_handles();
    }
    moved
}

/// Makes point `i` sharp if it has handles, or else smooth: handles along the line
/// between its neighbors, a third of the way to each. Returns true if it changed.
pub fn toggle_smooth(layer: &mut Layer, i: usize) -> bool {
    let Some(shape) = layer
        .shape
        .as_deref_mut()
        .filter(|s| s.kind.has_points() && i < s.points.len())
    else {
        return false;
    };
    if shape.handle(i) != [[0.0; 2]; 2] {
        shape.handles_mut()[i] = [[0.0; 2]; 2];
        shape.tidy_handles();
        return true;
    }
    let p = shape.points[i];
    let [prev, next] = shape.neighbors(i).map(|k| k.map(|k| shape.points[k]));
    let (Some(a), Some(b)) = (prev.or(next), next.or(prev)) else {
        return false;
    };
    // An end of an open line has one neighbor: its handle points at it.
    let d = if prev.is_some() && next.is_some() {
        [b[0] - a[0], b[1] - a[1]]
    } else if next.is_some() {
        [b[0] - p[0], b[1] - p[1]]
    } else {
        [p[0] - a[0], p[1] - a[1]]
    };
    let l = len(d);
    if l < 1e-6 {
        return false;
    }
    let u = [d[0] / l, d[1] / l];
    let (li, lo) = (
        len([p[0] - a[0], p[1] - a[1]]) / 3.0,
        len([b[0] - p[0], b[1] - p[1]]) / 3.0,
    );
    let h = [
        if prev.is_some() {
            [-u[0] * li, -u[1] * li]
        } else {
            [0.0; 2]
        },
        if next.is_some() {
            [u[0] * lo, u[1] * lo]
        } else {
            [0.0; 2]
        },
    ];
    shape.handles_mut()[i] = h;
    shape.tidy_handles();
    true
}

/// Each segment's curve on the page, as `steps + 1` points from one end to the other.
pub fn page_curves(layer: &Layer, steps: usize) -> Vec<Vec<P2>> {
    let Some(s) = layer.shape.as_deref().filter(|s| s.kind.has_points()) else {
        return Vec::new();
    };
    (0..s.segment_count())
        .filter_map(|k| s.segment(k))
        .map(|c| {
            let straight = c[1] == c[0] && c[2] == c[3];
            let steps = if straight { 1 } else { steps };
            (0..=steps)
                .filter_map(|j| to_page(layer, cubic_at(c, j as f32 / steps as f32)))
                .collect()
        })
        .collect()
}

/// Keeps the center of the geometry's box where it is on the page through an edit that
/// moves it in the shape's frame by `shift`.
fn pin_center(layer: &mut Layer, shift: P2) {
    let Some(shape) = &layer.shape else { return };
    let c = shape.geometry_center();
    if let Some(page) = page_of(layer, c) {
        layer.shape_state.pin = Some(([c[0] + shift[0], c[1] + shift[1]], page));
    }
}

/// Resizes the geometry to `size` mm (stroke left out), keeping its center in place.
/// A zero width or height (a straight line) can't be changed.
pub fn set_geometry_size(layer: &mut Layer, size: P2) {
    pin_center(layer, [0.0; 2]);
    let Some(shape) = layer.shape.as_deref_mut() else {
        return;
    };
    let old = shape.geometry_size();
    let k = std::array::from_fn(|i| {
        if old[i] > 1e-6 {
            size[i].max(MIN_SIZE_MM) / old[i]
        } else {
            1.0
        }
    });
    let c = shape.geometry_center();
    let stretch = shape.stretch_paint;
    shape.scale(k, c, stretch);
}

/// Turns the shape into another kind, keeping its place and size: its outline's
/// corners become points, points become a box. A path stays one: its other contours
/// would be lost.
pub fn set_kind(layer: &mut Layer, kind: ShapeKind) {
    let Some(shape) = layer.shape.as_deref() else {
        return;
    };
    if shape.kind == kind || shape.kind == ShapeKind::Path {
        return;
    }
    let mut new = shape.clone();
    new.kind = kind;
    let mut shift = [0.0; 2];
    if kind.has_points() {
        let (mut points, mut handles) = shape.outline();
        if kind == ShapeKind::Line {
            // From one end to the other.
            (points, handles) = match shape.kind {
                ShapeKind::Line | ShapeKind::Polyline => {
                    let last = points.len() - 1;
                    (
                        vec![points[0], points[last]],
                        vec![shape.handle(0), shape.handle(last)],
                    )
                }
                _ => {
                    let [w, _] = shape.size;
                    (vec![[-w / 2.0, 0.0], [w / 2.0, 0.0]], Vec::new())
                }
            };
        } else if points.len() < kind.min_points() {
            // A line becomes a triangle on it.
            let (a, b) = (points[0], points[1]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            points = vec![a, b, [m[0] + d[1] / 2.0, m[1] - d[0] / 2.0]];
        }
        new.contours = if kind == ShapeKind::Path {
            vec![Contour {
                len: points.len(),
                // A box kind's outline is closed.
                closed: !shape.kind.has_points() || shape.contours().first().is_some_and(|c| c.1),
            }]
        } else {
            Vec::new()
        };
        new.points = points;
        new.handles = handles;
        new.tidy_handles();
        if !kind.can_fill() {
            new.fill = false;
        }
    } else {
        // Drawn in a box centered on the frame's origin.
        let c = shape.geometry_center();
        shift = [-c[0], -c[1]];
        let s = shape.geometry_size();
        let big = s[0].max(s[1]).max(1.0);
        // A straight line gets a square box.
        new.size = s.map(|v| if v < big * 0.01 { big } else { v });
        new.points.clear();
        new.handles.clear();
        new.contours.clear();
        // The painted strokes move with the geometry, now centered on the frame's origin.
        for p in new.strokes.iter_mut().flat_map(|s| &mut s.points) {
            p[0] += shift[0];
            p[1] += shift[1];
        }
    }
    if !shape.kind.can_fill() && kind.can_fill() && kind.closed() {
        new.fill = true;
    }
    pin_center(layer, shift);
    layer.shape = Some(Box::new(new));
}

fn hash_of(v: &impl Serialize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(v).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Brings the layer's box up to date with its shape, keeping the shape in place on the
/// page. With `settle`, a scale the layer was given (by its handles) goes into its
/// geometry first.
pub(crate) fn layout(layer: &mut Layer, settle: bool) {
    lay_out_box(layer, settle);
    // Where the frame is on the page once settled, for paint that doesn't stretch.
    if !is_scaled(layer) {
        layer.shape_state.settled = frame_map(layer);
        if settle {
            layer.shape_state.flip_paint = false;
        }
    }
}

fn lay_out_box(layer: &mut Layer, settle: bool) {
    let fixed_paint = paint_map(layer);
    let stretch = stretches_paint(layer);
    let pin = layer
        .shape_state
        .pin
        .take()
        .or_else(|| Some(([0.0; 2], page_of(layer, [0.0; 2])?)));
    let s = layer.transform.scale;
    let k = s.map(|v| v.abs() * UNITS_PER_MM);
    let bake = settle && k.iter().any(|v| (v - 1.0).abs() > 1e-4);
    let Some(shape) = layer.shape.as_deref_mut() else {
        return;
    };
    let mut pin = pin;
    if bake {
        // The map from the frame to the page is linear: scaled around the origin, every
        // point of the geometry stays where the stretched layer showed it.
        shape.scale(k, [0.0; 2], stretch);
        // Paint that doesn't stretch stays where the resized layer showed it.
        if let Some(m) = fixed_paint {
            for s in &mut shape.strokes {
                for p in &mut s.points {
                    let q = m.apply([p[0], p[1]]);
                    (p[0], p[1]) = (q[0] * k[0], q[1] * k[1]);
                }
                s.turn_nib(|v| {
                    let (q, o) = (m.apply(v), m.apply([0.0; 2]));
                    [(q[0] - o[0]) * k[0], (q[1] - o[1]) * k[1]]
                });
            }
        }
        if let Some((p, _)) = &mut pin {
            *p = [p[0] * k[0], p[1] * k[1]];
        }
        layer.transform.scale = s.map(|v| v.signum() / UNITS_PER_MM);
    }
    let key = hash_of(&*shape);
    if key == layer.shape_state.layout_key && layer.shape_state.origin.is_some() && !bake {
        return;
    }
    let (lo, hi) = shape.bounds().unwrap_or(([0.0; 2], [0.0; 2]));
    let size = [hi[0] - lo[0], hi[1] - lo[1]].map(|v| (v * UNITS_PER_MM).ceil().max(1.0));
    // The box is centered on the shape's own box.
    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    layer.size_px = size.map(|v| v as u32);
    layer.shape_state.origin = Some([
        c[0] - size[0] / UNITS_PER_MM / 2.0,
        c[1] - size[1] / UNITS_PER_MM / 2.0,
    ]);
    layer.shape_state.layout_key = key;
    if let Some((p, page)) = pin
        && let Some(now) = page_of(layer, p)
    {
        let t = &mut layer.transform.center;
        *t = [t[0] + page[0] - now[0], t[1] + page[1] - now[1]];
    }
}

/// Draws the layer's shape at `px_per_unit` pixels per layer unit.
fn draw(layer: &Layer, px_per_unit: f32) -> Option<Pixmap> {
    let shape = layer.shape.as_ref()?;
    let o = layer.shape_state.origin?;
    let [sw, sh] = layer.size_px.map(|v| v.max(1) as f32);
    let w = (sw * px_per_unit).round().max(1.0);
    let h = (sh * px_per_unit).round().max(1.0);
    let mut pixmap = Pixmap::new(w as u32, h as u32)?;
    // Exactly the layer's box, even where the pixel size was rounded.
    let ts = Transform::from_translate(-o[0], -o[1])
        .post_scale(UNITS_PER_MM * w / sw, UNITS_PER_MM * h / sh);
    // The painted strokes are cut off at the outline; while the shape is resized, paint
    // that doesn't stretch is drawn where it was.
    let paint_ts = match paint_map(layer) {
        Some(m) => ts.pre_concat(Transform::from_row(m.a, m.b, m.c, m.d, m.e, m.f)),
        None => ts,
    };
    let mut clip = None;
    for (path, ink, clipped, rule) in shape.coats() {
        if clipped && clip.is_none() {
            let mut mask = Mask::new(pixmap.width(), pixmap.height())?;
            if let Some(outline) = shape.path() {
                mask.fill_path(&outline, shape.fill_rule(), true, ts);
            }
            clip = Some(mask);
        }
        let mask = clip.as_ref().filter(|_| clipped);
        let t = if clipped { paint_ts } else { ts };
        pixmap.fill_path(&path, &ink.paint(), rule, t, mask);
    }
    Some(pixmap)
}

/// The layer's shape as filled shapes, for vector output such as PDF.
pub fn outlines(layer: &Layer) -> Option<Outlines> {
    let shape = layer.shape.as_ref()?;
    let o = layer.shape_state.origin?;
    let ts = Transform::from_translate(-o[0], -o[1]).post_scale(UNITS_PER_MM, UNITS_PER_MM);
    // The paths, and the box a gradient spans, from the shape's mm to layer units.
    let coats: Vec<_> = shape
        .coats()
        .into_iter()
        .filter_map(|(path, ink, clipped, rule)| {
            let ink = match ink {
                Ink::Gradient { gradient, frame } => Ink::Gradient {
                    gradient,
                    frame: frame.transform(ts)?,
                },
                solid => solid,
            };
            Some((path.transform(ts)?, ink, clipped, rule))
        })
        .collect();
    // The painted strokes, one after the other, are cut off at the outline.
    let first = coats.iter().position(|c| c.2);
    let count = coats.iter().filter(|c| c.2).count();
    let clip = first
        .zip(shape.path().and_then(|p| p.transform(ts)))
        .map(|(i, p)| (p, i..i + count, shape.fill_rule()));
    let fills = coats
        .into_iter()
        .map(|(p, ink, _, rule)| (p, ink, rule))
        .collect();
    Some(Outlines {
        fills,
        clip,
        map: layer.homography(),
    })
}

fn key(layer: &Layer, ppu: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    layer.shape_state.layout_key.hash(&mut h);
    layer.size_px.hash(&mut h);
    ppu.to_bits().hash(&mut h);
    // Paint that doesn't stretch is drawn again as the shape is resized.
    if let Some(m) = paint_map(layer) {
        for v in [m.a, m.b, m.c, m.d, m.e, m.f] {
            v.to_bits().hash(&mut h);
        }
    }
    h.finish()
}

/// Brings shape layers' boxes and pixels up to date with their shapes. With `settle`
/// (no drag going on), a scale given to a layer goes into its shape. Returns true if one
/// was redrawn.
pub fn sync(ctx: &egui::Context, doc: &mut Document, settle: bool) -> bool {
    let max_side = text::screen_max_side(ctx);
    let mut changed = false;
    doc.for_each_layer_mut(|layer| {
        if layer.shape.is_none() {
            return;
        }
        layout(layer, settle);
        let ppu = text::screen_ppu(layer, layer.size_px, max_side);
        let key = key(layer, ppu);
        if layer.shape_state.key == key && layer.pixmap.is_some() {
            return;
        }
        if let Some(pixmap) = draw(layer, ppu) {
            layer.shape_state.key = key;
            text::set_screen_pixmap(ctx, layer, pixmap);
            changed = true;
        }
    });
    changed
}

/// Redraws the visible shape layers of `doc` (a copy made for the export) at the export
/// resolution.
pub fn prepare_export(doc: &mut Document, dpi: f32) {
    let px_per_mm = dpi / 25.4;
    doc.for_each_layer_mut(|layer| {
        if layer.shape.is_none() || !layer.visible {
            return;
        }
        layout(layer, true);
        let ppu = text::px_per_unit(layer, layer.size_px, px_per_mm, text::EXPORT_SIDE);
        if let Some(pixmap) = draw(layer, ppu) {
            layer.pixmap = Some(std::sync::Arc::new(pixmap));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P2, b: P2) -> bool {
        (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05
    }

    fn layer(kind: ShapeKind) -> Layer {
        new_layer(1, kind, [100.0, 80.0])
    }

    #[test]
    fn new_layer_box_holds_shape_and_stroke() {
        let l = layer(ShapeKind::Rectangle);
        // 40 x 25 mm with a 0.5 mm stroke centered on the outline.
        assert_eq!(l.size_px, [405, 255]);
        assert!(close(l.size_mm(), [40.5, 25.5]));
        assert!(close(l.transform.center, [100.0, 80.0]));
        // A horizontal line is as thick as its stroke, round caps included.
        let line = layer(ShapeKind::Line);
        assert_eq!(line.size_px, [410, 10]);
    }

    #[test]
    fn every_kind_draws_something() {
        for kind in ShapeKind::ALL {
            let l = layer(kind);
            let pm = draw(&l, 1.0).unwrap();
            assert!(pm.pixels().iter().any(|p| p.alpha() > 0), "{kind:?}");
            assert!(!outlines(&l).unwrap().fills.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn scaling_goes_into_geometry_and_keeps_stroke_width() {
        let mut l = layer(ShapeKind::Ellipse);
        l.transform.rotation_deg = 30.0;
        let s = l.transform.scale;
        // A stretch on the canvas: twice as wide.
        l.transform.scale = [s[0] * 2.0, s[1]];
        let before = page_of(&l, [15.0, 0.0]).unwrap();
        layout(&mut l, false);
        assert_eq!(
            l.shape.as_ref().unwrap().size,
            [30.0, 30.0],
            "not while dragging"
        );
        layout(&mut l, true);
        let shape = l.shape.as_ref().unwrap();
        assert!(close(shape.size, [60.0, 30.0]), "{:?}", shape.size);
        assert_eq!(shape.stroke_width, 0.5);
        assert_eq!(l.transform.scale, [0.1, 0.1]);
        // The right end of the ellipse is still where the stretched layer showed it.
        let after = page_of(&l, [30.0, 0.0]).unwrap();
        assert!(close(before, after), "{before:?} {after:?}");
    }

    #[test]
    fn thicker_stroke_grows_box_around_same_place() {
        let mut l = layer(ShapeKind::Polyline);
        let p = page_of(&l, [-20.0, 8.0]).unwrap();
        l.shape.as_mut().unwrap().stroke_width = 4.0;
        layout(&mut l, true);
        assert!(close(page_of(&l, [-20.0, 8.0]).unwrap(), p));
        assert!(l.size_mm()[0] > 43.9);
    }

    #[test]
    fn kind_changes_keep_the_shape_in_place() {
        let mut l = layer(ShapeKind::Rectangle);
        l.transform.rotation_deg = 90.0;
        layout(&mut l, true);
        let corner = page_of(&l, [20.0, -12.5]).unwrap();
        set_kind(&mut l, ShapeKind::Polygon);
        layout(&mut l, true);
        let shape = l.shape.as_ref().unwrap();
        assert_eq!(shape.points.len(), 4);
        assert!(close(page_of(&l, shape.points[1]).unwrap(), corner));

        // Points off center become a box around them, in the same place.
        let mut l = layer(ShapeKind::Polygon);
        l.shape.as_mut().unwrap().points = vec![[10.0, 10.0], [30.0, 10.0], [30.0, 20.0]];
        layout(&mut l, true);
        let mid = page_of(&l, [20.0, 15.0]).unwrap();
        set_kind(&mut l, ShapeKind::Ellipse);
        layout(&mut l, true);
        assert!(close(l.shape.as_ref().unwrap().size, [20.0, 10.0]));
        assert!(close(page_of(&l, [0.0, 0.0]).unwrap(), mid));
        assert!(close(l.transform.center, mid));

        // A line has no fill; a closed shape made from it gets one.
        let mut l = layer(ShapeKind::Line);
        set_kind(&mut l, ShapeKind::Polygon);
        let shape = l.shape.as_ref().unwrap();
        assert_eq!(shape.points.len(), 3);
        assert!(shape.fill);
    }

    #[test]
    fn resize_sets_geometry_size_around_its_center() {
        let mut l = layer(ShapeKind::Polyline);
        let c = l.transform.center;
        set_geometry_size(&mut l, [80.0, 16.0]);
        layout(&mut l, true);
        let shape = l.shape.as_ref().unwrap();
        assert!(close(shape.geometry_size(), [80.0, 16.0]));
        assert!(close(l.transform.center, c));
        // A horizontal line has no height to scale.
        let mut line = layer(ShapeKind::Line);
        set_geometry_size(&mut line, [10.0, 5.0]);
        assert!(close(
            line.shape.as_ref().unwrap().geometry_size(),
            [10.0, 0.0]
        ));
    }

    #[test]
    fn dashes_leave_gaps() {
        let mut l = layer(ShapeKind::Line);
        let solid = draw(&l, 1.0).unwrap();
        let count = |pm: &Pixmap| pm.pixels().iter().filter(|p| p.alpha() > 128).count();
        for dash in [DashStyle::Dashed, DashStyle::Dotted, DashStyle::DashDot] {
            for cap in Cap::ALL {
                let s = l.shape.as_mut().unwrap();
                s.dash = dash;
                s.cap = cap;
                layout(&mut l, true);
                let n = count(&draw(&l, 1.0).unwrap());
                assert!(n > 0 && n < count(&solid) * 4 / 5, "{dash:?} {cap:?}: {n}");
            }
        }
    }

    /// A 20 mm square with a 10 mm square inside, as a path.
    fn square_with_hole(even_odd: bool) -> Layer {
        let mut s = Shape::new(ShapeKind::Path);
        s.points = vec![
            [-10.0, -10.0],
            [10.0, -10.0],
            [10.0, 10.0],
            [-10.0, 10.0],
            [-5.0, -5.0],
            [5.0, -5.0],
            [5.0, 5.0],
            [-5.0, 5.0],
        ];
        s.contours = vec![
            Contour {
                len: 4,
                closed: true,
            },
            Contour {
                len: 4,
                closed: true,
            },
        ];
        s.even_odd = even_odd;
        s.stroke = false;
        new_shape_layer(1, "p".into(), s, [50.0, 50.0])
    }

    #[test]
    fn path_contours_and_holes() {
        let l = square_with_hole(true);
        let s = l.shape.as_deref().unwrap();
        assert_eq!(s.segment_count(), 8);
        // The last segment of each contour goes back to its first point.
        assert_eq!(s.segment_ends(3), Some((3, 0)));
        assert_eq!(s.segment_ends(7), Some((7, 4)));
        assert_eq!(s.neighbors(4), [Some(7), Some(5)]);
        assert!(close(l.transform.center, [50.0, 50.0]));
        let pm = draw(&l, 1.0).unwrap();
        let alpha = |x, y| pm.pixel(x, y).unwrap().alpha();
        let [w, h] = [pm.width() / 2, pm.height() / 2];
        assert!(alpha(w, h) == 0, "a hole in the middle");
        assert!(alpha(w / 4, h) == 255);
        // Under the non-zero rule, the inner square turning the same way fills it.
        let pm = draw(&square_with_hole(false), 1.0).unwrap();
        assert!(pm.pixel(w, h).unwrap().alpha() == 255);
        // The PDF gets the same rule.
        let o = outlines(&square_with_hole(true)).unwrap();
        assert_eq!(o.fills[0].2, FillRule::EvenOdd);
    }

    #[test]
    fn path_points_edited_per_contour() {
        let mut l = square_with_hole(true);
        // A point on the inner square's closing segment goes at the end of its contour.
        assert!(split_segment(&mut l, 7, 0.5));
        let s = l.shape.as_deref().unwrap();
        assert_eq!(s.contours[1].len, 5);
        assert!(close(s.points[8], [-5.0, 0.0]));
        assert_eq!(s.segment_count(), 9);
        // Contours keep two points each.
        for _ in 0..3 {
            assert!(remove_point(&mut l, 4));
        }
        assert!(!remove_point(&mut l, 4));
        assert_eq!(l.shape.as_deref().unwrap().contours[1].len, 2);
        // It stays a path, keeping both contours.
        set_kind(&mut l, ShapeKind::Polygon);
        let s = l.shape.as_deref().unwrap();
        assert_eq!(s.kind, ShapeKind::Path);
        assert_eq!(s.contours.len(), 2);
    }

    #[test]
    fn star_and_polygon_fill_their_box() {
        let mut s = Shape::new(ShapeKind::Star);
        s.size = [50.0, 20.0];
        let (lo, hi) = bbox(&s.star_points()).unwrap();
        assert!(close(lo, [-25.0, -10.0]) && close(hi, [25.0, 10.0]));
        assert_eq!(s.star_points().len(), 10);
        s.kind = ShapeKind::RegularPolygon;
        s.sides = 3;
        assert_eq!(s.star_points().len(), 3);
    }

    #[test]
    fn points_move_on_the_page_and_others_stay() {
        let mut l = layer(ShapeKind::Polyline);
        l.transform.rotation_deg = 30.0;
        l.transform.flip_x = true;
        layout(&mut l, true);
        let before = page_points(&l);
        assert_eq!(before.len(), 4);
        // Point 0 lands where it was asked to, far out: the box grows around it.
        let target = [before[0][0] - 30.0, before[0][1] + 12.0];
        assert!(move_point(&mut l, 0, target));
        layout(&mut l, true);
        let after = page_points(&l);
        assert!(close(after[0], target), "{:?} {target:?}", after[0]);
        for k in 1..4 {
            assert!(close(after[k], before[k]), "point {k} stayed");
        }
        assert!(!move_point(&mut l, 0, target), "already there");

        // Added between points 1 and 2, then removed.
        let mid = [
            (after[1][0] + after[2][0]) / 2.0,
            (after[1][1] + after[2][1]) / 2.0,
        ];
        assert!(split_segment(&mut l, 1, 0.5));
        layout(&mut l, true);
        assert!(close(page_points(&l)[2], mid));
        assert!(remove_point(&mut l, 2));
        assert_eq!(l.shape.as_ref().unwrap().points.len(), 4);

        // A line keeps its two points; a polygon at least three.
        let mut line = layer(ShapeKind::Line);
        assert!(!split_segment(&mut line, 0, 0.5));
        assert!(!remove_point(&mut line, 0));
        let mut tri = layer(ShapeKind::Polygon);
        assert!(!remove_point(&mut tri, 0));
        // Not a shape with points.
        let mut rect = layer(ShapeKind::Rectangle);
        assert!(!move_point(&mut rect, 0, [0.0, 0.0]));
        assert!(page_points(&rect).is_empty());
    }

    #[test]
    fn snap_points_leave_the_stroke_out() {
        let mut l = layer(ShapeKind::Rectangle);
        l.shape.as_mut().unwrap().stroke_width = 4.0;
        layout(&mut l, true);
        let p = snap_points(&l);
        // Center, then the corners of the 40 x 25 box around [100, 80].
        assert!(close(p[0], [100.0, 80.0]));
        assert!(close(p[1], [80.0, 67.5]) && close(p[3], [120.0, 92.5]));
        // Then the middles of its sides: top, right, bottom, left.
        assert!(close(p[5], [100.0, 67.5]) && close(p[6], [120.0, 80.0]));
        assert!(close(p[7], [100.0, 92.5]) && close(p[8], [80.0, 80.0]));
        let line = layer(ShapeKind::Line);
        let p = snap_points(&line);
        assert_eq!(p.len(), 3);
        assert!(close(p[1], [80.0, 80.0]) && close(p[2], [120.0, 80.0]));
    }

    #[test]
    fn curves_split_turn_smooth_and_follow_their_handles() {
        // An ellipse becomes a polygon of four smooth points that draws the same.
        let mut l = layer(ShapeKind::Ellipse);
        let before = l.size_px;
        set_kind(&mut l, ShapeKind::Polygon);
        layout(&mut l, true);
        let s = l.shape.as_ref().unwrap();
        assert_eq!((s.points.len(), s.handles.len()), (4, 4));
        assert!((0..4).all(|i| s.is_smooth(i)));
        assert!(
            close(s.geometry_size(), [30.0, 30.0]),
            "{:?}",
            s.geometry_size()
        );
        assert_eq!(l.size_px, before);

        // Splitting a curve keeps its shape: the new point lies on it.
        let c = s.segment(0).unwrap();
        let on = cubic_at(c, 0.3);
        assert!(split_segment(&mut l, 0, 0.3));
        let s = l.shape.as_ref().unwrap();
        assert_eq!(s.points.len(), 5);
        assert!(close(s.points[1], on));
        for t in [0.2, 0.7] {
            // The first half of the old curve is the new first segment.
            let old = cubic_at(c, 0.3 * t);
            assert!(close(cubic_at(s.segment(0).unwrap(), t), old));
        }

        // Dragging a handle of a smooth point turns the other with it.
        let p = page_points(&l)[1];
        assert!(set_handle(&mut l, 1, 1, [p[0], p[1] + 5.0], Mirror::Turn));
        let s = l.shape.as_ref().unwrap();
        assert!(s.is_smooth(1));
        // Without mirroring, they part.
        assert!(set_handle(&mut l, 1, 1, [p[0] + 5.0, p[1]], Mirror::None));
        assert!(!l.shape.as_ref().unwrap().is_smooth(1));

        // A straight polyline: a point made smooth gets handles, then loses them.
        let mut l = layer(ShapeKind::Polyline);
        assert!(toggle_smooth(&mut l, 1));
        let s = l.shape.as_ref().unwrap();
        assert!(s.is_smooth(1) && s.handles.len() == 4);
        assert!(
            s.path()
                .unwrap()
                .segments()
                .any(|g| matches!(g, tiny_skia::PathSegment::CubicTo(..)))
        );
        assert!(toggle_smooth(&mut l, 1));
        assert!(
            l.shape.as_ref().unwrap().handles.is_empty(),
            "straight again"
        );
        // Removing and adding points keeps one pair of handles per point.
        toggle_smooth(&mut l, 2);
        assert!(remove_point(&mut l, 0));
        let s = l.shape.as_ref().unwrap();
        assert_eq!(s.handles.len(), s.points.len());
        assert!(s.is_smooth(1), "the smooth point moved down one");
        // Scaling scales the handles too.
        let h = s.handle(1)[1];
        let g = s.geometry_size();
        set_geometry_size(&mut l, [g[0] * 2.0, g[1]]);
        let h2 = l.shape.as_ref().unwrap().handle(1)[1];
        assert!(
            (h2[0] - h[0] * 2.0).abs() < 0.2 && (h2[1] - h[1]).abs() < 1e-3,
            "{h:?} {h2:?}"
        );
    }

    #[test]
    fn saved_shape_loads_back() {
        let l = layer(ShapeKind::Star);
        let json = serde_json::to_string(&l).unwrap();
        assert!(!json.contains("points"), "only for lines and polygons");
        let back: Layer = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shape, l.shape);
    }

    #[test]
    fn a_canvas_keeps_its_box_and_cuts_its_strokes_off_there() {
        use crate::paint::Tool;
        let mut l = new_canvas_layer(
            1,
            crate::shape::ShapeKind::Rectangle,
            [10.0, 20.0],
            [110.0, 70.0],
        );
        let size = l.size_px;
        assert_eq!(size, [1000, 500]);
        // Past its right edge: the box stays, the stroke is kept.
        let line = [[30.0, 45.0, 1.0], [200.0, 45.0, 1.0]];
        assert!(add_stroke(
            &mut l,
            &line,
            &crate::paint::Pen::new(Tool::Pencil, 2.0, [0, 0, 0, 255])
        ));
        layout(&mut l, false);
        assert_eq!(l.size_px, size);
        assert!(close(l.corners()[0], [10.0, 20.0]), "{:?}", l.corners());
        // All outside: left out.
        let out = [[150.0, 45.0, 1.0], [200.0, 45.0, 1.0]];
        assert!(!add_stroke(
            &mut l,
            &out,
            &crate::paint::Pen::new(Tool::Pencil, 2.0, [0, 0, 0, 255])
        ));
        // Cut off in the PDF at its box.
        let o = outlines(&l).unwrap();
        let (clip, range, _) = o.clip.expect("a clip");
        assert_eq!(range, 0..1, "the stroke, nothing else");
        let clip = clip.compute_tight_bounds().unwrap();
        assert_eq!(
            [clip.left(), clip.top(), clip.right(), clip.bottom()],
            [0.0, 0.0, 1000.0, 500.0]
        );
        // Resized by its handles: the strokes stretch with it.
        l.transform.scale = [0.2, 0.1];
        layout(&mut l, true);
        let shape = l.shape.as_ref().unwrap();
        assert!(close(shape.size, [200.0, 50.0]), "{:?}", shape.size);
        assert!(close(
            [shape.strokes[0].points[0][0], shape.strokes[0].points[0][1]],
            [-60.0, 0.0]
        ));
        // Saved as a Canvas or a Drawing layer: a rectangle, the strokes kept.
        let json = serde_json::to_string(&l)
            .unwrap()
            .replace("\"Rectangle\"", "\"Drawing\"");
        let back: Layer = serde_json::from_str(&json).unwrap();
        assert!(paintable(&back));
        assert_eq!(back.shape, l.shape);
    }

    #[test]
    fn paint_in_a_shape_goes_between_its_fill_and_its_stroke_cut_off_at_its_outline() {
        use crate::paint::Tool;
        let mut l = new_layer(1, ShapeKind::Ellipse, [50.0, 50.0]);
        // Across the whole box, corners included.
        let line = [[30.0, 40.0, 1.0], [70.0, 40.0, 1.0]];
        assert!(add_stroke(
            &mut l,
            &line,
            &crate::paint::Pen::new(Tool::Pencil, 3.0, [255, 0, 0, 255])
        ));
        let coats = l.shape.as_ref().unwrap().coats();
        assert_eq!(
            coats.iter().map(|c| c.2).collect::<Vec<_>>(),
            [false, true, false],
            "fill, paint, stroke"
        );
        let o = outlines(&l).unwrap();
        assert_eq!(o.clip.as_ref().unwrap().1, 1..2);
        // Drawn: red inside the ellipse, nothing at the box's corner.
        let pixmap = draw(&l, 1.0).unwrap();
        let at = |page: P2| {
            let o = l.shape_state.origin.unwrap();
            let p = from_page(&l, page).unwrap();
            let [x, y] = [(p[0] - o[0]) * UNITS_PER_MM, (p[1] - o[1]) * UNITS_PER_MM];
            pixmap.pixel(x as u32, y as u32).unwrap()
        };
        let mid = at([50.0, 40.0]);
        assert_eq!([mid.red(), mid.green()], [255, 0]);
        assert_eq!(at([31.0, 40.0]).alpha(), 0, "outside the ellipse");
        // A line has no inside: nothing painted shows.
        let mut line_layer = l.clone();
        set_kind(&mut line_layer, ShapeKind::Line);
        assert!(!paintable(&line_layer));
        assert!(
            line_layer
                .shape
                .as_ref()
                .unwrap()
                .coats()
                .iter()
                .all(|c| !c.2)
        );
    }

    #[test]
    fn paint_stays_in_place_when_resizing_without_stretching_it() {
        use crate::paint::Tool;
        let page_ends = |l: &Layer| -> Vec<P2> {
            l.shape.as_ref().unwrap().strokes[0]
                .points
                .iter()
                .map(|p| to_page(l, [p[0], p[1]]).unwrap())
                .collect()
        };
        let mut l = new_canvas_layer(1, ShapeKind::Rectangle, [0.0, 0.0], [100.0, 50.0]);
        let line = [[20.0, 25.0, 1.0], [60.0, 25.0, 1.0]];
        assert!(add_stroke(
            &mut l,
            &line,
            &crate::paint::Pen::new(Tool::Pencil, 2.0, [0, 0, 0, 255])
        ));
        let before = page_ends(&l);
        l.shape.as_mut().unwrap().stretch_paint = false;

        // By the handles: twice as wide, the left edge kept (as a drag on the right edge
        // leaves it), then settled.
        l.transform.scale[0] *= 2.0;
        l.transform.center[0] += 50.0;
        layout(&mut l, false);
        assert!(paint_map(&l).is_some(), "drawn in place while resized");
        layout(&mut l, true);
        assert!(close(l.shape.as_ref().unwrap().size, [200.0, 50.0]));
        let after = page_ends(&l);
        for (a, b) in before.iter().zip(&after) {
            assert!(close(*a, *b), "{before:?} {after:?}");
        }
        // By the Size field.
        set_geometry_size(&mut l, [80.0, 40.0]);
        layout(&mut l, true);
        for (a, b) in before.iter().zip(&page_ends(&l)) {
            assert!(close(*a, *b));
        }

        // Stretched, the default: the stroke grows with the shape.
        l.shape.as_mut().unwrap().stretch_paint = true;
        set_geometry_size(&mut l, [160.0, 40.0]);
        layout(&mut l, true);
        let e = page_ends(&l);
        let len = |e: &[P2]| e[e.len() - 1][0] - e[0][0];
        assert!((len(&e) - 2.0 * len(&before)).abs() < 0.01, "{e:?}");
    }

    #[test]
    fn a_calligraphy_nib_keeps_its_angle_on_the_page_in_a_turned_shape() {
        use crate::paint::{BrushKind, Pen, Tip};
        let mut l = new_layer(1, ShapeKind::Rectangle, [100.0, 100.0]);
        l.transform.rotation_deg = 90.0;
        layout(&mut l, false);
        let line = [[95.0, 100.0, 1.0], [105.0, 100.0, 1.0]];
        let pen = Pen {
            angle: 0.0,
            ..Pen::brush(BrushKind::Calligraphy, 2.0, [0, 0, 0, 255])
        };
        assert!(add_stroke(&mut l, &line, &pen));
        let Tip::Nib { angle } = l.shape.as_ref().unwrap().strokes[0].tip else {
            panic!("a nib");
        };
        // Flat on the page: upright in the shape's frame.
        assert!(
            (angle.abs() - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "{angle}"
        );
    }
}
