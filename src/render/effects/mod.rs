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
//!
//! A [`Generator`] makes pixels instead of changing them: a shape's fill (see
//! `shape`), such as noise or stripes. Generators share effects' parameters, their
//! registry and their plugins; the built-in ones live in [`generators`].

pub mod builtin;
pub mod cache;
pub mod generators;
pub mod plugin;
pub mod python;
pub mod repository;

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

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
    /// Stops what runs it, its plugin uninstalled: it is not called again.
    fn unload(&self) {}
}

/// Draws a shape's fill.
pub trait Generator: Send + Sync {
    /// Unique, stable id saved in projects, e.g. `"builtin.noise"`; no effect has it.
    fn id(&self) -> &str;
    /// Shown in a fill's Generated menu.
    fn name(&self) -> &str;
    /// One line shown as a tooltip.
    fn description(&self) -> &str {
        ""
    }
    /// Parameters, in display order.
    fn params(&self) -> &[ParamSpec];
    /// Name of the plugin providing it; `None` for a built-in one.
    fn plugin(&self) -> Option<&str> {
        None
    }
    /// Draws into `img`, transparent and straight (not premultiplied) RGBA, which covers
    /// the shape's box at `px_per_mm` pixels per mm: lengths in mm times `px_per_mm`
    /// are in pixels. Equal parameters must give the same picture at any resolution.
    fn generate(&self, img: &mut RgbaImage, params: &Params, px_per_mm: f32) -> Result<(), String>;
    /// Stops what runs it, its plugin uninstalled: it is not called again.
    fn unload(&self) {}
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
    /// An easing curve from (0, 0) to (1, 1), as CSS's `cubic-bezier(x1, y1, x2, y2)`:
    /// its two control points, saved as `[x1, y1, x2, y2]`. Built-in effects only.
    Curve {
        default: [f32; 4],
    },
}

/// What a parameter is for, which picks its control (see `ui::effects`), as plugins
/// give it with `artwork_plugin_sdk::Role`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// An int: a random variation, given a new random value by a regenerate button.
    Seed,
    /// A float in degrees, counterclockwise from the right: a value and a dial.
    Angle,
    /// A float length in layer pixels: a logarithmic slider, in mm when asked.
    Size,
}

/// A parameter of an effect.
#[derive(Clone, Debug)]
pub struct ParamSpec {
    /// Key in the saved values.
    pub id: String,
    /// Shown next to the control.
    pub label: String,
    pub kind: ParamKind,
    /// What it is for, as given; see [`ParamSpec::role`].
    pub role: Option<Role>,
}

impl ParamSpec {
    fn new(id: &str, label: &str, kind: ParamKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            role: None,
        }
    }

    /// Sets what it is for.
    pub fn with_role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// The range of a seed (see [`Role::Seed`]); `None` for another parameter.
    pub fn seed_range(&self) -> Option<(i32, i32)> {
        match self.kind {
            ParamKind::Int { min, max, .. } if self.role() == Some(Role::Seed) => Some((min, max)),
            _ => None,
        }
    }

    /// What it is for: as given, else guessed as plugins made before roles expect (an
    /// int with the id "seed", a float in "°" or in "px").
    pub fn role(&self) -> Option<Role> {
        if self.role.is_some() {
            return self.role;
        }
        match &self.kind {
            ParamKind::Int { .. } if self.id == "seed" => Some(Role::Seed),
            ParamKind::Float { unit, .. } if unit.trim() == "°" => Some(Role::Angle),
            ParamKind::Float { unit, .. } if unit.trim() == "px" => Some(Role::Size),
            _ => None,
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

    pub fn curve(id: &str, label: &str, default: [f32; 4]) -> Self {
        Self::new(id, label, ParamKind::Curve { default })
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
            ParamKind::Curve { default } => curve_value(*default),
        }
    }
}

/// The largest seed of the built-in generators, the same as the SDK's seed helpers': a
/// seed takes any value from 0 to it.
pub const SEED_MAX: i32 = artwork_plugin_sdk::SEED_MAX;

