//! Offscreen compositing shared by PNG and PDF export.

use std::sync::Arc;

use tiny_skia::{
    FillRule, FilterQuality, IntSize, Mask, Paint, PathBuilder, Pixmap, PixmapMut, PixmapPaint,
    Stroke, Transform,
};

use crate::document::{Affine, Document, Homography, Node, P2};
use crate::effects::{self, EffectInstance};
use crate::template::{ElementKind, Polyline, Template};

/// Width of printed cut lines.
pub const OUTLINE_WIDTH_MM: f32 = 0.25;

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
}

/// How the template cuts the artwork.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cuts {
    /// Nothing outside the outline is printed.
    pub clip_to_outline: bool,
    /// Nothing is printed inside buttons.
    pub blank_buttons: bool,
    /// Nothing is printed inside cutouts.
    pub blank_cutouts: bool,
}

impl Cuts {
    /// Whether the inside of `kind` is left blank. The outline is never a hole.
    pub fn blanks(self, kind: ElementKind) -> bool {
        match kind {
            ElementKind::Outline => false,
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
        }
    }
}

/// A template element as exported.
pub struct SceneElement {
    /// Its shapes on the page.
    pub placed: PlacedElement,
    /// Line colour (RGB).
    pub color: [u8; 3],
    /// Its lines are printed.
    pub print: bool,
}

/// Everything needed to render the page, detached from the UI so it can move to a thread.
pub struct Scene {
    /// Page size in mm.
    pub page_mm: P2,
    /// Page colour (RGB).
    pub background: [u8; 3],
    /// What to draw, bottom first.
    pub layers: Vec<SceneNode>,
    /// Template elements on the page.
    pub elements: Vec<SceneElement>,
    /// What the template cuts away.
    pub cuts: Cuts,
}

impl Scene {
    /// The page as exported. `colors` and `print` are per element kind
    /// (`ElementKind::index`).
    pub fn new(
        doc: &Document,
        template: Option<&Template>,
        colors: [[u8; 3]; 3],
        print: [bool; 3],
        cuts: Cuts,
    ) -> Self {
        Self::build(doc, template, colors, print, cuts, false)
    }

    /// Like `new`, but with the layers' smaller preview images: quick to render small.
    pub fn preview(
        doc: &Document,
        template: Option<&Template>,
        colors: [[u8; 3]; 3],
        print: [bool; 3],
        cuts: Cuts,
    ) -> Self {
        Self::build(doc, template, colors, print, cuts, true)
    }

    fn build(
        doc: &Document,
        template: Option<&Template>,
        colors: [[u8; 3]; 3],
        print: [bool; 3],
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
            layers: scene_nodes(&doc.layers, preview),
            elements,
            cuts,
        }
    }
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
        })
        .collect()
}

