//! Effects from plugin libraries (`.dll` on Windows, `.so` on Linux) found in the
//! `plugins` folder, through the C ABI of `artwork_plugin_sdk::abi`.
//!
//! Plugins run in the app's process with its full rights: a crash in one takes the app
//! down. They are loaded once at startup and never unloaded.

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use artwork_plugin_sdk::{self as sdk, abi};
use image::RgbaImage;
use libloading::Library;

use super::{Effect, ParamKind, ParamSpec, Params, color_from_hex};

/// A plugin that loaded.
#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    /// Ids of its effects.
    pub effects: Vec<String>,
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
        if r == 0 {
            return Ok(());
        }
        // Make sure the message ends within the buffer, whatever the plugin wrote.
        *error.last_mut().unwrap() = 0;
        // SAFETY: NUL-terminated within the buffer.
        let msg = unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy();
        Err(if msg.is_empty() {
            format!("failed (error {r})")
        } else {
            msg.into_owned()
        })
    }
}

/// The values passed to a plugin: every parameter, checked against its range.
fn values_json(params: &Params) -> CString {
    // JSON escapes control characters, so there is no NUL in it.
    CString::new(params.resolved().to_string()).unwrap_or_default()
}

/// Folder holding the plugins: next to the executable first, then the working directory.
pub fn plugins_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("plugins"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("plugins"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// What loading the plugins of a folder gave.
#[derive(Default)]
pub struct Loaded {
    pub effects: Vec<Box<dyn Effect>>,
    pub plugins: Vec<PluginInfo>,
    /// One message per library that failed to load.
    pub errors: Vec<String>,
}

/// Loads every library of this platform (`.dll` or `.so`) in `dir`, in name order.
pub fn load_dir(dir: &Path) -> Loaded {
    let mut loaded = Loaded::default();
    let Ok(read) = std::fs::read_dir(dir) else {
        return loaded;
    };
    let mut paths: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(std::env::consts::DLL_EXTENSION))
        })
        .collect();
    paths.sort();
    for path in paths {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        match load_library(&path) {
            Ok((info, effects)) => {
                log::info!(
                    "loaded plugin {} {} from {} ({} effects)",
                    info.name,
                    info.version,
                    path.display(),
                    effects.len()
                );
                loaded.plugins.push(info);
                loaded
                    .effects
                    .extend(effects.into_iter().map(|e| Box::new(e) as Box<dyn Effect>));
            }
            Err(e) => {
                log::warn!("plugin {} not loaded: {e}", path.display());
                loaded.errors.push(format!("{file}: {e}"));
            }
        }
    }
    loaded
}

/// Loads one plugin library.
pub fn load_library(path: &Path) -> Result<(PluginInfo, Vec<NativeEffect>), String> {
    // SAFETY: none can be given: loading runs the library's initializers, and its code
    // is trusted from here on. The README warns to install only trusted plugins.
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

/// Reads a plugin table.
///
/// # Safety
/// `table` is null or follows the ABI, and stays valid while `library` is loaded (or
/// forever when `library` is `None`).
pub unsafe fn from_table(
    table: *const abi::PluginTable,
    path: &Path,
    library: Option<Arc<Library>>,
) -> Result<(PluginInfo, Vec<NativeEffect>), String> {
    if table.is_null() {
        return Err("the plugin failed to start".into());
    }
    // The version is read first, on its own: the rest of the layout depends on it.
    // SAFETY: every ABI version starts with this field.
    let version = unsafe { std::ptr::addr_of!((*table).abi_version).read() };
    if version != abi::ABI_VERSION {
        return Err(format!(
            "made for plugin interface version {version}, this app uses version {}",
            abi::ABI_VERSION
        ));
    }
    // SAFETY: the version matches, so the layout is this one.
    let t = unsafe { &*table };
    let (name, plugin_version) = unsafe { (string(t.name), string(t.version)) };
    let name = if name.is_empty() {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    } else {
        name
    };
    let tables: &[abi::EffectTable] = if t.effect_count == 0 || t.effects.is_null() {
        &[]
    } else {
        // SAFETY: `effects` points to `effect_count` tables.
        unsafe { std::slice::from_raw_parts(t.effects, t.effect_count as usize) }
    };
    let mut effects = Vec::with_capacity(tables.len());
    for e in tables {
        // SAFETY: the strings are NUL-terminated or null.
        let (id, effect_name, description, params) = unsafe {
            (
                string(e.id),
                string(e.name),
                string(e.description),
                string(e.params_json),
            )
        };
        let err = |m: String| format!("effect \"{id}\": {m}");
        if id.is_empty() {
            return Err("an effect has no id".into());
        }
        if id.starts_with("builtin.") {
            return Err(err("ids starting with \"builtin.\" are reserved".into()));
        }
        if effects.iter().any(|x: &NativeEffect| x.id == id) {
            return Err(err("id used twice".into()));
        }
        let params = param_specs(&params).map_err(err)?;
        effects.push(NativeEffect {
            name: if effect_name.is_empty() {
                id.clone()
            } else {
                effect_name
            },
            id,
            description,
            params,
            plugin: name.clone(),
            table: *e,
            _library: library.clone(),
        });
    }
    let info = PluginInfo {
        name,
        version: plugin_version,
        path: path.to_owned(),
        effects: effects.iter().map(|e| e.id.clone()).collect(),
    };
    Ok((info, effects))
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
    use crate::effects::{EffectInstance, ParamValues};
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

    fn fill() -> NativeEffect {
        let table = sdk::build_table("Test", "1.0", || vec![Box::new(Fill)]).unwrap();
        let (info, mut effects) = unsafe { from_table(table, Path::new("test.so"), None) }.unwrap();
        assert_eq!((info.name.as_str(), info.version.as_str()), ("Test", "1.0"));
        assert_eq!(info.effects, ["test.fill"]);
        effects.pop().unwrap()
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

    /// Loads a real library, e.g. the halftone example built by `make plugins`:
    /// `ARTWORK_TEST_PLUGIN=plugins/libhalftone.so cargo test loads_plugin_library`.
    #[test]
    fn loads_plugin_library() {
        let Some(path) = std::env::var_os("ARTWORK_TEST_PLUGIN") else {
            return;
        };
        let (info, effects) = load_library(Path::new(&path)).unwrap();
        assert!(!effects.is_empty(), "{info:?}");
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
