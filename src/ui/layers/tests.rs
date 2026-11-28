use super::ops::{all_names, copy_base};
use super::*;
use crate::document::Layer;

fn layer(id: u64) -> Node {
    Node::Layer(Box::new(Layer::new(id, format!("l{id}"), "x.png".into())))
}

/// Ids in the tree, as "id" or "id[children]", bottom first.
fn shape(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| match n {
            Node::Layer(l) => l.id.to_string(),
            Node::Group(g) => format!("{}[{}]", g.id, shape(&g.children)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn doc_with(nodes: Vec<Node>) -> Document {
    Document {
        layers: nodes,
        next_id: 100,
        ..Default::default()
    }
}

/// Drives the real panel with synthetic pointer input.
struct Harness {
    ctx: egui::Context,
    doc: Document,
    selected: Option<u64>,
    state: LayersPanel,
    time: f64,
    /// Centers of the dots drawn in the last frame (effect drag handles).
    dots: Vec<Pos2>,
    /// Filled rectangles drawn in the last frame, with their color.
    fills: Vec<(Rect, egui::Color32)>,
    /// The mouse cursor asked for in the last frame.
    cursor: egui::CursorIcon,
    /// A shape kind picked from the Shape button's menu.
    added_shape: Option<ShapeKind>,
}

impl Harness {
    fn new(doc: Document) -> Self {
        let ctx = egui::Context::default();
        crate::app::set_interaction(&ctx);
        let mut h = Self {
            ctx,
            doc,
            selected: None,
            state: LayersPanel::default(),
            time: 0.0,
            dots: Vec::new(),
            fills: Vec::new(),
            cursor: egui::CursorIcon::Default,
            added_shape: None,
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    /// Runs one frame; returns where each text label was drawn.
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, Pos2)> {
        self.time += 0.02;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1000.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut added = None;
        let mut out = self.ctx.run_ui(input, |ui| {
            // The Layers panel above the Properties panel, as docked by default.
            added = show(ui, &mut self.doc, &mut self.selected, &mut self.state).add_shape;
            ui.separator();
            properties_panel(
                ui,
                &mut self.doc,
                self.selected,
                &mut self.state,
                None,
                &FontLibrary::builtin_only(),
            );
        });
        self.added_shape = self.added_shape.or(added);
        out.textures_delta.clear();
        self.cursor = out.platform_output.cursor_icon;
        self.dots = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Circle(d) => Some(d.center),
                _ => None,
            })
            .collect();
        self.fills = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect();
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos)),
                _ => None,
            })
            .collect()
    }

    /// The "+" at the right of the Effects header.
    fn add_effect_button(&mut self) -> Pos2 {
        let labels = self.frame(vec![]);
        let (_, p) = labels
            .into_iter()
            .find(|(t, _)| t.starts_with("Effects"))
            .expect("Effects header");
        Pos2::new(388.0, p.y + 6.0)
    }

    fn label(&mut self, text: &str) -> Pos2 {
        let labels = self.frame(vec![]);
        let (_, p) = labels.into_iter().find(|(t, _)| t == text).expect(text);
        p + vec2(4.0, 6.0)
    }

    fn pointer(&mut self, p: Pos2, pressed: Option<bool>) {
        let mut events = vec![egui::Event::PointerMoved(p)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        self.frame(events);
    }

    fn click(&mut self, p: Pos2) {
        self.pointer(p, None);
        self.pointer(p, Some(true));
        self.pointer(p, Some(false));
    }

    /// Middle of the drag handle left of an effect's name.
    fn handle(&mut self, name: &str) -> Pos2 {
        let labels = self.frame(vec![]);
        let (_, t) = labels.into_iter().find(|(t, _)| t == name).expect(name);
        let dots: Vec<Pos2> = self
            .dots
            .iter()
            .copied()
            .filter(|d| d.x < t.x && (t.y - 6.0..t.y + 22.0).contains(&d.y))
            .collect();
        assert_eq!(dots.len(), 6, "handle of {name}");
        let sum = dots.iter().fold(vec2(0.0, 0.0), |a, d| a + d.to_vec2());
        (sum / 6.0).to_pos2()
    }

    fn secondary_click(&mut self, p: Pos2) {
        self.pointer(p, None);
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
        self.frame(vec![]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.pointer(from, None);
        self.pointer(from, Some(true));
        for k in 1..=5 {
            self.pointer(from + (to - from) * (k as f32 / 5.0), None);
        }
        self.pointer(to, Some(false));
        self.frame(vec![]);
    }
}

#[test]
fn name_field_gives_up_focus_on_a_press_elsewhere() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    let p = h.label("alpha");
    // Double-clicking the name in the list doesn't edit it there.
    h.click(p);
    h.click(p);
    h.frame(vec![]);
    assert!(h.ctx.memory(|m| m.focused().is_none()));

    // The Name field of the properties.
    let field = h.label("Name") + vec2(150.0, 0.0);
    h.click(field);
    assert!(h.ctx.memory(|m| m.focused().is_some()));
    // Press and drag on empty space below the panel, as when grabbing the canvas.
    let empty = Pos2::new(200.0, 780.0);
    h.drag(empty, empty + vec2(40.0, -20.0));
    assert!(h.ctx.memory(|m| m.focused().is_none()));
}

