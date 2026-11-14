//! Template diagnostic: whether a template is valid. Its files and settings are checked
//! (`template.json`, the groups of `template.svg`, the DXF files), then its geometry: one
//! closed outline, closed shapes, holes inside the outline and apart from each other,
//! all on the page it asks for. Installed templates are checked when they are listed
//! (`check_entry`); the template being edited is checked as it changes
//! (`check_document`), each problem naming the layer to select.

use crate::document::{Document, Layer, Node, Orientation};
use crate::i18n::{tr, tr_n, translate};
use crate::template::{self, ElementKind, TemplateConfig, TemplateEntry};

/// How serious a problem is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The template is not valid: it can't be used, or not as it is meant.
    Error,
    /// Likely a mistake, but the template can be used.
    Warning,
}

/// A problem found in a template.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub severity: Severity,
    /// What is wrong, translated.
    pub text: String,
    /// The layer it is about, in the template editor.
    pub layer: Option<u64>,
}

impl Problem {
    fn error(text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            text: text.into(),
            layer: None,
        }
    }

    fn warning(text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            text: text.into(),
            layer: None,
        }
    }

    fn on(self, layer: Option<u64>) -> Self {
        Self { layer, ..self }
    }
}

/// Whether some of `problems` are errors.
pub fn has_errors(problems: &[Problem]) -> bool {
    problems.iter().any(|p| p.severity == Severity::Error)
}

/// A shape to check: its points (mm) and whether it is closed.
struct Shape {
    points: Vec<[f64; 2]>,
    closed: bool,
    /// How problems name it.
    name: String,
    /// Its layer in the template editor.
    layer: Option<u64>,
}

/// An element to check, with its shapes.
struct Element {
    kind: ElementKind,
    name: String,
    shapes: Vec<Shape>,
}

// ---- Installed templates ----

/// The problems of an installed template, its files and settings first.
pub fn check_entry(entry: &TemplateEntry) -> Vec<Problem> {
    let mut out = Vec::new();
    let config_path = entry.config_path();
    if let Some(e) = &entry.config_error {
        out.push(Problem::error(e.clone()));
    }
    version_problem(&entry.config, &mut out);
    for e in &entry.config.elements {
        if let Some(c) = &e.color
            && template::parse_color(c).is_none()
        {
            out.push(Problem::error(tr!(
                "TEMPLATE_INVALID_COLOR",
                path = config_path.display(),
                c = c
            )));
        }
        if e.id.is_none() && e.dxf.is_none() {
            out.push(Problem::error(tr!(
                "TEMPLATE_ELEMENT_WITHOUT_DRAWING",
                path = config_path.display()
            )));
        }
    }
    let ids: Vec<&str> = entry
        .config
        .elements
        .iter()
        .filter_map(|e| e.id.as_deref())
        .collect();
    // Each id named twice or more, told once.
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].iter().filter(|x| *x == id).count() == 1 {
            out.push(Problem::error(tr!("TEMPLATE_CHECK_SAME_GROUP", id = id)));
        }
    }
    if let Some(u) = &entry.config.units
        && template::units_override(u).is_none()
    {
        out.push(Problem::error(tr!(
            "TEMPLATE_UNKNOWN_UNITS",
            u = u,
            path = config_path.display()
        )));
    }
    // Each element's drawing is there.
    let mut readable = true;
    for src in &entry.sources {
        if !src.path.is_file() {
            readable = false;
            out.push(Problem::error(tr!(
                "TEMPLATE_CHECK_MISSING_FILE",
                element = translate(&src.name),
                path = src.path.display()
            )));
            continue;
        }
        if let (true, Some(id)) = (src.is_svg(), &src.group) {
            match crate::template_svg::has_group(&src.path, id) {
                Ok(true) => {}
                Ok(false) => out.push(Problem::error(tr!(
                    "TEMPLATE_CHECK_MISSING_GROUP",
                    element = translate(&src.name),
                    id = id
                ))),
                Err(e) => {
                    readable = false;
                    out.push(Problem::error(e));
                }
            }
        }
    }
    // Shapes of the SVG in no element: the template leaves them out.
    if let Some(svg) = entry.sources.iter().find(|s| s.is_svg())
        && readable
        && entry
            .sources
            .iter()
            .all(|s| !s.is_svg() || s.group.is_some())
    {
        let ids: Vec<&str> = entry
            .sources
            .iter()
            .filter_map(|s| s.group.as_deref())
            .collect();
        if let Ok(orphans) = crate::template_svg::read_orphans(&svg.path, &ids) {
            let n = template::item_paths(&orphans).len();
            if n > 0 {
                out.push(Problem::warning(tr_n!("TEMPLATE_CHECK_SVG_ORPHANS", n)));
            }
        }
    }
    // The geometry, as projects get it.
    match template::load(entry) {
        Ok(t) => {
            let elements: Vec<Element> = t
                .elements
                .iter()
                .map(|e| Element {
                    kind: e.kind,
                    name: translate(&e.name).to_owned(),
                    shapes: e
                        .polylines
                        .iter()
                        .map(|p| {
                            let points: Vec<[f64; 2]> = p
                                .points
                                .iter()
                                .map(|q| [q[0] as f64, q[1] as f64])
                                .collect();
                            let (min, max) = bounds(&points);
                            Shape {
                                name: tr!(
                                    "TEMPLATE_CHECK_SHAPE_AT",
                                    x = format!("{:.1}", (min[0] + max[0]) / 2.0),
                                    y = format!("{:.1}", (min[1] + max[1]) / 2.0)
                                ),
                                points,
                                closed: p.closed,
                                layer: None,
                            }
                        })
                        .collect(),
                })
                .collect();
            check_geometry(&elements, &entry.config, &mut out);
        }
        // Already told when a file can't be read.
        Err(e) if readable => out.push(Problem::error(e)),
        Err(_) => {}
    }
    dedup(out)
}

