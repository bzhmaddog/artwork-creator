//! PNG and PDF writers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use printpdf::{
    Color, ExtendedGraphicsState, Line, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage,
    PdfSaveOptions, Point, Polygon, PolygonRing, Pt, RawImage, RawImageData, RawImageFormat, Rgb,
    WindingOrder, XObjectTransform,
};
use tiny_skia::PathSegment;

use crate::document::{Homography, P2};
use crate::i18n::tr;
use crate::render::{self, Cuts, OUTLINE_WIDTH_MM, Part, Scene};
use crate::template::{ElementKind, KINDS};

/// Options of the Export window.
#[derive(Clone, Debug)]
pub struct ExportSettings {
    /// Write a PNG, with the cut lines drawn in.
    pub png: bool,
    /// Write a PDF, with the cut lines as vectors.
    pub pdf: bool,
    /// Raster resolution, for both formats.
    pub dpi: u32,
    /// Print the lines of each element kind (`ElementKind::index`).
    pub print_lines: [bool; KINDS],
    /// Clip to the outline and leave holes blank.
    pub cuts: Cuts,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            png: true,
            pdf: true,
            dpi: 300,
            // Anchors are construction marks: printed only when asked for.
            print_lines: ElementKind::ALL.map(|k| k != ElementKind::Anchors),
            cuts: Cuts::default(),
        }
    }
}

/// Progress of a running export, shared between its thread and the UI.
#[derive(Clone, Default)]
pub struct Progress(Arc<Mutex<ProgressState>>);

#[derive(Clone, Default)]
struct ProgressState {
    /// 0 to 1.
    fraction: f32,
    /// What is being done, e.g. "Rendering".
    stage: String,
    cancel: bool,
}

impl Progress {
    fn state(&self) -> std::sync::MutexGuard<'_, ProgressState> {
        // A panic in the export thread must not take the UI down with it.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, stage: &str, fraction: f32) {
        let mut s = self.state();
        s.stage = stage.to_owned();
        s.fraction = fraction.clamp(0.0, 1.0);
    }

    /// Fraction done (0 to 1) and the current stage.
    pub fn get(&self) -> (f32, String) {
        let s = self.state();
        (s.fraction, s.stage.clone())
    }

    /// Asks the export to stop at the next step.
    pub fn cancel(&self) {
        self.state().cancel = true;
    }

    pub fn is_canceled(&self) -> bool {
        self.state().cancel
    }
}

/// Share of the progress bar taken by rendering; saving the files takes the rest.
const RENDER_SHARE: f32 = 0.6;

/// Writes the requested formats next to each other, using `base` with its extension
/// replaced. The page is rendered once and shared by both formats. On cancel, files
/// already written are removed.
pub fn run(
    scene: &Scene,
    settings: &ExportSettings,
    base: &Path,
    progress: &Progress,
) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    let result = write_all(scene, settings, base, progress, &mut written);
    if result.is_err() {
        for path in &written {
            std::fs::remove_file(path).ok();
        }
    }
    result.map(|()| written)
}

fn write_all(
    scene: &Scene,
    settings: &ExportSettings,
    base: &Path,
    progress: &Progress,
    written: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let dpi = settings.dpi.clamp(72, 1200) as f32;
    progress.set(tr!("EXPORT_STEP_RENDERING"), 0.0);
    // The PDF keeps text and shapes as vectors: the rest is rendered in parts around them.
    let parts = render::compose_parts(scene, dpi, settings.pdf, &mut |f| {
        progress.set(tr!("EXPORT_STEP_RENDERING"), f * RENDER_SHARE);
        !progress.is_canceled()
    })?;
    let saves = settings.png as u32 + settings.pdf as u32;
    let mut done = 0;
    let mut step = |stage: &str| -> Result<(), String> {
        if progress.is_canceled() {
            return Err(render::CANCELED.into());
        }
        let share = (1.0 - RENDER_SHARE) / saves.max(1) as f32;
        progress.set(stage, RENDER_SHARE + share * done as f32);
        done += 1;
        Ok(())
    };
    if settings.png {
        step(tr!("EXPORT_STEP_SAVING_PNG"))?;
        let path = base.with_extension("png");
        let mut pixmap = render::flatten(&parts, scene, dpi)?;
        render::draw_lines(&mut pixmap, scene, dpi);
        written.push(path.clone());
        write_png(&pixmap, dpi, &path)?;
    }
    if settings.pdf {
        step(tr!("EXPORT_STEP_SAVING_PDF"))?;
        let path = base.with_extension("pdf");
        // Text and cut lines go in as vectors so they stay sharp at any zoom.
        written.push(path.clone());
        write_pdf(scene, &parts, dpi, &path)?;
    }
    progress.set(tr!("EXPORT_STEP_DONE"), 1.0);
    Ok(())
}

