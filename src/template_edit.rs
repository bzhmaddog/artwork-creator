//! The template editor's document: a template opened as groups of shape layers, one
//! group per element, and saved back as its `template.svg` (one group per element) and
//! `template.json`. It is shown
//! centered on the page its `template.json` asks for, as a project shows it; its settings
//! are kept in the document (`Document::edited_template`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::document::{Document, Group, Layer, Node, Orientation, P2, PageSize};
use crate::i18n::{tr, translate};
use crate::shape::{self, DashStyle, Shape, ShapeKind};
use crate::template::{
    self, CONFIG_FILE, ElementConfig, ElementKind, ElementTag, TemplateConfig, TemplateEntry,
};
use crate::template_geom::{self, Frame, Made};
use crate::template_svg::{self, SVG_FILE};

/// Room around the template on the editor's page, in mm.
const MARGIN_MM: f64 = 20.0;
/// The page of a new template: A3 landscape.
const NEW_PAGE_MM: P2 = [420.0, 297.0];
/// Width of the lines shapes are drawn with in the editor, in mm (the compared source's
/// too).
pub const LINE_WIDTH_MM: f32 = 0.3;

/// What the editor's document holds of the template besides its shapes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditedTemplate {
    /// Its `template.json` settings, written with its elements.
    pub config: TemplateConfig,
    /// The size of its frame in mm, as opened: the shapes are placed on the page from
    /// that frame (its top-left corner at 0, y down) as `Document::template_affine` says.
    pub size_mm: P2,
}

/// The template's settings in the editor's document.
pub fn config(doc: &Document) -> Option<&TemplateConfig> {
    doc.edited_template.as_deref().map(|e| &e.config)
}

/// Where the top-left corner of the template's frame is on the page: the template is
/// centered on it.
fn frame_origin(doc: &Document) -> P2 {
    match &doc.edited_template {
        Some(e) => doc.template_affine(e.size_mm).apply([0.0, 0.0]),
        None => [0.0, 0.0],
    }
}

/// Moves every layer by `d` mm.
fn move_layers(doc: &mut Document, d: P2) {
    doc.for_each_layer_mut(|l| {
        let c = &mut l.transform.center;
        *c = [c[0] + d[0], c[1] + d[1]];
    });
}

/// Sets the page the template's settings ask for (what they leave out: a page around the
/// template, as wide as it is) and moves the shapes, then in the template's frame, onto
/// its middle.
fn place(doc: &mut Document) {
    let Some(edited) = doc.edited_template.clone() else {
        return;
    };
    let size = edited.size_mm;
    set_page(
        doc,
        [
            size[0] + 2.0 * MARGIN_MM as f32,
            size[1] + 2.0 * MARGIN_MM as f32,
        ]
        .map(|v| v.max(50.0)),
    );
    if let Some(Ok(setup)) = edited.config.page.as_ref().map(|p| p.setup()) {
        if let Some(p) = setup.size {
            doc.page = p;
        }
        if let Some(mm) = setup.custom_mm {
            doc.custom_page_mm = mm;
        }
        if let Some(o) = setup.orientation {
            doc.orientation = o;
        }
    }
    move_layers(doc, frame_origin(doc));
}

/// Changes the template's settings; a new page moves the shapes with the template, kept
/// in its middle.
pub fn set_config(doc: &mut Document, config: TemplateConfig) {
    let [x, y] = frame_origin(doc);
    move_layers(doc, [-x, -y]);
    if let Some(e) = &mut doc.edited_template {
        e.config = config;
    }
    place(doc);
    // Linked positions move with the shapes.
    let [nx, ny] = frame_origin(doc);
    crate::variables::shift_positions(doc, [nx - x, ny - y]);
}

/// A group standing for an element of `kind`, named `name`.
fn element_group(doc: &mut Document, kind: ElementKind, name: String, color: [u8; 3]) -> Group {
    Group {
        id: doc.alloc_id(),
        name,
        visible: true,
        locked: false,
        opacity: 1.0,
        expanded: true,
        children: Vec::new(),
        element: Some(ElementTag { kind, color }),
    }
}

