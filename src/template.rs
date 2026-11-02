//! Controller templates: each subfolder of `templates/` holding `.dxf` files.
//!
//! A template is made of elements (outline, buttons, cutouts), each drawn in its own DXF
//! file. The files share one coordinate system, as when exported from the same drawing.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dxf::entities::EntityType;
use dxf::enums::Units;
use serde::{Deserialize, Serialize};

use crate::document::{P2, TemplatePlacement};

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

    /// Name shown in the app.
    pub fn label(self) -> &'static str {
        match self {
            ElementKind::Outline => "Outline",
            ElementKind::Buttons => "Buttons",
            ElementKind::Cutouts => "Cutouts",
        }
    }

    /// Line colour used when `template.json` sets none.
    pub fn default_color(self) -> [u8; 3] {
        match self {
            ElementKind::Outline => [255, 0, 255],
            ElementKind::Buttons => [0, 170, 255],
            ElementKind::Cutouts => [255, 140, 0],
        }
    }

    /// Material is removed inside these shapes.
    pub fn is_hole(self) -> bool {
        self != ElementKind::Outline
    }
}

/// One element in `template.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ElementConfig {
    /// What the element is for.
    pub kind: ElementKind,
    /// DXF file inside the template folder.
    pub dxf: String,
    /// Display name. Defaults to the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Line colour as "#rrggbb". Defaults to a colour per kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Contents of `template.json`. Every field is optional.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateConfig {
    /// Display name. Defaults to the folder name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The template's elements. When empty, a single DXF (`dxf`, or else the first
    /// `.dxf` found) is the outline.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<ElementConfig>,
    /// DXF file of a template without `elements`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dxf: Option<String>,
    /// Drawing units when the DXF does not declare them: "mm", "cm", "inch".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// Default placement on the page. Rotation is chosen automatically when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<TemplatePlacement>,
}

/// An element's DXF file, resolved from the config.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementSource {
    /// What the element is for.
    pub kind: ElementKind,
    /// Shown in the app.
    pub name: String,
    /// The DXF file.
    pub path: PathBuf,
    /// Colour from `template.json`, if set.
    pub color: Option<[u8; 3]>,
}

/// An installed template folder, read but not loaded yet.
#[derive(Clone, Debug)]
pub struct TemplateEntry {
    /// Folder name; this is what projects store.
    pub id: String,
    /// Shown to the user.
    pub name: String,
    /// The template's folder.
    pub folder: PathBuf,
    /// Its elements' DXF files.
    pub sources: Vec<ElementSource>,
    /// Parsed `template.json`; defaults if missing or invalid.
    pub config: TemplateConfig,
    /// Error from reading `template.json`, if any.
    pub config_error: Option<String>,
}

impl TemplateEntry {
    /// Path of the folder's `template.json`.
    pub fn config_path(&self) -> PathBuf {
        self.folder.join(CONFIG_FILE)
    }

    /// Modification times of the config and the DXF files, used to reload on change.
    pub fn stamp(&self) -> Vec<Option<SystemTime>> {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        std::iter::once(self.config_path())
            .chain(self.sources.iter().map(|s| s.path.clone()))
            .map(|p| mtime(&p))
            .collect()
    }

    /// Placement from the config, or an automatic rotation for this page.
    pub fn default_placement(&self, tpl_size: P2, page: P2) -> TemplatePlacement {
        self.config
            .placement
            .unwrap_or_else(|| TemplatePlacement::auto(tpl_size, page))
    }

