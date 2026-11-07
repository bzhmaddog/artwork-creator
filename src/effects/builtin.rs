//! Effects that ship with the app.

use image::RgbaImage;

use super::{Effect, ParamSpec, Params, par_pixels, par_rows};
use crate::i18n::{tr_noop, translate};

type ApplyFn = fn(&mut RgbaImage, &Params, f32) -> Result<(), String>;
type PaddingFn = fn(&Params) -> [f32; 4];

/// An effect made of a function and its parameter list.
struct Builtin {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    params: Vec<ParamSpec>,
    apply: ApplyFn,
    padding: PaddingFn,
}

impl Effect for Builtin {
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
    fn apply(&self, img: &mut RgbaImage, params: &Params, scale: f32) -> Result<(), String> {
        (self.apply)(img, params, scale)
    }
    fn padding(&self, params: &Params) -> [f32; 4] {
        (self.padding)(params)
    }
}

fn effect(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    params: Vec<ParamSpec>,
    apply: ApplyFn,
) -> Box<dyn Effect> {
    Box::new(Builtin {
        id,
        name,
        description,
        params,
        apply,
        padding: |_| [0.0; 4],
    })
}

fn percent(id: &str, label: &str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec::float(id, label, min, max, default)
        .unit("%")
        .decimals(0)
}

