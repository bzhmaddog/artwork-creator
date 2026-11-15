#[test]
fn a_click_in_the_template_editor_picks_the_hole_under_it() {
    use crate::template::ElementKind;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let entry = crate::template::scan(&dir)
        .into_iter()
        .find(|t| t.id.contains("T16"))
        .expect("Cosmox T16");
    let mut doc = crate::template::edit::open(&entry, entry.config.clone()).unwrap();
    let of_kind = |doc: &crate::document::Document, kind| {
        doc.leaves()
            .iter()
            .find(|l| {
                crate::template::edit::element_of(doc, l.layer.id).is_some_and(|t| t.kind == kind)
            })
            .map(|l| (l.layer.id, l.layer.transform.center))
            .unwrap()
    };
    let (outline, _) = of_kind(&doc, ElementKind::Outline);
    let (hole, at) = of_kind(&doc, ElementKind::Holes);
    assert_eq!(super::layer_to_select(&doc, at), Some(hole));
    // In a project, the topmost layer: the outline, at the top of the list.
    let (_, at) = of_kind(&doc, ElementKind::Holes);
    doc.edited_template = None;
    assert_eq!(super::layer_to_select(&doc, at), Some(outline));
}
use super::draw::paint_safe_area;
use super::rulers::ruler_step;
use super::*;

#[test]
fn safe_area_guide_is_inside_the_outline_and_around_holes() {
    use crate::template::Polyline;
    let rect_poly = |lo: P2, hi: P2| Polyline {
        points: vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
        closed: true,
    };
    let element = |kind, poly| PlacedElement {
        kind,
        polylines: vec![poly],
        centers: Vec::new(),
    };
    let elements = [
        element(ElementKind::Outline, rect_poly([10.0, 10.0], [90.0, 40.0])),
        element(ElementKind::Holes, rect_poly([20.0, 20.0], [30.0, 30.0])),
    ];
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let run = |state: &mut CanvasState| {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_safe_area(ui.painter(), &elements, state, rect);
        });
        out.textures_delta.clear();
    };
    run(&mut state);
    assert!(state.safe_area.is_none(), "no guide without a margin");
    state.safe_margin_mm = 2.0;
    run(&mut state);
    let (_, rings) = state.safe_area.as_ref().unwrap();
    assert_eq!(rings.len(), 2);
    let b = |r: &Vec<P2>| {
        crate::export::marks::bounds(&[Polyline {
            points: r.clone(),
            closed: true,
        }])
        .unwrap()
    };
    let mut boxes: Vec<_> = rings.iter().map(b).collect();
    boxes.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    let near = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    assert!(near(boxes[0].0, [12.0, 12.0]) && near(boxes[0].1, [88.0, 38.0]));
    assert!(near(boxes[1].0, [18.0, 18.0]) && near(boxes[1].1, [32.0, 32.0]));
}

#[test]
fn display_settings_come_back_next_session() {
    #[derive(Default)]
    struct Memory(HashMap<String, String>);
    impl eframe::Storage for Memory {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }
    let mut storage = Memory::default();
    let fresh = CanvasState::load(Some(&storage));
    assert!(fresh.preview_cuts && fresh.preview_bleed && fresh.preview_safe_area);
    let state = CanvasState {
        preview_cuts: false,
        cut_opacity: 0.4,
        preview_bleed: false,
        preview_safe_area: false,
        safe_margin_mm: 3.0,
        show_template: false,
        show_rulers: false,
        show_guides: false,
        ..Default::default()
    };
    state.save(&mut storage);
    let back = CanvasState::load(Some(&storage));
    assert!(!back.preview_cuts && !back.preview_bleed && !back.preview_safe_area);
    assert!(!back.show_template && !back.show_rulers && !back.show_guides);
    assert_eq!((back.cut_opacity, back.safe_margin_mm), (0.4, 3.0));
}

#[test]
fn the_safe_area_preview_can_be_turned_off() {
    use crate::template::Polyline;
    let elements = [PlacedElement {
        kind: ElementKind::Outline,
        polylines: vec![Polyline {
            points: vec![[10.0, 10.0], [90.0, 10.0], [90.0, 40.0], [10.0, 40.0]],
            closed: true,
        }],
        centers: Vec::new(),
    }];
    let ctx = egui::Context::default();
    let mut doc = Document::default();
    let mut selected = None;
    let mut frame = |state: &mut CanvasState| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, &mut doc, &elements, &mut selected, state);
        });
        out.textures_delta.clear();
    };
    let mut state = CanvasState {
        safe_margin_mm: 2.0,
        preview_safe_area: false,
        ..Default::default()
    };
    frame(&mut state);
    assert!(state.safe_area.is_none(), "hidden: not drawn");
    state.preview_safe_area = true;
    frame(&mut state);
    assert!(state.safe_area.is_some(), "shown");
}

#[test]
fn dragging_a_member_of_the_selected_group_moves_the_group() {
    use crate::document::Group;
    let square = |id: u64, x: f32| {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [10, 10];
        l.transform.center = [x, 50.0];
        Node::Layer(Box::new(l))
    };
    let mut doc = Document {
        layers: vec![
            Node::Group(Group {
                id: 1,
                name: "g".into(),
                visible: true,
                locked: false,
                opacity: 0.5,
                clip: false,
                blend: crate::document::BlendMode::Normal,
                expanded: true,
                children: vec![square(2, 50.0), square(3, 100.0)],
                element: None,
            }),
            square(4, 200.0),
        ],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = Some(1);
    let mut time = 0.0;
    let mut frame =
        |doc: &mut Document, sel: &mut Option<u64>, state: &mut CanvasState, events: Vec<Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], sel, state);
            });
            out.textures_delta.clear();
        };
    frame(&mut doc, &mut selected, &mut state, vec![]);
    let cam = state.camera;
    let rect = state.rect;
    let from = cam.to_screen(rect, [50.0, 50.0]);
    let to = cam.to_screen(rect, [60.0, 55.0]);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(
        &mut doc,
        &mut selected,
        &mut state,
        vec![Event::PointerMoved(from)],
    );
    frame(
        &mut doc,
        &mut selected,
        &mut state,
        vec![button(from, true)],
    );
    for k in 1..=4 {
        let p = from + (to - from) * (k as f32 / 4.0);
        frame(
            &mut doc,
            &mut selected,
            &mut state,
            vec![Event::PointerMoved(p)],
        );
    }
    frame(&mut doc, &mut selected, &mut state, vec![button(to, false)]);
    assert_eq!(selected, Some(1), "group stays selected");
    let c = |id| doc.layer(id).unwrap().transform.center;
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01;
    assert!(close(c(2), [60.0, 55.0]), "{:?}", c(2));
    assert!(close(c(3), [110.0, 55.0]), "{:?}", c(3));
    assert!(close(c(4), [200.0, 50.0]), "outside the group stays put");
}

#[test]
fn the_eyedropper_takes_the_artworks_color_without_selecting() {
    let img = image::RgbaImage::from_pixel(10, 10, image::Rgba([200, 30, 40, 255]));
    let pixmap = std::sync::Arc::new(render::premultiplied_pixmap(&img).unwrap());
    let mut l = Layer::new(1, "red".into(), "x.png".into());
    l.size_px = [10, 10];
    l.transform.center = [50.0, 50.0];
    l.pixmap = Some(pixmap.clone());
    l.preview = Some(pixmap);
    let mut doc = Document {
        layers: vec![Node::Layer(Box::new(l))],
        background: [10, 20, 30],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let selected = std::cell::RefCell::new(None);
    let mut time = 0.0;
    let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
        time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &[], &mut selected.borrow_mut(), state);
        });
        out.textures_delta.clear();
        out.platform_output
    };
    frame(&mut doc, &mut state, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let mut click = |doc: &mut Document, state: &mut CanvasState, at: P2, ctrl: bool| {
        let pos = cam.to_screen(rect, at);
        let modifiers = if ctrl {
            egui::Modifiers::COMMAND
        } else {
            Default::default()
        };
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        let held = Event::ModifiersChanged(modifiers);
        frame(doc, state, vec![held, Event::PointerMoved(pos)]);
        frame(doc, state, vec![button(true)]);
        frame(doc, state, vec![button(false)])
    };

    state.eyedropper = true;
    let out = click(&mut doc, &mut state, [50.0, 50.0], false);
    assert_eq!(state.brushes.color, [200, 30, 40, 255], "the layer's color");
    assert!(
        state.picked.is_empty(),
        "a click doesn't add it to the colors in use"
    );
    assert!(
        out.commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "#C81E28")),
        "its code copied: {:?}",
        out.commands
    );
    assert_eq!(*selected.borrow(), None, "the layer clicked isn't selected");
    assert!(state.eyedropper, "still in hand");

    click(&mut doc, &mut state, [20.0, 20.0], true);
    assert_eq!(state.brushes.color, [10, 20, 30, 255]);
    assert_eq!(
        state.picked,
        [[10, 20, 30, 255]],
        "Ctrl+click: the page's background added to the colors in use"
    );
    // Another one is added before it; one added again moves first, once.
    click(&mut doc, &mut state, [50.0, 50.0], true);
    assert_eq!(state.picked, [[200, 30, 40, 255], [10, 20, 30, 255]]);
    click(&mut doc, &mut state, [20.0, 20.0], true);
    assert_eq!(state.picked, [[10, 20, 30, 255], [200, 30, 40, 255]]);

    // Off the page, nothing is drawn: nothing taken.
    click(&mut doc, &mut state, [-20.0, -20.0], true);
    assert_eq!(state.picked, [[10, 20, 30, 255], [200, 30, 40, 255]]);

    let esc = Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    frame(&mut doc, &mut state, vec![esc]);
    assert!(!state.eyedropper, "Esc: back to selecting");
}

#[test]
fn a_canvas_is_placed_by_a_drag_then_painted_in_and_erased() {
    use crate::document::paint::Tool;
    let mut doc = Document::default();
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = None;
    let mut time = 0.0;
    let mut frame =
        |doc: &mut Document, sel: &mut Option<u64>, state: &mut CanvasState, events: Vec<Event>| {
            time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                show(ui, doc, &[], sel, state);
            });
            out.textures_delta.clear();
        };
    frame(&mut doc, &mut selected, &mut state, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut drag =
        |doc: &mut Document, sel: &mut Option<u64>, state: &mut CanvasState, from: P2, to: P2| {
            let (a, b) = (cam.to_screen(rect, from), cam.to_screen(rect, to));
            frame(doc, sel, state, vec![Event::PointerMoved(a)]);
            frame(doc, sel, state, vec![button(a, true)]);
            for k in 1..=10 {
                let p = a + (b - a) * (k as f32 / 10.0);
                frame(doc, sel, state, vec![Event::PointerMoved(p)]);
            }
            frame(doc, sel, state, vec![button(b, false)]);
            frame(doc, sel, state, vec![]);
        };
    let strokes = |doc: &Document, id: u64| {
        doc.layer(id)
            .unwrap()
            .shape
            .as_ref()
            .unwrap()
            .strokes
            .clone()
    };

    // No Canvas selected: the pencil waits, a click paints nothing (a drag on the
    // empty page would pan it).
    state.brushes.tool = Some(Tool::Pencil);
    drag(
        &mut doc,
        &mut selected,
        &mut state,
        [50.0, 100.0],
        [50.0, 100.0],
    );
    assert_eq!(
        state.brushes.tool,
        Some(Tool::Pencil),
        "kept for the next Canvas"
    );
    assert!(doc.layers.is_empty());

    // A Canvas selected, as the Canvas button leaves it.
    doc.layers.push(Node::Layer(Box::new(
        crate::document::shape::new_canvas_layer(
            7,
            crate::document::shape::ShapeKind::Rectangle,
            [40.0, 80.0],
            [160.0, 140.0],
        ),
    )));
    // The pencil picked before is in hand.
    selected = Some(7);
    let id = 7;
    let size = doc.layer(id).unwrap().shape.as_ref().unwrap().size;

    drag(
        &mut doc,
        &mut selected,
        &mut state,
        [50.0, 100.0],
        [150.0, 100.0],
    );
    assert_eq!(strokes(&doc, id).len(), 1);
    let s = &strokes(&doc, id)[0];
    assert_eq!(s.points.len(), 2, "a straight line: its two ends");
    // Painted where the pointer went: its ends on the page.
    let ends: Vec<P2> = s
        .points
        .iter()
        .map(|p| crate::document::shape::to_page(doc.layer(id).unwrap(), [p[0], p[1]]).unwrap())
        .collect();
    assert!(
        (ends[0][0] - 50.0).abs() < 0.5 && (ends[1][0] - 150.0).abs() < 0.5,
        "{ends:?}"
    );

    // The next stroke goes into it too; the canvas keeps its size.
    drag(
        &mut doc,
        &mut selected,
        &mut state,
        [50.0, 120.0],
        [150.0, 120.0],
    );
    assert_eq!(selected, Some(id));
    assert_eq!(strokes(&doc, id).len(), 2);
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.layer(id).unwrap().shape.as_ref().unwrap().size, size);
    // A stroke all outside it is left out.
    drag(
        &mut doc,
        &mut selected,
        &mut state,
        [50.0, 200.0],
        [150.0, 200.0],
    );
    assert_eq!(strokes(&doc, id).len(), 2);

    // The eraser across the first stroke, between both: it is cut in two.
    state.brushes.tool = Some(Tool::Eraser);
    drag(
        &mut doc,
        &mut selected,
        &mut state,
        [100.0, 90.0],
        [100.0, 110.0],
    );
    assert_eq!(strokes(&doc, id).len(), 3);
}