/// Sets the page to a custom size of `size` mm, as wide as it says.
fn set_page(doc: &mut Document, size: P2) {
    doc.page = PageSize::Custom;
    doc.custom_page_mm = [size[0].min(size[1]), size[0].max(size[1])];
    doc.orientation = if size[0] >= size[1] {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    };
}

/// A new template's document, of settings `config`: an empty outline element, on an A3
/// page unless they say otherwise.
pub fn new_document(config: TemplateConfig) -> Document {
    let mut doc = Document::default();
    let margin = 2.0 * MARGIN_MM as f32;
    doc.edited_template = Some(Box::new(EditedTemplate {
        config,
        size_mm: [NEW_PAGE_MM[0] - margin, NEW_PAGE_MM[1] - margin],
    }));
    place(&mut doc);
    let kind = ElementKind::Outline;
    let g = element_group(
        &mut doc,
        kind,
        translate(kind.label()).to_owned(),
        kind.default_color(),
    );
    doc.layers.push(Node::Group(g));
    doc
}

/// A shape layer for what the editor made of a DXF item.
fn made_layer(doc: &mut Document, made: Made) -> Layer {
    let id = doc.alloc_id();
    match made {
        Made::Circle { center, diameter } => {
            let mut s = Shape::new(ShapeKind::Ellipse);
            s.size = [diameter, diameter];
            let name = tr!("TEMPLATE_CIRCLE_NAME", d = format!("{diameter:.1}"));
            shape::new_shape_layer(id, name, s, center)
        }
        Made::Rect {
            center,
            size,
            rotation,
            radius,
        } => {
            let mut s = Shape::new(ShapeKind::Rectangle);
            s.size = size;
            s.corner_radius = radius;
            let name = translate(ShapeKind::Rectangle.name_key()).to_owned();
            let mut layer = shape::new_shape_layer(id, name, s, center);
            layer.transform.rotation_deg = rotation;
            layer
        }
        Made::Path { shape, at } => {
            let name = translate(ShapeKind::Path.name_key()).to_owned();
            shape::new_shape_layer(id, name, *shape, at)
        }
    }
}

/// A template opened for editing, with settings `config` (its own, or renamed for a
/// copy): one group per element, in `template.json` order from the top of the layer list,
/// holding its circles, rectangles and paths, on its page.
pub fn open(entry: &TemplateEntry, config: TemplateConfig) -> Result<Document, String> {
    let units = match &entry.config.units {
        Some(u) => Some(template::units_override(u).ok_or_else(|| {
            tr!(
                "TEMPLATE_UNKNOWN_UNITS",
                u = u,
                path = entry.config_path().display()
            )
        })?),
        None => None,
    };
    // Each item with the id of its SVG shape, which its linked fields are listed by.
    let mut read = Vec::new();
    for src in &entry.sources {
        let items = if src.is_svg() {
            template_svg::read_with_ids(&src.path, src.group.as_deref())?
        } else {
            template_geom::read(&src.path, units)?
                .into_iter()
                .map(|i| (None, i))
                .collect()
        };
        read.push((src, items));
    }
    // Shapes of the SVG in no element: shown apart, to be moved into one or deleted.
    // (A template whose SVG is its single element has none.)
    let mut orphans = Vec::new();
    let svg = entry.sources.iter().find(|s| s.is_svg());
    if let Some(svg) = svg
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
        orphans = template_svg::read_orphans(&svg.path, &ids)?;
    }
    // The frame is the elements' alone, as `template::load` makes it.
    let bounds =
        template_geom::items_bounds(read.iter().flat_map(|(_, i)| i.iter().map(|(_, i)| i)))
            .or_else(|| template_geom::items_bounds(&orphans))
            .unwrap_or(([0.0; 2], [100.0, 100.0]));
    // In the template's frame first, as `template::load` makes it.
    let frame = Frame::for_bounds(bounds, 0.0);
    let mut doc = Document::default();
    let size = [
        (bounds.1[0] - bounds.0[0]) as f32,
        (bounds.1[1] - bounds.0[1]) as f32,
    ];
    // The first element at the top of the list: the last in the stack.
    for (src, items) in read.into_iter().rev() {
        let color = src.color.unwrap_or_else(|| src.kind.default_color());
        let mut g = element_group(&mut doc, src.kind, translate(&src.name).to_owned(), color);
        for (id, item) in &items {
            if let Some(made) = template_geom::make(item, frame) {
                let mut layer = made_layer(&mut doc, made);
                if let Some(b) = id.as_ref().and_then(|id| config.bindings.get(id)) {
                    layer.bindings = b.clone();
                }
                g.children.push(Node::Layer(Box::new(layer)));
            }
        }
        doc.layers.push(Node::Group(g));
    }
    // At the top of the list, in sight.
    for item in &orphans {
        if let Some(made) = template_geom::make(item, frame) {
            let layer = made_layer(&mut doc, made);
            doc.layers.push(Node::Layer(Box::new(layer)));
        }
    }
    // The variables and links live in the document while it is edited.
    let mut config = config;
    doc.variables = std::mem::take(&mut config.variables);
    config.bindings.clear();
    let saved_origin = config.bindings_origin.take();
    doc.edited_template = Some(Box::new(EditedTemplate {
        config,
        size_mm: size,
    }));
    place(&mut doc);
    // Linked positions are page positions as they were saved: moved by as much as the
    // SVG's top-left corner is now elsewhere on the page (the SVG's own (0, 0) is
    // (-lo x, hi y) in the frame).
    if let Some([ox, oy]) = saved_origin {
        let [fx, fy] = frame_origin(&doc);
        let now = [fx - bounds.0[0] as f32, fy + bounds.1[1] as f32];
        crate::variables::shift_positions(&mut doc, [now[0] - ox, now[1] - oy]);
    }
    sync(&mut doc);
    // Laid out as the canvas lays them out, with their lines' width: the document opened
    // is the one saved, not changed by the first frame.
    doc.for_each_layer_mut(|l| {
        if l.shape.is_some() {
            shape::layout(l, true);
        }
    });
    Ok(doc)
}

