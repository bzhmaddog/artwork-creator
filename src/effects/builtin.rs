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
                // The same names as a shape's stroke dashes.
                ParamSpec::choice(
                    "style",
                    tr_noop!("STROKE_DASHES"),
                    &[
                        ("solid", tr_noop!("STROKE_DASH_SOLID")),
                        ("dashed", tr_noop!("STROKE_DASH_DASHED")),
                        ("dotted", tr_noop!("STROKE_DASH_DOTTED")),
                        ("dash_dot", tr_noop!("STROKE_DASH_DASH_DOT")),
                    ],
                    "solid",
                ),
                percent("opacity", tr_noop!("PARAM_OPACITY"), 0.0, 100.0, 100.0),
            ],
            apply: outline,
            padding: outline_padding,
        }),
        effect(
            "builtin.fade_linear",
            tr_noop!("EFFECT_FADE_LINEAR"),
            tr_noop!("EFFECT_FADE_LINEAR_HINT"),
            vec![
                ParamSpec::choice(
                    "toward",
                    tr_noop!("PARAM_FADE_TOWARD"),
                    &[
                        ("right", tr_noop!("FADE_TOWARD_RIGHT")),
                        ("left", tr_noop!("FADE_TOWARD_LEFT")),
                        ("top", tr_noop!("FADE_TOWARD_TOP")),
                        ("bottom", tr_noop!("FADE_TOWARD_BOTTOM")),
                    ],
                    "right",
                ),
                percent("start", tr_noop!("PARAM_FADE_START"), 0.0, 100.0, 50.0),
                percent("end", tr_noop!("PARAM_FADE_END"), 0.0, 100.0, 100.0),
                ParamSpec::curve("curve", tr_noop!("PARAM_FADE_CURVE"), EASE_IN_OUT),
            ],
            fade_linear,
        ),
        effect(
            "builtin.fade_radial",
            tr_noop!("EFFECT_FADE_RADIAL"),
            tr_noop!("EFFECT_FADE_RADIAL_HINT"),
            vec![
                percent("center_x", tr_noop!("PARAM_CENTER_X"), 0.0, 100.0, 50.0),
                percent("center_y", tr_noop!("PARAM_CENTER_Y"), 0.0, 100.0, 50.0),
                percent("start", tr_noop!("PARAM_FADE_START"), 0.0, 200.0, 50.0),
                percent("end", tr_noop!("PARAM_FADE_END"), 0.0, 200.0, 100.0),
                ParamSpec::curve("curve", tr_noop!("PARAM_FADE_CURVE"), EASE_IN_OUT),
                ParamSpec::bool("inward", tr_noop!("PARAM_FADE_INWARD"), false),
            ],
            fade_radial,
        ),
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
/// The box around the visible pixels (alpha above 0): x0, y0, x1, y1, in pixels. The
/// image may have margins added for other effects; fades span the object, not those.
fn visible_bounds(img: &RgbaImage) -> Option<[f32; 4]> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let raw = img.as_raw();
    let mut b = [usize::MAX, usize::MAX, 0, 0];
    for y in 0..h {
        let row = &raw[y * w * 4..][..w * 4];
        let Some(x0) = row.as_chunks::<4>().0.iter().position(|p| p[3] > 0) else {
            continue;
        };
        let x1 = w
            - 1
            - row
                .as_chunks::<4>()
                .0
                .iter()
                .rev()
                .position(|p| p[3] > 0)
                .unwrap_or(0);
        b = [b[0].min(x0), b[1].min(y), b[2].max(x1 + 1), y + 1];
    }
    (b[0] != usize::MAX).then(|| b.map(|v| v as f32))
}

/// The fades' curve before it is changed: eased in and out.
const EASE_IN_OUT: [f32; 4] = super::CURVE_PRESETS[3].2;