// ---- The template editor ----

/// A version missing or not following the "x.y" convention.
fn version_problem(config: &TemplateConfig, out: &mut Vec<Problem>) {
    match config.version.as_deref().map(str::trim) {
        None | Some("") => out.push(Problem::error(tr!("TEMPLATE_VERSION_REQUIRED"))),
        Some(v) if !template::is_valid_version(v) => {
            out.push(Problem::error(tr!("TEMPLATE_VERSION_INVALID", version = v)));
        }
        Some(_) => {}
    }
}

/// The problems of the template edited in `doc`; those about a layer name it.
pub fn check_document(doc: &Document) -> Vec<Problem> {
    let mut out = Vec::new();
    let config = crate::template_edit::config(doc)
        .cloned()
        .unwrap_or_default();
    if config.name.as_deref().is_none_or(|n| n.trim().is_empty()) {
        out.push(Problem::error(tr!("TEMPLATE_NAME_REQUIRED")));
    }
    version_problem(&config, &mut out);
    // Fields linked to variables the template doesn't have.
    for leaf in doc.leaves() {
        let l = &leaf.layer;
        let gone = crate::variables::broken_links(l, &doc.variables);
        if !gone.is_empty() {
            out.push(
                Problem::warning(tr!(
                    "TEMPLATE_CHECK_BROKEN_LINKS",
                    shape = l.name,
                    names = gone.join(", ")
                ))
                .on(Some(l.id)),
            );
        }
    }
    let mut elements = Vec::new();
    // In list order, from the top: the order of `template.json`.
    for node in doc.layers.iter().rev() {
        match node {
            Node::Group(g) if g.element.is_some() => {
                let kind = g.element.map(|t| t.kind).unwrap_or(ElementKind::Outline);
                let mut shapes = Vec::new();
                let mut layers = Vec::new();
                layers_of(node, &mut layers);
                // Top of the list first.
                for l in layers.into_iter().rev() {
                    if l.shape.is_none() {
                        out.push(
                            Problem::error(tr!(
                                "TEMPLATE_CHECK_NOT_A_SHAPE",
                                element = g.name,
                                shape = l.name
                            ))
                            .on(Some(l.id)),
                        );
                        continue;
                    }
                    let items = crate::template_geom::layer_items(l);
                    for (points, closed) in template::item_paths(&items) {
                        shapes.push(Shape {
                            points,
                            closed,
                            name: l.name.clone(),
                            layer: Some(l.id),
                        });
                    }
                }
                elements.push(Element {
                    kind,
                    name: g.name.clone(),
                    shapes,
                });
            }
            _ => out.push(
                Problem::error(tr!("TEMPLATE_CHECK_ORPHAN", shape = node_name(node)))
                    .on(Some(node.id())),
            ),
        }
    }
    check_geometry(&elements, &config, &mut out);
    dedup(out)
}

