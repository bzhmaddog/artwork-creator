//! Offscreen compositing shared by PNG and PDF export.

use std::sync::Arc;

use tiny_skia::{
    FillRule, FilterQuality, IntSize, Mask, Paint, PathBuilder, Pixmap, PixmapMut, PixmapPaint,
    Stroke, Transform,
};

use crate::document::{Affine, Document, Homography, Node, P2};
use crate::effects::{self, EffectInstance};
use crate::i18n::tr;
use crate::print;
use crate::template::{ElementKind, KINDS, Polyline, Template};
use crate::text;

/// Width of printed cut lines.
pub const OUTLINE_WIDTH_MM: f32 = 0.25;
/// Half the size of the cross marking a printed anchor.
pub const ANCHOR_MARK_MM: f32 = 2.0;

/// An image layer ready to draw.
pub struct SceneLayer {
    /// Premultiplied pixels.
    pub pixmap: Arc<Pixmap>,
    /// Pixmap pixels to page mm; projective when the layer is distorted.
    pub map: Homography,
    /// Its opacity, with any group opacity folded in.
    pub opacity: f32,
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
        let gradients_fit = v.fills.iter().all(|(_, ink, _)| match ink {
            crate::gradient::Ink::Solid(_) => true,
            crate::gradient::Ink::Gradient { gradient, .. } => {
                gradient.is_opaque() && v.map.as_affine().is_some()
            }
        });
        (gradients_fit && (self.opacity >= 1.0 || v.fills.len() <= 1)).then_some(v)
    }
}

/// What gets drawn: an image, or a group composited on its own.
pub enum SceneNode {
    Image(SceneLayer),
    /// Children are composited on their own, then blended with `opacity`.
    Group {
        opacity: f32,
        children: Vec<SceneNode>,
    },
}

impl SceneNode {
    /// The node's opacity, image or group.
    fn opacity_mut(&mut self) -> &mut f32 {
        match self {
            SceneNode::Image(l) => &mut l.opacity,
            SceneNode::Group { opacity, .. } => opacity,
        }
    }
}

/// Converts layers and groups into what gets drawn: hidden or empty entries are dropped,
/// opaque groups are flattened, and a group with a single child folds its opacity into
/// that child (same result, no offscreen buffer). With `preview`, uses the layers'
/// smaller preview pixmaps.
pub fn scene_nodes(nodes: &[Node], preview: bool) -> Vec<SceneNode> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            Node::Layer(l) => {
                if !l.visible || l.opacity <= 0.0 {
                    continue;
                }
                let src = if preview {
                    l.shown_preview()
                } else {
                    l.pixmap.as_ref()
                };
                let Some(pm) = src else { continue };
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
                    vector: (!preview && effects.is_empty())
                        .then(|| outlines(l).map(Arc::new))
                        .flatten(),
                    effects,
                    effects_scale: 1.0 / k[0],
                }));
            }
            Node::Group(g) => {
                if !g.visible || g.opacity <= 0.0 {
                    continue;
                }
                let mut children = scene_nodes(&g.children, preview);
                if g.opacity >= 1.0 {
                    out.append(&mut children);
                } else if children.len() == 1 {
                    let mut only = children.remove(0);
                    *only.opacity_mut() *= g.opacity;
                    out.push(only);
                } else if !children.is_empty() {
                    out.push(SceneNode::Group {
                        opacity: g.opacity,
                        children,
                    });
                }
            }
        }
    }
    out
}

/// A template element placed on the page, in page mm.
#[derive(Clone, Debug)]
pub struct PlacedElement {
    /// What the element is for.
    pub kind: ElementKind,
    /// Its shapes in page mm.
    pub polylines: Vec<Polyline>,
    /// Its anchor points in page mm (anchors only).
    pub anchors: Vec<P2>,
}

impl PlacedElement {
    /// The lines to print: its shapes, then a small cross on each anchor.
    pub fn printed_lines(&self) -> impl Iterator<Item = std::borrow::Cow<'_, Polyline>> {
        let d = ANCHOR_MARK_MM;
        let crosses = self.anchors.iter().flat_map(move |&[x, y]| {
            [[[x - d, y], [x + d, y]], [[x, y - d], [x, y + d]]].map(|seg| Polyline {
                points: seg.to_vec(),
                closed: false,
            })
        });
        self.polylines
            .iter()
            .map(std::borrow::Cow::Borrowed)
            .chain(crosses.map(std::borrow::Cow::Owned))
    }
}

