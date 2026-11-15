//! Drawing the canvas: the page, layers, group previews, template elements, cut
//! lines, safe area, compare divider and badges.

use super::*;

/// Line colors of `template`'s elements, in its order: each element's own, or its kind's.
pub fn element_colors(template: &crate::template::Template) -> Vec<Color32> {
    template
        .elements
        .iter()
        .map(|e| {
            let [r, g, b] = e.color.unwrap_or(e.kind.default_color());
            Color32::from_rgb(r, g, b)
        })
        .collect()
}

/// The template's lines and their shapes' centers, and the safe-area guide, as the
/// canvas shows them.
pub(super) fn paint_template(
    painter: &egui::Painter,
    elements: &[PlacedElement],
    colors: &[Color32],
    state: &mut CanvasState,
    rect: Rect,
    width_mm: Option<f32>,
) {
    let cam = state.camera;
    for (i, e) in elements.iter().enumerate().filter(|_| state.show_template) {
        if !state.element_show.get(i).copied().unwrap_or(true) {
            continue;
        }
        let color = colors.get(i).copied().unwrap_or_else(|| {
            let [r, g, b] = e.kind.default_color();
            Color32::from_rgb(r, g, b)
        });
        let stroke = Stroke::new(1.2, color);
        let line = Stroke {
            width: width_mm.map_or(stroke.width, |w| w * cam.zoom),
            ..stroke
        };
        for poly in &e.polylines {
            let pts: Vec<Pos2> = poly
                .points
                .iter()
                .map(|&p| cam.to_screen(rect, p))
                .collect();
            painter.add(if poly.closed {
                Shape::closed_line(pts, line)
            } else {
                Shape::line(pts, line)
            });
        }
        // The centers images snap to: small crosses.
        for &a in &e.centers {
            let p = cam.to_screen(rect, a);
            let (h, v) = (vec2(ANCHOR_MARK, 0.0), vec2(0.0, ANCHOR_MARK));
            painter.line_segment([p - h, p + h], stroke);
            painter.line_segment([p - v, p + v], stroke);
        }
    }
    if state.show_template && state.preview_safe_area {
        paint_safe_area(painter, elements, state, rect);
    }
}

/// Puts the compared template's cut preview and safe-area guide in place of the
/// project's, or back.
pub(super) fn swap_compare_caches(state: &mut CanvasState) {
    std::mem::swap(&mut state.cut_preview, &mut state.compare_cut_preview);
    std::mem::swap(&mut state.safe_area, &mut state.compare_safe_area);
}

/// The divider between the two versions of the template at `x`, a round handle in its
/// middle, and what each side shows at the top.
pub(super) fn paint_divider(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    x: f32,
    labels: &[String; 2],
) {
    let visuals = ui.visuals();
    let line = Stroke::new(2.0, Color32::WHITE);
    let shadow = Stroke::new(4.0, Color32::from_black_alpha(90));
    let ends = [pos2(x, rect.top()), pos2(x, rect.bottom())];
    painter.line_segment(ends, shadow);
    painter.line_segment(ends, line);
    let c = pos2(x, rect.center().y);
    painter.circle(c, 12.0, Color32::WHITE, shadow);
    // Arrows both ways on the handle.
    let ink = Color32::from_gray(60);
    for dir in [-1.0, 1.0] {
        let tip = c + vec2(dir * 8.0, 0.0);
        painter.add(Shape::convex_polygon(
            vec![tip, tip - vec2(dir * 5.0, 4.0), tip - vec2(dir * 5.0, -4.0)],
            ink,
            Stroke::NONE,
        ));
    }
    let font = egui::FontId::proportional(12.0);
    for (k, text) in labels.iter().enumerate() {
        let galley = painter.layout_no_wrap(text.clone(), font.clone(), visuals.text_color());
        let size = galley.size() + vec2(12.0, 6.0);
        let min = if k == 0 {
            pos2(x - 10.0 - size.x, rect.top() + 10.0)
        } else {
            pos2(x + 10.0, rect.top() + 10.0)
        };
        let chip = Rect::from_min_size(min, size);
        painter.rect_filled(chip, 4.0, visuals.extreme_bg_color.gamma_multiply(0.9));
        painter.galley(chip.min + vec2(6.0, 3.0), galley, visuals.text_color());
    }
}

/// Draws a textured quad with the given corners on screen.
pub(super) fn paint_quad(
    painter: &egui::Painter,
    tex: egui::TextureId,
    corners: [Pos2; 4],
    opacity: f32,
) {
    let uv = |u: f32, v: f32| {
        let top = corners[0] + (corners[1] - corners[0]) * u;
        let bottom = corners[3] + (corners[2] - corners[3]) * u;
        top + (bottom - top) * v
    };
    paint_grid(painter, tex, 1, uv, opacity);
}

