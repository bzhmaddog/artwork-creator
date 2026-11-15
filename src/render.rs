//! Offscreen compositing shared by PNG and PDF export.

use std::sync::Arc;

use tiny_skia::{
    FillRule, FilterQuality, IntSize, Mask, Paint, PathBuilder, Pixmap, PixmapMut, PixmapPaint,
    Stroke, Transform,
};

use crate::document::{Affine, BlendMode, Document, Homography, Node, P2};
use crate::effects::{self, EffectInstance};
use crate::i18n::tr;
use crate::print;
use crate::template::{ElementKind, KINDS, Polyline, Template};
use crate::text;

/// Width of printed cut lines.
pub const OUTLINE_WIDTH_MM: f32 = 0.25;

/// An image layer ready to draw.
#[derive(Clone)]
pub struct SceneLayer {
    /// Premultiplied pixels.
    pub pixmap: Arc<Pixmap>,
    /// Pixmap pixels to page mm; projective when the layer is distorted.
    pub map: Homography,
    /// Its opacity, with any group opacity folded in.
    pub opacity: f32,
    /// How it mixes with what lies below.
    pub blend: BlendMode,
    /// Effects still to apply to `pixmap` when drawing: empty for previews, whose
    /// pixmaps already have them.
    pub effects: Vec<EffectInstance>,
    /// Size of `pixmap` relative to the source image, for the effects.
    pub effects_scale: f32,
    /// A text or shape layer without effects as filled shapes, for formats that keep
    /// them as vectors. Not for previews.
    pub vector: Option<Arc<Outlines>>,
}

/// A text or shape layer as filled shapes, for vector output such as PDF.
#[derive(Debug)]
pub struct Outlines {
    /// Shapes in the layer's frame (layer units, y down), each filled with its color or
    /// gradient under its rule, bottom first.
    pub fills: Vec<(tiny_skia::Path, crate::gradient::Ink, FillRule)>,
    /// An outline in the same frame, the fills cut off at it (the strokes painted inside
    /// a shape) and the rule finding its inside, if any.
    pub clip: Option<(tiny_skia::Path, std::ops::Range<usize>, FillRule)>,
    /// Layer units to page mm, including any distortion.
    pub map: Homography,
}

/// A text or shape layer as filled shapes; `None` for images.
pub fn outlines(layer: &crate::document::Layer) -> Option<Outlines> {
    if layer.shape.is_some() {
        crate::shape::outlines(layer)
    } else {
        text::outlines(layer)
    }
}

impl SceneLayer {
    /// Its shapes, if drawing them as vectors gives what its pixels show: they are drawn
    /// one by one, so with several (outline, fake bold, a shape's fill and stroke) the
    /// layer must be opaque.
    /// A gradient goes in as vectors only when opaque (PDF gradients are RGB only) and
    /// on a layer that is not distorted (they cannot follow a perspective).
    pub fn vector(&self) -> Option<&Outlines> {
        let v = self.vector.as_deref()?;
        let gradients_fit = v.fills.iter().all(|(path, ink, _)| match ink {
            crate::gradient::Ink::Solid(_) => true,
            crate::gradient::Ink::Gradient { gradient, .. } => {
                gradient.is_opaque() && v.map.as_affine().is_some()
            }
            // Its tiles drawn one by one, so not too many.
            crate::gradient::Ink::Pattern { tile, transform } => {
                v.map.as_affine().is_some()
                    && crate::pattern::tiles(path.bounds(), tile.width(), tile.height(), *transform)
                        .is_some_and(|[x0, x1, y0, y1]| {
                            (x1 - x0) * (y1 - y0) <= crate::pattern::MAX_VECTOR_TILES
                        })
            }
        });
        // Several shapes in another mode than normal would each mix with the ones
        // below them, where the pixels mix once.
        let single = v.fills.len() <= 1;
        (gradients_fit && (self.opacity >= 1.0 || single) && (self.blend.is_normal() || single))
            .then_some(v)
    }

    /// Its shapes, when they show exactly where its pixels do, and fully: then what is
    /// clipped to it can be cut by their outline as vectors (`clip_rings`).
    pub fn clip_outline(&self) -> Option<&Outlines> {
        let v = self.vector()?;
        let opaque = v.fills.iter().all(|(_, ink, _)| match ink {
            crate::gradient::Ink::Solid(c) => c[3] == 255,
            crate::gradient::Ink::Gradient { gradient, .. } => gradient.is_opaque(),
            crate::gradient::Ink::Pattern { tile, .. } => crate::pattern::opaque(tile),
        });
        (opaque && self.opacity >= 1.0 && v.clip.is_none()).then_some(v)
    }
}

/// Longest straight piece a curve becomes in `clip_rings`, in mm.
const CLIP_PIECE_MM: f32 = 0.05;

/// The outline of all the shapes of `v` together, in page mm, as rings to fill under
/// the nonzero rule: curves become straight pieces of at most `CLIP_PIECE_MM`, and each
/// shape keeps its own fill rule. For formats whose clipping path has a single rule
/// (PDF).
pub fn clip_rings(v: &Outlines) -> Vec<Vec<P2>> {
    use i_overlay::core::fill_rule::FillRule as Rule;
    use i_overlay::float::simplify::SimplifyShape;
    let mut all: Vec<Vec<P2>> = Vec::new();
    for (path, _, rule) in &v.fills {
        let rings = flat_rings(path, v.map);
        let rule = match rule {
            FillRule::Winding => Rule::NonZero,
            FillRule::EvenOdd => Rule::EvenOdd,
        };
        // Oriented by what they enclose, so that together they add up under nonzero.
        all.extend(rings.simplify_shape(rule).into_iter().flatten());
    }
    all.simplify_shape(Rule::NonZero)
        .into_iter()
        .flatten()
        .collect()
}

/// A path's contours mapped to page mm by `map`, curves as straight pieces.
fn flat_rings(path: &tiny_skia::Path, map: Homography) -> Vec<Vec<P2>> {
    use tiny_skia::PathSegment;
    let at = |p: tiny_skia::Point| map.apply([p.x, p.y]);
    let mut out: Vec<Vec<P2>> = Vec::new();
    let mut last = tiny_skia::Point::zero();
    // Points along a curve after its start, given its control points in path units.
    let curve = |pts: &[tiny_skia::Point], out: &mut Vec<Vec<P2>>| {
        let mapped: Vec<P2> = pts.iter().map(|&p| at(p)).collect();
        let len: f32 = mapped
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum();
        let n = (len / CLIP_PIECE_MM).ceil().clamp(1.0, 256.0) as usize;
        let ring = out.last_mut();
        let Some(ring) = ring else { return };
        for k in 1..=n {
            let t = k as f32 / n as f32;
            // De Casteljau on the control points.
            let mut q: Vec<tiny_skia::Point> = pts.to_vec();
            while q.len() > 1 {
                q = q
                    .windows(2)
                    .map(|w| {
                        tiny_skia::Point::from_xy(
                            w[0].x + (w[1].x - w[0].x) * t,
                            w[0].y + (w[1].y - w[0].y) * t,
                        )
                    })
                    .collect();
            }
            ring.push(at(q[0]));
        }
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => out.push(vec![at(p)]),
            PathSegment::LineTo(p) => {
                if let Some(ring) = out.last_mut() {
                    ring.push(at(p));
                }
            }
            PathSegment::QuadTo(c, p) => curve(&[last, c, p], &mut out),
            PathSegment::CubicTo(c1, c2, p) => curve(&[last, c1, c2, p], &mut out),
            PathSegment::Close => {}
        }
        last = match seg {
            PathSegment::MoveTo(p)
            | PathSegment::LineTo(p)
            | PathSegment::QuadTo(_, p)
            | PathSegment::CubicTo(_, _, p) => p,
            PathSegment::Close => last,
        };
    }
    out.retain(|r| r.len() > 2);
    out
}

/// What gets drawn: an image, or a group composited on its own.
pub enum SceneNode {
    Image(SceneLayer),
    /// Children are composited on their own, then blended with `opacity`.
    Group {
        opacity: f32,
        blend: BlendMode,
        children: Vec<SceneNode>,
    },
    /// `base` is drawn, then `clipped` composited on their own and kept only where
    /// `base` shows, as much as it shows (see `Node::clip`).
    Clip {
        base: Box<SceneNode>,
        clipped: Vec<SceneNode>,
    },
}

impl SceneNode {
    /// The node's opacity, image or group; `None` for a clipped stack, which has none of
    /// its own.
    fn opacity_mut(&mut self) -> Option<&mut f32> {
        match self {
            SceneNode::Image(l) => Some(&mut l.opacity),
            SceneNode::Group { opacity, .. } => Some(opacity),
            SceneNode::Clip { .. } => None,
        }
    }

    /// How it mixes with what lies below; a clipped stack, normally.
    fn blend(&self) -> BlendMode {
        match self {
            SceneNode::Image(l) => l.blend,
            SceneNode::Group { blend, .. } => *blend,
            SceneNode::Clip { .. } => BlendMode::Normal,
        }
    }

    /// It, or something in it, mixes in a mode other than normal.
    fn has_blend(&self) -> bool {
        match self {
            SceneNode::Image(l) => !l.blend.is_normal(),
            SceneNode::Group {
                blend, children, ..
            } => !blend.is_normal() || children.iter().any(SceneNode::has_blend),
            SceneNode::Clip { base, clipped } => {
                base.has_blend() || clipped.iter().any(SceneNode::has_blend)
            }
        }
    }
}

/// Converts layers and groups into what gets drawn: hidden or empty entries are dropped,
/// opaque groups are flattened, and a group with a single child folds its opacity into
/// that child (same result, no offscreen buffer). A node with nodes clipped to it
/// becomes a `SceneNode::Clip`; when it draws nothing, neither do they. With `preview`,
/// uses the layers' smaller preview pixmaps.
pub fn scene_nodes(nodes: &[Node], preview: bool) -> Vec<SceneNode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        let start = i;
        i += 1;
        // Clipped nodes at the bottom of the list have no base: drawn as they are.
        if nodes[start].clip() {
            scene_node(&nodes[start], preview, &mut out);
            continue;
        }
        while i < nodes.len() && nodes[i].clip() {
            i += 1;
        }
        if i == start + 1 {
            scene_node(&nodes[start], preview, &mut out);
            continue;
        }
        let mut base = Vec::new();
        scene_node(&nodes[start], preview, &mut base);
        let base = match base.len() {
            0 => continue,
            1 => base.remove(0),
            _ => SceneNode::Group {
                opacity: 1.0,
                blend: BlendMode::Normal,
                children: base,
            },
        };
        let mut clipped = Vec::new();
        for n in &nodes[start + 1..i] {
            scene_node(n, preview, &mut clipped);
        }
        out.push(if clipped.is_empty() {
            base
        } else {
            SceneNode::Clip {
                base: Box::new(base),
                clipped,
            }
        });
    }
    out
}

