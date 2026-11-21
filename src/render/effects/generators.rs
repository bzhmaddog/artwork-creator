//! Generators that ship with the app: noise, clouds, a checkerboard and stripes.
//!
//! Sizes are in mm and every pixel is worked out from where it lies in mm, so the
//! picture is the same at any resolution, only sharper.

use image::RgbaImage;

use super::{Generator, ParamSpec, Params, par_rows};
use crate::i18n::{tr_noop, translate};

type GenerateFn = fn(&mut RgbaImage, &Params, f32) -> Result<(), String>;

/// A generator made of a function and its parameter list.
struct Builtin {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    params: Vec<ParamSpec>,
    generate: GenerateFn,
}

impl Generator for Builtin {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        translate(self.name)
    }
    fn description(&self) -> &str {
        translate(self.description)
    }
    fn params(&self) -> &[ParamSpec] {
        &self.params
    }
    fn generate(&self, img: &mut RgbaImage, params: &Params, px_per_mm: f32) -> Result<(), String> {
        (self.generate)(img, params, px_per_mm)
    }
}

fn generator(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    params: Vec<ParamSpec>,
    generate: GenerateFn,
) -> Box<dyn Generator> {
    Box::new(Builtin {
        id,
        name,
        description,
        params,
        generate,
    })
}

fn mm(id: &str, label: &str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec::float(id, label, min, max, default)
        .unit("mm")
        .decimals(2)
}

/// The two colors a generator mixes.
fn colors(a: [u8; 4], b: [u8; 4]) -> [ParamSpec; 2] {
    [
        ParamSpec::color("color1", tr_noop!("PARAM_COLOR_1"), a),
        ParamSpec::color("color2", tr_noop!("PARAM_COLOR_2"), b),
    ]
}

fn seed() -> ParamSpec {
    ParamSpec::int("seed", tr_noop!("PARAM_SEED"), 0, 9999, 1)
}

fn angle(default: f32) -> ParamSpec {
    ParamSpec::float("angle", tr_noop!("PARAM_ANGLE"), -180.0, 180.0, default)
        .unit("°")
        .decimals(0)
}

/// The built-in generators, in menu order.
pub fn all() -> Vec<Box<dyn Generator>> {
    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    let with = |first: Vec<ParamSpec>, colors: [ParamSpec; 2], last: Vec<ParamSpec>| {
        first.into_iter().chain(colors).chain(last).collect()
    };
    vec![
        generator(
            "builtin.noise",
            tr_noop!("GENERATOR_NOISE"),
            tr_noop!("GENERATOR_NOISE_HINT"),
            with(
                vec![mm("size", tr_noop!("PARAM_GRAIN"), 0.05, 10.0, 0.2)],
                colors(BLACK, WHITE),
                vec![seed()],
            ),
            noise,
        ),
        generator(
            "builtin.clouds",
            tr_noop!("GENERATOR_CLOUDS"),
            tr_noop!("GENERATOR_CLOUDS_HINT"),
            with(
                vec![
                    mm("size", tr_noop!("PARAM_SIZE"), 1.0, 500.0, 40.0),
                    ParamSpec::int("detail", tr_noop!("PARAM_DETAIL"), 1, 8, 5),
                ],
                colors([40, 90, 200, 255], WHITE),
                vec![seed()],
            ),
            clouds,
        ),
        generator(
            "builtin.checkerboard",
            tr_noop!("GENERATOR_CHECKERBOARD"),
            tr_noop!("GENERATOR_CHECKERBOARD_HINT"),
            with(
                vec![mm("size", tr_noop!("PARAM_SQUARE_SIZE"), 0.2, 200.0, 10.0)],
                colors(BLACK, WHITE),
                vec![angle(0.0)],
            ),
            checkerboard,
        ),
        generator(
            "builtin.stripes",
            tr_noop!("GENERATOR_STRIPES"),
            tr_noop!("GENERATOR_STRIPES_HINT"),
            with(
                vec![mm("size", tr_noop!("PARAM_WIDTH"), 0.2, 200.0, 5.0)],
                colors(BLACK, WHITE),
                vec![angle(45.0)],
            ),
            stripes,
        ),
    ]
}

/// `a` mixed with `t` of `b`, straight RGBA.
fn mix(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// Sets each pixel to `f(x, y)` of its center, in mm.
fn fill(img: &mut RgbaImage, px_per_mm: f32, f: impl Fn(f32, f32) -> [u8; 4] + Sync) {
    let row = img.width() as usize * 4;
    let k = 1.0 / px_per_mm.max(1e-6);
    par_rows(img, row, |y0, chunk| {
        for (j, r) in chunk.chunks_exact_mut(row).enumerate() {
            let y = ((y0 + j) as f32 + 0.5) * k;
            for (i, px) in r.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *px = f((i as f32 + 0.5) * k, y);
            }
        }
    });
}

