//! Effects and generators from plugin libraries (`.dll` on Windows, `.so` on Linux) found in the
//! `plugins` folder, through the C ABI of `artwork_plugin_sdk::abi`, and from Python
//! plugins (`.py`) in the same folder, run in processes of their own (see [`python`]).
//!
//! Plugins run in the app's process with its full rights: a crash in one takes the app
//! down. They are loaded at startup, or when installed or allowed; one uninstalled is
//! unloaded (see [`Registry::remove`](super::Registry::remove)).

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use artwork_plugin_sdk::{self as sdk, abi};
use image::RgbaImage;
use libloading::Library;

use super::texts::{self, Shown, Texts};
use super::{Effect, Generator, ParamKind, ParamSpec, Params, Role, color_from_hex, python};

/// A plugin that loaded.
#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    /// Ids of its effects.
    pub effects: Vec<String>,
    /// Ids of its generators.
    pub generators: Vec<String>,
    /// Its file.
    pub path: PathBuf,
    /// The repository it was installed from; `None` for one put in a plugins folder by
    /// hand.
    pub source: Option<Source>,
    /// Its id and version in that repository, as recorded when it was installed.
    pub entry: Option<Installed>,
}

/// A plugin file installed from a repository, as its origin's `source.json` records it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Installed {
    #[serde(flatten)]
    pub entry: super::PluginRef,
    /// The oldest version of the plugin whose saved settings this one reads; all of them
    /// if `None` (see [`Version::compatible_from`](super::repository::Version)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatible_from: Option<String>,
    /// For a Python plugin, the oldest SDK it works with (its `engines.sdk` starts there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_sdk: Option<String>,
}

/// A repository plugins were installed from, as `source.json` in its origin's folder
/// says (see [`origin`]).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Source {
    /// Its folder's name.
    #[serde(skip)]
    pub origin: String,
    /// The name its list gives; empty if unknown.
    pub name: String,
    /// The address of its list.
    pub url: String,
    /// The plugins installed from it: by file name, their id and version there.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub plugins: std::collections::BTreeMap<String, Installed>,
}

/// The file in an origin's folder naming its repository.
const SOURCE_FILE: &str = "source.json";

impl Source {
    /// The repository of the origin folder `dir`, from its `source.json`.
    fn read(dir: &Path, origin: &str) -> Self {
        let file = std::fs::read(dir.join(SOURCE_FILE)).ok();
        let mut s: Self = file
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        s.origin = origin.to_owned();
        s
    }

    /// Writes the `source.json` of each repository of `lists` (address and name) that
    /// has a folder in the user's plugins folder, when it doesn't say that already: the
    /// folders of plugins installed without it, or of a repository renamed since.
    pub fn name_folders<'a>(lists: impl IntoIterator<Item = (&'a str, &'a str)>) {
        let Some(plugins) = user_plugins_dir() else {
            return;
        };
        for (url, name) in lists {
            let origin = origin(url);
            let dir = plugins.join(&origin);
            let s = Self::read(&dir, &origin);
            if dir.is_dir()
                && (s.name != name || s.url != url)
                && let Err(e) = Self::write(&dir, name, url)
            {
                log::warn!("cannot write {}: {e}", dir.join(SOURCE_FILE).display());
            }
        }
    }

    /// Writes the `source.json` of the repository `name` at `url` in `dir`, its origin's
    /// folder (created if need be).
    pub fn write(dir: &Path, name: &str, url: &str) -> std::io::Result<()> {
        let mut s = Self::read(dir, "");
        s.name = name.to_owned();
        s.url = url.to_owned();
        s.save(dir)
    }

    /// Records in the `source.json` of `dir` that the plugin file `file` is `entry`,
    /// in place of the file `replaces`.
    pub fn record(
        dir: &Path,
        file: &str,
        entry: Installed,
        replaces: Option<&str>,
    ) -> std::io::Result<()> {
        let mut s = Self::read(dir, "");
        if let Some(old) = replaces {
            s.plugins.remove(old);
        }
        s.plugins.insert(file.to_owned(), entry);
        s.save(dir)
    }

    fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(SOURCE_FILE), serde_json::to_vec_pretty(self)?)
    }
}

