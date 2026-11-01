//! PNG and PDF writers.

use std::path::{Path, PathBuf};

use printpdf::{
    Color, Line, LinePoint, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Point, Pt, RawImage,
    RawImageData, RawImageFormat, Rgb, XObjectTransform,
};

use crate::render::{self, Cuts, OUTLINE_WIDTH_MM, Scene};

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
    pub print_lines: [bool; 3],
    /// Clip to the outline and leave holes blank.
    pub cuts: Cuts,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            png: true,
            pdf: true,
            dpi: 300,
            print_lines: [true; 3],
            cuts: Cuts::default(),
        }
    }
}

/// Writes the requested formats next to each other, using `base` with its extension
/// replaced.
pub fn run(scene: &Scene, settings: &ExportSettings, base: &Path) -> Result<Vec<PathBuf>, String> {
    let dpi = settings.dpi.clamp(72, 1200) as f32;
    let mut written = Vec::new();
    if settings.png {
        let path = base.with_extension("png");
        let pixmap = render::compose(scene, dpi, true)?;
        write_png(&pixmap, dpi, &path)?;
        written.push(path);
    }
    if settings.pdf {
        let path = base.with_extension("pdf");
        // Cut lines go in as vectors so they stay sharp at any zoom.
        let pixmap = render::compose(scene, dpi, false)?;
        write_pdf(scene, &pixmap, dpi, &path)?;
        written.push(path);
    }
    Ok(written)
}

/// Saves an opaque RGB PNG that records its DPI, so it prints at true size.
fn write_png(pixmap: &tiny_skia::Pixmap, dpi: f32, path: &Path) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| format!("Cannot write {}: {e}", path.display());
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

/// Millimetres to PDF points (1/72 inch).
fn mm_to_pt(mm: f32) -> f32 {
    mm * 72.0 / 25.4
}

/// Saves a one-page PDF: the artwork as an image covering the page, with the printed
/// template lines on top as vectors.
fn write_pdf(
    scene: &Scene,
    pixmap: &tiny_skia::Pixmap,
    dpi: f32,
    path: &Path,
) -> Result<(), String> {
    let mut doc = PdfDocument::new("Controller artwork");
    let image = RawImage {
        pixels: RawImageData::U8(render::to_rgb(pixmap)),
        width: pixmap.width() as usize,
        height: pixmap.height() as usize,
        data_format: RawImageFormat::RGB8,
        tag: Vec::new(),
    };
    let image_id = doc.add_image(&image);

    // The image is placed at the bottom-left corner and sized by its DPI to cover the page.
    let mut ops = vec![Op::UseXobject {
        id: image_id,
        transform: XObjectTransform {
            dpi: Some(dpi),
            ..Default::default()
        },
    }];

    let page_h = scene.page_mm[1];
    for element in scene.elements.iter().filter(|e| e.print) {
        let [r, g, b] = element.color;
        ops.push(Op::SetOutlineColor {
            col: Color::Rgb(Rgb::new(
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                None,
            )),
        });
        ops.push(Op::SetOutlineThickness {
            pt: Pt(mm_to_pt(OUTLINE_WIDTH_MM)),
        });
        for poly in &element.placed.polylines {
            let points = poly
                .points
                .iter()
                .map(|p| LinePoint {
                    p: Point {
                        x: Pt(mm_to_pt(p[0])),
                        y: Pt(mm_to_pt(page_h - p[1])),
                    },
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

    let page = PdfPage::new(Mm(scene.page_mm[0]), Mm(scene.page_mm[1]), ops);
    let bytes = doc
        .with_pages(vec![page])
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    std::fs::write(path, bytes).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::template;

    #[test]
    fn exports_png_and_pdf_with_t16_outline() {
        let dir = std::env::temp_dir().join(format!("artwork-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry =
            template::scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")).remove(0);
        let tpl = template::load(&entry).unwrap();
        let mut doc = Document::default();
        doc.template_placement = entry.default_placement(tpl.size_mm, doc.page_mm());
        let scene = Scene::new(
            &doc,
            Some(&tpl),
            [[255, 0, 255]; 3],
            [true; 3],
            Cuts::default(),
        );
        let settings = ExportSettings {
            dpi: 100,
            ..Default::default()
        };
        let out = run(&scene, &settings, &dir.join("art")).unwrap();
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
        std::fs::remove_dir_all(&dir).ok();
    }
}