/// How much of a pixel stays at `t` along a fade from `start` (still opaque) to `end`
/// (transparent), following `curve`. With `end` before `start`, a hard edge at `start`.
fn fade_factor(t: f32, start: f32, end: f32, curve: [f32; 4]) -> f32 {
    if end <= start {
        return if t < start { 1.0 } else { 0.0 };
    }
    1.0 - super::ease(curve, (t - start) / (end - start))
}

/// Multiplies each pixel's opacity by `keep(x, y)`, at the pixel's center.
fn fade_alpha(img: &mut RgbaImage, keep: impl Fn(f32, f32) -> f32 + Sync) {
    let row = img.width() as usize * 4;
    par_rows(img, row, |y0, chunk| {
        for (j, line) in chunk.chunks_exact_mut(row).enumerate() {
            let y = (y0 + j) as f32 + 0.5;
            for (x, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                if px[3] > 0 {
                    px[3] = clamp_u8(px[3] as f32 * keep(x as f32 + 0.5, y));
                }
            }
        }
    });
}

/// Fades the object to transparent toward one side, horizontally or vertically: across
/// its visible pixels, from `start` % of the way to `end` %.
fn fade_linear(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let Some([x0, y0, x1, y1]) = visible_bounds(img) else {
        return Ok(());
    };
    let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    // How far toward the fading side, from 0 on the opposite side to 1 on it.
    let toward: fn(f32, f32, [f32; 4]) -> f32 = match p.choice("toward") {
        "left" => |x, _, [x0, _, w, _]| 1.0 - (x - x0) / w,
        "top" => |_, y, [_, y0, _, h]| 1.0 - (y - y0) / h,
        "bottom" => |_, y, [_, y0, _, h]| (y - y0) / h,
        _ => |x, _, [x0, _, w, _]| (x - x0) / w,
    };
    let (start, end) = (p.f32("start") / 100.0, p.f32("end") / 100.0);
    let curve = p.curve("curve");
    fade_alpha(img, |x, y| {
        fade_factor(toward(x, y, [x0, y0, w, h]), start, end, curve)
    });
    Ok(())
}

/// Fades the object to transparent around a center: in an ellipse the shape of its
/// visible pixels' box, 100% reaching the box's sides. `inward` fades the center instead.
fn fade_radial(img: &mut RgbaImage, p: &Params, _: f32) -> Result<(), String> {
    let Some([x0, y0, x1, y1]) = visible_bounds(img) else {
        return Ok(());
    };
    let (w, h) = (x1 - x0, y1 - y0);
    let cx = x0 + w * p.f32("center_x") / 100.0;
    let cy = y0 + h * p.f32("center_y") / 100.0;
    let (rx, ry) = ((w / 2.0).max(1.0), (h / 2.0).max(1.0));
    let (start, end) = (p.f32("start") / 100.0, p.f32("end") / 100.0);
    let inward = p.bool("inward");
    let curve = p.curve("curve");
    fade_alpha(img, |x, y| {
        let r = (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt();
        let k = fade_factor(r, start, end, curve);
        if inward { 1.0 - k } else { k }
    });
    Ok(())
}

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
    edt_row_arg(f, None, v, z, d);
}

