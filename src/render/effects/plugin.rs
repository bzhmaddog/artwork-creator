//! Effects and generators from plugin libraries (`.dll` on Windows, `.so` on Linux) found in the
//! `plugins` folder, through the C ABI of `artwork_plugin_sdk::abi`, and from Python
//! plugins (`.py`) in the same folder, run in processes of their own (see [`python`]).
//!
//! Plugins run in the app's process with its full rights: a crash in one takes the app
//! down. They are loaded once at startup and never unloaded.

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use artwork_plugin_sdk::{self as sdk, abi};
use image::RgbaImage;
use libloading::Library;

use super::{Effect, Generator, ParamKind, ParamSpec, Params, color_from_hex, python};

/// A plugin that loaded.
#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    /// Ids of its effects.
    pub effects: Vec<String>,
    /// Ids of its generators.
    pub generators: Vec<String>,
}

/// An effect of a plugin.
pub struct NativeEffect {
    id: String,
    name: String,
    description: String,
    params: Vec<ParamSpec>,
    plugin: String,
    table: abi::EffectTable,
    /// Keeps the code behind `table` loaded; `None` for a table built in this process.
    _library: Option<Arc<Library>>,
}

// SAFETY: the ABI requires `padding` and `apply` to be callable from any thread, and the
// table to stay valid while the library is loaded, which `_library` ensures.
unsafe impl Send for NativeEffect {}
unsafe impl Sync for NativeEffect {}

/// Longest error message read back from a plugin.
const ERROR_LEN: usize = 1024;

impl Effect for NativeEffect {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn params(&self) -> &[ParamSpec] {
        &self.params
    }

    fn plugin(&self) -> Option<&str> {
        Some(&self.plugin)
    }

    fn padding(&self, params: &Params) -> [f32; 4] {
        let Some(padding) = self.table.padding else {
            return [0.0; 4];
        };
        let values = values_json(params);
        let mut out = [0.0f32; 4];
        // SAFETY: valid table (see above), NUL-terminated values, room for 4 floats.
        let r = unsafe { padding(self.table.user_data, values.as_ptr(), out.as_mut_ptr()) };
        if r != 0 {
            return [0.0; 4];
        }
        out.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
    }

