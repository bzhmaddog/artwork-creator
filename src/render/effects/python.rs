//! Python plugins: `.py` files in the `plugins` folder, each run by the user's Python 3
//! in a process of its own, with the SDK module `artwork_plugin_sdk`
//! (`plugin-sdk/python/artwork_plugin_sdk.py`, built into the app), which also describes the
//! protocol: a line of JSON each way, with the pixels after it.
//!
//! The process stays running between calls, which wait their turn. If it stops (a crash,
//! a broken answer), the call fails and the next one starts it again: a plugin can't
//! take the app down.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use image::RgbaImage;
use serde_json::{Value, json};

use super::plugin::{PluginInfo, check};
use super::{Effect, Generator, ParamSpec, Params};

/// The SDK module, run with `python -c`; it then runs the plugin file given after it.
const SDK: &str = include_str!("../../../plugin-sdk/python/artwork_plugin_sdk.py");

/// Reported, once, when the plugins folder has `.py` files and no Python 3 was found.
pub const PYTHON_MISSING: &str = "Python plugins need Python 3, which was not found";

/// A Python 3 command: the program and the arguments before the script's.
#[derive(Clone, Debug)]
pub struct Python {
    program: PathBuf,
    args: Vec<String>,
}

impl Python {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window for Python next to the app's.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    /// Whether it runs Python 3.8 or later.
    fn works(&self) -> bool {
        self.command()
            .args(["-c", "import sys; sys.exit(sys.version_info < (3, 8))"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

/// The Python 3 to run plugins with, looked for once: first the one the packages ship
/// (see [`bundled`]); then on Windows the `py` launcher (also where a per-user install
/// puts it), then `python`; on Linux, only in debug builds (which come without one),
/// `python3`, then `python`.
pub fn python() -> Option<&'static Python> {
    static PYTHON: OnceLock<Option<Python>> = OnceLock::new();
    PYTHON
        .get_or_init(|| {
            let found = candidates().into_iter().find(Python::works);
            match &found {
                Some(p) => log::info!("Python plugins run with {}", p.program.display()),
                None => log::info!("no Python 3 found"),
            }
            found
        })
        .as_ref()
}

fn candidates() -> Vec<Python> {
    let python = |program: PathBuf, args: &[&str]| Python {
        program,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
    };
    let mut list = Vec::new();
    if let Some((dir, program)) = bundled() {
        // Isolated (-I): no user site-packages or PYTHON* variables. The embeddable
        // Python of Windows is, by its python._pth.
        let args: &[&str] = if cfg!(windows) { &[] } else { &["-I"] };
        list.push(python(dir.join(program), args));
    }
    if cfg!(windows) {
        list.push(python("py".into(), &["-3"]));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let launcher = Path::new(&local).join(r"Programs\Python\Launcher\py.exe");
            list.push(python(launcher, &["-3"]));
        }
        list.push(python("python".into(), &[]));
    } else if cfg!(debug_assertions) {
        list.push(python("python3".into(), &[]));
        list.push(python("python".into(), &[]));
    }
    list
}

/// The Python the packages ship, its folder and its program in it: on Windows
/// python.org's embeddable Python with pip's wheel (`pip.whl`), in `python` next to the
/// executable; on Linux python-build-standalone's, with pip, in `python` next to it (the
/// .tar.gz) or in `../lib/artwork-creator/python` (the .deb and the AppImage).
fn bundled() -> Option<(PathBuf, &'static str)> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.parent()?;
    if cfg!(windows) {
        return Some((exe.join("python"), "python.exe"));
    }
    let program = "bin/python3";
    [
        exe.join("python"),
        exe.join("../lib").join(crate::APP_ID).join("python"),
    ]
    .into_iter()
    .find(|d| d.join(program).is_file())
    .map(|d| (d, program))
}

/// Where the packages Python plugins need are installed, in the app's data folder (the
/// app's own `python` folder may be read-only): added to every plugin's path.
pub fn packages_dir() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("python-packages"))
}

/// The pip to install plugin packages with, when plugins run with the app's own Python:
/// pip's wheel on Windows; an empty path on Linux, where that Python has pip installed.
pub fn pip() -> Option<PathBuf> {
    let (dir, program) = bundled()?;
    if python()?.program != dir.join(program) {
        return None;
    }
    if !cfg!(windows) {
        return Some(PathBuf::new());
    }
    let wheel = dir.join("pip.whl");
    wheel.is_file().then_some(wheel)
}