/// The built-in effects, in menu order.
pub fn all() -> Vec<Box<dyn Effect>> {
    vec![
        effect(
            "builtin.brightness_contrast",
            tr_noop!("EFFECT_BRIGHTNESS_CONTRAST"),
            tr_noop!("EFFECT_BRIGHTNESS_CONTRAST_HINT"),
            vec![
                percent(
                    "brightness",
                    tr_noop!("PARAM_BRIGHTNESS"),
                    -100.0,
                    100.0,
                    0.0,
                ),
                percent("contrast", tr_noop!("PARAM_CONTRAST"), -100.0, 100.0, 0.0),
            ],
            brightness_contrast,
        ),
        effect(
            "builtin.hue_saturation",
            tr_noop!("EFFECT_HUE_SATURATION"),
            tr_noop!("EFFECT_HUE_SATURATION_HINT"),
            vec![
                ParamSpec::float("hue", tr_noop!("PARAM_HUE"), -180.0, 180.0, 0.0)
                    .unit("°")
                    .decimals(0),
                percent(
                    "saturation",
                    tr_noop!("PARAM_SATURATION"),
                    -100.0,
                    100.0,
                    0.0,
                ),
                percent("lightness", tr_noop!("PARAM_LIGHTNESS"), -100.0, 100.0, 0.0),
            ],
            hue_saturation,
        ),
        effect(
            "builtin.grayscale",
            tr_noop!("EFFECT_GRAYSCALE"),
            tr_noop!("EFFECT_GRAYSCALE_HINT"),
            vec![percent(
                "amount",
                tr_noop!("PARAM_AMOUNT"),
                0.0,
                100.0,
                100.0,
            )],
            grayscale,
        ),
        effect(
            "builtin.sepia",
            tr_noop!("EFFECT_SEPIA"),
            tr_noop!("EFFECT_SEPIA_HINT"),
            vec![percent(
                "amount",
                tr_noop!("PARAM_AMOUNT"),
                0.0,
                100.0,
                100.0,
            )],
            sepia,
        ),
        effect(
            "builtin.tint",
            tr_noop!("EFFECT_TINT"),
            tr_noop!("EFFECT_TINT_HINT"),
            vec![
                ParamSpec::color("color", tr_noop!("PARAM_COLOR"), [255, 140, 0, 255]),
                ParamSpec::choice(
                    "mode",
                    tr_noop!("PARAM_MODE"),
                    &[
                        ("colorize", tr_noop!("TINT_MODE_COLORIZE")),
                        ("multiply", tr_noop!("TINT_MODE_MULTIPLY")),
                        ("screen", tr_noop!("TINT_MODE_SCREEN")),
                    ],
                    "colorize",
                ),
                percent("amount", tr_noop!("PARAM_AMOUNT"), 0.0, 100.0, 100.0),
            ],
            tint,
        ),
        effect(
            "builtin.invert",
            tr_noop!("EFFECT_INVERT"),
            tr_noop!("EFFECT_INVERT_HINT"),
            vec![],
            invert,
        ),
        effect(
            "builtin.posterize",
            tr_noop!("EFFECT_POSTERIZE"),
            tr_noop!("EFFECT_POSTERIZE_HINT"),
            vec![ParamSpec::int("levels", tr_noop!("PARAM_LEVELS"), 2, 64, 4)],
            posterize,
        ),
        effect(
            "builtin.threshold",
            tr_noop!("EFFECT_THRESHOLD"),
            tr_noop!("EFFECT_THRESHOLD_HINT"),
            vec![
                ParamSpec::int("level", tr_noop!("PARAM_LEVEL"), 0, 255, 128),
                ParamSpec::bool("invert", tr_noop!("PARAM_WHITE_ON_BLACK"), false),
            ],
            threshold,
        ),
        Box::new(Builtin {
            id: "builtin.drop_shadow",
            name: tr_noop!("EFFECT_DROP_SHADOW"),
            description: tr_noop!("EFFECT_DROP_SHADOW_HINT"),
            params: vec![
                ParamSpec::color("color", tr_noop!("PARAM_COLOR"), [0, 0, 0, 255]),
                percent("opacity", tr_noop!("PARAM_OPACITY"), 0.0, 100.0, 60.0),
                ParamSpec::float("angle", tr_noop!("PARAM_LIGHT_ANGLE"), -180.0, 180.0, 135.0)
                    .unit("°")
                    .decimals(0),
                ParamSpec::float("distance", tr_noop!("PARAM_DISTANCE"), 0.0, 1000.0, 20.0)
                    .unit(" px")
                    .decimals(0),
                ParamSpec::float("softness", tr_noop!("PARAM_SOFTNESS"), 0.0, 200.0, 10.0)
                    .unit(" px")
                    .decimals(1),
            ],
            apply: drop_shadow,
            padding: shadow_padding,
        }),
        Box::new(Builtin {
            id: "builtin.outline",
            name: tr_noop!("EFFECT_OUTLINE"),
            description: tr_noop!("EFFECT_OUTLINE_HINT"),
            params: vec![
                ParamSpec::color("color", tr_noop!("PARAM_COLOR"), [255, 255, 255, 255]),
                ParamSpec::float("width", tr_noop!("PARAM_WIDTH"), 0.0, 500.0, 8.0)
                    .unit(" px")
                    .decimals(1),
                ParamSpec::choice(
                    "position",
                    tr_noop!("PARAM_POSITION"),
                    &[
                        ("outside", tr_noop!("OUTLINE_POSITION_OUTSIDE")),
                        ("center", tr_noop!("OUTLINE_POSITION_CENTER")),
                        ("inside", tr_noop!("OUTLINE_POSITION_INSIDE")),
                    ],
                    "outside",
                ),
                percent("opacity", tr_noop!("PARAM_OPACITY"), 0.0, 100.0, 100.0),
            ],
            apply: outline,
            padding: outline_padding,
        }),
        effect(
            "builtin.blur",
            tr_noop!("EFFECT_GAUSSIAN_BLUR"),
            tr_noop!("EFFECT_GAUSSIAN_BLUR_HINT"),
            vec![
                ParamSpec::float("radius", tr_noop!("PARAM_RADIUS"), 0.0, 200.0, 4.0)
                    .unit(" px")
                    .decimals(1),
            ],
            blur,
        ),
        effect(
            "builtin.sharpen",
            tr_noop!("EFFECT_SHARPEN"),
            tr_noop!("EFFECT_SHARPEN_HINT"),
            vec![
                percent("amount", tr_noop!("PARAM_AMOUNT"), 0.0, 500.0, 100.0),
                ParamSpec::float("radius", tr_noop!("PARAM_RADIUS"), 0.1, 50.0, 2.0)
                    .unit(" px")
                    .decimals(1),
                ParamSpec::int("threshold", tr_noop!("PARAM_THRESHOLD"), 0, 255, 0),
            ],
            sharpen,
        ),
    ]
}

// ---- Helpers ----

fn clamp_u8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Rec. 601 luma, 0 to 255.
fn luma(px: &[u8; 4]) -> f32 {
    0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32
}

/// Maps the color channels through a lookup table.
fn apply_lut(img: &mut RgbaImage, lut: &[u8; 256]) {
    par_pixels(img, |px| {
        for c in &mut px[..3] {
            *c = lut[*c as usize];
        }
    });
}

fn lut(f: impl Fn(f32) -> f32) -> [u8; 256] {
    std::array::from_fn(|i| clamp_u8(f(i as f32)))
}

/// Mixes the color channels of `px` towards `to` by `t` (0 to 1).
fn mix(px: &mut [u8; 4], to: [f32; 3], t: f32) {
    for (c, v) in px[..3].iter_mut().zip(to) {
        *c = clamp_u8(*c as f32 + (v - *c as f32) * t);
    }
}

// ---- Color effects ----