/// Saves an opaque RGB PNG that records its DPI, so it prints at true size.
fn write_png(pixmap: &tiny_skia::Pixmap, dpi: f32, path: &Path) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e);
    let file = std::fs::File::create(path).map_err(|e| err(&e))?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        pixmap.width(),
        pixmap.height(),
    );
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let ppm = (dpi / 0.0254).round() as u32;
    enc.set_pixel_dims(Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    }));
    let mut writer = enc.write_header().map_err(|e| err(&e))?;
    writer
        .write_image_data(&render::to_rgb(pixmap))
        .map_err(|e| err(&e))?;
    writer.finish().map_err(|e| err(&e))
}

/// Millimeters to PDF points (1/72 inch).
fn mm_to_pt(mm: f32) -> f32 {
    mm * 72.0 / 25.4
}

/// Page mm to PDF points, y up from the bottom of a page `page_h` mm tall.
fn to_pdf(page_h: f32, p: P2) -> Point {
    Point {
        x: Pt(mm_to_pt(p[0])),
        y: Pt(mm_to_pt(page_h - p[1])),
    }
}

/// Number of straight pieces a curve becomes when it has to be bent by a perspective.
const CURVE_PIECES: usize = 8;

/// A path's contours mapped onto the PDF page by `map` (path units to page mm). Curves
/// stay curves under an affine map; under a perspective, which bends them otherwise,
/// they become short straight pieces (straight lines stay straight under both).
fn rings(path: &tiny_skia::Path, map: Homography, page_h: f32) -> Vec<PolygonRing> {
    let affine = map.as_affine().is_some();
    let at = |p: tiny_skia::Point| to_pdf(page_h, map.apply([p.x, p.y]));
    let point = |p, bezier| LinePoint { p, bezier };
    let mut out: Vec<PolygonRing> = Vec::new();
    let mut last = tiny_skia::Point::zero();
    for seg in path.segments() {
        let mut push = |points: Vec<LinePoint>| {
            if let Some(ring) = out.last_mut() {
                ring.points.extend(points);
            }
        };
        match seg {
            PathSegment::MoveTo(p) => {
                out.push(PolygonRing {
                    points: vec![point(at(p), false)],
                });
                last = p;
            }
            PathSegment::LineTo(p) => {
                push(vec![point(at(p), false)]);
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
                let (c1, c2) = (toward(last), toward(p));
                push(curve(last, c1, c2, p, affine, &at));
                last = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                push(curve(last, c1, c2, p, affine, &at));
                last = p;
            }
            PathSegment::Close => {}
        }
    }
    out.retain(|r| r.points.len() > 2);
    out
}

/// A cubic curve's points after its start: control points and end, or straight pieces.
fn curve(
    p0: tiny_skia::Point,
    c1: tiny_skia::Point,
    c2: tiny_skia::Point,
    p: tiny_skia::Point,
    affine: bool,
    at: &dyn Fn(tiny_skia::Point) -> Point,
) -> Vec<LinePoint> {
    if affine {
        return vec![
            LinePoint {
                p: at(c1),
                bezier: true,
            },
            LinePoint {
                p: at(c2),
                bezier: true,
            },
            LinePoint {
                p: at(p),
                bezier: false,
            },
        ];
    }
    (1..=CURVE_PIECES)
        .map(|i| {
            let t = i as f32 / CURVE_PIECES as f32;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            let q = tiny_skia::Point::from_xy(
                w[0] * p0.x + w[1] * c1.x + w[2] * c2.x + w[3] * p.x,
                w[0] * p0.y + w[1] * c1.y + w[2] * c2.y + w[3] * p.y,
            );
            LinePoint {
                p: at(q),
                bezier: false,
            }
        })
        .collect()
}