/// The shape layers of a node, in stack order.
fn layers_of<'a>(node: &'a Node, out: &mut Vec<&'a Layer>) {
    match node {
        Node::Layer(l) => out.push(l),
        Node::Group(g) => g.children.iter().for_each(|c| layers_of(c, out)),
    }
}

/// An SVG group id for an element named `name`, not among `taken`: lower case letters,
/// digits and dashes, from a letter.
fn group_id(name: &str, taken: &[String]) -> String {
    let slug: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = match slug.chars().next() {
        None => "element".to_owned(),
        Some(c) if c.is_ascii_alphabetic() => slug,
        Some(_) => format!("element-{slug}"),
    };
    let free = |n: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if free(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base}-{i}"))
        .find(|n| free(n))
        .expect("some number is free")
}

/// Writes the document into the template folder `folder`: `template.svg`, a group per
/// element (in mm, in the template's frame), and `template.json`, its settings. The DXF
/// files the old `template.json` listed (an older template's) are deleted. Returns the new
/// settings.
pub fn save(doc: &Document, folder: &Path) -> Result<TemplateConfig, String> {
    let config = config(doc).cloned().unwrap_or_default();
    let config = &config;
    match config.version.as_deref().map(str::trim) {
        None | Some("") => return Err(tr!("TEMPLATE_VERSION_REQUIRED").to_owned()),
        Some(v) if !template::is_valid_version(v) => {
            return Err(tr!("TEMPLATE_VERSION_INVALID", version = v));
        }
        Some(_) => {}
    }
    let variables = doc.variables.clone();
    let mut doc = doc.clone();
    let [x, y] = frame_origin(&doc);
    move_layers(&mut doc, [-x, -y]);
    let doc = &doc;
    // Every layer must be a shape in an element.
    let mut stray = Vec::new();
    let mut elements: Vec<(&Group, Vec<&Layer>)> = Vec::new();
    for node in doc.layers.iter().rev() {
        let mut layers = Vec::new();
        layers_of(node, &mut layers);
        match node {
            Node::Group(g) if g.element.is_some() => {
                stray.extend(
                    layers
                        .iter()
                        .filter(|l| l.shape.is_none())
                        .map(|l| l.name.clone()),
                );
                elements.push((g, layers));
            }
            _ => stray.extend(layers.iter().map(|l| l.name.clone())),
        }
    }
    if !stray.is_empty() {
        return Err(tr!("TEMPLATE_STRAY_LAYERS", names = stray.join(", ")));
    }
    let has_lines = elements
        .iter()
        .any(|(_, layers)| layers.iter().any(|l| l.shape.is_some()));
    if !has_lines {
        return Err(tr!("TEMPLATE_HAS_NO_LINES").to_owned());
    }
    std::fs::create_dir_all(folder)
        .map_err(|e| tr!("CANNOT_WRITE_FILE", path = folder.display(), e = e))?;
    let mut configs = Vec::new();
    let mut ids: Vec<String> = Vec::new();
    let mut drawn = Vec::new();
    let mut bindings = std::collections::BTreeMap::new();
    for (g, layers) in elements {
        let Some(tag) = g.element else { continue };
        let id = group_id(&g.name, &ids);
        ids.push(id.clone());
        let [r, gr, b] = tag.color;
        configs.push(ElementConfig {
            kind: tag.kind,
            id: Some(id.clone()),
            dxf: None,
            name: Some(g.name.clone()),
            color: (tag.color != tag.kind.default_color())
                .then(|| format!("#{r:02x}{gr:02x}{b:02x}")),
        });
        // A shape with linked fields gets an id, its links listed by it; one drawn in
        // several pieces (a path of several outlines) can't be told apart from them once
        // read again, and loses its links.
        let mut items = Vec::new();
        let mut shape_ids = Vec::new();
        for l in &layers {
            let mine = template_geom::layer_items(l);
            let shape_id = (!l.bindings.is_empty() && mine.len() == 1).then(|| {
                let shape_id = format!("{id}-{}", bindings.len() + 1);
                bindings.insert(shape_id.clone(), l.bindings.clone());
                shape_id
            });
            shape_ids.extend(std::iter::repeat_n(shape_id, mine.len()));
            items.extend(mine);
        }
        drawn.push((g, tag, items, shape_ids));
    }
    // Where the SVG's top-left corner (that of all its items) is on the page.
    let origin = template_geom::items_bounds(drawn.iter().flat_map(|(_, _, i, _)| i))
        .map(|(lo, hi)| [x + lo[0] as f32, y - hi[1] as f32]);
    let svg: Vec<template_svg::Element> = drawn
        .into_iter()
        .zip(&ids)
        .map(|((g, tag, items, shape_ids), id)| template_svg::Element {
            id,
            name: &g.name,
            color: tag.color,
            items,
            ids: shape_ids,
        })
        .collect();
    template_svg::write(&folder.join(SVG_FILE), &svg)?;
    let mut new = config.clone();
    // The SVG is in mm.
    new.units = None;
    new.dxf = None;
    new.elements = configs;
    new.variables = variables;
    new.bindings_origin = (!bindings.is_empty()).then_some(origin).flatten();
    new.bindings = bindings;
    let json = serde_json::to_string_pretty(&new).map_err(|e| e.to_string())?;
    let path = folder.join(CONFIG_FILE);
    std::fs::write(&path, json + "\n")
        .map_err(|e| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e))?;
    // The DXF files of an older template, now in the SVG; only plain names in the
    // folder.
    let old = config
        .elements
        .iter()
        .filter_map(|e| e.dxf.clone())
        .chain(config.dxf.clone());
    for f in old {
        let p = Path::new(&f);
        if p.components().count() == 1
            && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dxf"))
        {
            let _ = std::fs::remove_file(folder.join(p));
        }
    }
    // The variables and links stay in the document.
    new.variables.clear();
    new.bindings.clear();
    new.bindings_origin = None;
    Ok(new)
}