#[test]
fn locked_layers_are_selected_by_a_click_but_not_moved() {
    let square = |id: u64, x: f32, size: u32, locked: bool| {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [size, size];
        l.transform.center = [x, 50.0];
        l.locked = locked;
        Node::Layer(Box::new(l))
    };
    // A locked background under an unlocked square, and a locked square on top of it.
    let mut doc = Document {
        layers: vec![
            square(1, 100.0, 150, true),
            square(2, 60.0, 10, false),
            square(3, 60.0, 4, true),
        ],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = None;
    let mut time = 0.0;
    let mut frame = |doc: &mut Document, sel: &mut Option<u64>, events: Vec<Event>| {
        time += 0.5;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &[], sel, &mut state);
        });
        out.textures_delta.clear();
        (state.camera, state.rect)
    };
    let (cam, rect) = frame(&mut doc, &mut selected, vec![]);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut click = |doc: &mut Document, sel: &mut Option<u64>, at: P2| {
        let p = cam.to_screen(rect, at);
        frame(doc, sel, vec![Event::PointerMoved(p)]);
        frame(doc, sel, vec![button(p, true)]);
        frame(doc, sel, vec![button(p, false)]);
    };
    // Only the locked background there: selected.
    click(&mut doc, &mut selected, [140.0, 50.0]);
    assert_eq!(selected, Some(1));
    // The locked square over the unlocked one lets the click through to it.
    click(&mut doc, &mut selected, [60.0, 50.0]);
    assert_eq!(selected, Some(2));

    // Dragging the locked background selects it, and it stays put.
    let (from, to) = (
        cam.to_screen(rect, [140.0, 50.0]),
        cam.to_screen(rect, [150.0, 60.0]),
    );
    frame(&mut doc, &mut selected, vec![Event::PointerMoved(from)]);
    frame(&mut doc, &mut selected, vec![button(from, true)]);
    for k in 1..=4 {
        let p = from + (to - from) * (k as f32 / 4.0);
        frame(&mut doc, &mut selected, vec![Event::PointerMoved(p)]);
    }
    frame(&mut doc, &mut selected, vec![button(to, false)]);
    assert_eq!(selected, Some(1));
    assert_eq!(doc.layer(1).unwrap().transform.center, [100.0, 50.0]);
}

#[test]
fn a_pen_pressing_lightly_paints_thinner() {
    use crate::document::paint::Tool;
    let mut doc = Document::default();
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    state.brushes.tool = Some(Tool::Pencil);
    *state.brushes.width_mut(Tool::Pencil) = 2.0;
    doc.layers.push(Node::Layer(Box::new(
        crate::document::shape::new_canvas_layer(
            1,
            crate::document::shape::ShapeKind::Rectangle,
            [0.0, 50.0],
            [200.0, 150.0],
        ),
    )));
    let mut selected = Some(1);
    let mut time = 0.0;
    let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
        time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &[], &mut selected, state);
        });
        out.textures_delta.clear();
    };
    frame(&mut doc, &mut state, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let (a, b) = (
        cam.to_screen(rect, [50.0, 100.0]),
        cam.to_screen(rect, [150.0, 100.0]),
    );
    // A pen: touch events with its pressure, alongside the pointer it moves.
    let touch = |pos, phase| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase,
        pos,
        force: Some(0.5),
    };
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(&mut doc, &mut state, vec![Event::PointerMoved(a)]);
    frame(
        &mut doc,
        &mut state,
        vec![touch(a, egui::TouchPhase::Start), button(a, true)],
    );
    for k in 1..=10 {
        let p = a + (b - a) * (k as f32 / 10.0);
        frame(
            &mut doc,
            &mut state,
            vec![touch(p, egui::TouchPhase::Move), Event::PointerMoved(p)],
        );
    }
    frame(
        &mut doc,
        &mut state,
        vec![touch(b, egui::TouchPhase::End), button(b, false)],
    );
    let layer = doc.leaves()[0].layer;
    let s = &layer.shape.as_ref().unwrap().strokes[0];
    assert!(
        s.points.iter().all(|p| (p[2] - 1.0).abs() < 0.01),
        "{:?}",
        s.points
    );
}

#[test]
fn clear_paint_empties_the_selected_shape_only() {
    use crate::document::paint::{Pen, Tool};
    let mut l = crate::document::shape::new_layer(
        1,
        crate::document::shape::ShapeKind::Rectangle,
        [50.0, 50.0],
    );
    let line = [[45.0, 50.0, 1.0], [55.0, 50.0, 1.0]];
    assert!(crate::document::shape::add_stroke(
        &mut l,
        &line,
        &Pen::new(Tool::Pencil, 1.0, [0; 4])
    ));
    let mut doc = Document {
        layers: vec![Node::Layer(Box::new(l))],
        ..Default::default()
    };
    assert!(!clear_paint(&mut doc, None));
    assert!(has_paint(&doc, Some(1)));
    assert!(clear_paint(&mut doc, Some(1)));
    assert!(
        doc.layer(1)
            .unwrap()
            .shape
            .as_ref()
            .unwrap()
            .strokes
            .is_empty()
    );
    // Nothing left: nothing changes.
    assert!(!has_paint(&doc, Some(1)) && !clear_paint(&mut doc, Some(1)));
}

/// A stroke of the pencil and of each kind of brush painted in an ellipse, one cut
/// by the eraser, the strokes cut off at its outline, under its stroke, with the
/// tool bar and the calligraphy brush in use, to look at them:
/// `PAINT_PREVIEW=out.png cargo test paint_preview -- --ignored`.
#[test]
#[ignore]
fn paint_preview() {
    use crate::document::paint::{BrushKind, Pen, Tool};
    let Ok(path) = std::env::var("PAINT_PREVIEW") else {
        return;
    };
    let mut layer = crate::document::shape::new_canvas_layer(
        1,
        crate::document::shape::ShapeKind::Rectangle,
        [40.0, 40.0],
        [200.0, 150.0],
    );
    // Its background and border: the paint goes between them.
    crate::document::shape::set_kind(&mut layer, crate::document::shape::ShapeKind::Ellipse);
    if let Some(s) = layer.shape.as_deref_mut() {
        (s.fill, s.fill_color) = (true, [250, 240, 200, 255]);
        (s.stroke, s.stroke_width) = (true, 2.0);
    }
    let wave = |y0: f32, amp: f32| -> Vec<[f32; 3]> {
        (0..=120)
            .map(|i| {
                let x = 40.0 + i as f32 * 1.4;
                [x, y0 + amp * (x / 18.0).sin(), 1.0]
            })
            .collect()
    };
    let strokes = [
        (Pen::new(Tool::Pencil, 0.8, [20, 20, 20, 255]), 55.0),
        (Pen::brush(BrushKind::Round, 8.0, [200, 40, 60, 255]), 70.0),
        (
            Pen::brush(BrushKind::Calligraphy, 7.0, [20, 20, 90, 255]),
            85.0,
        ),
        (
            Pen::brush(BrushKind::Marker, 8.0, [240, 200, 0, 255]),
            100.0,
        ),
        (Pen::brush(BrushKind::Spray, 9.0, [30, 140, 60, 255]), 115.0),
        (
            Pen::brush(BrushKind::Bristle, 8.0, [120, 60, 20, 255]),
            132.0,
        ),
    ];
    for (pen, y) in strokes {
        assert!(crate::document::shape::add_stroke(
            &mut layer,
            &wave(y, 6.0),
            &pen
        ));
    }
    crate::document::shape::erase(&mut layer, &[[120.0, 40.0], [125.0, 80.0]], 2.0);
    let doc = std::cell::RefCell::new(Document {
        layers: vec![Node::Layer(Box::new(layer))],
        ..Default::default()
    });
    let mut st = CanvasState::default();
    st.brushes.tool = Some(Tool::Brush);
    st.brushes.brush = BrushKind::Calligraphy;
    st.brushes.color = [200, 40, 60, 255];
    let state = std::cell::RefCell::new(st);
    // The preview draws each theme with a context of its own: the layer's texture
    // is made again for each.
    let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
    crate::ui::preview::render(&path, vec2(760.0, 560.0), |ui| {
        let mut doc = doc.borrow_mut();
        if last.borrow().as_ref() != Some(ui.ctx()) {
            doc.for_each_layer_mut(|l| l.pixmap = None);
            *last.borrow_mut() = Some(ui.ctx().clone());
        }
        crate::document::shape::sync(ui.ctx(), &mut doc, true);
        let mut sel = Some(1);
        show(ui, &mut doc, &[], &mut sel, &mut state.borrow_mut());
    });
}

/// Compares two outlines on the canvas, the divider in the middle, in both themes:
/// `COMPARE_PREVIEW=out.png cargo test compare_preview -- --ignored`.
#[test]
#[ignore]
fn compare_preview() {
    let Ok(path) = std::env::var("COMPARE_PREVIEW") else {
        return;
    };
    let doc = std::cell::RefCell::new(Document {
        layers: vec![Node::Layer(Box::new(crate::document::shape::new_layer(
            1,
            crate::document::shape::ShapeKind::Ellipse,
            [148.5, 105.0],
        )))],
        ..Default::default()
    });
    let state = std::cell::RefCell::new(CanvasState::default());
    let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
    crate::ui::preview::render(&path, vec2(760.0, 560.0), |ui| {
        let mut doc = doc.borrow_mut();
        let mut state = state.borrow_mut();
        // Textures belong to their context: made again for each.
        if last.borrow().as_ref() != Some(ui.ctx()) {
            doc.for_each_layer_mut(|l| l.pixmap = None);
            *state = CanvasState::default();
            *last.borrow_mut() = Some(ui.ctx().clone());
        }
        crate::document::shape::sync(ui.ctx(), &mut doc, true);
        state.compare = Some(Compare {
            elements: outline([30.0, 30.0], [267.0, 180.0]),
            colors: Vec::new(),
            labels: ["Project (1.0)".into(), "Installed (1.1)".into()],
            on_left: false,
        });
        let mut sel = None;
        show(
            ui,
            &mut doc,
            &outline([50.0, 40.0], [247.0, 170.0]),
            &mut sel,
            &mut state,
        );
    });
}

