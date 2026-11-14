//! PNG and PDF writers, and the export of every format (SVG in `svg_export`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use printpdf::{
    Color, ExtendedGraphicsState, Line, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfPage,
    PdfSaveOptions, Point, Polygon, PolygonRing, Pt, RawImage, RawImageData, RawImageFormat, Rgb,
    WindingOrder, XObjectTransform,
};
use tiny_skia::PathSegment;

use crate::document::{Affine, Homography, Orientation, P2, PageSize};
use crate::gradient::{Geometry as GradientGeometry, Gradient, Ink};
use crate::i18n::tr;
use crate::print;
use crate::render::{self, Cuts, OUTLINE_WIDTH_MM, Part, Scene};
use crate::template::{KINDS, Polyline};

/// How the artwork is exported and printed. Two parts: what to export (the formats and
/// the template lines printed), chosen in the Export and Print windows and saved with the
/// project; and the print settings (the rest), set in the Print panel and the same for
/// every project, saved with the app's preferences.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    /// Write a PNG, with the cut lines drawn in.
    pub png: bool,
    /// Write a PDF, with the cut lines as vectors.
    pub pdf: bool,
    /// Write an SVG, like the PDF, the cut lines in layers of their own.
    pub svg: bool,
    /// Raster resolution, for both formats.
    pub dpi: u32,
    /// Print the lines of each element kind (`ElementKind::index`).
    #[serde(deserialize_with = "print_lines_saved")]
    pub print_lines: [bool; KINDS],
    /// Clip to the outline and leave holes blank. Its bleed is `bleed_mm`.
    pub cuts: Cuts,
    /// How far the artwork goes past the template's cut lines, in mm.
    pub bleed_mm: f32,
    /// Print crop marks at the corners of the outline's box, outside the bleed.
    pub crop_marks: bool,
    /// Print registration targets at the middle of each side of the outline's box.
    pub registration_marks: bool,
    /// Split the page over several sheets of `sheet`.
    pub tile: bool,
    /// Paper the tiles are printed on (not `PageSize::Custom`).
    pub sheet: PageSize,
    pub sheet_orientation: Orientation,
    /// Blank border of each sheet, in mm.
    pub sheet_margin_mm: f32,
    /// Artwork shared by neighboring sheets, in mm.
    pub tile_overlap_mm: f32,
    /// Crop marks and trim ticks in each sheet's margin.
    pub tile_marks: bool,
    /// Where the sheets sit, in mm from the grid centered on the page.
    pub tile_offset_mm: P2,
    /// Mirror the page left to right, to print on the back of clear film or plexi.
    pub mirror: bool,
    /// Image layers printed below this resolution (DPI) are flagged.
    pub min_image_dpi: u32,
}

impl ExportSettings {
    /// These print settings with the choices of what to export of `choices`.
    pub fn with_choices_of(&self, choices: &ExportSettings) -> Self {
        Self {
            png: choices.png,
            pdf: choices.pdf,
            svg: choices.svg,
            print_lines: choices.print_lines,
            ..self.clone()
        }
    }

    /// Both choose to export the same: the part saved with a project.
    pub fn same_choices(&self, other: &ExportSettings) -> bool {
        (self.png, self.pdf, self.svg, self.print_lines)
            == (other.png, other.pdf, other.svg, other.print_lines)
    }

    /// The cuts, their bleed included.
    pub fn cuts(&self) -> Cuts {
        Cuts {
            bleed_mm: self.bleed_mm,
            ..self.cuts
        }
    }

    /// How the page is split over sheets, if it is.
    pub fn tiling(&self) -> Option<print::Tiling> {
        let [w, h] = self.sheet.portrait_mm()?;
        self.tile.then_some(print::Tiling {
            sheet_mm: match self.sheet_orientation {
                Orientation::Portrait => [w, h],
                Orientation::Landscape => [h, w],
            },
            margin_mm: self.sheet_margin_mm,
            overlap_mm: self.tile_overlap_mm,
            marks: self.tile_marks,
            offset_mm: self.tile_offset_mm,
        })
    }

    /// The files an export of a `page_mm` page to `base` writes: a PNG and an SVG per
    /// sheet when tiled, and one PDF.
    pub fn files(&self, base: &Path, page_mm: P2) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let tiles = self.tiling().map(|t| t.tiles(page_mm));
        let mut per_sheet = |ext: &str| match &tiles {
            Some(tiles) => out.extend(tiles.iter().map(|&tile| tile_path(base, tile, ext))),
            None => out.push(base.with_extension(ext)),
        };
        if self.png {
            per_sheet("png");
        }
        if self.svg {
            per_sheet("svg");
        }
        if self.pdf {
            out.push(base.with_extension("pdf"));
        }
        out
    }

    /// Some format is chosen.
    pub fn any_format(&self) -> bool {
        self.png || self.pdf || self.svg
    }
}