/// A shape on the page from closed outlines in page mm.
fn polygon(shapes: &[Vec<P2>], page_h: f32, mode: PaintMode) -> Polygon {
    Polygon {
        rings: shapes
            .iter()
            .map(|pts| PolygonRing {
                points: pts
                    .iter()
                    .map(|&p| LinePoint {
                        p: to_pdf(page_h, p),
                        bezier: false,
                    })
                    .collect(),
            })
            .collect(),
        mode,
        winding_order: WindingOrder::NonZero,
    }
}

fn rgb(c: [u8; 3]) -> Color {
    Color::Rgb(Rgb::new(
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        None,
    ))
}

/// The page's drawing: the parts bottom first (images, and text and shapes as filled
/// shapes), cut by the template, then the printed template lines.
fn page_ops(doc: &mut PdfDocument, scene: &Scene, parts: &[Part], dpi: f32) -> Vec<Op> {
    let page_h = scene.page_mm[1];
    let page_h_pt = mm_to_pt(page_h);
    let px_to_pt = 72.0 / dpi;
    let mut ops = Vec::new();
    // The first part is the page already cut. What goes over it is cut here: clipped to
    // the outline, and blank paper painted over the holes.
    let (outline, holes) = if parts.len() > 1 {
        render::cut_shapes(scene)
    } else {
        Default::default()
    };
    ops.push(Op::SaveGraphicsState);
    for (i, part) in parts.iter().enumerate() {
        match part {
            Part::Raster { pixmap, origin } => {
                let image = if i == 0 {
                    RawImage {
                        pixels: RawImageData::U8(render::to_rgb(pixmap)),
                        width: pixmap.width() as usize,
                        height: pixmap.height() as usize,
                        data_format: RawImageFormat::RGB8,
                        tag: Vec::new(),
                    }
                } else {
                    // Straight RGBA: the parts above the page keep their transparency.
                    let rgba = crate::effects::cache::demultiply(pixmap);
                    RawImage {
                        pixels: RawImageData::U8(rgba.into_raw()),
                        width: pixmap.width() as usize,
                        height: pixmap.height() as usize,
                        data_format: RawImageFormat::RGBA8,
                        tag: Vec::new(),
                    }
                };
                let id = doc.add_image(&image);
                // Placed by its bottom-left corner, sized by its DPI.
                let bottom = origin[1] + pixmap.height();
                ops.push(Op::UseXobject {
                    id,
                    transform: XObjectTransform {
                        translate_x: Some(Pt(origin[0] as f32 * px_to_pt)),
                        translate_y: Some(Pt(page_h_pt - bottom as f32 * px_to_pt)),
                        dpi: Some(dpi),
                        ..Default::default()
                    },
                });
                if i == 0 && !outline.is_empty() {
                    ops.push(Op::DrawPolygon {
                        polygon: polygon(&outline, page_h, PaintMode::Clip),
                    });
                }
            }
            Part::Vector(layer) => {
                let Some(v) = layer.vector() else { continue };
                for (path, c) in &v.fills {
                    let rings = rings(path, v.map, page_h);
                    if rings.is_empty() {
                        continue;
                    }
                    let alpha = c[3] as f32 / 255.0 * layer.opacity.clamp(0.0, 1.0);
                    ops.push(Op::SaveGraphicsState);
                    if alpha < 1.0 {
                        let gs = ExtendedGraphicsState::default().with_current_fill_alpha(alpha);
                        ops.push(Op::LoadGraphicsState {
                            gs: doc.add_graphics_state(gs),
                        });
                    }
                    ops.push(Op::SetFillColor {
                        col: rgb([c[0], c[1], c[2]]),
                    });
                    ops.push(Op::DrawPolygon {
                        polygon: Polygon {
                            rings,
                            mode: PaintMode::Fill,
                            winding_order: WindingOrder::NonZero,
                        },
                    });
                    ops.push(Op::RestoreGraphicsState);
                }
            }
        }
    }
    if !holes.is_empty() {
        ops.push(Op::SetFillColor {
            col: rgb([255, 255, 255]),
        });
        ops.push(Op::DrawPolygon {
            polygon: polygon(&holes, page_h, PaintMode::Fill),
        });
    }
    ops.push(Op::RestoreGraphicsState);

    for element in scene.elements.iter().filter(|e| e.print) {
        ops.push(Op::SetOutlineColor {
            col: rgb(element.color),
        });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(mm_to_pt(OUTLINE_WIDTH_MM)),
        });
        for poly in element.placed.printed_lines() {
            let points = poly
                .points
                .iter()
                .map(|&p| LinePoint {
                    p: to_pdf(page_h, p),
                    bezier: false,
                })
                .collect();
            ops.push(Op::DrawLine {
                line: Line {
                    points,
                    is_closed: poly.closed,
                },
            });
        }
    }
    ops
}