#[test]
fn new_layers_get_two_random_words_not_used_yet() {
    let mut doc = Document::default();
    for k in 0..40 {
        let name = random_name(&doc);
        let words: Vec<&str> = name.split(' ').collect();
        assert_eq!(words.len(), 2, "{name}");
        assert!(tr!("LAYER_NAME_ADJECTIVES").contains(words[0]), "{name}");
        assert!(tr!("LAYER_NAME_NOUNS").contains(words[1]), "{name}");
        assert!(
            !all_names(&doc).contains(&name.as_str()),
            "{name} used already"
        );
        doc.layers.push(named(k + 1, &name));
    }
}

fn named(id: u64, name: &str) -> Node {
    Node::Layer(Box::new(Layer::new(id, name.into(), "x.png".into())))
}

#[test]
fn properties_and_effects_fold_from_their_titles() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    h.selected = Some(1);
    let shown = |h: &mut Harness, text: &str| h.frame(vec![]).iter().any(|(t, _)| t == text);
    assert!(shown(&mut h, "Opacity") && shown(&mut h, "Effects"));

    let p = h.label("General");
    h.click(p);
    // Let the folding animation finish.
    for _ in 0..30 {
        h.frame(vec![]);
    }
    assert!(!shown(&mut h, "Opacity"), "General folded");
    // The image's own section comes after General, not inside it.
    let res = |h: &mut Harness| {
        h.frame(vec![])
            .iter()
            .any(|(t, _)| t.starts_with("Print resolution"))
    };
    assert!(res(&mut h), "Image still open");
    assert!(
        shown(
            &mut h,
            "Effects change how the image looks without changing its file."
        ),
        "Effects still open"
    );

    let p = h.label("Effects");
    h.click(p);
    for _ in 0..30 {
        h.frame(vec![]);
    }
    assert!(!shown(
        &mut h,
        "Effects change how the image looks without changing its file."
    ));
    // The + stays in the header, so effects can be added while folded; that opens it.
    let p = h.add_effect_button();
    h.click(p);
    let p = h.label("Invert");
    h.click(p);
    for _ in 0..30 {
        h.frame(vec![]);
    }
    assert_eq!(h.doc.layer(1).unwrap().effects.len(), 1);
    assert!(shown(&mut h, "Effects (1)"));
    assert!(h.handle("Invert").x > 0.0, "Effects opened on adding");

    let p = h.label("General");
    h.click(p);
    for _ in 0..30 {
        h.frame(vec![]);
    }
    assert!(shown(&mut h, "Opacity"), "General open again");
}

#[test]
fn text_cursor_only_over_what_can_be_edited() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    h.selected = Some(1);
    for (label, want) in [
        ("Opacity", egui::CursorIcon::Default),
        ("General", egui::CursorIcon::Default),
    ] {
        let p = h.label(label);
        h.pointer(p, None);
        h.frame(vec![]);
        assert_eq!(h.cursor, want, "over {label:?}");
    }
    // The name field, right of its label.
    let p = h.label("Name") + vec2(120.0, 0.0);
    h.pointer(p, None);
    h.frame(vec![]);
    assert_eq!(h.cursor, egui::CursorIcon::Text, "over the name field");
}

#[test]
fn padlock_right_of_the_size_toggles_the_aspect_ratio() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    h.selected = Some(1);
    let labels = h.frame(vec![]);
    let (_, hpos) = labels
        .iter()
        .find(|(t, _)| t == "h")
        .unwrap_or_else(|| panic!("height field: {labels:?}"));
    // The closed padlock's body: a filled 10 x 7 rectangle right of the height, on
    // the same line.
    let body = h
        .fills
        .iter()
        .map(|(r, _)| *r)
        .find(|r| {
            (r.width() - 10.0).abs() < 0.1
                && (r.height() - 7.0).abs() < 0.1
                && r.left() > hpos.x
                && (r.center().y - hpos.y).abs() < 16.0
        })
        .expect("padlock");
    assert!(!h.state.free_aspect);
    h.click(body.center());
    assert!(h.state.free_aspect, "unlocked");
    h.click(body.center());
    assert!(!h.state.free_aspect, "locked again");
}

#[test]
fn hovered_row_is_highlighted_across_its_width() {
    let mut h = Harness::new(doc_with(vec![named(1, "a"), named(2, "b")]));
    let b = h.label("b");
    h.pointer(Pos2::new(380.0, b.y), None);
    h.frame(vec![]);
    let fill = h.ctx.global_style().visuals.widgets.hovered.weak_bg_fill;
    let rows: Vec<Rect> = h
        .fills
        .iter()
        .filter(|(r, c)| *c == fill && r.left() < 20.0 && r.width() > 300.0)
        .map(|(r, _)| *r)
        .collect();
    assert_eq!(rows.len(), 1, "{:?}", h.fills);
    assert!(rows[0].y_range().contains(b.y));
}

#[test]
fn clicking_right_of_a_short_name_selects_the_layer() {
    let mut h = Harness::new(doc_with(vec![named(1, "a"), named(2, "b")]));
    let p = h.label("b");
    h.click(Pos2::new(380.0, p.y));
    assert_eq!(h.selected, Some(2));
}

#[test]
fn selected_row_is_highlighted_across_its_width() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha"), named(2, "beta")]));
    let alpha = h.label("alpha");
    h.click(alpha);
    assert_eq!(h.selected, Some(1));
    let beta = h.label("beta");
    let fill = h.ctx.global_style().visuals.selection.bg_fill;
    let rows: Vec<Rect> = h
        .fills
        .iter()
        .filter(|(r, c)| *c == fill && r.left() < 20.0 && r.width() > 300.0)
        .map(|(r, _)| *r)
        .collect();
    assert_eq!(rows.len(), 1, "one full-row highlight: {:?}", h.fills);
    let row = rows[0];
    // From the eye icon on the left to the name and beyond.
    assert!(
        row.left() < 20.0 && row.y_range().contains(alpha.y),
        "{row:?}"
    );
    assert!(!row.y_range().contains(beta.y), "{row:?}");
}