/// An effect of a plugin.
pub struct NativeEffect {
    id: String,
    texts: Texts,
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
        &self.texts.current().name
    }

    fn description(&self) -> &str {
        &self.texts.current().description
    }

    fn params(&self) -> &[ParamSpec] {
        &self.texts.current().params
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
    texts: Texts,
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
        &self.texts.current().name
    }

    fn description(&self) -> &str {
        &self.texts.current().description
    }

    fn params(&self) -> &[ParamSpec] {
        &self.texts.current().params
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

/// Length of an origin's name (see [`origin`]).
const ORIGIN_LEN: usize = 16;

/// The origin of the plugins installed from the repository at `url`: a hash of its
/// address, the name of the folder of the user's plugins folder they go in. A plugin's
/// effects and generators from there have ids made of it and theirs, `<origin>/<id>`:
/// the same plugin from two repositories is two plugins.
///
/// The official repository's is that of its former address: its plugins, installed before
/// it moved, stay in their folder, and the effects saved in documents keep their ids.
pub fn origin(url: &str) -> String {
    use super::repository::{OFFICIAL, OFFICIAL_BEFORE};
    let url = if url == OFFICIAL {
        OFFICIAL_BEFORE
    } else {
        url
    };
    hash(url.as_bytes())[..ORIGIN_LEN].to_owned()
}

/// The origin of the plugin file `path`: its folder's name if that is an origin's, else
/// `None` (a plugin put in a plugins folder by hand).
pub fn origin_of(path: &Path) -> Option<&str> {
    let name = path.parent()?.file_name()?.to_str()?;
    is_origin(name).then_some(name)
}

/// The id and version of the plugin file `path` in the repository it was installed from,
/// as its origin's `source.json` recorded them; `None` for a file not installed from one.
pub fn recorded(path: &Path) -> Option<Installed> {
    let file = path.file_name()?.to_str()?;
    source_of(path)?.plugins.remove(file)
}

/// The repository the plugin file `path` was installed from, as its origin's
/// `source.json` says; `None` for a file not installed from one.
pub fn source_of(path: &Path) -> Option<Source> {
    let origin = origin_of(path)?;
    Some(Source::read(path.parent()?, origin))
}

/// Why the Python plugin `path` doesn't run with this app, if its install recorded an
/// SDK older than the oldest this app runs (see [`artwork_plugin_sdk::OLDEST_VERSION`]).
fn sdk_no_longer_run(path: &Path) -> Option<String> {
    let min = recorded(path)?.min_sdk?;
    (!super::repository::sdk_still_supported(Some(&min))).then(|| {
        format!(
            "made for plugin SDK {min}; this app runs plugins made for SDK {} and later: update it",
            artwork_plugin_sdk::OLDEST_VERSION
        )
    })
}

/// Whether `name` is an origin's (see [`origin`]).
pub fn is_origin(name: &str) -> bool {
    name.len() == ORIGIN_LEN && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The user's plugins folder, in the app's data folder; it may not exist yet.
pub fn user_plugins_dir() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("plugins"))
}

/// The user's choices about plugins, kept in `plugins.json` in the app's data folder:
/// the Python plugins allowed to run, each file's path with the SHA-256 of the content
/// allowed (a file that changed is asked about again), the plugins disabled, and the
/// plugin repositories the user added.
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
    /// Addresses of plugin lists; `None`: never changed, the official one only.
    repositories: Option<Vec<String>>,
}

impl Settings {
    /// The user's, from the data folder (none if it can't be read).
    pub fn read() -> Self {
        Self::read_from(crate::data_dir().map(|d| d.join("plugins.json")))
    }