/// Saves a one-page PDF: the artwork as images, with its text and shape layers and the
/// printed template lines on top as vectors.
fn write_pdf(scene: &Scene, parts: &[Part], dpi: f32, path: &Path) -> Result<(), String> {
    let mut doc = PdfDocument::new(tr!("PDF_DOCUMENT_TITLE"));
    let ops = page_ops(&mut doc, scene, parts, dpi);
    let page = PdfPage::new(Mm(scene.page_mm[0]), Mm(scene.page_mm[1]), ops);
    let bytes = doc
        .with_pages(vec![page])
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    std::fs::write(path, bytes).map_err(|e| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::template;
    use printpdf::Polygon;

    /// A page with the test template, a photo-like square, a text layer over it and a
    /// square over part of the text, as exported at 100 DPI.
    fn text_scene(distort: bool) -> Scene {
        use crate::document::{Layer, Node};
        use crate::text;
        let entry = template::tests::fixture();
        let tpl = template::load(&entry).unwrap();
        let mut doc = Document::default();
        doc.template_placement = entry.default_placement(tpl.size_mm, doc.page_mm());
        let c = doc.page_center();
        let square = |id: u64, rgb: [u8; 3], center: P2| {
            let img =
                image::RgbaImage::from_pixel(40, 40, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
            let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
            l.size_px = [40, 40];
            l.transform.center = center;
            l.pixmap = Some(std::sync::Arc::new(
                render::premultiplied_pixmap(&img).unwrap(),
            ));
            Node::Layer(Box::new(l))
        };
        let mut t = text::new_layer(2, "Start", text::BUILTIN_FAMILY.into(), c);
        let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        t.text.as_mut().unwrap().color = [200, 0, 0, 255];
        t.size_px = text::layout(t.text.as_ref().unwrap(), &font).size;
        t.transform.scale = t.transform.scale.map(|s| s * 3.0);
        if distort {
            t.transform.distort[1] = [0.1, 0.2];
        }
        doc.layers = vec![
            square(1, [0, 90, 200], c),
            Node::Layer(Box::new(t)),
            square(3, [0, 160, 0], [c[0] + 30.0, c[1]]),
        ];
        text::prepare_export(&mut doc, 100.0);
        Scene::new(
            &doc,
            Some(&tpl),
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        )
    }

    #[test]
    fn pdf_keeps_text_as_vectors() {
        let scene = text_scene(false);
        let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
        assert_eq!(parts.len(), 3);
        let mut doc = PdfDocument::new("test");
        let ops = page_ops(&mut doc, &scene, &parts, 100.0);
        let polygons: Vec<&Polygon> = ops
            .iter()
            .filter_map(|op| match op {
                Op::DrawPolygon { polygon } => Some(polygon),
                _ => None,
            })
            .collect();
        // The outline clips, the letters are filled with curves, the holes are blanked.
        assert_eq!(polygons[0].mode, PaintMode::Clip);
        assert_eq!(polygons[1].mode, PaintMode::Fill);
        assert!(polygons[1].rings.len() >= 5, "a ring per letter at least");
        assert!(
            polygons[1]
                .rings
                .iter()
                .flat_map(|r| &r.points)
                .any(|p| p.bezier)
        );
        assert_eq!(polygons.len(), 3);
        assert_eq!(polygons[2].rings.len(), 7, "5 buttons and 2 fastener holes");
        let images = ops
            .iter()
            .filter(|op| matches!(op, Op::UseXobject { .. }))
            .count();
        assert_eq!(images, 2, "the page and the square over the text");
        // The letters sit around the page center, in points from the bottom left.
        let c = scene.page_mm.map(|v| mm_to_pt(v / 2.0));
        for p in polygons[1].rings.iter().flat_map(|r| &r.points) {
            assert!((p.p.x.0 - c[0]).abs() < 120.0 && (p.p.y.0 - c[1]).abs() < 60.0);
        }

        // In perspective, curves become straight pieces.
        let scene = text_scene(true);
        let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
        let ops = page_ops(&mut doc, &scene, &parts, 100.0);
        let letters = ops
            .iter()
            .find_map(|op| match op {
                Op::DrawPolygon { polygon } if polygon.mode == PaintMode::Fill => Some(polygon),
                _ => None,
            })
            .unwrap();
        assert!(
            letters
                .rings
                .iter()
                .flat_map(|r| &r.points)
                .all(|p| !p.bezier)
        );

        // A sample to look at, when asked for.
        if let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") {
            let dir = std::path::PathBuf::from(dir);
            for (name, distort) in [("text", false), ("text-distorted", true)] {
                let settings = ExportSettings {
                    dpi: 100,
                    ..Default::default()
                };
                run(
                    &text_scene(distort),
                    &settings,
                    &dir.join(name),
                    &Progress::default(),
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn pdf_keeps_shapes_as_vectors() {
        use crate::document::Node;
        use crate::shape::{self, ShapeKind};
        let mut doc = Document::default();
        let c = doc.page_center();
        let mut star = shape::new_layer(1, ShapeKind::Star, c);
        star.shape.as_mut().unwrap().fill_color = [200, 0, 0, 128];
        let mut line = shape::new_layer(2, ShapeKind::Line, c);
        line.shape.as_mut().unwrap().dash = shape::DashStyle::Dashed;
        doc.layers = vec![Node::Layer(Box::new(star)), Node::Layer(Box::new(line))];
        shape::prepare_export(&mut doc, 100.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
        assert_eq!(parts.len(), 3, "the page, then both shapes as vectors");
        let mut pdf = PdfDocument::new("test");
        let ops = page_ops(&mut pdf, &scene, &parts, 100.0);
        let polygons: Vec<&Polygon> = ops
            .iter()
            .filter_map(|op| match op {
                Op::DrawPolygon { polygon } => Some(polygon),
                _ => None,
            })
            .collect();
        // The star's fill and stroke, then the line's dashes, one ring each.
        assert_eq!(polygons.len(), 3);
        assert!(polygons.iter().all(|p| p.mode == PaintMode::Fill));
        assert_eq!(polygons[0].rings.len(), 1);
        assert!(polygons[2].rings.len() >= 5, "{}", polygons[2].rings.len());
        // The semi-transparent fill keeps its opacity.
        assert!(
            ops.iter()
                .any(|op| matches!(op, Op::LoadGraphicsState { .. }))
        );

        // A sample of every kind and stroke style to look at, when asked for.
        if let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") {
            let mut doc = Document::default();
            let mut id = 0;
            for (row, kind) in ShapeKind::ALL.into_iter().enumerate() {
                for (col, dash) in shape::DashStyle::ALL.into_iter().enumerate() {
                    id += 1;
                    let at = [40.0 + col as f32 * 65.0, 15.0 + row as f32 * 25.0];
                    let mut l = shape::new_layer(id, kind, at);
                    let s = l.shape.as_mut().unwrap();
                    s.dash = dash;
                    s.stroke_width = 1.5;
                    s.corner_radius = 4.0 * col as f32;
                    s.cap = shape::Cap::ALL[col % 3];
                    s.join = shape::Join::ALL[col % 3];
                    s.fill_color[3] = 255 - 60 * col as u8;
                    shape::set_geometry_size(&mut l, [50.0, 20.0]);
                    doc.layers.push(Node::Layer(Box::new(l)));
                }
            }
            // Curves: an ellipse turned into a polygon, a polyline with smooth points.
            let mut l = shape::new_layer(id + 1, ShapeKind::Ellipse, [40.0, 200.0]);
            shape::set_kind(&mut l, ShapeKind::Polygon);
            doc.layers.push(Node::Layer(Box::new(l)));
            let mut l = shape::new_layer(id + 2, ShapeKind::Polyline, [110.0, 200.0]);
            l.shape.as_mut().unwrap().stroke_width = 1.5;
            shape::toggle_smooth(&mut l, 1);
            shape::toggle_smooth(&mut l, 2);
            doc.layers.push(Node::Layer(Box::new(l)));
            shape::prepare_export(&mut doc, 150.0);
            let scene = Scene::new(
                &doc,
                None,
                [[255, 0, 255]; crate::template::KINDS],
                [true; crate::template::KINDS],
                Cuts::default(),
            );
            let settings = ExportSettings {
                dpi: 150,
                ..Default::default()
            };
            let base = std::path::PathBuf::from(dir).join("shapes");
            run(&scene, &settings, &base, &Progress::default()).unwrap();
        }
    }

    /// With ARTWORK_PDF_SAMPLE set, writes text on paths there to look at: around a
    /// circle, under it on the other side, on an arc, and along a curved polyline.
    #[test]
    fn text_on_path_sample() {
        use crate::document::Node;
        use crate::shape::{self, ShapeKind};
        use crate::text::{self, PathKind};
        let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") else {
            return;
        };
        let mut doc = Document::default();
        let mut ring = shape::new_layer(1, ShapeKind::Ellipse, [70.0, 70.0]);
        ring.shape.as_mut().unwrap().fill = false;
        let mut curve = shape::new_layer(2, ShapeKind::Polyline, [200.0, 150.0]);
        shape::set_geometry_size(&mut curve, [120.0, 40.0]);
        shape::toggle_smooth(&mut curve, 1);
        shape::toggle_smooth(&mut curve, 2);
        let mut layers = vec![Node::Layer(Box::new(ring)), Node::Layer(Box::new(curve))];
        let mut add = |id: u64, words: &str, at: P2, set: &dyn Fn(&mut text::TextPath)| {
            let mut t = text::new_layer(id, words, text::BUILTIN_FAMILY.into(), at);
            set(&mut t.text.as_mut().unwrap().path);
            layers.push(Node::Layer(Box::new(t)));
        };
        // An em is 8.5 mm at 24 pt: a little more than the 15 mm ring's radius.
        add(3, "PLAYER ONE START", [70.0, 70.0], &|p| {
            p.kind = PathKind::Circle;
            p.radius = 2.0;
        });
        add(4, "SELECT", [70.0, 70.0], &|p| {
            p.kind = PathKind::Circle;
            p.radius = 2.0;
            p.flip = true;
            p.position = 50.0;
            p.shift = -0.7;
        });
        add(5, "ARCADE STICK", [200.0, 60.0], &|p| {
            p.kind = PathKind::Arc;
            p.bend = 30.0;
        });
        add(6, "Along a curved polyline", [0.0, 0.0], &|p| {
            p.kind = PathKind::Shape;
            p.shape = 2;
            p.position = 50.0;
            p.shift = 0.3;
        });
        doc.layers = layers;
        let ctx = egui::Context::default();
        let fonts = text::FontLibrary::builtin_only();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                shape::sync(ui.ctx(), &mut doc, true);
                text::sync(ui.ctx(), &mut doc, &fonts);
            });
            out.textures_delta.clear();
        }
        text::prepare_export(&mut doc, 150.0);
        shape::prepare_export(&mut doc, 150.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let settings = ExportSettings {
            dpi: 150,
            ..Default::default()
        };
        let base = std::path::PathBuf::from(dir).join("text-on-path");
        run(&scene, &settings, &base, &Progress::default()).unwrap();
    }

    #[test]
    fn exports_png_and_pdf_with_template_outline() {
        let dir = std::env::temp_dir().join(format!("artwork-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = template::tests::fixture();
        let tpl = template::load(&entry).unwrap();
        let mut doc = Document::default();
        doc.template_placement = entry.default_placement(tpl.size_mm, doc.page_mm());
        let scene = Scene::new(
            &doc,
            Some(&tpl),
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let settings = ExportSettings {
            dpi: 100,
            ..Default::default()
        };
        let progress = Progress::default();
        let out = run(&scene, &settings, &dir.join("art"), &progress).unwrap();
        assert_eq!(progress.get(), (1.0, "Done".to_owned()));
        assert_eq!(out.len(), 2);

        let png = image::open(&out[0]).unwrap();
        assert_eq!((png.width(), png.height()), (1169, 827)); // A4 landscape at 100 DPI

        let pdf = std::fs::read(&out[1]).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        // 297 x 210 mm in points.
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("841.8") && text.contains("595.2"),
            "page box should be A4 landscape"
        );

        // A canceled export fails and leaves no files behind.
        let canceled = Progress::default();
        canceled.cancel();
        let err = run(&scene, &settings, &dir.join("gone"), &canceled).unwrap_err();
        assert_eq!(err, render::CANCELED);
        assert!(!dir.join("gone.png").exists() && !dir.join("gone.pdf").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