/// Draws a texture through a map from texture coordinates (0..1) to screen, as an
/// `n` x `n` grid of quads.
pub(super) fn paint_grid(
    painter: &egui::Painter,
    tex: egui::TextureId,
    n: usize,
    map: impl Fn(f32, f32) -> Pos2,
    opacity: f32,
) {
    let tint = Color32::WHITE.gamma_multiply(opacity.clamp(0.0, 1.0));
    let mut mesh = Mesh::with_texture(tex);
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            mesh.vertices.push(Vertex {
                pos: map(u, v),
                uv: pos2(u, v),
                color: tint,
            });
        }
    }
    let row = n as u32 + 1;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let k = j * row + i;
            mesh.add_triangle(k, k + 1, k + row + 1);
            mesh.add_triangle(k, k + row + 1, k + row);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws a layer's texture, in perspective if it is distorted.
pub(super) fn paint_layer(
    painter: &egui::Painter,
    cam: Camera,
    rect: Rect,
    l: &Layer,
    tex: egui::TextureId,
    opacity: f32,
) {
    // The texture may reach past the image, where effects such as shadows draw.
    let m: Homography = l.homography().then_affine(cam.affine(rect));
    let [x0, y0, x1, y1] = l.shown_rect();
    let at = |u: f32, v: f32| {
        let p = m.apply([x0 + u * (x1 - x0), y0 + v * (y1 - y0)]);
        pos2(p[0], p[1])
    };
    if !l.transform.is_distorted() {
        let corners = [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)];
        paint_quad(painter, tex, corners, opacity);
    } else {
        paint_grid(painter, tex, WARP_GRID, at, opacity);
    }
}

/// Missing image: its footprint crossed out.
pub(super) fn paint_missing(painter: &egui::Painter, corners: [Pos2; 4]) {
    let stroke = Stroke::new(1.5, Color32::from_rgb(220, 60, 60));
    painter.add(Shape::closed_line(corners.to_vec(), stroke));
    painter.line_segment([corners[0], corners[2]], stroke);
    painter.line_segment([corners[1], corners[3]], stroke);
}

/// Hashes what a group preview depends on, to know when to render it again.
pub(super) fn hash_scene(nodes: &[SceneNode], h: &mut impl Hasher) {
    for n in nodes {
        match n {
            SceneNode::Image(l) => {
                0u8.hash(h);
                (std::sync::Arc::as_ptr(&l.pixmap) as usize).hash(h);
                for v in l.map.m {
                    v.to_bits().hash(h);
                }
                l.opacity.to_bits().hash(h);
                l.blend.hash(h);
            }
            SceneNode::Group {
                opacity,
                blend,
                children,
            } => {
                1u8.hash(h);
                opacity.to_bits().hash(h);
                blend.hash(h);
                children.len().hash(h);
                hash_scene(children, h);
            }
            SceneNode::Clip { base, clipped } => {
                2u8.hash(h);
                hash_scene(std::slice::from_ref(&**base), h);
                clipped.len().hash(h);
                hash_scene(clipped, h);
            }
        }
    }
}

/// Draws the layer tree on the canvas, reusing cached group previews.
pub(super) struct Painting<'a> {
    painter: egui::Painter,
    ctx: egui::Context,
    cam: Camera,
    rect: Rect,
    page: P2,
    /// The page's background color, gradient and pattern.
    background: (
        [u8; 3],
        Option<crate::document::gradient::Gradient>,
        Option<crate::document::pattern::Pattern>,
    ),
    /// Previews of the previous frame; the ones still in use move to `used`.
    cache: &'a mut HashMap<u64, GroupPreview>,
    used: HashMap<u64, GroupPreview>,
}

/// Cache key of the offscreen render of the layers up to the last one in a blend mode.
pub(super) const BLENDED_PREVIEW: u64 = u64::MAX;