/// A random seed in `min..=max`, different from `old` when there is room for another.
pub fn random_seed(min: i32, max: i32, old: Option<i32>) -> i32 {
    use std::hash::{BuildHasher, Hasher};
    let (min, max) = (min.min(max), max.max(min));
    // Each RandomState has new keys, so two calls in the same instant differ too.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    h.write_u64(nanos);
    let span = (max as i64 - min as i64) as u64 + 1;
    let mut v = min as i64 + (h.finish() % span) as i64;
    if old == Some(v as i32) && span > 1 {
        v = if v >= max as i64 { min as i64 } else { v + 1 };
    }
    v as i32
}

/// A curve's control points as saved.
pub fn curve_value(c: [f32; 4]) -> Value {
    Value::from(c.to_vec())
}

/// Easing curves to start from, as CSS names them: (id, label key, control points).
pub const CURVE_PRESETS: [(&str, &str, [f32; 4]); 4] = [
    (
        "linear",
        crate::i18n::tr_noop!("CURVE_LINEAR"),
        [0.0, 0.0, 1.0, 1.0],
    ),
    (
        "ease_in",
        crate::i18n::tr_noop!("CURVE_EASE_IN"),
        [0.42, 0.0, 1.0, 1.0],
    ),
    (
        "ease_out",
        crate::i18n::tr_noop!("CURVE_EASE_OUT"),
        [0.0, 0.0, 0.58, 1.0],
    ),
    (
        "ease_in_out",
        crate::i18n::tr_noop!("CURVE_EASE_IN_OUT"),
        [0.42, 0.0, 0.58, 1.0],
    ),
];