    /// Writes `placement` into `template.json`, keeping the other settings.
    pub fn save_placement(&mut self, placement: TemplatePlacement) -> Result<(), String> {
        self.config.placement = Some(placement);
        let json = serde_json::to_string_pretty(&self.config).map_err(|e| e.to_string())?;
        let path = self.config_path();
        std::fs::write(&path, json + "\n")
            .map_err(|e| format!("Cannot write {}: {e}", path.display()))
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

/// Reads one template folder. Returns `None` if it holds no DXF file and lists no elements.
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
                config_error.get_or_insert(format!(
                    "{}: invalid colour \"{c}\" (expected #rrggbb)",
                    folder.join(CONFIG_FILE).display()
                ));
            }
            parsed
        });
        sources.push(ElementSource {
            kind: e.kind,
            name: e.name.clone().unwrap_or_else(|| e.kind.label().to_owned()),
            path: folder.join(&e.dxf),
            color,
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
            color: None,
        });
    }
    Some(TemplateEntry {
        name: config.name.clone().unwrap_or_else(|| id.clone()),
        id,
        folder: folder.to_path_buf(),
        sources,
        config,
        config_error,
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
    /// Colour from `template.json`, if set.
    pub color: Option<[u8; 3]>,
    /// Its shapes in template mm.
    pub polylines: Vec<Polyline>,
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

/// Lists every subfolder that contains a `.dxf` file, sorted by display name.
pub fn scan(dir: &Path) -> Vec<TemplateEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<TemplateEntry> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| read_entry(&p))
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

/// Millimetres per unit for the `units` setting of `template.json`.
fn units_override(name: &str) -> Option<f64> {
    match name.trim().to_lowercase().as_str() {
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => Some(1.0),
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => Some(10.0),
        "in" | "inch" | "inches" => Some(25.4),
        _ => None,
    }
}

/// Millimetres per drawing unit declared in a DXF header.
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
type RawPath = (Vec<[f64; 2]>, bool);

/// Paths of one DXF file in mm, y up.
fn read_dxf(path: &Path, units: Option<f64>) -> Result<Vec<RawPath>, String> {
    let drawing = dxf::Drawing::load_file(path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;

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
    let k = units.unwrap_or_else(|| units_to_mm(drawing.header.default_drawing_units));
    for (pts, _) in &mut raw {
        for p in pts.iter_mut() {
            *p = [p[0] * k, p[1] * k];
        }
    }
    Ok(raw)
}

/// Endpoints closer than this (mm) are treated as the same point.
const JOIN_TOLERANCE_MM: f64 = 0.01;

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

/// Reads a template's DXF files into one frame: mm, y down, origin at the top-left of
/// everything.
pub fn load(entry: &TemplateEntry) -> Result<Template, String> {
    let units = match &entry.config.units {
        Some(u) => Some(units_override(u).ok_or_else(|| {
            format!("Unknown units \"{u}\" in {}", entry.config_path().display())
        })?),
        None => None,
    };
    let mut elements = Vec::new();
    for src in &entry.sources {
        let paths = join_paths(read_dxf(&src.path, units)?);
        elements.push((src, paths));
    }

    // One frame for all elements: the bounding box of everything.
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in elements
        .iter()
        .flat_map(|(_, paths)| paths)
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
        return Err(format!("No template geometry in {}", files.join(", ")));
    }
    let elements = elements
        .into_iter()
        .map(|(src, paths)| TemplateElement {
            kind: src.kind,
            name: src.name.clone(),
            color: src.color,
            polylines: paths
                .into_iter()
                .map(|(pts, closed)| Polyline {
                    points: pts
                        .iter()
                        .map(|p| [(p[0] - min[0]) as f32, (max[1] - p[1]) as f32])
                        .collect(),
                    closed,
                })
                .collect(),
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
    fn config_placement_is_read() {
        let e = fixture();
        assert!(e.config_error.is_none(), "{:?}", e.config_error);
        assert_eq!(e.name, "Test stick");
        let p = e.config.placement.expect("fixture placement");
        assert_eq!((p.rotation, p.flip_x, p.flip_y), (90, false, true));
    }

    #[test]
    fn config_roundtrip_keeps_other_fields() {
        let dir = std::env::temp_dir().join(format!("artwork-tpl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&fixture().sources[0].path, dir.join("a.dxf")).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{ "name": "My stick", "units": "inch" }"#,
        )
        .unwrap();
        let mut e = read_entry(&dir).unwrap();
        assert_eq!(e.name, "My stick");
        // Without "elements", the folder's DXF is the outline.
        let t = load(&e).unwrap();
        assert_eq!(t.elements.len(), 1);
        assert_eq!(t.elements[0].kind, ElementKind::Outline);
        let placement = TemplatePlacement {
            rotation: 270,
            flip_x: true,
            flip_y: false,
            offset_mm: [1.0, -2.0],
        };
        e.save_placement(placement).unwrap();
        let back = read_entry(&dir).unwrap();
        assert_eq!(back.config.placement, Some(placement));
        assert_eq!(back.config.units.as_deref(), Some("inch"));
        assert_eq!(back.name, "My stick");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Writes a DXF in mm with the given closed polygons and circles (centre, radius).
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
        assert_eq!(kinds, ElementKind::ALL);
        assert_eq!(t.elements[1].color, Some([0, 255, 0]));
        assert_eq!(t.elements[2].name, "Screws");
        // Button centre: x 40 - 10 = 30, y flipped: 70 - 45 = 25.
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
    fn colours() {
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
