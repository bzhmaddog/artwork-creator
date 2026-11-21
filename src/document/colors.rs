//! The project's colors, to reuse them: those in use, gathered from the document, and
//! its color variables (see `variables`), each with its own name.
//!
//! Every color button of the app is `ui::colors::button`: its picker shows both lists as
//! swatches, and a click on one applies it. A click on a color variable also links the
//! button's color to it, by name, like a field linked to a variable: each frame, [`apply`]
//! gives every linked color its variable's value, so changing a color variable changes
//! all the colors using it. A color changed some other way in the picker is unlinked.
//! The buttons and pickers are in `ui::colors`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::document::variables::{self, Kind, Value, Variable};
use crate::document::{Document, Node};
use crate::render::effects::{ParamKind, Params, color_from_hex, color_to_hex, registry};

/// A saved color of the first projects that had them, before color variables: read,
/// then made a color variable ([`migrate_palette`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OldSwatch {
    /// Straight RGBA.
    pub color: [u8; 4],
    #[serde(default)]
    pub name: String,
}

/// A layer's color that can be linked to a color variable. An effect's color parameters
/// are linked in the effect itself (`EffectInstance::links`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorField {
    /// A text's color.
    Text,
    /// A text's outline color.
    TextOutline,
    /// A shape's fill.
    Fill,
    /// A shape's stroke.
    Stroke,
}

/// Names of the color variables a layer's colors are linked to.
pub type Links = BTreeMap<ColorField, String>;

/// Every color the project uses, once each, in stack order: the page background, then
/// for each layer from the bottom its text, outline, fill and stroke colors and those of
/// its effects. A gradient counts with the colors of its stops. Colors switched off (no
/// fill, no outline) don't count.
pub fn used(doc: &Document) -> Vec<[u8; 4]> {
    let mut colors = Vec::new();
    let mut add = |c: [u8; 4]| {
        if !colors.contains(&c) {
            colors.push(c);
        }
    };
    // One color, or a gradient's stops.
    fn either(
        add: &mut dyn FnMut([u8; 4]),
        c: [u8; 4],
        g: Option<&crate::document::gradient::Gradient>,
    ) {
        match g {
            Some(g) => g.sorted().iter().for_each(|s| add(s.color)),
            None => add(c),
        }
    }
    let [r, g, b] = doc.background;
    // The page is opaque, and so are its gradient's colors.
    let bg = doc.background_gradient.as_ref().map(|g| {
        let mut g = g.clone();
        g.stops.iter_mut().for_each(|s| s.color[3] = 255);
        g
    });
    either(&mut add, [r, g, b, 255], bg.as_ref());
    fn walk(nodes: &[Node], add: &mut dyn FnMut([u8; 4])) {
        for node in nodes {
            match node {
                Node::Group(g) => walk(&g.children, add),
                Node::Layer(l) => {
                    if let Some(t) = &l.text {
                        either(add, t.color, t.gradient.as_ref());
                        if t.outline {
                            add(t.outline_color);
                        }
                    }
                    if let Some(s) = &l.shape {
                        if s.fill && s.kind.can_fill() {
                            either(add, s.fill_color, s.fill_gradient.as_ref());
                        }
                        if s.stroke {
                            either(add, s.stroke_color, s.stroke_gradient.as_ref());
                        }
                    }
                    for fx in l.param_sets() {
                        let Some(specs) = registry().params_of(&fx.effect) else {
                            continue;
                        };
                        let params = Params::new(specs, &fx.params);
                        for spec in specs {
                            if let ParamKind::Color { .. } = spec.kind {
                                add(params.color(&spec.id));
                            }
                        }
                    }
                }
            }
        }
    }
    walk(&doc.layers, &mut add);
    colors
}

/// The colors in use but `picked`, the colors added with the eyedropper, shown before
/// them.
pub fn in_use(doc: &Document, picked: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let mut used = used(doc);
    used.retain(|u| !picked.contains(u));
    used
}