impl Painting<'_> {
    /// Draws the layer tree. A layer in another mode than normal mixes with what lies
    /// below it, the page's background included, which drawing each layer on its own
    /// can't do: the layers up to the last such one (and those clipped to it) are
    /// composited offscreen, over the background.
    fn top(&mut self, layers: &[Node]) {
        let Some(mut k) = layers.iter().rposition(Node::has_blend) else {
            self.nodes(layers, 1.0);
            return;
        };
        while layers.get(k + 1).is_some_and(Node::clip) {
            k += 1;
        }
        let below = &layers[..=k];
        let scene = render::scene_nodes(below, true);
        if self.offscreen(BLENDED_PREVIEW, &scene, 1.0, true) {
            self.missing(below);
        } else {
            self.nodes(below, 1.0);
        }
        self.nodes(&layers[k + 1..], 1.0);
    }

    /// Draws nodes under an inherited opacity. Semi-transparent groups with overlapping
    /// children, and nodes with others clipped to them, go through an offscreen render.
    fn nodes(&mut self, nodes: &[Node], opacity: f32) {
        let mut i = 0;
        while i < nodes.len() {
            let start = i;
            i += 1;
            if !nodes[start].clip() {
                while i < nodes.len() && nodes[i].clip() {
                    i += 1;
                }
            }
            if i == start + 1 {
                self.node(&nodes[start], opacity);
                continue;
            }
            let stack = &nodes[start..i];
            let scene = render::scene_nodes(stack, true);
            if self.group(nodes[start].id(), &scene, opacity) {
                self.missing(stack);
            } else {
                // Not clipped rather than not drawn.
                for node in stack {
                    self.node(node, opacity);
                }
            }
        }
    }

    /// Draws one layer or group under an inherited opacity.
    fn node(&mut self, node: &Node, opacity: f32) {
        match node {
            Node::Layer(l) if l.visible => match l.shown_texture() {
                Some(tex) => paint_layer(
                    &self.painter,
                    self.cam,
                    self.rect,
                    l,
                    tex.id(),
                    l.opacity * opacity,
                ),
                None => paint_missing(
                    &self.painter,
                    l.corners().map(|p| self.cam.to_screen(self.rect, p)),
                ),
            },
            Node::Group(g) if g.visible && g.opacity > 0.0 => {
                if g.opacity >= 1.0 {
                    self.nodes(&g.children, opacity);
                    return;
                }
                let scene = render::scene_nodes(&g.children, true);
                if scene.len() <= 1 || !self.group(g.id, &scene, opacity * g.opacity) {
                    // Nothing overlaps within the group: per-layer opacity is exact.
                    self.nodes(&g.children, opacity * g.opacity);
                } else {
                    self.missing(&g.children);
                }
            }
            _ => {}
        }
    }

    /// Footprints of visible layers whose image is missing.
    fn missing(&self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Layer(l) if l.visible && l.shown_texture().is_none() => {
                    paint_missing(
                        &self.painter,
                        l.corners().map(|p| self.cam.to_screen(self.rect, p)),
                    );
                }
                Node::Group(g) if g.visible => self.missing(&g.children),
                _ => {}
            }
        }
    }

    /// Draws a group composited offscreen. Returns false if it could not be rendered.
    fn group(&mut self, id: u64, scene: &[SceneNode], opacity: f32) -> bool {
        self.offscreen(id, scene, opacity, false)
    }

    /// Draws nodes composited offscreen, over the page's background with `on_page`.
    /// Returns false if they could not be rendered.
    fn offscreen(&mut self, id: u64, scene: &[SceneNode], opacity: f32, on_page: bool) -> bool {
        let bounds = render::bounds(scene, Affine::IDENTITY);
        let Some((lo, hi)) = (match (bounds, on_page) {
            (Some((lo, hi)), true) => Some((
                [lo[0].min(0.0), lo[1].min(0.0)],
                [hi[0].max(self.page[0]), hi[1].max(self.page[1])],
            )),
            (None, true) => Some(([0.0, 0.0], self.page)),
            (b, false) => b,
        }) else {
            return true;
        };
        // Content far off the page is not worth the pixels.
        let [pw, ph] = self.page;
        let lo = [lo[0].max(-pw), lo[1].max(-ph)];
        let hi = [hi[0].min(2.0 * pw), hi[1].min(2.0 * ph)];
        if hi[0] <= lo[0] || hi[1] <= lo[1] {
            return true;
        }
        // Resolution follows the zoom in powers of two, so zooming re-renders rarely.
        let screen_px = self.cam.zoom * self.ctx.pixels_per_point();
        let mut px_per_mm = 2f32.powf(screen_px.max(1e-3).log2().ceil());
        let longest = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        px_per_mm = px_per_mm.min(MAX_GROUP_PREVIEW_SIDE / longest);
        let size = [
            ((hi[0] - lo[0]) * px_per_mm).ceil().max(1.0) as u32,
            ((hi[1] - lo[1]) * px_per_mm).ceil().max(1.0) as u32,
        ];
        let mut h = std::collections::hash_map::DefaultHasher::new();
        hash_scene(scene, &mut h);
        px_per_mm.to_bits().hash(&mut h);
        for v in [lo[0], lo[1], hi[0], hi[1]] {
            v.to_bits().hash(&mut h);
        }
        if on_page {
            self.background.0.hash(&mut h);
            format!("{:?}", self.background.1).hash(&mut h);
            pattern_key(self.background.2.as_ref()).hash(&mut h);
        }
        let key = h.finish();

        let preview = match self.cache.remove(&id) {
            Some(p) if p.key == key => p,
            old => {
                let composed = if on_page {
                    render::compose_region_on_page(
                        scene,
                        lo,
                        size,
                        px_per_mm,
                        self.page,
                        self.background.0,
                        self.background.1.as_ref(),
                        self.background.2.as_ref(),
                    )
                } else {
                    render::compose_region(scene, lo, size, px_per_mm)
                };
                let pixmap = match composed {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("group preview: {e}");
                        return false;
                    }
                };
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [size[0] as usize, size[1] as usize],
                    pixmap.data(),
                );
                let texture = match old {
                    Some(mut p) => {
                        p.texture.set(image, egui::TextureOptions::LINEAR);
                        p.texture
                    }
                    None => self.ctx.load_texture(
                        format!("group-{id}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                };
                GroupPreview {
                    key,
                    texture,
                    rect_mm: (lo, hi),
                }
            }
        };
        let (a, b) = preview.rect_mm;
        let corners = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]]
            .map(|p| self.cam.to_screen(self.rect, p));
        paint_quad(&self.painter, preview.texture.id(), corners, opacity);
        self.used.insert(id, preview);
        true
    }
}