#[test]
fn panel_click_selects_and_drag_drops_into_group() {
    let mut d = doc_with(vec![named(1, "alpha"), named(2, "beta")]);
    group(&mut d, &mut Some(2)); // alpha 100[beta]
    let mut h = Harness::new(d);

    let g = h.label("Group 1");
    h.click(g);
    assert_eq!(h.selected, Some(100), "clicking a group row selects it");

    // Drop alpha onto the middle of the group row.
    let from = h.label("alpha");
    let to = h.label("Group 1");
    h.drag(from, to);
    assert_eq!(shape(&h.doc.layers), "100[2 1]");

    // Drop alpha below beta (lower half of beta's row): bottom of the group.
    let from = h.label("alpha");
    let to = h.label("beta") + vec2(0.0, 7.0);
    h.drag(from, to);
    assert_eq!(shape(&h.doc.layers), "100[1 2]");
}

#[test]
fn panel_adds_and_removes_effects() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    h.selected = Some(1);
    let p = h.add_effect_button();
    h.click(p);
    let p = h.label("Invert");
    h.click(p);
    let effects = |h: &Harness| h.doc.layer(1).unwrap().effects.clone();
    assert_eq!(effects(&h).len(), 1);
    let labels = h.frame(vec![]);
    assert!(
        !labels.iter().any(|(t, _)| t == "Grayscale"),
        "menu closed after picking"
    );
    assert_eq!(effects(&h)[0].effect, "builtin.invert");

    // Parameters get controls built from their specs.
    let p = h.add_effect_button();
    h.click(p);
    let p = h.label("Gaussian blur");
    h.click(p);
    h.label("Radius");
    assert_eq!(effects(&h)[1].effect, "builtin.blur");

    let ids = |h: &Harness| -> Vec<String> { effects(h).into_iter().map(|e| e.effect).collect() };
    // Dragging the name does nothing: only the handle moves effects.
    let from = h.label("Invert");
    let to = h.label("Radius");
    h.drag(from, to);
    assert_eq!(ids(&h), ["builtin.invert", "builtin.blur"]);

    // Shown as layers are, the last applied at the top: the blur, added last, is above.
    assert!(h.handle("Gaussian blur").y < h.handle("Invert").y);
    // Drag Invert's handle into the space just above the list: applied last.
    let from = h.handle("Invert");
    let to = h.handle("Gaussian blur") - vec2(0.0, 14.0);
    h.drag(from, to);
    assert_eq!(ids(&h), ["builtin.blur", "builtin.invert"]);
    assert!(h.handle("Invert").y < h.handle("Gaussian blur").y);
    // And back, onto the lower half of the blur (its Radius row): applied first.
    let from = h.handle("Invert");
    let to = h.label("Radius");
    h.drag(from, to);
    assert_eq!(ids(&h), ["builtin.invert", "builtin.blur"]);

    // Remove from the right-click menu, after confirming; Cancel keeps it.
    let p = h.label("Invert");
    h.secondary_click(p);
    let p = h.label("Remove");
    h.click(p);
    let p = h.label("Cancel");
    assert!(
        h.frame(vec![])
            .iter()
            .any(|(t, _)| t == "Remove the effect Invert?")
    );
    h.click(p);
    assert_eq!(ids(&h), ["builtin.invert", "builtin.blur"]);
    let p = h.label("Invert");
    h.secondary_click(p);
    let p = h.label("Remove");
    h.click(p);
    assert_eq!(
        ids(&h),
        ["builtin.invert", "builtin.blur"],
        "not before confirming"
    );
    let p = h.label("Remove");
    h.click(p);
    // The blur stays.
    assert_eq!(ids(&h), ["builtin.blur"]);
}

#[test]
fn right_click_duplicates_a_layer_with_its_effects() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    h.doc
        .layer_mut(1)
        .unwrap()
        .effects
        .push(crate::render::effects::EffectInstance::new(
            crate::render::effects::registry()
                .get("builtin.invert")
                .unwrap(),
        ));
    let p = h.label("alpha");
    h.secondary_click(p);
    let p = h.label("Duplicate");
    h.click(p);
    let copy = h
        .doc
        .node(h.selected.unwrap())
        .and_then(Node::as_layer)
        .unwrap();
    assert_eq!(copy.name, "alpha copy");
    assert_eq!(copy.effects, h.doc.layer(1).unwrap().effects);
    assert_ne!(copy.id, 1);
}

#[test]
fn text_layer_shows_its_text_settings() {
    let mut doc = Document::default();
    let mut layer = text::new_layer(1, "Hello", text::BUILTIN_FAMILY.into(), [50.0, 50.0]);
    // Laid out, as the app does before showing the panel.
    let font = skrifa::FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
    layer.size_px = text::layout(layer.text.as_ref().unwrap(), &font).size;
    doc.layers.push(Node::Layer(Box::new(layer)));
    let mut h = Harness::new(doc);
    h.selected = Some(1);
    let labels = h.frame(vec![]);
    for want in ["Font", "Font size", "Letter spacing", "Outline", "General"] {
        assert!(labels.iter().any(|(t, _)| t == want), "no {want:?}");
    }
    assert!(
        !labels
            .iter()
            .any(|(t, _)| t.starts_with("Print resolution")),
        "no DPI for text"
    );
    // The font size field shows the default size.
    assert!(labels.iter().any(|(t, _)| t == "24.0"), "{labels:?}");
    // The text settings are a part of Properties, below the common ones.
    let y = |want: &str| labels.iter().find(|(t, _)| t == want).unwrap().1.y;
    assert!(y("General") < y("Opacity") && y("Opacity") < y("Text"));
    assert!(y("Text") < y("Font"));
}