/// A number from 0 to 1 that only depends on the cell and the seed.
fn hash(x: i64, y: i64, seed: i32) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (seed as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn noise(img: &mut RgbaImage, p: &Params, px_per_mm: f32) -> Result<(), String> {
    let size = p.f32("size").max(1e-3);
    let (a, b, seed) = (p.color("color1"), p.color("color2"), p.i32("seed"));
    fill(img, px_per_mm, |x, y| {
        let t = hash((x / size).floor() as i64, (y / size).floor() as i64, seed);
        mix(a, b, t)
    });
    Ok(())
}

/// Smooth noise from 0 to 1, one random value per unit cell's corner.
fn value_noise(x: f32, y: f32, seed: i32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (x0 as i64, y0 as i64);
    let top = hash(i, j, seed) + (hash(i + 1, j, seed) - hash(i, j, seed)) * sx;
    let bottom = hash(i, j + 1, seed) + (hash(i + 1, j + 1, seed) - hash(i, j + 1, seed)) * sx;
    top + (bottom - top) * sy
}

fn clouds(img: &mut RgbaImage, p: &Params, px_per_mm: f32) -> Result<(), String> {
    let size = p.f32("size").max(1e-3);
    let detail = p.i32("detail").clamp(1, 8);
    let (a, b, seed) = (p.color("color1"), p.color("color2"), p.i32("seed"));
    fill(img, px_per_mm, |x, y| {
        // Each finer layer half the size and half as strong.
        let (mut sum, mut total, mut k, mut w) = (0.0, 0.0, 1.0 / size, 1.0);
        for o in 0..detail {
            sum += value_noise(x * k, y * k, seed.wrapping_add(o * 7919)) * w;
            total += w;
            k *= 2.0;
            w *= 0.5;
        }
        mix(a, b, sum / total)
    });
    Ok(())
}

/// From -1 to 1 across a band edge at `u` (in band widths, a band from 0 to 1 then the
/// next): -1 inside an even band, 1 inside an odd one, blended over about a pixel
/// (`px` band widths) at the edges.
fn square_wave(u: f32, px: f32) -> f32 {
    let f = u.rem_euclid(2.0);
    // Signed distance to the nearest edge, positive in the odd band.
    let d = if f < 1.0 {
        -(f.min(1.0 - f))
    } else {
        (f - 1.0).min(2.0 - f)
    };
    (d / px.max(1e-6) * 2.0).clamp(-1.0, 1.0)
}

/// The coordinates turned by `deg` counterclockwise.
fn turned(x: f32, y: f32, deg: f32) -> (f32, f32) {
    let (s, c) = deg.to_radians().sin_cos();
    // y is down: counterclockwise on the page.
    (x * c - y * s, x * s + y * c)
}

fn checkerboard(img: &mut RgbaImage, p: &Params, px_per_mm: f32) -> Result<(), String> {
    let size = p.f32("size").max(1e-3);
    let (a, b, deg) = (p.color("color1"), p.color("color2"), p.f32("angle"));
    let px = 1.0 / (px_per_mm * size);
    fill(img, px_per_mm, |x, y| {
        let (u, v) = turned(x, y, deg);
        let t = 0.5 - 0.5 * square_wave(u / size, px) * square_wave(v / size, px);
        mix(a, b, t)
    });
    Ok(())
}

fn stripes(img: &mut RgbaImage, p: &Params, px_per_mm: f32) -> Result<(), String> {
    let size = p.f32("size").max(1e-3);
    let (a, b, deg) = (p.color("color1"), p.color("color2"), p.f32("angle"));
    let px = 1.0 / (px_per_mm * size);
    fill(img, px_per_mm, |x, y| {
        let (_, v) = turned(x, y, deg);
        mix(a, b, 0.5 + 0.5 * square_wave(v / size, px))
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::effects::EffectInstance;

    fn run(id: &str, w: u32, h: u32, px_per_mm: f32) -> RgbaImage {
        let gens = all();
        let g = gens.iter().find(|g| g.id() == id).unwrap();
        let inst = EffectInstance::generator(g.as_ref());
        let mut img = RgbaImage::new(w, h);
        g.generate(&mut img, &Params::new(g.params(), &inst.params), px_per_mm)
            .unwrap();
        img
    }

    #[test]
    fn every_generator_fills_the_image_with_both_colors() {
        for g in all() {
            let img = run(g.id(), 200, 200, 4.0);
            assert!(img.pixels().all(|p| p[3] == 255), "{}", g.id());
            let dark = img.pixels().filter(|p| p[0] < 100).count();
            let light = img.pixels().filter(|p| p[0] > 155).count();
            assert!(dark > 1000 && light > 1000, "{}: {dark} {light}", g.id());
        }
    }

    #[test]
    fn same_picture_at_any_resolution() {
        for g in all() {
            let low = run(g.id(), 100, 100, 1.0);
            let high = run(g.id(), 400, 400, 4.0);
            // Pixels in the middle of the cells, away from the edges; noise has 0.2 mm
            // grains, smaller than a pixel here: it is left out.
            if g.id() == "builtin.noise" {
                continue;
            }
            let mut differ = 0;
            for (x, y) in [(13, 27), (50, 50), (71, 8), (90, 61)] {
                let a = low.get_pixel(x, y)[0] as i32;
                let b = high.get_pixel(x * 4 + 2, y * 4 + 2)[0] as i32;
                differ += ((a - b).abs() > 40) as i32;
            }
            assert!(differ <= 1, "{}", g.id());
        }
    }

    #[test]
    fn the_seed_changes_the_noise() {
        let a = hash(3, 4, 1);
        assert_eq!(a, hash(3, 4, 1));
        assert_ne!(a, hash(3, 4, 2));
        assert!((0.0..1.0).contains(&a));
    }
}