/// `edt_row`, also writing into `arg` where in the row each pixel's distance comes from.
fn edt_row_arg(
    f: &mut [f32],
    mut arg: Option<&mut [u32]>,
    v: &mut Vec<usize>,
    z: &mut Vec<f32>,
    d: &mut Vec<f32>,
) {
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
        if let Some(a) = arg.as_deref_mut() {
            a[q] = v[k] as u32;
        }
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

/// Where each pixel near a shape's edge is along that edge, for dashed outlines. The
/// edge is traced through the image's alpha (marching squares on a grid one pixel larger
/// on each side, so every contour closes), and each pixel takes the place along it of
/// its nearest pixel across the edge.
struct Along {
    /// Width of the padded grid.
    gw: usize,
    /// For each padded pixel beside the edge: its contour and a point of it there.
    at: Vec<Option<(u32, u32)>>,
    /// The contours' points (padded pixel coordinates, centers on whole numbers), each
    /// with its length from the contour's start.
    contours: Vec<Vec<([f32; 2], f32)>>,
    /// Length of each contour.
    lengths: Vec<f32>,
    /// For each padded pixel: the nearest padded pixel inside the shape.
    near_in: Vec<u32>,
    /// For each padded pixel: the nearest padded pixel outside the shape.
    near_out: Vec<u32>,
}

impl Along {
    fn new(img: &RgbaImage) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (gw, gh) = (w + 2, h + 2);
        let src = img.as_raw();
        let alpha = |x: usize, y: usize| -> f32 {
            if x == 0 || y == 0 || x == gw - 1 || y == gh - 1 {
                0.0
            } else {
                src[((y - 1) * w + x - 1) * 4 + 3] as f32
            }
        };
        let inside = |x: usize, y: usize| alpha(x, y) >= 128.0;

        // Edges of the grid the contour crosses: between pixel (x, y) and its right
        // neighbor (key 2 i) or the one below (key 2 i + 1). Each gets the point where
        // the alpha crosses half way, and is linked to the two other crossed edges of
        // the cells on both sides of it.
        let key = |x: usize, y: usize, down: bool| 2 * (y * gw + x) + down as usize;
        let mut point: std::collections::HashMap<usize, [f32; 2]> = Default::default();
        let mut links: std::collections::HashMap<usize, Vec<usize>> = Default::default();
        let cross = |x: usize, y: usize, down: bool| -> Option<[f32; 2]> {
            let (x1, y1) = if down { (x, y + 1) } else { (x + 1, y) };
            if inside(x, y) == inside(x1, y1) {
                return None;
            }
            let (a0, a1) = (alpha(x, y), alpha(x1, y1));
            let t = ((128.0 - a0) / (a1 - a0)).clamp(0.0, 1.0);
            Some(if down {
                [x as f32, y as f32 + t]
            } else {
                [x as f32 + t, y as f32]
            })
        };
        for y in 0..gh - 1 {
            for x in 0..gw - 1 {
                // The cell's edges: top, right, bottom, left.
                let edges = [
                    (key(x, y, false), cross(x, y, false)),
                    (key(x + 1, y, true), cross(x + 1, y, true)),
                    (key(x, y + 1, false), cross(x, y + 1, false)),
                    (key(x, y, true), cross(x, y, true)),
                ];
                let hit: Vec<usize> = (0..4).filter(|&e| edges[e].1.is_some()).collect();
                let pairs: Vec<(usize, usize)> = match hit.len() {
                    2 => vec![(hit[0], hit[1])],
                    4 => {
                        // A saddle: the cell's middle decides which corners are cut off.
                        let mid =
                            (alpha(x, y) + alpha(x + 1, y) + alpha(x, y + 1) + alpha(x + 1, y + 1))
                                / 4.0
                                >= 128.0;
                        if inside(x, y) == mid {
                            vec![(0, 1), (2, 3)]
                        } else {
                            vec![(0, 3), (1, 2)]
                        }
                    }
                    _ => Vec::new(),
                };
                for (a, b) in pairs {
                    let (ka, kb) = (edges[a].0, edges[b].0);
                    for (k, e) in [(ka, a), (kb, b)] {
                        if let Some(p) = edges[e].1 {
                            point.insert(k, p);
                        }
                    }
                    links.entry(ka).or_default().push(kb);
                    links.entry(kb).or_default().push(ka);
                }
            }
        }

        // Walk each closed contour, giving both pixels of every crossed edge their
        // length along it.
        let mut at: Vec<Option<(u32, u32)>> = vec![None; gw * gh];
        let mut contours: Vec<Vec<([f32; 2], f32)>> = Vec::new();
        let mut lengths = Vec::new();
        let mut seen: std::collections::HashSet<usize> = Default::default();
        let mut starts: Vec<usize> = links.keys().copied().collect();
        starts.sort_unstable();
        for start in starts {
            if !seen.insert(start) {
                continue;
            }
            let id = lengths.len() as u32;
            let (mut prev, mut cur) = (usize::MAX, start);
            let mut s = 0.0f32;
            let mut points = Vec::new();
            loop {
                let pixel = cur / 2;
                let other = if cur % 2 == 1 { pixel + gw } else { pixel + 1 };
                for q in [pixel, other] {
                    if at[q].is_none() {
                        at[q] = Some((id, points.len() as u32));
                    }
                }
                points.push((point[&cur], s));
                let next = links[&cur]
                    .iter()
                    .copied()
                    .find(|&n| n != prev && (!seen.contains(&n) || n == start));
                let Some(next) = next else { break };
                let (a, b) = (point[&cur], point[&next]);
                s += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
                if next == start {
                    break;
                }
                seen.insert(next);
                (prev, cur) = (cur, next);
            }
            lengths.push(s.max(1e-3));
            contours.push(points);
        }

        let near_in = nearest(gw, gh, inside);
        let near_out = nearest(gw, gh, |x, y| !inside(x, y));
        Self {
            gw,
            at,
            contours,
            lengths,
            near_in,
            near_out,
        }
    }

    /// The contour and length along it of the edge nearest image pixel (x, y): found
    /// from its nearest pixel on the other side of the edge, then exactly, on the
    /// contour's pieces around there.
    fn place(&self, x: usize, y: usize, is_inside: bool) -> Option<(u32, f32)> {
        let i = (y + 1) * self.gw + x + 1;
        let site = if is_inside {
            self.near_out[i]
        } else {
            self.near_in[i]
        } as usize;
        // A site touching the other side only by a corner: one of its neighbors has it.
        let (sx, sy) = (site % self.gw, site / self.gw);
        let (id, k) = self.at[site].or_else(|| {
            (sy.saturating_sub(1)..=sy + 1)
                .flat_map(|ny| (sx.saturating_sub(1)..=sx + 1).map(move |nx| (nx, ny)))
                .filter_map(|(nx, ny)| self.at.get(ny * self.gw + nx).copied().flatten())
                .next()
        })?;
        let points = &self.contours[id as usize];
        let n = points.len();
        let total = self.lengths[id as usize];
        let p = [(x + 1) as f32, (y + 1) as f32];
        let mut best = (f32::MAX, points[k as usize].1);
        for j in 0..12.min(n) {
            let a = (k as usize + n + j - 6) % n;
            let b = (a + 1) % n;
            let ((pa, sa), (pb, _)) = (points[a], points[b]);
            let d = [pb[0] - pa[0], pb[1] - pa[1]];
            let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-9);
            let t = (((p[0] - pa[0]) * d[0] + (p[1] - pa[1]) * d[1]) / len2).clamp(0.0, 1.0);
            let q = [pa[0] + d[0] * t, pa[1] + d[1] * t];
            let dist = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
            if dist < best.0 {
                best = (dist, (sa + t * len2.sqrt()).rem_euclid(total));
            }
        }
        Some((id, best.1))
    }

    /// How much of a pixel of the line the dashes or dots cover, 0 to 1. `across` is its
    /// distance from the middle of the line, `width` the line's width.
    fn coverage(
        &self,
        x: usize,
        y: usize,
        is_inside: bool,
        across: f32,
        width: f32,
        style: &str,
    ) -> f32 {
        let Some((id, s)) = self.place(x, y, is_inside) else {
            return 1.0;
        };
        let total = self.lengths[id as usize];
        // Visible lengths of the dashes and gaps, in line widths.
        let pattern: &[f32] = match style {
            "dotted" => &[1.0, 1.0],
            "dash_dot" => &[3.0, 1.5, 1.0, 1.5],
            _ => &[3.0, 2.0],
        };
        let period: f32 = pattern.iter().sum::<f32>() * width.max(1.0);
        // A whole number of patterns round the contour, so there is no odd one where it
        // starts.
        let n = (total / period).round().max(1.0);
        let k = total / (n * period);
        let u = (s / k).rem_euclid(period) / width.max(1.0);
        if style == "dotted" {
            // Round dots in the middle of each "on" part.
            let c = ((u - 0.5) / 2.0).round() * 2.0 + 0.5;
            let along = (u - c) * width.max(1.0) * k;
            let r = width / 2.0;
            return (r - (along * along + across * across).sqrt() + 0.5).clamp(0.0, 1.0);
        }
        // Distance (in pixels) inside or outside the nearest dash; dashes start at 0.
        let mut start = 0.0;
        let mut best = f32::MAX;
        for (j, &len) in pattern.iter().enumerate() {
            if j % 2 == 0 {
                let (a, b) = (start, start + len);
                // Measured both ways round the period.
                for off in [0.0, -period / width.max(1.0), period / width.max(1.0)] {
                    let (a, b) = (a + off, b + off);
                    let d = if u < a {
                        a - u
                    } else if u > b {
                        u - b
                    } else {
                        -(u - a).min(b - u)
                    };
                    best = best.min(d);
                }
            }
            start += len;
        }
        (0.5 - best * width.max(1.0) * k).clamp(0.0, 1.0)
    }
}

