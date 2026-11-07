//! Image effects: non-destructive filters stacked on each image layer.
//!
//! An effect is anything implementing [`Effect`]. It describes its parameters with
//! [`ParamSpec`]s, from which the Properties panel builds its controls, and changes the
//! pixels of an image in place. Effects are looked up by id in the [`Registry`]; the
//! built-in ones live in [`builtin`], those of plugin libraries come through [`plugin`]
//! behind the same trait.
//!
//! A layer keeps its effect stack as [`EffectInstance`]s, saved with the project. The
//! original image is never changed: the stack is applied to a copy for the screen (see
//! [`cache`]) and again at full resolution when exporting.

pub mod builtin;
pub mod cache;
pub mod plugin;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tiny_skia::Pixmap;

/// An image effect.
pub trait Effect: Send + Sync {
    /// Unique, stable id saved in projects, e.g. `"builtin.blur"`.
    fn id(&self) -> &str;
    /// Shown in menus and in the effect list.
    fn name(&self) -> &str;
    /// One line shown as a tooltip.
    fn description(&self) -> &str {
        ""
    }
    /// Parameters, in display order.
    fn params(&self) -> &[ParamSpec];
    /// Name of the plugin providing the effect; `None` for a built-in one.
    fn plugin(&self) -> Option<&str> {
        None
    }
    /// Room the effect draws into around the image (left, top, right, bottom), in
    /// source pixels: e.g. a shadow falling outside it. The image passed to `apply` has
    /// transparent margins at least this wide.
    fn padding(&self, _params: &Params) -> [f32; 4] {
        [0.0; 4]
    }
    /// Changes `img` in place. Pixels are straight (not premultiplied) RGBA.
    ///
    /// `scale` is the size of `img` relative to the layer's source image: 1 at export,
    /// less on screen where a smaller copy is used. Multiply distances given in source
    /// pixels (like a blur radius) by it, so every size looks the same.
    fn apply(&self, img: &mut RgbaImage, params: &Params, scale: f32) -> Result<(), String>;
}

/// What a parameter holds, with its range and default.
#[derive(Clone, Debug)]
pub enum ParamKind {
    Float {
        min: f32,
        max: f32,
        default: f32,
        /// Shown after the value, e.g. "px" or "%".
        unit: String,
        decimals: usize,
    },
    Int {
        min: i32,
        max: i32,
        default: i32,
        unit: String,
    },
    Bool {
        default: bool,
    },
    /// Straight RGBA.
    Color {
        default: [u8; 4],
    },
    /// One of several options, saved by id.
    Choice {
        /// (id, label) pairs.
        options: Vec<(String, String)>,
        default: String,
    },
}

/// A parameter of an effect.
#[derive(Clone, Debug)]
pub struct ParamSpec {
    /// Key in the saved values.
    pub id: String,
    /// Shown next to the control.
    pub label: String,
    pub kind: ParamKind,
}

impl ParamSpec {
    fn new(id: &str, label: &str, kind: ParamKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
        }
    }

    pub fn float(id: &str, label: &str, min: f32, max: f32, default: f32) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Float {
                min,
                max,
                default,
                unit: String::new(),
                decimals: 1,
            },
        )
    }

    pub fn int(id: &str, label: &str, min: i32, max: i32, default: i32) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Int {
                min,
                max,
                default,
                unit: String::new(),
            },
        )
    }

    pub fn bool(id: &str, label: &str, default: bool) -> Self {
        Self::new(id, label, ParamKind::Bool { default })
    }

    pub fn color(id: &str, label: &str, default: [u8; 4]) -> Self {
        Self::new(id, label, ParamKind::Color { default })
    }

    pub fn choice(id: &str, label: &str, options: &[(&str, &str)], default: &str) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Choice {
                options: options
                    .iter()
                    .map(|(i, l)| ((*i).into(), (*l).into()))
                    .collect(),
                default: default.into(),
            },
        )
    }

    /// Sets the unit shown after a number.
    pub fn unit(mut self, u: &str) -> Self {
        match &mut self.kind {
            ParamKind::Float { unit, .. } | ParamKind::Int { unit, .. } => *unit = u.into(),
            _ => {}
        }
        self
    }

    /// Sets the decimals shown for a float.
    pub fn decimals(mut self, n: usize) -> Self {
        if let ParamKind::Float { decimals, .. } = &mut self.kind {
            *decimals = n;
        }
        self
    }

    /// The default value, as saved.
    pub fn default_value(&self) -> Value {
        match &self.kind {
            ParamKind::Float { default, .. } => Value::from(*default),
            ParamKind::Int { default, .. } => Value::from(*default),
            ParamKind::Bool { default } => Value::from(*default),
            ParamKind::Color { default } => Value::from(color_to_hex(*default)),
            ParamKind::Choice { default, .. } => Value::from(default.clone()),
        }
    }
}