/// The file of one sheet: the base name followed by its row and column, e.g.
/// "art-r1c2.png".
fn tile_path(base: &Path, tile: print::Tile, ext: &str) -> PathBuf {
    let stem = base.file_stem().unwrap_or_default().to_string_lossy();
    base.with_file_name(format!("{stem}-r{}c{}.{ext}", tile.row + 1, tile.col + 1))
}

/// `print_lines` as saved, also by a version that had more element kinds.
fn print_lines_saved<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[bool; KINDS], D::Error> {
    use serde::Deserialize;
    let saved = Vec::<bool>::deserialize(d)?;
    Ok(crate::template::per_kind(&saved, [true; KINDS]))
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            png: true,
            pdf: true,
            svg: false,
            dpi: 300,
            print_lines: [true; KINDS],
            cuts: Cuts::default(),
            bleed_mm: 0.0,
            crop_marks: false,
            registration_marks: false,
            tile: false,
            sheet: PageSize::A4,
            sheet_orientation: Orientation::Portrait,
            sheet_margin_mm: 10.0,
            tile_overlap_mm: 10.0,
            tile_marks: true,
            tile_offset_mm: [0.0, 0.0],
            mirror: false,
            min_image_dpi: 150,
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

    pub(crate) fn set(&self, stage: &str, fraction: f32) {
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
/// replaced. The page is rendered once and shared by all formats. On cancel, files
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
    // The PDF and SVG keep text and shapes as vectors: the rest is rendered in parts
    // around them.
    let vectors = settings.pdf || settings.svg;
    let parts = render::compose_parts(scene, dpi, vectors, &mut |f| {
        progress.set(tr!("EXPORT_STEP_RENDERING"), f * RENDER_SHARE);
        !progress.is_canceled()
    })?;
    let tiling = settings.tiling();
    let tiles = tiling.map(|t| t.tiles(scene.page_mm)).unwrap_or_default();
    let per_sheet = tiles.len().max(1) as u32;
    let saves = (settings.png as u32 + settings.svg as u32) * per_sheet + settings.pdf as u32;
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
        for_each_sheet(&parts, scene, settings, dpi, &mut |tile, pixmap| {
            step(tr!("EXPORT_STEP_SAVING_PNG"))?;
            let path = match tile {
                Some(tile) => tile_path(base, tile, "png"),
                None => base.with_extension("png"),
            };
            written.push(path.clone());
            write_png(pixmap, dpi, &path)
        })?;
    }
    if settings.svg {
        let sheets: Vec<_> = match &tiling {
            Some(t) => tiles.iter().map(|&tile| Some((t, tile))).collect(),
            None => vec![None],
        };
        for sheet in sheets {
            step(tr!("EXPORT_STEP_SAVING_SVG"))?;
            let path = match sheet {
                Some((_, tile)) => tile_path(base, tile, "svg"),
                None => base.with_extension("svg"),
            };
            written.push(path.clone());
            crate::svg_export::write(scene, &parts, dpi, &path, sheet)?;
        }
    }
    if settings.pdf {
        step(tr!("EXPORT_STEP_SAVING_PDF"))?;
        let path = base.with_extension("pdf");
        // Text and cut lines go in as vectors so they stay sharp at any zoom.
        written.push(path.clone());
        write_pdf(scene, &parts, dpi, &path, tiling)?;
    }
    progress.set(tr!("EXPORT_STEP_DONE"), 1.0);
    Ok(())
}

/// Called with each piece of paper's picture, and its tile when the page is split.
pub(crate) type SheetFn<'a> =
    dyn FnMut(Option<print::Tile>, &tiny_skia::Pixmap) -> Result<(), String> + 'a;

/// What goes on each piece of paper, from the parts rendered at `dpi`: the page with its
/// lines and marks, or when split, each sheet with its tile. `each` gets them one at a
/// time, so only one sheet is held at once.
pub(crate) fn for_each_sheet(
    parts: &[Part],
    scene: &Scene,
    settings: &ExportSettings,
    dpi: f32,
    each: &mut SheetFn,
) -> Result<(), String> {
    let mut page = render::flatten(parts, scene, dpi)?;
    match settings.tiling() {
        None => {
            render::draw_lines(&mut page, scene, dpi);
            each(None, &page)
        }
        Some(t) => {
            // Sheets draw the lines themselves: marks may lie past the page, on a sheet.
            for tile in t.tiles(scene.page_mm) {
                let sheet = sheet_pixmap(&page, scene, &t, tile, dpi)?;
                each(Some(tile), &sheet)?;
            }
            Ok(())
        }
    }
}

