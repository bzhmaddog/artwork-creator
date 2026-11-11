//! SVG writer: the same page as the PDF, for vector editors and cutting machines. Images
//! are embedded as PNG, text and shapes kept as paths (text as outlines), gradients as SVG
//! gradients. The artwork, each template element's lines and the marks go in layers of
//! their own (Inkscape layers, groups elsewhere), so the cut lines can be picked apart.

use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;
use tiny_skia::{FillRule, PathSegment};

use crate::document::{Affine, Homography, P2};
use crate::gradient::{Geometry as GradientGeometry, Gradient, Ink};
use crate::i18n::{tr, translate};
use crate::print;
use crate::render::{self, OUTLINE_WIDTH_MM, Part, Scene};
use crate::template::Polyline;

/// Number of straight pieces a curve becomes when it has to be bent by a perspective.
const CURVE_PIECES: usize = 8;

/// A length or coordinate in mm, to the micrometer.
fn num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// An affine map as an SVG `matrix(...)`, at full precision: its scales may be small.
fn matrix(m: Affine) -> String {
    format!("matrix({} {} {} {} {} {})", m.a, m.b, m.c, m.d, m.e, m.f)
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Text for an XML attribute or element.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn rule(r: FillRule) -> &'static str {
    match r {
        FillRule::Winding => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}

/// A path's outlines as SVG path data in page mm, mapped by `map` (path units to page
/// mm). Curves stay curves under an affine map; under a perspective, which bends them
/// otherwise, they become short straight pieces.
fn path_data(path: &tiny_skia::Path, map: Homography) -> String {
    let affine = map.as_affine().is_some();
    let at = |p: tiny_skia::Point| {
        let [x, y] = map.apply([p.x, p.y]);
        format!("{} {}", num(x), num(y))
    };
    let mut d = String::new();
    let mut last = tiny_skia::Point::zero();
    let cubic = |d: &mut String, p0: tiny_skia::Point, c1, c2, p: tiny_skia::Point| {
        if affine {
            let _ = write!(d, "C{} {} {}", at(c1), at(c2), at(p));
            return;
        }
        for i in 1..=CURVE_PIECES {
            let t = i as f32 / CURVE_PIECES as f32;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            let q = tiny_skia::Point::from_xy(
                w[0] * p0.x + w[1] * c1.x + w[2] * c2.x + w[3] * p.x,
                w[0] * p0.y + w[1] * c1.y + w[2] * c2.y + w[3] * p.y,
            );
            let _ = write!(d, "L{}", at(q));
        }
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                let _ = write!(d, "M{}", at(p));
                last = p;
            }
            PathSegment::LineTo(p) => {
                let _ = write!(d, "L{}", at(p));
                last = p;
            }
            PathSegment::QuadTo(c, p) => {
                // The same curve as a cubic one.
                let toward = |a: tiny_skia::Point| {
                    tiny_skia::Point::from_xy(
                        a.x + (c.x - a.x) * 2.0 / 3.0,
                        a.y + (c.y - a.y) * 2.0 / 3.0,
                    )
                };
                cubic(&mut d, last, toward(last), toward(p), p);
                last = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                cubic(&mut d, last, c1, c2, p);
                last = p;
            }
            PathSegment::Close => d.push('Z'),
        }
    }
    d
}

/// Closed outlines in page mm as SVG path data.
fn rings_data(rings: &[Vec<P2>]) -> String {
    let mut d = String::new();
    for ring in rings {
        for (i, p) in ring.iter().enumerate() {
            let _ = write!(
                d,
                "{}{} {}",
                if i == 0 { 'M' } else { 'L' },
                num(p[0]),
                num(p[1])
            );
        }
        if !ring.is_empty() {
            d.push('Z');
        }
    }
    d
}

