//! Typing a text layer's text in place on the canvas.
//!
//! The keyboard goes to a hidden egui `TextEdit` holding the layer's text, which does
//! the editing: cursor moves, selection, clipboard, undo while typing, IME. The canvas
//! draws the cursor and selection on the layer itself, from the text's own layout, and
//! turns clicks and drags on the layer into cursor positions. Up and down follow the
//! lines as laid out on the canvas.

use egui::text::{CCursor, CCursorRange};
use egui::{
    Color32, CursorIcon, Event, Id, Key, PointerButton, Pos2, Rect, Response, Shape, Stroke,
    TextEdit, UiBuilder, Vec2, pos2, vec2,
};

use crate::document::{Document, Layer, P2};
use crate::text;
use crate::view::Camera;

/// Typing in progress in a text layer on the canvas.
#[derive(Debug)]
pub struct Typing {
    pub layer: u64,
    /// Cursor to give the text field before it next reads the keyboard: the fixed end of
    /// the selection, then the moving one, as character indices.
    pending: Option<(usize, usize)>,
    /// A left drag from a press on the text is selecting from this character.
    selecting: Option<usize>,
    /// The text field has been given the keyboard: losing it ends the typing.
    focused: bool,
}

fn field_id() -> Id {
    Id::unique("canvas_text_field")
}

/// Starts typing in `layer`, the characters from `anchor` to `cursor` selected.
pub fn begin(slot: &mut Option<Typing>, layer: u64, anchor: usize, cursor: usize) {
    *slot = Some(Typing {
        layer,
        pending: Some((anchor, cursor)),
        selecting: None,
        focused: false,
    });
}

/// Index of the character a point on the page stands before, in a text layer.
pub fn index_at(layer: &Layer, mm: P2) -> usize {
    let p = layer.homography().inverse().map_or(mm, |h| h.apply(mm));
    text::caret_at(&layer.text_state.carets, p)
}

/// The text field's selection: fixed end, moving end.
fn selection(ctx: &egui::Context) -> Option<(usize, usize)> {
    let st = TextEdit::load_state(ctx, field_id())?;
    let r = st.cursor.char_range()?;
    Some((r.secondary.index.0, r.primary.index.0))
}

/// The word around character `i`: letters and digits, or else the run of other
/// characters, up to the line's end.
fn word_at(content: &str, i: usize) -> (usize, usize) {
    let chars: Vec<char> = content.chars().collect();
    let kind = |c: char| {
        if c == '\n' {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            2
        } else {
            3
        }
    };
    let i = i.min(chars.len());
    // Past a word's end, the click is still on that word.
    let at = if i == chars.len() || (kind(chars[i]) != 1 && i > 0 && kind(chars[i - 1]) == 1) {
        i.saturating_sub(1)
    } else {
        i
    };
    let Some(&c) = chars.get(at) else {
        return (i, i);
    };
    let k = kind(c);
    if k == 0 {
        return (at, at);
    }
    let mut a = at;
    while a > 0 && kind(chars[a - 1]) == k {
        a -= 1;
    }
    let mut b = at;
    while b < chars.len() && kind(chars[b]) == k {
        b += 1;
    }
    (a, b)
}

/// What typing did with the pointer this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pointer {
    /// A press, click or drag on the text: it moved the cursor.
    pub took: bool,
    /// The pointer is over the text being typed in.
    pub over_text: bool,
}