fn brightness_contrast(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let b = p.f32("brightness") / 100.0 * 255.0;
    // Contrast as in the common "contrast correction" formula, -255 to 255.
    let c = (p.f32("contrast") / 100.0 * 255.0).clamp(-254.0, 254.0);
    let k = 259.0 * (c + 255.0) / (255.0 * (259.0 - c));
    apply_lut(img, &lut(|v| k * (v + b - 128.0) + 128.0));
    Ok(())
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

fn hue_saturation(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let dh = p.f32("hue");
    let ds = p.f32("saturation") / 100.0;
    let dl = p.f32("lightness") / 100.0;
    par_pixels(img, |px| {
        let [r, g, b] = [px[0], px[1], px[2]].map(|v| v as f32 / 255.0);
        let (h, s, l) = rgb_to_hsl(r, g, b);
        let s = if ds >= 0.0 {
            s + (1.0 - s) * ds
        } else {
            s * (1.0 + ds)
        };
        let l = if dl >= 0.0 {
            l + (1.0 - l) * dl
        } else {
            l * (1.0 + dl)
        };
        let rgb = hsl_to_rgb(h + dh, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        for (c, v) in px[..3].iter_mut().zip(rgb) {
            *c = clamp_u8(v * 255.0);
        }
    });
    Ok(())
}

fn grayscale(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let t = p.f32("amount") / 100.0;
    par_pixels(img, |px| {
        let y = luma(px);
        mix(px, [y; 3], t);
    });
    Ok(())
}

fn sepia(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let t = p.f32("amount") / 100.0;
    par_pixels(img, |px| {
        let [r, g, b] = [px[0], px[1], px[2]].map(f32::from);
        let to = [
            0.393 * r + 0.769 * g + 0.189 * b,
            0.349 * r + 0.686 * g + 0.168 * b,
            0.272 * r + 0.534 * g + 0.131 * b,
        ];
        mix(px, to, t);
    });
    Ok(())
}

fn tint(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let [tr, tg, tb, ta] = p.color("color");
    let tc = [tr, tg, tb].map(f32::from);
    let t = p.f32("amount") / 100.0 * ta as f32 / 255.0;
    let mode = p.choice("mode");
    par_pixels(img, |px| {
        let to = match mode {
            "multiply" => std::array::from_fn(|i| px[i] as f32 * tc[i] / 255.0),
            "screen" => {
                std::array::from_fn(|i| 255.0 - (255.0 - px[i] as f32) * (255.0 - tc[i]) / 255.0)
            }
            // Colorize: the tint color, as light or dark as the pixel.
            _ => {
                let y = luma(px) / 255.0;
                tc.map(|c| {
                    if y < 0.5 {
                        c * 2.0 * y
                    } else {
                        c + (255.0 - c) * (2.0 * y - 1.0)
                    }
                })
            }
        };
        mix(px, to, t);
    });
    Ok(())
}

fn invert(img: &mut RgbaImage, _: &Params, _: f32) -> Result<(), String> {
    apply_lut(img, &lut(|v| 255.0 - v));
    Ok(())
}

fn posterize(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let n = (p.i32("levels").max(2) - 1) as f32;
    apply_lut(img, &lut(|v| (v / 255.0 * n).round() / n * 255.0));
    Ok(())
}

fn threshold(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let level = p.i32("level") as f32;
    let invert = p.bool("invert");
    par_pixels(img, |px| {
        let v = if (luma(px) >= level) != invert {
            255
        } else {
            0
        };
        px[..3].fill(v);
    });
    Ok(())
}

// ---- Blur ----

/// Radii of three box blurs that together approximate a Gaussian of `sigma`.
fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let (wlf, s2) = (wl as f32, sigma * sigma);
    let m = ((12.0 * s2 - n * wlf * wlf - 4.0 * n * wlf - 3.0 * n) / (-4.0 * wlf - 4.0)).round();
    std::array::from_fn(|i| {
        let w = if (i as f32) < m { wl } else { wu };
        ((w - 1) / 2) as usize
    })
}

/// Box blurs every row of 4-channel pixels with each radius in turn, edges extended.
/// All passes run on a row while it is in cache.
fn box_rows(data: &mut [u8], w: usize, radii: &[usize]) {
    if w == 0 {
        return;
    }
    par_rows(data, w * 4, |_, chunk| {
        let mut src = vec![[0u8; 4]; w];
        for row in chunk.chunks_exact_mut(w * 4) {
            for &r in radii.iter().filter(|&&r| r > 0) {
                box_row(row, &mut src, r);
            }
        }
    });
}

/// One box blur of a row; `src` is scratch space of the row's length.
fn box_row(row: &mut [u8], src: &mut [[u8; 4]], r: usize) {
    let w = src.len();
    let div = (2 * r + 1) as u32;
    // Fixed-point reciprocal: (sum * inv) >> 24 == sum / div, rounded.
    let inv = ((1u64 << 24) / div as u64) as u32;
    let half = div / 2;
    src.copy_from_slice(row.as_chunks::<4>().0);
    let px = |x: isize| src[x.clamp(0, w as isize - 1) as usize];
    let mut sum = [0u32; 4];
    for x in -(r as isize)..=r as isize {
        let p = px(x);
        for c in 0..4 {
            sum[c] += p[c] as u32;
        }
    }
    let out = row.as_chunks_mut::<4>().0;
    for (x, o) in out.iter_mut().enumerate() {
        for c in 0..4 {
            o[c] = (((sum[c] + half) as u64 * inv as u64) >> 24) as u8;
        }
        let (add, sub) = (x + r + 1, x as isize - r as isize);
        let a = if add < w { src[add] } else { src[w - 1] };
        let b = if sub >= 0 { src[sub as usize] } else { src[0] };
        for c in 0..4 {
            sum[c] = sum[c] + a[c] as u32 - b[c] as u32;
        }
    }
}

/// Swaps rows and columns of a `w` x `h` grid of pixels.
fn transpose<P: Copy + Default + Send + Sync>(data: &[P], w: usize, h: usize) -> Vec<P> {
    let mut out = vec![P::default(); data.len()];
    // Each output row is an input column; tiles of rows keep the reads cache-friendly.
    par_rows(&mut out, h, |x0, chunk| {
        let cols = chunk.len() / h.max(1);
        const B: usize = 32;
        for by in (0..h).step_by(B) {
            for dx in 0..cols {
                let x = x0 + dx;
                for y in by..(by + B).min(h) {
                    chunk[dx * h + y] = data[y * w + x];
                }
            }
        }
    });
    out
}

/// Swaps rows and columns of RGBA pixels.
fn transpose_rgba(data: &[u8], w: usize, h: usize) -> Vec<u8> {
    transpose(data.as_chunks::<4>().0, w, h).into_flattened()
}

/// Gaussian blur of straight RGBA pixels, done on premultiplied values so transparent
/// pixels don't bleed their color.
fn gaussian(img: &RgbaImage, sigma: f32) -> RgbaImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut data = img.as_raw().clone();
    if sigma < 0.3 || w == 0 || h == 0 {
        return img.clone();
    }
    par_rows(&mut data, w * 4, |_, chunk| {
        for px in chunk.as_chunks_mut::<4>().0 {
            let a = px[3] as u16;
            for c in &mut px[..3] {
                *c = ((*c as u16 * a + 127) / 255) as u8;
            }
        }
    });
    let radii = box_radii(sigma);
    box_rows(&mut data, w, &radii);
    let mut t = transpose_rgba(&data, w, h);
    box_rows(&mut t, h, &radii);
    let mut data = transpose_rgba(&t, h, w);
    par_rows(&mut data, w * 4, |_, chunk| {
        for px in chunk.as_chunks_mut::<4>().0 {
            let a = px[3] as u16;
            if a == 0 {
                px[..3].fill(0);
            } else if a < 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u16 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    });
    RgbaImage::from_raw(w as u32, h as u32, data).unwrap_or_else(|| img.clone())
}

fn blur(img: &mut RgbaImage, p: &Params, scale: f32) -> Result<(), String> {
    *img = gaussian(img, p.f32("radius") * scale);
    Ok(())
}

fn sharpen(img: &mut RgbaImage, p: &Params, scale: f32) -> Result<(), String> {
    let amount = p.f32("amount") / 100.0;
    let threshold = p.i32("threshold");
    let blurred = gaussian(img, p.f32("radius") * scale);
    let row = img.width() as usize * 4;
    let b = blurred.as_raw();
    par_rows(img, row, |y0, chunk| {
        let b = &b[y0 * row..y0 * row + chunk.len()];
        for (px, bp) in chunk
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(b.as_chunks::<4>().0)
        {
            for c in 0..3 {
                let d = px[c] as i32 - bp[c] as i32;
                if d.abs() >= threshold {
                    px[c] = clamp_u8(px[c] as f32 + d as f32 * amount);
                }
            }
        }
    });
    Ok(())
}

// ---- Drop shadow ----

/// How far the shadow is moved, in source pixels. The light comes from `angle`
/// (counterclockwise from the right, as in most editors): at 135° it comes from the
/// top left and the shadow falls to the bottom right.
fn shadow_offset(p: &Params) -> (f32, f32) {
    let (a, d) = (p.f32("angle").to_radians(), p.f32("distance"));
    (-a.cos() * d, a.sin() * d)
}

fn shadow_padding(p: &Params) -> [f32; 4] {
    let (dx, dy) = shadow_offset(p);
    // A Gaussian is all but gone at three times its radius.
    let soft = 3.0 * p.f32("softness");
    [
        (-dx).max(0.0) + soft,
        (-dy).max(0.0) + soft,
        dx.max(0.0) + soft,
        dy.max(0.0) + soft,
    ]
}

fn drop_shadow(img: &mut RgbaImage, p: &Params, scale: f32) -> Result<(), String> {
    let (dx, dy) = shadow_offset(p);
    let (dx, dy) = ((dx * scale).round() as i64, (dy * scale).round() as i64);
    let [r, g, b, a] = p.color("color");
    let k = p.f32("opacity") / 100.0 * a as f32 / 255.0;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let row = w * 4;

    // The image's alpha, moved and tinted.
    let src = img.as_raw();
    let mut shadow = RgbaImage::new(w as u32, h as u32);
    par_rows(&mut shadow, row, |y0, chunk| {
        for (j, line) in chunk.chunks_exact_mut(row).enumerate() {
            let sy = (y0 + j) as i64 - dy;
            for (x, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let sx = x as i64 - dx;
                let alpha = if (0..w as i64).contains(&sx) && (0..h as i64).contains(&sy) {
                    src[(sy as usize * w + sx as usize) * 4 + 3]
                } else {
                    0
                };
                *px = [r, g, b, clamp_u8(alpha as f32 * k)];
            }
        }
    });
    let shadow = gaussian(&shadow, p.f32("softness") * scale);

    // The image over its shadow.
    let sh = shadow.as_raw();
    par_rows(img, row, |y0, chunk| {
        let sh = &sh[y0 * row..y0 * row + chunk.len()];
        for (px, s) in chunk
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(sh.as_chunks::<4>().0)
        {
            let (ia, sa) = (px[3] as f32 / 255.0, s[3] as f32 / 255.0);
            let oa = ia + sa * (1.0 - ia);
            if oa <= 0.0 {
                continue;
            }
            for c in 0..3 {
                px[c] = clamp_u8((px[c] as f32 * ia + s[c] as f32 * sa * (1.0 - ia)) / oa);
            }
            px[3] = clamp_u8(oa * 255.0);
        }
    });
    Ok(())
}

// ---- Outline ----

/// Exact squared Euclidean distance transform of one row (Felzenszwalb & Huttenlocher):
/// `f` holds 0 at feature pixels and `INF` elsewhere, and becomes the squared distance
/// to the nearest feature. `v`, `z` and `d` are scratch space.
fn edt_row(f: &mut [f32], v: &mut Vec<usize>, z: &mut Vec<f32>, d: &mut Vec<f32>) {
    let n = f.len();
    v.clear();
    z.clear();
    d.clear();
    d.extend_from_slice(f);
    let Some(first) = (0..n).find(|&q| f[q] < EDT_INF) else {
        return;
    };
    v.push(first);
    z.push(f32::NEG_INFINITY);
    z.push(f32::INFINITY);
    for q in first + 1..n {
        if f[q] >= EDT_INF {
            continue;
        }
        loop {
            let p = *v.last().unwrap();
            let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q - p) as f32);
            if s <= z[v.len() - 1] {
                v.pop();
                z.pop();
                if v.is_empty() {
                    z.push(f32::NEG_INFINITY);
                    break;
                }
            } else {
                break;
            }
        }
        v.push(q);
        let k = v.len() - 1;
        if k > 0 {
            let p = v[k - 1];
            z[k] = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q - p) as f32);
        }
        z.push(f32::INFINITY);
    }
    let mut k = 0;
    for (q, out) in f.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dq = q as f32 - v[k] as f32;
        *out = dq * dq + d[v[k]];
    }
}

