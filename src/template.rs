//! Controller templates: each subfolder of `templates/` holding a `template.svg` (or the
//! `.dxf` files of an older template).
//!
//! Templates come from two folders: the provided ones, shipped next to the executable and
//! never changed by the app (an update replaces them), and the user's, in the app's data
//! folder, which the user makes by duplicating a template and may change or delete.
//!
//! A template is made of elements (outline, buttons, cutouts), each a group (layer) of
//! its `template.svg`, named by its `id` in `template.json`. Older templates draw each
//! element in its own DXF file; the files share one coordinate system, as when exported
//! from the same drawing. Both are read; the template editor writes the SVG.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dxf::entities::EntityType;
use dxf::enums::Units;
use serde::{Deserialize, Serialize};

use crate::document::{CUSTOM_PAGE_RANGE_MM, Orientation, P2, PageSize};
use crate::i18n::{tr, tr_noop};

/// Name of the optional per-template configuration file.
pub const CONFIG_FILE: &str = "template.json";

/// What a template element is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElementKind {
    /// Edge of the panel: artwork outside it is not printed.
    Outline,
    /// Button holes.
    Buttons,
    /// Other holes: screws, switches, windows.
    Cutouts,
}

/// Number of element kinds, for per-kind settings arrays.
pub const KINDS: usize = ElementKind::ALL.len();

impl ElementKind {
    pub const ALL: [ElementKind; 3] = [
        ElementKind::Outline,
        ElementKind::Buttons,
        ElementKind::Cutouts,
    ];

    /// Position in `ALL`, for per-kind settings arrays.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Translation key of the name shown in the app: show it with `i18n::translate`.
    pub fn label(self) -> &'static str {
        match self {
            ElementKind::Outline => tr_noop!("ELEMENT_OUTLINE"),
            ElementKind::Buttons => tr_noop!("ELEMENT_BUTTONS"),
            ElementKind::Cutouts => tr_noop!("ELEMENT_CUTOUTS"),
        }
    }

    /// Line color used when `template.json` sets none.
    pub fn default_color(self) -> [u8; 3] {
        match self {
            ElementKind::Outline => [255, 0, 255],
            ElementKind::Buttons => [0, 170, 255],
            ElementKind::Cutouts => [255, 140, 0],
        }
    }
}

/// What a group of the template editor stands for: an element of the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementTag {
    pub kind: ElementKind,
    /// Its line color.
    pub color: [u8; 3],
}

/// Per-kind settings (`ElementKind::index`) as saved: a list longer than the kinds, from
/// a version that had more (anchors, last), keeps the first ones; a shorter one is
/// completed with `fill`.
pub fn per_kind<T: Copy>(saved: &[T], fill: [T; KINDS]) -> [T; KINDS] {
    std::array::from_fn(|i| saved.get(i).copied().unwrap_or(fill[i]))
}

/// The elements of `template.json`, without those of kinds no longer known ("anchors",
/// from an older version: element centers are snapped to now).
fn known_elements<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ElementConfig>, D::Error> {
    let values = Vec::<serde_json::Value>::deserialize(d)?;
    values
        .into_iter()
        .filter(|v| v.get("kind").and_then(|k| k.as_str()) != Some("anchors"))
        .map(|v| serde_json::from_value(v).map_err(serde::de::Error::custom))
        .collect()
}

/// One element in `template.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementConfig {
    /// What the element is for.
    pub kind: ElementKind,
    /// Its group's id in `template.svg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Its DXF file inside the template folder, in an older template (without `id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dxf: Option<String>,
    /// Display name. Defaults to the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Line color as "#rrggbb". Defaults to a color per kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Contents of `template.json`. Every field is optional. Written in the order of the
/// fields here, always the same: who it is, where it comes from, its page, its
/// drawing, then what the template editor keeps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateConfig {
    /// Display name, the model's when there is a vendor ("T16"). Defaults to the folder
    /// name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Who makes the controller ("Cosmox"): the templates are listed by vendor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Which version of the template this is ("1.2"), shown to tell copies apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The template this one was made from, for a copy. One lacking its name or version
    /// is ignored.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "complete_source"
    )]
    pub source: Option<TemplateSource>,
    /// The page the template is made for, set when it is picked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<PageConfig>,
    /// Drawing units when the DXF does not declare them: "mm", "cm", "inch".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// DXF file of a template without `elements`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dxf: Option<String>,
    /// The template's elements. When empty, the whole `template.svg` (or else a single
    /// DXF: `dxf`, or the first `.dxf` found) is the outline.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "known_elements"
    )]
    pub elements: Vec<ElementConfig>,
    /// Variables the shapes' sizes and positions are worked out from, in the template
    /// editor (see `variables`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<crate::variables::Variable>,
    /// The formulas of the shapes' linked fields, by the `id` of the shape in
    /// `template.svg`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub bindings: std::collections::BTreeMap<String, crate::variables::Bindings>,
    /// Where the top-left corner of `template.svg` was on the editor's page when
    /// `bindings` were written, in mm: their X and Y formulas are page positions, moved
    /// along when the template opens elsewhere on its page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bindings_origin: Option<P2>,
}

/// The template a copy was made from, in `template.json`: `{ "name": "T16", "vendor":
/// "Cosmox", "version": "1.1" }`. Its name and version are required, as every
/// template's are; its vendor may be left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateSource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    pub version: String,
}

impl TemplateSource {
    /// The template a copy of `entry` comes from: none unless it has a version (it has
    /// a name).
    pub fn of(entry: &TemplateEntry) -> Option<Self> {
        Self {
            name: entry.name.clone(),
            vendor: entry.vendor.clone(),
            version: entry.version.clone()?,
        }
        .complete()
    }

    /// The source if its name and version aren't blank; a blank vendor is none.
    fn complete(mut self) -> Option<Self> {
        if self.vendor.as_deref().is_some_and(|v| v.trim().is_empty()) {
            self.vendor = None;
        }
        (!self.name.trim().is_empty() && !self.version.trim().is_empty()).then_some(self)
    }

    /// As shown: "vendor - name - vversion", or "name - vversion" without a vendor.
    pub fn describe(&self) -> String {
        match &self.vendor {
            Some(v) => format!("{v} - {} - v{}", self.name, self.version),
            None => format!("{} - v{}", self.name, self.version),
        }
    }
}

/// A `source` with its name and version; anything else (one missing, blank or not a
/// text) is ignored rather than making `template.json` unreadable.
fn complete_source<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<TemplateSource>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value
        .and_then(|v| serde_json::from_value::<TemplateSource>(v).ok())
        .and_then(TemplateSource::complete))
}

/// The version of a new template, or of a copy of one without a valid version.
pub const FIRST_VERSION: &str = "1.0";

/// Whether `version` follows the "x.y" convention templates use: two whole numbers
/// ("1.0", "2.13").
pub fn is_valid_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let number =
        |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    number(parts.next()) && number(parts.next()) && parts.next().is_none()
}

/// The page a template is made for, in `template.json`: `{ "size": "A4", "orientation":
/// "landscape" }`, or a size in mm, `{ "size": [250, 400] }`. Either may be left out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageConfig {
    /// A paper's name, or its width and height in mm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<PageSizeConfig>,
    /// "landscape" or "portrait".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
}

/// A page size in `template.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PageSizeConfig {
    /// "A5", "A4", "A3", "A2", "Letter", "Legal" or "Tabloid", in any case.
    Named(String),
    /// Width and height in mm.
    Mm([f32; 2]),
}