/// The layers of a node, bottom of the stack first.
fn layers_of<'a>(node: &'a Node, out: &mut Vec<&'a Layer>) {
    match node {
        Node::Layer(l) => out.push(l),
        Node::Group(g) => g.children.iter().for_each(|c| layers_of(c, out)),
    }
}

/// A layer's or group's name.
fn node_name(node: &Node) -> &str {
    match node {
        Node::Layer(l) => &l.name,
        Node::Group(g) => &g.name,
    }
}

// ---- Geometry ----

/// Same text and layer once.
fn dedup(problems: Vec<Problem>) -> Vec<Problem> {
    let mut out: Vec<Problem> = Vec::new();
    for p in problems {
        if !out.iter().any(|q| q.text == p.text && q.layer == p.layer) {
            out.push(p);
        }
    }
    out
}

/// Bounding box of points.
fn bounds(points: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    points
        .iter()
        .fold(([f64::MAX; 2], [f64::MIN; 2]), |(mn, mx), p| {
            (
                [mn[0].min(p[0]), mn[1].min(p[1])],
                [mx[0].max(p[0]), mx[1].max(p[1])],
            )
        })
}

/// Two boxes overlap.
fn boxes_meet(a: ([f64; 2], [f64; 2]), b: ([f64; 2], [f64; 2])) -> bool {
    a.0[0] <= b.1[0] && b.0[0] <= a.1[0] && a.0[1] <= b.1[1] && b.0[1] <= a.1[1]
}

/// Point `p` is inside the closed polygon `poly` (even-odd).
fn inside(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut odd = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// Segments `a`-`b` and `c`-`d` cross (touching ends does not count).
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let side = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    const EPS: f64 = 1e-9;
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    ((d1 > EPS && d2 < -EPS) || (d1 < -EPS && d2 > EPS))
        && ((d3 > EPS && d4 < -EPS) || (d3 < -EPS && d4 > EPS))
}

/// The edges of two closed polygons cross.
fn edges_cross(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    fn edges(p: &[[f64; 2]]) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        let n = p.len();
        (0..n).map(move |i| (p[i], p[(i + 1) % n]))
    }
    let bb = bounds(b);
    edges(a).any(|(p, q)| {
        let eb = (
            [p[0].min(q[0]), p[1].min(q[1])],
            [p[0].max(q[0]), p[1].max(q[1])],
        );
        boxes_meet(eb, bb) && edges(b).any(|(r, s)| segments_cross(p, q, r, s))
    })
}

/// Two closed shapes overlap: their edges cross, or one is inside the other.
fn overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    boxes_meet(bounds(a), bounds(b)) && (edges_cross(a, b) || inside(a[0], b) || inside(b[0], a))
}

/// Page width and height (mm) the settings ask for, both ways when they leave the
/// orientation out; `None` without a size.
fn pages(config: &TemplateConfig) -> Option<Vec<[f64; 2]>> {
    let setup = config.page.as_ref()?.setup().ok()?;
    let [w, h] = setup
        .custom_mm
        .or_else(|| setup.size.and_then(|s| s.portrait_mm()))?
        .map(f64::from);
    Some(match setup.orientation {
        Some(Orientation::Portrait) => vec![[w, h]],
        Some(Orientation::Landscape) => vec![[h, w]],
        None => vec![[w, h], [h, w]],
    })
}