/// Covers what is cut out of the panel (outside the outline, inside the button holes and
/// cutouts) with paper, whether the export prints there or not; with `cut_opacity` below
/// 1, what will be cut shows through faintly.
pub(super) fn paint_cuts(
    ctx: &egui::Context,
    painter: &egui::Painter,
    page: P2,
    elements: &[PlacedElement],
    state: &mut CanvasState,
    rect: Rect,
) {
    if !state.preview_cuts || elements.is_empty() {
        state.cut_preview = None;
        return;
    }
    let cam = state.camera;
    // Resolution follows the zoom in powers of two, like group previews.
    let screen_px = cam.zoom * ctx.pixels_per_point();
    let longest = page[0].max(page[1]);
    let px_per_mm = 2f32
        .powf(screen_px.max(1e-3).log2().ceil())
        .min(MAX_GROUP_PREVIEW_SIDE * 2.0 / longest);
    let size = [
        (page[0] * px_per_mm).ceil().max(1.0) as u32,
        (page[1] * px_per_mm).ceil().max(1.0) as u32,
    ];
    // The bleed only when asked for: the cut lines themselves otherwise.
    let cuts = Cuts {
        bleed_mm: if state.preview_bleed {
            state.cuts.bleed_mm
        } else {
            0.0
        },
        ..state.cuts
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cuts.hash(&mut h);
    px_per_mm.to_bits().hash(&mut h);
    hash_elements(elements, &mut h);
    let key = h.finish();
    if state.cut_preview.as_ref().is_none_or(|c| c.key != key) {
        let Some(keep) = crate::render::keep_mask(elements, cuts, [0.0, 0.0], size, px_per_mm)
        else {
            state.cut_preview = None;
            return;
        };
        // Premultiplied white, opaque where everything is cut.
        let rgba: Vec<u8> = keep
            .iter()
            .flat_map(|&k| {
                let a = 255 - k;
                [a, a, a, a]
            })
            .collect();
        let [w, h] = size.map(|v| v as usize);
        let image = egui::ColorImage::from_rgba_premultiplied([w, h], &rgba);
        // Diagonal lines, where it is cut, smoothed at their edges.
        let mut lines = vec![0u8; w * h * 4];
        for (i, (px, &k)) in lines
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&keep)
            .enumerate()
        {
            if k == 255 {
                continue;
            }
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let d = (x + y).rem_euclid(HATCH_PERIOD);
            let off = d.min(HATCH_PERIOD - d) - HATCH_WIDTH / 2.0;
            let line = (0.5 - off).clamp(0.0, 1.0) * (255 - k) as f32 / 255.0;
            // Premultiplied.
            let a = HATCH_COLOR[3] as f32 / 255.0 * line;
            let [r, g, b, _] = HATCH_COLOR.map(|v| (v as f32 * a).round() as u8);
            px.copy_from_slice(&[r, g, b, (a * 255.0).round() as u8]);
        }
        let hatch = egui::ColorImage::from_rgba_premultiplied([w, h], &lines);
        let options = egui::TextureOptions::LINEAR;
        let (texture, hatch) = match state.cut_preview.take() {
            Some(mut c) => {
                c.texture.set(image, options);
                c.hatch.set(hatch, options);
                (c.texture, c.hatch)
            }
            None => (
                ctx.load_texture("template-cuts", image, options),
                ctx.load_texture("template-cuts-hatch", hatch, options),
            ),
        };
        state.cut_preview = Some(CutPreview {
            key,
            texture,
            hatch,
        });
    }
    if let Some(c) = &state.cut_preview {
        let corners =
            [[0.0, 0.0], [page[0], 0.0], page, [0.0, page[1]]].map(|p| cam.to_screen(rect, p));
        paint_quad(painter, c.texture.id(), corners, state.cut_opacity);
        // The hatching shows what is cut away at any opacity.
        paint_quad(painter, c.hatch.id(), corners, 1.0);
    }
}