/// Line color of the shapes in no element: an error, shown in red.
pub const ORPHAN_COLOR: [u8; 3] = [220, 40, 40];

/// Gives a layer the look of a template shape: lines of `color`, no fill, no paint, no
/// effects. Returns true if something changed.
fn give_look(l: &mut Layer, [r, g, b]: [u8; 3]) -> bool {
    let mut changed = false;
    if !l.effects.is_empty() || l.opacity != 1.0 {
        l.effects.clear();
        l.opacity = 1.0;
        changed = true;
    }
    let Some(s) = l.shape.as_deref_mut() else {
        return changed;
    };
    let color = [r, g, b, 255];
    let look = (
        s.fill,
        s.stroke,
        s.stroke_color,
        s.stroke_width,
        s.dash,
        s.fill_gradient.is_some() || s.stroke_gradient.is_some(),
        s.strokes.is_empty(),
    );
    let want = (
        false,
        true,
        color,
        LINE_WIDTH_MM,
        DashStyle::Solid,
        false,
        true,
    );
    if look != want {
        s.fill = false;
        s.stroke = true;
        s.stroke_color = color;
        s.stroke_width = LINE_WIDTH_MM;
        s.dash = DashStyle::Solid;
        s.fill_gradient = None;
        s.stroke_gradient = None;
        s.strokes.clear();
        changed = true;
    }
    changed
}