/// Appends what one layer or group draws to `out`, as for `scene_nodes`.
fn scene_node(node: &Node, preview: bool, out: &mut Vec<SceneNode>) {
    {
        match node {
            Node::Layer(l) => {
                if !l.visible || l.opacity <= 0.0 {
                    return;
                }
                let src = if preview {
                    l.shown_preview()
                } else {
                    l.pixmap.as_ref()
                };
                let Some(pm) = src else { return };
                let effects = if !preview && effects::is_active(&l.effects) {
                    l.effects.clone()
                } else {
                    Vec::new()
                };
                // Area of the pixmap in source pixels: previews may carry margins drawn by
                // effects, and pixmaps may be smaller than the source image.
                let [x0, y0, x1, y1] = if preview {
                    l.shown_rect()
                } else {
                    [0.0, 0.0, l.size_px[0] as f32, l.size_px[1] as f32]
                };
                let k = [
                    (x1 - x0) / pm.width() as f32,
                    (y1 - y0) / pm.height() as f32,
                ];
                out.push(SceneNode::Image(SceneLayer {
                    pixmap: pm.clone(),
                    map: Homography::from(
                        Affine::scale(k[0], k[1]).then(Affine::translate(x0, y0)),
                    )
                    .then(l.homography()),
                    opacity: l.opacity.min(1.0),
                    blend: l.blend,
                    vector: (!preview && effects.is_empty())
                        .then(|| outlines(l).map(Arc::new))
                        .flatten(),
                    effects,
                    effects_scale: 1.0 / k[0],
                }));
            }
            Node::Group(g) => {
                if !g.visible || g.opacity <= 0.0 {
                    return;
                }
                let mut children = scene_nodes(&g.children, preview);
                // A group in normal mode lets its children mix with what lies below.
                if !g.blend.is_normal() {
                    if !children.is_empty() {
                        out.push(SceneNode::Group {
                            opacity: g.opacity,
                            blend: g.blend,
                            children,
                        });
                    }
                } else if g.opacity >= 1.0 {
                    out.append(&mut children);
                } else if let [only] = &mut children[..]
                    && only.blend().is_normal()
                    && let Some(opacity) = only.opacity_mut()
                {
                    *opacity *= g.opacity;
                    out.append(&mut children);
                } else if !children.is_empty() {
                    out.push(SceneNode::Group {
                        opacity: g.opacity,
                        blend: BlendMode::Normal,
                        children,
                    });
                }
            }
        }
    }
}

/// A template element placed on the page, in page mm.
#[derive(Clone, Debug)]
pub struct PlacedElement {
    /// What the element is for.
    pub kind: ElementKind,
    /// Its shapes in page mm.
    pub polylines: Vec<Polyline>,
    /// The centers of its closed shapes in page mm, which images snap to. Not printed.
    pub centers: Vec<P2>,
}

impl PlacedElement {
    /// The lines to print: its shapes.
    pub fn printed_lines(&self) -> impl Iterator<Item = std::borrow::Cow<'_, Polyline>> {
        self.polylines.iter().map(std::borrow::Cow::Borrowed)
    }
}

/// How the template cuts the artwork.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Cuts {
    /// Nothing outside the outline is printed.
    pub clip_to_outline: bool,
    /// Nothing is printed inside the holes. Read from `blank_buttons` too, saved by
    /// versions that had buttons and cutouts.
    #[serde(alias = "blank_buttons")]
    pub blank_holes: bool,
    /// The artwork goes this far past the cut lines, in mm, so a cut slightly off the
    /// line leaves no blank edge: the outline grows by it and the holes shrink by it.
    /// Set from the project's bleed, so not saved with the export settings.
    #[serde(skip)]
    pub bleed_mm: f32,
}

impl std::hash::Hash for Cuts {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (self.clip_to_outline, self.blank_holes).hash(h);
        self.bleed_mm.to_bits().hash(h);
    }
}

impl Cuts {
    /// Whether the inside of `kind` is left blank. The outline is never a hole.
    pub fn blanks(self, kind: ElementKind) -> bool {
        match kind {
            ElementKind::Outline => false,
            ElementKind::Holes => self.blank_holes,
        }
    }
}

impl Default for Cuts {
    fn default() -> Self {
        Self {
            clip_to_outline: true,
            blank_holes: true,
            bleed_mm: 0.0,
        }
    }
}

/// A template element as exported.
pub struct SceneElement {
    /// Its shapes on the page.
    pub placed: PlacedElement,
    /// Line color (RGB).
    pub color: [u8; 3],
    /// Its lines are printed.
    pub print: bool,
}

/// Everything needed to render the page, detached from the UI so it can move to a thread.
pub struct Scene {
    /// Page size in mm.
    pub page_mm: P2,
    /// Page color (RGB).
    pub background: [u8; 3],
    /// Fills the page instead of `background`, when set; opaque.
    pub background_gradient: Option<crate::gradient::Gradient>,
    /// Fills the page over `background`, instead of `background_gradient`, when set.
    pub background_pattern: Option<crate::pattern::Pattern>,
    /// What to draw, bottom first.
    pub layers: Vec<SceneNode>,
    /// Template elements on the page.
    pub elements: Vec<SceneElement>,
    /// What the template cuts away.
    pub cuts: Cuts,
    /// Crop and registration marks in page mm, printed in `print::MARK_COLOR`.
    pub marks: Vec<Polyline>,
}

impl Scene {
    /// The template elements as placed on the page.
    fn placed(&self) -> Vec<PlacedElement> {
        self.elements.iter().map(|e| e.placed.clone()).collect()
    }

    /// The page as exported. `colors` are per template element, in its order (the kind's
    /// default color for one missing); `print` is per element kind (`ElementKind::index`).
    pub fn new(
        doc: &Document,
        template: Option<&Template>,
        colors: &[[u8; 3]],
        print: [bool; KINDS],
        cuts: Cuts,
    ) -> Self {
        Self::build(doc, template, colors, print, cuts, false)
    }

    /// Like `new`, but with the layers' smaller preview images: quick to render small.
    pub fn preview(
        doc: &Document,
        template: Option<&Template>,
        colors: &[[u8; 3]],
        print: [bool; KINDS],
        cuts: Cuts,
    ) -> Self {
        Self::build(doc, template, colors, print, cuts, true)
    }

    fn build(
        doc: &Document,
        template: Option<&Template>,
        colors: &[[u8; 3]],
        print: [bool; KINDS],
        cuts: Cuts,
        preview: bool,
    ) -> Self {
        let elements = template
            .map(|t| placed_elements(doc, t))
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, placed)| SceneElement {
                color: colors
                    .get(i)
                    .copied()
                    .unwrap_or(placed.kind.default_color()),
                print: print[placed.kind.index()],
                placed,
            })
            .collect();
        Self {
            page_mm: doc.page_mm(),
            background: doc.background,
            background_gradient: doc.background_gradient.clone(),
            background_pattern: doc.background_pattern.clone(),
            layers: scene_nodes(&doc.layers, preview),
            elements,
            cuts,
            marks: Vec::new(),
        }
    }

    /// Adds crop and / or registration marks around the outline, outside the bleed. Does
    /// nothing without an outline.
    pub fn with_marks(mut self, crop: bool, registration: bool) -> Self {
        self.marks = trim_box(self.elements.iter().map(|e| &e.placed))
            .map(|trim| print::marks(trim, self.cuts.bleed_mm, crop, registration))
            .unwrap_or_default();
        self
    }
}

impl Scene {
    /// The page mirrored left to right (`horizontally`) and / or top to bottom
    /// (`vertically`), everything on it included (layers, template lines, cuts and
    /// marks): for printing on the back of a clear sheet, to be seen through it.
    pub fn mirrored(mut self, horizontally: bool, vertically: bool) -> Self {
        if !horizontally && !vertically {
            return self;
        }
        let [w, h] = self.page_mm;
        let (sx, dx) = if horizontally { (-1.0, w) } else { (1.0, 0.0) };
        let (sy, dy) = if vertically { (-1.0, h) } else { (1.0, 0.0) };
        let m = Affine::scale(sx, sy).then(Affine::translate(dx, dy));
        mirror_nodes(&mut self.layers, m);
        let flip = |p: &mut P2| *p = m.apply(*p);
        for e in &mut self.elements {
            e.placed
                .polylines
                .iter_mut()
                .flat_map(|p| &mut p.points)
                .for_each(flip);
            e.placed.centers.iter_mut().for_each(flip);
        }
        self.marks
            .iter_mut()
            .flat_map(|p| &mut p.points)
            .for_each(flip);
        self
    }
}

/// Maps every layer of `nodes`, and its vector shapes, through `m` after its own map.
fn mirror_nodes(nodes: &mut [SceneNode], m: Affine) {
    for node in nodes {
        match node {
            SceneNode::Image(l) => {
                l.map = l.map.then_affine(m);
                if let Some(v) = &l.vector {
                    l.vector = Some(Arc::new(Outlines {
                        fills: v.fills.clone(),
                        clip: v.clip.clone(),
                        map: v.map.then_affine(m),
                    }));
                }
            }
            SceneNode::Group { children, .. } => mirror_nodes(children, m),
            SceneNode::Clip { base, clipped } => {
                mirror_nodes(std::slice::from_mut(base), m);
                mirror_nodes(clipped, m);
            }
        }
    }
}

/// The box around the template's outline, where the print is trimmed: corners in mm.
pub fn trim_box<'a>(elements: impl Iterator<Item = &'a PlacedElement>) -> Option<(P2, P2)> {
    print::bounds(
        elements
            .filter(|e| e.kind == ElementKind::Outline)
            .flat_map(|e| &e.polylines),
    )
}

/// Template elements mapped onto the page, in mm.
pub fn placed_elements(doc: &Document, template: &Template) -> Vec<PlacedElement> {
    let m = doc.template_affine(template.size_mm);
    template
        .elements
        .iter()
        .map(|e| PlacedElement {
            kind: e.kind,
            polylines: e
                .polylines
                .iter()
                .map(|p| Polyline {
                    points: p.points.iter().map(|&q| m.apply(q)).collect(),
                    closed: p.closed,
                })
                .collect(),
            centers: e.centers.iter().map(|&q| m.apply(q)).collect(),
        })
        .collect()
}

/// Closed rings as one path, mapped by `to_px`.
fn closed_path(rings: &[Vec<P2>], to_px: Affine) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for ring in rings.iter().filter(|r| r.len() > 2) {
        let mut pts = ring.iter().map(|&p| to_px.apply(p));
        let Some(first) = pts.next() else { continue };
        pb.move_to(first[0], first[1]);
        for p in pts {
            pb.line_to(p[0], p[1]);
        }
        pb.close();
    }
    pb.finish()
}

/// What the template cuts away, as closed shapes in page mm filled under the non-zero
/// rule: the outline that clips, grown by the bleed (`None` if nothing is clipped), and
/// the holes left blank, shrunk by it. An outline without closed paths clips nothing.
pub fn cut_rings(elements: &[PlacedElement], cuts: Cuts) -> (Option<Vec<Vec<P2>>>, Vec<Vec<P2>>) {
    let rings = |keep: &dyn Fn(ElementKind) -> bool| -> Vec<Vec<P2>> {
        elements
            .iter()
            .filter(|e| keep(e.kind))
            .flat_map(|e| &e.polylines)
            .filter(|p| p.closed && p.points.len() > 2)
            .map(|p| p.points.clone())
            .collect()
    };
    let outline = if cuts.clip_to_outline {
        Some(rings(&|k| k == ElementKind::Outline))
            .filter(|r| !r.is_empty())
            .map(|r| print::offset(&r, cuts.bleed_mm))
    } else {
        None
    };
    let holes = print::offset(&rings(&|k| cuts.blanks(k)), -cuts.bleed_mm);
    (outline, holes)
}

/// Coverage of what gets printed (255) and what the template cuts away (0), for a
/// `size_px` raster whose top-left corner sits at `origin_mm`. `None` when nothing is cut.
/// The outline only clips if it has closed paths, so a broken outline cuts nothing.
pub fn keep_mask(
    elements: &[PlacedElement],
    cuts: Cuts,
    origin_mm: P2,
    size_px: [u32; 2],
    px_per_mm: f32,
) -> Option<Vec<u8>> {
    let to_px =
        Affine::translate(-origin_mm[0], -origin_mm[1]).then(Affine::scale(px_per_mm, px_per_mm));
    let (outline, holes) = cut_rings(elements, cuts);
    if outline.is_none() && holes.is_empty() {
        return None;
    }
    let [w, h] = size_px;
    // Non-zero winding: shapes drawn twice or overlapping still count once.
    let fill = |rings: &[Vec<P2>]| {
        let mut m = Mask::new(w, h)?;
        if let Some(path) = closed_path(rings, to_px) {
            m.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }
        Some(m.take())
    };
    let mut keep = match &outline {
        Some(rings) => fill(rings)?,
        None => vec![255; w as usize * h as usize],
    };
    if !holes.is_empty() {
        for (k, hole) in keep.iter_mut().zip(fill(&holes)?) {
            *k = ((*k as u32 * (255 - hole as u32) + 127) / 255) as u8;
        }
    }
    Some(keep)
}