/// Draws the template editor comparing T16 opened for editing, one button moved,
/// with T16 as installed, in both themes:
/// `SOURCE_COMPARE_PREVIEW=out.png cargo test source_compare_preview -- --ignored`.
#[test]
#[ignore]
fn source_compare_preview() {
    let Ok(path) = std::env::var("SOURCE_COMPARE_PREVIEW") else {
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let entry = crate::template::scan(&dir)
        .into_iter()
        .find(|t| t.id.contains("T16"))
        .expect("Cosmox T16");
    let source = crate::template::load(&entry).unwrap();
    let mut doc = crate::template::edit::open(&entry, entry.config.clone()).unwrap();
    // Moved: the comparison shows it.
    let moved = doc
        .leaves()
        .iter()
        .find(|l| {
            crate::template::edit::element_of(&doc, l.layer.id)
                .is_some_and(|t| t.kind == crate::template::ElementKind::Holes)
        })
        .map(|l| l.layer.id)
        .unwrap();
    doc.for_each_layer_mut(|l| {
        if l.id == moved {
            l.transform.center[0] += 15.0;
        }
    });
    let elements = crate::render::placed_elements(&doc, &source);
    let doc = std::cell::RefCell::new(doc);
    let state = std::cell::RefCell::new(CanvasState::default());
    let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
    crate::ui::preview::render(&path, vec2(900.0, 640.0), |ui| {
        let mut doc = doc.borrow_mut();
        let mut state = state.borrow_mut();
        if last.borrow().as_ref() != Some(ui.ctx()) {
            doc.for_each_layer_mut(|l| l.pixmap = None);
            *state = CanvasState::default();
            state.fit_pending = true;
            *last.borrow_mut() = Some(ui.ctx().clone());
        }
        crate::document::shape::sync(ui.ctx(), &mut doc, true);
        state.compare = Some(Compare {
            elements: elements.clone(),
            colors: Vec::new(),
            labels: ["Source (Cosmox - T16 - v1.1)".into(), "Being edited".into()],
            on_left: true,
        });
        let mut sel = None;
        show(ui, &mut doc, &[], &mut sel, &mut state);
    });
}

/// Draws the rulers and two guides, in both themes:
/// `RULERS_PREVIEW=out.png cargo test rulers_preview -- --ignored`.
#[test]
#[ignore]
fn rulers_preview() {
    let Ok(path) = std::env::var("RULERS_PREVIEW") else {
        return;
    };
    let mut doc = Document {
        layers: vec![Node::Layer(Box::new(crate::document::shape::new_layer(
            1,
            crate::document::shape::ShapeKind::Rectangle,
            [148.5, 105.0],
        )))],
        ..Default::default()
    };
    doc.guides = vec![
        Guide {
            vertical: true,
            at: 60.0,
        },
        Guide {
            vertical: false,
            at: 150.0,
        },
    ];
    let doc = std::cell::RefCell::new(doc);
    let state = std::cell::RefCell::new(CanvasState::default());
    let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
    crate::ui::preview::render(&path, vec2(760.0, 560.0), |ui| {
        let mut doc = doc.borrow_mut();
        if last.borrow().as_ref() != Some(ui.ctx()) {
            doc.for_each_layer_mut(|l| l.pixmap = None);
            *last.borrow_mut() = Some(ui.ctx().clone());
        }
        crate::document::shape::sync(ui.ctx(), &mut doc, true);
        let mut sel = None;
        show(ui, &mut doc, &[], &mut sel, &mut state.borrow_mut());
    });
}

/// A rectangle under a square outline with a square button hole, the cut preview
/// covering and hatching what is cut away, to look at it:
/// `CUTS_PREVIEW=out.png cargo test cuts_preview -- --ignored`.
#[test]
#[ignore]
fn cuts_preview() {
    use crate::template::Polyline;
    let Ok(path) = std::env::var("CUTS_PREVIEW") else {
        return;
    };
    let mut l = crate::document::shape::new_layer(
        1,
        crate::document::shape::ShapeKind::Rectangle,
        [100.0, 80.0],
    );
    crate::document::shape::set_geometry_size(&mut l, [180.0, 140.0]);
    let doc = std::cell::RefCell::new(Document {
        layers: vec![Node::Layer(Box::new(l))],
        ..Default::default()
    });
    let square = |c: P2, r: f32| Polyline {
        points: vec![
            [c[0] - r, c[1] - r],
            [c[0] + r, c[1] - r],
            [c[0] + r, c[1] + r],
            [c[0] - r, c[1] + r],
        ],
        closed: true,
    };
    let element = |kind, poly| PlacedElement {
        kind,
        polylines: vec![poly],
        centers: Vec::new(),
    };
    let elements = vec![
        element(ElementKind::Outline, square([100.0, 80.0], 50.0)),
        element(ElementKind::Holes, square([100.0, 80.0], 15.0)),
    ];
    let state = std::cell::RefCell::new(CanvasState::default());
    // Each theme is drawn with a context of its own: textures are made again.
    let last: std::cell::RefCell<Option<egui::Context>> = Default::default();
    crate::ui::preview::render(&path, vec2(500.0, 400.0), |ui| {
        let mut doc = doc.borrow_mut();
        if last.borrow().as_ref() != Some(ui.ctx()) {
            doc.for_each_layer_mut(|l| l.pixmap = None);
            *state.borrow_mut() = CanvasState::default();
            *last.borrow_mut() = Some(ui.ctx().clone());
        }
        crate::document::shape::sync(ui.ctx(), &mut doc, true);
        let mut sel = None;
        show(ui, &mut doc, &elements, &mut sel, &mut state.borrow_mut());
    });
}

/// A selected locked layer, with its padlock badge, to look at it:
/// `LOCK_PREVIEW=out.png cargo test lock_preview -- --ignored`.
#[test]
#[ignore]
fn lock_preview() {
    let Ok(path) = std::env::var("LOCK_PREVIEW") else {
        return;
    };
    let mut l = Layer::new(1, "l".into(), "x.png".into());
    l.size_px = [80, 50];
    l.transform.center = [100.0, 80.0];
    l.transform.rotation_deg = 10.0;
    l.locked = true;
    let doc = std::cell::RefCell::new(Document {
        layers: vec![Node::Layer(Box::new(l))],
        ..Default::default()
    });
    let state = std::cell::RefCell::new(CanvasState::default());
    crate::ui::preview::render(&path, vec2(500.0, 360.0), |ui| {
        let mut sel = Some(1);
        show(
            ui,
            &mut doc.borrow_mut(),
            &[],
            &mut sel,
            &mut state.borrow_mut(),
        );
    });
}

#[test]
fn the_group_handle_turns_the_group() {
    let square = |id: u64, x: f32| {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [20, 20];
        l.transform.center = [x, 100.0];
        Node::Layer(Box::new(l))
    };
    let mut doc = Document {
        layers: vec![Node::Group(crate::document::Group {
            id: 1,
            name: "g".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            clip: false,
            blend: crate::document::BlendMode::Normal,
            expanded: true,
            children: vec![square(2, 80.0), square(3, 120.0)],
            element: None,
        })],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = Some(1);
    let mut time = 0.0;
    let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
        time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &[], &mut selected, state);
        });
        out.textures_delta.clear();
    };
    frame(&mut doc, &mut state, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let (_, from) = group_rotate_handle(group_box(&doc, &cam, rect, 1).unwrap());
    // From straight above the middle (100, 100) to straight right of it: a quarter
    // turn clockwise.
    let mid = cam.to_screen(rect, [100.0, 100.0]);
    let to = mid + vec2(from.distance(mid), 0.0);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(&mut doc, &mut state, vec![Event::PointerMoved(from)]);
    frame(&mut doc, &mut state, vec![button(from, true)]);
    for k in 1..=8 {
        let a = std::f32::consts::FRAC_PI_2 * k as f32 / 8.0;
        let r = from.distance(mid);
        let p = mid + vec2(a.sin(), -a.cos()) * r;
        frame(&mut doc, &mut state, vec![Event::PointerMoved(p)]);
    }
    frame(&mut doc, &mut state, vec![button(to, false)]);
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let (l, r) = (doc.layer(2).unwrap(), doc.layer(3).unwrap());
    assert!(
        close(l.transform.center, [100.0, 80.0]),
        "{:?}",
        l.transform.center
    );
    assert!(
        close(r.transform.center, [100.0, 120.0]),
        "{:?}",
        r.transform.center
    );
    assert!((l.transform.rotation_deg - 90.0).abs() < 0.1);
}

#[test]
fn ctrl_drags_one_corner() {
    let mut l = Layer::new(1, "l".into(), "x.png".into());
    l.size_px = [100, 50];
    l.transform.center = [100.0, 100.0];
    l.transform.rotation_deg = 30.0;
    let mut doc = Document {
        layers: vec![Node::Layer(Box::new(l))],
        ..Default::default()
    };
    let before = doc.layer(1).unwrap().corners();
    let target = [before[1][0] - 10.0, before[1][1] + 8.0];
    canvas_drag(&mut doc, before[1], target, egui::Modifiers::COMMAND);

    let after = doc.layer(1).unwrap().corners();
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    assert!(close(after[1], target), "{:?} vs {:?}", after[1], target);
    for i in [0, 2, 3] {
        assert!(close(after[i], before[i]), "corner {i} stays put");
    }
    assert_eq!(doc.layer(1).unwrap().transform.scale, [1.0, 1.0]);
}

/// Drags on the canvas from one page point to another, with the layer selected.
fn canvas_drag(doc: &mut Document, from: P2, to: P2, modifiers: egui::Modifiers) {
    canvas_drag_in(doc, from, to, modifiers, &[]);
}

/// `canvas_drag` with the template's `elements` on the page.
fn canvas_drag_in(
    doc: &mut Document,
    from: P2,
    to: P2,
    modifiers: egui::Modifiers,
    elements: &[PlacedElement],
) {
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = Some(1);
    let mut time = 0.0;
    let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
        time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, elements, &mut selected, state);
        });
        out.textures_delta.clear();
    };
    frame(doc, &mut state, vec![Event::ModifiersChanged(modifiers)]);
    let (cam, rect) = (state.camera, state.rect);
    let (from, to) = (cam.to_screen(rect, from), cam.to_screen(rect, to));
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    };
    frame(doc, &mut state, vec![Event::PointerMoved(from)]);
    frame(doc, &mut state, vec![button(from, true)]);
    for k in 1..=4 {
        let p = from + (to - from) * (k as f32 / 4.0);
        frame(doc, &mut state, vec![Event::PointerMoved(p)]);
    }
    frame(doc, &mut state, vec![button(to, false)]);
}

/// A 100 x 50 mm image centered at (100, 100): corners (50, 75) to (150, 125).
fn snap_doc() -> Document {
    let mut l = Layer::new(1, "l".into(), "x.png".into());
    l.size_px = [100, 50];
    l.transform.center = [100.0, 100.0];
    Document {
        layers: vec![Node::Layer(Box::new(l))],
        ..Default::default()
    }
}

/// The canvas run frame by frame on a 1000 x 800 window, with layer 1 selected.
struct Canvas {
    ctx: egui::Context,
    time: f64,
    selected: Option<u64>,
    state: CanvasState,
}

impl Canvas {
    fn new(doc: &mut Document) -> Self {
        let mut c = Self {
            ctx: egui::Context::default(),
            time: 0.0,
            selected: Some(1),
            state: CanvasState::default(),
        };
        c.frame(doc, Vec::new());
        c
    }