/// Stands for "no feature": larger than any squared distance in an image.
const EDT_INF: f32 = 1e20;

/// Distance from each pixel to the nearest pixel where `feature` is true. With
/// `border`, the pixels just past the image's edges count as features too.
fn distance_map(
    w: usize,
    h: usize,
    border: bool,
    feature: impl Fn(usize) -> bool + Sync,
) -> Vec<f32> {
    // One more pixel on each side, for the border.
    let (w0, h0) = (w, h);
    let (w, h) = (w + 2, h + 2);
    let mut f = vec![0.0f32; w * h];
    par_rows(&mut f, w, |y0, chunk| {
        for (j, row) in chunk.chunks_exact_mut(w).enumerate() {
            let y = y0 + j;
            for (x, v) in row.iter_mut().enumerate() {
                let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                let hit = if edge {
                    border
                } else {
                    feature((y - 1) * w0 + x - 1)
                };
                *v = if hit { 0.0 } else { EDT_INF };
            }
        }
    });
    let rows = |data: &mut Vec<f32>, len: usize| {
        par_rows(data, len, |_, chunk| {
            let (mut v, mut z, mut d) = (Vec::new(), Vec::new(), Vec::new());
            for row in chunk.chunks_exact_mut(len) {
                edt_row(row, &mut v, &mut z, &mut d);
            }
        });
    };
    rows(&mut f, w);
    let mut t = transpose(&f, w, h);
    rows(&mut t, h);
    let full = transpose(&t, h, w);
    let mut out = vec![0.0f32; w0 * h0];
    par_rows(&mut out, w0, |y0, chunk| {
        for (j, row) in chunk.chunks_exact_mut(w0).enumerate() {
            let src = &full[(y0 + j + 1) * w + 1..][..w0];
            for (o, v) in row.iter_mut().zip(src) {
                *o = v.sqrt();
            }
        }
    });
    out
}