/// One sheet of a tiled print: white paper, the part of the rendered `page` (without
/// lines) that falls on the tile within the margins, the template lines and the marks
/// around the outline over the whole of that area, even past the page, and the sheet's
/// own marks.
fn sheet_pixmap(
    page: &tiny_skia::Pixmap,
    scene: &Scene,
    tiling: &print::Tiling,
    tile: print::Tile,
    dpi: f32,
) -> Result<tiny_skia::Pixmap, String> {
    let page_mm = scene.page_mm;
    let [w, h] = render::pixel_size(tiling.sheet_mm, dpi);
    let mut sheet =
        tiny_skia::Pixmap::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h))?;
    sheet.fill(tiny_skia::Color::WHITE);
    let k = dpi / 25.4;
    let px = |mm: f32| (mm * k).round() as i64;
    let ([mx, my], _) = tiling.content_box();
    let [cw, ch] = tiling.content_mm().map(px);
    let [ox, oy] = tile.origin_mm.map(px);
    // The tile's artwork, cut to what the page has: the outer sheets reach past it.
    let (x0, y0) = (ox.max(0), oy.max(0));
    let rw = (ox + cw).min(page.width() as i64) - x0;
    let rh = (oy + ch).min(page.height() as i64) - y0;
    if rw > 0
        && rh > 0
        && let Some(rect) =
            tiny_skia::IntRect::from_xywh(x0 as i32, y0 as i32, rw as u32, rh as u32)
        && let Some(part) = page.clone_rect(rect)
    {
        sheet.draw_pixmap(
            (px(mx) + x0 - ox) as i32,
            (px(my) + y0 - oy) as i32,
            part.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
    }
    // The page's lines, moved onto the sheet and kept to its artwork area.
    let ([cx0, cy0], [cx1, cy1]) = tiling.content_box();
    let mut clip =
        tiny_skia::Mask::new(w, h).ok_or_else(|| tr!("CANNOT_ALLOCATE_IMAGE", w = w, h = h))?;
    if let Some(rect) = tiny_skia::Rect::from_ltrb(cx0 * k, cy0 * k, cx1 * k, cy1 * k) {
        clip.fill_path(
            &tiny_skia::PathBuilder::from_rect(rect),
            tiny_skia::FillRule::Winding,
            false,
            tiny_skia::Transform::identity(),
        );
    }
    let shift = [mx - tile.origin_mm[0], my - tile.origin_mm[1]];
    render::draw_lines_at(&mut sheet, scene, dpi, shift, Some(&clip));
    render::stroke_polylines(
        &mut sheet,
        tiling.marks(tile, page_mm).iter(),
        (print::MARK_COLOR, print::MARK_WIDTH_MM),
        dpi,
        [0.0, 0.0],
        None,
    );
    Ok(sheet)
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

/// The PDF fill rule matching tiny-skia's.
fn winding(rule: tiny_skia::FillRule) -> WindingOrder {
    match rule {
        tiny_skia::FillRule::Winding => WindingOrder::NonZero,
        tiny_skia::FillRule::EvenOdd => WindingOrder::EvenOdd,
    }
}

/// Millimeters to PDF points (1/72 inch).
fn mm_to_pt(mm: f32) -> f32 {
    mm * 72.0 / 25.4
}

/// A shape filled with a gradient: clipped to its outline (`rings`, in PDF points, under
/// the fill rule `order`), then a PDF shading painted in the layer's own units through
/// its map. Only for an opaque gradient on a layer that is not distorted (see
/// `SceneLayer::vector`).
#[allow(clippy::too_many_arguments)]
fn gradient_ops(
    doc: &mut PdfDocument,
    rings: Vec<PolygonRing>,
    order: WindingOrder,
    gradient: &Gradient,
    frame: tiny_skia::Rect,
    map: Homography,
    opacity: f32,
    page_h: f32,
) -> Vec<Op> {
    let Some(a) = map.as_affine() else {
        return Vec::new();
    };
    let mut ops = vec![Op::SaveGraphicsState];
    if opacity < 1.0 {
        let gs = ExtendedGraphicsState::default().with_current_fill_alpha(opacity.max(0.0));
        ops.push(Op::LoadGraphicsState {
            gs: doc.add_graphics_state(gs),
        });
    }
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings,
            mode: PaintMode::Clip,
            winding_order: order,
        },
    });
    // Layer units to page mm (the layer's map), then to PDF points with y up.
    let k = mm_to_pt(1.0);
    let to_pt = a.then(Affine {
        a: k,
        b: 0.0,
        c: 0.0,
        d: -k,
        e: 0.0,
        f: mm_to_pt(page_h),
    });
    let raw = |m: Affine| printpdf::CurTransMat::Raw([m.a, m.b, m.c, m.d, m.e, m.f]);
    ops.push(Op::SetTransformationMatrix { matrix: raw(to_pt) });
    let geometry = match gradient.geometry(frame) {
        GradientGeometry::Linear { from, to } => printpdf::ShadingGeometry::Axial {
            coords: [from[0], from[1], to[0], to[1]],
        },
        // A circle of radius 1 around the origin, stretched into the ellipse.
        GradientGeometry::Radial { ellipse: e } => {
            ops.push(Op::SetTransformationMatrix {
                matrix: raw(Affine {
                    a: e.sx,
                    b: e.ky,
                    c: e.kx,
                    d: e.sy,
                    e: e.tx,
                    f: e.ty,
                }),
            });
            printpdf::ShadingGeometry::Radial {
                coords: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            }
        }
    };
    let stops = gradient
        .sorted()
        .iter()
        .map(|s| printpdf::GradientStop {
            offset: s.pos,
            color: [0, 1, 2].map(|i| s.color[i] as f32 / 255.0),
        })
        .collect();
    let id = doc.add_shading(printpdf::Shading {
        geometry,
        stops,
        extend: (true, true),
    });
    ops.push(Op::PaintShading { id });
    ops.push(Op::RestoreGraphicsState);
    ops
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
                // The strokes painted inside a shape are cut off at its outline.
                let clip = v.clip.as_ref().map(|(c, range, rule)| {
                    (rings(c, v.map, page_h), range.clone(), winding(*rule))
                });
                for (i, (path, ink, rule)) in v.fills.iter().enumerate() {
                    if let Some((rings, range, order)) = &clip {
                        if i == range.start {
                            ops.push(Op::SaveGraphicsState);
                            ops.push(Op::DrawPolygon {
                                polygon: Polygon {
                                    rings: rings.clone(),
                                    mode: PaintMode::Clip,
                                    winding_order: *order,
                                },
                            });
                        }
                        if i == range.end {
                            ops.push(Op::RestoreGraphicsState);
                        }
                    }
                    let rings = rings(path, v.map, page_h);
                    if rings.is_empty() {
                        continue;
                    }
                    let c = match ink {
                        Ink::Solid(c) => *c,
                        Ink::Gradient { gradient, frame } => {
                            ops.extend(gradient_ops(
                                doc,
                                rings,
                                winding(*rule),
                                gradient,
                                *frame,
                                v.map,
                                layer.opacity,
                                page_h,
                            ));
                            continue;
                        }
                    };
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
                            winding_order: winding(*rule),
                        },
                    });
                    ops.push(Op::RestoreGraphicsState);
                }
                if clip.is_some_and(|(_, r, _)| r.end >= v.fills.len()) {
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
        let lines: Vec<_> = element.placed.printed_lines().collect();
        ops.extend(line_ops(
            lines.iter().map(|l| l.as_ref()),
            element.color,
            OUTLINE_WIDTH_MM,
            page_h,
        ));
    }
    if !scene.marks.is_empty() {
        ops.extend(line_ops(
            scene.marks.iter(),
            print::MARK_COLOR,
            print::MARK_WIDTH_MM,
            page_h,
        ));
    }
    ops
}