/// Feeds the template elements' shapes to `h`.
pub(super) fn hash_elements(elements: &[PlacedElement], h: &mut impl Hasher) {
    for e in elements {
        e.kind.hash(h);
        for poly in &e.polylines {
            poly.closed.hash(h);
            for p in &poly.points {
                p[0].to_bits().hash(h);
                p[1].to_bits().hash(h);
            }
        }
    }
}

/// Dashed line along the safe area's edge: `safe_margin_mm` inside the outline, and as
/// far around every button and cutout. Only a guide: it is never printed.
pub(super) fn paint_safe_area(
    painter: &egui::Painter,
    elements: &[PlacedElement],
    state: &mut CanvasState,
    rect: Rect,
) {
    let margin = state.safe_margin_mm;
    if margin <= 0.0 || elements.is_empty() {
        state.safe_area = None;
        return;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    margin.to_bits().hash(&mut h);
    hash_elements(elements, &mut h);
    let key = h.finish();
    if state.safe_area.as_ref().is_none_or(|(k, _)| *k != key) {
        let rings = |keep: &dyn Fn(ElementKind) -> bool| -> Vec<Vec<P2>> {
            elements
                .iter()
                .filter(|e| keep(e.kind))
                .flat_map(|e| &e.polylines)
                .filter(|p| p.closed && p.points.len() > 2)
                .map(|p| p.points.clone())
                .collect()
        };
        let mut guide =
            crate::export::marks::offset(&rings(&|k| k == ElementKind::Outline), -margin);
        guide.extend(crate::export::marks::offset(
            &rings(&|k| k == ElementKind::Holes),
            margin,
        ));
        state.safe_area = Some((key, guide));
    }
    let Some((_, guide)) = &state.safe_area else {
        return;
    };
    let cam = state.camera;
    let stroke = Stroke::new(1.0, SAFE_AREA_COLOR);
    for ring in guide {
        let mut pts: Vec<Pos2> = ring.iter().map(|&p| cam.to_screen(rect, p)).collect();
        if let Some(&first) = pts.first() {
            pts.push(first);
        }
        painter.extend(Shape::dashed_line(&pts, stroke, 6.0, 4.0));
    }
}

/// Draws the whole canvas: page, layers, cut preview, template lines and the selection
/// handles.
pub(super) fn paint(
    ui: &egui::Ui,
    doc: &Document,
    elements: &[PlacedElement],
    selected: Option<u64>,
    state: &mut CanvasState,
    rect: Rect,
    distorting: bool,
) {
    let cam = state.camera;
    let painter = ui.painter_at(rect);
    // Surroundings of the page, matched to the light or dark theme.
    let dark = ui.visuals().dark_mode;
    let (surround, shadow, dim, border) = if dark {
        (45, 100, 160, 110)
    } else {
        (205, 60, 90, 150)
    };
    painter.rect_filled(rect, 0.0, Color32::from_gray(surround));

    let page = Rect::from_min_max(
        cam.to_screen(rect, [0.0, 0.0]),
        cam.to_screen(rect, doc.page_mm()),
    );
    painter.rect_filled(
        page.translate(vec2(5.0, 5.0)),
        0.0,
        Color32::from_black_alpha(shadow),
    );
    let [r, g, b] = doc.background;
    painter.rect_filled(page, 0.0, Color32::from_rgb(r, g, b));
    let filled = doc.background_gradient.is_some()
        || doc
            .background_pattern
            .as_ref()
            .is_some_and(|p| p.tile.is_some());
    match filled {
        true => {
            let texture = background_texture(ui.ctx(), state, doc);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(texture, page, uv, Color32::WHITE);
        }
        false => state.background = None,
    }

    // While comparing, the project's template on one side of the divider (`own`), the
    // other one on the other side (`theirs`).
    let split = state
        .compare
        .as_ref()
        .map(|c| (rect.left() + rect.width() * state.compare_split, c.on_left));
    let left_of = |x: f32| Rect::from_min_max(rect.min, pos2(x, rect.bottom()));
    let right_of = |x: f32| Rect::from_min_max(pos2(x, rect.top()), rect.max);
    let own = split.map_or(
        rect,
        |(x, on_left)| if on_left { right_of(x) } else { left_of(x) },
    );
    let theirs = split.map(|(x, on_left)| if on_left { left_of(x) } else { right_of(x) });
    // In the template editor, the layers are the template compared: on its side too.
    let layers_painter = if doc.edited_template.is_some() {
        painter.with_clip_rect(own)
    } else {
        painter.clone()
    };
    let mut painting = Painting {
        painter: layers_painter,
        ctx: ui.ctx().clone(),
        cam,
        rect,
        page: doc.page_mm(),
        background: (
            doc.background,
            doc.background_gradient.clone(),
            doc.background_pattern.clone(),
        ),
        cache: &mut state.group_previews,
        used: HashMap::new(),
    };
    painting.top(&doc.layers);
    // In the template editor, the centers of the elements' closed shapes, crossed as a
    // project's template shows them.
    if doc.edited_template.is_some() && state.show_template {
        for leaf in doc.leaves().iter().filter(|l| l.visible) {
            let Some(tag) = leaf
                .path
                .first()
                .and_then(|&i| doc.layers.get(i)?.as_group()?.element)
            else {
                continue;
            };
            let [r, g, b] = tag.color;
            let stroke = Stroke::new(1.2, Color32::from_rgb(r, g, b));
            for c in crate::template::edit::layer_centers(leaf.layer) {
                let p = cam.to_screen(rect, c);
                let (h, v) = (vec2(ANCHOR_MARK, 0.0), vec2(0.0, ANCHOR_MARK));
                painting.painter.line_segment([p - h, p + h], stroke);
                painting.painter.line_segment([p - v, p + v], stroke);
            }
        }
    }
    // Previews not drawn this frame are dropped with their textures.
    state.group_previews = painting.used;

    let painter_own = painter.with_clip_rect(own);
    paint_cuts(ui.ctx(), &painter_own, doc.page_mm(), elements, state, rect);
    let compare = state.compare.take();
    // In the template editor, both sides are plain lines: no cut preview on the source.
    if let (Some(theirs), Some(other), None) = (theirs, &compare, &doc.edited_template) {
        swap_compare_caches(state);
        let painter_theirs = painter.with_clip_rect(theirs);
        paint_cuts(
            ui.ctx(),
            &painter_theirs,
            doc.page_mm(),
            &other.elements,
            state,
            rect,
        );
        swap_compare_caches(state);
    }

    // Darken whatever falls outside the page: it will not be printed.
    let dim = Color32::from_black_alpha(dim);
    for r in [
        Rect::from_min_max(rect.min, pos2(rect.max.x, page.min.y)),
        Rect::from_min_max(pos2(rect.min.x, page.max.y), rect.max),
        Rect::from_min_max(pos2(rect.min.x, page.min.y), pos2(page.min.x, page.max.y)),
        Rect::from_min_max(pos2(page.max.x, page.min.y), pos2(rect.max.x, page.max.y)),
    ] {
        if r.is_positive() {
            painter.rect_filled(r, 0.0, dim);
        }
    }
    painter.rect_stroke(
        page,
        0.0,
        Stroke::new(1.0, Color32::from_gray(border)),
        StrokeKind::Outside,
    );

    let colors = state.element_color.clone();
    paint_template(&painter_own, elements, &colors, state, rect, None);
    if let (Some(theirs), Some(other)) = (theirs, &compare) {
        swap_compare_caches(state);
        // In the template editor, as wide as the shapes it is compared with.
        let width = doc
            .edited_template
            .is_some()
            .then_some(crate::template::edit::LINE_WIDTH_MM);
        paint_template(
            &painter.with_clip_rect(theirs),
            &other.elements,
            &other.colors,
            state,
            rect,
            width,
        );
        swap_compare_caches(state);
    }
    if let (Some((x, _)), Some(other)) = (split, &compare) {
        paint_divider(ui, &painter, rect, x, &other.labels);
    }
    state.compare = compare;
    if state.show_guides {
        for (i, g) in doc.guides.iter().enumerate() {
            let width = if state.guide_hover == Some(i) {
                2.0
            } else {
                1.0
            };
            painter.line_segment(guide_line(cam, rect, g), Stroke::new(width, GUIDE_COLOR));
        }
    }
    // The center the dragged image snapped to, shown even when its lines are hidden.
    if let Some(a) = state.snapped {
        let p = cam.to_screen(rect, a);
        painter.circle_stroke(p, ANCHOR_MARK * 1.6, Stroke::new(2.0, ACCENT));
    }
    canvas_frames(doc, &painter, state, rect);
    paint_brush(ui, &painter, doc, selected, state, rect);

    // The guides the dragged layer snapped to, across the whole canvas.
    let guide = Stroke::new(1.0, ACCENT);
    if let Some(x) = state.snapped_guides[0] {
        let x = cam.to_screen(rect, [x, 0.0]).x;
        let line = [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())];
        painter.extend(Shape::dashed_line(&line, guide, 6.0, 4.0));
    }
    if let Some(y) = state.snapped_guides[1] {
        let y = cam.to_screen(rect, [0.0, y]).y;
        let line = [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)];
        painter.extend(Shape::dashed_line(&line, guide, 6.0, 4.0));
    }

    let Some(sel) = selected.filter(|&id| doc.effectively_visible(id)) else {
        return;
    };
    let locked = doc.effectively_locked(sel);
    let color = if locked { Color32::GRAY } else { ACCENT };
    // A handle's fill: the accent under the pointer or while dragged, else white.
    let fill = |hot: Hot| {
        if state.hot == Some(hot) {
            color
        } else {
            Color32::WHITE
        }
    };
    // In the template editor, a shape in no element: an error.
    let orphan = crate::template::edit::is_orphan(doc, sel);
    if let Some(Node::Group(_)) = doc.node(sel) {
        // Group: outline its visible layers and a dashed box around them all.
        let path = doc.path_of(sel).unwrap_or_default();
        let mut pts = Vec::new();
        for leaf in doc.leaves() {
            if leaf.visible && leaf.path.starts_with(&path) {
                let c = leaf.layer.corners().map(|p| cam.to_screen(rect, p));
                painter.add(Shape::closed_line(
                    c.to_vec(),
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                ));
                pts.extend(c);
            }
        }
        if !pts.is_empty() {
            let b = Rect::from_points(&pts).expand(4.0);
            let line = [
                b.left_top(),
                b.right_top(),
                b.right_bottom(),
                b.left_bottom(),
                b.left_top(),
            ];
            painter.extend(Shape::dashed_line(&line, Stroke::new(1.5, color), 6.0, 4.0));
            badges(ui, &painter, b.center(), locked, orphan);
            // Its rotation handle; not in the template editor, whose groups are elements.
            if !locked && doc.edited_template.is_none() {
                let (top_mid, rot) = group_rotate_handle(b);
                let stroke = Stroke::new(1.5, color);
                painter.line_segment([top_mid, rot], stroke);
                painter.circle(rot, HANDLE_RADIUS, fill(Hot::Rotate), stroke);
            }
        }
    } else if let Some(layer) = doc.layer(sel)
        // Its points are being edited or drawn: they are shown instead.
        && state.points.as_ref().is_none_or(|e| e.layer != sel)
        && state.drawing.as_ref().is_none_or(|d| d.layer != Some(sel))
    {
        let (corners, top_mid, rot) = handles(&cam, rect, layer);
        let stroke = Stroke::new(1.5, color);
        painter.add(Shape::closed_line(corners.to_vec(), stroke));
        let c = cam.to_screen(rect, layer.transform.center);
        badges(ui, &painter, c, locked, orphan);
        if !locked {
            painter.line_segment([top_mid, rot], stroke);
            for (i, c) in corners.into_iter().enumerate() {
                if distorting {
                    // Diamonds: each corner moves on its own.
                    let r = HANDLE_RADIUS * 1.3;
                    painter.add(Shape::convex_polygon(
                        vec![
                            c - vec2(0.0, r),
                            c + vec2(r, 0.0),
                            c + vec2(0.0, r),
                            c - vec2(r, 0.0),
                        ],
                        fill(Hot::Corner(i)),
                        stroke,
                    ));
                } else {
                    painter.rect(
                        Rect::from_center_size(c, Vec2::splat(HANDLE_RADIUS * 2.0)),
                        1.0,
                        fill(Hot::Corner(i)),
                        stroke,
                        StrokeKind::Middle,
                    );
                }
            }
            // Edge middles: smaller bars along the edge.
            for (i, c) in edge_handles(&cam, rect, layer).into_iter().enumerate() {
                let along = (corners[(i + 1) % 4] - corners[i]).normalized();
                let along = if along.is_finite() { along } else { Vec2::X };
                let across = along.rot90();
                let (a, b) = (along * HANDLE_RADIUS * 0.9, across * HANDLE_RADIUS * 0.55);
                painter.add(Shape::convex_polygon(
                    vec![c - a - b, c + a - b, c + a + b, c - a + b],
                    fill(Hot::Edge(i)),
                    stroke,
                ));
            }
            painter.circle(rot, HANDLE_RADIUS, fill(Hot::Rotate), stroke);
        }
    }
}