/// Adds the color variable `var` unless a color variable has its color or a variable
/// its name already. Returns true if it was added.
pub fn save(doc: &mut Document, var: Variable) -> bool {
    let color = var.value.color();
    if color.is_none()
        || doc
            .variables
            .iter()
            .any(|v| v.name == var.name || (v.value.color().is_some() && v.value.color() == color))
    {
        return false;
    }
    doc.variables.push(var);
    true
}

/// The color variable `name`'s color, if there is one.
fn find(vars: &[Variable], name: &str) -> Option<[u8; 4]> {
    vars.iter().find(|v| v.name == name)?.value.color()
}

/// Makes the saved colors of a project from before color variables color variables.
/// A name that can't be a variable's (such as "Color 1") or is taken becomes one that
/// can ("Color_1"), in the links too.
pub fn migrate_palette(doc: &mut Document) {
    let old = std::mem::take(&mut doc.old_palette);
    if old.is_empty() {
        return;
    }
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    for swatch in old {
        let mut base: String = swatch
            .name
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if !base
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        {
            base.insert(0, '_');
        }
        let free = |n: &str| doc.variables.iter().all(|v| v.name != n);
        let name = if base.chars().all(|c| c == '_') {
            variables::new_name(&doc.variables, Kind::Color)
        } else if free(&base) {
            base
        } else {
            (2..)
                .map(|n| format!("{base}_{n}"))
                .find(|n| free(n))
                .expect("an unused name")
        };
        if !swatch.name.is_empty() && !renamed.contains_key(&swatch.name) {
            renamed.insert(swatch.name, name.clone());
        }
        doc.variables.push(Variable {
            formula: None,
            name,
            value: Value::Color(swatch.color),
        });
    }
    for_each_link(doc, |link| match renamed.get(link.as_str()) {
        Some(new) => {
            *link = new.clone();
            true
        }
        None => false,
    });
}

/// Gives every linked color its color variable's value. A color whose variable is gone
/// (or no longer a color) keeps its link, broken, and its last value: its button shows
/// a broken link icon, and a color variable of that name links it again. Links to a
/// color the layer doesn't have are dropped. Returns true if the document changed.
pub fn apply(doc: &mut Document) -> bool {
    let vars = std::mem::take(&mut doc.variables);
    let find = |name: &str| find(&vars, name);
    let mut changed = false;
    if let Some([r, g, b, _]) = doc.background_link.as_deref().and_then(find) {
        changed |= doc.background != [r, g, b];
        doc.background = [r, g, b];
    }
    // A gradient's stops, each linked on its own.
    let mut stops = |g: &mut crate::document::gradient::Gradient| {
        for stop in &mut g.stops {
            if let Some(c) = stop.link.as_deref().and_then(find) {
                changed |= stop.color != c;
                stop.color = c;
            }
        }
    };
    if let Some(g) = &mut doc.background_gradient {
        stops(g);
    }
    doc.for_each_layer_mut(|layer| layer.gradients_mut().into_iter().for_each(&mut stops));
    doc.for_each_layer_mut(|layer| {
        let mut links = std::mem::take(&mut layer.color_links);
        links.retain(|&field, name| {
            let Some(slot) = slot(layer, field) else {
                changed = true;
                return false;
            };
            if let Some(c) = find(name) {
                changed |= *slot != c;
                *slot = c;
            }
            true
        });
        layer.color_links = links;
        for fx in layer.param_sets_mut() {
            let params = &mut fx.params;
            fx.links.retain(|param, name| {
                let Some(c) = find(name) else {
                    return true;
                };
                let now = params
                    .get(param)
                    .and_then(|v| v.as_str())
                    .and_then(color_from_hex);
                if now != Some(c) {
                    params.insert(param.clone(), color_to_hex(c).into());
                    changed = true;
                }
                true
            });
        }
    });
    doc.variables = vars;
    changed
}