/// The page setup a template asks for: what it leaves out stays as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageSetup {
    pub size: Option<PageSize>,
    /// For a custom size: width and height in portrait orientation, in mm.
    pub custom_mm: Option<P2>,
    pub orientation: Option<Orientation>,
}

impl PageConfig {
    /// The setup it asks for, or the value it doesn't understand.
    pub fn setup(&self) -> Result<PageSetup, String> {
        let mut setup = PageSetup::default();
        match &self.size {
            Some(PageSizeConfig::Named(name)) => {
                setup.size = Some(
                    PageSize::ALL
                        .into_iter()
                        .filter(|&p| p != PageSize::Custom)
                        .find(|p| format!("{p:?}").eq_ignore_ascii_case(name.trim()))
                        .ok_or_else(|| name.clone())?,
                );
            }
            Some(PageSizeConfig::Mm([w, h])) => {
                if !CUSTOM_PAGE_RANGE_MM.contains(w) || !CUSTOM_PAGE_RANGE_MM.contains(h) {
                    return Err(format!("[{w}, {h}]"));
                }
                setup.size = Some(PageSize::Custom);
                setup.custom_mm = Some([w.min(*h), w.max(*h)]);
                // As given, unless the orientation says otherwise.
                setup.orientation = Some(if w > h {
                    Orientation::Landscape
                } else {
                    Orientation::Portrait
                });
            }
            None => {}
        }
        match self.orientation.as_deref().map(|o| o.trim().to_lowercase()) {
            Some(o) if o == "landscape" => setup.orientation = Some(Orientation::Landscape),
            Some(o) if o == "portrait" => setup.orientation = Some(Orientation::Portrait),
            Some(o) => return Err(o),
            None => {}
        }
        Ok(setup)
    }
}

/// Where an element is drawn, resolved from the config.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementSource {
    /// What the element is for.
    pub kind: ElementKind,
    /// Shown in the app.
    pub name: String,
    /// The file: the template's SVG, or a DXF file.
    pub path: PathBuf,
    /// Its group's id in the SVG; `None` for all of it (or for a DXF file).
    pub group: Option<String>,
    /// Color from `template.json`, if set.
    pub color: Option<[u8; 3]>,
}

impl ElementSource {
    /// It is drawn in an SVG file (else a DXF file).
    pub fn is_svg(&self) -> bool {
        self.path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("svg"))
    }
}

/// Which folder a template comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    /// Shipped with the app: read only.
    #[default]
    Provided,
    /// In the user's templates folder: can be changed and deleted.
    User,
}

/// An installed template folder, read but not loaded yet.
#[derive(Clone, Debug)]
pub struct TemplateEntry {
    /// Folder name; this is what projects store.
    pub id: String,
    /// Which folder it comes from.
    pub origin: Origin,
    /// Shown to the user: the model's name when there is a vendor.
    pub name: String,
    /// Who makes the controller, if `template.json` says.
    pub vendor: Option<String>,
    /// Its version, if `template.json` says.
    pub version: Option<String>,
    /// The page it is made for, from `template.json`.
    pub page: PageSetup,
    /// The template's folder.
    pub folder: PathBuf,
    /// Where its elements are drawn.
    pub sources: Vec<ElementSource>,
    /// Parsed `template.json`; defaults if missing or invalid.
    pub config: TemplateConfig,
    /// Error from reading `template.json`, if any.
    pub config_error: Option<String>,
    /// What the template diagnostic found wrong with it (`template_check`), filled when
    /// the templates are listed (`scan_all`).
    pub problems: Vec<crate::template_check::Problem>,
}

impl TemplateEntry {
    /// Its vendor and name ("Cosmox - T16"), or its name alone.
    pub fn full_name(&self) -> String {
        match &self.vendor {
            Some(v) => format!("{v} - {}", self.name),
            None => self.name.clone(),
        }
    }

    /// Path of the folder's `template.json`.
    pub fn config_path(&self) -> PathBuf {
        self.folder.join(CONFIG_FILE)
    }

    /// Modification times of the config and the drawing files, used to reload on change.
    pub fn stamp(&self) -> Vec<Option<SystemTime>> {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        std::iter::once(self.config_path())
            .chain(self.sources.iter().map(|s| s.path.clone()))
            .map(|p| mtime(&p))
            .collect()
    }

    /// Deletes its folder. Only a user template can be deleted.
    pub fn delete(&self) -> Result<(), String> {
        if self.origin != Origin::User {
            return Err(tr!("PROVIDED_TEMPLATE_READ_ONLY", name = self.full_name()));
        }
        std::fs::remove_dir_all(&self.folder)
            .map_err(|e| tr!("CANNOT_DELETE_FOLDER", path = self.folder.display(), e = e))
    }

    /// Copies it into a new folder of `dir` (the user's templates folder), named after
    /// `name` and `vendor`, which its `template.json` takes. `taken` are the folder names
    /// in use, in any templates folder: the new one is none of them, so it hides no
    /// template. Returns the new folder.
    pub fn duplicate(
        &self,
        dir: &Path,
        name: &str,
        vendor: Option<&str>,
        taken: &[String],
    ) -> Result<PathBuf, String> {
        let files = TemplateFiles::read(self)?;
        let base = match vendor {
            Some(v) => format!("{v} - {name}"),
            None => name.to_owned(),
        };
        let id = unique_folder_name(&base, taken, dir);
        let folder = dir.join(&id);
        let cannot =
            |path: &Path, e: std::io::Error| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e);
        std::fs::create_dir_all(&folder).map_err(|e| cannot(&folder, e))?;
        let written = (|| {
            for (rel, data) in &files.0 {
                if rel == CONFIG_FILE {
                    continue;
                }
                let path = folder.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| cannot(parent, e))?;
                }
                std::fs::write(&path, data.as_slice()).map_err(|e| cannot(&path, e))?;
            }
            let mut config = self.config.clone();
            config.name = Some(name.to_owned());
            config.vendor = vendor.map(str::to_owned);
            // A folder without `template.json` took its first DXF file: name it, so the
            // copy keeps the same one.
            if config.elements.is_empty() && config.dxf.is_none() {
                config.dxf = self
                    .sources
                    .first()
                    .filter(|s| !s.is_svg())
                    .and_then(|s| s.path.file_name())
                    .map(|f| f.to_string_lossy().into_owned());
            }
            write_config(&folder, &config)
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(e);
        }
        Ok(folder)
    }
}

/// Writes `config` as the `template.json` of `folder`.
fn write_config(folder: &Path, config: &TemplateConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let path = folder.join(CONFIG_FILE);
    std::fs::write(&path, json + "\n")
        .map_err(|e| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e))
}

/// A folder name made from `name` that is none of `taken` and not in `dir` yet: the
/// characters Windows refuses replaced, then " (2)", " (3)"... added when needed.
pub fn unique_folder_name(name: &str, taken: &[String], dir: &Path) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Windows also refuses names ending in a dot or a space.
    let clean = clean.trim().trim_end_matches('.').trim();
    let base = if clean.is_empty() { "template" } else { clean };
    let free = |n: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(n)) && !dir.join(n).exists();
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|n| free(n))
        .expect("some number is free")
}

/// A template's files, as a project keeps them: `template.json` and the SVG or DXF files
/// it uses, each by its path in the template's folder ("/"-separated).
#[derive(Clone, Debug, Default)]
pub struct TemplateFiles(pub Vec<(String, std::sync::Arc<Vec<u8>>)>);