    /// One frame; returns the cursor it left.
    fn frame(&mut self, doc: &mut Document, events: Vec<Event>) -> CursorIcon {
        self.time += 0.02;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (selected, state) = (&mut self.selected, &mut self.state);
        let mut out = self.ctx.run_ui(input, |ui| {
            show(ui, doc, &[], selected, state);
        });
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    }

    /// A page point on screen.
    fn screen(&self, p: P2) -> Pos2 {
        self.state.camera.to_screen(self.state.rect, p)
    }

    /// A point in the top ruler above page x, or in the left one beside page y.
    fn on_ruler(&self, vertical: bool, at: f32) -> Pos2 {
        let full = self.state.full_rect;
        if vertical {
            pos2(full.left() + RULER / 2.0, self.screen([0.0, at]).y)
        } else {
            pos2(self.screen([at, 0.0]).x, full.top() + RULER / 2.0)
        }
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Presses at `from`, moves to `to` in a few steps and lets go there.
    fn drag(&mut self, doc: &mut Document, from: Pos2, to: Pos2) {
        self.frame(doc, vec![Event::PointerMoved(from)]);
        self.frame(doc, vec![Self::button(from, true)]);
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            self.frame(doc, vec![Event::PointerMoved(p)]);
        }
        self.frame(doc, vec![Self::button(to, false)]);
        self.frame(doc, Vec::new());
    }

    fn double_click(&mut self, doc: &mut Document, at: Pos2) {
        // Long after any earlier click, so it isn't taken for a triple click.
        self.time += 1.0;
        self.frame(doc, vec![Event::PointerMoved(at)]);
        for _ in 0..2 {
            self.frame(doc, vec![Self::button(at, true)]);
            self.frame(doc, vec![Self::button(at, false)]);
        }
        self.frame(doc, Vec::new());
    }
}

#[test]
fn the_handle_under_the_pointer_is_highlighted_and_rotation_has_its_cursor() {
    use super::handles::{Hot, edge_handles, handles};
    let mut doc = snap_doc();
    let mut c = Canvas::new(&mut doc);
    c.frame(&mut doc, Vec::new());
    let layer = doc.layer(1).unwrap().clone();
    let (corners, _, rot) = handles(&c.state.camera, c.state.rect, &layer);
    let edges = edge_handles(&c.state.camera, c.state.rect, &layer);
    for (at, hot) in [
        (corners[2], Hot::Corner(2)),
        (edges[1], Hot::Edge(1)),
        (rot, Hot::Rotate),
    ] {
        let cursor = c.frame(&mut doc, vec![Event::PointerMoved(at)]);
        assert_eq!(c.state.hot, Some(hot));
        // Turning has no system cursor: one is drawn instead.
        assert_eq!(cursor == CursorIcon::None, hot == Hot::Rotate, "{hot:?}");
    }
    // A little off the rotation handle's circle still grabs it.
    c.frame(&mut doc, vec![Event::PointerMoved(rot + vec2(10.0, 0.0))]);
    assert_eq!(c.state.hot, Some(Hot::Rotate));
    // Away from the handles: none; and while turning, it stays lit.
    c.frame(
        &mut doc,
        vec![Event::PointerMoved(c.screen([100.0, 100.0]))],
    );
    assert_eq!(c.state.hot, None);
    c.frame(&mut doc, vec![Event::PointerMoved(rot)]);
    c.frame(&mut doc, vec![Canvas::button(rot, true)]);
    let cursor = c.frame(&mut doc, vec![Event::PointerMoved(rot + vec2(40.0, 30.0))]);
    assert_eq!(c.state.hot, Some(Hot::Rotate));
    assert_eq!(cursor, CursorIcon::None);
    c.frame(
        &mut doc,
        vec![Canvas::button(rot + vec2(40.0, 30.0), false)],
    );
}

#[test]
fn a_guide_is_dragged_out_of_a_ruler_and_back_onto_it() {
    let mut doc = snap_doc();
    let mut c = Canvas::new(&mut doc);
    let (from, to) = (c.on_ruler(false, 30.0), c.screen([30.0, 40.0]));
    c.drag(&mut doc, from, to);
    assert_eq!(doc.guides.len(), 1);
    let g = doc.guides[0];
    assert!(!g.vertical && (g.at - 40.0).abs() < 0.2, "{g:?}");
    assert_eq!(g.at, (g.at * 10.0).round() / 10.0, "to a tenth of a mm");

    let from = c.screen([30.0, g.at]);
    let to = c.on_ruler(false, 30.0);
    c.drag(&mut doc, from, to);
    assert!(doc.guides.is_empty(), "dropped on the ruler: deleted");
}

#[test]
fn a_dragged_guide_snaps_to_layer_edges() {
    let mut doc = snap_doc();
    let mut c = Canvas::new(&mut doc);
    let (from, to) = (c.on_ruler(true, 160.0), c.screen([151.0, 160.0]));
    c.drag(&mut doc, from, to);
    assert_eq!(
        doc.guides,
        [Guide {
            vertical: true,
            at: 150.0
        }]
    );
}

#[test]
fn layers_snap_to_guides_unless_turned_off() {
    for snap in [true, false] {
        let mut doc = snap_doc();
        doc.guides.push(Guide {
            vertical: true,
            at: 160.0,
        });
        let mut c = Canvas::new(&mut doc);
        c.state.snap_to_guides = snap;
        let (from, to) = (c.screen([100.0, 100.0]), c.screen([108.5, 100.0]));
        c.drag(&mut doc, from, to);
        let x = doc.layer(1).unwrap().transform.center[0];
        let expected = if snap { 110.0 } else { 108.5 };
        assert!((x - expected).abs() < 0.05, "snap {snap}: {x}");
    }
}

#[test]
fn locked_guides_stay_put_and_a_double_click_types_a_position() {
    let mut doc = snap_doc();
    doc.guides.push(Guide {
        vertical: false,
        at: 160.0,
    });
    let mut c = Canvas::new(&mut doc);
    c.state.lock_guides = true;
    let (from, to) = (c.screen([20.0, 160.0]), c.screen([20.0, 180.0]));
    c.drag(&mut doc, from, to);
    assert_eq!(doc.guides[0].at, 160.0, "locked");
    c.double_click(&mut doc, c.screen([20.0, 160.0]));
    assert!(c.state.guide_edit.is_none(), "locked: no position to type");

    // That click was on empty canvas: it unselected the layer.
    c.selected = Some(1);
    c.state.lock_guides = false;
    c.double_click(&mut doc, c.screen([20.0, 160.0]));
    assert_eq!(
        c.state.guide_edit.map(|e| (e.index, e.at)),
        Some((0, 160.0))
    );
    assert_eq!(
        c.selected,
        Some(1),
        "clicks on a guide leave the selection alone"
    );
}

#[test]
fn guides_are_saved_only_when_there_are_some() {
    let mut doc = Document::default();
    assert!(!serde_json::to_string(&doc).unwrap().contains("guides"));
    doc.guides.push(Guide {
        vertical: true,
        at: 12.5,
    });
    let back: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(back.guides, doc.guides);
    assert!(clear_guides(&mut doc) && doc.guides.is_empty() && !clear_guides(&mut doc));
}

#[test]
fn ruler_marks_stay_apart_at_every_zoom() {
    for zoom in [MIN_ZOOM, 0.5, 1.0, 3.0, 10.0, MAX_ZOOM] {
        let (step, parts) = ruler_step(zoom);
        assert!(step * zoom >= RULER_LABEL_SPACING, "{zoom}");
        assert!(step * zoom < RULER_LABEL_SPACING * 2.5, "{zoom}");
        assert!(step / parts as f32 * zoom >= 4.0, "{zoom}");
    }
}

/// A template element with only the given shape centers.
fn anchors(points: &[P2]) -> Vec<PlacedElement> {
    vec![PlacedElement {
        kind: ElementKind::Holes,
        polylines: Vec::new(),
        centers: points.to_vec(),
    }]
}

#[test]
fn distorted_layer_snaps_by_the_middle_of_what_is_drawn() {
    let mut l = Layer::new(1, "l".into(), "x.png".into());
    l.size_px = [100, 50];
    l.transform.center = [50.0, 25.0];
    assert_eq!(snap_points(&l)[0], [50.0, 25.0], "undistorted: its center");
    l.transform.rotation_deg = 30.0;
    let m = snap_points(&l)[0];
    assert!(
        (m[0] - 50.0).abs() < 1e-3 && (m[1] - 25.0).abs() < 1e-3,
        "rotated too"
    );
    // The right corners pulled further right: the drawn shape spans x 0..140.
    l.transform.rotation_deg = 0.0;
    l.transform.distort = [[0.0, 0.0], [0.4, 0.0], [0.4, 0.0], [0.0, 0.0]];
    let m = snap_points(&l)[0];
    assert!(
        (m[0] - 70.0).abs() < 1e-3 && (m[1] - 25.0).abs() < 1e-3,
        "{m:?}"
    );
}

#[test]
fn editing_a_polyline_s_points_on_the_canvas() {
    use crate::document::shape::{self, ShapeKind};
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let mut doc = Document {
        layers: vec![Node::Layer(Box::new(shape::new_layer(
            1,
            ShapeKind::Polyline,
            [100.0, 100.0],
        )))],
        ..Default::default()
    };
    let elements = anchors(&[[70.5, 120.5]]);
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = Some(1);
    let mut time = 0.0;
    let mut frame = |doc: &mut Document, state: &mut CanvasState, events: Vec<Event>| {
        time += 0.02;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &elements, &mut selected, state);
            shape::sync(ui.ctx(), doc, !ui.input(|i| i.pointer.any_down()));
        });
        out.textures_delta.clear();
    };
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    frame(&mut doc, &mut state, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let screen = |p: P2| cam.to_screen(rect, p);
    let drag = |doc: &mut Document,
                state: &mut CanvasState,
                from: P2,
                to: P2,
                frame: &mut dyn FnMut(&mut Document, &mut CanvasState, Vec<Event>)| {
        let (from, to) = (screen(from), screen(to));
        frame(doc, state, vec![Event::PointerMoved(from)]);
        frame(doc, state, vec![button(from, true)]);
        for k in 1..=4 {
            frame(
                doc,
                state,
                vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))],
            );
        }
        frame(doc, state, vec![button(to, false)]);
        frame(doc, state, vec![]);
    };

    // A double click on the shape shows its points.
    let c = screen([100.0, 100.0]);
    frame(&mut doc, &mut state, vec![Event::PointerMoved(c)]);
    for pressed in [true, false, true, false] {
        frame(&mut doc, &mut state, vec![button(c, pressed)]);
    }
    assert_eq!(state.points.as_ref().map(|e| e.layer), Some(1));

    // The first point dragged near the anchor lands on it; the others stay.
    let before = shape::page_points(doc.layer(1).unwrap());
    drag(&mut doc, &mut state, before[0], [72.0, 119.0], &mut frame);
    let after = shape::page_points(doc.layer(1).unwrap());
    assert!(close(after[0], [70.5, 120.5]), "{:?}", after[0]);
    for k in 1..4 {
        assert!(
            close(after[k], before[k]),
            "point {k}: {:?} {:?}",
            after[k],
            before[k]
        );
    }
    assert_eq!(state.points.as_ref().unwrap().selected, Some(0));

    // A double click on the first segment adds a point there.
    let mid = [
        (after[0][0] + after[1][0]) / 2.0,
        (after[0][1] + after[1][1]) / 2.0,
    ];
    let m = screen(mid);
    // Long enough after the last clicks not to make a triple click with them.
    for _ in 0..20 {
        frame(&mut doc, &mut state, vec![Event::PointerMoved(m)]);
    }
    for pressed in [true, false, true, false] {
        frame(&mut doc, &mut state, vec![button(m, pressed)]);
    }
    frame(&mut doc, &mut state, vec![]);
    let now = shape::page_points(doc.layer(1).unwrap());
    assert_eq!(now.len(), 5);
    assert!(close(now[1], mid), "{:?} {mid:?}", now[1]);
    assert!(close(now[0], after[0]));
    assert_eq!(state.points.as_ref().unwrap().selected, Some(1));

    // Moving points again and again never adds any.
    for round in 0..3 {
        for i in [0usize, 2, 3, 4, 1] {
            let p = shape::page_points(doc.layer(1).unwrap())[i];
            let to = [p[0] + 3.0 + round as f32, p[1] - 2.0];
            drag(&mut doc, &mut state, p, to, &mut frame);
            let pts = shape::page_points(doc.layer(1).unwrap());
            assert_eq!(pts.len(), 5, "round {round}, point {i}");
            assert!(
                close(pts[i], to),
                "round {round}, point {i}: {:?} {to:?}",
                pts[i]
            );
        }
    }

    // Real mice: a move and the press in one frame, a click first, a pause.
    for i in [0usize, 2, 3] {
        let p = shape::page_points(doc.layer(1).unwrap())[i];
        let sp = screen(p);
        frame(
            &mut doc,
            &mut state,
            vec![Event::PointerMoved(sp), button(sp, true)],
        );
        frame(&mut doc, &mut state, vec![button(sp, false)]);
        frame(&mut doc, &mut state, vec![]);
        let to = screen([p[0] + 4.0, p[1] + 4.0]);
        frame(
            &mut doc,
            &mut state,
            vec![
                Event::PointerMoved(sp + vec2(1.0, 1.0)),
                button(sp + vec2(1.0, 1.0), true),
            ],
        );
        frame(&mut doc, &mut state, vec![Event::PointerMoved(to)]);
        frame(
            &mut doc,
            &mut state,
            vec![Event::PointerMoved(to), button(to, false)],
        );
        frame(&mut doc, &mut state, vec![]);
        assert_eq!(
            shape::page_points(doc.layer(1).unwrap()).len(),
            5,
            "point {i}"
        );
    }

    // Escape stops editing.
    frame(
        &mut doc,
        &mut state,
        vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(state.points.is_none());
}