/// The curve's height at `x` (0 to 1), as a CSS cubic-bezier timing function: the point
/// of the curve whose horizontal position is `x`.
pub fn ease(c: [f32; 4], x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    // One coordinate of the curve at parameter s, from (0, 0) through a, b to (1, 1).
    let at = |a: f32, b: f32, s: f32| {
        let r = 1.0 - s;
        3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
    };
    // x(s) grows with s since x1 and x2 are in [0, 1]: bisection finds it.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if at(c[0], c[2], mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(c[1], c[3], (lo + hi) / 2.0)
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

    /// A curve's control points, x within 0 to 1 (so the curve goes forward), y too (an
    /// opacity cannot go past either end).
    pub fn curve(&self, id: &str) -> [f32; 4] {
        let default = match self.spec(id) {
            Some(ParamKind::Curve { default }) => *default,
            _ => [0.0, 0.0, 1.0, 1.0],
        };
        let saved = self.values.get(id).and_then(Value::as_array).and_then(|a| {
            let v: Vec<f32> = a
                .iter()
                .filter_map(|n| n.as_f64().map(|n| n as f32))
                .collect();
            <[f32; 4]>::try_from(v).ok()
        });
        saved
            .filter(|c| c.iter().all(|n| n.is_finite()))
            .unwrap_or(default)
            .map(|n| n.clamp(0.0, 1.0))
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
                ParamKind::Curve { .. } => curve_value(self.curve(id)),
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
    /// Color parameters linked to saved colors: parameter id to the color's name (see
    /// `colors`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub links: std::collections::BTreeMap<String, String>,
    /// The repository plugin it came from and its version, for an effect of a plugin
    /// installed from a repository: what to offer to install when it is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<PluginRef>,
    /// The name of a plugin's effect, to show it by when it is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A plugin of a repository: its id there and a version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRef {
    pub id: String,
    pub version: String,
}

impl EffectInstance {
    /// A new instance with every parameter at its default.
    pub fn new(effect: &dyn Effect) -> Self {
        Self {
            plugin: registry().plugin_ref(effect.id()),
            name: effect.plugin().map(|_| effect.name().to_owned()),
            ..Self::with_defaults(effect.id(), effect.params())
        }
    }

    /// A generator's settings, every parameter at its default; `effect` is then the
    /// generator's id.
    pub fn generator(generator: &dyn Generator) -> Self {
        Self {
            plugin: registry().plugin_ref(generator.id()),
            name: generator.plugin().map(|_| generator.name().to_owned()),
            ..Self::with_defaults(generator.id(), generator.params())
        }
    }

    /// The same, with a new random value for each seed (see [`Role::Seed`]), so that a
    /// new effect or fill doesn't look like every other one.
    pub fn with_random_seeds(mut self) -> Self {
        let specs = registry().params_of(&self.effect).unwrap_or_default();
        for spec in specs {
            if let Some((min, max)) = spec.seed_range() {
                let v = random_seed(min, max, None);
                self.params.insert(spec.id.clone(), Value::from(v));
            }
        }
        self
    }

    fn with_defaults(id: &str, specs: &[ParamSpec]) -> Self {
        Self {
            effect: id.into(),
            enabled: true,
            params: specs
                .iter()
                .map(|p| (p.id.clone(), p.default_value()))
                .collect(),
            links: Default::default(),
            plugin: None,
            name: None,
        }
    }
}

/// All available effects and generators.
pub struct Registry {
    first: Part,
    /// Python plugins new or changed since they were allowed to run, held back at
    /// startup until the user allows them (see [`Registry::allow`]).
    pending: Vec<plugin::Pending>,
    /// For each pending plugin, once allowed: what it added, or why it failed.
    late: Vec<OnceLock<Part>>,
    /// Plugins installed from a repository since startup, loaded at once, in the order
    /// installed; past [`INSTALLED_SLOTS`], they load at the next start.
    installed: Vec<OnceLock<Part>>,
    /// What the plugins uninstalled since startup added, left out from then on.
    removed: RwLock<Removed>,
}

/// The plugins uninstalled since startup.
#[derive(Default)]
struct Removed {
    /// Their effects, generators and [`plugin::PluginInfo`]s, by address (see [`addr`]).
    items: std::collections::HashSet<usize>,
    /// Their files.
    files: Vec<PathBuf>,
    /// How many were unloaded.
    count: usize,
}

/// Where `r` is: what tells an effect, a generator or a plugin from another one with
/// the same id (the same plugin installed again).
fn addr<T: ?Sized>(r: &T) -> usize {
    (r as *const T).cast::<()>() as usize
}

/// How many plugins installed from a repository can load without a restart.
const INSTALLED_SLOTS: usize = 16;

/// Effects, generators and plugins loaded together, checked against those before them.
#[derive(Default)]
struct Part {
    effects: Vec<Box<dyn Effect>>,
    generators: Vec<Box<dyn Generator>>,
    /// Plugins that loaded.
    plugins: Vec<plugin::PluginInfo>,
    /// Why some plugins, or some of their effects, are not available.
    errors: Vec<String>,
    /// The plugin files, with what became of them.
    files: Vec<plugin::PluginFile>,
}

impl Part {
    /// What was `loaded`, but an effect or a generator whose id is already used, by one
    /// for which `taken` is true or by one before it in `loaded`.
    fn checked(loaded: plugin::Loaded, taken: impl Fn(&str) -> bool) -> Self {
        let mut part = Part {
            plugins: loaded.plugins,
            errors: loaded.errors,
            files: loaded.files,
            ..Default::default()
        };
        let left_out = |what: &str, plugin: Option<&str>, id: &str| {
            let msg = format!(
                "{}: {what} \"{id}\" left out, its id is already used",
                plugin.unwrap_or_default()
            );
            log::warn!("{msg}");
            msg
        };
        for e in loaded.effects {
            if taken(e.id()) || part.params_of(e.id()).is_some() {
                let msg = left_out("effect", e.plugin(), e.id());
                part.errors.push(msg);
            } else {
                part.effects.push(e);
            }
        }
        for g in loaded.generators {
            if taken(g.id()) || part.params_of(g.id()).is_some() {
                let msg = left_out("generator", g.plugin(), g.id());
                part.errors.push(msg);
            } else {
                part.generators.push(g);
            }
        }
        part
    }

    fn params_of(&self, id: &str) -> Option<&[ParamSpec]> {
        let effect = self.effects.iter().find(|e| e.id() == id);
        effect.map(|e| e.params()).or_else(|| {
            let g = self.generators.iter().find(|g| g.id() == id);
            g.map(|g| g.params())
        })
    }
}

impl Registry {
    /// The built-in effects and generators.
    fn builtin() -> Self {
        Self {
            first: Part {
                effects: builtin::all(),
                generators: generators::all(),
                ..Default::default()
            },
            pending: Vec::new(),
            late: Vec::new(),
            installed: (0..INSTALLED_SLOTS).map(|_| OnceLock::new()).collect(),
            removed: Default::default(),
        }
    }

    /// The built-in effects and generators, then those of the plugins in `dirs`, in
    /// order, but those `settings` doesn't allow (see [`Registry::pending`]). One whose
    /// id is already taken is left out.
    fn with_plugins(dirs: &[PathBuf], settings: &plugin::Settings) -> Self {
        let mut reg = Self::builtin();
        let mut loaded = plugin::Loaded::default();
        for dir in dirs {
            let more = plugin::load_dir(dir, settings);
            for e in more.errors {
                // "No Python" comes once, whatever the number of folders.
                if e != python::PYTHON_MISSING || !loaded.errors.contains(&e) {
                    loaded.errors.push(e);
                }
            }
            loaded.effects.extend(more.effects);
            loaded.generators.extend(more.generators);
            loaded.plugins.extend(more.plugins);
            loaded.files.extend(more.files);
            reg.pending.extend(more.pending);
        }
        let mut part = Part::checked(loaded, |id| reg.params_of(id).is_some());
        reg.first.effects.append(&mut part.effects);
        reg.first.generators.append(&mut part.generators);
        reg.first.plugins = part.plugins;
        reg.first.errors = part.errors;
        reg.first.files = part.files;
        reg.late = reg.pending.iter().map(|_| OnceLock::new()).collect();
        reg
    }

    /// The parts: the first, then those of the plugins allowed since.
    fn parts(&self) -> impl Iterator<Item = &Part> {
        let late = self.late.iter().chain(&self.installed);
        std::iter::once(&self.first).chain(late.filter_map(OnceLock::get))
    }

    /// The Python plugins waiting to be allowed.
    pub fn pending(&self) -> Vec<&plugin::Pending> {
        let waiting = self.pending.iter().zip(&self.late);
        let removed = self.removed();
        waiting
            .filter(|(p, l)| l.get().is_none() && !removed.files.contains(&p.path))
            .map(|(p, _)| p)
            .collect()
    }

    /// Allows the pending plugin `path` to run: remembered in `settings`, saved, and
    /// loaded at once. Does nothing for a plugin not pending.
    pub fn allow(&self, path: &std::path::Path, settings: &mut plugin::Settings) {
        let Some((p, late)) = self
            .pending
            .iter()
            .zip(&self.late)
            .find(|(p, l)| p.path == path && l.get().is_none())
        else {
            return;
        };
        settings.allow(p);
        settings.save();
        let _ = late.set(self.load(&p.path));
    }

    /// Loads the plugin installed at `path` from a repository, already allowed in the
    /// user's settings: false if it can't be until the next start (one loaded there
    /// already, or no room left).
    pub fn add_installed(&self, path: &std::path::Path) -> bool {
        if self.files().iter().any(|f| f.path == path) {
            return false;
        }
        let Some(slot) = self.installed.iter().find(|s| s.get().is_none()) else {
            return false;
        };
        // Installed again after it was uninstalled.
        let mut removed = self.removed.write().unwrap_or_else(|e| e.into_inner());
        removed.files.retain(|f| f != path);
        drop(removed);
        slot.set(self.load(path)).is_ok()
    }

    /// Unloads the plugin file `path`, uninstalled: its effects, generators and file are
    /// left out from then on, and a Python plugin's process stops. A library's code stays
    /// in memory until the app quits (an effect may be running it), never called again.
    pub fn remove(&self, path: &std::path::Path) {
        if !self.files().iter().any(|f| f.path == path) {
            return;
        }
        let plugins: Vec<&plugin::PluginInfo> = self.plugins().filter(|p| p.path == path).collect();
        let of = |id: &str, name: Option<&str>| {
            (plugins.iter()).any(|p| {
                name == Some(p.name.as_str())
                    && p.effects.iter().chain(&p.generators).any(|i| i == id)
            })
        };
        let effects: Vec<&dyn Effect> = self.all().filter(|e| of(e.id(), e.plugin())).collect();
        let generators: Vec<&dyn Generator> = self
            .generators()
            .filter(|g| of(g.id(), g.plugin()))
            .collect();
        let mut removed = self.removed.write().unwrap_or_else(|e| e.into_inner());
        removed.items.extend(plugins.iter().map(|p| addr(*p)));
        removed.items.extend(effects.iter().map(|e| addr(*e)));
        removed.items.extend(generators.iter().map(|g| addr(*g)));
        removed.files.push(path.to_owned());
        removed.count += 1;
        drop(removed);
        effects.iter().for_each(|e| e.unload());
        generators.iter().for_each(|g| g.unload());
    }

    fn removed(&self) -> std::sync::RwLockReadGuard<'_, Removed> {
        self.removed.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether `item` is of a plugin uninstalled (see [`Registry::remove`]).
    fn is_removed<T: ?Sized>(&self, item: &T) -> bool {
        self.removed().items.contains(&addr(item))
    }

    /// The plugin file `path` loaded, checked against what is loaded already.
    fn load(&self, path: &std::path::Path) -> Part {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let mut loaded = plugin::Loaded::default();
        let result = plugin::load_file(path, python::python());
        let status = match result {
            Ok((info, effects, generators)) => {
                let status = plugin::Status::Loaded {
                    name: info.name.clone(),
                    version: info.version.clone(),
                };
                loaded.plugins.push(info);
                loaded.effects.extend(effects);
                loaded.generators.extend(generators);
                status
            }
            Err(e) => {
                loaded.errors.push(format!("{file}: {e}"));
                plugin::Status::Failed(e)
            }
        };
        loaded.files.push(plugin::PluginFile {
            path: path.to_owned(),
            python: plugin::is_python(path),
            status,
        });
        Part::checked(loaded, |id| self.params_of(id).is_some())
    }

    /// How many plugins were allowed, installed or uninstalled since startup: cached
    /// pictures made with effects or generators that were missing, or that are gone, are
    /// made again when it changes.
    pub fn generation(&self) -> usize {
        let late = self.late.iter().chain(&self.installed);
        late.filter(|l| l.get().is_some()).count() + self.removed().count
    }

    /// Every plugin file found, in loading order, with what became of it (since it was
    /// allowed, for a pending one), then those installed since startup.
    pub fn files(&self) -> Vec<plugin::PluginFile> {
        let mut files = self.first.files.clone();
        for late in self
            .late
            .iter()
            .chain(&self.installed)
            .filter_map(OnceLock::get)
            .flat_map(|p| &p.files)
        {
            match files.iter_mut().find(|f| f.path == late.path) {
                Some(f) => *f = late.clone(),
                None => files.push(late.clone()),
            }
        }
        let removed = self.removed();
        files.retain(|f| !removed.files.contains(&f.path));
        files
    }

    /// Plugins that loaded.
    pub fn plugins(&self) -> impl Iterator<Item = &plugin::PluginInfo> {
        let plugins = self.parts().flat_map(|p| &p.plugins);
        plugins.filter(|p| !self.is_removed(*p))
    }

    /// Why some plugins, or some of their effects, are not available.
    pub fn errors(&self) -> Vec<&str> {
        self.parts()
            .flat_map(|p| p.errors.iter().map(String::as_str))
            .collect()
    }

    /// The repository plugin and version the effect or generator `id` comes from, if
    /// known.
    pub fn plugin_ref(&self, id: &str) -> Option<PluginRef> {
        self.installed(id).map(|i| i.entry.clone())
    }

    /// What the plugin of the effect or generator `id` recorded when installed from a
    /// repository.
    fn installed(&self, id: &str) -> Option<&plugin::Installed> {
        let mut plugins = self.plugins();
        let p = plugins.find(|p| p.effects.iter().chain(&p.generators).any(|e| e == id))?;
        p.entry.as_ref()
    }

    /// Brings the plugin version `inst` records up to the one installed, which reads
    /// its settings from now on; true if they were saved by a version older than its
    /// `compatible_from`, that it may read differently.
    pub fn update_version(&self, inst: &mut EffectInstance) -> bool {
        let (Some(saved), Some(i)) = (&mut inst.plugin, self.installed(&inst.effect)) else {
            return false;
        };
        if saved.id != i.entry.id {
            return false;
        }
        let parts = repository::version_parts;
        let older =
            (i.compatible_from.as_deref()).is_some_and(|c| parts(c) > parts(&saved.version));
        saved.version = i.entry.version.clone();
        older
    }

    pub fn get(&self, id: &str) -> Option<&dyn Effect> {
        self.all().find(|e| e.id() == id)
    }

    /// In menu order.
    pub fn all(&self) -> impl Iterator<Item = &dyn Effect> {
        self.parts()
            .flat_map(|p| p.effects.iter().map(|e| e.as_ref()))
            .filter(|e| !self.is_removed(*e))
    }

    pub fn generator(&self, id: &str) -> Option<&dyn Generator> {
        self.generators().find(|g| g.id() == id)
    }

    /// In menu order.
    pub fn generators(&self) -> impl Iterator<Item = &dyn Generator> {
        self.parts()
            .flat_map(|p| p.generators.iter().map(|g| g.as_ref()))
            .filter(|g| !self.is_removed(*g))
    }

    /// The parameters of the effect or generator `id`.
    pub fn params_of(&self, id: &str) -> Option<&[ParamSpec]> {
        self.get(id)
            .map(|e| e.params())
            .or_else(|| self.generator(id).map(|g| g.params()))
    }
}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Builds the registry with the plugins of `plugins_dirs` (see
/// [`plugin::plugins_dirs`]), the Python ones only if `allowed` lists them. Call it once
/// at startup, before [`registry`]; later calls return the registry already built.
pub fn init(plugins_dirs: &[PathBuf], allowed: &plugin::Settings) -> &'static Registry {
    REGISTRY.get_or_init(|| Registry::with_plugins(plugins_dirs, allowed))
}

/// The effects available in this run of the app: only the built-in ones if [`init`]
/// wasn't called first.
pub fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::builtin)
}