impl TemplateFiles {
    /// The files of an installed template.
    pub fn read(entry: &TemplateEntry) -> Result<Self, String> {
        let config = entry.folder.join(CONFIG_FILE);
        let paths = config
            .is_file()
            .then_some(config)
            .into_iter()
            .chain(entry.sources.iter().map(|s| s.path.clone()));
        let mut files: Vec<(String, std::sync::Arc<Vec<u8>>)> = Vec::new();
        for path in paths {
            let name = path
                .strip_prefix(&entry.folder)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if files.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let data = std::fs::read(&path)
                .map_err(|e| tr!("CANNOT_READ_FILE", path = path.display(), e = e))?;
            files.push((name, std::sync::Arc::new(data)));
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Self(files))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A fingerprint of the files' names and contents: the same for the same files, on
    /// any computer and in any version of the app (64-bit FNV-1a, in hex).
    pub fn fingerprint(&self) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        let mut files: Vec<_> = self.0.iter().collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, data) in files {
            eat(name.as_bytes());
            eat(&[0]);
            eat(&(data.len() as u64).to_le_bytes());
            eat(data);
        }
        format!("{h:016x}")
    }

    /// Writes the files into a folder named `id` of their own (under the system's
    /// temporary folder, once for each fingerprint) and returns it, to be read as an
    /// installed template is. Names that would lead out of the folder are refused.
    pub fn unpack(&self, id: &str) -> Result<PathBuf, String> {
        let normal = |name: &str| {
            let p = Path::new(name);
            (!name.is_empty()
                && p.components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))))
            .then(|| p.to_path_buf())
        };
        let id = normal(id)
            .filter(|p| p.components().count() == 1)
            .unwrap_or_else(|| PathBuf::from("template"));
        let dir = std::env::temp_dir()
            .join("artwork-creator-templates")
            .join(self.fingerprint())
            .join(id);
        let done = dir.join(".complete");
        if done.is_file() {
            return Ok(dir);
        }
        let cannot =
            |path: &Path, e: std::io::Error| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e);
        for (name, data) in &self.0 {
            let Some(rel) = normal(name) else {
                return Err(tr!("TEMPLATE_FILE_NAME_REFUSED", name = name));
            };
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| cannot(parent, e))?;
            }
            std::fs::write(&path, data.as_slice()).map_err(|e| cannot(&path, e))?;
        }
        std::fs::write(&done, b"").map_err(|e| cannot(&done, e))?;
        Ok(dir)
    }
}

/// Parses "#rrggbb" (the # is optional).
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Reads one template folder. Returns `None` if it lists no elements and holds no
/// `template.svg` nor DXF file.
pub fn read_entry(folder: &Path) -> Option<TemplateEntry> {
    let id = folder.file_name()?.to_string_lossy().into_owned();
    let (config, mut config_error) = match std::fs::read_to_string(folder.join(CONFIG_FILE)) {
        Ok(text) => match serde_json::from_str::<TemplateConfig>(&text) {
            Ok(c) => (c, None),
            Err(e) => (
                TemplateConfig::default(),
                Some(format!("{}: {e}", folder.join(CONFIG_FILE).display())),
            ),
        },
        Err(_) => (TemplateConfig::default(), None),
    };
    let mut sources = Vec::new();
    for e in &config.elements {
        let color = e.color.as_deref().and_then(|c| {
            let parsed = parse_color(c);
            if parsed.is_none() {
                config_error.get_or_insert(tr!(
                    "TEMPLATE_INVALID_COLOR",
                    path = folder.join(CONFIG_FILE).display(),
                    c = c
                ));
            }
            parsed
        });
        let (path, group) = match (&e.id, &e.dxf) {
            (Some(id), _) => (folder.join(crate::template_svg::SVG_FILE), Some(id.clone())),
            (None, Some(dxf)) => (folder.join(dxf), None),
            (None, None) => {
                config_error.get_or_insert(tr!(
                    "TEMPLATE_ELEMENT_WITHOUT_DRAWING",
                    path = folder.join(CONFIG_FILE).display()
                ));
                continue;
            }
        };
        sources.push(ElementSource {
            kind: e.kind,
            name: e.name.clone().unwrap_or_else(|| e.kind.label().to_owned()),
            path,
            group,
            color,
        });
    }
    let svg = folder.join(crate::template_svg::SVG_FILE);
    if sources.is_empty() && config.dxf.is_none() && svg.is_file() {
        sources.push(ElementSource {
            kind: ElementKind::Outline,
            name: ElementKind::Outline.label().to_owned(),
            path: svg,
            group: None,
            color: None,
        });
    }
    if sources.is_empty() {
        let path = match &config.dxf {
            Some(name) => folder.join(name),
            None => {
                let mut dxfs: Vec<PathBuf> = std::fs::read_dir(folder)
                    .ok()?
                    .flatten()
                    .map(|f| f.path())
                    .filter(|p| {
                        p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dxf"))
                    })
                    .collect();
                dxfs.sort();
                dxfs.into_iter().next()?
            }
        };
        sources.push(ElementSource {
            kind: ElementKind::Outline,
            name: ElementKind::Outline.label().to_owned(),
            path,
            group: None,
            color: None,
        });
    }
    let page = match config.page.as_ref().map(PageConfig::setup) {
        Some(Ok(setup)) => setup,
        Some(Err(value)) => {
            let e = tr!("TEMPLATE_PAGE_NOT_UNDERSTOOD", value = value);
            config_error.get_or_insert(format!("{}: {e}", folder.join(CONFIG_FILE).display()));
            PageSetup::default()
        }
        None => PageSetup::default(),
    };
    Some(TemplateEntry {
        page,
        name: config.name.clone().unwrap_or_else(|| id.clone()),
        version: config
            .version
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned),
        vendor: config
            .vendor
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned),
        id,
        origin: Origin::Provided,
        folder: folder.to_path_buf(),
        sources,
        config,
        config_error,
        problems: Vec::new(),
    })
}

/// A path of points. Arcs and circles are already broken into short segments.
#[derive(Clone, Debug)]
pub struct Polyline {
    /// Points in mm.
    pub points: Vec<P2>,
    /// The last point connects back to the first.
    pub closed: bool,
}

/// One loaded element of a template.
#[derive(Clone, Debug)]
pub struct TemplateElement {
    /// What the element is for.
    pub kind: ElementKind,
    /// Shown in the app.
    pub name: String,
    /// Color from `template.json`, if set.
    pub color: Option<[u8; 3]>,
    /// Its shapes in template mm.
    pub polylines: Vec<Polyline>,
    /// The center of each of its closed shapes (of its box: a circle's center), in
    /// template mm: images snap to them.
    pub centers: Vec<P2>,
}

/// Template geometry in mm, origin at the top-left of the bounding box of all its
/// elements, y down.
#[derive(Clone, Debug)]
pub struct Template {
    /// In `template.json` order.
    pub elements: Vec<TemplateElement>,
    /// Width and height of all elements together.
    pub size_mm: P2,
}

impl Template {
    /// The template has at least one element of this kind.
    pub fn has(&self, kind: ElementKind) -> bool {
        self.elements.iter().any(|e| e.kind == kind)
    }
}