/// The packages a Python plugin needs, from the `dependencies` of the PEP 723 block at
/// its top (`# /// script` ... `# ///`), as pip requirements (`numpy>=1.24`); none
/// without one. Only that list is read from the block: its strings, quoted.
pub fn dependencies(source: &str) -> Vec<String> {
    let mut block = String::new();
    let mut inside = false;
    for line in source.lines() {
        if !inside {
            inside = line.trim_end() == "# /// script";
        } else if line.trim_end() == "# ///" {
            break;
        } else if let Some(rest) = line.strip_prefix('#') {
            block.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            block.push('\n');
        } else {
            // Not a comment: no block after all.
            return Vec::new();
        }
    }
    let list = block.lines().position(|l| {
        l.split_once('=').is_some_and(|(key, value)| {
            key.trim() == "dependencies" && value.trim().starts_with('[')
        })
    });
    let Some(first) = list else {
        return Vec::new();
    };
    let text: String = block.lines().skip(first).collect::<Vec<_>>().join("\n");
    let text = &text[text.find('[').unwrap_or(0) + 1..];
    let mut found = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            ']' => break,
            '"' | '\'' => {
                let s: String = chars.by_ref().take_while(|&q| q != c).collect();
                if !s.trim().is_empty() {
                    found.push(s.trim().to_owned());
                }
            }
            _ => {}
        }
    }
    found
}

/// Installs from PyPI, into [`packages_dir`] with the app's pip ([`pip`]), those of `requirements`
/// not already there; pip's last line when it fails.
pub fn install_dependencies(requirements: &[String]) -> Result<(), String> {
    let (Some(python), Some(pip), Some(dir)) = (python(), pip(), packages_dir()) else {
        return Err("pip is not available".into());
    };
    pip_install(python, &pip, &dir, requirements)
}

/// Checks each requirement against what `dir` has (with pip's own `packaging`), and runs
/// pip for the missing ones only: a plugin allowed again downloads nothing.
const PIP_INSTALL: &str = r#"
import sys
pip, target, *reqs = sys.argv[1:]
if pip:
    sys.path.insert(0, pip)
sys.path.append(target)
from importlib.metadata import PackageNotFoundError, version
from pip._vendor.packaging.requirements import Requirement

def missing(text):
    r = Requirement(text)
    if r.marker and not r.marker.evaluate():
        return False
    try:
        return not r.specifier.contains(version(r.name), prereleases=True)
    except PackageNotFoundError:
        return True

need = [r for r in reqs if missing(r)]
if need:
    from pip._internal.cli.main import main
    sys.exit(main(["install", "--upgrade", "--disable-pip-version-check", "--no-input", "--target", target, "--", *need]))
"#;

fn pip_install(
    python: &Python,
    pip: &Path,
    dir: &Path,
    requirements: &[String],
) -> Result<(), String> {
    let out = python
        .command()
        .arg("-c")
        .arg(PIP_INSTALL)
        .arg(pip)
        .arg(dir)
        .args(requirements)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Python did not start: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    log::info!("pip install {}:\n{text}", requirements.join(" "));
    if out.status.success() {
        return Ok(());
    }
    let last = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("pip failed");
    Err(last.trim().to_owned())
}

/// A running plugin process.
struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One Python plugin file and its process, shared by its effects and generators.
struct Script {
    path: PathBuf,
    python: &'static Python,
    process: Mutex<Option<Process>>,
    /// Uninstalled: its process stopped, never started again.
    unloaded: AtomicBool,
}