/// The problems of the elements' shapes, and of their size on the page `config` asks
/// for.
fn check_geometry(elements: &[Element], config: &TemplateConfig, out: &mut Vec<Problem>) {
    let outlines: Vec<&Element> = elements
        .iter()
        .filter(|e| e.kind == ElementKind::Outline)
        .collect();
    if outlines.is_empty() {
        out.push(Problem::error(tr!("TEMPLATE_CHECK_NO_OUTLINE")));
    } else if outlines.len() > 1 {
        out.push(Problem::error(tr_n!(
            "TEMPLATE_CHECK_SEVERAL_OUTLINES",
            outlines.len()
        )));
    }
    for e in elements {
        if e.shapes.is_empty() {
            out.push(Problem::warning(tr!(
                "TEMPLATE_CHECK_EMPTY_ELEMENT",
                element = e.name
            )));
        }
        for s in e.shapes.iter().filter(|s| !s.closed) {
            out.push(
                Problem::error(tr!(
                    "TEMPLATE_CHECK_OPEN_SHAPE",
                    element = e.name,
                    shape = s.name
                ))
                .on(s.layer),
            );
        }
    }
    let outline_shapes: Vec<&Shape> = outlines.iter().flat_map(|e| &e.shapes).collect();
    if outline_shapes.len() > 1 {
        out.push(
            Problem::warning(tr_n!("TEMPLATE_CHECK_OUTLINE_SHAPES", outline_shapes.len()))
                .on(outline_shapes.get(1).and_then(|s| s.layer)),
        );
    }
    // Holes: inside the outline, apart from each other.
    let rings: Vec<&[[f64; 2]]> = outline_shapes
        .iter()
        .filter(|s| s.closed && s.points.len() > 2)
        .map(|s| s.points.as_slice())
        .collect();
    let holes: Vec<(&Element, &Shape)> = elements
        .iter()
        .filter(|e| e.kind != ElementKind::Outline)
        .flat_map(|e| e.shapes.iter().map(move |s| (e, s)))
        .filter(|(_, s)| s.closed && s.points.len() > 2)
        .collect();
    if !rings.is_empty() {
        for (e, s) in &holes {
            let within = rings
                .iter()
                .any(|r| s.points.iter().all(|&p| inside(p, r)) && !edges_cross(&s.points, r));
            if within {
                continue;
            }
            let key = if rings.iter().any(|r| overlap(&s.points, r)) {
                tr!(
                    "TEMPLATE_CHECK_HOLE_CROSSES_OUTLINE",
                    element = e.name,
                    shape = s.name
                )
            } else {
                tr!(
                    "TEMPLATE_CHECK_HOLE_OUTSIDE",
                    element = e.name,
                    shape = s.name
                )
            };
            out.push(Problem::error(key).on(s.layer));
        }
    }
    for (i, (ea, a)) in holes.iter().enumerate() {
        for (eb, b) in &holes[i + 1..] {
            if overlap(&a.points, &b.points) {
                out.push(
                    Problem::error(tr!(
                        "TEMPLATE_CHECK_HOLES_OVERLAP",
                        a = format!("{} ({})", a.name, ea.name),
                        b = format!("{} ({})", b.name, eb.name)
                    ))
                    .on(a.layer.or(b.layer)),
                );
            }
        }
    }
    // On its page.
    let points: Vec<[f64; 2]> = elements
        .iter()
        .flat_map(|e| &e.shapes)
        .flat_map(|s| s.points.iter().copied())
        .collect();
    if let (Some(pages), false) = (pages(config), points.is_empty()) {
        let (min, max) = bounds(&points);
        let size = [max[0] - min[0], max[1] - min[1]];
        // Lines on the very edge still fit.
        const SLACK_MM: f64 = 0.01;
        if !pages
            .iter()
            .any(|p| size[0] <= p[0] + SLACK_MM && size[1] <= p[1] + SLACK_MM)
        {
            let [pw, ph] = pages[0];
            out.push(Problem::error(tr!(
                "TEMPLATE_CHECK_LARGER_THAN_PAGE",
                w = format!("{:.1}", size[0]),
                h = format!("{:.1}", size[1]),
                pw = format!("{pw:.0}"),
                ph = format!("{ph:.0}")
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The problems as one text, one per line, errors first.
    fn summary(problems: &[Problem]) -> String {
        let mut sorted: Vec<&Problem> = problems.iter().collect();
        sorted.sort_by_key(|p| p.severity);
        sorted
            .iter()
            .map(|p| format!("{:?}: {}", p.severity, p.text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn square(x: f64, y: f64, s: f64) -> Vec<[f64; 2]> {
        vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]]
    }

    fn shape(points: Vec<[f64; 2]>, closed: bool) -> Shape {
        Shape {
            points,
            closed,
            name: "s".into(),
            layer: None,
        }
    }

    fn element(kind: ElementKind, shapes: Vec<Shape>) -> Element {
        Element {
            kind,
            name: "e".into(),
            shapes,
        }
    }

    fn problems(elements: &[Element], config: &TemplateConfig) -> Vec<Problem> {
        let mut out = Vec::new();
        check_geometry(elements, config, &mut out);
        out
    }

    #[test]
    fn a_closed_outline_with_holes_inside_and_apart_is_valid() {
        let elements = [
            element(
                ElementKind::Outline,
                vec![shape(square(0.0, 0.0, 100.0), true)],
            ),
            element(
                ElementKind::Buttons,
                vec![
                    shape(square(10.0, 10.0, 20.0), true),
                    shape(square(40.0, 10.0, 20.0), true),
                ],
            ),
        ];
        assert_eq!(problems(&elements, &TemplateConfig::default()), vec![]);
    }

    #[test]
    fn holes_outside_crossing_or_overlapping_and_open_shapes_are_errors() {
        let elements = [
            element(
                ElementKind::Outline,
                vec![shape(square(0.0, 0.0, 100.0), true)],
            ),
            element(
                ElementKind::Buttons,
                vec![
                    // Overlap each other.
                    shape(square(10.0, 10.0, 20.0), true),
                    shape(square(20.0, 20.0, 20.0), true),
                    // Crosses the outline.
                    shape(square(90.0, 50.0, 20.0), true),
                    // Outside.
                    shape(square(200.0, 0.0, 20.0), true),
                ],
            ),
            element(
                ElementKind::Cutouts,
                vec![shape(square(60.0, 60.0, 5.0), false)],
            ),
        ];
        let found = problems(&elements, &TemplateConfig::default());
        let texts: Vec<&str> = found.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(found.len(), 4, "{texts:#?}");
        assert!(found.iter().all(|p| p.severity == Severity::Error));
    }

    #[test]
    fn a_template_without_outline_or_larger_than_its_page_is_an_error() {
        let elements = [element(
            ElementKind::Buttons,
            vec![shape(square(0.0, 0.0, 250.0), true)],
        )];
        let config: TemplateConfig = serde_json::from_str(r#"{"page": {"size": "A4"}}"#).unwrap();
        let found = problems(&elements, &config);
        assert_eq!(found.len(), 2, "{found:#?}");
        // A4 either way holds 200 x 290.
        let fits = [element(
            ElementKind::Outline,
            vec![shape(square(0.0, 0.0, 200.0), true)],
        )];
        assert_eq!(problems(&fits, &config), vec![]);
    }

    #[test]
    fn the_provided_templates_are_valid() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        for entry in template::scan(&dir) {
            let found = check_entry(&entry);
            assert!(!has_errors(&found), "{}: {}", entry.id, summary(&found));
        }
    }

    #[test]
    fn a_broken_template_folder_is_told_what_is_wrong() {
        let dir = std::env::temp_dir().join(format!("artwork-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("broken");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(template::CONFIG_FILE),
            r##"{"name": "Broken", "elements": [
                {"kind": "outline", "id": "outline"},
                {"kind": "buttons", "id": "buttons", "color": "red"},
                {"kind": "cutouts", "id": "buttons"},
                {"kind": "cutouts", "id": "gone"}
            ]}"##,
        )
        .unwrap();
        std::fs::write(
            folder.join(crate::template_svg::SVG_FILE),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">
                <g id="outline"><rect x="0" y="0" width="100" height="100"/></g>
                <g id="buttons"><circle cx="50" cy="50" r="10"/></g>
                <rect x="10" y="10" width="5" height="5"/>
            </svg>"#,
        )
        .unwrap();
        let entry = template::read_entry(&folder).unwrap();
        let found = check_entry(&entry);
        let _ = std::fs::remove_dir_all(&dir);
        let text = summary(&found);
        // The color, the group named twice (its holes on themselves), the group that
        // isn't there (an empty element), the rectangle in no element.
        assert!(text.contains("\"red\""), "{text}");
        assert!(text.contains("\"buttons\""), "{text}");
        assert!(text.contains("\"gone\""), "{text}");
        assert_eq!(
            found
                .iter()
                .filter(|p| p.severity == Severity::Warning)
                .count(),
            2,
            "{text}"
        );
        assert!(has_errors(&found));
    }

    #[test]
    fn the_editor_finds_the_same_as_the_files() {
        let entry = template::tests::fixture();
        let doc = crate::template_edit::open(&entry, entry.config.clone()).unwrap();
        let found = check_document(&doc);
        assert!(!has_errors(&found), "{}", summary(&found));
    }
}