/// A PNG of the pixels, recording `dpi`, as a data URL. Opaque RGB for the page, straight
/// RGBA for what goes over it.
fn png_data_url(pixmap: &tiny_skia::Pixmap, opaque: bool, dpi: f32) -> Result<String, String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mut bytes = Vec::new();
    let mut enc = png::Encoder::new(&mut bytes, pixmap.width(), pixmap.height());
    enc.set_depth(png::BitDepth::Eight);
    let ppm = (dpi / 0.0254).round() as u32;
    enc.set_pixel_dims(Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    }));
    let data = if opaque {
        enc.set_color(png::ColorType::Rgb);
        render::to_rgb(pixmap)
    } else {
        enc.set_color(png::ColorType::Rgba);
        crate::effects::cache::demultiply(pixmap).into_raw()
    };
    let mut writer = enc.write_header().map_err(|e| err(&e))?;
    writer.write_image_data(&data).map_err(|e| err(&e))?;
    writer.finish().map_err(|e| err(&e))?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// The document being written: definitions (clips, gradients) and drawing apart, so a
/// definition can be added while drawing.
#[derive(Default)]
struct Writer {
    defs: String,
    body: String,
    next_id: usize,
}

impl Writer {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    /// A clip path of path data `d`, filled under `rule`; its id.
    fn clip(&mut self, d: &str, rule: &str) -> String {
        let id = self.id("clip");
        let _ = writeln!(
            self.defs,
            r#"<clipPath id="{id}"><path d="{d}" clip-rule="{rule}"/></clipPath>"#
        );
        id
    }