#[test]
fn panel_eye_toggles_visibility() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    let name = h.label("alpha");
    // The eye sits left of the lock and the thumbnail space.
    let eye = name - vec2(3.0 + THUMB + 3.0 + ICON + 3.0 + ICON / 2.0 + 4.0, 0.0);
    h.click(eye);
    assert!(!h.doc.layer(1).unwrap().visible);
    h.click(eye);
    assert!(h.doc.layer(1).unwrap().visible);
}

#[test]
fn group_and_ungroup() {
    let mut d = doc_with(vec![layer(1), layer(2)]);
    let mut sel = Some(1);
    assert!(group(&mut d, &mut sel));
    assert_eq!(shape(&d.layers), "100[1] 2");
    assert_eq!(sel, Some(100));
    if let Some(Node::Group(g)) = d.node_mut(100) {
        g.opacity = 0.5;
        g.visible = false;
    }
    assert!(ungroup(&mut d, &mut sel));
    assert_eq!(shape(&d.layers), "1 2");
    let l = d.layer(1).unwrap();
    assert_eq!((l.opacity, l.visible), (0.5, false));
}

/// The layer list and Properties with layers clipped to a text layer, to look at:
/// `CLIP_PANEL_PREVIEW=out.png cargo test clip_panel_preview -- --ignored`.
#[test]
#[ignore]
fn clip_panel_preview() {
    let Ok(path) = std::env::var("CLIP_PANEL_PREVIEW") else {
        return;
    };
    let t = text::new_layer(2, "Title", text::BUILTIN_FAMILY.into(), [50.0, 50.0]);
    let mut a = layer(3);
    *a.clip_mut() = true;
    let mut b = layer(4);
    *b.clip_mut() = true;
    let doc = std::cell::RefCell::new(doc_with(vec![layer(1), Node::Layer(Box::new(t)), a, b]));
    let state = std::cell::RefCell::new(LayersPanel::default());
    crate::ui::preview::render(&path, vec2(320.0, 560.0), |ui| {
        let (mut doc, mut state) = (doc.borrow_mut(), state.borrow_mut());
        let mut sel = Some(3);
        show(ui, &mut doc, &mut sel, &mut state);
        ui.separator();
        properties_panel(
            ui,
            &mut doc,
            sel,
            &mut state,
            None,
            &FontLibrary::builtin_only(),
        );
    });
}

#[test]
fn flipping_a_group_mirrors_it_as_a_whole() {
    // Two 10 x 10 mm squares side by side, the right one turned and tilted: the
    // group covers [0, 30] x [0, 10] mm.
    let square = |id: u64, x: f32| {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [10, 10];
        l.transform.center = [x, 5.0];
        l
    };
    let mut b = square(2, 25.0);
    b.transform.rotation_deg = 20.0;
    b.transform.tilt_y_deg = 15.0;
    let mut d = doc_with(vec![Node::Group(Group {
        id: 10,
        name: "g".into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        clip: false,
        blend: crate::document::BlendMode::Normal,
        expanded: true,
        children: vec![
            Node::Layer(Box::new(square(1, 5.0))),
            Node::Layer(Box::new(b)),
        ],
        element: None,
    })]);
    let mirrored = |p: P2, mid: f32| [2.0 * mid - p[0], p[1]];
    let before: Vec<[P2; 4]> = [1, 2].map(|id| d.layer(id).unwrap().corners()).into();
    let (lo, hi) = layers_bounds(&d, &[1, 2]).unwrap();
    let mid = (lo[0] + hi[0]) / 2.0;
    assert!(flip_group(&mut d, 10, true));
    for (k, id) in [1, 2].into_iter().enumerate() {
        let after = d.layer(id).unwrap().corners();
        // The mirror image of each corner is one of the corners now.
        for p in before[k] {
            let m = mirrored(p, mid);
            assert!(
                after
                    .iter()
                    .any(|q| (q[0] - m[0]).abs() < 1e-3 && (q[1] - m[1]).abs() < 1e-3),
                "layer {id}: {m:?} in {after:?}"
            );
        }
    }
    let t = &d.layer(2).unwrap().transform;
    assert!(t.flip_x && t.rotation_deg == -20.0 && t.tilt_y_deg == 15.0);
    // Twice gives it back.
    assert!(flip_group(&mut d, 10, true));
    assert_eq!(d.layer(2).unwrap().corners(), before[1]);
}