#[test]
fn drawing_shapes_point_by_point() {
    use crate::document::shape::{self, ShapeKind};
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let elements = anchors(&[[150.5, 60.5]]);
    let ctx = egui::Context::default();
    let mut doc = Document::default();
    let mut state = CanvasState::default();
    let mut selected = None;
    let mut time = 0.0;
    let mut frame = |doc: &mut Document,
                     state: &mut CanvasState,
                     selected: &mut Option<u64>,
                     events: Vec<Event>| {
        // Apart enough that two clicks are not a double click.
        time += 0.5;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &elements, selected, state);
            shape::sync(ui.ctx(), doc, !ui.input(|i| i.pointer.any_down()));
        });
        out.textures_delta.clear();
    };
    frame(&mut doc, &mut state, &mut selected, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let click = |p: P2| {
        let s = cam.to_screen(rect, p);
        let b = |pressed| Event::PointerButton {
            pos: s,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![Event::PointerMoved(s), b(true), b(false)]
    };

    // A polyline closed by a click on its first point becomes a polygon.
    state.drawing = Some(Drawing::new(ShapeKind::Polyline));
    for p in [
        [100.0, 100.0],
        [150.0, 61.0],
        [180.0, 120.0],
        [100.2, 100.2],
    ] {
        frame(&mut doc, &mut state, &mut selected, click(p));
    }
    assert!(state.drawing.is_none(), "done");
    let id = selected.expect("the new shape is selected");
    let l = doc.layer(id).unwrap();
    let s = l.shape.as_ref().unwrap();
    assert_eq!(s.kind, ShapeKind::Polygon);
    // Named with two random words, kept when the polyline was closed.
    assert_eq!(l.name.split(' ').count(), 2, "{}", l.name);
    assert!(s.fill, "closed: filled");
    let pts = shape::page_points(l);
    assert_eq!(pts.len(), 3);
    assert!(close(pts[0], [100.0, 100.0]));
    assert!(close(pts[1], [150.5, 60.5]), "snapped: {:?}", pts[1]);

    // A polygon canvas from the Canvas menu: drawn the same, to paint in.
    let mut d = Drawing::new(ShapeKind::Polygon);
    d.canvas = true;
    state.drawing = Some(d);
    for p in [[20.0, 20.0], [60.0, 20.0], [40.0, 50.0], [20.1, 20.1]] {
        frame(&mut doc, &mut state, &mut selected, click(p));
    }
    let l = doc.layer(selected.unwrap()).unwrap();
    let s = l.shape.as_ref().unwrap();
    assert_eq!(s.kind, ShapeKind::Polygon);
    assert_eq!(l.name.split(' ').count(), 2, "{}", l.name);
    assert!(!s.fill && !s.stroke, "neither background nor border yet");
    assert!(can_paint(&doc, selected));

    // Enter finishes an open polyline.
    state.drawing = Some(Drawing::new(ShapeKind::Polyline));
    for p in [[50.0, 50.0], [60.0, 70.0], [70.0, 50.0]] {
        frame(&mut doc, &mut state, &mut selected, click(p));
    }
    frame(
        &mut doc,
        &mut state,
        &mut selected,
        vec![Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(state.drawing.is_none());
    let l = doc.layer(selected.unwrap()).unwrap();
    let s = l.shape.as_ref().unwrap();
    assert_eq!(
        (s.kind, s.points.len(), s.fill),
        (ShapeKind::Polyline, 3, false)
    );

    // A line is done at its second click.
    state.drawing = Some(Drawing::new(ShapeKind::Line));
    for p in [[20.0, 150.0], [80.0, 150.0]] {
        frame(&mut doc, &mut state, &mut selected, click(p));
    }
    assert!(state.drawing.is_none());
    let l = doc.layer(selected.unwrap()).unwrap();
    assert_eq!(shape::page_points(l).len(), 2);
    assert_eq!(doc.leaves().len(), 4);
}

/// Runs one canvas frame with these events, in the tests that need helpers.
type Frame<'a> = dyn FnMut(&mut Document, &mut CanvasState, &mut Option<u64>, Vec<Event>) + 'a;

#[test]
fn bezier_curves_on_the_canvas() {
    use crate::document::shape::{self, ShapeKind};
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.1 && (a[1] - b[1]).abs() < 0.1;
    let ctx = egui::Context::default();
    let mut doc = Document::default();
    let mut state = CanvasState::default();
    let mut selected = None;
    let mut time = 0.0;
    let mut frame = |doc: &mut Document,
                     state: &mut CanvasState,
                     selected: &mut Option<u64>,
                     events: Vec<Event>| {
        time += 0.05;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, doc, &[], selected, state);
            shape::sync(ui.ctx(), doc, !ui.input(|i| i.pointer.any_down()));
        });
        out.textures_delta.clear();
    };
    frame(&mut doc, &mut state, &mut selected, vec![]);
    let (cam, rect) = (state.camera, state.rect);
    let s = |p: P2| cam.to_screen(rect, p);
    let b = |p: P2, pressed, modifiers| Event::PointerButton {
        pos: s(p),
        button: PointerButton::Primary,
        pressed,
        modifiers,
    };
    let none = egui::Modifiers::NONE;
    let idle = |frame: &mut Frame,
                doc: &mut Document,
                state: &mut CanvasState,
                selected: &mut Option<u64>| {
        for _ in 0..10 {
            frame(doc, state, selected, vec![]);
        }
    };

    // Drawing: a click, a press dragged (a smooth point), a click, then Enter.
    state.drawing = Some(Drawing::new(ShapeKind::Polyline));
    frame(
        &mut doc,
        &mut state,
        &mut selected,
        vec![
            Event::PointerMoved(s([100.0, 100.0])),
            b([100.0, 100.0], true, none),
            b([100.0, 100.0], false, none),
        ],
    );
    idle(&mut frame, &mut doc, &mut state, &mut selected);
    frame(
        &mut doc,
        &mut state,
        &mut selected,
        vec![
            Event::PointerMoved(s([140.0, 100.0])),
            b([140.0, 100.0], true, none),
        ],
    );
    for x in [142.0, 146.0, 150.0] {
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![Event::PointerMoved(s([x, 90.0]))],
        );
    }
    frame(
        &mut doc,
        &mut state,
        &mut selected,
        vec![b([150.0, 90.0], false, none)],
    );
    idle(&mut frame, &mut doc, &mut state, &mut selected);
    frame(
        &mut doc,
        &mut state,
        &mut selected,
        vec![
            Event::PointerMoved(s([180.0, 100.0])),
            b([180.0, 100.0], true, none),
            b([180.0, 100.0], false, none),
        ],
    );
    let enter = Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: none,
    };
    frame(&mut doc, &mut state, &mut selected, vec![enter]);
    assert!(state.drawing.is_none());
    let id = selected.unwrap();
    let l = doc.layer(id).unwrap();
    let sh = l.shape.as_ref().unwrap();
    assert_eq!(sh.points.len(), 3);
    assert!(sh.is_smooth(1), "{:?}", sh.handles);
    let [hin, hout] = shape::page_handles(l, 1);
    assert!(close(hout.unwrap(), [150.0, 90.0]), "{hout:?}");
    assert!(close(hin.unwrap(), [130.0, 110.0]), "mirrored: {hin:?}");
    assert!(
        sh.handle(0) == [[0.0; 2]; 2] && sh.handle(2) == [[0.0; 2]; 2],
        "clicks: sharp"
    );

    // Editing: dragging the out handle turns the in handle with it...
    state.points = Some(PointEdit::new(id));
    let p1 = shape::page_points(l)[1];
    let click = |p: P2| {
        vec![
            Event::PointerMoved(s(p)),
            b(p, true, none),
            b(p, false, none),
        ]
    };
    frame(&mut doc, &mut state, &mut selected, click(p1));
    idle(&mut frame, &mut doc, &mut state, &mut selected);
    let drag = |frame: &mut Frame,
                doc: &mut Document,
                state: &mut CanvasState,
                selected: &mut Option<u64>,
                from: P2,
                to: P2,
                m: egui::Modifiers| {
        frame(
            doc,
            state,
            selected,
            vec![
                Event::ModifiersChanged(m),
                Event::PointerMoved(s(from)),
                b(from, true, m),
            ],
        );
        for k in 1..=4 {
            let t = k as f32 / 4.0;
            let p = [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ];
            frame(doc, state, selected, vec![Event::PointerMoved(s(p))]);
        }
        frame(
            doc,
            state,
            selected,
            vec![
                b(to, false, m),
                Event::ModifiersChanged(egui::Modifiers::NONE),
            ],
        );
        for _ in 0..10 {
            frame(doc, state, selected, vec![]);
        }
    };
    drag(
        &mut frame,
        &mut doc,
        &mut state,
        &mut selected,
        [150.0, 90.0],
        [140.0, 85.0],
        none,
    );
    let l = doc.layer(id).unwrap();
    let [hin, hout] = shape::page_handles(l, 1);
    assert!(close(hout.unwrap(), [140.0, 85.0]), "{hout:?}");
    assert!(l.shape.as_ref().unwrap().is_smooth(1));
    assert_eq!(shape::page_points(l).len(), 3, "no point added");
    // ...unless Alt is held.
    drag(
        &mut frame,
        &mut doc,
        &mut state,
        &mut selected,
        [140.0, 85.0],
        [150.0, 100.0],
        egui::Modifiers::ALT,
    );
    let l = doc.layer(id).unwrap();
    assert!(!l.shape.as_ref().unwrap().is_smooth(1));
    let still = shape::page_handles(l, 1)[0].unwrap();
    assert!(close(still, hin.unwrap()), "the in handle stayed");

    // A double click on the point makes it sharp: no handles left at all.
    let p1 = shape::page_points(l)[1];
    for pressed in [true, false, true, false] {
        frame(
            &mut doc,
            &mut state,
            &mut selected,
            vec![Event::PointerMoved(s(p1)), b(p1, pressed, none)],
        );
    }
    let sh = doc.layer(id).unwrap().shape.as_ref().unwrap();
    assert!(sh.handles.is_empty(), "{:?}", sh.handles);
    assert!(state.points.is_some(), "still editing");
}