/// Longest side of the page background's gradient as drawn on screen, in pixels: a
/// gradient is smooth, so a small texture stretched over the page shows it well.
pub(super) const BACKGROUND_SIDE: f32 = 512.0;

/// The same for a pattern, whose tiles have details.
pub(super) const PATTERN_BACKGROUND_SIDE: f32 = 4096.0;

/// Tells a pattern's settings and image from others, for the textures drawn from it.
pub(super) fn pattern_key(p: Option<&crate::document::pattern::Pattern>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(p) = p {
        serde_json::to_string(p).unwrap_or_default().hash(&mut h);
        p.tile
            .as_ref()
            .map(|t| std::sync::Arc::as_ptr(t) as usize)
            .hash(&mut h);
    }
    h.finish()
}

/// The page background's gradient as a texture the page's shape, drawn again when the
/// gradient, the color or the page changes.
pub(super) fn background_texture(
    ctx: &egui::Context,
    state: &mut CanvasState,
    doc: &Document,
) -> egui::TextureId {
    let page = doc.page_mm();
    let pattern = doc.background_pattern.as_ref();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&doc.background_gradient)
            .unwrap_or_default()
            .hash(&mut h);
        pattern_key(pattern).hash(&mut h);
        doc.background.hash(&mut h);
        page.map(f32::to_bits).hash(&mut h);
        h.finish()
    };
    if let Some((k, t)) = &state.background
        && *k == key
    {
        return t.id();
    }
    // A pattern's tiles need more pixels than a gradient.
    let side = if pattern.is_some() {
        PATTERN_BACKGROUND_SIDE
    } else {
        BACKGROUND_SIDE
    };
    let k = side / page[0].max(page[1]).max(1.0);
    let (w, h) = (
        (page[0] * k).round().max(1.0) as u32,
        (page[1] * k).round().max(1.0) as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("a small pixmap");
    crate::render::fill_background(
        &mut pixmap,
        doc.background,
        doc.background_gradient.as_ref(),
        pattern,
        k,
    );
    let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    let texture = ctx.load_texture("page-background", image, egui::TextureOptions::LINEAR);
    let id = texture.id();
    state.background = Some((key, texture));
    id
}