/// Fits the color links of `node`, pasted from the clipboard, to the variables `vars`,
/// which may not be those of the project it was copied from. A link stays when there
/// is a color variable of that name and value; it moves to the color variable of that
/// value when there is one; otherwise its color is saved as a variable of its name. A
/// link whose name another variable has is dropped, its color kept as it is.
pub fn adopt_links(vars: &mut Vec<Variable>, node: &mut Node) {
    let mut fit = |name: &mut String, c: [u8; 4]| {
        if find(vars, name) == Some(c) {
            return true;
        }
        if let Some(v) = vars.iter().find(|v| v.value.color() == Some(c)) {
            *name = v.name.clone();
            return true;
        }
        if vars.iter().any(|v| v.name == *name) {
            return false;
        }
        vars.push(Variable {
            formula: None,
            name: name.clone(),
            value: Value::Color(c),
        });
        true
    };
    node.for_each_layer_mut(&mut |l| {
        for g in l.gradients_mut() {
            for stop in &mut g.stops {
                if let Some(name) = &mut stop.link
                    && !fit(name, stop.color)
                {
                    stop.link = None;
                }
            }
        }
        let mut links = std::mem::take(&mut l.color_links);
        links.retain(|&field, name| match slot(l, field) {
            Some(c) => {
                let c = *c;
                fit(name, c)
            }
            None => false,
        });
        l.color_links = links;
        for fx in l.param_sets_mut() {
            let params = &fx.params;
            fx.links.retain(|param, name| {
                match params
                    .get(param)
                    .and_then(|v| v.as_str())
                    .and_then(color_from_hex)
                {
                    Some(c) => fit(name, c),
                    None => false,
                }
            });
        }
    });
}

/// Where a layer keeps the color `field`, if it has it.
fn slot(layer: &mut crate::document::Layer, field: ColorField) -> Option<&mut [u8; 4]> {
    match field {
        ColorField::Text => layer.text.as_deref_mut().map(|t| &mut t.color),
        ColorField::TextOutline => layer.text.as_deref_mut().map(|t| &mut t.outline_color),
        ColorField::Fill => layer.shape.as_deref_mut().map(|s| &mut s.fill_color),
        ColorField::Stroke => layer.shape.as_deref_mut().map(|s| &mut s.stroke_color),
    }
}

/// Every link to a color variable in the document: the page background's, the layers'
/// and their effects', and their gradients' stops'. A link `f` returns false for is
/// removed.
fn for_each_link(doc: &mut Document, mut f: impl FnMut(&mut String) -> bool) {
    if let Some(name) = &mut doc.background_link
        && !f(name)
    {
        doc.background_link = None;
    }
    let stops = |g: &mut crate::document::gradient::Gradient,
                 f: &mut dyn FnMut(&mut String) -> bool| {
        for stop in &mut g.stops {
            if let Some(name) = &mut stop.link
                && !f(name)
            {
                stop.link = None;
            }
        }
    };
    if let Some(g) = &mut doc.background_gradient {
        stops(g, &mut f);
    }
    doc.for_each_layer_mut(|l| {
        for g in l.gradients_mut() {
            stops(g, &mut f);
        }
        l.color_links.retain(|_, name| f(name));
        for fx in l.param_sets_mut() {
            fx.links.retain(|_, name| f(name));
        }
    });
}

/// How many colors are linked to the variable `name`.
pub fn use_count(doc: &mut Document, name: &str) -> usize {
    let mut n = 0;
    for_each_link(doc, |link| {
        n += (link == name) as usize;
        true
    });
    n
}

