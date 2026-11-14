//! The template editor's reference: a DXF, SVG, PNG or PDF file the template's shapes are
//! drawn over, shown at its true size as an image layer below every element. Only the
//! editor shows it (see `template_edit`).

use std::path::Path;
use std::sync::Arc;

use crate::document::{Layer, P2};
use crate::i18n::tr;

/// The files a reference can be.
pub const EXTENSIONS: &[&str] = &["dxf", "svg", "png", "pdf"];
/// Pixels per mm the drawings (DXF, SVG, PDF) are rendered at, unless that makes them
/// larger than `MAX_SIDE`.
const PX_PER_MM: f64 = 10.0;
/// Largest width or height of a rendered drawing, in pixels.
const MAX_SIDE: f64 = 4096.0;
/// Width of a DXF's lines, in mm.
const DXF_LINE_MM: f64 = 0.3;

/// What a reference file looks like: PNG bytes, its pixel size, and its size in mm.
struct Rendered {
    png: Vec<u8>,
    size_px: [u32; 2],
    size_mm: P2,
}

/// The reference layer of the file at `path`, at its true size, centered on (0, 0). Its
/// image is in its `data`, as a PNG, to be loaded as any image layer's.
pub fn layer(id: u64, path: &Path) -> Result<Layer, String> {
    let r = render(path)?;
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut l = Layer::new(
        id,
        tr!("REFERENCE_LAYER_NAME", file = file),
        path.to_owned(),
    );
    l.size_px = r.size_px;
    l.data = Some(Arc::new(r.png));
    l.transform.center = [0.0, 0.0];
    l.transform.scale = [
        r.size_mm[0] / r.size_px[0] as f32,
        r.size_mm[1] / r.size_px[1] as f32,
    ];
    Ok(l)
}

fn render(path: &Path) -> Result<Rendered, String> {
    let fail = |e: &dyn std::fmt::Display| tr!("CANNOT_OPEN_FILE", path = path.display(), e = e);
    let data = std::fs::read(path).map_err(|e| fail(&e))?;
    let ext = path
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => png(data).map_err(|e| fail(&e)),
        "svg" => svg(&data).map_err(|e| fail(&e)),
        "pdf" => pdf(data).map_err(|e| fail(&e)),
        "dxf" => dxf(path).map_err(|e| fail(&e)),
        _ => Err(fail(&"?")),
    }
}

/// Pixels per mm for a drawing of `size` mm: `PX_PER_MM`, less for a large one.
fn px_per_mm(size: [f64; 2]) -> f64 {
    PX_PER_MM.min(MAX_SIDE / size[0].max(size[1]).max(1.0))
}

/// The file as it is; its size from its resolution, 96 DPI when it says none.
fn png(data: Vec<u8>) -> Result<Rendered, String> {
    let decoder = ::png::Decoder::new(std::io::Cursor::new(&data));
    let reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let size_px = [info.width, info.height];
    let mm_per_px = match info.pixel_dims {
        Some(d) if d.unit == ::png::Unit::Meter && d.xppu > 0 && d.yppu > 0 => {
            [1000.0 / d.xppu as f32, 1000.0 / d.yppu as f32]
        }
        _ => [25.4 / 96.0; 2],
    };
    Ok(Rendered {
        size_mm: [
            size_px[0] as f32 * mm_per_px[0],
            size_px[1] as f32 * mm_per_px[1],
        ],
        size_px,
        png: data,
    })
}

/// Rendered with resvg; its size as the SVG says (CSS pixels, 96 to the inch, without
/// units).
fn svg(data: &[u8]) -> Result<Rendered, String> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size();
    let mm = [
        size.width() as f64 * 25.4 / 96.0,
        size.height() as f64 * 25.4 / 96.0,
    ];
    let k = px_per_mm(mm);
    let px = size_px(mm, k);
    let mut pixmap = tiny_skia::Pixmap::new(px[0], px[1]).ok_or("empty")?;
    let s = (k * 25.4 / 96.0) as f32;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(s, s),
        &mut pixmap.as_mut(),
    );
    encode(pixmap, mm)
}

/// The first page, rendered with hayro; its size in points (72 to the inch).
fn pdf(data: Vec<u8>) -> Result<Rendered, String> {
    use hayro::hayro_interpret::InterpreterSettings;
    let pdf = hayro::hayro_syntax::Pdf::new(data).map_err(|e| format!("{e:?}"))?;
    let page = pdf.pages().first().ok_or("no page")?;
    let (w, h) = page.render_dimensions();
    let mm = [w as f64 * 25.4 / 72.0, h as f64 * 25.4 / 72.0];
    let k = (px_per_mm(mm) * 25.4 / 72.0) as f32;
    let settings = hayro::PixmapSettings {
        x_scale: k,
        y_scale: k,
        ..Default::default()
    };
    let rendered = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &settings,
    );
    let px = [rendered.width() as u32, rendered.height() as u32];
    let rgba = rendered.take_rgba8(hayro::vello_cpu::peniko::ImageAlphaType::Alpha);
    let img = image::RgbaImage::from_raw(px[0], px[1], rgba).ok_or("empty")?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(Rendered {
        png,
        size_px: px,
        size_mm: [mm[0] as f32, mm[1] as f32],
    })
}