/// For each pixel of a `w` x `h` grid, the index of the nearest pixel where `feature`
/// is true (exact Euclidean distance). The nearest one found in each row's pass is kept
/// and combined with the column pass.
fn nearest(w: usize, h: usize, feature: impl Fn(usize, usize) -> bool + Sync) -> Vec<u32> {
    let mut f = vec![0.0f32; w * h];
    par_rows(&mut f, w, |y0, chunk| {
        for (j, row) in chunk.chunks_exact_mut(w).enumerate() {
            for (x, v) in row.iter_mut().enumerate() {
                *v = if feature(x, y0 + j) { 0.0 } else { EDT_INF };
            }
        }
    });
    // Rows: the nearest feature's x in each row. Then columns: the row it is in.
    let mut row_x = vec![0u32; w * h];
    edt_rows_arg(&mut f, &mut row_x, w);
    let mut t = transpose(&f, w, h);
    let mut col_y = vec![0u32; w * h];
    edt_rows_arg(&mut t, &mut col_y, h);
    let mut out = vec![0u32; w * h];
    par_rows(&mut out, w, |y0, chunk| {
        for (j, o) in chunk.iter_mut().enumerate() {
            let i = y0 * w + j;
            let (x, y) = (i % w, i / w);
            let ys = col_y[x * h + y] as usize;
            let xs = row_x[ys * w + x] as usize;
            *o = (ys * w + xs) as u32;
        }
    });
    out
}

