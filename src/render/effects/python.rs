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

/// The user's Python 3, looked for once: on Windows the `py` launcher (also where a
/// per-user install puts it, which is not yet in the `PATH` of an app started by the
/// installer), then `python`; elsewhere `python3`, then `python`.
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
    if cfg!(windows) {
        list.push(python("py".into(), &["-3"]));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let launcher = Path::new(&local).join(r"Programs\Python\Launcher\py.exe");
            list.push(python(launcher, &["-3"]));
        }
        list.push(python("python".into(), &[]));
    } else {
        list.push(python("python3".into(), &[]));
        list.push(python("python".into(), &[]));
    }
    list
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
}

impl Script {
    fn start(&self) -> Result<Process, String> {
        // With -c, Python imports from the working directory first, which may be any
        // folder: a json.py there would run. The plugin's own folder is the working
        // directory, and is taken out of the path too.
        let boot = format!(
            "import sys\nif sys.path and sys.path[0] in ('', '.'):\n    del sys.path[0]\n{SDK}\nmain(sys.argv[1])\n"
        );
        let mut child = self
            .python
            .command()
            .current_dir(self.path.parent().unwrap_or(Path::new(".")))
            .arg("-c")
            .arg(boot)
            .arg(&self.path)
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
        path: path.to_owned(),
        effects: effects.iter().map(|e| e.id().to_owned()).collect(),
        generators: generators.iter().map(|g| g.id().to_owned()).collect(),
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

    const EXAMPLE: &str = include_str!("../../../plugin-examples/python/posterize.py");

    const TEST: &str = r#"
import os
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
    fn python_plugins_work_through_the_process() {
        let Some(_) = python() else {
            eprintln!("no Python 3: skipped");
            return;
        };
        let dir = folder(
            "load",
            &[
                ("posterize.py", EXAMPLE),
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
        assert_eq!(names, ["Python example", "test"]);

        let posterize = loaded
            .effects
            .iter()
            .find(|e| e.id() == "pyexample.posterize")
            .unwrap();
        assert_eq!(posterize.plugin(), Some("Python example"));
        assert!(!posterize.description().is_empty());
        let inst = EffectInstance::new(posterize.as_ref());
        let params = Params::new(posterize.params(), &inst.params);
        assert_eq!(posterize.padding(&params), [0.0; 4]);
        let mut img = RgbaImage::from_pixel(3, 2, image::Rgba([10, 200, 100, 7]));
        posterize.apply(&mut img, &params, 1.0).unwrap();
        assert_eq!(img.get_pixel(2, 1).0, [0, 170, 85, 7]);

        let bands = &loaded.generators[0];
        assert_eq!(
            (bands.id(), bands.name(), bands.plugin()),
            ("pyexample.bands", "Bands", Some("Python example"))
        );
        assert!(!bands.description().is_empty());
        let inst = EffectInstance::generator(bands.as_ref());
        let mut img = RgbaImage::new(4, 4);
        bands
            .generate(&mut img, &Params::new(bands.params(), &inst.params), 1.0)
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
        let swap = include_str!("../../../plugin-examples/python/swap_channels.py");
        let dir = folder("allow", &[("posterize.py", EXAMPLE), ("swap.py", swap)]);
        let dirs = [dir.canonicalize().unwrap()];
        let path = dirs[0].join("posterize.py");
        let other = dirs[0].join("swap.py");
        // Not allowed yet: held back, nothing run.
        let reg = Registry::with_plugins(&dirs, &Settings::default());
        assert_eq!(reg.pending().len(), 2);
        assert!(reg.get("pyexample.posterize").is_none());
        assert_eq!(reg.generation(), 0);
        assert!(reg.files().iter().all(|f| f.status == Status::Waiting));
        // One allowed: loaded at once, remembered with the file's content; the other
        // still waits.
        let file = dir.join("settings.json");
        let mut settings = Settings::read_from(Some(file.clone()));
        reg.allow(&path, &mut settings);
        assert_eq!(reg.generation(), 1);
        assert!(reg.get("pyexample.posterize").is_some());
        assert!(reg.generator("pyexample.bands").is_some());
        assert_eq!(reg.plugins().count(), 1);
        assert_eq!(reg.files()[0].name(), "Python example");
        assert_eq!(reg.files()[1].status, Status::Waiting);
        assert_eq!(reg.pending().len(), 1);
        assert!(reg.get("pyswap.swap").is_none());
        // Allowing it again, or a file not pending, changes nothing.
        reg.allow(&path, &mut settings);
        reg.allow(&dir.join("nope.py"), &mut settings);
        assert_eq!(reg.generation(), 1);
        let saved = Settings::read_from(Some(file));
        assert!(saved.is_allowed(&pending(&path).unwrap()));
        assert!(!saved.is_allowed(&pending(&other).unwrap()));
        let reg = Registry::with_plugins(&dirs, &saved);
        assert_eq!(reg.pending().len(), 1);
        assert!(reg.get("pyexample.posterize").is_some());
        // Changed: waits again.
        std::fs::write(&path, format!("{EXAMPLE}\n# changed\n")).unwrap();
        let reg = Registry::with_plugins(&dirs, &saved);
        assert_eq!(reg.pending().len(), 2);
        assert!(pending(&dir.join("missing.py")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn errors_of_the_plugin_are_reported() {
        let Some(py) = python() else {
            return;
        };
        let dir = folder("errors", &[("posterize.py", EXAMPLE)]);
        let (_, effects, _) = load_script(&dir.join("posterize.py"), py).unwrap();
        let mut img = RgbaImage::new(1, 1);
        // No such parameter: the plugin fails, and says so.
        let err = effects[0].apply(&mut img, &Params::new(&[], &Default::default()), 1.0);
        assert_eq!(err.unwrap_err(), "'levels'");
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
}