#[test]
fn rotating_a_group_turns_it_as_a_whole() {
    let square = |id: u64, x: f32| {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [10, 10];
        l.transform.center = [x, 5.0];
        l
    };
    let mut b = square(2, 25.0);
    b.transform.rotation_deg = 20.0;
    b.transform.flip_x = true;
    let mut d = doc_with(vec![Node::Group(Group {
        id: 10,
        name: "g".into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        clip: false,
        blend: crate::document::BlendMode::Normal,
        expanded: true,
        children: vec![
            Node::Layer(Box::new(square(1, 5.0))),
            Node::Layer(Box::new(b)),
        ],
        element: None,
    })]);
    let mid = group_middle(&d, 10).unwrap();
    // A quarter turn clockwise with y down: (x, y) -> (-y, x) around the middle.
    let turned = |p: P2| [mid[0] - (p[1] - mid[1]), mid[1] + (p[0] - mid[0])];
    let before: Vec<[P2; 4]> = [1, 2].map(|id| d.layer(id).unwrap().corners()).into();
    assert!(rotate_group(&mut d, 10, 90.0));
    for (k, id) in [1, 2].into_iter().enumerate() {
        let after = d.layer(id).unwrap().corners();
        for (p, q) in before[k].iter().zip(after) {
            let t = turned(*p);
            assert!(
                (q[0] - t[0]).abs() < 1e-3 && (q[1] - t[1]).abs() < 1e-3,
                "layer {id}: {t:?} vs {q:?}"
            );
        }
    }
    assert_eq!(d.layer(2).unwrap().transform.rotation_deg, 110.0);
    assert!(!rotate_group(&mut d, 10, 360.0), "a full turn does nothing");
}

#[test]
fn reset_tilt_clears_only_the_tilt() {
    let mut l = Layer::new(1, "l1".into(), "x.png".into());
    l.transform.tilt_x_deg = 20.0;
    l.transform.tilt_y_deg = -10.0;
    l.transform.perspective = 0.9;
    l.transform.rotation_deg = 30.0;
    let mut h = Harness::new(doc_with(vec![Node::Layer(Box::new(l))]));
    h.selected = Some(1);
    h.frame(vec![]);
    let button = h.label("Reset 3D tilt");
    h.click(button);
    let t = &h.doc.layer(1).unwrap().transform;
    assert_eq!((t.tilt_x_deg, t.tilt_y_deg), (0.0, 0.0));
    assert_eq!(t.perspective, crate::document::DEFAULT_PERSPECTIVE);
    assert_eq!(t.rotation_deg, 30.0, "the rotation stays");
}

/// A group's Properties, to look at them:
/// `GROUP_PANEL_PREVIEW=out.png cargo test group_panel_preview -- --ignored`.
#[test]
#[ignore]
fn group_panel_preview() {
    let Ok(path) = std::env::var("GROUP_PANEL_PREVIEW") else {
        return;
    };
    let doc = std::cell::RefCell::new(doc_with(vec![layer(1), layer(2)]));
    let state = std::cell::RefCell::new(LayersPanel {
        group_turn_deg: 12.5,
        ..Default::default()
    });
    {
        let mut sel = Some(2);
        group(&mut doc.borrow_mut(), &mut sel);
    }
    crate::ui::preview::render(&path, vec2(320.0, 300.0), |ui| {
        let (mut doc, mut state) = (doc.borrow_mut(), state.borrow_mut());
        properties_panel(
            ui,
            &mut doc,
            Some(100),
            &mut state,
            None,
            &FontLibrary::builtin_only(),
        );
    });
}

#[test]
fn clip_toggles_only_with_a_layer_below() {
    let mut d = doc_with(vec![layer(1), layer(2)]);
    // The bottom layer has nothing below to clip to.
    assert!(!toggle_clip(&mut d, Some(1)));
    assert!(!d.node(1).unwrap().clip());
    assert!(toggle_clip(&mut d, Some(2)));
    assert!(d.node(2).unwrap().clip());
    assert!(toggle_clip(&mut d, Some(2)));
    assert!(!d.node(2).unwrap().clip());
    // Moved to the bottom while clipped, it can still be turned off.
    *d.node_mut(1).unwrap().clip_mut() = true;
    assert!(toggle_clip(&mut d, Some(1)));
    assert!(!d.node(1).unwrap().clip());
}

#[test]
fn grouping_keeps_clipping_to_the_layer_below() {
    let mut d = doc_with(vec![layer(1), layer(2)]);
    *d.node_mut(2).unwrap().clip_mut() = true;
    let mut sel = Some(2);
    assert!(group(&mut d, &mut sel));
    assert_eq!(shape(&d.layers), "1 100[2]");
    assert!(d.node(100).unwrap().clip(), "the group is clipped");
    assert!(!d.node(2).unwrap().clip(), "its layer no longer is");
    assert!(ungroup(&mut d, &mut sel));
    assert!(d.node(2).unwrap().clip(), "clipped again once ungrouped");
}

#[test]
fn steps_enter_and_leave_groups() {
    let mut d = doc_with(vec![layer(1), layer(2)]);
    group(&mut d, &mut Some(2)); // 1 100[2]
    assert!(move_step(&mut d, 1, true));
    assert_eq!(shape(&d.layers), "100[1 2]");
    assert!(move_step(&mut d, 1, true));
    assert_eq!(shape(&d.layers), "100[2 1]");
    assert!(move_step(&mut d, 1, true));
    assert_eq!(shape(&d.layers), "100[2] 1");
    assert!(!can_step(&d, 1, true));
    assert!(move_step(&mut d, 1, false));
    assert_eq!(shape(&d.layers), "100[2 1]");
}