/// Before the canvas' own handling: ends the typing when it should, and takes presses,
/// clicks and drags on the text, which move the cursor. `on_handle` is true when the
/// pointer is on a handle of the selected layer: those keep working while typing.
#[allow(clippy::too_many_arguments)]
pub fn input(
    ui: &egui::Ui,
    doc: &Document,
    selected: Option<u64>,
    slot: &mut Option<Typing>,
    cam: Camera,
    rect: Rect,
    response: &Response,
    on_handle: bool,
) -> Pointer {
    let Some(t) = slot else {
        return Pointer::default();
    };
    let layer = doc
        .layer(t.layer)
        .filter(|l| crate::text::is_straight(l))
        .filter(|_| selected == Some(t.layer))
        .filter(|_| !doc.effectively_locked(t.layer) && doc.effectively_visible(t.layer));
    // Escape, or a click in another field, took the keyboard.
    let lost = t.focused && !ui.memory(|m| m.has_focus(field_id()));
    let Some(layer) = layer.filter(|_| !lost) else {
        *slot = None;
        return Pointer::default();
    };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let on_text = pointer
        .filter(|_| response.hovered() || response.is_pointer_button_down_on())
        .filter(|&p| layer.contains(cam.to_mm(rect, p)))
        .filter(|_| !on_handle);
    let index = |p: Pos2| index_at(layer, cam.to_mm(rect, p));
    let (pressed, down, shift) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.primary_down(),
            i.modifiers.shift,
        )
    });

    if pressed {
        let Some(p) = on_text else {
            // A press anywhere else ends the typing.
            *slot = None;
            return Pointer::default();
        };
        let i = index(p);
        let anchor = if shift {
            selection(ui.ctx()).map_or(i, |s| s.0)
        } else {
            i
        };
        t.pending = Some((anchor, i));
        t.selecting = Some(anchor);
    } else if response.double_clicked_by(PointerButton::Primary)
        && let Some(p) = on_text
        && let Some(content) = layer.text.as_ref().map(|t| &t.content)
    {
        let (a, b) = word_at(content, index(p));
        t.pending = Some((a, b));
        t.selecting = None;
    }
    let over_text = on_text.is_some();
    if over_text {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    let took = if let Some(anchor) = t.selecting {
        if down {
            if let Some(p) = pointer {
                t.pending = Some((anchor, index(p)));
            }
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        } else {
            t.selecting = None;
        }
        // Down to the release of that press: not a drag or a click for the canvas.
        true
    } else {
        over_text
            && (response.clicked_by(PointerButton::Primary)
                || response.double_clicked_by(PointerButton::Primary))
    };
    Pointer { took, over_text }
}

/// The hidden text field, given the keyboard. Returns true if the text changed.
pub fn field(
    ui: &mut egui::Ui,
    doc: &mut Document,
    slot: &mut Option<Typing>,
    cam: Camera,
    rect: Rect,
) -> bool {
    let Some(t) = slot else { return false };
    let Some(layer) = doc.layer_mut(t.layer) else {
        *slot = None;
        return false;
    };
    let ctx = ui.ctx().clone();
    let id = field_id();
    let carets = layer.text_state.carets.clone();
    let mut st = TextEdit::load_state(&ctx, id).unwrap_or_default();
    let mut range = st.cursor.char_range();
    if let Some((a, c)) = t.pending.take() {
        range = Some(CCursorRange::two(CCursor::new(a), CCursor::new(c)));
    }
    // Up and down go to the line above or below as laid out, or to the text's start or
    // end from the first or last line.
    for (key, step) in [(Key::ArrowUp, -1i32), (Key::ArrowDown, 1)] {
        // Without Ctrl or Alt; Shift extends the selection.
        while let Some(shift) = ui.input_mut(|i| {
            let at = i.events.iter().position(|e| {
                matches!(e, Event::Key { key: k, pressed: true, modifiers: m, .. }
                    if *k == key && !m.alt && !m.command && !m.ctrl)
            })?;
            match i.events.remove(at) {
                Event::Key { modifiers, .. } => Some(modifiers.shift),
                _ => None,
            }
        }) {
            if carets.is_empty() {
                continue;
            }
            let r = range.unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
            let i = r.primary.index.0;
            let Some((line, x)) = text::caret_place(&carets, i) else {
                continue;
            };
            let to = line as i32 + step;
            let j = if to < 0 {
                0
            } else if to as usize >= carets.len() {
                carets
                    .iter()
                    .map(|l| l.xs.len())
                    .sum::<usize>()
                    .saturating_sub(1)
            } else {
                text::caret_on_line(&carets, to as usize, x)
            };
            let anchor = if shift { r.secondary.index.0 } else { j };
            range = Some(CCursorRange::two(CCursor::new(anchor), CCursor::new(j)));
        }
    }
    st.cursor.set_char_range(range);
    st.store(&ctx, id);
    // Focused before it runs: without the keyboard, the field drops the selection.
    if !ctx.memory(|m| m.has_focus(id)) {
        ctx.memory_mut(|m| m.request_focus(id));
    }

    // Where the cursor is on screen, for the input method's window.
    let to_screen = layer.homography().then_affine(cam.affine(rect));
    let at = range
        .and_then(|r| text::caret_place(&carets, r.primary.index.0))
        .map(|(line, x)| {
            let p = to_screen.apply([x, carets[line].top]);
            pos2(p[0], p[1])
        })
        .filter(|p| p.is_finite())
        .unwrap_or(rect.center());

    let Some(text) = layer.text.as_deref_mut() else {
        *slot = None;
        return false;
    };
    // The name follows the text until it is renamed, as in the Properties panel.
    let auto = layer.name == text::auto_name(&text.content);
    // Clipped to nothing: neither drawn nor under the pointer.
    let mut child =
        ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(at, vec2(1e4, 1e4))));
    child.set_clip_rect(Rect::from_min_size(at, Vec2::ZERO));
    let r = child.add(
        TextEdit::multiline(&mut text.content)
            .id(id)
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY),
    );
    // A press on the text took the keyboard from the field (it is not under the
    // pointer): it keeps it.
    if !r.has_focus() {
        r.request_focus();
    }
    t.focused = true;
    if r.changed() {
        // Typed in, the text is no longer its variable's (see `text_links`).
        crate::text_links::typed_in(&ctx, &mut text.link);
        if auto {
            layer.name = text::auto_name(&text.content);
        }
        // Laid out again on the next frame, then the cursor drawn there.
        ctx.request_repaint();
        return true;
    }
    false
}