/// Lines in page mm, stroked in `color`, on a page `page_h` mm tall.
fn line_ops<'a>(
    polylines: impl Iterator<Item = &'a Polyline>,
    color: [u8; 3],
    width_mm: f32,
    page_h: f32,
) -> Vec<Op> {
    let mut ops = vec![
        Op::SetOutlineColor { col: rgb(color) },
        Op::SetOutlineThickness {
            pt: Pt(mm_to_pt(width_mm)),
        },
    ];
    for poly in polylines {
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
    ops
}

/// The pages of a tiled print: on each sheet, the page's drawing moved so the tile's
/// artwork falls within the margins, clipped there, then the sheet's marks.
fn tile_pages(page_ops: &[Op], page_mm: P2, tiling: &print::Tiling) -> Vec<PdfPage> {
    let [sw, sh] = tiling.sheet_mm;
    let ([x0, y0], [x1, y1]) = tiling.content_box();
    tiling
        .tiles(page_mm)
        .into_iter()
        .map(|tile| {
            let [ox, oy] = tile.origin_mm;
            let mut ops = vec![
                Op::SaveGraphicsState,
                Op::DrawPolygon {
                    polygon: polygon(
                        &[vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]],
                        sh,
                        PaintMode::Clip,
                    ),
                },
                // Page point (x, y) lands at (x - ox + x0, y - oy + y0) on the sheet.
                Op::SetTransformationMatrix {
                    matrix: printpdf::CurTransMat::Translate(
                        Pt(mm_to_pt(x0 - ox)),
                        Pt(mm_to_pt(sh - y0 + oy - page_mm[1])),
                    ),
                },
            ];
            ops.extend_from_slice(page_ops);
            ops.push(Op::RestoreGraphicsState);
            ops.extend(line_ops(
                tiling.marks(tile, page_mm).iter(),
                print::MARK_COLOR,
                print::MARK_WIDTH_MM,
                sh,
            ));
            PdfPage::new(Mm(sw), Mm(sh), ops)
        })
        .collect()
}