/// Gives the element shapes the look of their element: lines of its color, no fill, no
/// paint, no effects; shapes in no element get red lines. Returns true if something
/// changed.
pub fn sync(doc: &mut Document) -> bool {
    let mut changed = false;
    for node in &mut doc.layers {
        let color = match node {
            Node::Group(g) => g.element.map_or(ORPHAN_COLOR, |t| t.color),
            Node::Layer(_) => ORPHAN_COLOR,
        };
        node.for_each_layer_mut(&mut |l| changed |= give_look(l, color));
    }
    changed
}

/// Whether node `id` is a shape in no element of the template being edited: at the top
/// of the layer list without being an element.
pub fn is_orphan(doc: &Document, id: u64) -> bool {
    doc.edited_template.is_some()
        && doc
            .layers
            .iter()
            .any(|n| n.id() == id && n.as_group().is_none_or(|g| g.element.is_none()))
}

/// Whether node `id` may sit in the list at `parent` (the path of its group, empty for
/// the top of the list): an element only at the top, anything else only inside an
/// element. Shapes thus move from an element to another, never out of them.
pub fn fits_in(doc: &Document, id: u64, parent: &[usize]) -> bool {
    let is_element = doc
        .node(id)
        .and_then(Node::as_group)
        .is_some_and(|g| g.element.is_some());
    match parent.first() {
        None => is_element,
        Some(&i) => {
            !is_element
                && doc
                    .layers
                    .get(i)
                    .and_then(Node::as_group)
                    .is_some_and(|g| g.element.is_some())
        }
    }
}

/// Adds an element of `kind` at the top of the layer list, named after its kind, and
/// returns its id.
pub fn add_element(doc: &mut Document, kind: ElementKind) -> u64 {
    let g = element_group(
        doc,
        kind,
        translate(kind.label()).to_owned(),
        kind.default_color(),
    );
    let id = g.id;
    doc.layers.push(Node::Group(g));
    id
}

/// The centers of a shape's closed outlines on the page, as a project's template gives
/// them (images snap to them): crosses mark them in the editor.
pub fn layer_centers(layer: &Layer) -> Vec<P2> {
    let items = template_geom::layer_items(layer);
    template::centers(&template::item_paths(&items))
        .into_iter()
        // Items are y up.
        .map(|[x, y]| [x as f32, -y as f32])
        .collect()
}