/// Closed polylines as one path, mapped by `to_px`.
fn closed_path<'a>(
    polylines: impl Iterator<Item = &'a Polyline>,
    to_px: Affine,
) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for poly in polylines.filter(|p| p.closed && p.points.len() > 2) {
        let mut pts = poly.points.iter().map(|&p| to_px.apply(p));
        let Some(first) = pts.next() else { continue };
        pb.move_to(first[0], first[1]);
        for p in pts {
            pb.line_to(p[0], p[1]);
        }
        pb.close();
    }
    pb.finish()
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
    let of = |keep: &dyn Fn(ElementKind) -> bool| {
        let polylines = elements
            .iter()
            .filter(|e| keep(e.kind))
            .flat_map(|e| &e.polylines);
        closed_path(polylines, to_px)
    };
    let outline = cuts
        .clip_to_outline
        .then(|| of(&|k| !k.is_hole()))
        .flatten();
    let holes = of(&|k| cuts.blanks(k));
    if outline.is_none() && holes.is_none() {
        return None;
    }
    let [w, h] = size_px;
    // Non-zero winding: shapes drawn twice or overlapping still count once.
    let fill = |path: &tiny_skia::Path| {
        let mut m = Mask::new(w, h)?;
        m.fill_path(path, FillRule::Winding, true, Transform::identity());
        Some(m.take())
    };
    let mut keep = match &outline {
        Some(p) => fill(p)?,
        None => vec![255; w as usize * h as usize],
    };
    if let Some(p) = &holes {
        for (k, hole) in keep.iter_mut().zip(fill(p)?) {
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
        let next = halve(cur).ok_or("Cannot allocate a reduced image")?;
        k *= 2.0;
        level = Some(next);
    }
    let src = level.as_ref().unwrap_or(src);
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    let inv = map
        .inverse()
        .ok_or("Degenerate perspective")?
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
            // Bilinear sample, relative to texel centres.
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
fn draw_nodes(target: &mut PixmapMut, nodes: &[SceneNode], to_px: Affine) -> Result<(), String> {
    for node in nodes {
        match node {
            SceneNode::Image(l) => {
                let processed;
                let (pixmap, map) = if l.effects.is_empty() {
                    (&l.pixmap, l.map)
                } else {
                    let (pm, pad) = effects::apply_pixmap(&l.pixmap, &l.effects, l.effects_scale)
                        .ok_or("Cannot apply the effects of a layer")?;
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
            }
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
                    .ok_or_else(|| format!("Cannot allocate a {w} x {h} group buffer"))?;
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
    let mut pixmap =
        Pixmap::new(w, h).ok_or_else(|| format!("Cannot allocate a {w} x {h} image"))?;
    let to_px =
        Affine::translate(-origin_mm[0], -origin_mm[1]).then(Affine::scale(px_per_mm, px_per_mm));
    draw_nodes(&mut pixmap.as_mut(), nodes, to_px)?;
    Ok(pixmap)
}

/// Error returned when an export is cancelled.
pub const CANCELLED: &str = "Export cancelled";

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
/// false cancels with the `CANCELLED` error.
pub fn compose_artwork(
    scene: &Scene,
    dpi: f32,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<Pixmap, String> {
    let [w, h] = pixel_size(scene.page_mm, dpi);
    let mut pixmap =
        Pixmap::new(w, h).ok_or_else(|| format!("Cannot allocate a {w} x {h} image"))?;
    let [r, g, b] = scene.background;
    pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, 255));

    let mm_to_px = Affine::scale(dpi / 25.4, dpi / 25.4);
    // One step per top-level node, plus one for the cuts.
    let steps = scene.layers.len() as f32 + 1.0;
    for (i, node) in scene.layers.iter().enumerate() {
        if !progress(i as f32 / steps) {
            return Err(CANCELLED.into());
        }
        draw_nodes(&mut pixmap.as_mut(), std::slice::from_ref(node), mm_to_px)?;
    }
    if !progress((steps - 1.0) / steps) {
        return Err(CANCELLED.into());
    }

    let placed: Vec<PlacedElement> = scene.elements.iter().map(|e| e.placed.clone()).collect();
    if let Some(keep) = keep_mask(&placed, scene.cuts, [0.0, 0.0], [w, h], dpi / 25.4) {
        apply_cuts(&mut pixmap, &keep);
    }
    progress(1.0);
    Ok(pixmap)
}

/// Draws the lines of the template elements marked for printing, at least 1 px wide.
pub fn draw_lines(pixmap: &mut Pixmap, scene: &Scene, dpi: f32) {
    let k = dpi / 25.4;
    let stroke = Stroke {
        width: (OUTLINE_WIDTH_MM * k).max(1.0),
        ..Default::default()
    };
    for element in scene.elements.iter().filter(|e| e.print) {
        let mut paint = Paint::default();
        let [r, g, b] = element.color;
        paint.set_color_rgba8(r, g, b, 255);
        paint.anti_alias = true;
        for poly in &element.placed.polylines {
            let mut pb = PathBuilder::new();
            let mut pts = poly.points.iter();
            let Some(first) = pts.next() else { continue };
            pb.move_to(first[0] * k, first[1] * k);
            for p in pts {
                pb.line_to(p[0] * k, p[1] * k);
            }
            if poly.closed {
                pb.close();
            }
            if let Some(path) = pb.finish() {
                pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
    }
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
    use crate::document::{Group, Layer, Orientation, PageSize};

    /// A 10 x 10 mm square layer of `rgb`, centred at `center`, at 1 px per mm.
    fn square(id: u64, rgb: [u8; 3], center: P2) -> Node {
        let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [10, 10];
        l.transform.center = center;
        l.pixmap = Some(Arc::new(premultiplied_pixmap(&img).unwrap()));
        Node::Layer(l)
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
        let scene = Scene::new(&doc, None, [[0; 3]; 3], [true; 3], Cuts::default());
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
        let scene = Scene::new(&doc, None, [[0; 3]; 3], [true; 3], Cuts::default());
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
        let scene = Scene::new(&doc, None, [[0; 3]; 3], [true; 3], Cuts::default());
        let pm = compose(&scene, 254.0, false).unwrap(); // 10 px per mm
        let blue = |x: u32, y: u32| pm.pixel(x, y).unwrap().red() < 10;
        let white = |x: u32, y: u32| pm.pixel(x, y).unwrap().red() > 245;
        assert!(blue(150, 105) && blue(150, 195), "centre column");
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
        // A 1 px checkerboard squeezed to a fraction of its size averages to grey.
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
            layers: vec![Node::Layer(l)],
            ..Default::default()
        };
        let scene = Scene::new(&doc, None, [[0; 3]; 3], [true; 3], Cuts::default());
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
            placed: PlacedElement { kind, polylines },
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
            layers: scene_nodes(&[Node::Layer(l)], false),
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
        };
        let pm = compose(&scene, 25.4, false).unwrap();
        assert_eq!(pm.pixel(5, 5).unwrap().red(), 0);
        assert_eq!(pm.pixel(25, 25).unwrap().red(), 0);
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
            layers: scene_nodes(&[Node::Layer(l)], false),
            elements: Vec::new(),
            cuts: Cuts::default(),
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
            // A 10 mm blue square centred at (25, 25).
            layers: vec![SceneNode::Image(SceneLayer {
                pixmap: Arc::new(premultiplied_pixmap(&img).unwrap()),
                map: Affine::translate(-5.0, -5.0)
                    .then(Affine::translate(25.0, 25.0))
                    .into(),
                opacity: 1.0,
                effects: Vec::new(),
                effects_scale: 1.0,
            })],
            elements: vec![SceneElement {
                placed: PlacedElement {
                    kind: ElementKind::Outline,
                    polylines: vec![Polyline {
                        points: vec![[60.0, 10.0], [90.0, 10.0], [90.0, 40.0]],
                        closed: true,
                    }],
                },
                color: [255, 0, 0],
                print: true,
            }],
            cuts: Cuts {
                clip_to_outline: false,
                blank_buttons: false,
                blank_cutouts: false,
            },
        };
        let pm = compose(&scene, 254.0, true).unwrap(); // 10 px per mm
        assert_eq!((pm.width(), pm.height()), (1000, 500));
        let px = |x: u32, y: u32| pm.pixel(x, y).unwrap();
        assert_eq!((px(250, 250).red(), px(250, 250).blue()), (0, 255));
        assert_eq!(px(10, 10).red(), 255);
        assert_eq!((px(750, 100).red(), px(750, 100).green()), (255, 0));
    }
}