fn outline_padding(p: &Params) -> [f32; 4] {
    let w = p.f32("width");
    let out = match p.choice("position") {
        "inside" => 0.0,
        "center" => w / 2.0 + 1.0,
        _ => w + 1.0,
    };
    [out; 4]
}

fn outline(img: &mut RgbaImage, p: &Params, scale: f32) -> Result<(), String> {
    let width = p.f32("width") * scale;
    if width <= 0.0 {
        return Ok(());
    }
    let [r, g, b, a] = p.color("color");
    let k = p.f32("opacity") / 100.0 * a as f32 / 255.0;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let src = img.as_raw();
    let inside = |i: usize| src[i * 4 + 3] >= 128;
    // Signed distance to the shape's edge: positive outside, negative inside. Pixel
    // centers lie half a pixel from the edge between them.
    let position = p.choice("position");
    // The image's own edges count as the shape's edge from inside, so a fully opaque
    // image is outlined along its border.
    let d_out = (position != "inside").then(|| distance_map(w, h, false, inside));
    let d_in = (position != "outside").then(|| distance_map(w, h, true, |i| !inside(i)));
    let signed = |i: usize| {
        if inside(i) {
            -(d_in.as_ref().map_or(EDT_INF, |d| d[i]) - 0.5)
        } else {
            d_out.as_ref().map_or(EDT_INF, |d| d[i]) - 0.5
        }
    };
    // Where the line is, as a band of signed distances; edges anti-aliased over a pixel.
    let (lo, hi) = match position {
        // Under the image, so it meets the shape without a gap.
        "outside" => (f32::NEG_INFINITY, width),
        "inside" => (-width, 0.0),
        _ => (-width / 2.0, width / 2.0),
    };
    let mut coverage = vec![0.0f32; w * h];
    par_rows(&mut coverage, w, |y0, chunk| {
        for (j, c) in chunk.iter_mut().enumerate() {
            let s = signed(y0 * w + j);
            *c = ((s - lo).min(hi - s) + 0.5).clamp(0.0, 1.0) * k;
        }
    });

    let row = w * 4;
    par_rows(img, row, |y0, chunk| {
        let cov = &coverage[y0 * w..y0 * w + chunk.len() / 4];
        for (px, &c) in chunk.as_chunks_mut::<4>().0.iter_mut().zip(cov) {
            if c <= 0.0 {
                continue;
            }
            let ia = px[3] as f32 / 255.0;
            let (top, top_a, bottom, bottom_a) = if position == "outside" {
                // Image over the line.
                ([px[0], px[1], px[2]], ia, [r, g, b], c)
            } else {
                // Line over the image, clipped to it when inside.
                let c = if position == "inside" { c * ia } else { c };
                ([r, g, b], c, [px[0], px[1], px[2]], ia)
            };
            let oa = top_a + bottom_a * (1.0 - top_a);
            if oa <= 0.0 {
                continue;
            }
            for i in 0..3 {
                px[i] = clamp_u8(
                    (top[i] as f32 * top_a + bottom[i] as f32 * bottom_a * (1.0 - top_a)) / oa,
                );
            }
            px[3] = clamp_u8(oa * 255.0);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{EffectInstance, apply, registry};

    fn run(id: &str, img: &mut RgbaImage, set: &[(&str, serde_json::Value)], scale: f32) {
        let mut inst = EffectInstance::new(registry().get(id).unwrap());
        for (k, v) in set {
            inst.params.insert((*k).into(), v.clone());
        }
        assert!(apply(img, &[inst], scale).errors.is_empty());
    }

    #[test]
    fn every_builtin_has_a_unique_id_and_runs_with_defaults() {
        let all = all();
        let mut ids: Vec<_> = all.iter().map(|e| e.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for e in &all {
            let mut img = RgbaImage::from_fn(9, 7, |x, y| {
                image::Rgba([
                    (x * 28) as u8,
                    (y * 36) as u8,
                    90,
                    if x == 0 { 0 } else { 255 },
                ])
            });
            let inst = EffectInstance::new(e.as_ref());
            let pad = crate::effects::padding(std::slice::from_ref(&inst), 1.0);
            run(e.id(), &mut img, &[], 1.0);
            // The same size, plus the margins the effect asks for.
            let size = (9 + pad[0] + pad[2], 7 + pad[1] + pad[3]);
            assert_eq!(img.dimensions(), size, "{}", e.id());
            // Effects keep alpha, except those that spread it.
            let spreads = ["blur", "sharpen", "shadow"];
            if !spreads.iter().any(|s| e.id().contains(s)) {
                assert_eq!(img.get_pixel(0, 0).0[3], 0, "{}", e.id());
            }
        }
    }

    #[test]
    fn grayscale_and_invert() {
        let mut img = RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        run("builtin.grayscale", &mut img, &[], 1.0);
        let [r, g, b, _] = img.get_pixel(0, 0).0;
        assert!(r == g && g == b && r == 76);
        run("builtin.invert", &mut img, &[], 1.0);
        assert_eq!(img.get_pixel(0, 0).0, [179, 179, 179, 255]);
    }

    #[test]
    fn hue_rotation_turns_red_green() {
        let mut img = RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        run(
            "builtin.hue_saturation",
            &mut img,
            &[("hue", 120.into())],
            1.0,
        );
        assert_eq!(img.get_pixel(0, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn blur_spreads_a_dot_and_keeps_flat_areas() {
        let mut img = RgbaImage::from_pixel(41, 41, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(20, 20, image::Rgba([255, 255, 255, 255]));
        run("builtin.blur", &mut img, &[("radius", 3.0.into())], 1.0);
        let center = img.get_pixel(20, 20).0[0];
        let near = img.get_pixel(23, 20).0[0];
        assert!(center < 255 && center > near && near > 0);
        // Symmetric.
        assert_eq!(img.get_pixel(17, 20).0, img.get_pixel(23, 20).0);
        assert_eq!(img.get_pixel(20, 17).0, img.get_pixel(20, 23).0);
        // Far away stays black.
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn blur_radius_follows_scale() {
        let dot = |n: u32| {
            let mut img = RgbaImage::from_pixel(n, 1, image::Rgba([0, 0, 0, 255]));
            img.put_pixel(n / 2, 0, image::Rgba([255, 255, 255, 255]));
            img
        };
        // A half-size image blurred with scale 0.5 keeps more of the peak than at
        // scale 1, since its radius in pixels is halved.
        let (mut a, mut b) = (dot(81), dot(81));
        run("builtin.blur", &mut a, &[("radius", 8.0.into())], 1.0);
        run("builtin.blur", &mut b, &[("radius", 8.0.into())], 0.5);
        assert!(b.get_pixel(40, 0).0[0] > a.get_pixel(40, 0).0[0]);
    }

    #[test]
    fn transparent_pixels_dont_bleed_color() {
        let mut img = RgbaImage::from_fn(20, 1, |x, _| {
            if x < 10 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 255, 0, 0])
            }
        });
        run("builtin.blur", &mut img, &[("radius", 2.0.into())], 1.0);
        let px = img.get_pixel(11, 0).0;
        assert!(px[3] > 0 && px[1] == 0, "{px:?}");
    }

    #[test]
    fn drop_shadow_grows_the_image_and_falls_bottom_right() {
        // A 10 x 10 opaque white square; a hard shadow 4 px right and down.
        let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        let inst = {
            let mut i = EffectInstance::new(registry().get("builtin.drop_shadow").unwrap());
            i.params.insert("angle".into(), 135.0.into());
            i.params.insert("distance".into(), 32f32.sqrt().into());
            i.params.insert("softness".into(), 0.0.into());
            i.params.insert("opacity".into(), 100.0.into());
            i
        };
        let done = apply(&mut img, &[inst], 1.0);
        assert!(done.errors.is_empty());
        assert_eq!(done.pad, [0, 0, 4, 4]);
        assert_eq!(img.dimensions(), (14, 14));
        // The image is unchanged and on top.
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255, 255]);
        // The shadow shows in the new margin, not above or left of the image.
        assert_eq!(img.get_pixel(12, 12).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(12, 2).0[3], 0);
        assert_eq!(img.get_pixel(2, 12).0[3], 0);
    }

    #[test]
    fn soft_shadow_padding_follows_scale() {
        let e = registry().get("builtin.drop_shadow").unwrap();
        let mut i = EffectInstance::new(e);
        i.params.insert("angle".into(), 0.0.into()); // light from the right
        i.params.insert("distance".into(), 10.0.into());
        i.params.insert("softness".into(), 2.0.into());
        let stack = [i];
        // Shadow to the left: 10 px plus three times the softness; 6 px elsewhere.
        assert_eq!(crate::effects::padding(&stack, 1.0), [16, 6, 6, 6]);
        assert_eq!(crate::effects::padding(&stack, 0.5), [8, 3, 3, 3]);
    }

    #[test]
    fn distance_map_is_euclidean() {
        // One feature pixel in the middle of a 9 x 9 grid.
        let d = distance_map(9, 9, false, |i| i == 4 * 9 + 4);
        assert_eq!(d[4 * 9 + 4], 0.0);
        assert_eq!(d[4 * 9 + 7], 3.0);
        assert!((d[0] - 32f32.sqrt()).abs() < 1e-5);
        // Two features: the nearest one counts.
        let d = distance_map(10, 1, false, |i| i == 0 || i == 9);
        assert_eq!(d, [0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0]);
        // With the border: no feature inside, distances to the nearest edge.
        let d = distance_map(5, 1, true, |_| false);
        assert_eq!(d, [1.0, 1.0, 1.0, 1.0, 1.0]);
        let d = distance_map(5, 3, true, |_| false);
        assert_eq!(d[5..10], [1.0, 2.0, 2.0, 2.0, 1.0]);
    }

    /// A 10 x 10 opaque red square with an outline of `width` px at `position`.
    fn outlined(position: &str, width: f32) -> (RgbaImage, [u32; 4]) {
        let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 0, 0, 255]));
        let mut inst = EffectInstance::new(registry().get("builtin.outline").unwrap());
        inst.params.insert("position".into(), position.into());
        inst.params.insert("width".into(), width.into());
        let done = apply(&mut img, &[inst], 1.0);
        assert!(done.errors.is_empty());
        (img, done.pad)
    }

    #[test]
    fn outline_outside_surrounds_the_shape() {
        let (img, pad) = outlined("outside", 3.0);
        assert_eq!(pad, [4; 4]);
        let at = |x: u32, y: u32| img.get_pixel(x + 4, y + 4).0;
        // The square is untouched; the line is white around it, then nothing.
        assert_eq!(at(0, 0), [255, 0, 0, 255]);
        assert_eq!(at(5, 5), [255, 0, 0, 255]);
        // Rows 10 to 12 lie below the square: the line covers its 3 px width there.
        for y in 10..13 {
            assert_eq!(at(5, y), [255, 255, 255, 255], "below, row {y}");
        }
        assert_eq!(img.get_pixel(4 + 5, 4 + 13).0[3], 0, "past the width");
    }

    #[test]
    fn outline_inside_stays_within_the_shape() {
        let (img, pad) = outlined("inside", 2.0);
        assert_eq!(pad, [0; 4]);
        assert_eq!(img.dimensions(), (10, 10));
        assert_eq!(img.get_pixel(0, 5).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(1, 5).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
    }

    #[test]
    fn outline_center_straddles_the_edge() {
        let (img, pad) = outlined("center", 4.0);
        assert_eq!(pad, [3; 4]);
        let at = |x: i64| img.get_pixel((x + 3) as u32, 3 + 5).0;
        assert_eq!(at(-2), [255, 255, 255, 255], "outside");
        assert_eq!(at(1), [255, 255, 255, 255], "inside");
        assert_eq!(at(5), [255, 0, 0, 255], "middle untouched");
        assert_eq!(at(-3)[3], 0, "beyond");
    }

    #[test]
    fn box_radii_match_sigma() {
        // Three boxes of radius r have a variance of 3 * ((2r+1)^2 - 1) / 12.
        // Small radii are coarser: boxes have whole-pixel widths.
        for sigma in [3.0f32, 10.0, 40.0] {
            let var: f32 = box_radii(sigma)
                .iter()
                .map(|&r| (((2 * r + 1).pow(2) - 1) as f32) / 12.0)
                .sum();
            assert!((var.sqrt() - sigma).abs() / sigma < 0.15, "{sigma}: {var}");
        }
    }
}