/// Template elements made of a rectangular outline from `lo` to `hi`.
fn outline(lo: P2, hi: P2) -> Vec<PlacedElement> {
    vec![PlacedElement {
        kind: ElementKind::Outline,
        polylines: vec![crate::template::Polyline {
            points: vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
            closed: true,
        }],
        centers: Vec::new(),
    }]
}

#[test]
fn guides_are_the_page_and_the_outline_s_bounds_middle_straight_sides_and_bleed_line() {
    let mut els = outline([20.0, 20.0], [250.0, 180.0]);
    // A notch with a short vertical side, too short to count, and a long one.
    els[0].polylines.push(crate::template::Polyline {
        points: vec![[60.0, 40.0], [60.0, 41.0], [90.0, 41.0], [90.0, 70.0]],
        closed: false,
    });
    let g = guides(&els, 3.0, [297.0, 210.0]);
    assert_eq!(
        g.x,
        [0.0, 17.0, 20.0, 90.0, 135.0, 148.5, 250.0, 253.0, 297.0]
    );
    assert_eq!(
        g.y,
        [0.0, 17.0, 20.0, 41.0, 100.0, 105.0, 180.0, 183.0, 210.0]
    );
    let page_only = guides(&anchors(&[[1.0, 1.0]]), 3.0, [297.0, 210.0]);
    assert_eq!(page_only.x, [0.0, 148.5, 297.0]);
    assert_eq!(page_only.y, [0.0, 105.0, 210.0]);
}

#[test]
fn moving_and_resizing_snap_edges_to_the_outline() {
    let close = |a: f32, b: f32| (a - b).abs() < 0.05;
    let none = egui::Modifiers::NONE;
    // Guides x 40, 110, 180 and y 20, 100, 180. The image: (50, 75) to (150, 125).
    let els = outline([40.0, 20.0], [180.0, 180.0]);
    let drag = |doc: &mut Document, from: P2, to: P2, m| canvas_drag_in(doc, from, to, m, &els);

    // The left edge comes within reach of the outline's and lands on it.
    let mut doc = snap_doc();
    drag(&mut doc, [100.0, 100.0], [91.5, 100.0], none);
    let c = doc.layer(1).unwrap().transform.center;
    assert!(close(c[0], 90.0) && close(c[1], 100.0), "{c:?}");
    // Alt drags freely.
    let mut doc = snap_doc();
    drag(
        &mut doc,
        [100.0, 100.0],
        [91.5, 100.0],
        egui::Modifiers::ALT,
    );
    assert!(close(doc.layer(1).unwrap().transform.center[0], 91.5));

    // The bottom edge stretched near the outline's bottom ends on it, the top staying.
    let mut doc = snap_doc();
    drag(&mut doc, [100.0, 125.0], [100.0, 177.5], none);
    let l = doc.layer(1).unwrap();
    let (lo, hi) = layers::layers_bounds(&doc, &[1]).unwrap();
    assert!(close(lo[1], 75.0) && close(hi[1], 180.0), "{lo:?} {hi:?}");
    assert!(close(l.transform.scale[0], 1.0));

    // Without a template, the page's edges: the right one, 297.
    let mut doc = snap_doc();
    canvas_drag_in(&mut doc, [100.0, 100.0], [245.5, 100.0], none, &[]);
    assert!(close(doc.layer(1).unwrap().transform.center[0], 247.0));

    // A corner scaled near the outline's right side ends on it.
    let mut doc = snap_doc();
    drag(&mut doc, [150.0, 125.0], [178.0, 139.0], none);
    let (_, hi) = layers::layers_bounds(&doc, &[1]).unwrap();
    assert!(close(hi[0], 180.0), "{hi:?}");
}

#[test]
fn moving_snaps_center_or_corner_to_anchors() {
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let (none, alt) = (egui::Modifiers::NONE, egui::Modifiers::ALT);
    // The center comes within reach of an anchor and lands on it.
    let mut doc = snap_doc();
    let near_center = anchors(&[[104.0, 102.0]]);
    canvas_drag_in(&mut doc, [100.0, 100.0], [103.0, 101.0], none, &near_center);
    let c = doc.layer(1).unwrap().transform.center;
    assert!(close(c, [104.0, 102.0]), "{c:?}");
    // Alt drags freely.
    let mut doc = snap_doc();
    canvas_drag_in(&mut doc, [100.0, 100.0], [103.0, 101.0], alt, &near_center);
    let c = doc.layer(1).unwrap().transform.center;
    assert!(close(c, [103.0, 101.0]), "{c:?}");
    // A corner snaps too, moving the whole image with it.
    let mut doc = snap_doc();
    let near_corner = anchors(&[[155.0, 76.0]]);
    canvas_drag_in(&mut doc, [100.0, 100.0], [104.0, 100.0], none, &near_corner);
    let l = doc.layer(1).unwrap();
    assert!(close(l.corners()[1], [155.0, 76.0]), "{:?}", l.corners());
    assert!(close(l.transform.center, [105.0, 101.0]));
    // So does the middle of an edge: the right one, at (150, 100).
    let mut doc = snap_doc();
    let near_edge = anchors(&[[156.0, 99.0]]);
    canvas_drag_in(&mut doc, [100.0, 100.0], [104.0, 100.0], none, &near_edge);
    let c = doc.layer(1).unwrap().transform.center;
    assert!(close(c, [106.0, 99.0]), "{c:?}");
    // Out of reach, nothing snaps.
    let mut doc = snap_doc();
    let far = anchors(&[[130.0, 100.0]]);
    canvas_drag_in(&mut doc, [100.0, 100.0], [103.0, 101.0], none, &far);
    let c = doc.layer(1).unwrap().transform.center;
    assert!(close(c, [103.0, 101.0]), "{c:?}");
}

#[test]
fn resizing_and_distorting_snap_the_dragged_corner() {
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let none = egui::Modifiers::NONE;
    // Scaling around the center: the corner lands on an anchor on its diagonal.
    let mut doc = snap_doc();
    let on_diagonal = anchors(&[[160.0, 130.0]]);
    canvas_drag_in(&mut doc, [150.0, 125.0], [158.0, 129.0], none, &on_diagonal);
    let l = doc.layer(1).unwrap();
    assert!(close(l.corners()[2], [160.0, 130.0]), "{:?}", l.corners());
    assert!(
        close(l.transform.scale, [1.2, 1.2]),
        "{:?}",
        l.transform.scale
    );
    // Distorting: the corner lands exactly on the anchor, the others stay.
    let mut doc = snap_doc();
    let anchor = anchors(&[[160.0, 81.0]]);
    let ctrl = egui::Modifiers::COMMAND;
    canvas_drag_in(&mut doc, [150.0, 75.0], [158.0, 80.0], ctrl, &anchor);
    let c = doc.layer(1).unwrap().corners();
    assert!(close(c[1], [160.0, 81.0]), "{c:?}");
    assert!(
        close(c[0], [50.0, 75.0]) && close(c[2], [150.0, 125.0]),
        "{c:?}"
    );
}

#[test]
fn edge_handles_stretch_one_axis() {
    let layer = |rotation: f32| {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [100.0, 100.0];
        l.transform.rotation_deg = rotation;
        Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        }
    };
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let none = egui::Modifiers::NONE;

    // Bottom edge up by 20 mm: height 50 -> 30, top edge stays.
    let mut doc = layer(0.0);
    canvas_drag(&mut doc, [100.0, 125.0], [103.0, 105.0], none);
    let t = doc.layer(1).unwrap().transform;
    assert!(close(t.scale, [1.0, 0.6]), "{:?}", t.scale);
    assert!(close(t.center, [100.0, 90.0]), "{:?}", t.center);

    // Top edge: its handle sits under the rotation handle's stem, and still stretches.
    let mut doc = layer(0.0);
    canvas_drag(&mut doc, [100.0, 75.0], [100.0, 85.0], none);
    let t = doc.layer(1).unwrap().transform;
    assert!(close(t.scale, [1.0, 0.8]) && t.rotation_deg == 0.0, "{t:?}");

    // Rotated a quarter turn, the image's right edge points down the page.
    let mut doc = layer(90.0);
    canvas_drag(&mut doc, [100.0, 150.0], [100.0, 170.0], none);
    let t = doc.layer(1).unwrap().transform;
    assert!(close(t.scale, [1.2, 1.0]), "{:?}", t.scale);
    assert!(close(t.center, [100.0, 110.0]), "{:?}", t.center);

    // Alt stretches both sides around the center.
    let mut doc = layer(0.0);
    canvas_drag(
        &mut doc,
        [150.0, 100.0],
        [160.0, 100.0],
        egui::Modifiers::ALT,
    );
    let t = doc.layer(1).unwrap().transform;
    assert!(
        close(t.scale, [1.2, 1.0]) && close(t.center, [100.0, 100.0]),
        "{t:?}"
    );

    // Squashing stops at a minimum instead of flipping the image.
    let mut doc = layer(0.0);
    canvas_drag(&mut doc, [100.0, 125.0], [100.0, 40.0], none);
    let t = doc.layer(1).unwrap().transform;
    assert!((t.scale[1] * 50.0 - MIN_STRETCH_MM).abs() < 1e-4, "{t:?}");
}

#[test]
fn ctrl_drag_on_the_layer_tilts_it_in_3d() {
    let doc = |rotation: f32| {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [100.0, 100.0];
        l.transform.rotation_deg = rotation;
        Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        }
    };
    let ctrl = egui::Modifiers::COMMAND;
    let tilt = |d: &Document| {
        let t = d.layer(1).unwrap().transform;
        (t.tilt_x_deg, t.tilt_y_deg, t.center)
    };
    // Up: the top goes away; it doesn't move.
    let mut d = doc(0.0);
    canvas_drag(&mut d, [100.0, 100.0], [100.0, 90.0], ctrl);
    let (x, y, c) = tilt(&d);
    assert!(x > 1.0 && y == 0.0 && c == [100.0, 100.0], "{x} {y} {c:?}");
    // Right: the right side goes away.
    let mut d = doc(0.0);
    canvas_drag(&mut d, [100.0, 100.0], [110.0, 100.0], ctrl);
    let (x, y, _) = tilt(&d);
    assert!(x == 0.0 && y > 1.0, "{x} {y}");
    // Turned a quarter, dragging down the page is toward its right side.
    let mut d = doc(90.0);
    canvas_drag(&mut d, [100.0, 100.0], [100.0, 110.0], ctrl);
    let (x, y, _) = tilt(&d);
    assert!(x.abs() < 1e-3 && y > 1.0, "{x} {y}");
    // Without Ctrl it moves, untilted.
    let mut d = doc(0.0);
    canvas_drag(
        &mut d,
        [100.0, 100.0],
        [110.0, 100.0],
        egui::Modifiers::NONE,
    );
    let (x, y, c) = tilt(&d);
    assert!(x == 0.0 && y == 0.0 && c != [100.0, 100.0]);
}