    fn apply(&self, img: &mut RgbaImage, params: &Params, scale: f32) -> Result<(), String> {
        let (w, h) = img.dimensions();
        let stride = w
            .checked_mul(4)
            .ok_or_else(|| "image too wide".to_owned())?;
        let values = values_json(params);
        let mut error = vec![0 as c_char; ERROR_LEN];
        // SAFETY: the buffer holds `h` rows of `stride` bytes; the error buffer is
        // `ERROR_LEN` long.
        let r = unsafe {
            (self.table.apply)(
                self.table.user_data,
                img.as_mut_ptr(),
                w,
                h,
                stride,
                values.as_ptr(),
                scale,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        outcome(r, &mut error)
    }
}

/// What a plugin call returning `r` gave, with the message it wrote to `error`.
fn outcome(r: i32, error: &mut [c_char]) -> Result<(), String> {
    if r == 0 {
        return Ok(());
    }
    // Make sure the message ends within the buffer, whatever the plugin wrote.
    if let Some(last) = error.last_mut() {
        *last = 0;
    }
    // SAFETY: NUL-terminated within the buffer.
    let msg = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
    Err(if msg.is_empty() {
        format!("failed (error {r})")
    } else {
        msg.into_owned()
    })
}

/// A generator of a plugin.
pub struct NativeGenerator {
    id: String,
    name: String,
    description: String,
    params: Vec<ParamSpec>,
    plugin: String,
    table: abi::GeneratorTable,
    /// Keeps the code behind `table` loaded; `None` for a table built in this process.
    _library: Option<Arc<Library>>,
}

// SAFETY: as for `NativeEffect`.
unsafe impl Send for NativeGenerator {}
unsafe impl Sync for NativeGenerator {}

impl Generator for NativeGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn params(&self) -> &[ParamSpec] {
        &self.params
    }

    fn plugin(&self) -> Option<&str> {
        Some(&self.plugin)
    }

    fn generate(&self, img: &mut RgbaImage, params: &Params, px_per_mm: f32) -> Result<(), String> {
        let (w, h) = img.dimensions();
        let stride = w
            .checked_mul(4)
            .ok_or_else(|| "image too wide".to_owned())?;
        let values = values_json(params);
        let mut error = vec![0 as c_char; ERROR_LEN];
        // SAFETY: the buffer holds `h` rows of `stride` bytes; the error buffer is
        // `ERROR_LEN` long.
        let r = unsafe {
            (self.table.generate)(
                self.table.user_data,
                img.as_mut_ptr(),
                w,
                h,
                stride,
                values.as_ptr(),
                px_per_mm,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        outcome(r, &mut error)
    }
}

/// The values passed to a plugin: every parameter, checked against its range.
fn values_json(params: &Params) -> CString {
    // JSON escapes control characters, so there is no NUL in it.
    CString::new(params.resolved().to_string()).unwrap_or_default()
}

/// The folders holding plugins, those that exist, in loading order: `plugins` next to
/// the executable, in `../share/artwork-creator/` from it, and in the app's data folder
/// (the user's plugins). Never the working directory, which may be any folder (one a
/// project was opened from): its plugins would run unasked. Debug builds also look there,
/// for `make run` from the repository.
pub fn plugins_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join("plugins"));
        dirs.push(dir.join("../share").join(crate::APP_ID).join("plugins"));
    }
    dirs.extend(user_plugins_dir());
    if cfg!(debug_assertions)
        && let Ok(cwd) = std::env::current_dir()
    {
        dirs.push(cwd.join("plugins"));
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if let Ok(dir) = dir.canonicalize()
            && dir.is_dir()
            && !found.contains(&dir)
        {
            found.push(dir);
        }
    }
    found
}

/// The user's plugins folder, in the app's data folder; it may not exist yet.
pub fn user_plugins_dir() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("plugins"))
}

/// The user's choices about plugins, kept in `plugins.json` in the app's data folder:
/// the Python plugins allowed to run, each file's path with the SHA-256 of the content
/// allowed (a file that changed is asked about again), and the plugins disabled.
#[derive(Debug, Default)]
pub struct Settings {
    /// Where it is saved; `None`: not saved.
    file: Option<PathBuf>,
    saved: Saved,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    /// Path to hash.
    allowed: std::collections::BTreeMap<String, String>,
    disabled: std::collections::BTreeSet<String>,
}

impl Settings {
    /// The user's, from the data folder (none if it can't be read).
    pub fn read() -> Self {
        Self::read_from(crate::data_dir().map(|d| d.join("plugins.json")))
    }

    pub(super) fn read_from(file: Option<PathBuf>) -> Self {
        let saved = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        Self { file, saved }
    }

    /// Whether this Python plugin, with this content, was allowed.
    pub fn is_allowed(&self, p: &Pending) -> bool {
        self.saved.allowed.get(&key(&p.path)) == Some(&p.hash)
    }

    pub fn allow(&mut self, p: &Pending) {
        self.saved.allowed.insert(key(&p.path), p.hash.clone());
    }

    pub fn is_disabled(&self, path: &Path) -> bool {
        self.saved.disabled.contains(&key(path))
    }

    pub fn set_disabled(&mut self, path: &Path, disabled: bool) {
        if disabled {
            self.saved.disabled.insert(key(path));
        } else {
            self.saved.disabled.remove(&key(path));
        }
    }