/// Radius of the badge showing that the selected layer is locked.
pub(super) const LOCK_BADGE: f32 = 18.0;

/// The badges in the middle of the selection on screen, at `c`: the padlock when
/// `locked`, the error mark when `orphan` (a template shape in no element), side by side
/// when both.
pub(super) fn badges(ui: &egui::Ui, painter: &egui::Painter, c: Pos2, locked: bool, orphan: bool) {
    let d = if locked && orphan {
        LOCK_BADGE + 3.0
    } else {
        0.0
    };
    if locked {
        lock_badge(ui, painter, c - vec2(d, 0.0));
    }
    if orphan {
        error_badge(ui, painter, c + vec2(d, 0.0));
    }
}

/// A white exclamation point on a red disc at `c`, the size of the padlock badge, with a
/// tooltip saying the shape is in no element.
pub(super) fn error_badge(ui: &egui::Ui, painter: &egui::Painter, c: Pos2) {
    painter.circle_filled(
        c + vec2(0.0, 1.5),
        LOCK_BADGE + 1.5,
        Color32::from_black_alpha(60),
    );
    let [r, g, b] = crate::template::edit::ORPHAN_COLOR;
    painter.circle(
        c,
        LOCK_BADGE,
        Color32::from_rgb(r, g, b),
        Stroke::new(2.0, Color32::WHITE),
    );
    let ink = Color32::WHITE;
    painter.line_segment(
        [c + vec2(0.0, -10.0), c + vec2(0.0, 3.0)],
        Stroke::new(3.6, ink),
    );
    painter.circle_filled(c + vec2(0.0, 9.0), 2.2, ink);
    let badge = Rect::from_center_size(c, Vec2::splat(LOCK_BADGE * 2.0));
    ui.interact(
        badge,
        ui.scope_id().with("error_badge"),
        egui::Sense::hover(),
    )
    .on_hover_text(crate::i18n::tr!("TEMPLATE_ORPHAN_SHAPE"));
}