/// Turns what the template cuts away into blank paper (white).
fn apply_cuts(pixmap: &mut Pixmap, keep: &[u8]) {
    for (px, &k) in pixmap
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(keep)
    {
        if k == 255 {
            continue;
        }
        let k = k as u32;
        for c in px.iter_mut() {
            *c = ((*c as u32 * k + 255 * (255 - k) + 127) / 255) as u8;
        }
    }
}

/// Size in pixels of the page rendered at `dpi`.
pub fn pixel_size(page_mm: P2, dpi: f32) -> [u32; 2] {
    [
        (page_mm[0] / 25.4 * dpi).round() as u32,
        (page_mm[1] / 25.4 * dpi).round() as u32,
    ]
}

/// Converts to tiny-skia's transform type.
fn to_skia(m: Affine) -> Transform {
    Transform::from_row(m.a, m.b, m.c, m.d, m.e, m.f)
}

/// Premultiplies straight-alpha RGBA pixels.
pub fn premultiplied_pixmap(img: &image::RgbaImage) -> Option<Pixmap> {
    let mut data = img.as_raw().clone();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u16;
        if a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
    }
    Pixmap::from_vec(data, IntSize::from_wh(img.width(), img.height())?)
}

/// Shrinks an image so its longest side is at most `max_side`; borrows it if it's
/// already small enough.
pub fn downscale(img: &image::RgbaImage, max_side: u32) -> std::borrow::Cow<'_, image::RgbaImage> {
    let (w, h) = img.dimensions();
    if w.max(h) <= max_side {
        return std::borrow::Cow::Borrowed(img);
    }
    let k = max_side as f32 / w.max(h) as f32;
    let (dw, dh) = (
        ((w as f32 * k).round() as u32).max(1),
        ((h as f32 * k).round() as u32).max(1),
    );
    std::borrow::Cow::Owned(image::imageops::resize(
        img,
        dw,
        dh,
        image::imageops::FilterType::Triangle,
    ))
}

/// Axis-aligned bounds (min, max) of the drawn nodes, after mapping by `m`.
pub fn bounds(nodes: &[SceneNode], m: Affine) -> Option<(P2, P2)> {
    let mut acc: Option<(P2, P2)> = None;
    let mut add = |p: P2| {
        acc = Some(match acc {
            None => (p, p),
            Some((lo, hi)) => (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            ),
        });
    };
    for n in nodes {
        match n {
            SceneNode::Image(l) => {
                let (w, h) = (l.pixmap.width() as f32, l.pixmap.height() as f32);
                let t = l.map.then_affine(m);
                for c in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]] {
                    add(t.apply(c));
                }
            }
            SceneNode::Group { children, .. } => {
                if let Some((lo, hi)) = bounds(children, m) {
                    add(lo);
                    add(hi);
                }
            }
            // Nothing clipped shows outside the base.
            SceneNode::Clip { base, .. } => {
                if let Some((lo, hi)) = bounds(std::slice::from_ref(base), m) {
                    add(lo);
                    add(hi);
                }
            }
        }
    }
    acc
}

/// Halves a pixmap by averaging 2 x 2 blocks (premultiplied, so this is exact).
fn halve(src: &Pixmap) -> Option<Pixmap> {
    let (w, h) = ((src.width() / 2).max(1), (src.height() / 2).max(1));
    let mut out = Pixmap::new(w, h)?;
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let s = src.data();
    let d = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let xs = [(2 * x).min(sw - 1), (2 * x + 1).min(sw - 1)];
            let ys = [(2 * y).min(sh - 1), (2 * y + 1).min(sh - 1)];
            for c in 0..4 {
                let sum: u32 = ys
                    .iter()
                    .flat_map(|&yy| xs.iter().map(move |&xx| s[(yy * sw + xx) * 4 + c] as u32))
                    .sum();
                d[(y * w as usize + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    Some(out)
}

/// Area of a polygon (shoelace formula).
fn area(pts: &[P2]) -> f32 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        .abs()
        / 2.0
}

/// Draws a pixmap through a perspective map (pixmap pixels to target pixels), with
/// bilinear filtering. Large reductions first shrink the source to avoid aliasing.
fn draw_warped(
    target: &mut PixmapMut,
    src: &Pixmap,
    map: Homography,
    opacity: f32,
) -> Result<(), String> {
    let (w, h) = (src.width() as f32, src.height() as f32);
    let quad = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|p| map.apply(p));
    if quad.iter().flatten().any(|v| !v.is_finite()) {
        return Ok(());
    }
    // Shrink by powers of two while the source has over 4x the pixels it covers.
    let mut level: Option<Pixmap> = None;
    let mut k = 1.0f32;
    let covered = area(&quad).max(1.0);
    loop {
        let cur = level.as_ref().unwrap_or(src);
        if (cur.width() as f32 * cur.height() as f32) < covered * 4.0
            || cur.width() < 2
            || cur.height() < 2
        {
            break;
        }
        let next = halve(cur).ok_or(tr!("CANNOT_ALLOCATE_REDUCED_IMAGE"))?;
        k *= 2.0;
        level = Some(next);
    }
    let src = level.as_ref().unwrap_or(src);
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    let inv = map
        .inverse()
        .ok_or(tr!("DEGENERATE_PERSPECTIVE"))?
        .then_affine(Affine::scale(1.0 / k, 1.0 / k));

    let (tw, th) = (target.width() as i64, target.height() as i64);
    let lo = quad
        .iter()
        .fold([f32::MAX; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
    let hi = quad
        .iter()
        .fold([f32::MIN; 2], |a, p| [a[0].max(p[0]), a[1].max(p[1])]);
    let x0 = (lo[0].floor() as i64 - 1).clamp(0, tw);
    let x1 = (hi[0].ceil() as i64 + 1).clamp(0, tw);
    let y0 = (lo[1].floor() as i64 - 1).clamp(0, th);
    let y1 = (hi[1].ceil() as i64 + 1).clamp(0, th);

    let data = src.data();
    // Clamped to the edge: the image border is antialiased from the geometry instead,
    // so enlarged images keep crisp edges.
    let texel = |x: i64, y: i64| -> [f32; 4] {
        let (x, y) = (x.clamp(0, sw - 1), y.clamp(0, sh - 1));
        let i = ((y * sw + x) * 4) as usize;
        [0, 1, 2, 3].map(|c| data[i + c] as f32)
    };
    let (fw, fh) = (sw as f32, sh as f32);
    let op = opacity.clamp(0.0, 1.0);
    let m = inv.m;
    let tw = tw as usize;
    let out = target.data_mut();
    for y in y0..y1 {
        let py = y as f64 + 0.5;
        for x in x0..x1 {
            let px = x as f64 + 0.5;
            let wq = m[6] * px + m[7] * py + m[8];
            if wq.abs() < 1e-12 {
                continue;
            }
            let (uq, vq) = (m[0] * px + m[1] * py + m[2], m[3] * px + m[4] * py + m[5]);
            let (u, v) = ((uq / wq) as f32, (vq / wq) as f32);
            // Coverage: distance to the image border in target pixels, from the
            // derivatives of the map.
            let w2 = wq * wq;
            let grad = |a: f64, b: f64, q: f64| {
                let dx = (a * wq - q * m[6]) / w2;
                let dy = (b * wq - q * m[7]) / w2;
                (dx * dx + dy * dy).sqrt().max(1e-9) as f32
            };
            let (gu, gv) = (grad(m[0], m[1], uq), grad(m[3], m[4], vq));
            let cu = (u.min(fw - u) / gu + 0.5).clamp(0.0, 1.0);
            let cv = (v.min(fh - v) / gv + 0.5).clamp(0.0, 1.0);
            let cover = cu * cv;
            if cover <= 0.0 {
                continue;
            }
            // Bilinear sample, relative to texel centers.
            let (u, v) = (u - 0.5, v - 0.5);
            let (iu, iv) = (u.floor() as i64, v.floor() as i64);
            let (fu, fv) = (u - iu as f32, v - iv as f32);
            let (a, b) = (texel(iu, iv), texel(iu + 1, iv));
            let (c, d) = (texel(iu, iv + 1), texel(iu + 1, iv + 1));
            let s: [f32; 4] = std::array::from_fn(|i| {
                let top = a[i] + (b[i] - a[i]) * fu;
                let bot = c[i] + (d[i] - c[i]) * fu;
                (top + (bot - top) * fv) * op * cover
            });
            if s[3] <= 0.0 {
                continue;
            }
            let o = (y as usize * tw + x as usize) * 4;
            let keep = 1.0 - s[3] / 255.0;
            for i in 0..4 {
                out[o + i] = (s[i] + out[o + i] as f32 * keep).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok(())
}

/// Draws nodes onto `target`; `to_px` maps page mm to target pixels.
/// Draws an image layer, applying its effects.
fn draw_layer(target: &mut PixmapMut, l: &SceneLayer, to_px: Affine) -> Result<(), String> {
    let processed;
    let (pixmap, map) = if l.effects.is_empty() {
        (&l.pixmap, l.map)
    } else {
        let (pm, pad) = effects::apply_pixmap(&l.pixmap, &l.effects, l.effects_scale)
            .ok_or(tr!("CANNOT_APPLY_EFFECTS"))?;
        processed = Arc::new(pm);
        // The margins the effects added lie left of and above the image.
        let shift = Affine::translate(-(pad[0] as f32), -(pad[1] as f32));
        (&processed, Homography::from(shift).then(l.map))
    };
    let map = map.then_affine(to_px);
    match map.as_affine() {
        Some(affine) => {
            let paint = PixmapPaint {
                opacity: l.opacity.clamp(0.0, 1.0),
                blend_mode: l.blend.skia(),
                quality: FilterQuality::Bicubic,
            };
            target.draw_pixmap(
                0,
                0,
                pixmap.as_ref().as_ref(),
                &paint,
                to_skia(affine),
                None,
            );
        }
        None if l.blend.is_normal() => draw_warped(target, pixmap, map, l.opacity)?,
        // Warped on its own, then mixed.
        None => {
            let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
            let quad = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|p| map.apply(p));
            let lo = quad
                .iter()
                .fold([f32::MAX; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
            let hi = quad
                .iter()
                .fold([f32::MIN; 2], |a, p| [a[0].max(p[0]), a[1].max(p[1])]);
            if let Some((x0, y0, mut buf, shift)) = region(target, lo, hi)? {
                draw_warped(&mut buf.as_mut(), pixmap, map.then_affine(shift), l.opacity)?;
                blit(target, &buf, x0, y0, 1.0, l.blend);
            }
        }
    }
    Ok(())
}

fn draw_nodes(target: &mut PixmapMut, nodes: &[SceneNode], to_px: Affine) -> Result<(), String> {
    for node in nodes {
        match node {
            SceneNode::Image(l) => draw_layer(target, l, to_px)?,
            SceneNode::Group {
                opacity,
                blend,
                children,
            } => {
                let Some((x0, y0, mut buf, to_buf)) = offscreen(target, children, to_px)? else {
                    continue;
                };
                draw_nodes(&mut buf.as_mut(), children, to_buf)?;
                blit(target, &buf, x0, y0, *opacity, *blend);
            }
            SceneNode::Clip { base, clipped } => {
                let base = std::slice::from_ref(&**base);
                draw_nodes(target, base, to_px)?;
                let Some((x0, y0, mut mask, to_buf)) = offscreen(target, base, to_px)? else {
                    continue;
                };
                draw_nodes(&mut mask.as_mut(), base, to_buf)?;
                // What is clipped is drawn over a copy of what lies below, so that it
                // mixes with it, then kept as much as the base shows.
                let backdrop = copy_region(target, x0, y0, mask.width(), mask.height())?;
                let mut buf = backdrop.clone();
                draw_nodes(&mut buf.as_mut(), clipped, to_buf)?;
                keep_where(target, x0 as u32, y0 as u32, &backdrop, &buf, &mask);
            }
        }
    }
    Ok(())
}

/// A transparent buffer covering the footprint of `nodes` on `target`, to draw them on
/// their own: its top-left corner on the target, the buffer, and page mm to its pixels.
/// `None` when they don't reach the target.
#[allow(clippy::type_complexity)]
fn offscreen(
    target: &PixmapMut,
    nodes: &[SceneNode],
    to_px: Affine,
) -> Result<Option<(i32, i32, Pixmap, Affine)>, String> {
    let Some((lo, hi)) = bounds(nodes, to_px) else {
        return Ok(None);
    };
    Ok(region(target, lo, hi)?.map(|(x0, y0, buf, shift)| (x0, y0, buf, to_px.then(shift))))
}

/// A transparent buffer covering the box from `lo` to `hi` (target pixels) on
/// `target`: its top-left corner on the target, the buffer, and target pixels to its
/// own. `None` when the box is off the target.
#[allow(clippy::type_complexity)]
fn region(
    target: &PixmapMut,
    lo: P2,
    hi: P2,
) -> Result<Option<(i32, i32, Pixmap, Affine)>, String> {
    let x0 = (lo[0].floor() as i64 - 1).max(0);
    let y0 = (lo[1].floor() as i64 - 1).max(0);
    let x1 = (hi[0].ceil() as i64 + 1).min(target.width() as i64);
    let y1 = (hi[1].ceil() as i64 + 1).min(target.height() as i64);
    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let buf = Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_GROUP_BUFFER", w = w, h = h))?;
    let shift = Affine::translate(-x0 as f32, -y0 as f32);
    Ok(Some((x0 as i32, y0 as i32, buf, shift)))
}

/// Draws a buffer from `offscreen` onto the target at (`x0`, `y0`) with `opacity`,
/// mixed in `blend` mode.
fn blit(target: &mut PixmapMut, buf: &Pixmap, x0: i32, y0: i32, opacity: f32, blend: BlendMode) {
    let paint = PixmapPaint {
        opacity: opacity.clamp(0.0, 1.0),
        blend_mode: blend.skia(),
        quality: FilterQuality::Nearest,
    };
    target.draw_pixmap(x0, y0, buf.as_ref(), &paint, Transform::identity(), None);
}

/// A copy of the `w` x `h` pixels of `target` from (`x0`, `y0`).
fn copy_region(target: &PixmapMut, x0: i32, y0: i32, w: u32, h: u32) -> Result<Pixmap, String> {
    let rect = tiny_skia::IntRect::from_xywh(x0, y0, w, h).ok_or("empty region")?;
    target
        .as_ref()
        .clone_rect(rect)
        .ok_or_else(|| tr!("CANNOT_ALLOCATE_GROUP_BUFFER", w = w, h = h))
}

/// Over the area of `target` from (`x0`, `y0`) the size of the buffers: `drawn` where
/// `mask` is opaque, `backdrop` where it is transparent, in between as much as it shows.
fn keep_where(
    target: &mut PixmapMut,
    x0: u32,
    y0: u32,
    backdrop: &Pixmap,
    drawn: &Pixmap,
    mask: &Pixmap,
) {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let tw = target.width() as usize;
    let (b, d, m) = (backdrop.data(), drawn.data(), mask.data());
    let out = target.data_mut();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let a = m[i + 3] as u32;
            let o = ((y0 as usize + y) * tw + x0 as usize + x) * 4;
            for c in 0..4 {
                let (bc, dc) = (b[i + c] as u32, d[i + c] as u32);
                out[o + c] = ((bc * (255 - a) + dc * a + 127) / 255) as u8;
            }
        }
    }
}

/// Renders nodes onto a transparent pixmap of `size_px`, whose top-left corner sits at
/// `origin_mm` on the page, at `px_per_mm`.
pub fn compose_region(
    nodes: &[SceneNode],
    origin_mm: P2,
    size_px: [u32; 2],
    px_per_mm: f32,
) -> Result<Pixmap, String> {
    let [w, h] = size_px;
    let mut pixmap = Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h))?;
    let to_px =
        Affine::translate(-origin_mm[0], -origin_mm[1]).then(Affine::scale(px_per_mm, px_per_mm));
    draw_nodes(&mut pixmap.as_mut(), nodes, to_px)?;
    Ok(pixmap)
}

/// As `compose_region`, over the page's background (`color`, or `gradient` when set),
/// which covers the page from (0, 0) to `page_mm`.
#[allow(clippy::too_many_arguments)]
pub fn compose_region_on_page(
    nodes: &[SceneNode],
    origin_mm: P2,
    size_px: [u32; 2],
    px_per_mm: f32,
    page_mm: P2,
    color: [u8; 3],
    gradient: Option<&crate::gradient::Gradient>,
    pattern: Option<&crate::pattern::Pattern>,
) -> Result<Pixmap, String> {
    let [w, h] = size_px;
    let mut pixmap = Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h))?;
    let [pw, ph] = [
        (page_mm[0] * px_per_mm).ceil().max(1.0) as u32,
        (page_mm[1] * px_per_mm).ceil().max(1.0) as u32,
    ];
    let mut page =
        Pixmap::new(pw, ph).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = pw, h = ph))?;
    fill_background(&mut page, color, gradient, pattern, px_per_mm);
    blit(
        &mut pixmap.as_mut(),
        &page,
        (-origin_mm[0] * px_per_mm).round() as i32,
        (-origin_mm[1] * px_per_mm).round() as i32,
        1.0,
        BlendMode::Normal,
    );
    let to_px =
        Affine::translate(-origin_mm[0], -origin_mm[1]).then(Affine::scale(px_per_mm, px_per_mm));
    draw_nodes(&mut pixmap.as_mut(), nodes, to_px)?;
    Ok(pixmap)
}