/// How the template cuts the artwork.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Cuts {
    /// Nothing outside the outline is printed.
    pub clip_to_outline: bool,
    /// Nothing is printed inside buttons.
    pub blank_buttons: bool,
    /// Nothing is printed inside cutouts.
    pub blank_cutouts: bool,
    /// The artwork goes this far past the cut lines, in mm, so a cut slightly off the
    /// line leaves no blank edge: the outline grows by it and the holes shrink by it.
    /// Set from the project's bleed, so not saved with the export settings.
    #[serde(skip)]
    pub bleed_mm: f32,
}

impl std::hash::Hash for Cuts {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (self.clip_to_outline, self.blank_buttons, self.blank_cutouts).hash(h);
        self.bleed_mm.to_bits().hash(h);
    }
}

impl Cuts {
    /// Whether the inside of `kind` is left blank. The outline is never a hole.
    pub fn blanks(self, kind: ElementKind) -> bool {
        match kind {
            ElementKind::Outline | ElementKind::Anchors => false,
            ElementKind::Buttons => self.blank_buttons,
            ElementKind::Cutouts => self.blank_cutouts,
        }
    }
}

impl Default for Cuts {
    fn default() -> Self {
        Self {
            clip_to_outline: true,
            blank_buttons: true,
            blank_cutouts: true,
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

    /// The page as exported. `colors` and `print` are per element kind
    /// (`ElementKind::index`).
    pub fn new(
        doc: &Document,
        template: Option<&Template>,
        colors: [[u8; 3]; KINDS],
        print: [bool; KINDS],
        cuts: Cuts,
    ) -> Self {
        Self::build(doc, template, colors, print, cuts, false)
    }

    /// Like `new`, but with the layers' smaller preview images: quick to render small.
    pub fn preview(
        doc: &Document,
        template: Option<&Template>,
        colors: [[u8; 3]; KINDS],
        print: [bool; KINDS],
        cuts: Cuts,
    ) -> Self {
        Self::build(doc, template, colors, print, cuts, true)
    }

    fn build(
        doc: &Document,
        template: Option<&Template>,
        colors: [[u8; 3]; KINDS],
        print: [bool; KINDS],
        cuts: Cuts,
        preview: bool,
    ) -> Self {
        let elements = template
            .map(|t| placed_elements(doc, t))
            .unwrap_or_default()
            .into_iter()
            .map(|placed| SceneElement {
                color: colors[placed.kind.index()],
                print: print[placed.kind.index()],
                placed,
            })
            .collect();
        Self {
            page_mm: doc.page_mm(),
            background: doc.background,
            background_gradient: doc.background_gradient.clone(),
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
    /// The page mirrored left to right, everything on it included (layers, template
    /// lines, cuts and marks): for printing on the back of a clear sheet, to be seen
    /// through it.
    pub fn mirrored(mut self) -> Self {
        let m = Affine::scale(-1.0, 1.0).then(Affine::translate(self.page_mm[0], 0.0));
        mirror_nodes(&mut self.layers, m);
        let flip = |p: &mut P2| *p = m.apply(*p);
        for e in &mut self.elements {
            e.placed
                .polylines
                .iter_mut()
                .flat_map(|p| &mut p.points)
                .for_each(flip);
            e.placed.anchors.iter_mut().for_each(flip);
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
            anchors: e.anchors.iter().map(|&q| m.apply(q)).collect(),
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
                quality: FilterQuality::Bicubic,
                ..Default::default()
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
        None => draw_warped(target, pixmap, map, l.opacity)?,
    }
    Ok(())
}

fn draw_nodes(target: &mut PixmapMut, nodes: &[SceneNode], to_px: Affine) -> Result<(), String> {
    for node in nodes {
        match node {
            SceneNode::Image(l) => draw_layer(target, l, to_px)?,
            SceneNode::Group { opacity, children } => {
                // Offscreen buffer covering only the group's footprint on the target.
                let Some((lo, hi)) = bounds(children, to_px) else {
                    continue;
                };
                let x0 = (lo[0].floor() as i64 - 1).max(0);
                let y0 = (lo[1].floor() as i64 - 1).max(0);
                let x1 = (hi[0].ceil() as i64 + 1).min(target.width() as i64);
                let y1 = (hi[1].ceil() as i64 + 1).min(target.height() as i64);
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
                let mut buf = Pixmap::new(w, h)
                    .ok_or_else(|| tr!("CANNOT_ALLOCATE_GROUP_BUFFER", w = w, h = h))?;
                let shift = Affine::translate(-x0 as f32, -y0 as f32);
                draw_nodes(&mut buf.as_mut(), children, to_px.then(shift))?;
                let paint = PixmapPaint {
                    opacity: opacity.clamp(0.0, 1.0),
                    quality: FilterQuality::Nearest,
                    ..Default::default()
                };
                target.draw_pixmap(
                    x0 as i32,
                    y0 as i32,
                    buf.as_ref(),
                    &paint,
                    Transform::identity(),
                    None,
                );
            }
        }
    }
    Ok(())
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
    Raster { pixmap: Pixmap, origin: [u32; 2] },
    /// A text or shape layer to draw as vectors: `SceneLayer::vector` gives its shapes.
    Vector(&'a SceneLayer),
}

/// Renders the page as parts to stack, bottom first. With `vectors`, the top-level text
/// and shape layers that can be drawn as vectors become parts of their own, and the layers between
/// them are rendered together. Without, there is a single part: the page as `compose`
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
    );

    let mm_to_px = Affine::scale(dpi / 25.4, dpi / 25.4);
    let mut parts = Vec::new();
    let mut current = Some(page);
    let finish = |parts: &mut Vec<Part<'a>>, pixmap: Pixmap| {
        if parts.is_empty() {
            parts.push(Part::Raster {
                pixmap,
                origin: [0, 0],
            });
        } else if let Some((pixmap, origin)) = crop_to_content(&pixmap) {
            parts.push(Part::Raster { pixmap, origin });
        }
    };
    // One step per top-level node, plus one for the cuts.
    let steps = scene.layers.len() as f32 + 1.0;
    for (i, node) in scene.layers.iter().enumerate() {
        if !progress(i as f32 / steps) {
            return Err(CANCELED.into());
        }
        if vectors
            && let SceneNode::Image(l) = node
            && l.vector().is_some()
        {
            if let Some(pixmap) = current.take() {
                finish(&mut parts, pixmap);
            }
            parts.push(Part::Vector(l));
            continue;
        }
        let target = match &mut current {
            Some(p) => p,
            None => current.insert(new_pixmap()?),
        };
        draw_nodes(&mut target.as_mut(), std::slice::from_ref(node), mm_to_px)?;
    }
    if let Some(pixmap) = current.take() {
        finish(&mut parts, pixmap);
    }
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
    for part in it {
        match part {
            Part::Raster { pixmap, origin } => out.draw_pixmap(
                origin[0] as i32,
                origin[1] as i32,
                pixmap.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            ),
            Part::Vector(l) => draw_layer(&mut out.as_mut(), l, mm_to_px)?,
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

/// Fills a page-sized pixmap with the background: its color, or its gradient over the
/// whole pixmap, kept opaque.
pub fn fill_background(
    page: &mut Pixmap,
    color: [u8; 3],
    gradient: Option<&crate::gradient::Gradient>,
) {
    let [r, g, b] = color;
    page.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));
    let Some(gradient) = gradient else {
        return;
    };
    let mut opaque = gradient.clone();
    for s in &mut opaque.stops {
        s.color[3] = 255;
    }
    let Some(frame) =
        tiny_skia::Rect::from_xywh(0.0, 0.0, page.width() as f32, page.height() as f32)
    else {
        return;
    };
    let ink = crate::gradient::Ink::Gradient {
        gradient: opaque,
        frame,
    };
    page.fill_rect(frame, &ink.paint(), tiny_skia::Transform::identity(), None);
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
mod tests {
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
            [[255, 0, 255]; crate::template::KINDS],
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
            expanded: true,
            children,
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
            [[0; 3]; crate::template::KINDS],
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
        let kinds: Vec<&str> = parts
            .iter()
            .map(|p| match p {
                Part::Raster { .. } => "raster",
                Part::Vector(_) => "vector",
            })
            .collect();
        assert_eq!(kinds, ["raster", "vector", "raster"]);
        // The upper raster is cropped to the green square: 10 mm at 2 px per mm.
        let Part::Raster { pixmap, origin } = &parts[2] else {
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
            [[0; 3]; crate::template::KINDS],
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
            [[0; 3]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        assert_eq!(scene.layers.len(), 1);
        match &scene.layers[0] {
            SceneNode::Image(l) => assert!((l.opacity - 0.25).abs() < 1e-6),
            SceneNode::Group { .. } => panic!("expected the layer itself"),
        }
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
            [[0; 3]; crate::template::KINDS],
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
            [[0; 3]; crate::template::KINDS],
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

    #[test]
    fn anchors_print_as_crosses() {
        let placed = PlacedElement {
            kind: ElementKind::Anchors,
            polylines: Vec::new(),
            anchors: vec![[50.0, 25.0]],
        };
        let lines: Vec<_> = placed.printed_lines().map(|p| p.into_owned()).collect();
        let d = ANCHOR_MARK_MM;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].points, [[50.0 - d, 25.0], [50.0 + d, 25.0]]);
        assert_eq!(lines[1].points, [[50.0, 25.0 - d], [50.0, 25.0 + d]]);
        assert!(lines.iter().all(|l| !l.closed));

        let mut scene = Scene {
            page_mm: [100.0, 50.0],
            background: [255, 255, 255],
            background_gradient: None,
            layers: Vec::new(),
            elements: vec![SceneElement {
                placed,
                color: [255, 0, 0],
                print: true,
            }],
            cuts: Cuts::default(),
            marks: Vec::new(),
        };
        let red = |pm: &Pixmap, x, y| pm.pixel(x, y).unwrap().green() < 50;
        let pm = compose(&scene, 254.0, true).unwrap(); // 10 px per mm
        assert!(red(&pm, 500, 250) && red(&pm, 515, 250) && red(&pm, 500, 235));
        assert!(!red(&pm, 515, 235), "a cross, not a square");
        scene.elements[0].print = false;
        let pm = compose(&scene, 254.0, true).unwrap();
        assert!(!red(&pm, 500, 250));
    }

    fn element(kind: ElementKind, polylines: Vec<Polyline>, print: bool) -> SceneElement {
        SceneElement {
            placed: PlacedElement {
                kind,
                polylines,
                anchors: Vec::new(),
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
            layers: scene_nodes(&[Node::Layer(Box::new(l))], false),
            elements: vec![
                element(
                    ElementKind::Outline,
                    vec![rect_poly([10.0, 10.0], [90.0, 40.0])],
                    true,
                ),
                element(
                    ElementKind::Buttons,
                    vec![rect_poly([20.0, 20.0], [30.0, 30.0])],
                    false,
                ),
                element(ElementKind::Cutouts, vec![screw.clone(), screw], false),
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

        // Buttons can be printed over while cutouts stay blank.
        scene.cuts.blank_buttons = false;
        let pm = compose(&scene, 25.4, false).unwrap();
        assert_eq!(pm.pixel(25, 25).unwrap().red(), 0, "button printed over");
        assert_eq!(
            pm.pixel(72, 22).unwrap().red(),
            255,
            "screw hole still blank"
        );

        scene.cuts = Cuts {
            clip_to_outline: false,
            blank_buttons: false,
            blank_cutouts: false,
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
            anchors: Vec::new(),
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
            // A 10 mm blue square centered at (25, 25).
            layers: vec![SceneNode::Image(SceneLayer {
                pixmap: Arc::new(premultiplied_pixmap(&img).unwrap()),
                map: Affine::translate(-5.0, -5.0)
                    .then(Affine::translate(25.0, 25.0))
                    .into(),
                opacity: 1.0,
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
                    anchors: Vec::new(),
                },
                color: [255, 0, 0],
                print: true,
            }],
            cuts: Cuts {
                clip_to_outline: false,
                blank_buttons: false,
                blank_cutouts: false,
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