    pub(crate) fn read_from(file: Option<PathBuf>) -> Self {
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

    /// Drops what is kept about the plugin file `path`, uninstalled.
    pub fn forget(&mut self, path: &Path) {
        self.saved.allowed.remove(&key(path));
        self.saved.disabled.remove(&key(path));
    }

    /// The plugin repositories, in the order added: the official one until removed. A
    /// former address is read as the current one (see [`repository::current`]).
    ///
    /// [`repository::current`]: super::repository::current
    pub fn repositories(&self) -> Vec<String> {
        let Some(saved) = &self.saved.repositories else {
            return vec![super::repository::OFFICIAL.to_owned()];
        };
        let mut list: Vec<String> = Vec::new();
        for url in saved.iter().map(|u| super::repository::current(u)) {
            if !list.iter().any(|r| r == url) {
                list.push(url.to_owned());
            }
        }
        list
    }

    pub fn add_repository(&mut self, url: &str) {
        let url = super::repository::current(url);
        let mut list = self.repositories();
        if !list.iter().any(|r| r == url) {
            list.push(url.to_owned());
        }
        self.saved.repositories = Some(list);
    }

    pub fn remove_repository(&mut self, url: &str) {
        let url = super::repository::current(url);
        let mut list = self.repositories();
        list.retain(|r| r != url);
        self.saved.repositories = Some(list);
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

/// Loads every library of this platform (`.dll` or `.so`) and Python plugin in `dir`,
/// in name order, then those of its origins' folders (see [`origin`]).
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
    let entries = |dir: &Path| -> Vec<PathBuf> {
        let read = std::fs::read_dir(dir).into_iter().flatten().flatten();
        let mut paths: Vec<PathBuf> = read.map(|e| e.path()).collect();
        paths.sort();
        paths
    };
    let is_plugin = |p: &PathBuf| {
        p.is_file()
            && p.extension().is_some_and(|e| {
                e.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION) || is_python(p)
            })
    };
    let all = entries(dir);
    let mut paths: Vec<PathBuf> = all.iter().filter(|p| is_plugin(p)).cloned().collect();
    for origin in all.iter().filter(|p| {
        p.is_dir()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(is_origin)
    }) {
        paths.extend(entries(origin).into_iter().filter(is_plugin));
    }
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
        // Installed for an SDK this app no longer runs: not started, it would fail.
        if found.python
            && let Some(e) = sdk_no_longer_run(&path)
        {
            log::warn!("plugin {} not loaded: {e}", path.display());
            loaded.errors.push(format!("{file}: {e}"));
            found.status = Status::Failed(e);
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

/// Loads one plugin file, a library or a Python plugin run with `python`; the ids of
/// its effects and generators start with its origin, if it has one (see [`origin`]).
pub(super) fn load_file(
    path: &Path,
    python: Option<&'static python::Python>,
) -> Result<python::Contents, String> {
    let (mut info, effects, generators) = load_file_as_is(path, python)?;
    let Some(origin) = origin_of(path) else {
        return Ok((info, effects, generators));
    };
    info.source = path.parent().map(|dir| Source::read(dir, origin));
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    info.entry = (info.source.as_ref()).and_then(|s| s.plugins.get(file.as_ref()).cloned());
    let id = |id: &str| format!("{origin}/{id}");
    info.effects = info.effects.iter().map(|e| id(e)).collect();
    info.generators = info.generators.iter().map(|g| id(g)).collect();
    let effects = effects
        .into_iter()
        .map(|e| {
            Box::new(FromOrigin {
                id: id(e.id()),
                inner: e,
            }) as Box<dyn Effect>
        })
        .collect();
    let generators = generators
        .into_iter()
        .map(|g| {
            Box::new(FromOrigin {
                id: id(g.id()),
                inner: g,
            }) as Box<dyn Generator>
        })
        .collect();
    Ok((info, effects, generators))
}

/// An effect or a generator of a plugin with an origin, under its id with the origin.
struct FromOrigin<T: ?Sized> {
    id: String,
    inner: Box<T>,
}

impl Effect for FromOrigin<dyn Effect> {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn params(&self) -> &[ParamSpec] {
        self.inner.params()
    }

    fn plugin(&self) -> Option<&str> {
        self.inner.plugin()
    }

    fn padding(&self, params: &Params) -> [f32; 4] {
        self.inner.padding(params)
    }

    fn apply(&self, img: &mut RgbaImage, params: &Params, scale: f32) -> Result<(), String> {
        self.inner.apply(img, params, scale)
    }

    fn unload(&self) {
        self.inner.unload();
    }
}

impl Generator for FromOrigin<dyn Generator> {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn params(&self) -> &[ParamSpec] {
        self.inner.params()
    }

    fn plugin(&self) -> Option<&str> {
        self.inner.plugin()
    }

    fn generate(&self, img: &mut RgbaImage, params: &Params, px_per_mm: f32) -> Result<(), String> {
        self.inner.generate(img, params, px_per_mm)
    }

    fn unload(&self) {
        self.inner.unload();
    }
}

/// [`load_file`], the ids as the plugin gives them.
fn load_file_as_is(
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
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(Pending {
        path: path.to_owned(),
        hash: hash(&data),
    })
}

/// The SHA-256 of `data`, in hex.
pub fn hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
    let (name, plugin_version, effect_tables, generator_tables, other_texts) = match version {
        1 => {
            let t = unsafe { &*(table as *const abi::PluginTableV1) };
            let effects = unsafe { slice(t.effects, t.effect_count) };
            unsafe { (string(t.name), string(t.version), effects, &[][..], None) }
        }
        2 => {
            let t = unsafe { &*(table as *const abi::PluginTableV2) };
            let effects = unsafe { slice(t.effects, t.effect_count) };
            let generators = unsafe { slice(t.generators, t.generator_count) };
            unsafe { (string(t.name), string(t.version), effects, generators, None) }
        }
        abi::ABI_VERSION => {
            let t = unsafe { &*table };
            let effects = unsafe { slice(t.effects, t.effect_count) };
            let generators = unsafe { slice(t.generators, t.generator_count) };
            let other = t.texts.map(|f| (f, t.texts_data));
            unsafe {
                (
                    string(t.name),
                    string(t.version),
                    effects,
                    generators,
                    other,
                )
            }
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
            texts: Texts::new(d.shown),
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
            texts: Texts::new(d.shown),
            plugin: name.clone(),
            table: *g,
            _library: library.clone(),
        });
    }
    // Its texts in each language of the app.
    for (language, code) in texts::LANGUAGES {
        let Some((f, data)) = other_texts else {
            break;
        };
        let code = CString::new(code).expect("no NUL");
        // SAFETY: the ABI's texts function, with its data; it returns null or a
        // NUL-terminated string valid while the library is loaded.
        let json = unsafe { string(f(data, code.as_ptr())) };
        if json.is_empty() {
            continue;
        }
        match serde_json::from_str(&json) {
            Ok(v) => {
                let e = effects.iter_mut().map(|e| (e.id.as_str(), &mut e.texts));
                texts::read(&v, "effects", language, e);
                let g = generators.iter_mut().map(|g| (g.id.as_str(), &mut g.texts));
                texts::read(&v, "generators", language, g);
            }
            Err(e) => log::warn!("{name}: texts in {language:?}: {e}"),
        }
    }
    let info = PluginInfo {
        name,
        version: plugin_version,
        effects: effects.iter().map(|e| e.id.clone()).collect(),
        generators: generators.iter().map(|g| g.id.clone()).collect(),
        path: path.to_owned(),
        source: None,
        entry: None,
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
    /// In U.S. English.
    pub shown: Shown,
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
    if id.contains('/') {
        return Err(err("ids can't contain \"/\"".into()));
    }
    if taken.contains(&id) {
        return Err(err("id used twice".into()));
    }
    let params = param_specs(params).map_err(err)?;
    Ok(Described {
        shown: Shown {
            name: if name.is_empty() { id.clone() } else { name },
            description,
            params,
        },
        id,
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
pub(super) fn param_specs(json: &str) -> Result<Vec<ParamSpec>, String> {
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
        // A role that doesn't fit the type, or unknown, is left out.
        let role = match (p.role, &kind) {
            (Some(sdk::Role::Seed), ParamKind::Int { .. }) => Some(Role::Seed),
            (Some(sdk::Role::Angle), ParamKind::Float { .. }) => Some(Role::Angle),
            (Some(sdk::Role::Size), ParamKind::Float { .. }) => Some(Role::Size),
            _ => None,
        };
        // A blank group name is no group.
        let group = p
            .group
            .map(|g| g.trim().to_owned())
            .filter(|g| !g.is_empty());
        specs.push(ParamSpec {
            id: p.id,
            label: p.label,
            kind,
            role,
            group,
        });
    }
    Ok(specs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;
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
    fn plugins_give_their_texts_in_the_app_s_languages() {
        let fr = r#"{"Fill": "Remplir", "Color": "Couleur", "No": "Non", "Half": "Moitie"}"#;
        let gb = r#"{"Color": "Colour"}"#;
        let table =
            sdk::build_translated_table("Test", "1.0", &[("fr", fr), ("en-GB", gb)], || {
                (vec![Box::new(Fill)], vec![Box::new(Half)])
            })
            .unwrap();
        let (_, effects, gens) = unsafe { from_table(table, Path::new("t.so"), None) }.unwrap();
        let fill = &effects[0].texts;
        let (us, fr, gb) = (
            fill.get(Language::English),
            fill.get(Language::French),
            fill.get(Language::BritishEnglish),
        );
        assert_eq!(
            (us.name.as_str(), fr.name.as_str(), gb.name.as_str()),
            ("Fill", "Remplir", "Fill")
        );
        let labels = |s: &Shown| s.params.iter().map(|p| p.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(us), ["Color", "Margin", "Fail"]);
        assert_eq!(labels(fr), ["Couleur", "Margin", "Fail"]);
        assert_eq!(labels(gb), ["Colour", "Margin", "Fail"]);
        assert!(
            matches!(&fr.params[2].kind, ParamKind::Choice { options, .. } if options[0].1 == "Non")
        );
        assert_eq!(gens[0].texts.get(Language::French).name, "Moitie");
        // A catalog that is not one of strings is left out, not the plugin.
        let bad = r#"{"Fill": 3}"#;
        let table = sdk::build_translated_table(
            "Test",
            "1.0",
            &[("fr", bad), ("en-GB", r#"{"Color": "Colour"}"#)],
            || (vec![Box::new(Fill)], vec![]),
        )
        .unwrap();
        let (_, effects, _) = unsafe { from_table(table, Path::new("t.so"), None) }.unwrap();
        let texts = &effects[0].texts;
        assert_eq!(texts.get(Language::French).name, "Fill");
        assert_eq!(labels(texts.get(Language::BritishEnglish))[0], "Colour");
        // A version 2 table has none: English.
        let (_, effects, _) = unsafe { from_table(v2(table), Path::new("t.so"), None) }.unwrap();
        assert_eq!(effects[0].texts.get(Language::French).name, "Fill");
    }

    /// The version 2 table of `t`, leaked.
    fn v2(t: &abi::PluginTable) -> *const abi::PluginTable {
        let v2 = Box::leak(Box::new(abi::PluginTableV2 {
            abi_version: 2,
            effect_count: t.effect_count,
            name: t.name,
            version: t.version,
            effects: t.effects,
            generator_count: t.generator_count,
            generators: t.generators,
        }));
        v2 as *const abi::PluginTableV2 as *const abi::PluginTable
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
            texts_data: std::ptr::null(),
            texts: None,
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
    fn param_roles_are_kept_when_they_fit() {
        let specs = param_specs(
            r#"[{"id":"v","label":"V","type":"int","min":0,"max":99,"default":0,"role":"seed"},
                {"id":"t","label":"T","type":"float","min":0,"max":9,"default":0,"role":"angle"},
                {"id":"n","label":"N","type":"int","min":0,"max":9,"default":0,"role":"angle"},
                {"id":"h","label":"H","type":"bool","default":true,"role":"hologram"}]"#,
        )
        .unwrap();
        let roles: Vec<_> = specs.iter().map(|s| s.role).collect();
        assert_eq!(roles, [Some(Role::Seed), Some(Role::Angle), None, None]);
    }

    #[test]
    fn param_groups_are_kept_and_blank_ones_dropped() {
        let specs = param_specs(
            r#"[{"id":"a","label":"A","type":"bool","default":true,"group":" Grain "},
                {"id":"b","label":"B","type":"bool","default":true,"group":""},
                {"id":"c","label":"C","type":"bool","default":true}]"#,
        )
        .unwrap();
        let groups: Vec<_> = specs.iter().map(|s| s.group.as_deref()).collect();
        assert_eq!(groups, [Some("Grain"), None, None]);
    }

    #[test]
    fn the_official_repository_s_former_address_is_read_as_its_current_one() {
        use super::super::repository::{OFFICIAL, OFFICIAL_BEFORE};
        // Its plugins stay in their folder, under their ids.
        assert_eq!(origin(OFFICIAL), origin(OFFICIAL_BEFORE));
        assert_ne!(origin(OFFICIAL), origin("https://example.com/plugins.json"));
        let dir = std::env::temp_dir().join(format!("artwork-moved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = dir.join("plugins.json");
        let other = "https://example.com/plugins.json";
        let both = [OFFICIAL_BEFORE, other, OFFICIAL];
        std::fs::write(
            &saved,
            serde_json::json!({ "repositories": both }).to_string(),
        )
        .unwrap();
        let mut settings = Settings::read_from(Some(saved.clone()));
        assert_eq!(settings.repositories(), [OFFICIAL, other]);
        settings.add_repository(OFFICIAL_BEFORE);
        assert_eq!(settings.repositories(), [OFFICIAL, other]);
        settings.save();
        let mut settings = Settings::read_from(Some(saved.clone()));
        assert_eq!(settings.repositories(), [OFFICIAL, other]);
        settings.remove_repository(OFFICIAL_BEFORE);
        assert_eq!(settings.repositories(), [other]);
        std::fs::remove_dir_all(&dir).unwrap();
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

    /// Loads a real library, e.g. one from the plugin repository:
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

    #[test]
    fn a_source_is_named_by_its_file_or_its_origin() {
        let dir = std::env::temp_dir().join(format!("artwork-source-{}", std::process::id()));
        let origin = origin("http://localhost:8000/plugins.json");
        let folder = dir.join(&origin);
        assert_eq!(Source::read(&folder, &origin).name, "");
        Source::write(&folder, "Test server", "http://localhost:8000/plugins.json").unwrap();
        let s = Source::read(&folder, &origin);
        assert_eq!(
            (s.origin.as_str(), s.name.as_str(), s.url.as_str()),
            (
                origin.as_str(),
                "Test server",
                "http://localhost:8000/plugins.json"
            )
        );
        // It records which plugin each file is, kept when it is named again.
        let entry = |v: &str| Installed {
            entry: super::super::PluginRef {
                id: "posterize".into(),
                version: v.into(),
            },
            compatible_from: Some("1.0".into()),
            min_sdk: None,
        };
        Source::record(&folder, "posterize.py", entry("1.0"), None).unwrap();
        Source::record(
            &folder,
            "posterize-2.py",
            entry("2.0"),
            Some("posterize.py"),
        )
        .unwrap();
        Source::write(&folder, "Renamed", "http://localhost:8000/plugins.json").unwrap();
        let s = Source::read(&folder, &origin);
        assert_eq!(s.name, "Renamed");
        assert_eq!(
            s.plugins.into_iter().collect::<Vec<_>>(),
            [("posterize-2.py".to_owned(), entry("2.0"))]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_python_plugin_installed_for_an_sdk_no_longer_run_is_not_started() {
        let dir = std::env::temp_dir().join(format!("artwork-old-sdk-{}", std::process::id()));
        let folder = dir.join(origin("https://example.com/plugins.json"));
        std::fs::create_dir_all(&folder).unwrap();
        let entry = |id: &str, min_sdk: Option<&str>| Installed {
            entry: super::super::PluginRef {
                id: id.into(),
                version: "1.0.0".into(),
            },
            compatible_from: None,
            min_sdk: min_sdk.map(Into::into),
        };
        for (file, min_sdk) in [("old.py", Some("0.1.0")), ("fine.py", Some("0.3.0"))] {
            std::fs::write(folder.join(file), "plugin = None\n").unwrap();
            Source::record(&folder, file, entry(file, min_sdk), None).unwrap();
        }
        assert!(sdk_no_longer_run(&folder.join("old.py")).is_some_and(|e| e.contains("SDK 0.1.0")));
        assert_eq!(sdk_no_longer_run(&folder.join("fine.py")), None);
        // Without Python, only the one that never could run says so.
        let settings = Settings::read_from(Some(dir.join("plugins.json")));
        let loaded = load_dir_with(&dir, None, &settings);
        let status = |name: &str| {
            let f = loaded
                .files
                .iter()
                .find(|f| f.path.ends_with(name))
                .unwrap();
            f.status.clone()
        };
        assert!(matches!(status("old.py"), Status::Failed(e) if e.contains("update it")));
        assert!(matches!(status("fine.py"), Status::Failed(e) if e == python::PYTHON_MISSING));
        let _ = std::fs::remove_dir_all(dir);
    }
}