    /// Allows every plugin file of `dir`, as it is now.
    #[cfg(test)]
    pub(super) fn allow_all(&mut self, dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if let Ok(p) = pending(&entry.path()) {
                self.allow(&p);
            }
        }
    }

    /// Writes it to its file; an error is logged.
    pub fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                let json = serde_json::to_vec_pretty(&self.saved).unwrap_or_default();
                std::fs::write(file, json)
            });
        if let Err(e) = result {
            log::warn!("cannot save {}: {e}", file.display());
        }
    }
}

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A plugin file found, and what became of it.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginFile {
    pub path: PathBuf,
    pub python: bool,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Loaded: the plugin's name and version.
    Loaded { name: String, version: String },
    /// Disabled by the user: not loaded.
    Disabled,
    /// Waiting for the user to allow it.
    Waiting,
    /// It failed to load, for this reason.
    Failed(String),
}

impl PluginFile {
    /// Its plugin's name once loaded, else its file name.
    pub fn name(&self) -> String {
        match &self.status {
            Status::Loaded { name, .. } => name.clone(),
            _ => self
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        }
    }
}

/// What loading the plugins of a folder gave.
#[derive(Default)]
pub struct Loaded {
    pub effects: Vec<Box<dyn Effect>>,
    pub generators: Vec<Box<dyn Generator>>,
    pub plugins: Vec<PluginInfo>,
    /// One message per library that failed to load.
    pub errors: Vec<String>,
    /// Plugins not loaded: new, or changed since they were allowed.
    pub pending: Vec<Pending>,
    /// Every plugin file, in loading order.
    pub files: Vec<PluginFile>,
}

/// A plugin waiting to be allowed to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub path: PathBuf,
    /// SHA-256 of the file, in hex: what is allowed is this content.
    pub hash: String,
}

/// Loads every library of this platform (`.dll` or `.so`) in `dir`, in name order.
/// Plugins disabled in `settings` are not loaded, and the others only if `settings`
/// allows them with their content; those are in [`Loaded::pending`].
pub fn load_dir(dir: &Path, settings: &Settings) -> Loaded {
    load_dir_with(dir, python::python(), settings)
}

/// [`load_dir`], running the Python plugins with `python`; none load without it.
pub(super) fn load_dir_with(
    dir: &Path,
    python: Option<&'static python::Python>,
    settings: &Settings,
) -> Loaded {
    let mut loaded = Loaded::default();
    let Ok(read) = std::fs::read_dir(dir) else {
        return loaded;
    };
    let mut paths: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| {
                    e.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION) || is_python(p)
                })
        })
        .collect();
    paths.sort();
    let mut python_missing = false;
    for path in paths {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let mut found = PluginFile {
            path: path.clone(),
            python: is_python(&path),
            status: Status::Disabled,
        };
        if settings.is_disabled(&path) {
            log::info!("plugin {} disabled", path.display());
            loaded.files.push(found);
            continue;
        }
        if found.python && python.is_none() {
            python_missing = true;
            found.status = Status::Failed(python::PYTHON_MISSING.to_owned());
            loaded.files.push(found);
            continue;
        }
        let result = match pending(&path) {
            Ok(p) if !settings.is_allowed(&p) => {
                log::info!("plugin {} waits to be allowed", path.display());
                loaded.pending.push(p);
                found.status = Status::Waiting;
                loaded.files.push(found);
                continue;
            }
            Ok(_) => load_file(&path, python),
            Err(e) => Err(e),
        };
        match result {
            Ok((info, effects, generators)) => {
                log::info!(
                    "loaded plugin {} {} from {} ({} effects, {} generators)",
                    info.name,
                    info.version,
                    path.display(),
                    effects.len(),
                    generators.len()
                );
                found.status = Status::Loaded {
                    name: info.name.clone(),
                    version: info.version.clone(),
                };
                loaded.plugins.push(info);
                loaded.effects.extend(effects);
                loaded.generators.extend(generators);
            }
            Err(e) => {
                log::warn!("plugin {} not loaded: {e}", path.display());
                loaded.errors.push(format!("{file}: {e}"));
                found.status = Status::Failed(e);
            }
        }
        loaded.files.push(found);
    }
    if python_missing {
        log::warn!("{}", python::PYTHON_MISSING);
        loaded.errors.push(python::PYTHON_MISSING.to_owned());
    }
    loaded
}