/// `edt_row_arg` on every row of `f` (rows of `len`), in parallel, with `arg` alongside.
fn edt_rows_arg(f: &mut [f32], arg: &mut [u32], len: usize) {
    let rows = f.len() / len.max(1);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per = rows.div_ceil(threads).max(16) * len;
    std::thread::scope(|s| {
        for (fc, ac) in f.chunks_mut(per).zip(arg.chunks_mut(per)) {
            s.spawn(move || {
                let (mut v, mut z, mut d) = (Vec::new(), Vec::new(), Vec::new());
                for (row, a) in fc.chunks_exact_mut(len).zip(ac.chunks_exact_mut(len)) {
                    edt_row_arg(row, Some(a), &mut v, &mut z, &mut d);
                }
            });
        }
    });
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
    // Dashes or dots: each pixel of the line takes its place along the edge from the
    // nearest point of it.
    let style = p.choice("style");
    if style != "solid" {
        let band_middle = match position {
            "outside" => width / 2.0,
            "inside" => -width / 2.0,
            _ => 0.0,
        };
        let along = Along::new(img);
        par_rows(&mut coverage, w, |y0, chunk| {
            for (j, c) in chunk.iter_mut().enumerate() {
                if *c <= 0.0 {
                    continue;
                }
                let i = y0 * w + j;
                let across = signed(i) - band_middle;
                *c *= along.coverage(i % w, i / w, inside(i), across, width, style);
            }
        });
    }

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

    fn alpha(img: &RgbaImage, x: u32, y: u32) -> u8 {
        img.get_pixel(x, y).0[3]
    }

    #[test]
    fn linear_fade_goes_transparent_toward_its_side() {
        let white = image::Rgba([255, 255, 255, 255]);
        let mut img = RgbaImage::from_pixel(100, 10, white);
        // Default: toward the right, from halfway.
        run("builtin.fade_linear", &mut img, &[], 1.0);
        assert_eq!(alpha(&img, 10, 5), 255);
        assert_eq!(alpha(&img, 49, 5), 255);
        let mid = alpha(&img, 75, 5);
        assert!((100..156).contains(&mid), "half faded: {mid}");
        assert_eq!(alpha(&img, 99, 5), 0);
        // Colors are kept, only opacity changes.
        assert_eq!(&img.get_pixel(75, 5).0[..3], &[255, 255, 255]);

        // Vertical: the top fades, or the bottom.
        let mut img = RgbaImage::from_pixel(10, 100, white);
        run(
            "builtin.fade_linear",
            &mut img,
            &[("toward", "top".into()), ("start", 0.into())],
            1.0,
        );
        assert_eq!(alpha(&img, 5, 0), 0);
        assert!(alpha(&img, 5, 99) > 250);
        assert!(alpha(&img, 5, 30) < alpha(&img, 5, 70));
        let mut img = RgbaImage::from_pixel(10, 100, white);
        run(
            "builtin.fade_linear",
            &mut img,
            &[("toward", "bottom".into())],
            1.0,
        );
        assert_eq!(alpha(&img, 5, 99), 0);
        assert_eq!(alpha(&img, 5, 10), 255);
        // Horizontal, toward the left.
        let mut img = RgbaImage::from_pixel(100, 10, white);
        run(
            "builtin.fade_linear",
            &mut img,
            &[("toward", "left".into())],
            1.0,
        );
        assert_eq!(alpha(&img, 0, 5), 0);
        assert_eq!(alpha(&img, 90, 5), 255);
    }

    #[test]
    fn the_curve_shapes_the_fade() {
        let white = image::Rgba([255, 255, 255, 255]);
        let at_quarter = |curve: [f32; 4]| {
            let mut img = RgbaImage::from_pixel(101, 4, white);
            run(
                "builtin.fade_linear",
                &mut img,
                &[
                    ("start", 0.into()),
                    ("curve", crate::effects::curve_value(curve)),
                ],
                1.0,
            );
            alpha(&img, 25, 2)
        };
        let [linear, ease_in, ease_out, _] = crate::effects::CURVE_PRESETS.map(|p| p.2);
        // A quarter of the way: linear has lost a quarter, ease in less, ease out more.
        let l = at_quarter(linear);
        assert!((185..197).contains(&l), "{l}");
        assert!(at_quarter(ease_in) > l + 20);
        assert!(at_quarter(ease_out) < l - 20);
    }

    #[test]
    fn fades_span_the_visible_pixels_not_the_margins() {
        // A 20 px wide object in the middle of a transparent 100 px image, as when
        // another effect added margins.
        let mut img = RgbaImage::from_fn(100, 10, |x, _| {
            let a = if (40..60).contains(&x) { 255 } else { 0 };
            image::Rgba([255, 0, 0, a])
        });
        run("builtin.fade_linear", &mut img, &[], 1.0);
        assert_eq!(alpha(&img, 45, 5), 255);
        assert!(alpha(&img, 55, 5) < 200);
        assert!(alpha(&img, 59, 5) < 20);
        // Next to a drop shadow, which adds margins first: the object still fades.
        let mut img = RgbaImage::from_pixel(40, 40, image::Rgba([0, 0, 255, 255]));
        let reg = registry();
        let mut fade = EffectInstance::new(reg.get("builtin.fade_linear").unwrap());
        fade.params.insert("start".into(), 0.into());
        let shadow = EffectInstance::new(reg.get("builtin.drop_shadow").unwrap());
        let done = apply(&mut img, &[fade, shadow], 1.0);
        let [l, t, ..] = done.pad;
        assert!(alpha(&img, l + 1, t + 20) > 240, "the left side stays");
        // The right edge is transparent, but the shadow falls there.
        assert_eq!(&img.get_pixel(l + 39, t + 20).0[..3], &[0, 0, 0]);
    }

    #[test]
    fn radial_fade_keeps_the_center_and_inward_reverses_it() {
        let white = image::Rgba([255, 255, 255, 255]);
        let mut img = RgbaImage::from_pixel(100, 50, white);
        run("builtin.fade_radial", &mut img, &[], 1.0);
        assert_eq!(alpha(&img, 50, 25), 255);
        // The ellipse fits the box: the middle of each side is at 100%, the corners
        // beyond.
        assert!(alpha(&img, 1, 25) < 15);
        assert!(alpha(&img, 50, 1) < 15);
        assert_eq!(alpha(&img, 0, 0), 0);
        // Halfway out (75%) is half faded, the same along both axes.
        let (side, top) = (alpha(&img, 12, 25), alpha(&img, 50, 6));
        assert!(side.abs_diff(top) < 12, "{side} {top}");

        let mut img = RgbaImage::from_pixel(100, 50, white);
        run(
            "builtin.fade_radial",
            &mut img,
            &[("inward", true.into())],
            1.0,
        );
        assert_eq!(alpha(&img, 50, 25), 0);
        assert_eq!(alpha(&img, 0, 0), 255);
    }

    /// Both fades on a colored disc over a checkerboard, to look at them:
    /// `FADE_PREVIEW=out.png cargo test fade_preview -- --ignored`.
    #[test]
    #[ignore]
    fn fade_preview() {
        let Ok(path) = std::env::var("FADE_PREVIEW") else {
            return;
        };
        let disc = || {
            RgbaImage::from_fn(200, 200, |x, y| {
                let d = ((x as f32 - 100.0).powi(2) + (y as f32 - 100.0).powi(2)).sqrt();
                let a = if d < 90.0 { 255 } else { 0 };
                image::Rgba([(x + 40) as u8, 90, (y + 40) as u8, a])
            })
        };
        let variants: [(&str, Vec<(&str, serde_json::Value)>); 4] = [
            ("builtin.fade_linear", vec![]),
            (
                "builtin.fade_linear",
                vec![("toward", "bottom".into()), ("start", 20.into())],
            ),
            ("builtin.fade_radial", vec![("start", 30.into())]),
            ("builtin.fade_radial", vec![("inward", true.into())]),
        ];
        let mut out = RgbaImage::new(840, 220);
        for (i, (id, set)) in variants.iter().enumerate() {
            let mut img = disc();
            run(id, &mut img, set, 1.0);
            for (x, y, p) in img.enumerate_pixels() {
                let (ox, oy) = (10 + i as u32 * 210 + x, 10 + y);
                let check = if (ox / 10 + oy / 10) % 2 == 0 {
                    200.0
                } else {
                    150.0
                };
                let a = p.0[3] as f32 / 255.0;
                let c = |v: u8| (v as f32 * a + check * (1.0 - a)) as u8;
                out.put_pixel(ox, oy, image::Rgba([c(p.0[0]), c(p.0[1]), c(p.0[2]), 255]));
            }
        }
        out.save(path).unwrap();
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

    /// A `size` px opaque disc on transparency, outlined `width` px wide in `style`.
    fn styled_disc(size: u32, width: f32, position: &str, style: &str) -> RgbaImage {
        let c = size as f32 / 2.0;
        let mut img = RgbaImage::from_fn(size, size, |x, y| {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            image::Rgba([255, 0, 0, if d < c - 8.0 { 255 } else { 0 }])
        });
        run(
            "builtin.outline",
            &mut img,
            &[
                ("width", width.into()),
                ("position", position.into()),
                ("style", style.into()),
                ("color", "#0000ff".into()),
            ],
            1.0,
        );
        img
    }

    /// How many separate runs of line color a ring of pixels at `radius` from the
    /// image's middle crosses, going round once.
    fn runs_round(img: &RgbaImage, radius: f32) -> usize {
        let c = img.width() as f32 / 2.0;
        let steps = 2000;
        let on: Vec<bool> = (0..steps)
            .map(|k| {
                let a = k as f32 / steps as f32 * std::f32::consts::TAU;
                let p = img.get_pixel((c + radius * a.cos()) as u32, (c + radius * a.sin()) as u32);
                // Blue: the line, not the red disc or the transparent rest.
                p.0[2] > 128 && p.0[0] < 128 && p.0[3] > 128
            })
            .collect();
        (0..steps)
            .filter(|&k| on[k] && !on[(k + steps - 1) % steps])
            .count()
    }

    #[test]
    fn outline_styles_cut_the_line_into_dashes_or_dots() {
        // A disc of radius 92 with a 6 px line outside it, round the circle at 95.
        let solid = styled_disc(200, 6.0, "outside", "solid");
        let pad = (solid.width() - 200) / 2;
        let r = 92.0 + 3.0;
        let ring = |img: &RgbaImage| {
            // Back in the padded image's middle.
            assert_eq!(img.width(), 200 + 2 * pad);
            runs_round(img, r)
        };
        assert_eq!(ring(&solid), 0, "one unbroken ring");
        // Dashes of 3 widths and gaps of 2: about 2 pi 95 / 30 = 20 of them.
        let dashed = ring(&styled_disc(200, 6.0, "outside", "dashed"));
        assert!((18..=22).contains(&dashed), "{dashed}");
        // Dots one width apart: about 2 pi 95 / 12 = 50.
        let dotted = ring(&styled_disc(200, 6.0, "outside", "dotted"));
        assert!((45..=55).contains(&dotted), "{dotted}");
        // Dash, gap, dot, gap: two runs per 7 widths.
        let dash_dot = ring(&styled_disc(200, 6.0, "outside", "dash_dot"));
        assert!((26..=31).contains(&dash_dot), "{dash_dot}");
        // Samples to look at, when asked for.
        if let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") {
            let dir = std::path::PathBuf::from(dir);
            let mut sheet = RgbaImage::from_pixel(4 * 230, 230, image::Rgba([255; 4]));
            for (k, style) in ["solid", "dashed", "dotted", "dash_dot"].iter().enumerate() {
                let img = styled_disc(200, 6.0, "outside", style);
                image::imageops::overlay(&mut sheet, &img, k as i64 * 230, 0);
            }
            sheet.save(dir.join("outline-styles.png")).unwrap();
        }
        // Inside and centered lines are cut too.
        assert!(runs_round(&styled_disc(200, 6.0, "inside", "dashed"), 89.0) >= 15);
        assert!(runs_round(&styled_disc(200, 6.0, "center", "dashed"), 92.0) >= 15);
    }

    #[test]
    fn outline_styles_on_odd_images() {
        // Nothing to outline, or a fully opaque image outlined along its border.
        for style in ["dashed", "dotted", "dash_dot"] {
            let mut empty = RgbaImage::new(20, 20);
            run(
                "builtin.outline",
                &mut empty,
                &[("style", style.into())],
                1.0,
            );
            let mut full = RgbaImage::from_pixel(40, 30, image::Rgba([255, 0, 0, 255]));
            run(
                "builtin.outline",
                &mut full,
                &[
                    ("style", style.into()),
                    ("position", "inside".into()),
                    ("width", 4.0.into()),
                ],
                1.0,
            );
            // Along the top border, the line comes and goes.
            // The line is white over the red image.
            let row: Vec<bool> = (0..40).map(|x| full.get_pixel(x, 1).0[1] > 128).collect();
            assert!(
                row.iter().any(|&on| on) && row.iter().any(|&on| !on),
                "{style}: {row:?}"
            );
        }
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
