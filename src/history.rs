//! Undo / redo by document snapshots.
//!
//! The app reports the document after every frame. When it differs from the last
//! snapshot and the edit is finished (no mouse button held, no text field being edited),
//! the previous snapshot goes on the undo stack. So a whole drag, slider move or rename
//! is one step, and every kind of edit is covered without hooks in the editing code.
//! Snapshots are cheap: image pixels and textures are shared, not copied.

use crate::document::Document;

/// Oldest steps are dropped beyond this.
const MAX_STEPS: usize = 200;

/// Undo and redo stacks of document snapshots.
pub struct History {
    undo: Vec<Document>,
    redo: Vec<Document>,
    /// The document as of the last recorded step.
    current: Document,
    current_key: String,
    /// `current_key` when the project was last opened or saved.
    saved_key: String,
}

/// A copy of `doc` to keep as a step, without the on-screen effect results: they can be
/// large, and `EffectCache` brings them back.
fn snapshot(doc: &Document) -> Document {
    let mut d = doc.clone();
    d.for_each_layer_mut(|l| l.fx = None);
    d
}

/// What gets compared: everything saved to the project file.
fn key(doc: &Document) -> String {
    serde_json::to_string(doc).unwrap_or_default()
}

impl History {
    /// A history starting at `doc`, which counts as saved.
    pub fn new(doc: &Document) -> Self {
        let k = key(doc);
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            current: snapshot(doc),
            current_key: k.clone(),
            saved_key: k,
        }
    }

    /// Forgets all steps, e.g. after opening a project.
    pub fn reset(&mut self, doc: &Document) {
        *self = Self::new(doc);
    }

    /// Records `doc` as a new step if it changed and the edit is `finished`.
    pub fn update(&mut self, doc: &Document, finished: bool) {
        if !finished {
            return;
        }
        let k = key(doc);
        if k == self.current_key {
            return;
        }
        let prev = std::mem::replace(&mut self.current, snapshot(doc));
        self.undo.push(prev);
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.current_key = k;
    }

    /// There is a step to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// There is a step to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Steps back; `doc` becomes the previous state. Returns false if there is none.
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        // An edit not recorded yet (e.g. Ctrl+Z mid-drag) becomes a step first.
        self.update(doc, true);
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.current, prev));
        self.restore(doc);
        true
    }

    /// Re-applies the last undone step.
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        self.update(doc, true);
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.current, next));
        self.restore(doc);
        true
    }

    /// Copies the current snapshot back into `doc`.
    fn restore(&mut self, doc: &mut Document) {
        *doc = self.current.clone();
        self.current_key = key(doc);
    }

    /// The current state is what was last saved.
    pub fn mark_saved(&mut self, doc: &Document) {
        self.update(doc, true);
        self.saved_key = self.current_key.clone();
    }

    /// The document differs from the saved project.
    pub fn is_modified(&self) -> bool {
        self.current_key != self.saved_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Layer, Node};

    fn doc_with_layer() -> Document {
        let mut l = Layer::new(1, "a".into(), "a.png".into());
        l.size_px = [10, 10];
        Document {
            layers: vec![Node::Layer(l)],
            next_id: 2,
            ..Default::default()
        }
    }

    fn x(doc: &Document) -> f32 {
        doc.layer(1).unwrap().transform.center[0]
    }

    #[test]
    fn undo_redo_steps() {
        let mut doc = doc_with_layer();
        let mut h = History::new(&doc);
        assert!(!h.can_undo() && !h.is_modified());

        // A drag over several frames, recorded once the button is released.
        for step in 1..=5 {
            doc.layer_mut(1).unwrap().transform.center[0] = step as f32;
            h.update(&doc, false);
        }
        h.update(&doc, true);
        doc.layer_mut(1).unwrap().name = "renamed".into();
        h.update(&doc, true);
        h.update(&doc, true); // no change, no step

        assert!(h.undo(&mut doc));
        assert_eq!(doc.layer(1).unwrap().name, "a");
        assert_eq!(x(&doc), 5.0);
        assert!(h.undo(&mut doc));
        assert_eq!(x(&doc), 0.0, "the whole drag is one step");
        assert!(!h.undo(&mut doc));
        assert!(!h.is_modified(), "back to the opened state");

        assert!(h.redo(&mut doc));
        assert_eq!(x(&doc), 5.0);
        assert!(h.is_modified());

        // A new edit drops what could be redone.
        doc.layer_mut(1).unwrap().transform.center[0] = 9.0;
        h.update(&doc, true);
        assert!(!h.can_redo());
        assert!(h.undo(&mut doc));
        assert_eq!(x(&doc), 5.0);
    }

    #[test]
    fn deleted_layer_comes_back_with_its_image() {
        let mut doc = doc_with_layer();
        let img = image::RgbaImage::new(2, 2);
        doc.layer_mut(1).unwrap().pixmap = Some(std::sync::Arc::new(
            crate::render::premultiplied_pixmap(&img).unwrap(),
        ));
        let mut h = History::new(&doc);
        doc.layers.clear();
        h.update(&doc, true);
        assert!(h.undo(&mut doc));
        assert!(doc.layer(1).unwrap().pixmap.is_some());
    }

    #[test]
    fn unfinished_edit_is_recorded_before_undo() {
        let mut doc = doc_with_layer();
        let mut h = History::new(&doc);
        doc.layer_mut(1).unwrap().transform.center[0] = 3.0;
        h.update(&doc, false);
        assert!(h.undo(&mut doc));
        assert_eq!(x(&doc), 0.0);
        assert!(h.redo(&mut doc));
        assert_eq!(x(&doc), 3.0);
    }

    #[test]
    fn saving_sets_the_clean_point() {
        let mut doc = doc_with_layer();
        let mut h = History::new(&doc);
        doc.background = [0, 0, 0];
        h.update(&doc, true);
        assert!(h.is_modified());
        h.mark_saved(&doc);
        assert!(!h.is_modified());
        h.undo(&mut doc);
        assert!(h.is_modified(), "undoing past a save modifies the project");
    }
}