/// Finds the templates folder: next to the executable first, then the working directory.
pub fn templates_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("templates"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("templates"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// The user's templates folder, in the app's data folder; it may not exist yet.
pub fn user_templates_dir() -> Option<PathBuf> {
    eframe::storage_dir("Artwork Creator").map(|d| d.join("templates"))
}

/// Lists every subfolder that is a template, sorted by display name.
#[cfg_attr(not(test), allow(dead_code))] // Tests read one folder of templates.
pub fn scan(dir: &Path) -> Vec<TemplateEntry> {
    scan_all(Some(dir), None)
}

/// Lists the templates of both folders, sorted by display name, each checked
/// (`TemplateEntry::problems`). A user template hides a provided one of the same folder
/// name.
pub fn scan_all(provided: Option<&Path>, user: Option<&Path>) -> Vec<TemplateEntry> {
    let read = |dir: &Path, origin: Origin| -> Vec<TemplateEntry> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        read.flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|p| read_entry(&p))
            .map(|e| TemplateEntry {
                origin,
                problems: crate::template_check::check_entry(&e),
                ..e
            })
            .collect()
    };
    let users = user.map(|d| read(d, Origin::User)).unwrap_or_default();
    let mut out: Vec<TemplateEntry> = provided
        .map(|d| read(d, Origin::Provided))
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !users.iter().any(|u| u.id == p.id))
        .collect();
    out.extend(users);
    // Those without a vendor first, then by vendor; by name within.
    out.sort_by_key(|t| {
        (
            t.vendor.as_ref().map(|v| v.to_lowercase()),
            t.name.to_lowercase(),
        )
    });
    out
}

/// Millimeters per unit for the `units` setting of `template.json`.
pub(crate) fn units_override(name: &str) -> Option<f64> {
    match name.trim().to_lowercase().as_str() {
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => Some(1.0),
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => Some(10.0),
        "in" | "inch" | "inches" => Some(25.4),
        _ => None,
    }
}

/// Millimeters per drawing unit of a DXF file, as its header declares them.
pub fn mm_per_unit(drawing: &dxf::Drawing) -> f64 {
    units_to_mm(drawing.header.default_drawing_units)
}

/// Millimeters per drawing unit declared in a DXF header.
fn units_to_mm(units: Units) -> f64 {
    match units {
        Units::Millimeters => 1.0,
        Units::Centimeters => 10.0,
        Units::Decimeters => 100.0,
        Units::Meters => 1000.0,
        Units::Feet | Units::USSurveyFeet => 304.8,
        Units::Mils => 0.0254,
        Units::Microns => 0.001,
        // Unitless drawings are treated as inches: that is what controller
        // templates like the Cosmox T16 use.
        _ => 25.4,
    }
}

/// Appends the points from `a` to `b`, following an arc when `bulge` is non-zero.
/// `a` itself is not pushed.
fn push_bulge_segment(out: &mut Vec<[f64; 2]>, a: [f64; 2], b: [f64; 2], bulge: f64) {
    if bulge.abs() < 1e-9 {
        out.push(b);
        return;
    }
    let theta = 4.0 * bulge.atan();
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let chord = (dx * dx + dy * dy).sqrt();
    if chord < 1e-12 {
        out.push(b);
        return;
    }
    let radius = chord / (2.0 * (theta / 2.0).sin());
    let offset = (chord / 2.0) / (theta / 2.0).tan();
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let center = [mid[0] - dy / chord * offset, mid[1] + dx / chord * offset];
    let start = (a[1] - center[1]).atan2(a[0] - center[0]);
    let steps = ((theta.abs() / 5f64.to_radians()).ceil() as usize).max(2);
    let r = radius.abs();
    for i in 1..=steps {
        let t = start + theta * i as f64 / steps as f64;
        out.push([center[0] + r * t.cos(), center[1] + r * t.sin()]);
    }
}

/// Points along an arc, counter-clockwise from `start_deg` to `end_deg`, at most 5° apart.
fn arc_points(center: [f64; 2], r: f64, start_deg: f64, mut end_deg: f64) -> Vec<[f64; 2]> {
    if end_deg <= start_deg {
        end_deg += 360.0;
    }
    let sweep = end_deg - start_deg;
    let steps = ((sweep / 5.0).ceil() as usize).max(2);
    (0..=steps)
        .map(|i| {
            let t = (start_deg + sweep * i as f64 / steps as f64).to_radians();
            [center[0] + r * t.cos(), center[1] + r * t.sin()]
        })
        .collect()
}

/// Follows a vertex list with bulges; closes back to the first vertex if `closed`.
fn bulged_path(verts: &[([f64; 2], f64)], closed: bool) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    let Some(first) = verts.first() else {
        return out;
    };
    out.push(first.0);
    for w in verts.windows(2) {
        push_bulge_segment(&mut out, w[0].0, w[1].0, w[0].1);
    }
    if closed && verts.len() > 2 {
        let last = verts[verts.len() - 1];
        if last.1.abs() > 1e-9 {
            push_bulge_segment(&mut out, last.0, first.0, last.1);
            out.pop(); // the closing point duplicates the first one
        }
    }
    out
}

/// Points of a path and whether it is closed.
pub(crate) type RawPath = (Vec<[f64; 2]>, bool);

/// The outlines of items (mm, y up) as paths of points, their curves cut in short lines.
/// Points of an outline closer than this (mm) are the same point.
const SAME_POINT_MM: f64 = 1e-4;

pub(crate) fn item_paths(items: &[crate::template_geom::Item]) -> Vec<RawPath> {
    use crate::template_geom::{Item, Seg};
    let mut out = Vec::new();
    for item in items {
        match item {
            Item::Circle { c, r } => {
                let mut pts = arc_points(*c, *r, 0.0, 360.0);
                pts.pop();
                out.push((pts, true));
            }
            Item::Outline { segs, closed } => {
                let Some(first) = segs.first() else { continue };
                let mut pts = vec![first.start()];
                for s in segs {
                    match *s {
                        Seg::Line(_, b) => pts.push(b),
                        Seg::Arc { c, r, a0, sweep } => {
                            let steps = ((sweep.abs() / 5f64.to_radians()).ceil() as usize).max(2);
                            pts.extend((1..=steps).map(|i| {
                                let t = a0 + sweep * i as f64 / steps as f64;
                                [c[0] + r * t.cos(), c[1] + r * t.sin()]
                            }));
                        }
                        Seg::Cubic([p0, p1, p2, p3]) => {
                            pts.extend((1..=16).map(|i| {
                                let t = i as f64 / 16.0;
                                let u = 1.0 - t;
                                let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                                [
                                    w[0] * p0[0] + w[1] * p1[0] + w[2] * p2[0] + w[3] * p3[0],
                                    w[0] * p0[1] + w[1] * p1[1] + w[2] * p2[1] + w[3] * p3[1],
                                ]
                            }));
                        }
                    }
                }
                // No point twice in a row (an arc's end computed where the next piece
                // starts), and a closed ring ends where it starts: once is enough. A
                // segment of no length has no direction: drawn, its join juts out.
                let same = |a: [f64; 2], b: [f64; 2]| {
                    (a[0] - b[0]).abs() < SAME_POINT_MM && (a[1] - b[1]).abs() < SAME_POINT_MM
                };
                pts.dedup_by(|b, a| same(*a, *b));
                while *closed && pts.len() > 2 && same(pts[0], pts[pts.len() - 1]) {
                    pts.pop();
                }
                if pts.len() >= 2 {
                    out.push((pts, *closed));
                }
            }
            Item::Point(_) => {}
        }
    }
    out
}