#[test]
fn ctrl_swaps_resize_and_distort_on_corners() {
    let doc = || {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [100.0, 100.0];
        Document {
            layers: vec![Node::Layer(Box::new(l))],
            ..Default::default()
        }
    };
    let ctrl = egui::Modifiers::COMMAND;
    let none = egui::Modifiers::NONE;
    // Drag the bottom-right corner (150, 125) outwards.
    let (from, to) = ([150.0, 125.0], [160.0, 135.0]);
    let distorted = |d: &Document| d.layer(1).unwrap().transform.is_distorted();
    let scaled = |d: &Document| d.layer(1).unwrap().transform.scale != [1.0, 1.0];

    for (mods, want_distort) in [
        (none, false), // plain drag resizes
        (ctrl, true),  // Ctrl distorts
    ] {
        let mut d = doc();
        canvas_drag(&mut d, from, to, mods);
        assert_eq!(distorted(&d), want_distort, "ctrl {}", mods.command);
        assert_eq!(scaled(&d), !want_distort, "ctrl {}", mods.command);
    }
}

#[test]
fn ctrl_alt_on_a_handle_resizes_a_shape_without_stretching_its_paint() {
    use crate::document::shape::ShapeKind;
    let close = |a: P2, b: P2| (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05;
    let ctx = egui::Context::default();
    let mut doc = Document::default();
    let mut l = crate::document::shape::new_canvas_layer(
        1,
        ShapeKind::Rectangle,
        [50.0, 75.0],
        [150.0, 125.0],
    );
    let line = [[60.0, 100.0, 1.0], [90.0, 100.0, 1.0]];
    let pencil = crate::document::paint::Tool::Pencil;
    assert!(crate::document::shape::add_stroke(
        &mut l,
        &line,
        &crate::document::paint::Pen::new(pencil, 2.0, [0; 4])
    ));
    doc.layers.push(Node::Layer(Box::new(l)));
    crate::document::shape::sync(&ctx, &mut doc, true);
    let ends = |doc: &Document| -> Vec<P2> {
        let l = doc.layer(1).unwrap();
        l.shape.as_ref().unwrap().strokes[0]
            .points
            .iter()
            .map(|p| crate::document::shape::to_page(l, [p[0], p[1]]).unwrap())
            .collect()
    };
    let before = ends(&doc);
    let ctrl_alt = egui::Modifiers::COMMAND | egui::Modifiers::ALT;

    // The right edge's handle dragged out with Ctrl+Alt: wider, the paint in place.
    canvas_drag_in(&mut doc, [150.0, 100.0], [200.0, 100.0], ctrl_alt, &[]);
    crate::document::shape::sync(&ctx, &mut doc, true);
    let size = |doc: &Document| doc.layer(1).unwrap().shape.as_ref().unwrap().size;
    assert!(close(size(&doc), [150.0, 50.0]), "{:?}", size(&doc));
    for (a, b) in before.iter().zip(&ends(&doc)) {
        assert!(close(*a, *b), "{before:?} {:?}", ends(&doc));
    }
    // Only for that drag: the next one stretches it, the shape's setting.
    let shape = doc.layer(1).unwrap().shape.as_ref().unwrap();
    assert!(shape.stretch_paint);
    canvas_drag_in(
        &mut doc,
        [200.0, 100.0],
        [350.0, 100.0],
        egui::Modifiers::NONE,
        &[],
    );
    crate::document::shape::sync(&ctx, &mut doc, true);
    let after = ends(&doc);
    assert!(close(size(&doc), [300.0, 50.0]), "{:?}", size(&doc));
    let len = |e: &[P2]| e[e.len() - 1][0] - e[0][0];
    assert!((len(&after) - 2.0 * len(&before)).abs() < 0.1, "{after:?}");
}

#[test]
fn the_paint_cursor_leaves_the_tool_bar_alone() {
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    state.brushes.tool = Some(crate::document::paint::Tool::Brush);
    let mut doc = Document {
        layers: vec![Node::Layer(Box::new(crate::document::shape::new_layer(
            1,
            crate::document::shape::ShapeKind::Rectangle,
            [100.0, 100.0],
        )))],
        ..Default::default()
    };
    let mut selected = Some(1);
    let mut frame = |state: &mut CanvasState, time: f64, at: Pos2| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events: vec![Event::PointerMoved(at)],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            crate::document::shape::sync(ui.ctx(), &mut doc, true);
            show(ui, &mut doc, &[], &mut selected, state);
        });
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    };
    // The tool bar needs a few frames to size and show itself.
    for k in 0..4 {
        frame(&mut state, k as f64 * 0.02, pos2(600.0, 500.0));
    }
    assert_eq!(
        frame(&mut state, 0.1, pos2(600.0, 500.0)),
        CursorIcon::Crosshair
    );
    // Over its pencil, at the canvas' top left, below and right of the rulers.
    assert_ne!(
        frame(&mut state, 0.2, pos2(48.0 + RULER, 20.0 + RULER)),
        CursorIcon::Crosshair
    );
}

#[test]
fn canvas_cursors_stay_on_the_canvas() {
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut doc = Document::default();
    let mut selected = None;
    let mut text = String::from("text");
    let mut frame = |state: &mut CanvasState, time: f64, events: Vec<Event>| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            // A panel with a text field at the left, drawn first as in the app.
            egui::Panel::left("side").exact_size(200.0).show(ui, |ui| {
                ui.add_sized(vec2(180.0, 40.0), egui::TextEdit::singleline(&mut text))
            });
            egui::CentralPanel::default().show(ui, |ui| {
                show(ui, &mut doc, &[], &mut selected, state);
            });
        });
        out.textures_delta.clear();
        out.platform_output.cursor_icon
    };
    let moved = |p| vec![Event::PointerMoved(p)];
    let button = |p, pressed| Event::PointerButton {
        pos: p,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let canvas = pos2(600.0, 400.0);
    let field = pos2(60.0, 25.0);
    for k in 0..3 {
        frame(&mut state, k as f64 * 0.02, moved(canvas));
    }
    // A drag on the empty canvas pans: a closed hand, while on the canvas.
    frame(&mut state, 0.1, vec![button(canvas, true)]);
    frame(&mut state, 0.15, moved(canvas + vec2(15.0, 0.0)));
    let pan = frame(&mut state, 0.2, moved(canvas + vec2(30.0, 0.0)));
    assert_eq!(pan, CursorIcon::Grabbing);
    // Carried over the text field: not the canvas' hand any more.
    assert_eq!(frame(&mut state, 0.3, moved(field)), CursorIcon::Default);
    // Once let go: the text field's own.
    frame(&mut state, 0.4, vec![button(field, false)]);
    assert_eq!(frame(&mut state, 0.5, moved(field)), CursorIcon::Text);
}

#[test]
fn wheel_over_a_window_does_not_zoom_the_canvas() {
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut doc = Document::default();
    let mut selected = None;
    let mut frame = |state: &mut CanvasState, time: f64, events: Vec<Event>, window: bool| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, &mut doc, &[], &mut selected, state);
            if window {
                egui::Window::new("over")
                    .fixed_pos(pos2(100.0, 100.0))
                    .fixed_size(vec2(300.0, 300.0))
                    .show(ui.ctx(), |ui| ui.allocate_space(vec2(300.0, 300.0)));
            }
        });
        out.textures_delta.clear();
    };
    let wheel = |p| {
        vec![
            Event::PointerMoved(p),
            Event::MouseWheel {
                unit: MouseWheelUnit::Line,
                delta: vec2(0.0, 1.0),
                modifiers: Default::default(),
                phase: egui::TouchPhase::Move,
                source: MouseWheelSource::Wheel,
            },
        ]
    };
    // Windows need a few frames to size and show themselves.
    for k in 0..4 {
        let hover = vec![Event::PointerMoved(pos2(200.0, 200.0))];
        frame(&mut state, k as f64 * 0.02, hover, true);
    }
    let zoom = state.camera.zoom;
    frame(&mut state, 0.2, wheel(pos2(200.0, 200.0)), true);
    assert_eq!(state.camera.zoom, zoom, "window covers the canvas");
    frame(&mut state, 0.3, wheel(pos2(800.0, 600.0)), true);
    assert!(state.camera.zoom > zoom, "bare canvas still zooms");
}

#[test]
fn new_things_go_in_view() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
    let mut state = CanvasState {
        rect,
        ..Default::default()
    };
    state.camera.fit(rect, [297.0, 210.0]);
    let center = [148.5, 105.0];
    assert_eq!(state.in_view(center), center, "the whole page shows");
    // Zoomed in on the top left corner: the page's center is out of sight.
    state
        .camera
        .zoom_at(rect, state.camera.to_screen(rect, [20.0, 20.0]), 8.0);
    let p = state.in_view(center);
    let middle = state.camera.to_mm(rect, rect.center());
    assert!((p[0] - middle[0]).abs() < 1e-3 && (p[1] - middle[1]).abs() < 1e-3);
    assert!(p[0] < 60.0 && p[1] < 60.0, "{p:?}");
    // Before the canvas was ever shown, nothing changes.
    assert_eq!(CanvasState::default().in_view(center), center);
}

#[test]
fn zoom_keeps_anchor_fixed() {
    let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
    let mut cam = Camera {
        zoom: 2.0,
        pan: vec2(30.0, 40.0),
    };
    let anchor = pos2(400.0, 300.0);
    let before = cam.to_mm(rect, anchor);
    cam.zoom_at(rect, anchor, 1.7);
    let after = cam.to_mm(rect, anchor);
    assert!((before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3);
}

#[test]
fn fit_centers_page() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
    let mut cam = Camera {
        zoom: 1.0,
        pan: Vec2::ZERO,
    };
    cam.fit(rect, [297.0, 210.0]);
    let c = cam.to_screen(rect, [148.5, 105.0]);
    assert!((c.x - 500.0).abs() < 1e-2 && (c.y - 250.0).abs() < 1e-2);
}

// ---- Characterization trace of `show` (temporary, for the show() refactor) ----

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0
    }
}

fn trace_doc() -> Document {
    use crate::document::Group;
    use crate::document::shape::{self, ShapeKind};
    let img = |id: u64, c: P2, w: u32, h: u32| {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.size_px = [w, h];
        l.transform.center = c;
        Node::Layer(Box::new(l))
    };
    let mut locked = Layer::new(5, "locked".into(), "x.png".into());
    locked.size_px = [40, 40];
    locked.transform.center = [220.0, 60.0];
    locked.locked = true;
    let mut linked = Layer::new(6, "linked".into(), "x.png".into());
    linked.size_px = [30, 30];
    linked.transform.center = [60.0, 160.0];
    linked
        .bindings
        .insert(crate::document::variables::Field::X, "60".into());
    let text = crate::document::text::new_layer(7, "Hi", "Sans".into(), [150.0, 40.0]);
    let poly = shape::new_layer(8, ShapeKind::Polyline, [200.0, 150.0]);
    let canvas = shape::new_canvas_layer(9, ShapeKind::Rectangle, [20.0, 20.0], [80.0, 60.0]);
    Document {
        layers: vec![
            img(1, [100.0, 100.0], 100, 50),
            Node::Group(Group {
                id: 2,
                name: "g".into(),
                visible: true,
                locked: false,
                opacity: 1.0,
                clip: false,
                blend: crate::document::BlendMode::Normal,
                expanded: true,
                children: vec![
                    img(3, [180.0, 110.0], 20, 20),
                    img(4, [200.0, 120.0], 20, 30),
                ],
                element: None,
            }),
            Node::Layer(Box::new(locked)),
            Node::Layer(Box::new(linked)),
            Node::Layer(Box::new(text)),
            Node::Layer(Box::new(poly)),
            Node::Layer(Box::new(canvas)),
        ],
        guides: vec![
            Guide {
                vertical: true,
                at: 120.0,
            },
            Guide {
                vertical: false,
                at: 70.0,
            },
        ],
        template_points: vec![("h1".into(), [70.5, 120.5])],
        ..Default::default()
    }
}