/// Error returned when an export is canceled.
pub const CANCELED: &str = "Export canceled";

/// Renders the page at `dpi`, cut by the template. With `lines`, also draws the lines of
/// the elements marked for printing. The background is opaque, so every output pixel is.
pub fn compose(scene: &Scene, dpi: f32, lines: bool) -> Result<Pixmap, String> {
    let mut pixmap = compose_artwork(scene, dpi, &mut |_| true)?;
    if lines {
        draw_lines(&mut pixmap, scene, dpi);
    }
    Ok(pixmap)
}

/// Renders the background and layers at `dpi`, cut by the template, without lines.
/// `progress` gets the fraction done after each top-level layer or group; returning
/// false cancels with the `CANCELED` error.
pub fn compose_artwork(
    scene: &Scene,
    dpi: f32,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Pixmap, String> {
    let parts = compose_parts(scene, dpi, false, progress)?;
    match parts.into_iter().next() {
        Some(Part::Raster { pixmap, .. }) => Ok(pixmap),
        _ => unreachable!("the page comes first"),
    }
}

/// A piece of the exported page, for formats that keep text and shapes as vectors.
pub enum Part<'a> {
    /// Layers drawn together at the export resolution. The first part is the whole page,
    /// with its background and cut by the template. The others are transparent and
    /// cropped to what they cover, their top-left corner at `origin` pixels.
    Raster {
        pixmap: Pixmap,
        origin: [u32; 2],
        /// How it mixes with the parts below: not normal for a layer, or a group as a
        /// whole, in another mode, drawn on its own.
        blend: BlendMode,
    },
    /// A text or shape layer to draw as vectors: `SceneLayer::vector` gives its shapes.
    Vector(&'a SceneLayer),
    /// The parts up to the matching `ClipEnd` are what is clipped to this text or shape
    /// layer, drawn just before as a `Vector`: they are cut by its outline
    /// (`SceneLayer::clip_outline`).
    ClipBegin(&'a SceneLayer),
    ClipEnd,
}

/// Renders the page as parts to stack, bottom first. With `vectors`, the top-level text
/// and shape layers that can be drawn as vectors become parts of their own, and the layers between
/// them are rendered together; so do the layers clipped to such a layer, between a
/// `ClipBegin` and a `ClipEnd`, when it can clip them as vectors. Without, there is a single part: the page as `compose`
/// renders it, without lines. `progress` is as for `compose_artwork`.
pub fn compose_parts<'a>(
    scene: &'a Scene,
    dpi: f32,
    vectors: bool,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Vec<Part<'a>>, String> {
    let [w, h] = pixel_size(scene.page_mm, dpi);
    let new_pixmap = || Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h));
    let mut page = new_pixmap()?;
    fill_background(
        &mut page,
        scene.background,
        scene.background_gradient.as_ref(),
        scene.background_pattern.as_ref(),
        dpi / 25.4,
    );

    let mut b = PartsBuilder {
        parts: Vec::new(),
        current: Some(page),
        size: [w, h],
        mm_to_px: Affine::scale(dpi / 25.4, dpi / 25.4),
        vectors,
    };
    // Up to the last layer that must mix with what lies below as pixels, everything is
    // drawn into the page.
    let flat_until = scene.layers.iter().rposition(needs_backdrop);
    // One step per top-level node, plus one for the cuts.
    let steps = scene.layers.len() as f32 + 1.0;
    for (i, node) in scene.layers.iter().enumerate() {
        if !progress(i as f32 / steps) {
            return Err(CANCELED.into());
        }
        b.vectors = vectors && flat_until.is_none_or(|k| i > k);
        b.add(node)?;
    }
    b.flush();
    let mut parts = b.parts;
    if !progress((steps - 1.0) / steps) {
        return Err(CANCELED.into());
    }
    if let Some(Part::Raster { pixmap, .. }) = parts.first_mut()
        && let Some(keep) = keep_mask(&scene.placed(), scene.cuts, [0.0, 0.0], [w, h], dpi / 25.4)
    {
        apply_cuts(pixmap, &keep);
    }
    progress(1.0);
    Ok(parts)
}

/// Something in the node mixes with what lies below in another mode than normal, and
/// only as pixels drawn over it: in a stack clipped as pixels. Layers and groups in
/// such a mode are parts of their own otherwise, which the formats mix themselves.
fn needs_backdrop(node: &SceneNode) -> bool {
    match node {
        SceneNode::Clip { base, clipped } => match &**base {
            SceneNode::Image(l) if l.clip_outline().is_some() => clipped.iter().any(needs_backdrop),
            _ => node.has_blend(),
        },
        _ => false,
    }
}

/// Gathers the parts of `compose_parts`.
struct PartsBuilder<'a> {
    parts: Vec<Part<'a>>,
    /// Where the layers since the last vector part are drawn.
    current: Option<Pixmap>,
    size: [u32; 2],
    mm_to_px: Affine,
    vectors: bool,
}