/// Its lines drawn in black, with a margin of a line's width around them.
fn dxf(path: &Path) -> Result<Rendered, String> {
    let items = crate::template_geom::read(path, None)?;
    let (lo, hi) = crate::template_geom::items_bounds(&items).ok_or("empty")?;
    let pad = DXF_LINE_MM;
    let mm = [hi[0] - lo[0] + 2.0 * pad, hi[1] - lo[1] + 2.0 * pad];
    let k = px_per_mm(mm);
    let px = size_px(mm, k);
    let mut pixmap = tiny_skia::Pixmap::new(px[0], px[1]).ok_or("empty")?;
    let paint = tiny_skia::Paint {
        anti_alias: true,
        ..Default::default()
    };
    let stroke = tiny_skia::Stroke {
        width: ((DXF_LINE_MM * k) as f32).max(1.0),
        ..Default::default()
    };
    // Items are y up.
    let at = |[x, y]: [f64; 2]| {
        (
            ((x - lo[0] + pad) * k) as f32,
            ((hi[1] - y + pad) * k) as f32,
        )
    };
    for (points, closed) in crate::template::item_paths(&items) {
        let mut pb = tiny_skia::PathBuilder::new();
        for (i, p) in points.iter().enumerate() {
            let (x, y) = at(*p);
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if closed {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            pixmap.stroke_path(
                &path,
                &paint,
                &stroke,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    }
    encode(pixmap, mm)
}

/// Pixels for `mm` at `k` pixels per mm, at least one.
fn size_px(mm: [f64; 2], k: f64) -> [u32; 2] {
    mm.map(|v| ((v * k).round() as u32).max(1))
}

fn encode(pixmap: tiny_skia::Pixmap, mm: [f64; 2]) -> Result<Rendered, String> {
    let size_px = [pixmap.width(), pixmap.height()];
    let png = pixmap.encode_png().map_err(|e| e.to_string())?;
    Ok(Rendered {
        png,
        size_px,
        size_mm: [mm[0] as f32, mm[1] as f32],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawings_come_at_their_true_size() {
        let dir = std::env::temp_dir().join(format!("artwork-reference-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let svg = dir.join("r.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50"><rect x="10" y="10" width="80" height="30" fill="red"/></svg>"#,
        )
        .unwrap();
        let l = layer(1, &svg).unwrap();
        let [w, h] = l.size_mm();
        assert!(
            (w - 100.0).abs() < 0.2 && (h - 50.0).abs() < 0.2,
            "{w} x {h}"
        );
        assert_eq!(l.size_px, [1000, 500]);
        assert!(image::load_from_memory(l.data.as_ref().unwrap()).is_ok());

        let dxf = crate::template::tests::fixture()
            .sources
            .iter()
            .find(|s| s.path.extension().is_some_and(|x| x == "dxf"))
            .map(|s| s.path.clone())
            .unwrap();
        let l = layer(2, &dxf).unwrap();
        assert!(l.size_mm()[0] > 10.0);
        assert!(image::load_from_memory(l.data.as_ref().unwrap()).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A one-page PDF of `w` x `h` points with a black square, offsets and all.
    fn small_pdf(w: u32, h: u32) -> Vec<u8> {
        let content = "0 0 0 rg 10 10 50 50 re f";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents 4 0 R >>"),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        out
    }

    #[test]
    fn a_pdf_comes_at_its_page_size() {
        let dir =
            std::env::temp_dir().join(format!("artwork-reference-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("r.pdf");
        // 100 x 50 mm.
        std::fs::write(&pdf, small_pdf(283, 142)).unwrap();
        let l = layer(1, &pdf).unwrap();
        let [w, h] = l.size_mm();
        assert!(
            (w - 99.8).abs() < 0.3 && (h - 50.1).abs() < 0.3,
            "{w} x {h}"
        );
        let img = image::load_from_memory(l.data.as_ref().unwrap())
            .unwrap()
            .to_rgba8();
        // The square is drawn: some pixel is opaque black.
        assert!(img.pixels().any(|p| p.0 == [0, 0, 0, 255]));
        std::fs::remove_dir_all(&dir).ok();
    }
}