    /// A gradient laid out in a layer's units, `map` taking them to page mm; its id.
    fn gradient(&mut self, gradient: &Gradient, frame: tiny_skia::Rect, map: Affine) -> String {
        let id = self.id("gradient");
        let stops: String = gradient
            .sorted()
            .iter()
            .map(|s| {
                let [r, g, b, a] = s.color;
                let opacity = if a < 255 {
                    format!(r#" stop-opacity="{}""#, a as f32 / 255.0)
                } else {
                    String::new()
                };
                format!(
                    r#"<stop offset="{}" stop-color="{}"{opacity}/>"#,
                    s.pos,
                    hex([r, g, b])
                )
            })
            .collect();
        let _ = match gradient.geometry(frame) {
            GradientGeometry::Linear { from, to } => writeln!(
                self.defs,
                r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" gradientTransform="{}" x1="{}" y1="{}" x2="{}" y2="{}">{stops}</linearGradient>"#,
                matrix(map),
                from[0],
                from[1],
                to[0],
                to[1]
            ),
            // A circle of radius 1 around the origin, stretched into the ellipse.
            GradientGeometry::Radial { ellipse: e } => {
                let ellipse = Affine {
                    a: e.sx,
                    b: e.ky,
                    c: e.kx,
                    d: e.sy,
                    e: e.tx,
                    f: e.ty,
                };
                writeln!(
                    self.defs,
                    r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" gradientTransform="{}" cx="0" cy="0" r="1">{stops}</radialGradient>"#,
                    matrix(ellipse.then(map))
                )
            }
        };
        id
    }

    /// Opens a layer: an Inkscape layer, a group elsewhere, moved by `shift` (in mm) and
    /// clipped to `clip` if given.
    fn open_layer(&mut self, label: &str, shift: Option<(P2, &str)>) {
        let _ = write!(
            self.body,
            r#"<g inkscape:groupmode="layer" inkscape:label="{}""#,
            escape(label)
        );
        if let Some(([x, y], clip)) = shift {
            let _ = write!(
                self.body,
                r#" transform="translate({} {})" clip-path="url(#{clip})""#,
                num(x),
                num(y)
            );
        }
        self.body.push_str(">\n");
    }

    fn close_layer(&mut self) {
        self.body.push_str("</g>\n");
    }

    /// Lines in page mm, stroked in `color`.
    fn lines<'a>(
        &mut self,
        polylines: impl Iterator<Item = &'a Polyline>,
        color: [u8; 3],
        width_mm: f32,
    ) {
        let _ = writeln!(
            self.body,
            r#"<g fill="none" stroke="{}" stroke-width="{}" stroke-linejoin="round" stroke-linecap="round">"#,
            hex(color),
            num(width_mm)
        );
        for poly in polylines {
            let points: Vec<String> = poly
                .points
                .iter()
                .map(|p| format!("{},{}", num(p[0]), num(p[1])))
                .collect();
            let tag = if poly.closed { "polygon" } else { "polyline" };
            let _ = writeln!(self.body, r#"<{tag} points="{}"/>"#, points.join(" "));
        }
        self.body.push_str("</g>\n");
    }
}

/// The page's artwork: the parts bottom first (images, and text and shapes as paths), cut
/// by the template.
fn artwork(w: &mut Writer, scene: &Scene, parts: &[Part], dpi: f32) -> Result<(), String> {
    let mm_per_px = 25.4 / dpi;
    // The first part is the page already cut. What goes over it is cut here: clipped to
    // the outline, and blank paper painted over the holes.
    let (outline, holes) = if parts.len() > 1 {
        render::cut_shapes(scene)
    } else {
        Default::default()
    };
    let mut clipped = false;
    for (i, part) in parts.iter().enumerate() {
        match part {
            Part::Raster { pixmap, origin } => {
                let _ = writeln!(
                    w.body,
                    r#"<image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" xlink:href="{}"/>"#,
                    num(origin[0] as f32 * mm_per_px),
                    num(origin[1] as f32 * mm_per_px),
                    num(pixmap.width() as f32 * mm_per_px),
                    num(pixmap.height() as f32 * mm_per_px),
                    png_data_url(pixmap, i == 0, dpi)?
                );
            }
            Part::Vector(layer) => {
                let Some(v) = layer.vector() else { continue };
                // The strokes painted inside a shape are cut off at its outline.
                let clip = v
                    .clip
                    .as_ref()
                    .map(|(c, range, r)| (w.clip(&path_data(c, v.map), rule(*r)), range.clone()));
                for (i, (path, ink, r)) in v.fills.iter().enumerate() {
                    if let Some((id, range)) = &clip {
                        if i == range.start {
                            let _ = writeln!(w.body, r#"<g clip-path="url(#{id})">"#);
                        }
                        if i == range.end {
                            w.body.push_str("</g>\n");
                        }
                    }
                    let d = path_data(path, v.map);
                    if d.is_empty() {
                        continue;
                    }
                    let opacity = layer.opacity.clamp(0.0, 1.0);
                    let (fill, alpha) = match ink {
                        Ink::Solid(c) => (hex([c[0], c[1], c[2]]), c[3] as f32 / 255.0 * opacity),
                        Ink::Gradient { gradient, frame } => {
                            let Some(map) = v.map.as_affine() else {
                                continue;
                            };
                            (
                                format!("url(#{})", w.gradient(gradient, *frame, map)),
                                opacity,
                            )
                        }
                    };
                    let alpha = if alpha < 1.0 {
                        format!(r#" fill-opacity="{}""#, num(alpha))
                    } else {
                        String::new()
                    };
                    let _ = writeln!(
                        w.body,
                        r#"<path d="{d}" fill="{fill}"{alpha} fill-rule="{}"/>"#,
                        rule(*r)
                    );
                }
                if clip.is_some_and(|(_, r)| r.end >= v.fills.len()) {
                    w.body.push_str("</g>\n");
                }
            }
        }
        if i == 0 && !outline.is_empty() {
            let id = w.clip(&rings_data(&outline), "nonzero");
            let _ = writeln!(w.body, r#"<g clip-path="url(#{id})">"#);
            clipped = true;
        }
    }
    if clipped {
        w.body.push_str("</g>\n");
    }
    if !holes.is_empty() {
        let _ = writeln!(
            w.body,
            r##"<path d="{}" fill="#ffffff" fill-rule="nonzero"/>"##,
            rings_data(&holes)
        );
    }
    Ok(())
}

/// An SVG file of `size_mm`: the page's artwork and lines in layers, moved by `shift`
/// (mm) and clipped to `clip` (page mm) when the page is split over sheets, then the
/// sheet's own marks.
fn document(
    scene: &Scene,
    parts: &[Part],
    dpi: f32,
    size_mm: P2,
    sheet: Option<Sheet>,
) -> Result<String, String> {
    let mut w = Writer::default();
    let shift = match &sheet {
        Some((shift, (min, max), _)) => {
            let d = rings_data(&[vec![*min, [max[0], min[1]], *max, [min[0], max[1]]]]);
            Some((*shift, w.clip(&d, "nonzero")))
        }
        None => None,
    };
    let shift = || shift.as_ref().map(|(s, c)| (*s, c.as_str()));
    w.open_layer(tr!("SVG_ARTWORK_LAYER"), shift());
    artwork(&mut w, scene, parts, dpi)?;
    w.close_layer();
    for element in scene.elements.iter().filter(|e| e.print) {
        let lines: Vec<_> = element.placed.printed_lines().collect();
        w.open_layer(translate(element.placed.kind.label()), shift());
        w.lines(
            lines.iter().map(|l| l.as_ref()),
            element.color,
            OUTLINE_WIDTH_MM,
        );
        w.close_layer();
    }
    if !scene.marks.is_empty() {
        w.open_layer(tr!("EXPORT_MARKS"), shift());
        w.lines(scene.marks.iter(), print::MARK_COLOR, print::MARK_WIDTH_MM);
        w.close_layer();
    }
    if let Some((_, _, marks)) = &sheet
        && !marks.is_empty()
    {
        w.open_layer(tr!("EXPORT_TILE_MARKS"), None);
        w.lines(marks.iter(), print::MARK_COLOR, print::MARK_WIDTH_MM);
        w.close_layer();
    }
    let [sw, sh] = size_mm;
    let mut out = String::new();
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" width="{0}mm" height="{1}mm" viewBox="0 0 {0} {1}">"#,
        num(sw),
        num(sh)
    );
    let _ = writeln!(out, "<title>{}</title>", escape(tr!("PDF_DOCUMENT_TITLE")));
    if !w.defs.is_empty() {
        let _ = write!(out, "<defs>\n{}</defs>\n", w.defs);
    }
    out.push_str(&w.body);
    out.push_str("</svg>\n");
    Ok(out)
}

/// How the page goes onto `tile`'s sheet: page point (x, y) lands at
/// (x - ox + x0, y - oy + y0) on it, only the tile's artwork (in page mm) shows, then the
/// sheet's marks.
type Sheet = (P2, (P2, P2), Vec<Polyline>);

fn sheet(tiling: &print::Tiling, tile: print::Tile, page_mm: P2) -> Sheet {
    let ([x0, y0], _) = tiling.content_box();
    let [ox, oy] = tile.origin_mm;
    let [cw, ch] = tiling.content_mm();
    (
        [x0 - ox, y0 - oy],
        ([ox, oy], [ox + cw, oy + ch]),
        tiling.marks(tile, page_mm),
    )
}

/// Saves the page as an SVG at `path`: the whole page, or with `tile` the sheet of
/// `tiling` that prints it.
pub fn write(
    scene: &Scene,
    parts: &[Part],
    dpi: f32,
    path: &Path,
    tile: Option<(&print::Tiling, print::Tile)>,
) -> Result<(), String> {
    let svg = match tile {
        None => document(scene, parts, dpi, scene.page_mm, None)?,
        Some((tiling, tile)) => {
            let sheet = sheet(tiling, tile, scene.page_mm);
            document(scene, parts, dpi, tiling.sheet_mm, Some(sheet))?
        }
    };
    std::fs::write(path, svg).map_err(|e| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::tests::text_scene;

    /// The SVG of a scene, as exported at 100 DPI.
    fn svg_of(scene: &Scene) -> String {
        let parts = render::compose_parts(scene, 100.0, true, &mut |_| true).unwrap();
        document(scene, &parts, 100.0, scene.page_mm, None).unwrap()
    }

    fn parse(svg: &str) -> usvg::Tree {
        usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap()
    }

    #[test]
    fn svg_keeps_text_as_paths_over_images() {
        let scene = text_scene(false);
        let svg = svg_of(&scene);
        // The page and the square over the text as images, the letters as curves.
        assert_eq!(svg.matches("<image ").count(), 2);
        let letters = svg
            .lines()
            .find(|l| l.starts_with("<path ") && l.contains("#c80000"))
            .expect("the red letters");
        assert!(letters.contains('C'), "curves kept");
        // Cut by the outline, the holes blanked: 5 buttons and 2 fastener holes.
        assert!(svg.contains(r#"<g clip-path="url(#clip"#));
        let holes = svg
            .lines()
            .find(|l| l.contains(r##"fill="#ffffff""##))
            .unwrap();
        assert_eq!(holes.matches('M').count(), 7);
        // A layer for the artwork and for each template element printed.
        assert!(svg.contains(r#"inkscape:label="Artwork""#));
        assert!(svg.contains(r#"inkscape:label="Outline""#));
        assert!(svg.contains("<polygon "));
        // Valid, and at true size: A4 landscape, 96 px per inch.
        let tree = parse(&svg);
        let size = tree.size();
        assert!((size.width() - 297.0 / 25.4 * 96.0).abs() < 0.5, "{size:?}");
        assert!((size.height() - 210.0 / 25.4 * 96.0).abs() < 0.5);

        // In perspective, curves become straight pieces.
        let svg = svg_of(&text_scene(true));
        let letters = svg
            .lines()
            .find(|l| l.starts_with("<path ") && l.contains("#c80000"))
            .unwrap();
        assert!(!letters.contains('C'));
    }

    #[test]
    fn svg_gradients_and_painted_strokes() {
        use crate::document::{Document, Node};
        use crate::gradient::GradientKind;
        use crate::shape::{self, ShapeKind};
        let mut doc = Document::default();
        let c = doc.page_center();
        let mut rect = shape::new_layer(1, ShapeKind::Rectangle, c);
        rect.transform.rotation_deg = 30.0;
        {
            let s = rect.shape.as_mut().unwrap();
            s.stroke = false;
            let mut g = Gradient::from_color([255, 0, 0, 255]);
            g.kind = GradientKind::Radial;
            s.fill_gradient = Some(g);
        }
        let mut ellipse = shape::new_layer(2, ShapeKind::Ellipse, c);
        let line = [[c[0] - 30.0, c[1], 1.0], [c[0] + 30.0, c[1], 1.0]];
        let pen = crate::paint::Pen::new(crate::paint::Tool::Pencil, 2.0, [0, 0, 255, 255]);
        assert!(shape::add_stroke(&mut ellipse, &line, &pen));
        doc.layers = vec![Node::Layer(Box::new(rect)), Node::Layer(Box::new(ellipse))];
        shape::prepare_export(&mut doc, 100.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            render::Cuts::default(),
        );
        let svg = svg_of(&scene);
        // A sample to look at beside the PNG, when asked for.
        if let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") {
            let settings = crate::export::ExportSettings {
                dpi: 100,
                svg: true,
                pdf: false,
                ..Default::default()
            };
            let base = std::path::PathBuf::from(dir).join("gradient-and-paint");
            crate::export::run(&scene, &settings, &base, &Default::default()).unwrap();
        }
        assert_eq!(svg.matches("<radialGradient ").count(), 1);
        assert!(svg.contains(r#"fill="url(#gradient"#));
        // The paint, inside a group clipped to the ellipse, between its fill and stroke.
        let paint = svg.find("#0000ff").unwrap();
        let open = svg.find(r#"<g clip-path="url(#clip"#).unwrap();
        assert!(open < paint && svg[paint..].contains("</g>"));
        // Valid, the gradient filling a path.
        fn walk(g: &usvg::Group, n: &mut usize) {
            for node in g.children() {
                match node {
                    usvg::Node::Group(g) => walk(g, n),
                    usvg::Node::Path(p) => {
                        if matches!(
                            p.fill().map(|f| f.paint()),
                            Some(usvg::Paint::RadialGradient(_))
                        ) {
                            *n += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut gradients = 0;
        walk(parse(&svg).root(), &mut gradients);
        assert_eq!(gradients, 1);
    }

    #[test]
    fn tiled_svg_moves_the_page_onto_each_sheet() {
        let scene = text_scene(false);
        let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
        let tiling = print::Tiling {
            sheet_mm: [210.0, 297.0],
            margin_mm: 10.0,
            overlap_mm: 10.0,
            marks: true,
            offset_mm: [0.0, 0.0],
        };
        let tiles = tiling.tiles(scene.page_mm);
        assert_eq!(tiles.len(), 2);
        let tile = tiles[1];
        let ([x0, y0], _) = tiling.content_box();
        let shift = [x0 - tile.origin_mm[0], y0 - tile.origin_mm[1]];
        let svg = document(
            &scene,
            &parts,
            100.0,
            tiling.sheet_mm,
            Some(sheet(&tiling, tile, scene.page_mm)),
        )
        .unwrap();
        let moved = format!("translate({} {})", num(shift[0]), num(shift[1]));
        assert!(svg.contains(&moved), "{moved}");
        assert!(svg.contains(r#"inkscape:label="Alignment marks""#));
        assert!(svg.contains(r#"width="210mm" height="297mm""#));
        parse(&svg);
    }
}