/// Saved parameter values by id. Missing or invalid values fall back to the default, so
/// projects survive effects gaining, losing or changing parameters.
pub type ParamValues = BTreeMap<String, Value>;

/// `#rrggbbaa`.
pub fn color_to_hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
}

/// Parses `#rrggbb` or `#rrggbbaa`.
pub fn color_from_hex(s: &str) -> Option<[u8; 4]> {
    let s = s.strip_prefix('#')?;
    if !s.is_ascii() || (s.len() != 6 && s.len() != 8) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    let a = if s.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, a])
}

/// Typed, range-checked access to an effect's parameter values.
pub struct Params<'a> {
    specs: &'a [ParamSpec],
    values: &'a ParamValues,
}

impl<'a> Params<'a> {
    pub fn new(specs: &'a [ParamSpec], values: &'a ParamValues) -> Self {
        Self { specs, values }
    }

    fn spec(&self, id: &str) -> Option<&ParamKind> {
        self.specs.iter().find(|s| s.id == id).map(|s| &s.kind)
    }

    /// A float parameter, clamped to its range; 0 if there is no such parameter.
    pub fn f32(&self, id: &str) -> f32 {
        let v = self.values.get(id).and_then(Value::as_f64);
        match self.spec(id) {
            Some(ParamKind::Float {
                min, max, default, ..
            }) => v.map_or(*default, |v| v as f32).clamp(*min, *max),
            Some(ParamKind::Int {
                min, max, default, ..
            }) => v
                .map_or(*default as f32, |v| v as f32)
                .clamp(*min as f32, *max as f32),
            _ => 0.0,
        }
    }

    /// An int parameter, clamped to its range; 0 if there is no such parameter.
    pub fn i32(&self, id: &str) -> i32 {
        match self.spec(id) {
            Some(ParamKind::Int {
                min, max, default, ..
            }) => self
                .values
                .get(id)
                .and_then(Value::as_f64)
                .map_or(*default, |v| v.round() as i32)
                .clamp(*min, *max),
            _ => self.f32(id).round() as i32,
        }
    }

    pub fn bool(&self, id: &str) -> bool {
        let default = match self.spec(id) {
            Some(ParamKind::Bool { default }) => *default,
            _ => false,
        };
        self.values
            .get(id)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    /// Straight RGBA.
    pub fn color(&self, id: &str) -> [u8; 4] {
        let default = match self.spec(id) {
            Some(ParamKind::Color { default }) => *default,
            _ => [0, 0, 0, 255],
        };
        self.values
            .get(id)
            .and_then(Value::as_str)
            .and_then(color_from_hex)
            .unwrap_or(default)
    }

    /// Id of the chosen option, or the default if the saved one doesn't exist.
    pub fn choice(&self, id: &str) -> &str {
        let Some(ParamKind::Choice { options, default }) = self.spec(id) else {
            return "";
        };
        match self.values.get(id).and_then(Value::as_str) {
            Some(v) if options.iter().any(|(o, _)| o == v) => v,
            _ => default,
        }
    }

    /// Every parameter's value as read by the getters above: in range, with colors as
    /// `#rrggbbaa`. This is what plugins receive.
    pub fn resolved(&self) -> Value {
        let mut map = serde_json::Map::new();
        for spec in self.specs {
            let id = spec.id.as_str();
            let v = match &spec.kind {
                ParamKind::Float { .. } => Value::from(self.f32(id)),
                ParamKind::Int { .. } => Value::from(self.i32(id)),
                ParamKind::Bool { .. } => Value::from(self.bool(id)),
                ParamKind::Color { .. } => Value::from(color_to_hex(self.color(id))),
                ParamKind::Choice { .. } => Value::from(self.choice(id)),
            };
            map.insert(spec.id.clone(), v);
        }
        Value::Object(map)
    }
}

fn yes() -> bool {
    true
}

/// An effect in a layer's stack, saved with the project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectInstance {
    /// `Effect::id`. Kept even when no such effect is available (e.g. a plugin that is
    /// not installed), so the project still saves it unchanged.
    pub effect: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub params: ParamValues,
}