/// Draws the cursor and the selection on the layer being typed in.
pub fn paint(ui: &egui::Ui, doc: &Document, slot: &Option<Typing>, cam: Camera, rect: Rect) {
    let Some(t) = slot else { return };
    let Some(layer) = doc.layer(t.layer) else {
        return;
    };
    let carets = &layer.text_state.carets;
    let Some((anchor, cursor)) = selection(ui.ctx()) else {
        return;
    };
    let painter = ui.painter_at(rect);
    let m = layer.homography().then_affine(cam.affine(rect));
    let at = |x: f32, y: f32| {
        let p = m.apply([x, y]);
        pos2(p[0], p[1])
    };
    let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
    if lo < hi {
        let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.5);
        let mut start = 0;
        for line in carets.iter() {
            let n = line.xs.len() - 1;
            let (a, b) = (lo.max(start), hi.min(start + n));
            if a < b || (a == b && lo <= start + n && hi > start + n) {
                let (x0, mut x1) = (line.xs[a - start], line.xs[b - start]);
                // A selected line break shows as a little space at the line's end.
                if hi > start + n {
                    x1 += (line.bottom - line.top) * 0.25;
                }
                let quad = vec![
                    at(x0, line.top),
                    at(x1, line.top),
                    at(x1, line.bottom),
                    at(x0, line.bottom),
                ];
                painter.add(Shape::convex_polygon(quad, fill, Stroke::NONE));
            }
            start += n + 1;
        }
    }
    if let Some((line, x)) = text::caret_place(carets, cursor) {
        let l = &carets[line];
        let seg = [at(x, l.top), at(x, l.bottom)];
        // Dark on a light halo: seen on any color.
        painter.line_segment(seg, Stroke::new(3.0, Color32::from_white_alpha(200)));
        painter.line_segment(seg, Stroke::new(1.5, Color32::BLACK));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Node;
    use crate::view::{self, CanvasState};
    use egui::Modifiers;

    /// A canvas with one selected text layer, driven frame by frame.
    struct Harness {
        ctx: egui::Context,
        doc: Document,
        state: CanvasState,
        selected: Option<u64>,
        fonts: text::FontLibrary,
        time: f64,
    }

    impl Harness {
        fn new(content: &str) -> Self {
            let layer = text::new_layer(1, content, text::BUILTIN_FAMILY.into(), [100.0, 100.0]);
            let mut h = Self {
                ctx: egui::Context::default(),
                doc: Document {
                    layers: vec![Node::Layer(Box::new(layer))],
                    ..Default::default()
                },
                state: CanvasState::default(),
                selected: Some(1),
                fonts: text::FontLibrary::builtin_only(),
                time: 0.0,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut out = self.ctx.clone().run_ui(input, |ui| {
                text::sync(ui.ctx(), &mut self.doc, &self.fonts);
                view::show(ui, &mut self.doc, &[], &mut self.selected, &mut self.state);
            });
            out.textures_delta.clear();
        }

        /// Screen position of a point in the layer's frame.
        fn screen(&self, p: P2) -> Pos2 {
            let l = self.doc.layer(1).unwrap();
            let mm = l.homography().apply(p);
            let s = self.state.camera.to_screen(self.state.rect, mm);
            pos2(s.x, s.y)
        }

        fn click(&mut self, p: Pos2) {
            self.frame(vec![Event::PointerMoved(p)]);
            for pressed in [true, false] {
                self.frame(vec![Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
        }

        fn key(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]);
        }

        fn content(&self) -> &str {
            &self.doc.layer(1).unwrap().text.as_ref().unwrap().content
        }

        fn carets(&self) -> std::sync::Arc<Vec<text::CaretLine>> {
            self.doc.layer(1).unwrap().text_state.carets.clone()
        }
    }

    #[test]
    fn double_click_types_where_clicked() {
        let mut h = Harness::new("Hello");
        let c = h.carets();
        let line = &c[0];
        let mid = (line.top + line.bottom) / 2.0;
        // Between "He" and "llo".
        let at = h.screen([line.xs[2], mid]);
        h.click(at);
        assert!(h.state.typing.is_none(), "one click only selects");
        h.click(at);
        assert!(h.state.typing.is_some(), "a double click starts typing");
        h.frame(vec![Event::Text("y".into())]);
        h.frame(vec![]);
        assert_eq!(h.content(), "Heyllo");
        assert_eq!(
            h.doc.layer(1).unwrap().name,
            "Heyllo",
            "name follows the text"
        );
        // The app's shortcuts (Delete, arrows, undo) stay off while typing.
        assert!(h.ctx.egui_wants_keyboard_input());
        h.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(h.content(), "Hello");
        assert!(
            h.doc.layer(1).is_some(),
            "Backspace deletes a letter, not the layer"
        );

        // Escape stops typing and gives the keyboard back.
        h.key(Key::Escape, Modifiers::NONE);
        h.frame(vec![]);
        assert!(h.state.typing.is_none());
        assert!(!h.ctx.egui_wants_keyboard_input());
    }

    #[test]
    fn up_and_down_follow_the_lines_on_the_canvas() {
        let mut h = Harness::new("abc\nde\nfghi");
        canvas_begin(&mut h, 1, 1);
        h.frame(vec![]);
        h.key(Key::ArrowDown, Modifiers::NONE);
        let (_, i) = selection(&h.ctx).unwrap();
        let c = h.carets();
        let x = c[0].xs[1];
        assert_eq!(
            i,
            text::caret_on_line(&c, 1, x),
            "nearest place on the next line"
        );
        h.key(Key::ArrowDown, Modifiers::SHIFT);
        let (a, j) = selection(&h.ctx).unwrap();
        assert_eq!(a, i, "Shift keeps the start of the selection");
        assert!(j > 7);
        h.key(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(
            selection(&h.ctx).unwrap(),
            (11, 11),
            "past the last line: the end"
        );
        h.key(Key::ArrowUp, Modifiers::NONE);
        h.key(Key::ArrowUp, Modifiers::NONE);
        h.key(Key::ArrowUp, Modifiers::NONE);
        assert_eq!(
            selection(&h.ctx).unwrap(),
            (0, 0),
            "past the first line: the start"
        );
        assert_eq!(h.content(), "abc\nde\nfghi");
    }

    fn canvas_begin(h: &mut Harness, layer: u64, i: usize) {
        begin(&mut h.state.typing, layer, i, i);
    }

    #[test]
    fn a_click_elsewhere_stops_typing() {
        let mut h = Harness::new("Hello");
        let layer = h.doc.layer(1).unwrap();
        h.state.type_in(layer);
        h.frame(vec![]);
        assert_eq!(
            selection(&h.ctx),
            Some((0, 5)),
            "new text: all of it selected"
        );
        // A click on the text moves the cursor and keeps typing.
        let c = h.carets();
        let at = h.screen([c[0].xs[1], (c[0].top + c[0].bottom) / 2.0]);
        h.click(at);
        assert!(h.state.typing.is_some());
        assert_eq!(selection(&h.ctx), Some((1, 1)));
        assert_eq!(h.selected, Some(1));
        h.click(pos2(5.0, 5.0));
        assert!(h.state.typing.is_none());
        assert!(!h.ctx.egui_wants_keyboard_input());
    }

    #[test]
    fn words_around_a_character() {
        let s = "Hello, big world\nnext";
        assert_eq!(word_at(s, 1), (0, 5));
        // Right after a word: still that word.
        assert_eq!(word_at(s, 5), (0, 5));
        assert_eq!(word_at(s, 8), (7, 10));
        assert_eq!(word_at(s, 16), (11, 16));
        assert_eq!(word_at(s, 17), (17, 21));
        assert_eq!(word_at(s, 21), (17, 21));
        assert_eq!(word_at("", 0), (0, 0));
    }
}