#[test]
fn drag_and_drop_moves() {
    let mut d = doc_with(vec![layer(1), layer(2), layer(3)]);
    group(&mut d, &mut Some(3)); // 1 2 100[3]
    assert!(drop_node(&mut d, 1, DropTarget::Into(100)));
    assert_eq!(shape(&d.layers), "2 100[3 1]");
    assert!(drop_node(&mut d, 2, DropTarget::Above(3)));
    assert_eq!(shape(&d.layers), "100[3 2 1]");
    assert!(drop_node(&mut d, 1, DropTarget::Below(100)));
    assert_eq!(shape(&d.layers), "1 100[3 2]");
    // A group cannot go inside itself.
    let mut sel = Some(100);
    group(&mut d, &mut sel); // 1 101[100[3 2]]
    assert!(!drop_node(&mut d, 101, DropTarget::Into(100)));
    assert_eq!(shape(&d.layers), "1 101[100[3 2]]");
}

#[test]
fn copies_are_numbered_after_the_original() {
    let named = |name: &str| {
        let mut n = layer(9);
        *name_mut(&mut n) = name.into();
        n
    };
    let mut d = doc_with(vec![named("star")]);
    let add = |d: &mut Document, name: &str| {
        let mut n = named(name);
        name_copy(d, &mut n);
        let name = name_mut(&mut n).clone();
        d.layers.push(n);
        name
    };
    assert_eq!(add(&mut d, "star"), "star copy");
    assert_eq!(add(&mut d, "star"), "star copy (2)");
    assert_eq!(add(&mut d, "star copy"), "star copy (3)");
    assert_eq!(add(&mut d, "star copy (2)"), "star copy (4)");
    assert_eq!(add(&mut d, "moon"), "moon", "an unused name stays");
    assert_eq!(add(&mut d, "moon"), "moon copy");
    assert_eq!(copy_base("(2)"), "(2)");
    assert_eq!(copy_base("a copy (x)"), "a copy (x)");
}

#[test]
fn duplicate_group_gets_fresh_ids_and_delete_selects_neighbor() {
    let mut d = doc_with(vec![layer(1), layer(2)]);
    let mut sel = Some(2);
    group(&mut d, &mut sel); // 1 100[2]
    assert!(duplicate(&mut d, &mut sel));
    assert_eq!(shape(&d.layers), "1 100[2] 101[102]");
    assert_eq!(sel, Some(101));
    assert!(delete(&mut d, &mut sel));
    assert_eq!(sel, Some(100));
    let mut sel = Some(2);
    delete(&mut d, &mut sel);
    assert_eq!(sel, Some(100), "falls back to the parent group");
}

#[test]
fn align_uses_drawn_bounds_and_moves_groups_as_a_block() {
    let sized = |id: u64, center: P2| {
        let mut l = crate::document::Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [20, 10];
        l.transform.center = center;
        Node::Layer(Box::new(l))
    };
    let page = ([0.0, 0.0], [297.0, 210.0]);
    let mut d = doc_with(vec![sized(1, [50.0, 50.0])]);
    assert!(align(&mut d, Some(1), Align::Left, page));
    assert_eq!(d.layer(1).unwrap().transform.center, [10.0, 50.0]);
    assert!(align(&mut d, Some(1), Align::Bottom, page));
    assert_eq!(d.layer(1).unwrap().transform.center, [10.0, 205.0]);
    assert!(align(&mut d, Some(1), Align::CenterX, page));
    assert!(align(&mut d, Some(1), Align::CenterY, page));
    assert_eq!(d.layer(1).unwrap().transform.center, [148.5, 105.0]);
    assert!(
        !align(&mut d, Some(1), Align::CenterY, page),
        "already there"
    );

    // Rotated a quarter turn, the 20 x 10 image is 10 wide on the page.
    d.layer_mut(1).unwrap().transform.rotation_deg = 90.0;
    assert!(align(&mut d, Some(1), Align::Right, page));
    assert!((d.layer(1).unwrap().transform.center[0] - 292.0).abs() < 1e-3);

    // A group keeps its members' spacing.
    let mut d = doc_with(vec![sized(1, [50.0, 50.0]), sized(2, [100.0, 80.0])]);
    group(&mut d, &mut Some(1));
    let g = d.layers.iter().find_map(|n| n.as_group()).unwrap().id;
    // Group holds layer 1 only; put 2 in as well.
    drop_node(&mut d, 2, DropTarget::Into(g));
    assert!(align(&mut d, Some(g), Align::Top, page));
    assert_eq!(d.layer(1).unwrap().transform.center, [50.0, 5.0]);
    assert_eq!(d.layer(2).unwrap().transform.center, [100.0, 35.0]);
    assert!(align(&mut d, Some(g), Align::Right, page));
    assert_eq!(d.layer(1).unwrap().transform.center, [237.0, 5.0]);
    assert_eq!(d.layer(2).unwrap().transform.center, [287.0, 35.0]);

    // Locked layers stay put.
    d.layer_mut(1).unwrap().locked = true;
    assert!(!align(&mut d, Some(1), Align::Left, page));
}

#[test]
fn nudge_moves_unlocked_group_members() {
    let mut d = doc_with(vec![layer(1), layer(2)]);
    group(&mut d, &mut Some(1));
    drop_node(&mut d, 2, DropTarget::Into(100));
    d.layer_mut(2).unwrap().locked = true;
    assert!(nudge(&mut d, Some(100), [1.0, 0.0]));
    assert_eq!(d.layer(1).unwrap().transform.center, [1.0, 0.0]);
    assert_eq!(d.layer(2).unwrap().transform.center, [0.0, 0.0]);
    if let Some(Node::Group(g)) = d.node_mut(100) {
        g.locked = true;
    }
    assert!(!nudge(&mut d, Some(1), [1.0, 0.0]));
}