impl<'a> PartsBuilder<'a> {
    fn add(&mut self, node: &'a SceneNode) -> Result<(), String> {
        if self.vectors {
            match node {
                SceneNode::Image(l) if l.vector().is_some() => {
                    self.flush();
                    self.parts.push(Part::Vector(l));
                    return Ok(());
                }
                // Drawn on its own in normal mode; the part mixes in its mode.
                SceneNode::Image(l) if !l.blend.is_normal() => {
                    let normal = SceneNode::Image(SceneLayer {
                        blend: BlendMode::Normal,
                        ..l.clone()
                    });
                    return self.alone(&normal, l.blend);
                }
                SceneNode::Group {
                    opacity,
                    blend,
                    children,
                } if !blend.is_normal() => {
                    self.flush();
                    let mut buf = self.new_pixmap()?;
                    draw_nodes(&mut buf.as_mut(), children, self.mm_to_px)?;
                    let mut part = self.new_pixmap()?;
                    blit(&mut part.as_mut(), &buf, 0, 0, *opacity, BlendMode::Normal);
                    self.push_raster(part, *blend);
                    return Ok(());
                }
                SceneNode::Clip { base, clipped } => {
                    if let SceneNode::Image(l) = &**base
                        && l.clip_outline().is_some()
                    {
                        self.flush();
                        self.parts.push(Part::Vector(l));
                        self.parts.push(Part::ClipBegin(l));
                        for n in clipped {
                            self.add(n)?;
                        }
                        self.flush();
                        self.parts.push(Part::ClipEnd);
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        if self.current.is_none() {
            self.current = Some(self.new_pixmap()?);
        }
        let target = self.current.as_mut().expect("just made");
        draw_nodes(
            &mut target.as_mut(),
            std::slice::from_ref(node),
            self.mm_to_px,
        )
    }

    fn new_pixmap(&self) -> Result<Pixmap, String> {
        let [w, h] = self.size;
        Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h))
    }

    /// `node` drawn alone into a part of its own, mixed in `blend` mode.
    fn alone(&mut self, node: &SceneNode, blend: BlendMode) -> Result<(), String> {
        self.flush();
        let mut part = self.new_pixmap()?;
        draw_nodes(
            &mut part.as_mut(),
            std::slice::from_ref(node),
            self.mm_to_px,
        )?;
        self.push_raster(part, blend);
        Ok(())
    }

    /// Ends the raster part being drawn, if any.
    fn flush(&mut self) {
        if let Some(pixmap) = self.current.take() {
            self.push_raster(pixmap, BlendMode::Normal);
        }
    }

    /// A raster part: the page when it is the first part, otherwise cropped to what it
    /// shows (and left out when it shows nothing).
    fn push_raster(&mut self, pixmap: Pixmap, blend: BlendMode) {
        if self.parts.is_empty() {
            self.parts.push(Part::Raster {
                pixmap,
                origin: [0, 0],
                blend,
            });
        } else if let Some((pixmap, origin)) = crop_to_content(&pixmap) {
            self.parts.push(Part::Raster {
                pixmap,
                origin,
                blend,
            });
        }
    }
}

/// The smallest part of a transparent pixmap holding all it shows, and where it starts;
/// `None` if it shows nothing.
fn crop_to_content(pixmap: &Pixmap) -> Option<(Pixmap, [u32; 2])> {
    let w = pixmap.width() as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (y, row) in pixmap.data().chunks_exact(w * 4).enumerate() {
        let mut row_has = false;
        for (x, px) in row.as_chunks::<4>().0.iter().enumerate() {
            if px[3] != 0 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                row_has = true;
            }
        }
        if row_has {
            y0 = y0.min(y);
            y1 = y + 1;
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let rect =
        tiny_skia::IntRect::from_xywh(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32)?;
    Some((pixmap.clone_rect(rect)?, [x0 as u32, y0 as u32]))
}

/// Stacks the parts into one picture of the page, cut by the template, without lines:
/// what `compose_artwork` renders.
pub fn flatten(parts: &[Part], scene: &Scene, dpi: f32) -> Result<Pixmap, String> {
    let mut it = parts.iter();
    let Some(Part::Raster { pixmap: page, .. }) = it.next() else {
        unreachable!("the page comes first");
    };
    let mut out = page.clone();
    if parts.len() == 1 {
        return Ok(out);
    }
    let mm_to_px = Affine::scale(dpi / 25.4, dpi / 25.4);
    let [w, h] = [out.width(), out.height()];
    // What is clipped is drawn over the page as it was at its start, then kept where
    // the layer it is clipped to shows.
    let mut clips: Vec<(Pixmap, &SceneLayer)> = Vec::new();
    for part in it {
        match part {
            Part::Raster {
                pixmap,
                origin,
                blend,
            } => blit(
                &mut out.as_mut(),
                pixmap,
                origin[0] as i32,
                origin[1] as i32,
                1.0,
                *blend,
            ),
            Part::Vector(l) => draw_layer(&mut out.as_mut(), l, mm_to_px)?,
            Part::ClipBegin(l) => clips.push((out.clone(), l)),
            Part::ClipEnd => {
                let Some((backdrop, l)) = clips.pop() else {
                    continue;
                };
                let mut mask =
                    Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h))?;
                let base = SceneLayer {
                    blend: BlendMode::Normal,
                    ..l.clone()
                };
                draw_layer(&mut mask.as_mut(), &base, mm_to_px)?;
                let drawn = out.clone();
                keep_where(&mut out.as_mut(), 0, 0, &backdrop, &drawn, &mask);
            }
        }
    }
    let [w, h] = [out.width(), out.height()];
    if let Some(keep) = keep_mask(&scene.placed(), scene.cuts, [0.0, 0.0], [w, h], dpi / 25.4) {
        apply_cuts(&mut out, &keep);
    }
    Ok(out)
}

/// What the template cuts away, as for `cut_rings`, with no outline when nothing clips.
pub fn cut_shapes(scene: &Scene) -> (Vec<Vec<P2>>, Vec<Vec<P2>>) {
    let (outline, holes) = cut_rings(&scene.placed(), scene.cuts);
    (outline.unwrap_or_default(), holes)
}

/// Draws the lines of the template elements marked for printing, then the crop and
/// registration marks, all at least 1 px wide.
pub fn draw_lines(pixmap: &mut Pixmap, scene: &Scene, dpi: f32) {
    draw_lines_at(pixmap, scene, dpi, [0.0, 0.0], None);
}

/// Like `draw_lines`, with page point (x, y) drawn at (x + shift x, y + shift y) mm, and
/// only where `clip` covers, if given.
pub fn draw_lines_at(
    pixmap: &mut Pixmap,
    scene: &Scene,
    dpi: f32,
    shift_mm: P2,
    clip: Option<&Mask>,
) {
    for element in scene.elements.iter().filter(|e| e.print) {
        let lines: Vec<_> = element.placed.printed_lines().collect();
        stroke_polylines(
            pixmap,
            lines.iter().map(|l| l.as_ref()),
            (element.color, OUTLINE_WIDTH_MM),
            dpi,
            shift_mm,
            clip,
        );
    }
    stroke_polylines(
        pixmap,
        scene.marks.iter(),
        (print::MARK_COLOR, print::MARK_WIDTH_MM),
        dpi,
        shift_mm,
        clip,
    );
}

/// Strokes lines in mm of `(color, width in mm)`, at least 1 px wide, onto a pixmap at
/// `dpi`, shifted by `shift_mm`, and only where `clip` covers, if given.
pub fn stroke_polylines<'a>(
    pixmap: &mut Pixmap,
    polylines: impl Iterator<Item = &'a Polyline>,
    (color, width_mm): ([u8; 3], f32),
    dpi: f32,
    shift_mm: P2,
    clip: Option<&Mask>,
) {
    let k = dpi / 25.4;
    let stroke = Stroke {
        width: (width_mm * k).max(1.0),
        ..Default::default()
    };
    let mut paint = Paint::default();
    let [r, g, b] = color;
    paint.set_color_rgba8(r, g, b, 255);
    paint.anti_alias = true;
    let at = |p: &P2| ((p[0] + shift_mm[0]) * k, (p[1] + shift_mm[1]) * k);
    for poly in polylines {
        let mut pb = PathBuilder::new();
        let mut pts = poly.points.iter().map(at);
        let Some((x, y)) = pts.next() else { continue };
        pb.move_to(x, y);
        for (x, y) in pts {
            pb.line_to(x, y);
        }
        if poly.closed {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), clip);
        }
    }
}

/// Fills a page-sized pixmap with the background: its color, then its pattern (in mm,
/// at `px_per_mm` pixels per mm) over it, or else its gradient over the whole pixmap,
/// kept opaque.
pub fn fill_background(
    page: &mut Pixmap,
    color: [u8; 3],
    gradient: Option<&crate::gradient::Gradient>,
    pattern: Option<&crate::pattern::Pattern>,
    px_per_mm: f32,
) {
    let size = [page.width() as f32, page.height() as f32];
    fill_background_at(page, color, gradient, pattern, px_per_mm, [0.0, 0.0], size);
}

/// As `fill_background`, for a pixmap where the page's top-left corner is at `at`
/// pixels and the page `page_px` pixels in size (its gradient spans that).
fn fill_background_at(
    pixmap: &mut Pixmap,
    color: [u8; 3],
    gradient: Option<&crate::gradient::Gradient>,
    pattern: Option<&crate::pattern::Pattern>,
    px_per_mm: f32,
    at: [f32; 2],
    page_px: [f32; 2],
) {
    let [r, g, b] = color;
    pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));
    let full = tiny_skia::Rect::from_xywh(0.0, 0.0, pixmap.width() as f32, pixmap.height() as f32);
    let to_px = Transform::from_scale(px_per_mm, px_per_mm).post_translate(at[0], at[1]);
    if let Some(ink) = pattern
        .and_then(|p| p.ink(tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0)?))
        .and_then(|ink| ink.transformed(to_px))
        && let Some(full) = full
    {
        pixmap.fill_rect(full, &ink.paint(), Transform::identity(), None);
        return;
    }
    let (Some(gradient), Some(full)) = (gradient, full) else {
        return;
    };
    let Some(frame) = tiny_skia::Rect::from_xywh(at[0], at[1], page_px[0], page_px[1]) else {
        return;
    };
    let mut opaque = gradient.clone();
    for s in &mut opaque.stops {
        s.color[3] = 255;
    }
    let ink = crate::gradient::Ink::Gradient {
        gradient: opaque,
        frame,
    };
    pixmap.fill_rect(full, &ink.paint(), tiny_skia::Transform::identity(), None);
}

/// The color of the artwork at `p` on the page, in mm, over the page's background
/// (`color`, `gradient` or `pattern`): one pixel rendered at `px_per_mm`, straight
/// RGBA. `None` where nothing is drawn.
#[allow(clippy::too_many_arguments)]
pub fn sample(
    nodes: &[SceneNode],
    p: P2,
    px_per_mm: f32,
    page_mm: P2,
    color: [u8; 3],
    gradient: Option<&crate::gradient::Gradient>,
    pattern: Option<&crate::pattern::Pattern>,
) -> Option<[u8; 4]> {
    let mut pixmap = Pixmap::new(1, 1)?;
    // The pixel is centered on `p`.
    let origin = [p[0] - 0.5 / px_per_mm, p[1] - 0.5 / px_per_mm];
    let on_page = (0.0..page_mm[0]).contains(&p[0]) && (0.0..page_mm[1]).contains(&p[1]);
    if on_page {
        let at = [-origin[0] * px_per_mm, -origin[1] * px_per_mm];
        let page_px = [page_mm[0] * px_per_mm, page_mm[1] * px_per_mm];
        fill_background_at(
            &mut pixmap,
            color,
            gradient,
            pattern,
            px_per_mm,
            at,
            page_px,
        );
    }
    let to_px = Affine::translate(-origin[0], -origin[1]).then(Affine::scale(px_per_mm, px_per_mm));
    draw_nodes(&mut pixmap.as_mut(), nodes, to_px).ok()?;
    let c = pixmap.pixel(0, 0)?.demultiply();
    (c.alpha() > 0).then(|| [c.red(), c.green(), c.blue(), c.alpha()])
}