/// Loads one plugin file, a library or a Python plugin run with `python`.
pub(super) fn load_file(
    path: &Path,
    python: Option<&'static python::Python>,
) -> Result<python::Contents, String> {
    if is_python(path) {
        let python = python.ok_or_else(|| python::PYTHON_MISSING.to_owned())?;
        return python::load_script(path, python);
    }
    load_library(path).map(|(info, effects, generators)| {
        (
            info,
            effects
                .into_iter()
                .map(|e| Box::new(e) as Box<dyn Effect>)
                .collect(),
            generators
                .into_iter()
                .map(|g| Box::new(g) as Box<dyn Generator>)
                .collect(),
        )
    })
}

/// The plugin file `path` with the hash of its content.
pub fn pending(path: &Path) -> Result<Pending, String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let hash = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Pending {
        path: path.to_owned(),
        hash,
    })
}

/// A Python plugin, by its `.py` extension.
pub(super) fn is_python(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("py"))
}

/// Loads one plugin library.
pub fn load_library(path: &Path) -> Result<Contents, String> {
    // SAFETY: none can be given: loading runs the library's initializers, and its code
    // is trusted from here on. docs/specs.md warns to install only trusted plugins.
    let library = unsafe { Library::new(path) }.map_err(|e| e.to_string())?;
    let entry = {
        // SAFETY: the symbol has the ABI's entry type.
        let entry: libloading::Symbol<abi::EntryFn> =
            unsafe { library.get(abi::ENTRY_SYMBOL.as_bytes()) }.map_err(|_| {
                format!(
                    "not an Artwork Creator plugin (no {} function)",
                    abi::ENTRY_SYMBOL
                )
            })?;
        *entry
    };
    let library = Arc::new(library);
    // SAFETY: the entry function returns null or a table valid while `library` is loaded.
    unsafe { from_table(entry(), path, Some(library)) }
}

/// What a plugin library holds.
pub type Contents = (PluginInfo, Vec<NativeEffect>, Vec<NativeGenerator>);

/// Reads a plugin table.
///
/// # Safety
/// `table` is null or follows the ABI of its version, and stays valid while `library` is
/// loaded (or forever when `library` is `None`).
pub unsafe fn from_table(
    table: *const abi::PluginTable,
    path: &Path,
    library: Option<Arc<Library>>,
) -> Result<Contents, String> {
    if table.is_null() {
        return Err("the plugin failed to start".into());
    }
    // The version is read first, on its own: the rest of the layout depends on it.
    // SAFETY: every ABI version starts with this field.
    let version = unsafe { std::ptr::addr_of!((*table).abi_version).read() };
    // SAFETY: the version says which layout it is.
    let (name, plugin_version, effect_tables, generator_tables) = match version {
        1 => {
            let t = unsafe { &*(table as *const abi::PluginTableV1) };
            let effects = unsafe { slice(t.effects, t.effect_count) };
            unsafe { (string(t.name), string(t.version), effects, &[][..]) }
        }
        abi::ABI_VERSION => {
            let t = unsafe { &*table };
            let effects = unsafe { slice(t.effects, t.effect_count) };
            let generators = unsafe { slice(t.generators, t.generator_count) };
            unsafe { (string(t.name), string(t.version), effects, generators) }
        }
        _ => {
            return Err(format!(
                "made for plugin interface version {version}, this app uses versions {} to {}",
                abi::OLDEST_ABI_VERSION,
                abi::ABI_VERSION
            ));
        }
    };
    let name = if name.is_empty() {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        name
    };
    let mut ids: Vec<String> = Vec::new();
    let mut effects = Vec::with_capacity(effect_tables.len());
    for e in effect_tables {
        // SAFETY: the strings are NUL-terminated or null.
        let d = unsafe { describe("effect", e.id, e.name, e.description, e.params_json, &ids) }?;
        ids.push(d.id.clone());
        effects.push(NativeEffect {
            id: d.id,
            name: d.name,
            description: d.description,
            params: d.params,
            plugin: name.clone(),
            table: *e,
            _library: library.clone(),
        });
    }
    let mut generators = Vec::with_capacity(generator_tables.len());
    for g in generator_tables {
        // SAFETY: the strings are NUL-terminated or null.
        let d = unsafe {
            describe(
                "generator",
                g.id,
                g.name,
                g.description,
                g.params_json,
                &ids,
            )
        }?;
        ids.push(d.id.clone());
        generators.push(NativeGenerator {
            id: d.id,
            name: d.name,
            description: d.description,
            params: d.params,
            plugin: name.clone(),
            table: *g,
            _library: library.clone(),
        });
    }
    let info = PluginInfo {
        name,
        version: plugin_version,
        path: path.to_owned(),
        effects: effects.iter().map(|e| e.id.clone()).collect(),
        generators: generators.iter().map(|g| g.id.clone()).collect(),
    };
    Ok((info, effects, generators))
}