fn drag_kind_name(k: DragKind) -> String {
    match k {
        DragKind::Move => "move".into(),
        DragKind::Scale => "scale".into(),
        DragKind::Rotate => "rotate".into(),
        DragKind::Distort(i) => format!("distort{i}"),
        DragKind::Stretch(i) => format!("stretch{i}"),
        DragKind::Tilt => "tilt".into(),
        DragKind::RotateGroup(p) => format!("rotgroup{p:?}"),
    }
}

/// Points worth pointing at this frame: the selected layer's handles, layer centers,
/// guides, the rulers, the divider, or anywhere.
fn interesting(rng: &mut Rng, doc: &Document, sel: Option<u64>, st: &CanvasState) -> Pos2 {
    use super::handles::{edge_handles, handles};
    let (cam, rect, full) = (st.camera, st.rect, st.full_rect);
    let jitter = |rng: &mut Rng, p: Pos2| p + vec2(rng.unit() * 6.0 - 3.0, rng.unit() * 6.0 - 3.0);
    match rng.below(9) {
        0 | 1 => {
            if let Some(id) = sel.filter(|&id| matches!(doc.node(id), Some(Node::Group(_))))
                && let Some(b) = super::handles::group_box(doc, &cam, rect, id)
            {
                return jitter(rng, super::handles::group_rotate_handle(b).1);
            }
            if let Some(l) = sel.and_then(|id| doc.layer(id)) {
                let (c, _, rot) = handles(&cam, rect, l);
                let e = edge_handles(&cam, rect, l);
                let all: Vec<Pos2> = c.into_iter().chain(e).chain([rot]).collect();
                let i = rng.below(all.len());
                return jitter(rng, all[i]);
            }
            pos2(
                full.left() + rng.unit() * full.width(),
                full.top() + rng.unit() * full.height(),
            )
        }
        2 | 3 => {
            let ids = [1, 3, 4, 5, 6, 7, 8, 9];
            let id = ids[rng.below(ids.len())];
            match doc.layer(id) {
                Some(l) => jitter(rng, cam.to_screen(rect, l.transform.center)),
                None => rect.center(),
            }
        }
        4 => match doc.guides.get(rng.below(doc.guides.len().max(1))) {
            Some(g) if g.vertical => pos2(
                cam.to_screen(rect, [g.at, 0.0]).x + rng.unit() * 4.0 - 2.0,
                rect.top() + rng.unit() * rect.height(),
            ),
            Some(g) => pos2(
                rect.left() + rng.unit() * rect.width(),
                cam.to_screen(rect, [0.0, g.at]).y + rng.unit() * 4.0 - 2.0,
            ),
            None => rect.center(),
        },
        5 => {
            if rng.below(2) == 0 {
                pos2(
                    full.left() + rng.unit() * full.width(),
                    full.top() + RULER / 2.0,
                )
            } else {
                pos2(
                    full.left() + RULER / 2.0,
                    full.top() + rng.unit() * full.height(),
                )
            }
        }
        6 => pos2(
            rect.left() + rect.width() * st.compare_split + rng.unit() * 10.0 - 5.0,
            rect.top() + rng.unit() * rect.height(),
        ),
        7 => pos2(rng.unit() * 1000.0, rng.unit() * 800.0),
        _ => pos2(
            full.left() + rng.unit() * full.width(),
            full.top() + rng.unit() * full.height(),
        ),
    }
}

fn trace_run(seed: u64, frames: usize, out: &mut String) {
    use std::fmt::Write;
    let mut rng = Rng(seed);
    let mut doc = trace_doc();
    let ctx = egui::Context::default();
    let mut state = CanvasState::default();
    let mut selected = Some(1 + (seed % 9));
    let mut time = 0.0;
    let mut pos = pos2(500.0, 400.0);
    let mut mods = egui::Modifiers::NONE;
    let mut down: Option<PointerButton> = None;
    let fonts = crate::document::text::FontLibrary::builtin_only();
    // A gesture in progress: the frames left and where it goes.
    let mut gesture: Option<(usize, Pos2, Pos2)> = None;
    let elements: Vec<PlacedElement> = outline([30.0, 30.0], [267.0, 180.0])
        .into_iter()
        .chain(anchors(&[[70.5, 120.5], [150.0, 60.0]]))
        .collect();
    // Settings varied per run.
    state.show_rulers = seed.is_multiple_of(2);
    state.snap_to_guides = !seed.is_multiple_of(3);
    state.snap_to_edges = seed % 4 != 1;
    state.snap_to_centers = seed % 5 != 2;
    state.lock_guides = seed % 7 == 3;
    if seed % 6 == 1 {
        state.compare = Some(Compare {
            elements: outline([40.0, 40.0], [250.0, 170.0]),
            colors: Vec::new(),
            labels: ["a".into(), "b".into()],
            on_left: seed % 12 == 1,
        });
    }
    // One frame first, so the canvas has its size.
    let mut o = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            max_texture_side: Some(8192),
            ..Default::default()
        },
        |ui| {
            show(ui, &mut doc, &elements, &mut selected, &mut state);
        },
    );
    o.textures_delta.clear();
    for f in 0..frames {
        time += if rng.below(10) == 0 { 0.6 } else { 0.03 };
        let mut events = Vec::new();
        if let Some((left, from, to)) = gesture {
            let k = 1.0 - left as f32 / 5.0;
            pos = from + (to - from) * k;
            events.push(Event::PointerMoved(pos));
            if left == 0 {
                events.push(Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: mods,
                });
                down = None;
                gesture = None;
            } else {
                gesture = Some((left - 1, from, to));
            }
        } else if down.is_none() && rng.below(6) == 0 {
            let from = interesting(&mut rng, &doc, selected, &state);
            let (cam, rect) = (state.camera, state.rect);
            let to = match rng.below(5) {
                // Onto an anchor or the template point (snapping, the link offer).
                0 => cam.to_screen(rect, [[70.5, 120.5], [150.0, 60.0]][rng.below(2)]),
                // A little way (a linked position held).
                1 => from + vec2(rng.unit() * 20.0 - 10.0, rng.unit() * 20.0 - 10.0),
                // Onto a guide.
                2 => cam.to_screen(rect, [120.0, 70.0]),
                _ => interesting(&mut rng, &doc, selected, &state),
            };
            // Ctrl (distort a corner, tilt) or Shift now and then.
            mods = egui::Modifiers {
                alt: rng.below(6) == 0,
                ctrl: false,
                shift: rng.below(5) == 0,
                mac_cmd: false,
                command: rng.below(3) == 0,
            };
            mods.ctrl = mods.command;
            events.push(Event::ModifiersChanged(mods));
            events.push(Event::PointerMoved(from));
            events.push(Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: mods,
            });
            pos = from;
            down = Some(PointerButton::Primary);
            gesture = Some((5, from, to));
        } else {
            match rng.below(20) {
                0..=6 => {
                    pos = if rng.below(3) == 0 {
                        pos + vec2(rng.unit() * 60.0 - 30.0, rng.unit() * 60.0 - 30.0)
                    } else {
                        interesting(&mut rng, &doc, selected, &state)
                    };
                    events.push(Event::PointerMoved(pos));
                }
                7..=9 => {
                    let button = match rng.below(8) {
                        0 => PointerButton::Secondary,
                        1 => PointerButton::Middle,
                        _ => PointerButton::Primary,
                    };
                    let pressed = down.is_none();
                    let button = down.unwrap_or(button);
                    events.push(Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: mods,
                    });
                    down = pressed.then_some(button);
                }
                10 | 11 => {
                    mods = egui::Modifiers {
                        alt: rng.below(4) == 0,
                        ctrl: false,
                        shift: rng.below(3) == 0,
                        mac_cmd: false,
                        command: rng.below(3) == 0,
                    };
                    mods.ctrl = mods.command;
                    events.push(Event::ModifiersChanged(mods));
                }
                12 => {
                    let unit = [
                        MouseWheelUnit::Point,
                        MouseWheelUnit::Line,
                        MouseWheelUnit::Page,
                    ][rng.below(3)];
                    let source = [
                        MouseWheelSource::Wheel,
                        MouseWheelSource::Unknown,
                        MouseWheelSource::Trackpad,
                    ][rng.below(3)];
                    events.push(Event::MouseWheel {
                        unit,
                        delta: vec2(rng.unit() * 4.0 - 2.0, rng.unit() * 4.0 - 2.0),
                        modifiers: mods,
                        phase: egui::TouchPhase::Move,
                        source,
                    });
                }
                13 => events.push(Event::Zoom(0.8 + rng.unit() * 0.4)),
                14 => {
                    let key = [Key::Space, Key::Escape, Key::Enter, Key::A][rng.below(4)];
                    let pressed = rng.below(2) == 0;
                    events.push(Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: mods,
                    });
                }
                15 => {
                    // A double click where the pointer is.
                    if down.is_none() {
                        for pressed in [true, false, true, false] {
                            events.push(Event::PointerButton {
                                pos,
                                button: PointerButton::Primary,
                                pressed,
                                modifiers: mods,
                            });
                        }
                    }
                }
                16 => {
                    // Tools, as the tool bar or the menus set them.
                    match rng.below(6) {
                        0 => state.eyedropper = !state.eyedropper,
                        1 => state.brushes.tool = Some(crate::document::paint::Tool::Pencil),
                        2 => state.brushes.tool = Some(crate::document::paint::Tool::Eraser),
                        3 => state.brushes.tool = None,
                        4 => {
                            selected = [
                                None,
                                Some(1),
                                Some(2),
                                Some(5),
                                Some(6),
                                Some(7),
                                Some(8),
                                Some(9),
                            ][rng.below(8)]
                        }
                        _ => state.show_guides = !state.show_guides,
                    }
                }
                17 => events.push(Event::Text("x".into())),
                _ => {}
            }
        }
        if std::env::var("TRACE_DEBUG").is_ok()
            && events.iter().any(
                |e| matches!(e, Event::PointerMoved(p) if !p.x.is_finite() || !p.y.is_finite()),
            )
        {
            let last: Vec<&str> = out.lines().rev().take(2).collect();
            panic!(
                "NaN pointer at {seed}/{f}; last lines:\n{}",
                last.join("\n")
            );
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
            max_texture_side: Some(8192),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut changed = false;
        let mut o = ctx.run_ui(input, |ui| {
            crate::document::shape::sync(ui.ctx(), &mut doc, true);
            crate::document::text::sync(ui.ctx(), &mut doc, &fonts);
            changed = show(ui, &mut doc, &elements, &mut selected, &mut state);
        });
        o.textures_delta.clear();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", o.shapes).hash(&mut h);
        let drag = state.drag.as_ref().map(|d| {
            (
                drag_kind_name(d.kind),
                d.layers.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                d.linked,
                d.held,
            )
        });
        writeln!(
            out,
            "{seed}/{f} ch={changed} sel={selected:?} cam={:?} rect={:?} drag={drag:?} pan={} gd={:?} gh={:?} ge={:?} sn={:?} sng={:?} lo={:?} pts={:?} draw={} typ={} cur={:?} mm={:?} split={} sg={} tool={:?} eye={} shapes={:x} doc={}",
            state.camera, state.rect, state.panning, state.guide_drag, state.guide_hover,
            state.guide_edit, state.snapped, state.snapped_guides, state.link_offer,
            state.points.as_ref().map(|p| p.layer), state.drawing.is_some(), state.typing.is_some(),
            o.platform_output.cursor_icon, state.cursor_mm, state.compare_split, state.show_guides,
            state.brushes.tool, state.eyedropper, h.finish(),
            serde_json::to_string(&doc).unwrap(),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "writes the trace to $CANVAS_TRACE, to compare before and after a refactor"]
fn show_trace() {
    let mut out = String::new();
    for seed in 1..=60 {
        trace_run(seed * 0x9E37_79B9 + 1, 300, &mut out);
    }
    std::fs::write(std::env::var("CANVAS_TRACE").unwrap(), out).unwrap();
}