/// Saves the PDF: the artwork as images, with its text and shape layers and the printed
/// template lines on top as vectors. One page, or a page per sheet when tiled; the
/// images are stored once and shared by the sheets.
fn write_pdf(
    scene: &Scene,
    parts: &[Part],
    dpi: f32,
    path: &Path,
    tiling: Option<print::Tiling>,
) -> Result<(), String> {
    let mut doc = PdfDocument::new(tr!("PDF_DOCUMENT_TITLE"));
    let ops = page_ops(&mut doc, scene, parts, dpi);
    let pages = match tiling {
        Some(t) => tile_pages(&ops, scene.page_mm, &t),
        None => vec![PdfPage::new(
            Mm(scene.page_mm[0]),
            Mm(scene.page_mm[1]),
            ops,
        )],
    };
    let bytes = doc
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    std::fs::write(path, bytes).map_err(|e| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::Document;
    use crate::template;
    use printpdf::Polygon;

    /// A page with the test template, a photo-like square, a text layer over it and a
    /// square over part of the text, as exported at 100 DPI.
    pub(crate) fn text_scene(distort: bool) -> Scene {
        use crate::document::{Layer, Node};
        use crate::text;
        let entry = template::tests::fixture();
        let tpl = template::load(&entry).unwrap();
        let mut doc = Document::default();
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
                    svg: true,
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
    fn pdf_gradients_are_shadings_when_opaque_and_images_when_not() {
        use crate::document::Node;
        use crate::gradient::{Gradient, GradientKind};
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
        // Fading to transparent: no PDF shading can do that.
        let mut faded = shape::new_layer(2, ShapeKind::Ellipse, c);
        {
            let s = faded.shape.as_mut().unwrap();
            let mut g = Gradient::from_color([0, 0, 255, 255]);
            g.stops[1].color = [0, 0, 255, 0];
            s.fill_gradient = Some(g);
        }
        doc.layers = vec![Node::Layer(Box::new(rect)), Node::Layer(Box::new(faded))];
        shape::prepare_export(&mut doc, 100.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
        let vectors = parts
            .iter()
            .filter(|p| matches!(p, Part::Vector(_)))
            .count();
        assert_eq!(
            vectors, 1,
            "the opaque gradient stays a vector, the faded one not"
        );
        let mut pdf = PdfDocument::new("test");
        let ops = page_ops(&mut pdf, &scene, &parts, 100.0);
        // Clipped to the rectangle, then the shading painted.
        let clip = ops.iter().position(|op| {
            matches!(op, Op::DrawPolygon { polygon } if polygon.mode == PaintMode::Clip
                && polygon.rings.len() == 1 && polygon.rings[0].points.len() == 4)
        });
        let paint = ops
            .iter()
            .position(|op| matches!(op, Op::PaintShading { .. }));
        assert!(
            matches!((clip, paint), (Some(c), Some(p)) if c < p),
            "{clip:?} {paint:?}"
        );
        assert_eq!(pdf.resources.shadings.map.len(), 1);
        let shading = pdf.resources.shadings.map.values().next().unwrap();
        assert!(matches!(
            shading.geometry,
            printpdf::ShadingGeometry::Radial { .. }
        ));
        // The shading's center lands on the rectangle's center on the page: the two
        // matrices set after the clip, applied to (0, 0).
        let raw: Vec<[f32; 6]> = ops[clip.unwrap()..paint.unwrap()]
            .iter()
            .filter_map(|op| match op {
                Op::SetTransformationMatrix {
                    matrix: printpdf::CurTransMat::Raw(m),
                } => Some(*m),
                _ => None,
            })
            .collect();
        assert_eq!(raw.len(), 2);
        let apply = |m: [f32; 6], p: [f32; 2]| {
            [
                m[0] * p[0] + m[2] * p[1] + m[4],
                m[1] * p[0] + m[3] * p[1] + m[5],
            ]
        };
        let center = apply(raw[0], apply(raw[1], [0.0, 0.0]));
        let want = to_pdf(scene.page_mm[1], c);
        assert!(
            (center[0] - want.x.0).abs() < 0.5 && (center[1] - want.y.0).abs() < 0.5,
            "{center:?} vs {want:?}"
        );
        assert_eq!(shading.stops[0].color, [1.0, 0.0, 0.0]);
        assert_eq!(shading.stops[1].color, [1.0, 1.0, 1.0]);
        // The whole thing writes.
        let page = PdfPage::new(Mm(297.0), Mm(210.0), ops);
        let bytes = pdf
            .with_pages(vec![page])
            .save(&PdfSaveOptions::default(), &mut Vec::new());
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("/ShadingType 3"),
            "a radial shading in the file"
        );
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
                svg: true,
                ..Default::default()
            };
            let base = std::path::PathBuf::from(dir).join("shapes");
            run(&scene, &settings, &base, &Progress::default()).unwrap();
        }
    }

    /// With ARTWORK_PDF_SAMPLE set, writes text on paths there to look at: around a
    /// circle, under it on the other side, on an arc, and along a curved polyline.
    #[test]
    fn pdf_cuts_paint_off_at_its_shape_under_the_stroke() {
        use crate::document::Node;
        use crate::shape::{self, ShapeKind};
        let mut doc = Document::default();
        let c = doc.page_center();
        let mut l = shape::new_layer(1, ShapeKind::Ellipse, c);
        let line = [[c[0] - 30.0, c[1], 1.0], [c[0] + 30.0, c[1], 1.0]];
        let pencil = crate::paint::Tool::Pencil;
        assert!(shape::add_stroke(
            &mut l,
            &line,
            &crate::paint::Pen::new(pencil, 2.0, [255, 0, 0, 255])
        ));
        doc.layers = vec![Node::Layer(Box::new(l))];
        shape::prepare_export(&mut doc, 100.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
        let mut pdf = PdfDocument::new("test");
        let ops = page_ops(&mut pdf, &scene, &parts, 100.0);
        // Fill, then the clip with the paint inside, then the stroke once it is undone.
        let modes: Vec<String> = ops
            .iter()
            .filter_map(|op| match op {
                Op::DrawPolygon { polygon } => Some(format!("{:?}", polygon.mode)),
                Op::SaveGraphicsState => Some("save".into()),
                Op::RestoreGraphicsState => Some("restore".into()),
                _ => None,
            })
            .collect();
        let shape_ops = &modes[modes.len() - 13..];
        assert_eq!(
            shape_ops,
            [
                "save", "Fill", "restore", "save", "Clip", "save", "Fill", "restore", "restore",
                "save", "Fill", "restore", "restore"
            ],
            "{modes:?}"
        );
    }

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
            svg: true,
            ..Default::default()
        };
        let base = std::path::PathBuf::from(dir).join("text-on-path");
        run(&scene, &settings, &base, &Progress::default()).unwrap();
    }

    #[test]
    fn pdf_bleed_and_marks() {
        // The page with its text layer as vectors, which the outline clips.
        let ops_of = |bleed_mm: f32, marks: bool| {
            let mut scene = text_scene(false);
            scene.cuts.bleed_mm = bleed_mm;
            scene.elements.iter_mut().for_each(|e| e.print = false);
            let scene = scene.with_marks(marks, marks);
            let parts = render::compose_parts(&scene, 100.0, true, &mut |_| true).unwrap();
            page_ops(&mut PdfDocument::new("test"), &scene, &parts, 100.0)
        };
        let clip_width = |ops: &[Op]| {
            let xs: Vec<f32> = ops
                .iter()
                .find_map(|op| match op {
                    Op::DrawPolygon { polygon } if polygon.mode == PaintMode::Clip => Some(polygon),
                    _ => None,
                })
                .unwrap()
                .rings
                .iter()
                .flat_map(|r| &r.points)
                .map(|p| p.p.x.0)
                .collect();
            xs.iter().fold(f32::MIN, |a, &b| a.max(b)) - xs.iter().fold(f32::MAX, |a, &b| a.min(b))
        };
        let lines = |ops: &[Op]| {
            ops.iter()
                .filter(|op| matches!(op, Op::DrawLine { .. }))
                .count()
        };
        let plain = ops_of(0.0, false);
        let bled = ops_of(3.0, true);
        let grown = clip_width(&bled) - clip_width(&plain);
        assert!((grown - mm_to_pt(6.0)).abs() < 0.5, "{grown}");
        assert_eq!(lines(&plain), 0);
        assert_eq!(
            lines(&bled),
            8 + 4 * 4,
            "crop marks and registration targets"
        );

        // A sample to look at, when asked for.
        if let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") {
            let mut scene = text_scene(false);
            scene.cuts.bleed_mm = 3.0;
            let settings = ExportSettings {
                dpi: 150,
                svg: true,
                ..Default::default()
            };
            let base = std::path::PathBuf::from(dir).join("bleed-and-marks");
            run(
                &scene.with_marks(true, true),
                &settings,
                &base,
                &Progress::default(),
            )
            .unwrap();
        }
    }

    #[test]
    fn tiled_export_writes_a_sheet_per_tile() {
        use crate::document::{Layer, Node};
        let dir = std::env::var_os("ARTWORK_PDF_SAMPLE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("artwork-tiles-{}", std::process::id()))
            });
        std::fs::create_dir_all(&dir).unwrap();
        // A 400 x 250 mm page covered by a gradient, its text on top.
        let mut doc = Document {
            page: PageSize::Custom,
            custom_page_mm: [400.0, 250.0],
            orientation: Orientation::Portrait,
            ..Default::default()
        };
        let img = image::RgbaImage::from_fn(400, 250, |x, y| {
            image::Rgba([(x * 255 / 400) as u8, (y * 255 / 250) as u8, 160, 255])
        });
        let mut l = Layer::new(1, "gradient".into(), "x.png".into());
        l.size_px = [400, 250];
        l.transform.center = [200.0, 125.0];
        l.pixmap = Some(std::sync::Arc::new(
            render::premultiplied_pixmap(&img).unwrap(),
        ));
        let mut t = crate::shape::new_layer(2, crate::shape::ShapeKind::Star, [200.0, 125.0]);
        crate::shape::set_geometry_size(&mut t, [120.0, 120.0]);
        doc.layers = vec![Node::Layer(Box::new(l)), Node::Layer(Box::new(t))];
        crate::shape::prepare_export(&mut doc, 100.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        // The grid moved from centered (550 x 277 mm of sheets) to the page's corner.
        let settings = ExportSettings {
            dpi: 100,
            tile: true,
            tile_offset_mm: [75.0, 13.5],
            svg: true,
            ..Default::default()
        };
        // A4 portrait, 190 x 277 mm of artwork each: 3 across, 1 down.
        let base = dir.join("tiled");
        let expected = settings.files(&base, scene.page_mm);
        let out = run(&scene, &settings, &base, &Progress::default()).unwrap();
        assert_eq!(out, expected);
        assert_eq!(out.len(), 7, "3 PNG sheets, 3 SVG sheets and the PDF");
        assert!(out[1].ends_with("tiled-r1c2.png"));
        assert!(out[5].ends_with("tiled-r1c3.svg"));
        let svg = std::fs::read_to_string(&out[4]).unwrap();
        assert!(
            svg.contains(r#"width="210mm" height="297mm""#),
            "A4 portrait sheets"
        );
        let sheet = image::open(&out[0]).unwrap().to_rgb8();
        assert_eq!(sheet.dimensions(), (827, 1169), "A4 portrait at 100 DPI");
        // Artwork within the margin, blank paper outside it and below the page.
        let k = 100.0 / 25.4;
        let at = |x: f32, y: f32| sheet.get_pixel((x * k) as u32, (y * k) as u32).0;
        assert_eq!(at(5.0, 100.0), [255, 255, 255]);
        assert_eq!(at(15.0, 15.0)[2], 160, "the gradient's corner");
        assert_eq!(at(100.0, 270.0), [255, 255, 255], "past the page's bottom");
        // The second sheet starts 180 mm into the page: 190 minus the overlap.
        let second = image::open(&out[1]).unwrap().to_rgb8();
        let red =
            |img: &image::RgbImage, x: f32| img.get_pixel((x * k) as u32, (50.0 * k) as u32).0[0];
        assert!(red(&second, 10.5).abs_diff((180.5 * 255.0 / 400.0) as u8) <= 3);
        assert_eq!(
            red(&sheet, 190.5),
            red(&second, 10.5),
            "the overlap matches"
        );

        // Centered, the first sheet starts 75 mm before the page and 13.5 mm above it:
        // its artwork begins 85 mm across and 23.5 mm down.
        let centered = ExportSettings {
            tile_offset_mm: [0.0, 0.0],
            pdf: false,
            svg: false,
            ..settings.clone()
        };
        let first = run(
            &scene,
            &centered,
            &dir.join("centered"),
            &Progress::default(),
        )
        .unwrap();
        let sheet = image::open(&first[0]).unwrap().to_rgb8();
        let at = |x: f32, y: f32| sheet.get_pixel((x * k) as u32, (y * k) as u32).0;
        assert_eq!(at(83.0, 100.0), [255, 255, 255]);
        assert_eq!(at(87.0, 100.0)[2], 160);
        assert_eq!(at(100.0, 22.0), [255, 255, 255]);
        assert_eq!(at(100.0, 25.0)[2], 160);

        let pdf = std::fs::read(&out[6]).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        let pages = text.matches("/Type/Page").count() - text.matches("/Type/Pages").count();
        assert_eq!(pages, 3, "a PDF page per sheet");
        if std::env::var_os("ARTWORK_PDF_SAMPLE").is_none() {
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn sheets_print_marks_past_the_page() {
        // A 100 x 50 mm page, centered on one A5 sheet: 128 x 190 mm of artwork, so the
        // sheet's artwork starts 14 mm left of the page and 70 mm above it.
        let scene = Scene {
            page_mm: [100.0, 50.0],
            background: [255, 255, 255],
            background_gradient: None,
            layers: Vec::new(),
            elements: Vec::new(),
            cuts: Cuts::default(),
            marks: vec![Polyline {
                points: vec![[10.0, -3.0], [40.0, -3.0]],
                closed: false,
            }],
        };
        let tiling = print::Tiling {
            sheet_mm: [148.0, 210.0],
            margin_mm: 10.0,
            overlap_mm: 10.0,
            marks: false,
            offset_mm: [0.0, 0.0],
        };
        let tile = tiling.tiles(scene.page_mm)[0];
        assert_eq!(tile.origin_mm, [-14.0, -70.0]);
        let page = render::compose(&scene, 254.0, false).unwrap();
        let sheet = sheet_pixmap(&page, &scene, &tiling, tile, 254.0).unwrap();
        // 3 mm above the page, at 10 px per mm: y 10 + 67 mm, x 10 + 24 to 10 + 54 mm.
        let dark = |x: u32, y: u32| sheet.pixel(x, y).unwrap().red() < 128;
        assert!(dark(500, 770), "the mark above the page is on the sheet");
        assert!(!dark(500, 760) && !dark(300, 770));
        // Moved so the mark falls in the sheet's margin: it is left out.
        let moved = print::Tiling {
            offset_mm: [0.0, 69.0],
            ..tiling
        };
        let tile = moved.tiles(scene.page_mm)[0];
        let sheet = sheet_pixmap(&page, &scene, &moved, tile, 254.0).unwrap();
        assert!((0..sheet.height()).all(|y| sheet.pixel(500, y).unwrap().red() == 255));
    }

    #[test]
    fn mirrored_export_is_the_page_flipped() {
        // Text, squares, template lines and crop marks, at 10 px per mm: the page is a
        // whole number of pixels wide, so mirrored pixels line up exactly.
        let scene = || text_scene(false).with_marks(true, true);
        let dpi = 254.0;
        let plain = render::compose(&scene(), dpi, true).unwrap();
        let mirrored = render::compose(&scene().mirrored(), dpi, true).unwrap();
        let (w, h) = (plain.width(), plain.height());
        assert_eq!(w as f32, scene().page_mm[0] * 10.0);
        let mut differ = 0;
        for y in 0..h {
            for x in 0..w {
                let a = plain.pixel(x, y).unwrap();
                let b = mirrored.pixel(w - 1 - x, y).unwrap();
                let d = [
                    a.red().abs_diff(b.red()),
                    a.green().abs_diff(b.green()),
                    a.blue().abs_diff(b.blue()),
                ];
                if d.iter().any(|&d| d > 40) {
                    differ += 1;
                }
            }
        }
        assert!(differ < (w * h) as usize / 2000, "{differ} pixels differ");
        // Something to mirror: the page isn't symmetric.
        assert!((0..w).any(|x| plain.pixel(x, h / 2) != plain.pixel(w - 1 - x, h / 2)));

        // In the PDF, the text's points are mirrored across the page.
        let letters = |scene: &Scene| -> Vec<f32> {
            let parts = render::compose_parts(scene, 100.0, true, &mut |_| true).unwrap();
            let ops = page_ops(&mut PdfDocument::new("test"), scene, &parts, 100.0);
            ops.iter()
                .find_map(|op| match op {
                    Op::DrawPolygon { polygon } if polygon.mode == PaintMode::Fill => {
                        Some(polygon.rings[0].points.iter().map(|p| p.p.x.0).collect())
                    }
                    _ => None,
                })
                .unwrap()
        };
        let page_w = mm_to_pt(scene().page_mm[0]);
        for (a, b) in letters(&scene()).iter().zip(letters(&scene().mirrored())) {
            assert!((a - (page_w - b)).abs() < 0.01, "{a} {b}");
        }
    }

    #[test]
    fn exports_png_and_pdf_with_template_outline() {
        let dir = std::env::temp_dir().join(format!("artwork-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = template::tests::fixture();
        let tpl = template::load(&entry).unwrap();
        let doc = Document::default();
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