/// `count` tables at `ptr`, none if it is null.
///
/// # Safety
/// `ptr` is null or points to `count` tables that outlive the slice.
unsafe fn slice<'a, T>(ptr: *const T, count: u32) -> &'a [T] {
    if count == 0 || ptr.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, count as usize) }
    }
}

/// What an effect or a generator of a plugin says about itself, checked.
pub(super) struct Described {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<ParamSpec>,
}

/// Reads and checks the description of an effect or generator (`what`), whose id must
/// not be one of `taken`.
///
/// # Safety
/// The strings are NUL-terminated or null.
unsafe fn describe(
    what: &str,
    id: *const c_char,
    name: *const c_char,
    description: *const c_char,
    params: *const c_char,
    taken: &[String],
) -> Result<Described, String> {
    let (id, name, description, params) = unsafe {
        (
            string(id),
            string(name),
            string(description),
            string(params),
        )
    };
    check(what, id, name, description, &params, taken)
}

/// Checks the description of an effect or generator (`what`), whose id must not be one
/// of `taken`; `params` is its JSON parameter list.
pub(super) fn check(
    what: &str,
    id: String,
    name: String,
    description: String,
    params: &str,
    taken: &[String],
) -> Result<Described, String> {
    let err = |m: String| format!("{what} \"{id}\": {m}");
    if id.is_empty() {
        return Err(format!("{what} without an id"));
    }
    if id.starts_with("builtin.") {
        return Err(err("ids starting with \"builtin.\" are reserved".into()));
    }
    if taken.contains(&id) {
        return Err(err("id used twice".into()));
    }
    let params = param_specs(params).map_err(err)?;
    Ok(Described {
        name: if name.is_empty() { id.clone() } else { name },
        id,
        description,
        params,
    })
}