#[test]
fn step_selection_follows_the_rows() {
    let mut d = doc_with(vec![layer(1), layer(2), layer(3)]);
    group(&mut d, &mut Some(2));
    drop_node(&mut d, 1, DropTarget::Into(100));
    // Rows, top first: 3, the group, 2, 1.
    let mut sel = None;
    assert!(step_selection(&d, &mut sel, true));
    assert_eq!(sel, Some(3));
    let order: Vec<_> = (0..3)
        .map(|_| {
            step_selection(&d, &mut sel, true);
            sel.unwrap()
        })
        .collect();
    assert_eq!(
        order,
        rows(&d).iter().skip(1).map(|r| r.id).collect::<Vec<_>>()
    );
    assert!(!step_selection(&d, &mut sel, true), "stays on the last row");
    assert!(step_selection(&d, &mut sel, false));
    assert_eq!(sel, Some(rows(&d)[2].id));
}

#[test]
fn linked_axes_of_layer_and_group() {
    use crate::document::variables::Field;
    let mut d = doc_with(vec![layer(1), layer(2)]);
    group(&mut d, &mut Some(1));
    drop_node(&mut d, 2, DropTarget::Into(100));
    assert_eq!(linked_axes(&d, Some(100)), [false, false]);
    d.layer_mut(2)
        .unwrap()
        .bindings
        .insert(Field::Y, "p_y".into());
    assert_eq!(linked_axes(&d, Some(1)), [false, false]);
    assert_eq!(linked_axes(&d, Some(2)), [false, true]);
    assert_eq!(linked_axes(&d, Some(100)), [false, true]);
    assert_eq!(linked_axes(&d, None), [false, false]);
}

#[test]
fn shape_menu_and_properties() {
    let mut h = Harness::new(doc_with(vec![layer(1)]));
    // The Shape button is the third in the toolbar, at the top of the panel, above
    // the layer list.
    let labels = h.frame(vec![]);
    assert!(!labels.iter().any(|(t, _)| t == "Star"), "menu closed");
    let first_layer = labels
        .iter()
        .find(|(t, _)| t == "l1")
        .map(|(_, p)| p.y)
        .expect("the layer is listed");
    let mut found = None;
    for y in (2..first_layer as i32).step_by(4) {
        let p = Pos2::new(2.0 * (TOOL + 2.0) + TOOL / 2.0, y as f32);
        h.click(p);
        if h.frame(vec![]).iter().any(|(t, _)| t == "Star") {
            found = Some(p);
            break;
        }
    }
    assert!(found.is_some(), "a toolbar button opens the shape menu");
    let star = h.label("Star");
    h.click(star);
    assert_eq!(h.added_shape, Some(ShapeKind::Star));
    // Last in the menu, paths come from SVG files.
    h.added_shape = None;
    h.click(found.unwrap());
    let path = h.label("Path...");
    h.click(path);
    assert_eq!(h.added_shape, Some(ShapeKind::Path));

    // A shape layer's Properties: its kind (shown, not changed), fill and stroke.
    let l = shape::new_layer(2, ShapeKind::Rectangle, [50.0, 50.0]);
    let mut h = Harness::new(doc_with(vec![Node::Layer(Box::new(l))]));
    h.selected = Some(2);
    let labels = h.frame(vec![]);
    for want in [
        "Shape",
        "Kind",
        "Corner radius",
        "Fill",
        "Stroke",
        "Dashes",
        "Joins",
    ] {
        assert!(labels.iter().any(|(t, _)| t == want), "{want}");
    }
    let kind = labels
        .iter()
        .filter(|(t, _)| t == "Rectangle")
        .map(|(_, p)| *p)
        .next_back()
        .expect("kind shown");
    h.click(kind + vec2(4.0, 6.0));
    assert!(
        !h.frame(vec![]).iter().any(|(t, _)| t == "Ellipse"),
        "no list of kinds"
    );
    let shape = h.doc.layer(2).unwrap().shape.as_ref().unwrap();
    assert_eq!(shape.kind, ShapeKind::Rectangle);

    // A polyline's points are edited on the canvas, turned on here too.
    let l = shape::new_layer(3, ShapeKind::Polyline, [50.0, 50.0]);
    let mut h = Harness::new(doc_with(vec![Node::Layer(Box::new(l))]));
    h.selected = Some(3);
    let button = h.label("Edit points");
    h.click(button);
    assert_eq!(h.state.edit_points, Some(3));
    h.click(button);
    assert_eq!(h.state.edit_points, None);
}

#[test]
fn text_path_in_properties() {
    let t = text::new_layer(1, "Hello", text::BUILTIN_FAMILY.into(), [50.0, 50.0]);
    let ring = shape::new_layer(2, ShapeKind::Ellipse, [50.0, 50.0]);
    let mut h = Harness::new(doc_with(vec![
        Node::Layer(Box::new(ring)),
        Node::Layer(Box::new(t)),
    ]));
    h.selected = Some(1);
    let labels = h.frame(vec![]);
    assert!(labels.iter().any(|(t, _)| t == "Path"));
    assert!(
        !labels.iter().any(|(t, _)| t == "Radius"),
        "straight: nothing more"
    );
    let straight = h.label("Straight");
    h.click(straight);
    let circle = h.label("Circle");
    h.click(circle);
    let labels = h.frame(vec![]);
    for want in ["Radius", "Position", "Other side", "Baseline shift"] {
        assert!(labels.iter().any(|(t, _)| t == want), "{want}");
    }
    let p = &h.doc.layer(1).unwrap().text.as_ref().unwrap().path;
    assert_eq!(p.kind, PathKind::Circle);
    assert!(p.radius >= 1.0);
    // Following a shape picks the first one.
    let circle = h.label("Circle");
    h.click(circle);
    let shape_kind = h.label("Shape layer");
    h.click(shape_kind);
    let p = &h.doc.layer(1).unwrap().text.as_ref().unwrap().path;
    assert_eq!((p.kind, p.shape), (PathKind::Shape, 2));
    assert!(h.frame(vec![]).iter().any(|(t, _)| t == "Follows"));
}