impl Script {
    /// Stops its process for good.
    fn unload(&self) {
        self.unloaded.store(true, Ordering::Relaxed);
        *self.process.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn start(&self) -> Result<Process, String> {
        // With -c, Python imports from the working directory first, which may be any
        // folder: a json.py there would run. The plugin's own folder is the working
        // directory, and is taken out of the path too. The packages folder, when there is
        // one, comes after the plugin file and goes at the end of the path.
        let boot = format!(
            "import sys\nif sys.path and sys.path[0] in ('', '.'):\n    del sys.path[0]\nsys.path.extend(sys.argv[2:])\n{SDK}\nmain(sys.argv[1])\n"
        );
        let mut child = self
            .python
            .command()
            .current_dir(self.path.parent().unwrap_or(Path::new(".")))
            .arg("-c")
            .arg(boot)
            .arg(&self.path)
            .args(packages_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Python did not start: {e}"))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err("Python did not start".into());
        };
        // What the plugin prints, and Python's errors, go to the log.
        let file = self
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                log::warn!("{file}: {line}");
            }
        });
        Ok(Process {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    /// Sends `request`, with `pixels` after it, and returns the answer with the
    /// `answer_len` bytes of pixels that follow it when it succeeded.
    fn call(
        &self,
        request: &Value,
        pixels: &[u8],
        answer_len: usize,
    ) -> Result<(Value, Vec<u8>), String> {
        let mut process = self.process.lock().unwrap_or_else(|e| e.into_inner());
        if self.unloaded.load(Ordering::Relaxed) {
            return Err("the plugin was uninstalled".into());
        }
        if process.is_none() {
            *process = Some(self.start()?);
        }
        let Some(p) = process.as_mut() else {
            return Err("Python did not start".into());
        };
        match exchange(p, request, pixels, answer_len) {
            Ok(Ok(answer)) => Ok(answer),
            // An error the plugin reported: its process goes on.
            Ok(Err(e)) => Err(e),
            Err(e) => {
                // Stopped or out of step: the next call starts it again.
                *process = None;
                log::warn!("{}: {e}", self.path.display());
                Err(format!("the plugin's Python process stopped ({e})"))
            }
        }
    }
}

/// One request and its answer. The outer error is a broken process, the inner one an
/// error the plugin reported.
fn exchange(
    p: &mut Process,
    request: &Value,
    pixels: &[u8],
    answer_len: usize,
) -> Result<Result<(Value, Vec<u8>), String>, String> {
    let io = |e: std::io::Error| e.to_string();
    let mut line = request.to_string().into_bytes();
    line.push(b'\n');
    p.stdin.write_all(&line).map_err(io)?;
    p.stdin.write_all(pixels).map_err(io)?;
    p.stdin.flush().map_err(io)?;
    let mut answer = String::new();
    if p.stdout.read_line(&mut answer).map_err(io)? == 0 {
        return Err("no answer".into());
    }
    let answer: Value =
        serde_json::from_str(&answer).map_err(|e| format!("invalid answer: {e}"))?;
    if answer.get("ok") != Some(&Value::Bool(true)) {
        let error = answer
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("failed");
        return Ok(Err(error.to_owned()));
    }
    let mut data = vec![0; answer_len];
    p.stdout.read_exact(&mut data).map_err(io)?;
    Ok(Ok((answer, data)))
}

/// What a Python plugin file holds.
pub type Contents = (PluginInfo, Vec<Box<dyn Effect>>, Vec<Box<dyn Generator>>);

/// Starts the plugin file `path` with `python` and reads what it provides.
pub fn load_script(path: &Path, python: &'static Python) -> Result<Contents, String> {
    let script = Arc::new(Script {
        path: path.to_owned(),
        python,
        process: Mutex::new(None),
        unloaded: AtomicBool::new(false),
    });
    let (d, _) = script.call(&json!({"op": "describe"}), &[], 0)?;
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let name = match text(&d, "name") {
        n if n.is_empty() => path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        n => n,
    };
    let list = |key: &str| {
        d.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut ids: Vec<String> = Vec::new();
    let mut describe = |what: &str, v: &Value| {
        let params = v.get("params").cloned().unwrap_or(json!([]));
        let d = check(
            what,
            text(v, "id"),
            text(v, "name"),
            text(v, "description"),
            &params.to_string(),
            &ids,
        )?;
        ids.push(d.id.clone());
        Ok::<_, String>(Part {
            id: d.id,
            name: d.name,
            description: d.description,
            params: d.params,
            plugin: name.clone(),
            script: script.clone(),
        })
    };
    let mut effects: Vec<Box<dyn Effect>> = Vec::new();
    for e in list("effects") {
        effects.push(Box::new(PythonEffect {
            part: describe("effect", &e)?,
            padding: e.get("padding") == Some(&Value::Bool(true)),
        }));
    }
    let mut generators: Vec<Box<dyn Generator>> = Vec::new();
    for g in list("generators") {
        generators.push(Box::new(PythonGenerator(describe("generator", &g)?)));
    }
    let info = PluginInfo {
        name: name.clone(),
        version: text(&d, "version"),
        effects: effects.iter().map(|e| e.id().to_owned()).collect(),
        generators: generators.iter().map(|g| g.id().to_owned()).collect(),
        path: path.to_owned(),
        source: None,
        entry: None,
    };
    Ok((info, effects, generators))
}

/// What a Python effect and a Python generator have in common.
struct Part {
    id: String,
    name: String,
    description: String,
    params: Vec<ParamSpec>,
    plugin: String,
    script: Arc<Script>,
}

impl Part {
    /// Calls `op` on an image of `img`'s size, sending its pixels if `send`, and puts
    /// the answer's pixels in it.
    fn draw(
        &self,
        op: &str,
        img: &mut RgbaImage,
        params: &Params,
        extra: (&str, f32),
        send: bool,
    ) -> Result<(), String> {
        let (w, h) = img.dimensions();
        let mut request = json!({
            "op": op, "id": self.id, "values": params.resolved(), "width": w, "height": h,
        });
        request[extra.0] = json!(extra.1);
        let pixels: &[u8] = if send { img.as_raw() } else { &[] };
        let len = img.as_raw().len();
        let (_, data) = self.script.call(&request, pixels, len)?;
        img.copy_from_slice(&data);
        Ok(())
    }
}

struct PythonEffect {
    part: Part,
    /// Whether it draws outside the image.
    padding: bool,
}

impl Effect for PythonEffect {
    fn id(&self) -> &str {
        &self.part.id
    }

    fn name(&self) -> &str {
        &self.part.name
    }

    fn description(&self) -> &str {
        &self.part.description
    }

    fn params(&self) -> &[ParamSpec] {
        &self.part.params
    }

    fn plugin(&self) -> Option<&str> {
        Some(&self.part.plugin)
    }

    fn padding(&self, params: &Params) -> [f32; 4] {
        if !self.padding {
            return [0.0; 4];
        }
        let request = json!({"op": "padding", "id": self.part.id, "values": params.resolved()});
        let Ok((answer, _)) = self.part.script.call(&request, &[], 0) else {
            return [0.0; 4];
        };
        let mut out = [0.0f32; 4];
        if let Some(pad) = answer.get("padding").and_then(Value::as_array) {
            for (o, v) in out.iter_mut().zip(pad) {
                let v = v.as_f64().unwrap_or(0.0) as f32;
                *o = if v.is_finite() { v.max(0.0) } else { 0.0 };
            }
        }
        out
    }

    fn apply(&self, img: &mut RgbaImage, params: &Params, scale: f32) -> Result<(), String> {
        self.part.draw("apply", img, params, ("scale", scale), true)
    }

    fn unload(&self) {
        self.part.script.unload();
    }
}

struct PythonGenerator(Part);

impl Generator for PythonGenerator {
    fn id(&self) -> &str {
        &self.0.id
    }

    fn name(&self) -> &str {
        &self.0.name
    }

    fn description(&self) -> &str {
        &self.0.description
    }

    fn params(&self) -> &[ParamSpec] {
        &self.0.params
    }

    fn plugin(&self) -> Option<&str> {
        Some(&self.0.plugin)
    }

    fn generate(&self, img: &mut RgbaImage, params: &Params, px_per_mm: f32) -> Result<(), String> {
        self.0
            .draw("generate", img, params, ("px_per_mm", px_per_mm), false)
    }

    fn unload(&self) {
        self.0.script.unload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::effects::{
        EffectInstance, Registry,
        plugin::{Settings, Status, load_dir, pending},
    };

    /// Allows every plugin of `dir`, as they are now.
    fn allow_all(dir: &Path) -> Settings {
        let mut settings = Settings::default();
        settings.allow_all(dir);
        settings
    }

    /// A folder of its own for one test, with these plugin files.
    fn folder(test: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("artwork-python-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        dir
    }

    /// A plugin with an effect and a generator.
    const SAMPLE: &str = r#"
from artwork_plugin_sdk import Effect, Generator, Plugin, color, color_param, int_param

class Halve(Effect):
    id = "sample.halve"
    name = "Halve"
    description = "Divides the color channels"
    params = [int_param("divisor", "Divisor", 1, 8, 2)]

    def apply(self, image, values, scale):
        d = int(values["divisor"])
        p = image.pixels
        for i in range(len(p)):
            if i % 4 != 3:
                p[i] //= d

class Solid(Generator):
    id = "sample.solid"
    name = "Solid"
    description = "One color"
    params = [color_param("color", "Color", (29, 53, 87, 255))]

    def generate(self, image, values, px_per_mm):
        image.pixels[:] = bytes(color(values["color"])) * (image.width * image.height)

plugin = Plugin("Sample", "1.0", effects=[Halve()], generators=[Solid()])
"#;

    const TEST: &str = r#"
import os
import sys
assert sys.path[-1].endswith("python-packages"), sys.path
from artwork_plugin_sdk import Effect, Plugin, bool_param, float_param

class Grow(Effect):
    id = "test.grow"
    name = "Grow"
    params = [float_param("margin", "Margin", 0, 10, 2), bool_param("die", "Die", False)]

    def padding(self, values):
        return (values["margin"], 0, 1, float("nan"))

    def apply(self, image, values, scale):
        if values["die"]:
            os._exit(1)
        print("applied")
        image.pixels[:] = b"\x01\x02\x03\x04" * (image.width * image.height)

plugin = Plugin("", "3", effects=[Grow()])
"#;

    #[test]
    fn the_same_plugin_from_two_origins_is_two_plugins() {
        let Some(_) = python() else {
            eprintln!("no Python 3: skipped");
            return;
        };
        let dir = folder("origins", &[("test.py", TEST)]);
        let origins = [
            "https://example.com/plugins.json",
            "http://localhost:8000/plugins.json",
        ]
        .map(super::super::plugin::origin);
        let mut settings = Settings::default();
        for o in &origins {
            std::fs::create_dir_all(dir.join(o)).unwrap();
            std::fs::write(dir.join(o).join("test.py"), TEST).unwrap();
            settings.allow(&pending(&dir.join(o).join("test.py")).unwrap());
        }
        settings.allow(&pending(&dir.join("test.py")).unwrap());
        // Not an origin's folder: not looked into.
        std::fs::create_dir_all(dir.join("other")).unwrap();
        std::fs::write(dir.join("other").join("test.py"), TEST).unwrap();
        let loaded = load_dir(&dir, &settings);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let mut ids: Vec<_> = loaded.effects.iter().map(|e| e.id().to_owned()).collect();
        ids.sort();
        let mut expected = origins.map(|o| format!("{o}/test.grow")).to_vec();
        expected.push("test.grow".into());
        expected.sort();
        assert_eq!(ids, expected);
        let ids = |i: usize| loaded.plugins[i].effects.clone();
        assert_eq!(
            (ids(1), ids(2)),
            (vec![expected[0].clone()], vec![expected[1].clone()])
        );
        // Each runs.
        for e in &loaded.effects {
            let inst = EffectInstance::new(e.as_ref());
            let mut img = RgbaImage::new(1, 1);
            e.apply(&mut img, &Params::new(e.params(), &inst.params), 1.0)
                .unwrap();
            assert_eq!(img.get_pixel(0, 0).0, [1, 2, 3, 4]);
        }
        let err = check(
            "effect",
            "a/b".into(),
            String::new(),
            String::new(),
            "[]",
            &[],
        );
        assert!(err.is_err_and(|e| e.contains("\"/\"")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn python_plugins_work_through_the_process() {
        let Some(_) = python() else {
            eprintln!("no Python 3: skipped");
            return;
        };
        let dir = folder(
            "load",
            &[
                ("sample.py", SAMPLE),
                ("test.py", TEST),
                ("broken.py", "plugin = 1\n"),
                // Not imported by the others (as the json module), only loaded as a plugin.
                ("json.py", "raise SystemExit('hijacked')\n"),
                (
                    "reserved.py",
                    "from artwork_plugin_sdk import Effect, Plugin\nclass E(Effect):\n    id = 'builtin.x'\nplugin = Plugin('R', '1', effects=[E()])\n",
                ),
            ],
        );
        let loaded = load_dir(&dir, &allow_all(&dir));
        assert!(loaded.pending.is_empty());
        assert_eq!(loaded.errors.len(), 3, "{:?}", loaded.errors);
        assert!(loaded.errors.iter().any(|e| e.starts_with("json.py: ")));
        assert!(loaded.errors.iter().any(|e| e.starts_with("broken.py: ")));
        assert!(loaded.errors.iter().any(|e| e.contains("reserved")));
        let names: Vec<_> = loaded.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Sample", "test"]);

        let halve = loaded
            .effects
            .iter()
            .find(|e| e.id() == "sample.halve")
            .unwrap();
        assert_eq!(halve.plugin(), Some("Sample"));
        assert!(!halve.description().is_empty());
        let inst = EffectInstance::new(halve.as_ref());
        let params = Params::new(halve.params(), &inst.params);
        assert_eq!(halve.padding(&params), [0.0; 4]);
        let mut img = RgbaImage::from_pixel(3, 2, image::Rgba([10, 200, 100, 7]));
        halve.apply(&mut img, &params, 1.0).unwrap();
        assert_eq!(img.get_pixel(2, 1).0, [5, 100, 50, 7]);

        let solid = &loaded.generators[0];
        assert_eq!(
            (solid.id(), solid.name(), solid.plugin()),
            ("sample.solid", "Solid", Some("Sample"))
        );
        assert!(!solid.description().is_empty());
        let inst = EffectInstance::generator(solid.as_ref());
        let mut img = RgbaImage::new(4, 4);
        solid
            .generate(&mut img, &Params::new(solid.params(), &inst.params), 1.0)
            .unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [0x1d, 0x35, 0x57, 0xff]);

        let grow = loaded
            .effects
            .iter()
            .find(|e| e.id() == "test.grow")
            .unwrap();
        assert_eq!(grow.name(), "Grow");
        let mut inst = EffectInstance::new(grow.as_ref());
        assert_eq!(
            grow.padding(&Params::new(grow.params(), &inst.params)),
            [2.0, 0.0, 1.0, 0.0]
        );
        inst.params.insert("die".into(), Value::Bool(true));
        let mut img = RgbaImage::new(1, 1);
        let err = grow.apply(&mut img, &Params::new(grow.params(), &inst.params), 1.0);
        assert!(err.unwrap_err().contains("stopped"));
        // The next call starts the process again.
        inst.params.insert("die".into(), Value::Bool(false));
        grow.apply(&mut img, &Params::new(grow.params(), &inst.params), 1.0)
            .unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [1, 2, 3, 4]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn plugins_not_allowed_wait_to_be_allowed() {
        let Some(_) = python() else {
            return;
        };
        let dir = folder("allow", &[("sample.py", SAMPLE), ("test.py", TEST)]);
        let dirs = [dir.canonicalize().unwrap()];
        let path = dirs[0].join("sample.py");
        let other = dirs[0].join("test.py");
        // Not allowed yet: held back, nothing run.
        let reg = Registry::with_plugins(&dirs, &Settings::default());
        assert_eq!(reg.pending().len(), 2);
        assert!(reg.get("sample.halve").is_none());
        assert_eq!(reg.generation(), 0);
        assert!(reg.files().iter().all(|f| f.status == Status::Waiting));
        // One allowed: loaded at once, remembered with the file's content; the other
        // still waits.
        let file = dir.join("settings.json");
        let mut settings = Settings::read_from(Some(file.clone()));
        reg.allow(&path, &mut settings);
        assert_eq!(reg.generation(), 1);
        assert!(reg.get("sample.halve").is_some());
        assert!(reg.generator("sample.solid").is_some());
        assert_eq!(reg.plugins().count(), 1);
        assert_eq!(reg.files()[0].name(), "Sample");
        assert_eq!(reg.files()[1].status, Status::Waiting);
        assert_eq!(reg.pending().len(), 1);
        assert!(reg.get("test.grow").is_none());
        // Allowing it again, or a file not pending, changes nothing.
        reg.allow(&path, &mut settings);
        reg.allow(&dir.join("nope.py"), &mut settings);
        assert_eq!(reg.generation(), 1);
        let saved = Settings::read_from(Some(file));
        assert!(saved.is_allowed(&pending(&path).unwrap()));
        assert!(!saved.is_allowed(&pending(&other).unwrap()));
        let reg = Registry::with_plugins(&dirs, &saved);
        assert_eq!(reg.pending().len(), 1);
        assert!(reg.get("sample.halve").is_some());
        // Changed: waits again.
        std::fs::write(&path, format!("{SAMPLE}\n# changed\n")).unwrap();
        let reg = Registry::with_plugins(&dirs, &saved);
        assert_eq!(reg.pending().len(), 2);
        assert!(pending(&dir.join("missing.py")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_uninstalled_plugin_is_unloaded() {
        let Some(_) = python() else {
            return;
        };
        let dir = folder("unload", &[("sample.py", SAMPLE), ("test.py", TEST)]);
        let dirs = [dir.canonicalize().unwrap()];
        let path = dirs[0].join("sample.py");
        let reg = Registry::with_plugins(&dirs, &allow_all(&dirs[0]));
        let halve = reg.get("sample.halve").unwrap();
        reg.remove(&path);
        assert!(reg.get("sample.halve").is_none() && reg.generator("sample.solid").is_none());
        assert!(reg.get("test.grow").is_some());
        assert_eq!(reg.plugins().count(), 1);
        assert!(reg.files().iter().all(|f| f.path != path));
        assert_eq!(reg.generation(), 1);
        // Its process stopped, not started again.
        let inst = EffectInstance::new(halve);
        let mut img = RgbaImage::new(1, 1);
        let err = halve.apply(&mut img, &Params::new(halve.params(), &inst.params), 1.0);
        assert!(err.unwrap_err().contains("uninstalled"));
        // Removing it again, or a file not loaded, changes nothing.
        reg.remove(&path);
        reg.remove(&dirs[0].join("nope.py"));
        assert_eq!(reg.generation(), 1);
        // Installed again: loaded anew.
        assert!(reg.add_installed(&path));
        let halve = reg.get("sample.halve").unwrap();
        halve
            .apply(&mut img, &Params::new(halve.params(), &inst.params), 1.0)
            .unwrap();
        assert!(reg.files().iter().any(|f| f.path == path));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn errors_of_the_plugin_are_reported() {
        let Some(py) = python() else {
            return;
        };
        let dir = folder("errors", &[("sample.py", SAMPLE)]);
        let (_, effects, _) = load_script(&dir.join("sample.py"), py).unwrap();
        let mut img = RgbaImage::new(1, 1);
        // No such parameter: the plugin fails, and says so.
        let err = effects[0].apply(&mut img, &Params::new(&[], &Default::default()), 1.0);
        assert_eq!(err.unwrap_err(), "'divisor'");
        assert!(load_script(&dir.join("missing.py"), py).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_python_that_does_not_start_is_reported() {
        let missing: &'static Python = Box::leak(Box::new(Python {
            program: "no-such-python-here".into(),
            args: Vec::new(),
        }));
        assert!(!missing.works());
        assert!(
            load_script(Path::new("x.py"), missing)
                .err()
                .unwrap()
                .contains("did not start")
        );
        assert!(!candidates().is_empty());
    }

    #[test]
    fn a_failing_pip_reports_its_last_line() {
        let Some(python) = python() else {
            eprintln!("no Python 3: skipped");
            return;
        };
        // A pip that fails, put first on the path as the app's wheel is.
        let dir = folder("pip", &[]);
        std::fs::create_dir(dir.join("pip")).unwrap();
        let broken = "import sys\nprint('pip broke')\nsys.exit(1)\n";
        std::fs::write(dir.join("pip/__init__.py"), broken).unwrap();
        let six = ["six".to_owned()];
        let e = pip_install(python, &dir, &dir.join("packages"), &six).unwrap_err();
        assert_eq!(e, "pip broke");
        // Not the app's own Python: no pip to offer.
        assert!(pip().is_none());
        assert!(install_dependencies(&six).is_err());
    }

    #[test]
    fn dependencies_come_from_the_pep_723_block() {
        let source = r#"#!/usr/bin/env python3
# /// script
# requires-python = ">=3.8"
# dependencies = [
#   "numpy>=1.24",
#   'pillow',  # for PNG
# ]
# ///
import numpy
"#;
        assert_eq!(dependencies(source), ["numpy>=1.24", "pillow"]);
        assert_eq!(
            dependencies("# /// script\n# dependencies = [\"six\"]\n# ///\n"),
            ["six"]
        );
        // No block, another kind of block, or no list in it.
        assert!(dependencies("import os\n# dependencies = [\"six\"]\n").is_empty());
        assert!(dependencies("# /// pyproject\n# dependencies = [\"six\"]\n# ///\n").is_empty());
        assert!(dependencies("# /// script\n# requires-python = \">=3.8\"\n# ///\n").is_empty());
    }
}