/// Links to the variable `old` go to `new`.
pub fn rename_links(doc: &mut Document, old: &str, new: &str) {
    for_each_link(doc, |link| {
        if link == old {
            *link = new.to_owned();
        }
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Group, Layer};

    /// A color variable.
    fn cv(name: &str, c: [u8; 4]) -> Variable {
        Variable {
            formula: None,
            name: name.into(),
            value: Value::Color(c),
        }
    }

    #[test]
    fn gradient_stops_are_colors_in_use_and_follow_their_variables() {
        use crate::document::gradient::Gradient;
        let mut doc = Document::default();
        doc.variables.push(cv("brand", [200, 0, 0, 255]));
        let mut layer = crate::document::shape::new_layer(
            1,
            crate::document::shape::ShapeKind::Rectangle,
            [0.0; 2],
        );
        let mut g = Gradient::from_color([1, 2, 3, 255]);
        g.stops[0].link = Some("brand".into());
        layer.shape.as_mut().unwrap().fill_gradient = Some(g);
        doc.layers.push(Node::Layer(Box::new(layer)));
        let mut bg = Gradient::from_color([9, 9, 9, 255]);
        bg.stops[1].link = Some("brand".into());
        doc.background_gradient = Some(bg);

        // Linked stops take their variable's color.
        assert!(apply(&mut doc));
        let stop = |doc: &Document| {
            doc.layer(1)
                .unwrap()
                .shape
                .as_ref()
                .unwrap()
                .fill_gradient
                .as_ref()
                .unwrap()
                .stops[0]
                .clone()
        };
        assert_eq!(stop(&doc).color, [200, 0, 0, 255]);
        assert_eq!(
            doc.background_gradient.as_ref().unwrap().stops[1].color,
            [200, 0, 0, 255]
        );
        // The stops are colors in use, the shape's single fill color is not.
        let used = used(&doc);
        assert!(used.contains(&[200, 0, 0, 255]) && used.contains(&[9, 9, 9, 255]));
        assert!(!used.contains(&[70, 130, 180, 255]), "{used:?}");
        // Counted, renamed, and broken like other links.
        assert_eq!(use_count(&mut doc, "brand"), 2);
        rename_links(&mut doc, "brand", "red");
        assert_eq!(stop(&doc).link.as_deref(), Some("red"));
        assert_eq!(
            crate::document::variables::broken_links(doc.layer(1).unwrap(), &doc.variables),
            ["red"]
        );
        // Pasted into a project without the variable: saved as one there.
        let mut vars = Vec::new();
        let mut node = doc.layers[0].clone();
        adopt_links(&mut vars, &mut node);
        assert_eq!(vars, [cv("red", [200, 0, 0, 255])]);
    }

    #[test]
    fn used_colors_come_once_each_in_stack_order_without_switched_off_ones() {
        let mut text = Layer::new(1, "t".into(), "".into());
        let mut t = crate::document::text::Text {
            color: [10, 20, 30, 255],
            outline: false,
            outline_color: [1, 1, 1, 255],
            ..Default::default()
        };
        text.text = Some(Box::new(t.clone()));
        let mut shape = Layer::new(2, "s".into(), "".into());
        shape.shape = Some(Box::new(crate::document::shape::Shape {
            fill: true,
            fill_color: [10, 20, 30, 255],
            stroke: true,
            stroke_color: [0, 0, 0, 128],
            ..Default::default()
        }));
        let mut shadow = EffectInstanceBuilder::new("builtin.drop_shadow");
        shadow.set_color("color", [5, 6, 7, 255]);
        shape.effects.push(shadow.0);
        t.outline = true;
        let mut outlined = Layer::new(3, "o".into(), "".into());
        outlined.text = Some(Box::new(t));
        let group = Node::Group(Group {
            id: 4,
            name: "g".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            clip: false,
            blend: crate::document::BlendMode::Normal,
            expanded: true,
            children: vec![Node::Layer(Box::new(outlined))],
            element: None,
        });
        let doc = Document {
            background: [255, 255, 255],
            layers: vec![
                Node::Layer(Box::new(text)),
                Node::Layer(Box::new(shape)),
                group,
            ],
            ..Default::default()
        };
        assert_eq!(
            used(&doc),
            [
                [255, 255, 255, 255],
                [10, 20, 30, 255],
                [0, 0, 0, 128],
                [5, 6, 7, 255],
                [1, 1, 1, 255],
            ]
        );
    }

    /// An effect instance with some parameters set.
    struct EffectInstanceBuilder(crate::render::effects::EffectInstance);

    impl EffectInstanceBuilder {
        fn new(id: &str) -> Self {
            Self(crate::render::effects::EffectInstance::new(
                registry().get(id).expect("built-in effect"),
            ))
        }

        fn set_color(&mut self, param: &str, c: [u8; 4]) {
            self.0.params.insert(
                param.into(),
                serde_json::Value::from(crate::render::effects::color_to_hex(c)),
            );
        }
    }

    #[test]
    fn saving_keeps_one_variable_per_color_and_name() {
        let mut doc = Document::default();
        assert!(save(&mut doc, cv("a", [1, 2, 3, 255])));
        assert!(!save(&mut doc, cv("b", [1, 2, 3, 255])));
        assert!(!save(&mut doc, cv("a", [4, 5, 6, 255])));
        assert!(save(&mut doc, cv("b", [1, 2, 3, 0])));
        doc.variables.push(Variable {
            formula: None,
            name: "c".into(),
            value: Value::Number(1.0),
        });
        assert!(
            !save(&mut doc, cv("c", [7, 7, 7, 255])),
            "a number has the name"
        );
        assert_eq!(doc.variables.len(), 3);
    }

    #[test]
    fn saved_colors_of_older_projects_become_color_variables() {
        let mut json = serde_json::to_value(Document::default()).unwrap();
        let fields = json.as_object_mut().unwrap();
        fields.insert("background_link".into(), "Color 1".into());
        fields.insert(
            "variables".into(),
            serde_json::json!([{"name": "ink", "value": 2}]),
        );
        fields.insert(
            "palette".into(),
            serde_json::json!([
                {"color": [1, 2, 3, 255], "name": "Color 1"},
                {"color": [4, 5, 6, 255], "name": "ink"},
                {"color": [7, 8, 9, 255], "name": ""},
                {"color": [0, 0, 0, 255], "name": "2 tone"},
            ]),
        );
        let json = json.to_string();
        let doc = Document::from_json(&json).unwrap();
        let vars: Vec<_> = doc
            .variables
            .iter()
            .map(|v| (v.name.as_str(), v.value.clone()))
            .collect();
        assert_eq!(
            vars,
            [
                ("ink", Value::Number(2.0)),
                ("Color_1", Value::Color([1, 2, 3, 255])),
                ("ink_2", Value::Color([4, 5, 6, 255])),
                ("color1", Value::Color([7, 8, 9, 255])),
                ("_2_tone", Value::Color([0, 0, 0, 255])),
            ]
        );
        assert_eq!(doc.background_link.as_deref(), Some("Color_1"));
        assert!(doc.old_palette.is_empty());
        let saved = serde_json::to_string(&doc).unwrap();
        assert!(!saved.contains("palette"), "{saved}");
    }

    #[test]
    fn pasted_links_fit_the_variables_of_the_project_they_land_in() {
        let sw = |c: [u8; 4], n: &str| cv(n, c);
        let red = [200, 0, 0, 255];
        let blue = [0, 0, 200, 255];
        let mut text = Layer::new(1, "t".into(), "".into());
        text.text = Some(Box::new(crate::document::text::Text {
            color: red,
            outline_color: blue,
            ..Default::default()
        }));
        text.color_links.insert(ColorField::Text, "Color 1".into());
        text.color_links
            .insert(ColorField::TextOutline, "ink".into());
        text.color_links.insert(ColorField::Fill, "Color 1".into());
        let mut shadow = EffectInstanceBuilder::new("builtin.drop_shadow");
        shadow.set_color("color", [9, 9, 9, 255]);
        shadow.0.links.insert("color".into(), "shadow".into());
        text.effects.push(shadow.0);
        let node = Node::Layer(Box::new(text));
        let links = |node: &Node| match node {
            Node::Layer(l) => (l.color_links.clone(), l.effects[0].links.clone()),
            _ => unreachable!(),
        };

        // "Color 1" is another color there, blue is saved as "navy", "shadow" is free.
        let mut palette = vec![sw([0, 255, 0, 255], "Color 1"), sw(blue, "navy")];
        let mut pasted = node.clone();
        adopt_links(&mut palette, &mut pasted);
        let (layer, fx) = links(&pasted);
        assert_eq!(
            layer.into_iter().collect::<Vec<_>>(),
            [(ColorField::TextOutline, "navy".to_owned())],
            "the clashing name and the missing fill are dropped"
        );
        assert_eq!(fx["color"], "shadow");
        assert_eq!(palette[2], sw([9, 9, 9, 255], "shadow"));

        // In the project it was copied from, nothing changes.
        let mut palette = vec![
            sw(red, "Color 1"),
            sw(blue, "ink"),
            sw([9, 9, 9, 255], "shadow"),
        ];
        let before = palette.clone();
        let mut pasted = node.clone();
        adopt_links(&mut palette, &mut pasted);
        assert_eq!(palette, before);
        assert_eq!(links(&pasted).0.len(), 2);

        // A variable of another kind has the name: that link is dropped.
        let number = Variable {
            formula: None,
            name: "shadow".into(),
            value: Value::Number(1.0),
        };
        let mut vars = vec![number.clone()];
        let mut pasted = node.clone();
        adopt_links(&mut vars, &mut pasted);
        assert!(links(&pasted).1.is_empty());
        assert_eq!(vars, [number, sw(red, "Color 1"), sw(blue, "ink")]);
    }

    #[test]
    fn linked_colors_follow_their_saved_color_and_keep_their_value_when_it_is_gone() {
        let mut shape = Layer::new(1, "s".into(), "".into());
        shape.shape = Some(Box::default());
        shape.color_links.insert(ColorField::Fill, "brand".into());
        shape.color_links.insert(ColorField::Text, "brand".into());
        let mut shadow = EffectInstanceBuilder::new("builtin.drop_shadow");
        shadow.0.links.insert("color".into(), "ink".into());
        shape.effects.push(shadow.0);
        let mut doc = Document {
            background_link: Some("brand".into()),
            layers: vec![Node::Layer(Box::new(shape))],
            variables: vec![cv("brand", [200, 0, 0, 255]), cv("ink", [0, 0, 50, 128])],
            ..Default::default()
        };
        assert!(apply(&mut doc));
        assert!(!apply(&mut doc), "nothing more to do");
        let layer = |doc: &Document| match &doc.layers[0] {
            Node::Layer(l) => l.clone(),
            _ => unreachable!(),
        };
        let l = layer(&doc);
        assert_eq!(doc.background, [200, 0, 0]);
        assert_eq!(l.shape.as_ref().unwrap().fill_color, [200, 0, 0, 255]);
        assert_eq!(
            l.color_links.keys().collect::<Vec<_>>(),
            [&ColorField::Fill],
            "a shape has no text color"
        );
        let fx = &l.effects[0];
        let params = Params::new(registry().get(&fx.effect).unwrap().params(), &fx.params);
        assert_eq!(params.color("color"), [0, 0, 50, 128]);
        assert_eq!(use_count(&mut doc, "brand"), 2);

        // Renamed in the links, then removed: the colors stay as they are, their links
        // kept, broken.
        rename_links(&mut doc, "brand", "red");
        doc.variables[0] = cv("red", [0, 200, 0, 255]);
        apply(&mut doc);
        assert_eq!(doc.background, [0, 200, 0]);
        // A variable of that name that isn't a color is no link either.
        doc.variables[0].value = Value::Number(1.0);
        doc.variables.remove(1);
        assert!(!apply(&mut doc), "nothing changes");
        let l = layer(&doc);
        assert_eq!(doc.background_link.as_deref(), Some("red"));
        assert_eq!(l.color_links[&ColorField::Fill], "red");
        assert_eq!(l.effects[0].links["color"], "ink");
        assert_eq!(l.shape.as_ref().unwrap().fill_color, [0, 200, 0, 255]);

        // A color variable of that name links them again.
        doc.variables.push(cv("ink", [9, 9, 9, 255]));
        assert!(apply(&mut doc));
        let fx = &layer(&doc).effects[0];
        let params = Params::new(registry().get(&fx.effect).unwrap().params(), &fx.params);
        assert_eq!(params.color("color"), [9, 9, 9, 255]);
    }
}