/// # Safety
/// `s` is null or NUL-terminated.
unsafe fn string(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

/// Parses and checks a plugin's parameter list.
fn param_specs(json: &str) -> Result<Vec<ParamSpec>, String> {
    let json = if json.trim().is_empty() { "[]" } else { json };
    let params: Vec<sdk::Param> =
        serde_json::from_str(json).map_err(|e| format!("invalid parameter list: {e}"))?;
    let mut specs: Vec<ParamSpec> = Vec::with_capacity(params.len());
    for p in params {
        let bad = |m: &str| format!("parameter \"{}\": {m}", p.id);
        if p.id.is_empty() {
            return Err("a parameter has no id".into());
        }
        if specs.iter().any(|s| s.id == p.id) {
            return Err(bad("id used twice"));
        }
        let kind = match p.kind {
            sdk::ParamKind::Float {
                min,
                max,
                default,
                unit,
                decimals,
            } => {
                if !(min.is_finite() && max.is_finite() && min <= max) {
                    return Err(bad("invalid range"));
                }
                ParamKind::Float {
                    min,
                    max,
                    default: if default.is_finite() { default } else { min }.clamp(min, max),
                    unit,
                    decimals: decimals.min(6),
                }
            }
            sdk::ParamKind::Int {
                min,
                max,
                default,
                unit,
            } => {
                if min > max {
                    return Err(bad("invalid range"));
                }
                ParamKind::Int {
                    min,
                    max,
                    default: default.clamp(min, max),
                    unit,
                }
            }
            sdk::ParamKind::Bool { default } => ParamKind::Bool { default },
            sdk::ParamKind::Color { default } => ParamKind::Color {
                default: color_from_hex(&default)
                    .ok_or_else(|| bad("default is not a #rrggbbaa color"))?,
            },
            sdk::ParamKind::Choice { options, default } => {
                if options.is_empty() {
                    return Err(bad("no options"));
                }
                let default = if options.iter().any(|o| o.id == default) {
                    default
                } else {
                    options[0].id.clone()
                };
                ParamKind::Choice {
                    options: options.into_iter().map(|o| (o.id, o.label)).collect(),
                    default,
                }
            }
        };
        specs.push(ParamSpec {
            id: p.id,
            label: p.label,
            kind,
        });
    }
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::effects::{EffectInstance, ParamValues};
    use artwork_plugin_sdk::{Image, Param, Values};
    use serde_json::Value;

    /// Fills the image with a color; panics or fails on request.
    struct Fill;

    impl sdk::Effect for Fill {
        fn id(&self) -> &str {
            "test.fill"
        }
        fn name(&self) -> &str {
            "Fill"
        }
        fn params(&self) -> Vec<Param> {
            vec![
                Param::color("color", "Color", [10, 20, 30, 255]),
                Param::float("margin", "Margin", 0.0, 10.0, 0.0),
                Param::choice(
                    "fail",
                    "Fail",
                    &[("no", "No"), ("err", "Error"), ("panic", "Panic")],
                    "no",
                ),
            ]
        }
        fn padding(&self, v: &Values) -> [f32; 4] {
            [v.f32("margin"); 4]
        }
        fn apply(&self, img: &mut Image, v: &Values, _scale: f32) -> Result<(), String> {
            match v.choice("fail") {
                "err" => return Err("asked to fail".into()),
                "panic" => panic!("asked to panic"),
                _ => {}
            }
            let c = v.color("color");
            img.for_each_pixel(|px| *px = c);
            Ok(())
        }
    }

    /// Draws the left half of the image in a color.
    struct Half;

    impl sdk::Generator for Half {
        fn id(&self) -> &str {
            "test.half"
        }
        fn name(&self) -> &str {
            "Half"
        }
        fn params(&self) -> Vec<Param> {
            vec![Param::color("color", "Color", [10, 20, 30, 255])]
        }
        fn generate(&self, img: &mut Image, v: &Values, px_per_mm: f32) -> Result<(), String> {
            if px_per_mm <= 0.0 {
                return Err("no size".into());
            }
            let (c, w) = (v.color("color"), img.width() / 2);
            img.par_rows(|_, row| {
                for px in &mut row.as_chunks_mut::<4>().0[..w as usize] {
                    *px = c;
                }
            });
            Ok(())
        }
    }

    fn table() -> &'static abi::PluginTable {
        sdk::build_table("Test", "1.0", || {
            (vec![Box::new(Fill)], vec![Box::new(Half)])
        })
        .unwrap()
    }

    fn fill() -> NativeEffect {
        let (info, mut effects, _) =
            unsafe { from_table(table(), Path::new("test.so"), None) }.unwrap();
        assert_eq!((info.name.as_str(), info.version.as_str()), ("Test", "1.0"));
        assert_eq!(info.effects, ["test.fill"]);
        assert_eq!(info.generators, ["test.half"]);
        effects.pop().unwrap()
    }

    #[test]
    fn plugin_generators_work_through_the_abi() {
        let (_, _, mut gens) = unsafe { from_table(table(), Path::new("test.so"), None) }.unwrap();
        let g = gens.pop().unwrap();
        assert_eq!((g.name(), g.plugin()), ("Half", Some("Test")));
        let mut inst = EffectInstance::generator(&g);
        inst.params.insert("color".into(), Value::from("#ff000080"));
        let p = Params::new(g.params(), &inst.params);
        let mut img = RgbaImage::new(4, 2);
        g.generate(&mut img, &p, 1.0).unwrap();
        assert_eq!(img.get_pixel(1, 1).0, [255, 0, 0, 128]);
        assert_eq!(img.get_pixel(2, 1).0, [0; 4]);
        assert!(
            g.generate(&mut img, &p, 0.0)
                .unwrap_err()
                .contains("no size")
        );
    }

    #[test]
    fn version_1_plugins_still_load() {
        let t = table();
        let v1 = abi::PluginTableV1 {
            abi_version: 1,
            effect_count: t.effect_count,
            name: t.name,
            version: t.version,
            effects: t.effects,
        };
        let table = &v1 as *const abi::PluginTableV1 as *const abi::PluginTable;
        let (info, effects, gens) =
            unsafe { from_table(table, Path::new("old.so"), None) }.unwrap();
        assert_eq!(info.effects, ["test.fill"]);
        assert_eq!(effects.len(), 1);
        assert!(gens.is_empty() && info.generators.is_empty());
    }

    #[test]
    fn plugin_effects_work_through_the_abi() {
        let e = fill();
        assert_eq!((e.name(), e.plugin()), ("Fill", Some("Test")));
        assert_eq!(e.params().len(), 3);
        let mut inst = EffectInstance::new(&e);
        inst.params.insert("color".into(), Value::from("#ff000080"));
        inst.params.insert("margin".into(), Value::from(50.0)); // clamped to 10
        let p = Params::new(e.params(), &inst.params);
        assert_eq!(e.padding(&p), [10.0; 4]);
        let mut img = RgbaImage::new(3, 2);
        e.apply(&mut img, &p, 1.0).unwrap();
        assert!(img.pixels().all(|px| px.0 == [255, 0, 0, 128]));
    }

    #[test]
    fn plugin_errors_and_panics_are_reported() {
        let e = fill();
        let mut values = ParamValues::new();
        let mut img = RgbaImage::new(1, 1);
        for (mode, msg) in [("err", "asked to fail"), ("panic", "asked to panic")] {
            values.insert("fail".into(), Value::from(mode));
            let r = e.apply(&mut img, &Params::new(e.params(), &values), 1.0);
            assert!(r.unwrap_err().contains(msg), "{mode}");
        }
    }

    #[test]
    fn bad_tables_are_rejected() {
        let path = Path::new("x.so");
        assert!(unsafe { from_table(std::ptr::null(), path, None) }.is_err());
        let table = abi::PluginTable {
            abi_version: abi::ABI_VERSION + 1,
            effect_count: 0,
            name: std::ptr::null(),
            version: std::ptr::null(),
            effects: std::ptr::null(),
            generator_count: 0,
            generators: std::ptr::null(),
        };
        let Err(e) = (unsafe { from_table(&table, path, None) }) else {
            panic!("accepted");
        };
        assert!(e.contains("version"), "{e}");
    }

    #[test]
    fn param_lists_are_checked() {
        let ok = param_specs(
            r#"[{"id":"a","label":"A","type":"choice","options":[{"id":"x","label":"X"}],"default":"zz"},
                {"id":"b","label":"B","type":"float","min":0,"max":1,"default":5}]"#,
        )
        .unwrap();
        assert!(matches!(&ok[0].kind, ParamKind::Choice { default, .. } if default == "x"));
        assert!(matches!(ok[1].kind, ParamKind::Float { default, .. } if default == 1.0));
        for bad in [
            "{}",
            r#"[{"id":"a","label":"A","type":"float","min":2,"max":1,"default":1}]"#,
            r#"[{"id":"a","label":"A","type":"color","default":"red"}]"#,
            r#"[{"id":"a","label":"A","type":"bool","default":true},{"id":"a","label":"A","type":"bool","default":true}]"#,
            r#"[{"id":"a","label":"A","type":"choice","options":[],"default":""}]"#,
        ] {
            assert!(param_specs(bad).is_err(), "{bad}");
        }
        assert!(param_specs("").unwrap().is_empty());
    }

    #[test]
    fn disabled_plugins_are_not_loaded_and_settings_are_kept() {
        let dir = std::env::temp_dir().join(format!("artwork-disabled-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let library = dir.join(format!("junk.{}", std::env::consts::DLL_EXTENSION));
        let script = dir.join("off.py");
        for f in [&library, &script] {
            std::fs::write(f, "plugin = None\n").unwrap();
        }
        let saved = dir.join("plugins.json");
        let mut settings = Settings::read_from(Some(saved.clone()));
        settings.set_disabled(&library, true);
        settings.set_disabled(&script, true);
        settings.allow(&pending(&script).unwrap());
        settings.save();
        let settings = Settings::read_from(Some(saved.clone()));
        let loaded = load_dir_with(&dir, None, &settings);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let status: Vec<_> = loaded
            .files
            .iter()
            .map(|f| (f.python, f.status.clone()))
            .collect();
        assert_eq!(
            status,
            [(false, Status::Disabled), (true, Status::Disabled)]
        );
        assert_eq!(loaded.files[1].name(), "off.py");
        // Enabled again: waiting to be allowed, then loaded (or failing).
        let mut settings = settings;
        settings.set_disabled(&library, false);
        let loaded = load_dir_with(&dir, None, &settings);
        assert_eq!(loaded.files[0].status, Status::Waiting);
        assert_eq!(loaded.pending[0].path, library);
        settings.allow(&pending(&library).unwrap());
        let loaded = load_dir_with(&dir, None, &settings);
        assert!(matches!(loaded.files[0].status, Status::Failed(_)));
        Settings::default().save();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn python_plugins_need_python() {
        let dir = std::env::temp_dir().join(format!("artwork-plugins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let library = format!("junk.{}", std::env::consts::DLL_EXTENSION);
        for name in ["a.py", "b.py", library.as_str(), "notes.txt"] {
            std::fs::write(dir.join(name), "plugin = None\n").unwrap();
        }
        let mut settings = Settings::default();
        settings.allow_all(&dir);
        let loaded = load_dir_with(&dir, None, &settings);
        assert_eq!(loaded.errors.len(), 2, "{:?}", loaded.errors);
        assert!(loaded.errors[0].starts_with(&library));
        assert_eq!(loaded.errors[1], python::PYTHON_MISSING);
        assert!(loaded.plugins.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Loads a real library, e.g. the halftone example built by `make plugins`:
    /// `ARTWORK_TEST_PLUGIN=plugins/libhalftone.so cargo test loads_plugin_library`.
    #[test]
    fn loads_plugin_library() {
        let Some(path) = std::env::var_os("ARTWORK_TEST_PLUGIN") else {
            return;
        };
        let (info, effects, generators) = load_library(Path::new(&path)).unwrap();
        assert!(!effects.is_empty(), "{info:?}");
        for g in &generators {
            let inst = EffectInstance::generator(g);
            let mut img = RgbaImage::new(64, 48);
            g.generate(&mut img, &Params::new(g.params(), &inst.params), 4.0)
                .unwrap();
        }
        for e in &effects {
            let inst = EffectInstance::new(e);
            let mut img = RgbaImage::from_fn(64, 48, |x, y| {
                image::Rgba([(x * 4) as u8, (y * 5) as u8, 128, 255])
            });
            e.apply(&mut img, &Params::new(e.params(), &inst.params), 0.5)
                .unwrap();
        }
    }
}
