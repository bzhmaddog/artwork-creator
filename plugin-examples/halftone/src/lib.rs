//! Example Artwork Creator plugin: a halftone effect, redrawing the image as a screen of
//! dots whose size follows its darkness, like print seen under a magnifier.
//!
//! Build it with `make plugins`, which copies the library into `plugins/`.

use artwork_plugin_sdk::{Effect, Image, Param, Values, export_plugin};

struct Halftone;

/// What one cell of the screen samples from the image.
#[derive(Clone, Copy, Default)]
struct Cell {
    /// Dot radius, in pixels.
    radius: f32,
    /// Average straight colour under the cell.
    color: [f32; 3],
}

impl Effect for Halftone {
    fn id(&self) -> &str {
        "halftone.dots"
    }

    fn name(&self) -> &str {
        "Halftone"
    }

    fn description(&self) -> &str {
        "Redraw the image as a screen of dots sized by its darkness (example plugin)"
    }

    fn params(&self) -> Vec<Param> {
        vec![
            Param::float("size", "Dot spacing", 2.0, 100.0, 8.0).unit(" px"),
            Param::float("angle", "Angle", 0.0, 90.0, 45.0)
                .unit("°")
                .decimals(0),
            Param::choice(
                "style",
                "Dots",
                &[("ink", "One ink"), ("color", "Image colours")],
                "ink",
            ),
            Param::color("ink", "Ink", [0, 0, 0, 255]),
            Param::color("paper", "Paper", [255, 255, 255, 255]),
        ]
    }

    fn apply(&self, img: &mut Image, v: &Values, scale: f32) -> Result<(), String> {
        let cell = (v.f32("size") * scale).max(2.0);
        let (sin, cos) = v.f32("angle").to_radians().sin_cos();
        let one_ink = v.choice("style") != "color";
        let ink = v.color("ink").map(|c| c as f32);
        let paper = v.color("paper").map(|c| c as f32);
        let (w, h) = (img.width() as f32, img.height() as f32);

        // Screen coordinates (u, v) are the image's, turned by the angle.
        let to_screen = |x: f32, y: f32| (x * cos + y * sin, -x * sin + y * cos);
        let to_image = |u: f32, v: f32| (u * cos - v * sin, u * sin + v * cos);
        let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| to_screen(x, y));
        let (u0, v0) = corners
            .iter()
            .fold((f32::MAX, f32::MAX), |a, c| (a.0.min(c.0), a.1.min(c.1)));
        let (u1, v1) = corners
            .iter()
            .fold((f32::MIN, f32::MIN), |a, c| (a.0.max(c.0), a.1.max(c.1)));
        // One spare cell all around, so every pixel has its 3x3 neighbours.
        let (cu0, cv0) = (
            (u0 / cell).floor() as i64 - 1,
            (v0 / cell).floor() as i64 - 1,
        );
        let cols = ((u1 / cell).ceil() as i64 - cu0 + 2) as usize;
        let rows = ((v1 / cell).ceil() as i64 - cv0 + 2) as usize;
        if cols.saturating_mul(rows) > 64_000_000 {
            return Err("image too large for this dot spacing".into());
        }

        // Sample each cell: average of 3x3 points spread over it, weighted by alpha.
        let mut cells = vec![Cell::default(); cols * rows];
        artwork_plugin_sdk::par_rows(&mut cells, cols, |r0, chunk| {
            for (i, c) in chunk.iter_mut().enumerate() {
                let (cu, cv) = (cu0 + (i % cols) as i64, cv0 + (r0 + i / cols) as i64);
                let (uc, vc) = ((cu as f32 + 0.5) * cell, (cv as f32 + 0.5) * cell);
                let (mut sum, mut alpha) = ([0.0f32; 3], 0.0f32);
                for du in [-1.0, 0.0, 1.0] {
                    for dv in [-1.0, 0.0, 1.0] {
                        let (x, y) = to_image(uc + du * cell / 3.0, vc + dv * cell / 3.0);
                        if x < 0.0 || y < 0.0 || x >= w || y >= h {
                            continue;
                        }
                        let px = img.pixel(x as u32, y as u32);
                        let a = px[3] as f32 / 255.0;
                        for k in 0..3 {
                            sum[k] += px[k] as f32 * a;
                        }
                        alpha += a;
                    }
                }
                if alpha <= 0.0 {
                    continue;
                }
                let color = sum.map(|s| s / alpha);
                let luma = (0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2]) / 255.0;
                // Coverage grows with darkness and with how much of the cell is opaque;
                // at full darkness the dot reaches the cell's corners.
                let dark = (1.0 - luma) * alpha / 9.0;
                *c = Cell {
                    radius: cell * std::f32::consts::FRAC_1_SQRT_2 * dark.sqrt(),
                    color,
                };
            }
        });

        img.par_rows(|y, row| {
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                if px[3] == 0 {
                    continue;
                }
                let (u, v) = to_screen(x as f32 + 0.5, y as f32 + 0.5);
                let (cu, cv) = ((u / cell).floor() as i64, (v / cell).floor() as i64);
                // Dots can overlap the neighbouring cells: keep the most covering one.
                let (mut cover, mut color) = (0.0f32, ink);
                for nv in cv - 1..=cv + 1 {
                    for nu in cu - 1..=cu + 1 {
                        let i = (nv - cv0) as usize * cols + (nu - cu0) as usize;
                        let c = cells[i];
                        let d = ((u - (nu as f32 + 0.5) * cell).powi(2)
                            + (v - (nv as f32 + 0.5) * cell).powi(2))
                        .sqrt();
                        // One pixel of antialiasing at the edge.
                        let k = (c.radius - d + 0.5).clamp(0.0, 1.0);
                        if k > cover {
                            cover = k;
                            if !one_ink {
                                color = [c.color[0], c.color[1], c.color[2], 255.0];
                            }
                        }
                    }
                }
                let a = px[3] as f32 / 255.0;
                for k in 0..4 {
                    let mut out = paper[k] + (color[k] - paper[k]) * cover;
                    if k == 3 {
                        out *= a;
                    }
                    px[k] = out.round().clamp(0.0, 255.0) as u8;
                }
            }
        });
        Ok(())
    }
}

export_plugin!(
    name: "Halftone",
    version: env!("CARGO_PKG_VERSION"),
    effects: [Halftone],
);