/// A white padlock on a dark disc at `c` (the middle of the selection on screen), seen on
/// any artwork, with a tooltip saying why the layer does not move.
pub(super) fn lock_badge(ui: &egui::Ui, painter: &egui::Painter, c: Pos2) {
    // A soft shadow, then the disc and its white ring.
    painter.circle_filled(
        c + vec2(0.0, 1.5),
        LOCK_BADGE + 1.5,
        Color32::from_black_alpha(60),
    );
    painter.circle(
        c,
        LOCK_BADGE,
        Color32::from_black_alpha(185),
        Stroke::new(2.0, Color32::WHITE),
    );
    let ink = Color32::WHITE;
    // Body in the lower half; the shackle, an arch, above it.
    let body = Rect::from_min_max(c + vec2(-8.0, -1.0), c + vec2(8.0, 10.0));
    painter.rect_filled(body, 2.5, ink);
    // The keyhole.
    let dark = Color32::from_black_alpha(185);
    painter.circle_filled(c + vec2(0.0, 3.5), 1.8, dark);
    painter.rect_filled(
        Rect::from_center_size(c + vec2(0.0, 6.0), vec2(1.6, 3.5)),
        0.5,
        dark,
    );
    let r = 5.0;
    let top = c + vec2(0.0, -6.0);
    let mut shackle = vec![c + vec2(-r, -1.0)];
    shackle.extend((0..=16).map(|i| {
        let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
        top + vec2(r * a.cos(), r * a.sin())
    }));
    shackle.push(c + vec2(r, -1.0));
    painter.add(Shape::line(shackle, Stroke::new(2.6, ink)));
    let badge = Rect::from_center_size(c, Vec2::splat(LOCK_BADGE * 2.0));
    ui.interact(
        badge,
        ui.scope_id().with("lock_badge"),
        egui::Sense::hover(),
    )
    .on_hover_text(crate::i18n::tr!("LAYER_LOCKED_HINT"));
}
