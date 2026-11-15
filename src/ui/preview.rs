//! Test helper: draws a piece of the interface to a PNG, in the dark and light themes
//! side by side, without a window or a GPU, to look at a widget from a test. egui's
//! triangles are filled in software, textured with its font atlas.

use std::collections::HashMap;

use egui::{Rect, Ui, Vec2, Visuals, vec2};

/// Draws `add` in a `size` panel, dark then light, at twice the size, to `path`.
pub fn render(path: &str, size: Vec2, add: impl Fn(&mut Ui)) {
    let ppp = 2.0;
    let (pw, ph) = ((size.x * ppp) as u32, (size.y * ppp) as u32);
    let mut sheet = image::RgbaImage::new(pw * 2, ph);
    for (i, visuals) in [Visuals::dark(), Visuals::light()].into_iter().enumerate() {
        let ctx = egui::Context::default();
        ctx.set_visuals(visuals);
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let run = |ui: &mut Ui| {
            egui::CentralPanel::default().show(ui, |ui| add(ui));
        };
        // The textures egui uses: its font atlas, and others (such as the checkers
        // behind transparent colors).
        let mut textures: HashMap<egui::TextureId, egui::ColorImage> = HashMap::new();
        let mut keep = |delta: &mut egui::TexturesDelta| {
            for (id, deltas) in delta.set.drain() {
                for d in deltas {
                    let egui::ImageData::Color(img) = &d.image;
                    match (textures.get_mut(&id), d.pos) {
                        (Some(t), Some([x, y])) => paste(t, img, x, y),
                        _ => {
                            textures.insert(id, (**img).clone());
                        }
                    }
                }
            }
            delta.clear();
        };
        // Passes for the layout and the font atlas to settle, then the one drawn.
        for _ in 0..2 {
            let mut first = ctx.run_ui(input(), run);
            keep(&mut first.textures_delta);
        }
        let mut out = ctx.run_ui(input(), run);
        keep(&mut out.textures_delta);
        let x0 = i as u32 * pw;
        let bg = ctx.global_style().visuals.panel_fill.to_array();
        for y in 0..ph {
            for x in 0..pw {
                sheet.put_pixel(x0 + x, y, image::Rgba(bg));
            }
        }
        for p in ctx.tessellate(out.shapes, ppp) {
            if let egui::epaint::Primitive::Mesh(mesh) = p.primitive {
                let tex = &textures[&mesh.texture_id];
                for t in mesh.indices.chunks(3) {
                    let v = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
                    fill_triangle(&mut sheet, x0, pw, ppp, p.clip_rect, &v, tex);
                }
            }
        }
    }
    sheet.save(path).unwrap();
}

fn paste(into: &mut egui::ColorImage, part: &egui::ColorImage, x: usize, y: usize) {
    for py in 0..part.size[1] {
        for px in 0..part.size[0] {
            let (tx, ty) = (x + px, y + py);
            if tx < into.size[0] && ty < into.size[1] {
                into.pixels[ty * into.size[0] + tx] = part.pixels[py * part.size[0] + px];
            }
        }
    }
}

/// Fills one of egui's triangles: vertex colors times the texture, blended over.
fn fill_triangle(
    img: &mut image::RgbaImage,
    x0: u32,
    width: u32,
    ppp: f32,
    clip: Rect,
    v: &[egui::epaint::Vertex; 3],
    tex: &egui::ColorImage,
) {
    let p = v.map(|v| v.pos.to_vec2() * ppp);
    let min = p[0].min(p[1]).min(p[2]);
    let max = p[0].max(p[1]).max(p[2]);
    let area = (p[1] - p[0]).x * (p[2] - p[0]).y - (p[1] - p[0]).y * (p[2] - p[0]).x;
    if area.abs() < 1e-6 {
        return;
    }
    let (cx0, cy0) = ((clip.min.x * ppp).max(0.0), (clip.min.y * ppp).max(0.0));
    let (cx1, cy1) = (clip.max.x * ppp, clip.max.y * ppp);
    for y in (min.y.max(cy0) as i32)..=(max.y.min(cy1 - 1.0) as i32) {
        for x in (min.x.max(cx0) as i32)..=(max.x.min(cx1 - 1.0) as i32) {
            if x < 0 || y < 0 || x as u32 >= width || y as u32 >= img.height() {
                continue;
            }
            let q = vec2(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1] - q).x * (p[2] - q).y - (p[1] - q).y * (p[2] - q).x) / area;
            let w1 = ((p[2] - q).x * (p[0] - q).y - (p[2] - q).y * (p[0] - q).x) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let mix =
                |f: fn(&egui::epaint::Vertex) -> f32| w0 * f(&v[0]) + w1 * f(&v[1]) + w2 * f(&v[2]);
            let (u, t) = (mix(|v| v.uv.x), mix(|v| v.uv.y));
            let tx = ((u * tex.size[0] as f32) as usize).min(tex.size[0] - 1);
            let ty = ((t * tex.size[1] as f32) as usize).min(tex.size[1] - 1);
            let texel = tex.pixels[ty * tex.size[0] + tx]
                .to_array()
                .map(|c| c as f32 / 255.0);
            let col = [
                mix(|v| v.color.r() as f32),
                mix(|v| v.color.g() as f32),
                mix(|v| v.color.b() as f32),
                mix(|v| v.color.a() as f32),
            ]
            .map(|c| c / 255.0);
            // Premultiplied source over the destination.
            let src: [f32; 4] = std::array::from_fn(|k| col[k] * texel[k]);
            let px = img.get_pixel_mut(x0 + x as u32, y as u32);
            for k in 0..3 {
                let d = px.0[k] as f32 / 255.0;
                px.0[k] = ((src[k] + d * (1.0 - src[3])) * 255.0).round() as u8;
            }
        }
    }
}