/// What the generator of `inst` makes over `w` x `h` pixels at `px_per_mm`, premultiplied;
/// `None` when it is off, unknown or fails (the error is logged).
pub fn generate(inst: &EffectInstance, w: u32, h: u32, px_per_mm: f32) -> Option<Pixmap> {
    if !inst.enabled {
        return None;
    }
    let generator = registry().generator(&inst.effect)?;
    let mut img = RgbaImage::new(w, h);
    let params = Params::new(generator.params(), &inst.params);
    if let Err(e) = generator.generate(&mut img, &params, px_per_mm) {
        log::warn!("generator {}: {e}", inst.effect);
        return None;
    }
    crate::render::premultiplied_pixmap(&img)
}

/// The stack changes the image: some effect is enabled.
pub fn is_active(stack: &[EffectInstance]) -> bool {
    stack.iter().any(|e| e.enabled)
}

/// Identifies what the stack does to an image: equal keys give equal pixels.
pub fn key(stack: &[EffectInstance]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    registry().generation().hash(&mut h);
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
    fn random_seeds_stay_in_range_and_change() {
        for _ in 0..200 {
            let v = random_seed(3, 7, None);
            assert!((3..=7).contains(&v), "{v}");
        }
        // With two values, the other one each time.
        for _ in 0..20 {
            assert_eq!(random_seed(0, 1, Some(0)), 1);
            assert_eq!(random_seed(0, 1, Some(1)), 0);
        }
        assert_eq!(random_seed(5, 5, Some(5)), 5);
        // A new fill starts with a seed of its own; other settings at their defaults.
        let noise = registry().generator("builtin.noise").unwrap();
        let a = EffectInstance::generator(noise).with_random_seeds();
        let b = EffectInstance::generator(noise).with_random_seeds();
        let seed = |i: &EffectInstance| i.params["seed"].as_i64().unwrap();
        assert!((0..=SEED_MAX as i64).contains(&seed(&a)));
        assert_ne!(seed(&a), seed(&b));
        let defaults = EffectInstance::generator(noise);
        assert_eq!(a.params["color1"], defaults.params["color1"]);
    }

    #[test]
    fn roles_are_guessed_only_when_not_given() {
        let seed = ParamSpec::int("seed", "Seed", 0, 99, 0);
        assert_eq!(seed.role(), Some(Role::Seed));
        let angle = ParamSpec::float("a", "A", 0.0, 9.0, 0.0).unit("°");
        assert_eq!(angle.role(), Some(Role::Angle));
        let size = ParamSpec::float("s", "S", 0.0, 9.0, 0.0).unit(" px");
        assert_eq!(size.role(), Some(Role::Size));
        // Only an int is a seed; a given role wins over the guess.
        assert_eq!(ParamSpec::float("seed", "S", 0.0, 9.0, 0.0).role(), None);
        let given = ParamSpec::float("s", "S", 0.0, 9.0, 0.0)
            .unit("px")
            .with_role(Role::Angle);
        assert_eq!(given.role(), Some(Role::Angle));
    }

    #[test]
    fn settings_older_than_compatible_from_are_told_and_brought_up_to_date() {
        let mut reg = Registry::builtin();
        let entry = |v: &str| PluginRef {
            id: "halftone".into(),
            version: v.into(),
        };
        reg.first.plugins.push(plugin::PluginInfo {
            name: "Halftone".into(),
            version: "1.0.0".into(),
            effects: vec!["o/halftone".into()],
            generators: vec!["o/dots".into()],
            path: PathBuf::new(),
            source: None,
            entry: Some(plugin::Installed {
                entry: entry("1.0.0"),
                compatible_from: Some("0.5.0".into()),
                min_sdk: None,
            }),
        });
        let inst = |effect: &str, v: &str| EffectInstance {
            effect: effect.into(),
            enabled: true,
            params: Default::default(),
            links: Default::default(),
            plugin: Some(entry(v)),
            name: None,
        };
        let mut old = inst("o/dots", "0.1.0");
        assert!(reg.update_version(&mut old));
        assert_eq!(old.plugin, Some(entry("1.0.0")));
        // Read the same: brought up to date without a word; so is it once updated.
        let mut recent = inst("o/halftone", "0.5.0");
        assert!(!reg.update_version(&mut recent));
        assert_eq!(recent.plugin, Some(entry("1.0.0")));
        assert!(!reg.update_version(&mut old));
        // Another plugin's, or one not installed, is left as it is.
        let mut other = inst("o/halftone", "0.1.0");
        other.plugin.as_mut().unwrap().id = "other".into();
        assert!(!reg.update_version(&mut other));
        let mut gone = inst("o/gone", "0.1.0");
        assert!(!reg.update_version(&mut gone));
        assert_eq!(gone.plugin, Some(entry("0.1.0")));
    }

    #[test]
    fn plugins_whose_ids_are_taken_are_left_out() {
        let reg = Registry::builtin();
        let loaded = plugin::Loaded {
            effects: builtin::all(),
            generators: generators::all(),
            ..Default::default()
        };
        let n = loaded.effects.len() + loaded.generators.len();
        let part = Part::checked(loaded, |id| reg.params_of(id).is_some());
        assert!(part.effects.is_empty() && part.generators.is_empty());
        assert_eq!(part.errors.len(), n);
        assert!(part.errors[0].contains("left out"));
    }

    #[test]
    fn a_plugin_allowed_that_fails_is_shown_failed() {
        let dir = std::env::temp_dir().join(format!("artwork-fails-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("junk.{}", std::env::consts::DLL_EXTENSION));
        std::fs::write(&path, "").unwrap();
        let reg = Registry::with_plugins(std::slice::from_ref(&dir), &plugin::Settings::default());
        assert_eq!(reg.pending().len(), 1);
        reg.allow(&path, &mut plugin::Settings::default());
        assert!(matches!(reg.files()[0].status, plugin::Status::Failed(_)));
        assert_eq!(reg.errors().len(), 1);
        // The user's settings, read as at startup.
        let _ = plugin::Settings::read();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn plugins_load_from_every_folder() {
        let base = std::env::temp_dir().join(format!("artwork-dirs-{}", std::process::id()));
        let junk = format!("junk.{}", std::env::consts::DLL_EXTENSION);
        let dirs = [base.join("a"), base.join("b")];
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(&junk), "").unwrap();
        }
        // Not allowed: they wait.
        let reg = Registry::with_plugins(&dirs, &plugin::Settings::default());
        assert_eq!(reg.pending().len(), 2);
        assert!(reg.errors().is_empty());
        // The same file name in both: one error each, not merged.
        let mut settings = plugin::Settings::default();
        dirs.iter().for_each(|d| settings.allow_all(d));
        let reg = Registry::with_plugins(&dirs, &settings);
        assert_eq!(reg.errors().len(), 2, "{:?}", reg.errors());
        assert!(reg.get("builtin.blur").is_some());
        let _ = std::fs::remove_dir_all(base);
        // Only folders that exist, once each; the working directory in debug builds only.
        let found = plugin::plugins_dirs();
        assert!(found.iter().all(|d| d.is_dir()));
        let cwd = std::env::current_dir().unwrap().join("plugins");
        assert_eq!(
            found
                .iter()
                .any(|d| Some(d) == cwd.canonicalize().ok().as_ref()),
            cwd.is_dir() && cfg!(debug_assertions)
        );
    }

    #[test]
    fn ease_follows_css_cubic_bezier() {
        let linear = CURVE_PRESETS[0].2;
        for x in [0.0, 0.25, 0.5, 0.9, 1.0] {
            assert!((ease(linear, x) - x).abs() < 1e-4);
        }
        let [_, ease_in, ease_out, in_out] = CURVE_PRESETS.map(|p| p.2);
        // Ease in starts slow, ease out fast; in-out is symmetric.
        assert!(ease(ease_in, 0.3) < 0.15);
        assert!(ease(ease_out, 0.3) > 0.4);
        // Ease out is ease in mirrored.
        for x in [0.1, 0.3, 0.6] {
            assert!((ease(ease_out, x) - (1.0 - ease(ease_in, 1.0 - x))).abs() < 1e-3);
        }
        assert!((ease(in_out, 0.5) - 0.5).abs() < 1e-3);
        assert!((ease(in_out, 0.2) + ease(in_out, 0.8) - 1.0).abs() < 1e-3);
        // CSS's value for ease-in at 0.5.
        assert!((ease(ease_in, 0.5) - 0.3153).abs() < 2e-3);
    }

    #[test]
    fn curve_values_fall_back_and_stay_in_range() {
        let specs = [ParamSpec::curve("c", "C", [0.42, 0.0, 0.58, 1.0])];
        let mut values = ParamValues::new();
        assert_eq!(
            Params::new(&specs, &values).curve("c"),
            [0.42, 0.0, 0.58, 1.0]
        );
        values.insert("c".into(), serde_json::json!([0.2, -1.0, 3.0, 0.5]));
        assert_eq!(
            Params::new(&specs, &values).curve("c"),
            [0.2, 0.0, 1.0, 0.5]
        );
        // Not four numbers: the default.
        values.insert("c".into(), serde_json::json!([0.2, 0.3]));
        assert_eq!(
            Params::new(&specs, &values).curve("c"),
            [0.42, 0.0, 0.58, 1.0]
        );
    }

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
                links: Default::default(),
                plugin: None,
                name: None,
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