/// Paths and POINT entities of one DXF file, in mm, y up.
fn read_dxf(path: &Path, units: Option<f64>) -> Result<Vec<RawPath>, String> {
    let drawing = dxf::Drawing::load_file(path)
        .map_err(|e| tr!("CANNOT_READ_FILE", path = path.display(), e = e))?;

    // Raw paths in drawing units, y up.
    let mut raw: Vec<RawPath> = Vec::new();
    for entity in drawing.entities() {
        match &entity.specific {
            EntityType::Polyline(pl) => {
                // Skip 3D meshes and polyface meshes.
                if pl.flags & (16 | 64) != 0 {
                    continue;
                }
                let verts: Vec<_> = pl
                    .vertices()
                    .map(|v| ([v.location.x, v.location.y], v.bulge))
                    .collect();
                let closed = pl.is_closed();
                raw.push((bulged_path(&verts, closed), closed));
            }
            EntityType::LwPolyline(pl) => {
                let verts: Vec<_> = pl.vertices.iter().map(|v| ([v.x, v.y], v.bulge)).collect();
                let closed = pl.is_closed();
                raw.push((bulged_path(&verts, closed), closed));
            }
            EntityType::Line(l) => raw.push((vec![[l.p1.x, l.p1.y], [l.p2.x, l.p2.y]], false)),
            EntityType::Circle(c) => {
                let mut pts = arc_points([c.center.x, c.center.y], c.radius, 0.0, 360.0);
                pts.pop();
                raw.push((pts, true));
            }
            EntityType::Spline(s) => {
                let ctrl: Vec<[f64; 2]> = s.control_points.iter().map(|p| [p.x, p.y]).collect();
                let degree = s.degree_of_curve.max(1) as usize;
                let pts = crate::template_geom::spline_points(degree, &s.knot_values, &ctrl);
                raw.push((pts, s.is_closed()));
            }
            EntityType::Arc(a) => raw.push((
                arc_points(
                    [a.center.x, a.center.y],
                    a.radius,
                    a.start_angle,
                    a.end_angle,
                ),
                false,
            )),
            _ => {}
        }
    }
    raw.retain(|(pts, _)| pts.len() >= 2);
    let k = units.unwrap_or_else(|| mm_per_unit(&drawing));
    for p in raw.iter_mut().flat_map(|(pts, _)| pts) {
        *p = [p[0] * k, p[1] * k];
    }
    Ok(raw)
}

/// The center of the bounding box of each closed shape (a circle's center). Open lines
/// give none.
pub(crate) fn centers(paths: &[RawPath]) -> Vec<[f64; 2]> {
    paths
        .iter()
        .filter(|(_, closed)| *closed)
        .map(|(pts, _)| {
            let (min, max) = pts
                .iter()
                .fold(([f64::MAX; 2], [f64::MIN; 2]), |(mn, mx), p| {
                    (
                        [mn[0].min(p[0]), mn[1].min(p[1])],
                        [mx[0].max(p[0]), mx[1].max(p[1])],
                    )
                });
            [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]
        })
        .collect()
}

/// Endpoints closer than this (mm) are treated as the same point.
const JOIN_TOLERANCE_MM: f64 = 0.01;