/// Opaque RGB bytes from a pixmap with an opaque background.
pub fn to_rgb(pixmap: &Pixmap) -> Vec<u8> {
    pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A page with a gradient background, a shape filled and stroked with gradients and
    /// a text in a gradient, rendered as an export does, to look at it:
    /// `GRADIENT_PAGE_PREVIEW=out.png cargo test gradient_page_preview -- --ignored`.
    #[test]
    #[ignore]
    fn gradient_page_preview() {
        use crate::document::{Document, Node};
        use crate::gradient::{Gradient, GradientKind, Stop};
        let Ok(path) = std::env::var("GRADIENT_PAGE_PREVIEW") else {
            return;
        };
        let mut doc = Document::default();
        let mut bg = Gradient::from_color([30, 40, 90, 255]);
        bg.angle = 90.0;
        bg.stops[1].color = [180, 210, 255, 255];
        doc.background_gradient = Some(bg);
        let mut star = crate::shape::new_layer(1, crate::shape::ShapeKind::Star, [100.0, 105.0]);
        star.transform.rotation_deg = 15.0;
        {
            let s = star.shape.as_mut().unwrap();
            s.size = [120.0, 120.0];
            let mut fill = Gradient::from_color([255, 220, 60, 255]);
            fill.kind = GradientKind::Radial;
            fill.stops[1].color = [230, 60, 30, 255];
            s.fill_gradient = Some(fill);
            s.stroke_width = 3.0;
            let mut stroke = Gradient::from_color([255, 255, 255, 255]);
            stroke.stops[1].color = [20, 20, 20, 255];
            stroke.angle = -45.0;
            s.stroke_gradient = Some(stroke);
        }
        let mut text = crate::text::new_layer(
            2,
            "Gradient",
            crate::text::BUILTIN_FAMILY.into(),
            [215.0, 105.0],
        );
        {
            let t = text.text.as_mut().unwrap();
            let mut g = Gradient::from_color([60, 230, 120, 255]);
            g.stops.push(Stop::new(0.5, [40, 120, 255, 255]));
            g.stops[1].color = [200, 60, 220, 255];
            t.gradient = Some(g);
        }
        let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        text.size_px = crate::text::layout(text.text.as_ref().unwrap(), &font).size;
        text.transform.scale = text.transform.scale.map(|s| s * 5.0);
        doc.layers = vec![Node::Layer(Box::new(star)), Node::Layer(Box::new(text))];
        crate::shape::prepare_export(&mut doc, 60.0);
        crate::text::prepare_export(&mut doc, 60.0);
        let scene = Scene::new(
            &doc,
            None,
            &[[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        compose(&scene, 60.0, false)
            .unwrap()
            .save_png(path)
            .unwrap();
    }
    use crate::document::{Group, Layer, Orientation, PageSize};

    /// A 10 x 10 mm square layer of `rgb`, centered at `center`, at 1 px per mm.
    fn square(id: u64, rgb: [u8; 3], center: P2) -> Node {
        let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [10, 10];
        l.transform.center = center;
        l.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
        Node::Layer(Box::new(l))
    }

    fn group(id: u64, opacity: f32, children: Vec<Node>) -> Node {
        Node::Group(Group {
            id,
            name: "g".into(),
            visible: true,
            locked: false,
            opacity,
            clip: false,
            blend: crate::document::BlendMode::Normal,
            expanded: true,
            children,
            element: None,
        })
    }

    /// A text layer saying "Hi" in red, laid out and drawn for an export at `dpi`.
    fn text_layer(id: u64, center: P2) -> Node {
        let mut l = text::new_layer(id, "Hi", text::BUILTIN_FAMILY.into(), center);
        let t = l.text.as_mut().unwrap();
        t.color = [255, 0, 0, 255];
        let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        l.size_px = text::layout(l.text.as_ref().unwrap(), &font).size;
        // Huge, so it shows at a low test DPI.
        l.transform.scale = l.transform.scale.map(|s| s * 6.0);
        Node::Layer(Box::new(l))
    }

    fn scene_of(mut doc: Document, dpi: f32) -> Scene {
        text::prepare_export(&mut doc, dpi);
        Scene::new(
            &doc,
            None,
            &[[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        )
    }

    #[test]
    fn text_layers_become_vector_parts_between_rasters() {
        let doc = Document {
            layers: vec![
                square(1, [0, 0, 255], [40.0, 40.0]),
                text_layer(2, [40.0, 40.0]),
                square(3, [0, 255, 0], [45.0, 40.0]),
            ],
            ..Default::default()
        };
        let dpi = 50.8;
        let scene = scene_of(doc, dpi);
        let parts = compose_parts(&scene, dpi, true, &mut |_| true).unwrap();
        let kinds: Vec<&str> = parts.iter().map(part_kind).collect();
        assert_eq!(kinds, ["raster", "vector", "raster"]);
        // The upper raster is cropped to the green square: 10 mm at 2 px per mm.
        let Part::Raster { pixmap, origin, .. } = &parts[2] else {
            unreachable!()
        };
        assert_eq!((pixmap.width(), pixmap.height()), (20, 20));
        assert_eq!(*origin, [80, 70]);
        // Stacked again, they give what is rendered in one go.
        let flat = flatten(&parts, &scene, dpi).unwrap();
        let whole = compose(&scene, dpi, false).unwrap();
        let diff = flat
            .data()
            .iter()
            .zip(whole.data())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(diff <= 2, "differs by {diff}");
        // The text shows in red where the green square doesn't cover it.
        assert!(
            flat.pixels()
                .iter()
                .any(|p| p.red() > 200 && p.green() < 50)
        );
        // Without vectors, one part only.
        assert_eq!(
            compose_parts(&scene, dpi, false, &mut |_| true)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn text_stays_pixels_when_vectors_would_differ() {
        let with = |f: &dyn Fn(&mut Layer)| {
            let mut node = text_layer(1, [40.0, 40.0]);
            if let Node::Layer(l) = &mut node {
                f(l);
            }
            let scene = scene_of(
                Document {
                    layers: vec![node],
                    ..Default::default()
                },
                50.8,
            );
            let SceneNode::Image(l) = &scene.layers[0] else {
                unreachable!()
            };
            l.vector().map(|v| v.fills.len())
        };
        assert_eq!(with(&|_| {}), Some(1));
        // Half transparent: fine with one shape, not with an outline under the letters.
        assert_eq!(with(&|l| l.opacity = 0.5), Some(1));
        assert_eq!(with(&|l| l.text.as_mut().unwrap().outline = true), Some(2));
        assert_eq!(
            with(&|l| {
                l.opacity = 0.5;
                l.text.as_mut().unwrap().outline = true;
            }),
            None
        );
        // Effects only exist as pixels.
        assert_eq!(
            with(&|l| {
                let blur = effects::registry().get("builtin.blur").unwrap();
                l.effects = vec![effects::EffectInstance::new(blur)];
            }),
            None
        );
    }

    #[test]
    fn group_opacity_composites_children_together() {
        // Red and blue squares overlapping on [15, 20] x [10, 20], in a 50 % group.
        let doc = Document {
            layers: vec![group(
                1,
                0.5,
                vec![
                    square(2, [255, 0, 0], [15.0, 15.0]),
                    square(3, [0, 0, 255], [20.0, 15.0]),
                ],
            )],
            ..Default::default()
        };
        let scene = Scene::new(
            &doc,
            None,
            &[[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        assert!(matches!(scene.layers[0], SceneNode::Group { .. }));
        let pm = compose(&scene, 254.0, false).unwrap();
        let px = pm.pixel(175, 150).unwrap();
        // Blue fully covers red inside the group, so no red shows through.
        assert!(px.red() > 120 && px.red() < 135, "red {}", px.red());
        assert!(px.green() > 120 && px.green() < 135);
        assert_eq!(px.blue(), 255);
        // Outside the overlap, half-transparent red over white.
        let px = pm.pixel(120, 150).unwrap();
        assert_eq!(px.red(), 255);
        assert!(px.green() > 120 && px.green() < 135);
    }

    /// `node` with its `clip` flag set.
    fn clipped(mut node: Node) -> Node {
        *node.clip_mut() = true;
        node
    }

    fn compose_doc(layers: Vec<Node>) -> Pixmap {
        let doc = Document {
            layers,
            ..Default::default()
        };
        compose(&scene_of(doc, 254.0), 254.0, false).unwrap()
    }

    #[test]
    fn clipped_layers_show_only_over_their_base() {
        // Red base on [10, 20] mm, blue clipped to it on [15, 25], green clipped too on
        // [5, 15], each 10 mm tall from y = 10.
        let pm = compose_doc(vec![
            square(1, [255, 0, 0], [15.0, 15.0]),
            clipped(square(2, [0, 0, 255], [20.0, 15.0])),
            clipped(square(3, [0, 255, 0], [10.0, 15.0])),
        ]);
        let rgb = |x: u32| {
            let p = pm.pixel(x, 150).unwrap();
            [p.red(), p.green(), p.blue()]
        };
        assert_eq!(rgb(175), [0, 0, 255], "blue over the base");
        assert_eq!(rgb(225), [255, 255, 255], "blue cut off past the base");
        assert_eq!(rgb(125), [0, 255, 0], "green over the base");
        assert_eq!(rgb(75), [255, 255, 255], "green cut off before the base");
    }

    #[test]
    fn clipping_needs_a_base_that_shows() {
        // At the bottom of the list, with no base, the clipped layer is drawn as it is.
        let pm = compose_doc(vec![clipped(square(1, [0, 0, 255], [15.0, 15.0]))]);
        assert_eq!(pm.pixel(150, 150).unwrap().blue(), 255);
        assert_eq!(pm.pixel(150, 150).unwrap().red(), 0);
        // A hidden base hides what is clipped to it.
        let mut base = square(1, [255, 0, 0], [15.0, 15.0]);
        if let Node::Layer(l) = &mut base {
            l.visible = false;
        }
        let pm = compose_doc(vec![base, clipped(square(2, [0, 0, 255], [15.0, 15.0]))]);
        assert_eq!(pm.pixel(150, 150).unwrap().red(), 255, "white page");
        assert_eq!(pm.pixel(150, 150).unwrap().green(), 255);
    }

    #[test]
    fn a_group_can_be_a_base() {
        // Two red squares side by side in a group, and blue across both clipped to it.
        let mut wide = square(3, [0, 0, 255], [25.0, 15.0]);
        if let Node::Layer(l) = &mut wide {
            l.transform.scale = [3.0, 1.0];
        }
        let pm = compose_doc(vec![
            group(
                1,
                1.0,
                vec![
                    square(2, [255, 0, 0], [15.0, 15.0]),
                    square(4, [255, 0, 0], [35.0, 15.0]),
                ],
            ),
            clipped(wide),
        ]);
        let blue = |x: u32| {
            pm.pixel(x, 150).unwrap().blue() == 255 && pm.pixel(x, 150).unwrap().red() == 0
        };
        assert!(blue(150) && blue(350), "over both squares");
        assert!(!blue(250), "not in the gap between them");
    }

    fn part_kind(p: &Part) -> &'static str {
        match p {
            Part::Raster { .. } => "raster",
            Part::Vector(_) => "vector",
            Part::ClipBegin(_) => "clip",
            Part::ClipEnd => "end",
        }
    }

    /// A 20 x 10 mm rectangle at [10, 30] x [15, 25] mm, filled and stroked 2 mm wide (so
    /// two shapes), and a blue square at [25, 35] x [15, 25] clipped to it, for an
    /// export at 254 dpi (10 px per mm).
    pub(crate) fn clip_to_shape_scene(base_opacity: f32) -> Scene {
        let mut base = crate::shape::new_layer(1, crate::shape::ShapeKind::Rectangle, [20.0, 20.0]);
        {
            let s = base.shape.as_mut().unwrap();
            s.size = [20.0, 10.0];
            s.fill = true;
            s.stroke = true;
            s.stroke_width = 2.0;
        }
        base.opacity = base_opacity;
        let mut doc = Document {
            layers: vec![
                Node::Layer(Box::new(base)),
                clipped(square(2, [0, 0, 255], [30.0, 20.0])),
            ],
            ..Default::default()
        };
        crate::shape::prepare_export(&mut doc, 254.0);
        scene_of(doc, 254.0)
    }

    /// Stripes clipped to a big text "CLIP", over a gray square, and a cyan square
    /// multiplying them, at 150 dpi.
    pub(crate) fn clip_page_scene() -> Scene {
        let img = image::RgbaImage::from_fn(200, 60, |x, y| {
            let band = (x + y) / 12 % 3;
            image::Rgba(
                [[230, 60, 40, 255], [250, 200, 40, 255], [40, 120, 230, 255]][band as usize],
            )
        });
        let mut stripes = Layer::new(3, "stripes".into(), "x.png".into());
        stripes.size_px = [200, 60];
        stripes.transform.center = [105.0, 60.0];
        stripes.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
        stripes.clip = true;
        let mut t = text::new_layer(2, "CLIP", text::BUILTIN_FAMILY.into(), [105.0, 60.0]);
        let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        t.size_px = text::layout(t.text.as_ref().unwrap(), &font).size;
        t.transform.scale = t.transform.scale.map(|s| s * 10.0);
        // A checkerboard tile, 8 mm, turned 30 degrees, in a star and a text.
        let checker = image::RgbaImage::from_fn(16, 16, |x, y| {
            image::Rgba(if (x / 8 + y / 8) % 2 == 0 {
                [250, 200, 40, 255]
            } else {
                [120, 40, 160, 255]
            })
        });
        let tile = crate::pattern::Pattern {
            source: "checker.png".into(),
            size: 8.0,
            angle_deg: 30.0,
            offset: [0.0; 2],
            data: None,
            tile: Some(Arc::new(premultiplied_pixmap(&checker).unwrap())),
        };
        let mut star = crate::shape::new_layer(5, crate::shape::ShapeKind::Star, [230.0, 60.0]);
        {
            let s = star.shape.as_mut().unwrap();
            s.size = [80.0, 80.0];
            s.fill_pattern = Some(tile.clone());
        }
        let mut pat = text::new_layer(6, "PATTERN", text::BUILTIN_FAMILY.into(), [150.0, 150.0]);
        pat.size_px = text::layout(pat.text.as_ref().unwrap(), &font).size;
        pat.transform.scale = pat.transform.scale.map(|s| s * 6.0);
        let k = pat.transform.scale[1].abs();
        pat.text.as_mut().unwrap().pattern = Some(crate::pattern::Pattern {
            size: tile.size / k,
            ..tile
        });
        let mut doc = Document {
            layers: vec![
                square(1, [200, 200, 200], [60.0, 60.0]),
                Node::Layer(Box::new(t)),
                Node::Layer(Box::new(stripes)),
                blended(square(4, [0, 255, 255], [62.0, 62.0]), BlendMode::Multiply),
                Node::Layer(Box::new(star)),
                Node::Layer(Box::new(pat)),
            ],
            ..Default::default()
        };
        crate::shape::prepare_export(&mut doc, 150.0);
        scene_of(doc, 150.0)
    }

    /// `clip_page_scene` exported to PNG and SVG, to look at:
    /// `CLIP_PAGE_PREVIEW=out cargo test clip_page_preview -- --ignored` writes out.png
    /// and out.svg.
    #[test]
    #[ignore]
    fn clip_page_preview() {
        let Ok(path) = std::env::var("CLIP_PAGE_PREVIEW") else {
            return;
        };
        let scene = clip_page_scene();
        compose(&scene, 150.0, false)
            .unwrap()
            .save_png(format!("{path}.png"))
            .unwrap();
        let parts = compose_parts(&scene, 150.0, true, &mut |_| true).unwrap();
        let svg = format!("{path}.svg");
        crate::svg_export::write(&scene, &parts, 150.0, std::path::Path::new(&svg), None).unwrap();
    }

    #[test]
    fn clipped_to_a_shape_cuts_by_its_outline_as_vectors() {
        let scene = clip_to_shape_scene(1.0);
        let parts = compose_parts(&scene, 254.0, true, &mut |_| true).unwrap();
        let kinds: Vec<&str> = parts.iter().map(part_kind).collect();
        assert_eq!(kinds, ["raster", "vector", "clip", "raster", "end"]);
        // Stacked, the parts give what the page renders as pixels.
        let flat = flatten(&parts, &scene, 254.0).unwrap();
        let pm = compose(&scene, 254.0, false).unwrap();
        for (x, blue) in [(270, true), (305, true), (330, false)] {
            let (a, b) = (flat.pixel(x, 200).unwrap(), pm.pixel(x, 200).unwrap());
            assert_eq!((a.red(), a.blue()), (b.red(), b.blue()), "x = {x}");
            assert_eq!(a.blue() == 255 && a.red() == 0, blue, "x = {x}");
        }
        // The outline of the fill and the stroke together: one ring, around the stroke.
        let Part::ClipBegin(l) = parts[2] else {
            unreachable!()
        };
        let rings = clip_rings(l.clip_outline().unwrap());
        assert_eq!(rings.len(), 1);
        let (lo, hi) = rings[0]
            .iter()
            .fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| {
                (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                )
            });
        for (got, want) in [(lo[0], 9.0), (lo[1], 14.0), (hi[0], 31.0), (hi[1], 26.0)] {
            assert!((got - want).abs() < 0.05, "{got} vs {want}");
        }
    }

    #[test]
    fn a_semi_transparent_base_clips_as_pixels() {
        // Its outline would let the clipped layers show fully where it shows half.
        let scene = clip_to_shape_scene(0.5);
        let parts = compose_parts(&scene, 254.0, true, &mut |_| true).unwrap();
        let kinds: Vec<&str> = parts.iter().map(part_kind).collect();
        assert_eq!(kinds, ["raster"]);
    }

    /// `node` in `blend` mode.
    fn blended(mut node: Node, blend: BlendMode) -> Node {
        match &mut node {
            Node::Layer(l) => l.blend = blend,
            Node::Group(g) => g.blend = blend,
        }
        node
    }

    fn rgb_at(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
        let p = pm.pixel(x, y).unwrap();
        [p.red(), p.green(), p.blue()]
    }

    /// Yellow on [10, 20] mm, cyan multiplied over it on [15, 25], each 10 mm tall from
    /// y = 10, at 10 px per mm.
    fn multiplied() -> Vec<Node> {
        vec![
            square(1, [255, 255, 0], [15.0, 15.0]),
            blended(square(2, [0, 255, 255], [20.0, 15.0]), BlendMode::Multiply),
        ]
    }

    #[test]
    fn a_blend_mode_mixes_with_what_lies_below() {
        let pm = compose_doc(multiplied());
        assert_eq!(rgb_at(&pm, 175, 150), [0, 255, 0], "yellow times cyan");
        assert_eq!(
            rgb_at(&pm, 225, 150),
            [0, 255, 255],
            "cyan times the white page"
        );
        assert_eq!(rgb_at(&pm, 125, 150), [255, 255, 0]);
    }

    #[test]
    fn a_group_in_a_blend_mode_puts_its_layers_together_first() {
        // Inside the group, the cyan square covers the yellow one in normal mode; the
        // group then multiplies the page.
        let pm = compose_doc(vec![
            square(3, [255, 0, 255], [20.0, 15.0]),
            blended(
                group(
                    10,
                    1.0,
                    vec![
                        square(1, [255, 255, 0], [15.0, 15.0]),
                        square(2, [0, 255, 255], [20.0, 15.0]),
                    ],
                ),
                BlendMode::Multiply,
            ),
        ]);
        assert_eq!(rgb_at(&pm, 175, 150), [0, 0, 255], "cyan times magenta");
        assert_eq!(rgb_at(&pm, 125, 150), [255, 255, 0], "yellow times white");
    }

    #[test]
    fn blended_layers_are_parts_of_their_own() {
        let doc = Document {
            layers: multiplied(),
            ..Default::default()
        };
        let scene = scene_of(doc, 254.0);
        let parts = compose_parts(&scene, 254.0, true, &mut |_| true).unwrap();
        assert!(matches!(
            parts[..],
            [
                Part::Raster { .. },
                Part::Raster {
                    blend: BlendMode::Multiply,
                    ..
                }
            ]
        ));
        let flat = flatten(&parts, &scene, 254.0).unwrap();
        let pm = compose(&scene, 254.0, false).unwrap();
        for x in [125, 175, 225] {
            assert_eq!(rgb_at(&flat, x, 150), rgb_at(&pm, x, 150), "x = {x}");
        }
    }

    #[test]
    fn a_clipped_layer_mixes_with_its_base() {
        let pm = compose_doc(vec![
            square(1, [255, 255, 0], [15.0, 15.0]),
            clipped(blended(
                square(2, [0, 255, 255], [20.0, 15.0]),
                BlendMode::Multiply,
            )),
        ]);
        assert_eq!(rgb_at(&pm, 175, 150), [0, 255, 0], "over the base");
        assert_eq!(rgb_at(&pm, 225, 150), [255, 255, 255], "cut off past it");
    }

    /// A pattern of a 2 x 1 px tile, red then blue, `size` units wide.
    pub(crate) fn red_blue(size: f32) -> crate::pattern::Pattern {
        let img = image::RgbaImage::from_fn(2, 1, |x, _| {
            image::Rgba(if x == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        });
        crate::pattern::Pattern {
            source: "tile.png".into(),
            size,
            angle_deg: 0.0,
            offset: [0.0; 2],
            data: None,
            tile: Some(Arc::new(premultiplied_pixmap(&img).unwrap())),
        }
    }

    /// A 20 x 10 mm rectangle at [10, 30] x [15, 25] mm filled with `red_blue(size)`:
    /// red and blue bands `size / 2` mm wide from x = 10, at 254 dpi (10 px per mm).
    pub(crate) fn pattern_scene(size: f32) -> Scene {
        let mut l = crate::shape::new_layer(1, crate::shape::ShapeKind::Rectangle, [20.0, 20.0]);
        {
            let s = l.shape.as_mut().unwrap();
            s.size = [20.0, 10.0];
            s.fill = true;
            s.stroke = false;
            s.fill_pattern = Some(red_blue(size));
        }
        let mut doc = Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        };
        crate::shape::prepare_export(&mut doc, 254.0);
        scene_of(doc, 254.0)
    }

    /// Red (true) or blue (false) at x px on the middle row; panics on anything else.
    fn red_at(pm: &Pixmap, x: u32) -> bool {
        let [r, _, b] = rgb_at(pm, x, 200);
        assert!(r.abs_diff(b) > 150, "x = {x}: {r}, {b}");
        r > b
    }

    #[test]
    fn a_pattern_repeats_its_tile_over_the_shape() {
        let scene = pattern_scene(10.0);
        let pm = compose(&scene, 254.0, false).unwrap();
        let bands: Vec<bool> = [125, 175, 225, 275].map(|x| red_at(&pm, x)).into();
        assert_eq!(bands, [true, false, true, false]);
        // As a vector part, stacked back the same.
        let parts = compose_parts(&scene, 254.0, true, &mut |_| true).unwrap();
        assert!(matches!(parts[..], [Part::Raster { .. }, Part::Vector(_)]));
        let flat = flatten(&parts, &scene, 254.0).unwrap();
        for x in [125, 175, 225, 275] {
            assert_eq!(red_at(&flat, x), red_at(&pm, x), "x = {x}");
        }
    }

    #[test]
    fn too_many_tiles_are_exported_as_pixels() {
        // 0.02 mm tiles: 1000 x 500 of them over the rectangle.
        let scene = pattern_scene(0.02);
        let parts = compose_parts(&scene, 254.0, true, &mut |_| true).unwrap();
        assert!(matches!(parts[..], [Part::Raster { .. }]));
    }

    #[test]
    fn a_background_pattern_fills_the_page() {
        let mut page = Pixmap::new(400, 100).unwrap();
        // 10 mm tiles at 10 px per mm: 50 px bands.
        fill_background(&mut page, [0, 255, 0], None, Some(&red_blue(10.0)), 10.0);
        let bands: Vec<bool> = [25, 75, 125, 175].map(|x| red_at_row(&page, x, 50)).into();
        assert_eq!(bands, [true, false, true, false]);
    }

    fn red_at_row(pm: &Pixmap, x: u32, y: u32) -> bool {
        let [r, g, b] = rgb_at(pm, x, y);
        assert!(g < 50 && r.abs_diff(b) > 150, "x = {x}: {r}, {g}, {b}");
        r > b
    }

    #[test]
    fn hidden_and_single_child_groups_are_simplified() {
        let mut doc = Document::default();
        let mut hidden = group(4, 1.0, vec![square(5, [0, 0, 0], [50.0, 25.0])]);
        if let Node::Group(g) = &mut hidden {
            g.visible = false;
        }
        doc.layers = vec![
            group(
                1,
                0.5,
                vec![group(2, 0.5, vec![square(3, [255, 0, 0], [15.0, 15.0])])],
            ),
            hidden,
        ];
        let scene = Scene::new(
            &doc,
            None,
            &[[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        assert_eq!(scene.layers.len(), 1);
        match &scene.layers[0] {
            SceneNode::Image(l) => assert!((l.opacity - 0.25).abs() < 1e-6),
            _ => panic!("expected the layer itself"),
        }
    }

    /// Checkerboards tilted in 3D (none, top away, right side away, both), exported:
    /// `TILT_PREVIEW=out.png cargo test tilt_preview -- --ignored`.
    #[test]
    #[ignore]
    fn tilt_preview() {
        let Ok(path) = std::env::var("TILT_PREVIEW") else {
            return;
        };
        let img = image::RgbaImage::from_fn(400, 400, |x, y| {
            if (x / 50 + y / 50) % 2 == 0 {
                image::Rgba([200, 40, 40, 255])
            } else {
                image::Rgba([250, 240, 220, 255])
            }
        });
        let layers = [(0.0, 0.0), (55.0, 0.0), (0.0, 55.0), (40.0, -35.0)]
            .iter()
            .enumerate()
            .map(|(i, &(tx, ty))| {
                let mut l = Layer::new(i as u64 + 1, "c".into(), "x.png".into());
                l.size_px = [400, 400];
                l.transform.scale = [0.1, 0.1];
                l.transform.center = [25.0 + 50.0 * i as f32, 25.0];
                l.transform.tilt_x_deg = tx;
                l.transform.tilt_y_deg = ty;
                l.transform.perspective = 0.8;
                l.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
                Node::Layer(Box::new(l))
            })
            .collect();
        let doc = Document {
            layers,
            page: crate::document::PageSize::Custom,
            custom_page_mm: [50.0, 200.0],
            ..Default::default()
        };
        let scene = Scene::new(
            &doc,
            None,
            &[[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let pm = compose(&scene, 200.0, false).unwrap();
        pm.save_png(&path).unwrap();
    }

    #[test]
    fn distorted_layer_is_warped_in_perspective() {
        // 10 mm square at (15, 15), top edge pulled in to 4 mm wide.
        let mut node = square(1, [0, 0, 255], [15.0, 15.0]);
        if let Node::Layer(l) = &mut node {
            l.transform.distort = [[0.3, 0.0], [-0.3, 0.0], [0.0, 0.0], [0.0, 0.0]];
        }
        let doc = Document {
            layers: vec![node],
            ..Default::default()
        };
        let scene = Scene::new(
            &doc,
            None,
            &[[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let pm = compose(&scene, 254.0, false).unwrap(); // 10 px per mm
        let blue = |x: u32, y: u32| pm.pixel(x, y).unwrap().red() < 10;
        let white = |x: u32, y: u32| pm.pixel(x, y).unwrap().red() > 245;
        assert!(blue(150, 105) && blue(150, 195), "center column");
        assert!(white(115, 105), "top-left corner was cut away");
        assert!(white(185, 105), "top-right corner too");
        assert!(
            blue(105, 195) && blue(195, 195),
            "bottom edge keeps its width"
        );
        // Smooth edge: a pixel straddling the slanted edge is a blend.
        let edge = (100..150)
            .map(|x| pm.pixel(x, 150).unwrap().red())
            .collect::<Vec<_>>();
        assert!(edge.iter().any(|&r| r > 20 && r < 235), "{edge:?}");
    }

    #[test]
    fn large_reductions_are_averaged() {
        // A 1 px checkerboard squeezed to a fraction of its size averages to gray.
        let img = image::RgbaImage::from_fn(400, 400, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            image::Rgba([v, v, v, 255])
        });
        let mut l = crate::document::Layer::new(1, "c".into(), "x.png".into());
        l.size_px = [400, 400];
        l.transform.center = [20.0, 20.0];
        l.transform.scale = [0.1, 0.1]; // 40 mm wide
        l.transform.distort = [[0.1, 0.0], [-0.1, 0.0], [0.0, 0.0], [0.0, 0.0]];
        l.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
        let doc = Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        };
        let scene = Scene::new(
            &doc,
            None,
            &[[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let pm = compose(&scene, 25.4, false).unwrap(); // 1 px per mm: 10x reduction
        for x in 15..25 {
            let r = pm.pixel(x, 30).unwrap().red();
            assert!((100..=155).contains(&r), "pixel {x}: {r}");
        }
    }

    fn rect_poly(lo: P2, hi: P2) -> Polyline {
        Polyline {
            points: vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
            closed: true,
        }
    }

    fn element(kind: ElementKind, polylines: Vec<Polyline>, print: bool) -> SceneElement {
        SceneElement {
            placed: PlacedElement {
                kind,
                polylines,
                centers: Vec::new(),
            },
            color: [0, 0, 0],
            print,
        }
    }

    #[test]
    fn template_clips_artwork_and_blanks_holes() {
        // A black page-sized layer, an outline [10, 90] x [10, 40], a button and a screw
        // hole, the screw hole drawn twice (as in the T16 file).
        let img = image::RgbaImage::from_pixel(100, 50, image::Rgba([0, 0, 0, 255]));
        let mut l = Layer::new(1, "black".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [50.0, 25.0];
        l.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
        let screw = rect_poly([70.0, 20.0], [74.0, 24.0]);
        let mut scene = Scene {
            page_mm: [100.0, 50.0],
            background: [255, 0, 0],
            background_gradient: None,
            background_pattern: None,
            layers: scene_nodes(&[Node::Layer(Box::new(l))], false),
            elements: vec![
                element(
                    ElementKind::Outline,
                    vec![rect_poly([10.0, 10.0], [90.0, 40.0])],
                    true,
                ),
                element(
                    ElementKind::Holes,
                    vec![rect_poly([20.0, 20.0], [30.0, 30.0])],
                    false,
                ),
                element(ElementKind::Holes, vec![screw.clone(), screw], false),
            ],
            cuts: Cuts::default(),
            marks: Vec::new(),
        };
        let pm = compose(&scene, 25.4, false).unwrap(); // 1 px per mm
        let px = |x: u32, y: u32| {
            let p = pm.pixel(x, y).unwrap();
            [p.red(), p.green(), p.blue()]
        };
        assert_eq!(px(50, 15), [0, 0, 0], "artwork inside the outline");
        assert_eq!(
            px(5, 5),
            [255, 255, 255],
            "outside the outline: blank paper"
        );
        assert_eq!(px(25, 25), [255, 255, 255], "button hole");
        assert_eq!(px(72, 22), [255, 255, 255], "screw hole drawn twice");

        // Only the elements marked for printing get lines.
        let pm = compose(&scene, 25.4, true).unwrap();
        assert!(pm.pixel(50, 10).unwrap().red() < 200, "outline line");
        assert_eq!(pm.pixel(25, 20).unwrap().red(), 255, "no button line");

        // Holes can be printed over.
        scene.cuts.blank_holes = false;
        let pm = compose(&scene, 25.4, false).unwrap();
        assert_eq!(pm.pixel(25, 25).unwrap().red(), 0, "button printed over");
        assert_eq!(
            pm.pixel(72, 22).unwrap().red(),
            0,
            "screw hole printed over"
        );

        scene.cuts = Cuts {
            clip_to_outline: false,
            blank_holes: false,
            bleed_mm: 0.0,
        };
        let pm = compose(&scene, 25.4, false).unwrap();
        assert_eq!(pm.pixel(5, 5).unwrap().red(), 0);
        assert_eq!(pm.pixel(25, 25).unwrap().red(), 0);

        // A bleed: the artwork goes past the outline and into the holes by 3 mm.
        scene.cuts = Cuts {
            bleed_mm: 3.0,
            ..Cuts::default()
        };
        let pm = compose(&scene, 25.4, false).unwrap();
        let black = |x, y| pm.pixel(x, y).unwrap().red() == 0;
        let white = |x, y| pm.pixel(x, y).unwrap().red() == 255;
        assert!(
            black(8, 25) && black(50, 7),
            "in the bleed past the outline"
        );
        assert!(white(5, 25) && white(50, 5), "past the bleed");
        assert!(black(21, 25) && white(25, 25), "the button hole shrunk");
        assert!(
            black(72, 22),
            "a screw hole thinner than the bleed is printed over"
        );
        // The PDF's vector cuts follow the bleed too.
        let (outline, holes) = cut_shapes(&scene);
        let rings: Vec<Polyline> = outline
            .into_iter()
            .map(|points| Polyline {
                points,
                closed: true,
            })
            .collect();
        let (mn, mx) = print::bounds(&rings).unwrap();
        assert!(
            (mn[0] - 7.0).abs() < 0.05 && (mx[1] - 43.0).abs() < 0.05,
            "{mn:?} {mx:?}"
        );
        assert_eq!(holes.len(), 1, "only the button is left");

        // Marks around the outline's box, outside the bleed, in black.
        let mut scene = scene.with_marks(true, false);
        assert_eq!(scene.marks.len(), 8);
        scene.elements.iter_mut().for_each(|e| e.print = false);
        let pm = compose(&scene, 25.4, true).unwrap();
        let p = pm.pixel(3, 10).unwrap();
        assert!(
            p.red() < 200 && p.green() < 200,
            "crop mark left of the top-left corner"
        );
        scene.elements.clear();
        assert!(scene.with_marks(true, true).marks.is_empty());
    }

    #[test]
    fn open_outline_does_not_clip() {
        let open = Polyline {
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            closed: false,
        };
        let placed = vec![PlacedElement {
            kind: ElementKind::Outline,
            polylines: vec![open],
            centers: Vec::new(),
        }];
        assert!(keep_mask(&placed, Cuts::default(), [0.0, 0.0], [10, 10], 1.0).is_none());
    }

    #[test]
    fn page_pixel_sizes_at_300_dpi() {
        let mut d = Document::default();
        assert_eq!(pixel_size(d.page_mm(), 300.0), [3508, 2480]);
        d.page = PageSize::A3;
        assert_eq!(pixel_size(d.page_mm(), 300.0), [4961, 3508]);
        d.orientation = Orientation::Portrait;
        assert_eq!(pixel_size(d.page_mm(), 300.0), [3508, 4961]);
    }

    #[test]
    fn export_applies_layer_effects() {
        use crate::effects::{EffectInstance, registry};
        let mut l = crate::document::Layer::new(1, "a".into(), "a.png".into());
        let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 255, 255]));
        l.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
        l.size_px = [10, 10];
        l.transform.center = [5.0, 5.0];
        l.effects.push(EffectInstance::new(
            registry().get("builtin.invert").unwrap(),
        ));
        let scene = Scene {
            page_mm: [10.0, 10.0],
            background: [255, 255, 255],
            background_gradient: None,
            background_pattern: None,
            layers: scene_nodes(&[Node::Layer(Box::new(l))], false),
            elements: Vec::new(),
            cuts: Cuts::default(),
            marks: Vec::new(),
        };
        let pm = compose(&scene, 25.4, false).unwrap();
        let px = pm.pixel(5, 5).unwrap();
        assert_eq!((px.red(), px.green(), px.blue()), (255, 255, 0));
    }

    #[test]
    fn compose_draws_layer_and_outline() {
        let mut img = image::RgbaImage::new(10, 10);
        for p in img.pixels_mut() {
            *p = image::Rgba([0, 0, 255, 255]);
        }
        let scene = Scene {
            page_mm: [100.0, 50.0],
            background: [255, 255, 255],
            background_gradient: None,
            background_pattern: None,
            // A 10 mm blue square centered at (25, 25).
            layers: vec![SceneNode::Image(SceneLayer {
                pixmap: Arc::new(premultiplied_pixmap(&img).unwrap()),
                map: Affine::translate(-5.0, -5.0)
                    .then(Affine::translate(25.0, 25.0))
                    .into(),
                opacity: 1.0,
                blend: BlendMode::Normal,
                effects: Vec::new(),
                effects_scale: 1.0,
                vector: None,
            })],
            elements: vec![SceneElement {
                placed: PlacedElement {
                    kind: ElementKind::Outline,
                    polylines: vec![Polyline {
                        points: vec![[60.0, 10.0], [90.0, 10.0], [90.0, 40.0]],
                        closed: true,
                    }],
                    centers: Vec::new(),
                },
                color: [255, 0, 0],
                print: true,
            }],
            cuts: Cuts {
                clip_to_outline: false,
                blank_holes: false,
                bleed_mm: 0.0,
            },
            marks: Vec::new(),
        };
        let pm = compose(&scene, 254.0, true).unwrap(); // 10 px per mm
        assert_eq!((pm.width(), pm.height()), (1000, 500));
        let px = |x: u32, y: u32| pm.pixel(x, y).unwrap();
        assert_eq!((px(250, 250).red(), px(250, 250).blue()), (0, 255));
        assert_eq!(px(10, 10).red(), 255);
        assert_eq!((px(750, 100).red(), px(750, 100).green()), (255, 0));
    }
}