impl EffectInstance {
    /// A new instance with every parameter at its default.
    pub fn new(effect: &dyn Effect) -> Self {
        Self {
            effect: effect.id().into(),
            enabled: true,
            params: effect
                .params()
                .iter()
                .map(|p| (p.id.clone(), p.default_value()))
                .collect(),
        }
    }
}

/// All available effects.
pub struct Registry {
    effects: Vec<Box<dyn Effect>>,
    /// Plugins that loaded.
    pub plugins: Vec<plugin::PluginInfo>,
    /// Why some plugins, or some of their effects, are not available.
    pub errors: Vec<String>,
}

impl Registry {
    /// The built-in effects.
    fn builtin() -> Self {
        Self {
            effects: builtin::all(),
            plugins: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// The built-in effects, then those of the plugins in `dir`. An effect whose id is
    /// already taken is left out.
    fn with_plugins(dir: Option<&Path>) -> Self {
        let mut reg = Self::builtin();
        let Some(dir) = dir else {
            return reg;
        };
        let loaded = plugin::load_dir(dir);
        reg.errors = loaded.errors;
        for e in loaded.effects {
            if reg.get(e.id()).is_some() {
                let msg = format!(
                    "{}: effect \"{}\" left out, its id is already used",
                    e.plugin().unwrap_or_default(),
                    e.id()
                );
                log::warn!("{msg}");
                reg.errors.push(msg);
            } else {
                reg.effects.push(e);
            }
        }
        reg.plugins = loaded.plugins;
        reg
    }

    pub fn get(&self, id: &str) -> Option<&dyn Effect> {
        self.effects
            .iter()
            .find(|e| e.id() == id)
            .map(|e| e.as_ref())
    }

    /// In menu order.
    pub fn all(&self) -> impl Iterator<Item = &dyn Effect> {
        self.effects.iter().map(|e| e.as_ref())
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Builds the registry with the plugins of `plugins_dir` (see
/// [`plugin::plugins_dir`]). Call it once at startup, before [`registry`]; later calls
/// return the registry already built.
pub fn init(plugins_dir: Option<&Path>) -> &'static Registry {
    REGISTRY.get_or_init(|| Registry::with_plugins(plugins_dir))
}

/// The effects available in this run of the app: only the built-in ones if [`init`]
/// wasn't called first.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::builtin)
}

/// The stack changes the image: some effect is enabled.
pub fn is_active(stack: &[EffectInstance]) -> bool {
    stack.iter().any(|e| e.enabled)
}

/// Identifies what the stack does to an image: equal keys give equal pixels.
pub fn key(stack: &[EffectInstance]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for e in stack.iter().filter(|e| e.enabled) {
        e.effect.hash(&mut h);
        serde_json::to_string(&e.params)
            .unwrap_or_default()
            .hash(&mut h);
    }
    h.finish()
}

/// Widest margin added on any side, in pixels of the processed image.
const MAX_PADDING: f32 = 4096.0;

/// Transparent margins (left, top, right, bottom) the stack needs around an image of
/// `scale`, in its pixels: what all enabled effects ask for, added up.
pub fn padding(stack: &[EffectInstance], scale: f32) -> [u32; 4] {
    let reg = registry();
    let mut pad = [0.0f32; 4];
    for inst in stack.iter().filter(|e| e.enabled) {
        if let Some(effect) = reg.get(&inst.effect) {
            let p = effect.padding(&Params::new(effect.params(), &inst.params));
            for (a, b) in pad.iter_mut().zip(p) {
                *a += b.max(0.0);
            }
        }
    }
    pad.map(|v| (v * scale).ceil().clamp(0.0, MAX_PADDING) as u32)
}

/// What applying a stack did.
#[derive(Debug, Default)]
pub struct Applied {
    /// Margins added around the image (left, top, right, bottom), in its pixels.
    pub pad: [u32; 4],
    /// Errors of the effects that failed; the others still applied.
    pub errors: Vec<String>,
}

/// Applies the enabled effects in order, after adding the margins they need. Unknown
/// effects are skipped.
pub fn apply(img: &mut RgbaImage, stack: &[EffectInstance], scale: f32) -> Applied {
    let pad = padding(stack, scale);
    if pad != [0; 4] {
        let [l, t, r, b] = pad;
        let (w, bw) = (img.width() as usize * 4, (img.width() + l + r) as usize * 4);
        let mut big = RgbaImage::new(img.width() + l + r, img.height() + t + b);
        let src = img.as_raw();
        par_rows(&mut big, bw, |y0, chunk| {
            for (j, row) in chunk.chunks_exact_mut(bw).enumerate() {
                let y = (y0 + j) as i64 - t as i64;
                if (0..img.height() as i64).contains(&y) {
                    let from = &src[y as usize * w..][..w];
                    row[l as usize * 4..][..w].copy_from_slice(from);
                }
            }
        });
        *img = big;
    }
    let reg = registry();
    let mut errors = Vec::new();
    for inst in stack.iter().filter(|e| e.enabled) {
        let Some(effect) = reg.get(&inst.effect) else {
            continue;
        };
        let params = Params::new(effect.params(), &inst.params);
        if let Err(e) = effect.apply(img, &params, scale) {
            errors.push(format!("{}: {e}", effect.name()));
        }
    }
    Applied { pad, errors }
}

/// Applies the stack to premultiplied pixels, for export, and returns the margins added
/// (see `apply`). Fully opaque pixels go through unchanged in precision; faint ones
/// lose some.
pub fn apply_pixmap(
    src: &Pixmap,
    stack: &[EffectInstance],
    scale: f32,
) -> Option<(Pixmap, [u32; 4])> {
    let mut img = cache::demultiply(src);
    let done = apply(&mut img, stack, scale);
    for e in done.errors {
        log::warn!("effect failed: {e}");
    }
    Some((crate::render::premultiplied_pixmap(&img)?, done.pad))
}

/// Runs `f` on chunks of whole rows of `data` in parallel. `row_len` is the length of a
/// row in elements; `f` gets the index of the chunk's first row.
pub fn par_rows<T: Send>(data: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = data.len() / row_len.max(1);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per = rows.div_ceil(threads).max(16);
    if rows <= per {
        f(0, data);
        return;
    }
    std::thread::scope(|s| {
        for (i, chunk) in data.chunks_mut(per * row_len).enumerate() {
            let f = &f;
            s.spawn(move || f(i * per, chunk));
        }
    });
}

/// Runs `f` on every pixel in parallel.
pub fn par_pixels(img: &mut RgbaImage, f: impl Fn(&mut [u8; 4]) + Sync) {
    let row = img.width() as usize * 4;
    par_rows(img, row, |_, chunk| {
        for px in chunk.as_chunks_mut::<4>().0 {
            f(px);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_round_trip() {
        let c = [1, 128, 255, 64];
        assert_eq!(color_from_hex(&color_to_hex(c)), Some(c));
        assert_eq!(color_from_hex("#ff0000"), Some([255, 0, 0, 255]));
        assert_eq!(color_from_hex("ff0000"), None);
        assert_eq!(color_from_hex("#ff00"), None);
    }

    #[test]
    fn params_fall_back_to_defaults_and_clamp() {
        let specs = [
            ParamSpec::float("r", "R", 0.0, 10.0, 2.0),
            ParamSpec::choice("m", "M", &[("a", "A"), ("b", "B")], "a"),
        ];
        let mut values = ParamValues::new();
        let p = Params::new(&specs, &values);
        assert_eq!((p.f32("r"), p.choice("m")), (2.0, "a"));
        values.insert("r".into(), Value::from(50.0));
        values.insert("m".into(), Value::from("zzz"));
        let p = Params::new(&specs, &values);
        assert_eq!((p.f32("r"), p.choice("m")), (10.0, "a"));
    }

    #[test]
    fn unknown_and_disabled_effects_are_skipped() {
        let mut img = RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 50, 255]));
        let mut invert = EffectInstance::new(registry().get("builtin.invert").unwrap());
        invert.enabled = false;
        let stack = vec![
            EffectInstance {
                effect: "plugin.not-installed".into(),
                enabled: true,
                params: ParamValues::new(),
            },
            invert,
        ];
        assert!(apply(&mut img, &stack, 1.0).errors.is_empty());
        assert_eq!(img.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert!(!is_active(&stack[1..]));
    }

    #[test]
    fn key_ignores_disabled_effects() {
        let invert = EffectInstance::new(registry().get("builtin.invert").unwrap());
        let mut off = invert.clone();
        off.enabled = false;
        assert_eq!(key(&[]), key(std::slice::from_ref(&off)));
        assert_ne!(key(&[]), key(&[invert]));
    }
}