/// Leaves out the paths drawn twice in a file: the same points, or for a closed path
/// the same ring started elsewhere or run the other way. A shape copied onto itself in
/// CAD would otherwise be printed twice, and cut twice by a plotter following the PDF.
fn drop_duplicates(paths: Vec<RawPath>) -> Vec<RawPath> {
    // The points as a closed ring is compared: from its least point, in the direction
    // that comes first.
    let key = |(pts, closed): &RawPath| -> (Vec<[u64; 2]>, bool) {
        let bits = |p: &[f64; 2]| p.map(f64::to_bits);
        let mut pts: Vec<[f64; 2]> = pts.clone();
        if *closed && pts.len() > 1 {
            // A ring repeating its first point at the end: once is enough.
            if pts.first() == pts.last() {
                pts.pop();
            }
            let least = (0..pts.len())
                .min_by(|&a, &b| {
                    pts[a]
                        .partial_cmp(&pts[b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(0);
            pts.rotate_left(least);
            let mut back = pts.clone();
            back[1..].reverse();
            let order = |v: &[[f64; 2]]| v.iter().map(bits).collect::<Vec<_>>();
            let (a, b) = (order(&pts), order(&back));
            return (a.min(b), true);
        }
        (pts.iter().map(bits).collect(), *closed)
    };
    let mut seen = std::collections::HashSet::new();
    paths.into_iter().filter(|p| seen.insert(key(p))).collect()
}

/// Joins open paths whose ends meet into longer paths, and closes those that come back
/// to their start. CAD outlines are often drawn as separate lines and arcs.
fn join_paths(paths: Vec<RawPath>) -> Vec<RawPath> {
    let near = |a: [f64; 2], b: [f64; 2]| {
        (a[0] - b[0]).abs() <= JOIN_TOLERANCE_MM && (a[1] - b[1]).abs() <= JOIN_TOLERANCE_MM
    };
    let (mut out, mut open): (Vec<_>, Vec<_>) = paths.into_iter().partition(|(_, closed)| *closed);
    while let Some((mut path, _)) = open.pop() {
        loop {
            let (start, end) = (path[0], path[path.len() - 1]);
            if path.len() > 2 && near(start, end) {
                path.pop();
                out.push((path, true));
                break;
            }
            // A piece continuing from either end, turned the right way round.
            let found = open.iter().position(|(q, _)| {
                let (qs, qe) = (q[0], q[q.len() - 1]);
                near(end, qs) || near(end, qe) || near(start, qs) || near(start, qe)
            });
            let Some(i) = found else {
                out.push((path, false));
                break;
            };
            let (mut q, _) = open.swap_remove(i);
            let (qs, qe) = (q[0], q[q.len() - 1]);
            if near(end, qs) {
                path.extend(q.drain(1..));
            } else if near(end, qe) {
                q.reverse();
                path.extend(q.drain(1..));
            } else {
                if near(start, qs) {
                    q.reverse();
                }
                // q now ends at `start`.
                q.pop();
                q.extend(path);
                path = q;
            }
        }
    }
    out
}

/// Reads a template's drawings into one frame: mm, y down, origin at the top-left of
/// everything.
pub fn load(entry: &TemplateEntry) -> Result<Template, String> {
    let units = match &entry.config.units {
        Some(u) => Some(units_override(u).ok_or_else(|| {
            tr!(
                "TEMPLATE_UNKNOWN_UNITS",
                u = u,
                path = entry.config_path().display()
            )
        })?),
        None => None,
    };
    // (source, paths, centers) in drawing mm.
    let mut elements = Vec::new();
    for src in &entry.sources {
        let raw = if src.is_svg() {
            item_paths(&crate::template_svg::read(&src.path, src.group.as_deref())?)
        } else {
            read_dxf(&src.path, units)?
        };
        let paths = drop_duplicates(join_paths(raw));
        let centers = centers(&paths);
        elements.push((src, paths, centers));
    }

    // One frame for all elements: the bounding box of their lines.
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in elements
        .iter()
        .flat_map(|(_, paths, _)| paths)
        .flat_map(|(pts, _)| pts)
    {
        min = [min[0].min(p[0]), min[1].min(p[1])];
        max = [max[0].max(p[0]), max[1].max(p[1])];
    }
    if min[0] > max[0] {
        let files: Vec<String> = entry
            .sources
            .iter()
            .map(|s| s.path.display().to_string())
            .collect();
        return Err(tr!("TEMPLATE_NO_GEOMETRY", files = files.join(", ")));
    }
    // Drawing mm, y up, to template mm, y down.
    let to_frame = |p: &[f64; 2]| [(p[0] - min[0]) as f32, (max[1] - p[1]) as f32];
    let elements = elements
        .into_iter()
        .map(|(src, paths, centers)| TemplateElement {
            kind: src.kind,
            name: src.name.clone(),
            color: src.color,
            polylines: paths
                .into_iter()
                .map(|(pts, closed)| Polyline {
                    points: pts.iter().map(to_frame).collect(),
                    closed,
                })
                .collect(),
            centers: centers.iter().map(to_frame).collect(),
        })
        .collect();

    Ok(Template {
        elements,
        size_mm: [(max[0] - min[0]) as f32, (max[1] - min[1]) as f32],
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn shapes_drawn_twice_are_kept_once() {
        let square = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        // Started at another corner, and run the other way.
        let shifted = vec![[10.0, 10.0], [0.0, 10.0], [0.0, 0.0], [10.0, 0.0]];
        let reversed = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]];
        let other = vec![[20.0, 0.0], [30.0, 0.0], [30.0, 10.0], [20.0, 10.0]];
        let paths = vec![
            (square.clone(), true),
            (square.clone(), true),
            (shifted, true),
            (reversed, true),
            (other.clone(), true),
            // The same points left open are another shape: a line, not a hole.
            (square.clone(), false),
        ];
        let kept = drop_duplicates(paths);
        assert_eq!(
            kept,
            vec![(square.clone(), true), (other, true), (square, false)]
        );
    }

    /// Folder of the templates used by the tests, independent of the shipped ones.
    pub(crate) fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/templates")
    }

    /// A 4 x 6 inch stick in four files, its outline drawn as separate lines.
    pub(crate) fn fixture() -> TemplateEntry {
        scan(&fixtures_dir())
            .into_iter()
            .find(|t| t.id == "test-stick")
            .expect("test-stick fixture folder")
    }

    #[test]
    fn fixture_loads_at_true_size() {
        let t = load(&fixture()).unwrap();
        let summary: Vec<_> = t
            .elements
            .iter()
            .map(|e| (e.kind, e.name.as_str(), e.polylines.len()))
            .collect();
        assert_eq!(
            summary,
            [
                (ElementKind::Outline, "Outline", 1),
                (ElementKind::Buttons, "Action buttons", 3),
                (ElementKind::Buttons, "Menu buttons", 2),
                (ElementKind::Cutouts, "Fastener holes", 2),
            ]
        );
        // Everything is closed, the panel edge included (drawn as separate pieces).
        assert!(
            t.elements
                .iter()
                .flat_map(|e| &e.polylines)
                .all(|p| p.closed)
        );
        // 4 x 6 inches.
        assert!(
            (t.size_mm[0] - 101.6).abs() < 1e-3,
            "width {}",
            t.size_mm[0]
        );
        assert!(
            (t.size_mm[1] - 152.4).abs() < 1e-3,
            "height {}",
            t.size_mm[1]
        );
        for p in t
            .elements
            .iter()
            .flat_map(|e| &e.polylines)
            .flat_map(|p| &p.points)
        {
            assert!(
                p[0] >= -1e-3
                    && p[1] >= -1e-3
                    && p[0] <= t.size_mm[0] + 1e-3
                    && p[1] <= t.size_mm[1] + 1e-3
            );
        }
    }

    #[test]
    fn config_is_read() {
        let e = fixture();
        assert!(e.config_error.is_none(), "{:?}", e.config_error);
        assert_eq!(e.name, "Test stick");
    }

    #[test]
    fn config_roundtrip_keeps_other_fields() {
        let dir = std::env::temp_dir().join(format!("artwork-tpl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&fixture().sources[0].path, dir.join("a.dxf")).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            // A placement, from an older version, is no error: left out.
            r#"{ "name": "My stick", "units": "inch",
                "placement": { "rotation": 90, "flip_y": true } }"#,
        )
        .unwrap();
        let mut e = read_entry(&dir).unwrap();
        assert!(e.config_error.is_none(), "{:?}", e.config_error);
        assert_eq!(e.name, "My stick");
        // A provided template is read only.
        assert!(e.delete().is_err());
        e.origin = Origin::User;
        // Without "elements", the folder's DXF is the outline.
        let t = load(&e).unwrap();
        assert_eq!(t.elements.len(), 1);
        assert_eq!(t.elements[0].kind, ElementKind::Outline);
        let mut config = e.config.clone();
        config.version = Some("2".into());
        write_config(&e.folder, &config).unwrap();
        let back = read_entry(&dir).unwrap();
        assert_eq!(back.config.version.as_deref(), Some("2"));
        assert_eq!(back.config.units.as_deref(), Some("inch"));
        assert_eq!(back.name, "My stick");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_goes_to_a_new_folder_with_the_new_name() {
        let root = std::env::temp_dir().join(format!("artwork-dup-{}", std::process::id()));
        let user = root.join("user");
        let source = fixture();
        let taken = vec![source.id.clone(), "Me - Stick".to_owned()];
        let folder = source
            .duplicate(&user, "Stick", Some("Me"), &taken)
            .unwrap();
        // "Me - Stick" is taken (by a provided template, say): the copy is numbered.
        assert_eq!(folder, user.join("Me - Stick (2)"));
        let all = scan_all(None, Some(&user));
        assert_eq!(all.len(), 1);
        let copy = &all[0];
        assert_eq!(copy.origin, Origin::User);
        assert_eq!(copy.full_name(), "Me - Stick");
        assert_eq!(copy.sources.len(), source.sources.len());
        let (a, b) = (load(&source).unwrap(), load(copy).unwrap());
        assert_eq!(a.size_mm, b.size_mm);
        copy.delete().unwrap();
        assert!(!folder.exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn user_template_hides_the_provided_one_of_the_same_folder() {
        let root = std::env::temp_dir().join(format!("artwork-hide-{}", std::process::id()));
        let dxf = &fixture().sources[0].path;
        for (dir, name) in [("provided", "Shipped"), ("user", "Mine")] {
            let folder = root.join(dir).join("stick");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::copy(dxf, folder.join("a.dxf")).unwrap();
            std::fs::write(
                folder.join(CONFIG_FILE),
                format!(r#"{{ "name": "{name}" }}"#),
            )
            .unwrap();
        }
        let all = scan_all(Some(&root.join("provided")), Some(&root.join("user")));
        assert_eq!(all.len(), 1);
        assert_eq!(
            (all[0].name.as_str(), all[0].origin),
            ("Mine", Origin::User)
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn folder_names_avoid_refused_characters() {
        let dir = Path::new("/nonexistent-artwork-dir");
        assert_eq!(unique_folder_name("A/B: c?. ", &[], dir), "A_B_ c_");
        assert_eq!(unique_folder_name(" .. ", &[], dir), "template");
        let taken = vec!["x".to_owned(), "X (2)".to_owned()];
        assert_eq!(unique_folder_name("X", &taken, dir), "X (3)");
    }

    #[test]
    fn templates_are_listed_by_vendor_then_name() {
        let dir = std::env::temp_dir().join(format!("artwork-vendors-{}", std::process::id()));
        let dxf = &fixture().sources[0].path;
        for (folder, json) in [
            ("b", r#"{ "vendor": "Zeta", "name": "A1" }"#),
            ("c", r#"{ "vendor": "Cosmox", "name": "T16" }"#),
            ("d", r#"{ "vendor": " cosmox ", "name": "M8" }"#),
            ("e", r#"{ "name": "Blank" }"#),
        ] {
            let f = dir.join(folder);
            std::fs::create_dir_all(&f).unwrap();
            std::fs::copy(dxf, f.join("a.dxf")).unwrap();
            std::fs::write(f.join(CONFIG_FILE), json).unwrap();
        }
        let list: Vec<String> = scan(&dir).iter().map(TemplateEntry::full_name).collect();
        // The vendor's case doesn't matter, nor spaces around it.
        assert_eq!(list, ["Blank", "cosmox - M8", "Cosmox - T16", "Zeta - A1"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_files_are_kept_fingerprinted_and_unpacked() {
        let installed = fixture();
        let files = TemplateFiles::read(&installed).unwrap();
        assert!(files.0.iter().any(|(n, _)| n == CONFIG_FILE));
        assert_eq!(files.0.len(), installed.sources.len() + 1);
        let fp = files.fingerprint();
        assert_eq!(fp.len(), 16);
        assert_eq!(fp, TemplateFiles::read(&installed).unwrap().fingerprint());
        // Unpacked, it reads and loads as the installed one.
        let dir = files.unpack(&installed.id).unwrap();
        let copy = read_entry(&dir).unwrap();
        assert_eq!(copy.id, installed.id);
        assert_eq!(
            load(&copy).unwrap().size_mm,
            load(&installed).unwrap().size_mm
        );
        assert_eq!(TemplateFiles::read(&copy).unwrap().fingerprint(), fp);
        // One byte changed: another fingerprint.
        let mut changed = files.clone();
        let mut data = (*changed.0[0].1).clone();
        data.push(b' ');
        changed.0[0].1 = std::sync::Arc::new(data);
        assert_ne!(changed.fingerprint(), fp);
        // Names leading out of the folder are refused.
        let evil = TemplateFiles(vec![("../evil.dxf".into(), std::sync::Arc::new(vec![]))]);
        assert!(evil.unpack("x").is_err());
    }

    #[test]
    fn templates_set_their_page() {
        let setup = |json: &str| serde_json::from_str::<PageConfig>(json).unwrap().setup();
        let a3 = setup(r#"{ "size": "a3", "orientation": "Portrait" }"#).unwrap();
        assert_eq!(a3.size, Some(PageSize::A3));
        assert_eq!(a3.orientation, Some(Orientation::Portrait));
        let custom = setup(r#"{ "size": [400, 250] }"#).unwrap();
        assert_eq!(custom.size, Some(PageSize::Custom));
        assert_eq!(custom.custom_mm, Some([250.0, 400.0]));
        assert_eq!(custom.orientation, Some(Orientation::Landscape));
        let only = setup(r#"{ "orientation": "landscape" }"#).unwrap();
        assert_eq!(
            (only.size, only.orientation),
            (None, Some(Orientation::Landscape))
        );
        assert_eq!(setup(r#"{ "size": "B5" }"#), Err("B5".into()));
        assert_eq!(
            setup(r#"{ "orientation": "sideways" }"#),
            Err("sideways".into())
        );
        assert!(setup(r#"{ "size": [5, 300] }"#).is_err(), "too small");
    }

    #[test]
    fn bundled_templates_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        for folder in std::fs::read_dir(&dir).unwrap().flatten() {
            let entry = read_entry(&folder.path()).expect("a template");
            assert!(entry.config_error.is_none(), "{:?}", entry.config_error);
            let t = load(&entry).unwrap();
            assert!(t.elements.iter().any(|e| e.kind == ElementKind::Outline));
            assert!(entry.page.size.is_some(), "{} sets its page", entry.id);
            if entry.id == "blank - A4 landscape" {
                let [w, h] = t.size_mm;
                assert!(
                    (w - 297.0).abs() < 0.01 && (h - 210.0).abs() < 0.01,
                    "{w} x {h}"
                );
                let outline = &t.elements[0].polylines;
                assert!(
                    outline.len() == 1 && outline[0].closed,
                    "one closed rectangle"
                );
            }
        }
    }

    /// Writes a DXF in mm with the given closed polygons and circles (center, radius).
    fn write_dxf(path: &Path, polygons: &[&[[f64; 2]]], circles: &[([f64; 2], f64)]) {
        use dxf::entities::{Circle, Entity, LwPolyline};
        use dxf::{LwPolylineVertex, Point};
        let mut d = dxf::Drawing::new();
        // R12, the default, has no LWPOLYLINE.
        d.header.version = dxf::enums::AcadVersion::R2000;
        d.header.default_drawing_units = Units::Millimeters;
        for poly in polygons {
            let mut pl = LwPolyline::default();
            pl.set_is_closed(true);
            pl.vertices = poly
                .iter()
                .map(|p| LwPolylineVertex {
                    x: p[0],
                    y: p[1],
                    ..Default::default()
                })
                .collect();
            d.add_entity(Entity::new(EntityType::LwPolyline(pl)));
        }
        for &(c, r) in circles {
            let circle = Circle::new(Point::new(c[0], c[1], 0.0), r);
            d.add_entity(Entity::new(EntityType::Circle(circle)));
        }
        d.save_file(path).unwrap();
    }

    #[test]
    fn closed_shapes_give_their_centers_and_anchors_are_left_out() {
        let dir = std::env::temp_dir().join(format!("artwork-centers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let panel: &[[f64; 2]] = &[[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
        write_dxf(&dir.join("edge.dxf"), &[panel], &[]);
        // Two buttons and a square.
        let square: &[[f64; 2]] = &[[70.0, 10.0], [80.0, 10.0], [80.0, 20.0], [70.0, 20.0]];
        write_dxf(
            &dir.join("buttons.dxf"),
            &[square],
            &[([20.0, 30.0], 12.0), ([50.0, 30.0], 12.0)],
        );
        // An anchors element, from an older version: left out, no error.
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{
                "units": "mm",
                "elements": [
                    { "kind": "outline", "dxf": "edge.dxf" },
                    { "kind": "buttons", "dxf": "buttons.dxf" },
                    { "kind": "anchors", "dxf": "buttons.dxf", "name": "Centers" }
                ]
            }"#,
        )
        .unwrap();
        let entry = read_entry(&dir).unwrap();
        assert!(entry.config_error.is_none(), "{:?}", entry.config_error);
        let t = load(&entry).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(t.elements.len(), 2);
        // The panel's middle.
        let outline = &t.elements[0];
        assert_eq!(outline.centers.len(), 1);
        assert!((outline.centers[0][0] - 50.0).abs() < 1e-3);
        // y down from the top of the panel.
        let mut got = t.elements[1].centers.clone();
        got.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let want = [[20.0, 20.0], [50.0, 20.0], [75.0, 35.0]];
        assert_eq!(got.len(), 3, "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g[0] - w[0]).abs() < 1e-3 && (g[1] - w[1]).abs() < 1e-3,
                "{got:?}"
            );
        }
    }

    #[test]
    fn outlines_have_no_point_twice_in_a_row() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        for entry in scan(&dir) {
            let t = load(&entry).unwrap();
            for e in &t.elements {
                for p in &e.polylines {
                    let n = p.points.len();
                    let last = if p.closed { n } else { n - 1 };
                    for i in 0..last {
                        let (a, b) = (p.points[i], p.points[(i + 1) % n]);
                        let d = (a[0] - b[0]).hypot(a[1] - b[1]);
                        assert!(
                            d > 1e-4,
                            "{} {}: point {i} of {n} repeated",
                            entry.id,
                            e.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn template_json_is_written_in_the_same_order() {
        let config: TemplateConfig = serde_json::from_str(
            r##"{ "bindings_origin": [1, 2], "bindings": { "b-1": { "x": "a" } },
                 "variables": [ { "name": "a", "kind": "length", "value": 1 } ],
                 "elements": [ { "color": "#ff0000", "name": "Outline", "id": "outline",
                                 "kind": "outline" } ],
                 "page": { "orientation": "landscape", "size": "A4" },
                 "source": { "version": "1.1", "vendor": "Cosmox", "name": "T16" },
                 "version": "1.0", "vendor": "Cosmox", "name": "T16 copy" }"##,
        )
        .unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let at = |key: &str| {
            json.find(&format!("\"{key}\":"))
                .unwrap_or_else(|| panic!("{key}"))
        };
        let keys = [
            "name",
            "vendor",
            "version",
            "source",
            "page",
            "elements",
            "variables",
            "bindings",
            "bindings_origin",
        ];
        for pair in keys.windows(2) {
            assert!(at(pair[0]) < at(pair[1]), "{} before {}", pair[0], pair[1]);
        }
        // Inside: an element's kind first, a source's name, a page's size.
        assert!(
            json.contains(
                r##"{"kind":"outline","id":"outline","name":"Outline","color":"#ff0000"}"##
            )
        );
        assert!(json.contains(r#"{"name":"T16","vendor":"Cosmox","version":"1.1"}"#));
        assert!(json.contains(r#"{"size":"A4","orientation":"landscape"}"#));
        // Read and written again: the same text.
        let again: TemplateConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&again).unwrap(), json);
    }

    #[test]
    fn versions_are_two_whole_numbers() {
        for v in ["1.0", "2.13", "0.1"] {
            assert!(is_valid_version(v), "{v}");
        }
        for v in ["", "1", "1.", ".1", "1.0.2", "v1.0", "1.a", " 1.0"] {
            assert!(!is_valid_version(v), "{v}");
        }
    }

    #[test]
    fn a_copy_names_its_source() {
        let config: TemplateConfig = serde_json::from_str(
            r#"{ "name": "T16 copy", "version": "1.2",
                 "source": { "name": "T16", "vendor": "Cosmox", "version": "1.1" } }"#,
        )
        .unwrap();
        let source = config.source.clone().unwrap();
        assert_eq!(source.describe(), "Cosmox - T16 - v1.1");
        // A source lacking its name or version, or with a blank one, is ignored.
        for json in [
            r#"{ "source": { "name": "T16", "vendor": "Cosmox" } }"#,
            r#"{ "source": { "vendor": "Cosmox", "version": "1.1" } }"#,
            r#"{ "source": { "name": " ", "vendor": "Cosmox", "version": "1.1" } }"#,
            r#"{ "source": "T16" }"#,
        ] {
            let config: TemplateConfig = serde_json::from_str(json).unwrap();
            assert_eq!(config.source, None, "{json}");
        }
        // Its vendor may be left out.
        let bare: TemplateConfig = serde_json::from_str(
            r#"{ "source": { "name": "T16", "vendor": " ", "version": "1.1" } }"#,
        )
        .unwrap();
        let source = bare.source.unwrap();
        assert_eq!(source.vendor, None);
        assert_eq!(source.describe(), "T16 - v1.1");
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(
            json["source"],
            serde_json::json!({ "name": "T16", "vendor": "Cosmox", "version": "1.1" })
        );
        // Nothing written for what a template doesn't have.
        assert!(json.get("variables").is_none() && json.get("bindings").is_none());
    }

    #[test]
    fn settings_per_kind_of_an_older_version_still_read() {
        // Outline, buttons, cutouts and anchors.
        assert_eq!(
            per_kind(&[false, true, true, false], [true; KINDS]),
            [false, true, true]
        );
        assert_eq!(per_kind(&[false], [true; KINDS]), [false, true, true]);
    }

    #[test]
    fn elements_from_separate_files_share_one_frame() {
        let dir = std::env::temp_dir().join(format!("artwork-elements-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A 100 x 50 panel from (10, 20), a button at (40, 45) and a hole at (100, 60).
        let panel: &[[f64; 2]] = &[[10.0, 20.0], [110.0, 20.0], [110.0, 70.0], [10.0, 70.0]];
        write_dxf(&dir.join("edge.dxf"), &[panel], &[]);
        write_dxf(&dir.join("buttons.dxf"), &[], &[([40.0, 45.0], 12.0)]);
        write_dxf(&dir.join("holes.dxf"), &[], &[([100.0, 60.0], 2.0)]);
        std::fs::write(
            dir.join(CONFIG_FILE),
            r##"{
                "name": "Split",
                "units": "mm",
                "elements": [
                    { "kind": "outline", "dxf": "edge.dxf" },
                    { "kind": "buttons", "dxf": "buttons.dxf", "color": "#00ff00" },
                    { "kind": "cutouts", "dxf": "holes.dxf", "name": "Screws" }
                ]
            }"##,
        )
        .unwrap();
        let entry = read_entry(&dir).unwrap();
        assert!(entry.config_error.is_none(), "{:?}", entry.config_error);
        assert_eq!(entry.stamp().len(), 4, "config and three DXF files");
        let t = load(&entry).unwrap();
        let s = t.size_mm;
        assert!(
            (s[0] - 100.0).abs() < 1e-3 && (s[1] - 50.0).abs() < 1e-3,
            "{s:?}"
        );
        let kinds: Vec<_> = t.elements.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                ElementKind::Outline,
                ElementKind::Buttons,
                ElementKind::Cutouts
            ]
        );
        assert_eq!(t.elements[1].color, Some([0, 255, 0]));
        assert_eq!(t.elements[2].name, "Screws");
        // Button center: x 40 - 10 = 30, y flipped: 70 - 45 = 25.
        let b = &t.elements[1].polylines[0].points;
        let n = b.len() as f32;
        let c = b
            .iter()
            .fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
        assert!(
            (c[0] - 30.0).abs() < 0.1 && (c[1] - 25.0).abs() < 0.1,
            "{c:?}"
        );

        // A missing element file is reported.
        std::fs::remove_file(dir.join("holes.dxf")).unwrap();
        let err = load(&read_entry(&dir).unwrap()).unwrap_err();
        assert!(err.contains("holes.dxf"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_pieces_are_joined_into_closed_paths() {
        // A square drawn as four separate lines, two of them backwards, plus a stray line.
        let pieces = vec![
            (vec![[0.0, 0.0], [10.0, 0.0]], false),
            (vec![[10.0, 10.0], [10.0, 0.0]], false),
            (vec![[0.0, 10.0], [0.0, 0.0]], false),
            (vec![[0.0, 10.0], [10.0, 10.0]], false),
            (vec![[50.0, 50.0], [60.0, 50.0]], false),
        ];
        let joined = join_paths(pieces);
        assert_eq!(joined.len(), 2);
        let square = joined.iter().find(|(_, c)| *c).expect("closed square");
        assert_eq!(square.0.len(), 4);
        assert!(joined.iter().any(|(p, c)| !c && p.len() == 2));
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_color("00AAff"), Some([0, 170, 255]));
        assert_eq!(parse_color("#fff"), None);
    }

    #[test]
    fn bulge_one_is_a_semicircle() {
        let mut out = Vec::new();
        push_bulge_segment(&mut out, [0.0, 0.0], [2.0, 0.0], 1.0);
        let end = *out.last().unwrap();
        assert!((end[0] - 2.0).abs() < 1e-9 && end[1].abs() < 1e-9);
        // Positive bulge runs counter-clockwise, so the arc dips below the chord.
        assert!(
            out.iter()
                .all(|p| ((p[0] - 1.0).powi(2) + p[1].powi(2) - 1.0).abs() < 1e-9)
        );
        assert!(out.iter().any(|p| p[1] < -0.99));
    }
}