/// The element a node is in, or is: the kind of its top-level group.
pub fn element_of(doc: &Document, id: u64) -> Option<ElementTag> {
    let path = doc.path_of(id)?;
    doc.layers.get(*path.first()?)?.as_group()?.element
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_saved_from_the_editor_opens_the_same() {
        let source = crate::template::tests::fixture();
        let doc = open(&source, source.config.clone()).unwrap();
        // One group per element, the first at the top of the list.
        let groups: Vec<(&str, ElementKind, usize)> = doc
            .layers
            .iter()
            .rev()
            .filter_map(|n| n.as_group())
            .map(|g| (g.name.as_str(), g.element.unwrap().kind, g.children.len()))
            .collect();
        assert_eq!(groups[0].1, ElementKind::Outline);
        assert_eq!(groups.len(), source.sources.len());

        let dir = std::env::temp_dir().join(format!("artwork-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = save(&doc, &dir).unwrap();
        assert_eq!(config.elements.len(), groups.len());
        let saved = crate::template::read_entry(&dir).unwrap();
        // All in template.svg, each element a group of it.
        assert!(
            saved
                .sources
                .iter()
                .all(|s| s.is_svg() && s.group.is_some())
        );
        assert!(dir.join(SVG_FILE).is_file());
        let (a, b) = (
            crate::template::load(&source).unwrap(),
            crate::template::load(&saved).unwrap(),
        );
        // Same size, same shapes per element, same centers.
        assert!(
            (a.size_mm[0] - b.size_mm[0]).abs() < 0.05,
            "{:?} {:?}",
            a.size_mm,
            b.size_mm
        );
        assert!((a.size_mm[1] - b.size_mm[1]).abs() < 0.05);
        for (x, y) in a.elements.iter().zip(&b.elements) {
            assert_eq!(
                (x.kind, x.polylines.len(), x.centers.len()),
                (y.kind, y.polylines.len(), y.centers.len()),
                "{}",
                x.name
            );
        }
        // Opened again, the same groups and shapes.
        let again = open(&saved, saved.config.clone()).unwrap();
        let count = |d: &Document| d.leaves().len();
        assert_eq!(count(&doc), count(&again));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn provided_templates_round_trip() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        for source in crate::template::scan(&dir) {
            let doc = open(&source, source.config.clone()).unwrap();
            let out = std::env::temp_dir().join(format!(
                "artwork-rt-{}-{}",
                std::process::id(),
                source.id
            ));
            save(&doc, &out).unwrap();
            let saved = crate::template::read_entry(&out).unwrap();
            let (a, b) = (
                crate::template::load(&source).unwrap(),
                crate::template::load(&saved).unwrap(),
            );
            let d = |i: usize| (a.size_mm[i] - b.size_mm[i]).abs();
            assert!(
                d(0) < 0.05 && d(1) < 0.05,
                "{}: {:?} {:?}",
                source.id,
                a.size_mm,
                b.size_mm
            );
            for (x, y) in a.elements.iter().zip(&b.elements) {
                assert_eq!(
                    (x.polylines.len(), x.centers.len()),
                    (y.polylines.len(), y.centers.len()),
                    "{} {}",
                    source.id,
                    x.name
                );
            }
            std::fs::remove_dir_all(&out).ok();
        }
    }

    #[test]
    fn a_template_opened_is_not_changed_by_the_first_frames() {
        // What the app does to the document each frame: it must find nothing to change
        // in a template just opened, or it would count as an unsaved change.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let ctx = egui::Context::default();
        for entry in crate::template::scan(&dir) {
            let mut doc = open(&entry, entry.config.clone()).unwrap();
            let before = serde_json::to_string(&doc).unwrap();
            let mut applied = crate::variables::Applied::new();
            for _ in 0..2 {
                crate::variables::apply(&mut doc, &mut applied);
                crate::colors::apply(&mut doc);
                crate::text_links::apply(&mut doc);
                crate::shape::sync(&ctx, &mut doc, true);
                sync(&mut doc);
            }
            assert_eq!(serde_json::to_string(&doc).unwrap(), before, "{}", entry.id);
        }
    }

    #[test]
    fn variables_and_links_are_saved_and_follow_the_page() {
        use crate::variables::{self, Field, Value, Variable};
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let source = crate::template::scan(&dir)
            .into_iter()
            .find(|t| t.id.contains("T16"))
            .expect("Cosmox T16");
        let mut doc = open(&source, source.config.clone()).unwrap();
        // A button linked to two variables: its x and its width.
        let button = doc
            .leaves()
            .iter()
            .find(|l| element_of(&doc, l.layer.id).is_some_and(|t| t.kind == ElementKind::Buttons))
            .map(|l| (l.layer.id, l.layer.transform.center[0]))
            .unwrap();
        doc.variables = vec![
            Variable {
                formula: None,
                name: "col".into(),
                value: Value::Length(button.1 + 5.0),
            },
            Variable {
                formula: None,
                name: "d".into(),
                value: Value::Length(20.0),
            },
        ];
        doc.for_each_layer_mut(|l| {
            if l.id == button.0 {
                l.bindings.insert(Field::X, "col".into());
                l.bindings.insert(Field::Width, "d".into());
            }
        });
        let mut applied = variables::Applied::new();
        assert!(variables::apply(&mut doc, &mut applied));
        let out = std::env::temp_dir().join(format!("artwork-vars-{}", std::process::id()));
        let config = save(&doc, &out).unwrap();
        // Kept in the document, not in its settings.
        assert!(config.variables.is_empty() && config.bindings.is_empty());
        let mut saved = crate::template::read_entry(&out).unwrap();
        assert_eq!(saved.config.variables.len(), 2);
        assert_eq!(saved.config.bindings.len(), 1);
        assert!(saved.config.bindings_origin.is_some());

        // Opened on a wider page: the link moved with the shape.
        saved.config.page = Some(crate::template::PageConfig {
            size: Some(crate::template::PageSizeConfig::Named("A3".into())),
            orientation: Some("landscape".into()),
        });
        let mut again = open(&saved, saved.config.clone()).unwrap();
        assert_eq!(again.variables, doc.variables);
        let linked: Vec<&Layer> = again
            .leaves()
            .into_iter()
            .map(|l| l.layer)
            .filter(|l| !l.bindings.is_empty())
            .collect();
        assert_eq!(linked.len(), 1);
        let (id, x) = (linked[0].id, linked[0].transform.center[0]);
        assert_eq!(linked[0].bindings[&Field::Width], "d");
        assert_ne!(linked[0].bindings[&Field::X], "col");
        let mut applied = variables::Applied::new();
        variables::apply(&mut again, &mut applied);
        let after = again
            .leaves()
            .into_iter()
            .find(|l| l.layer.id == id)
            .unwrap();
        assert!((after.layer.transform.center[0] - x).abs() < 0.01);
        assert!(after.layer.bindings.contains_key(&Field::X));
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn a_template_opens_centered_on_its_page_and_saves_as_it_was() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let source = crate::template::scan(&dir)
            .into_iter()
            .find(|t| t.id.contains("T16"))
            .expect("Cosmox T16");
        let mut doc = open(&source, source.config.clone()).unwrap();
        // A4 landscape, as its template.json says.
        assert_eq!(doc.page, PageSize::A4);
        assert_eq!(doc.orientation, Orientation::Landscape);
        // On another page, the shapes move with the template, kept in its middle.
        let mut config = source.config.clone();
        config.page = Some(crate::template::PageConfig {
            size: Some(crate::template::PageSizeConfig::Named("A3".into())),
            orientation: Some("portrait".into()),
        });
        set_config(&mut doc, config);
        assert_eq!(doc.page, PageSize::A3);
        let out = std::env::temp_dir().join(format!("artwork-placed-{}", std::process::id()));
        save(&doc, &out).unwrap();
        let saved = crate::template::read_entry(&out).unwrap();
        let (a, b) = (
            crate::template::load(&source).unwrap(),
            crate::template::load(&saved).unwrap(),
        );
        // The same centers in the template's frame.
        let centers = |t: &crate::template::Template| {
            let mut v: Vec<(i32, i32)> = t
                .elements
                .iter()
                .flat_map(|e| &e.centers)
                .map(|p| ((p[0] * 10.0).round() as i32, (p[1] * 10.0).round() as i32))
                .collect();
            v.sort();
            v
        };
        let (x, y) = (centers(&a), centers(&b));
        assert!(!x.is_empty());
        assert_eq!(x.len(), y.len());
        for (p, q) in x.iter().zip(&y) {
            assert!(
                (p.0 - q.0).abs() <= 1 && (p.1 - q.1).abs() <= 1,
                "{p:?} {q:?}"
            );
        }
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn shapes_in_no_element_are_shown_apart_and_left_out_of_projects() {
        let dir = std::env::temp_dir().join(format!("artwork-orphans-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{ "elements": [ { "kind": "outline", "id": "outline" } ] }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join(SVG_FILE),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
                 <g id="outline"><rect x="0" y="0" width="100" height="50"/></g>
                 <g id="unknown"><circle cx="20" cy="20" r="5"/></g>
                 <circle cx="60" cy="20" r="5"/>
               </svg>"#,
        )
        .unwrap();
        let entry = crate::template::read_entry(&dir).unwrap();
        // Projects: the outline alone, its frame its own.
        let t = crate::template::load(&entry).unwrap();
        assert_eq!(t.elements.len(), 1);
        assert_eq!(t.elements[0].polylines.len(), 1);
        assert!((t.size_mm[0] - 100.0).abs() < 0.05);
        // The editor: the element, then the two shapes in none at the top of the list.
        let mut doc = open(&entry, entry.config.clone()).unwrap();
        assert_eq!(doc.layers.len(), 3);
        let element = doc.layers[0].id();
        let orphan = doc.layers[2].id();
        assert!(doc.layers[1].as_layer().is_some() && doc.layers[2].as_layer().is_some());
        sync(&mut doc);
        let red = |d: &Document, id: u64| {
            d.node(id)
                .unwrap()
                .as_layer()
                .unwrap()
                .shape
                .as_ref()
                .unwrap()
                .stroke_color
        };
        let [r, g, b] = ORPHAN_COLOR;
        assert_eq!(red(&doc, orphan), [r, g, b, 255]);
        assert!(is_orphan(&doc, orphan) && !is_orphan(&doc, element));
        // Saving is refused while they are in no element.
        assert!(save(&doc, &dir.join("out")).is_err());
        // Into an element, never back out; the element stays at the top.
        use crate::layers_ui::{DropTarget, drop_allowed};
        assert!(drop_allowed(&doc, orphan, DropTarget::Into(element), true));
        assert!(!drop_allowed(
            &doc,
            element,
            DropTarget::Into(element),
            true
        ));
        crate::layers_ui::drop_node(&mut doc, orphan, DropTarget::Into(element));
        let shape = doc.layers[0].as_group().unwrap().children[0].id();
        assert!(!is_orphan(&doc, orphan));
        assert!(!drop_allowed(
            &doc,
            orphan,
            DropTarget::Above(element),
            true
        ));
        assert!(!drop_allowed(&doc, shape, DropTarget::Below(element), true));
        assert!(drop_allowed(&doc, shape, DropTarget::Above(orphan), true));
        assert!(drop_allowed(
            &doc,
            element,
            DropTarget::Above(doc.layers[1].id()),
            true
        ));
        sync(&mut doc);
        // In its element's color now.
        let [r, g, b] = doc.layers[0].as_group().unwrap().element.unwrap().color;
        assert_eq!(red(&doc, orphan), [r, g, b, 255]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn layers_outside_elements_are_refused() {
        let mut doc = new_document(TemplateConfig::default());
        let id = doc.alloc_id();
        let layer = shape::new_layer(id, ShapeKind::Rectangle, [50.0, 50.0]);
        doc.layers.push(Node::Layer(Box::new(layer)));
        let dir = std::env::temp_dir();
        assert!(save(&doc, &dir.join("artwork-never-written")).is_err());
    }
}

#[cfg(test)]
mod previews {
    use super::*;
    use std::cell::RefCell;

    /// The Cosmox T16 opened in the editor: its layer list and the canvas, in both
    /// themes: `TEMPLATE_EDITOR_PREVIEW=out.png cargo test template_editor_preview --
    /// --ignored`.
    #[test]
    #[ignore]
    fn template_editor_preview() {
        let Ok(path) = std::env::var("TEMPLATE_EDITOR_PREVIEW") else {
            return;
        };
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let entry = crate::template::scan(&dir)
            .into_iter()
            .find(|t| t.id.contains("T16"))
            .expect("Cosmox T16");
        let doc = RefCell::new(open(&entry, entry.config.clone()).unwrap());
        let state = RefCell::new(crate::view::CanvasState::default());
        let panel = RefCell::new(crate::layers_ui::LayersPanel {
            template_mode: true,
            ..Default::default()
        });
        let last: RefCell<Option<egui::Context>> = Default::default();
        crate::preview::render(&path, egui::vec2(1100.0, 700.0), |ui| {
            let mut doc = doc.borrow_mut();
            let mut state = state.borrow_mut();
            if last.borrow().as_ref() != Some(ui.ctx()) {
                doc.for_each_layer_mut(|l| l.pixmap = None);
                *state = crate::view::CanvasState::default();
                state.fit_pending = true;
                *last.borrow_mut() = Some(ui.ctx().clone());
            }
            crate::shape::sync(ui.ctx(), &mut doc, true);
            let mut sel = doc.layers.last().map(|n| n.id());
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    ui.set_height(680.0);
                    crate::layers_ui::show(ui, &mut doc, &mut sel, &mut panel.borrow_mut());
                });
                ui.vertical(|ui| {
                    ui.set_width(780.0);
                    ui.set_height(680.0);
                    crate::view::show(ui, &mut doc, &[], &mut sel, &mut state);
                });
            });
        });
    }
}