/// Draws the layer list with a layer linked to a variable gone, and a folded group
/// holding another, to the PNG named by `LAYER_LIST_PREVIEW`.
#[test]
#[ignore]
fn layer_list_preview() {
    let Ok(path) = std::env::var("LAYER_LIST_PREVIEW") else {
        return;
    };
    let mut fine = Layer::new(1, "Logo".into(), "".into());
    fine.size_px = [10, 10];
    let mut gone = Layer::new(2, "Button text".into(), "".into());
    gone.size_px = [10, 10];
    gone.bindings.insert(Field::X, "margin".into());
    let mut inner = gone.clone();
    inner.id = 3;
    inner.name = "Label".into();
    let doc = std::cell::RefCell::new(Document {
        layers: vec![
            Node::Group(crate::document::Group {
                id: 4,
                name: "Folded group".into(),
                visible: true,
                locked: false,
                opacity: 1.0,
                clip: false,
                blend: crate::document::BlendMode::Normal,
                expanded: false,
                children: vec![Node::Layer(Box::new(inner))],
                element: None,
            }),
            Node::Layer(Box::new(gone)),
            Node::Layer(Box::new(fine)),
        ],
        ..Default::default()
    });
    let state = std::cell::RefCell::new(LayersPanel::default());
    crate::ui::preview::render(&path, vec2(260.0, 330.0), |ui| {
        let mut selected = None;
        show(
            ui,
            &mut doc.borrow_mut(),
            &mut selected,
            &mut state.borrow_mut(),
        );
    });
}

/// The brush kinds' menu, as below the canvas tool bar's brush button, the
/// calligraphy brush picked, to look at their icons:
/// `BRUSH_MENU_PREVIEW=out.png cargo test brush_menu_preview -- --ignored`.
#[test]
#[ignore]
fn brush_menu_preview() {
    let Ok(path) = std::env::var("BRUSH_MENU_PREVIEW") else {
        return;
    };
    crate::ui::preview::render(&path, egui::vec2(220.0, 170.0), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for kind in crate::document::paint::BrushKind::ALL {
            let on = kind == crate::document::paint::BrushKind::Calligraphy;
            let labels = crate::document::paint::BrushKind::ALL.map(|k| translate(k.name_key()));
            icon_entry(
                ui,
                Tool::Brush(kind),
                translate(kind.name_key()),
                &labels,
                on,
            );
        }
    });
}

#[test]
fn position_size_rotation_and_tilt_fields_line_up_in_columns() {
    let mut h = Harness::new(doc_with(vec![named(1, "alpha")]));
    h.selected = Some(1);
    let labels = h.frame(vec![]);
    let row_of = |name: &str| labels.iter().find(|(t, _)| t == name).expect(name).1.y;
    // The fields' fx buttons, left to right, on the row of `name`.
    let fx = |name: &str| -> Vec<f32> {
        let y = row_of(name);
        let mut xs: Vec<f32> = labels
            .iter()
            .filter(|(t, p)| t == "fx" && (p.y - y).abs() < 6.0)
            .map(|(_, p)| p.x)
            .collect();
        xs.sort_by(f32::total_cmp);
        xs
    };
    let position = fx("Position");
    assert_eq!(position.len(), 2);
    for row in ["Size", "3D tilt"] {
        assert_eq!(fx(row), position, "{row}");
    }
    // Rotation, one field: in the first column.
    assert_eq!(fx("Rotation"), position[..1], "Rotation");
}

#[test]
fn new_layers_go_into_a_group_only_when_the_group_is_selected() {
    let mut d = doc_with(vec![
        layer(1),
        Node::Group(Group {
            id: 10,
            name: "g".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            clip: false,
            blend: crate::document::BlendMode::Normal,
            expanded: false,
            children: vec![layer(2), layer(3)],
            element: None,
        }),
        layer(4),
    ]);
    // The group selected: at its top.
    assert_eq!(insertion_path(&mut d, Some(10)), [1, 2]);
    // A layer in the group: above the group, at the top level.
    assert_eq!(insertion_path(&mut d, Some(2)), [2]);
    // A top-level layer: just above it.
    assert_eq!(insertion_path(&mut d, Some(1)), [1]);
    // Nothing selected: at the top of the stack.
    assert_eq!(insertion_path(&mut d, None), [3]);
}

#[test]
fn a_wide_row_spans_the_grid_from_its_label_column_indented() {
    let ctx = egui::Context::default();
    let (mut label_x, mut wide) = (0.0, egui::Rect::NOTHING);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        super::properties::prop_grid(ui, "wide").show(ui, |ui| {
            label_x = ui.label("Label").rect.left();
            ui.label("value");
            ui.end_row();
            wide = super::properties::wide_row(ui, |ui| ui.label("settings").rect);
            ui.label("Next");
            ui.end_row();
        });
    });
    out.textures_delta.clear();
    assert_eq!(wide.left(), label_x + crate::ui::tokens::INDENT);
}
