//! Cut, copy and paste of layers and groups.
//!
//! The copied node is kept here, and also put on the system clipboard as JSON after a
//! first line that says what it is. The system clipboard has to hold something: egui
//! turns Ctrl+V into a paste event carrying the clipboard's text, and sends none when it
//! is empty. It also lets another window of the app paste the layers. Whatever is copied
//! last, here or in another program, is what Ctrl+V pastes.

use crate::document::{Document, Node};
use crate::layers_ui;

/// First line of the clipboard text, before the node's JSON.
const HEADER: &str = "Artwork Creator layers 1\n";

/// How far each copy is moved from the one before, in mm, as for Duplicate.
const STEP_MM: f32 = 5.0;

/// The layer or group last cut or copied.
#[derive(Default)]
pub struct Clipboard {
    /// The node as copied, with its loaded images, while `text` is on the clipboard.
    node: Option<Node>,
    /// What was put on the system clipboard, or the last pasted text from elsewhere.
    text: String,
    /// The id the node had where it was copied from.
    source: Option<u64>,
    /// Pastes of `text` so far; each one lands a step further away.
    pastes: u32,
    /// The first paste goes where the node was: it was cut, or comes from elsewhere.
    in_place: bool,
}

/// What a paste added.
pub struct Pasted {
    /// The pasted node, now selected.
    pub id: u64,
    /// It came from another window of the app: its images still have to be loaded.
    pub foreign: bool,
}

impl Clipboard {
    /// Something was cut or copied here, so the Paste menu entry can be enabled.
    pub fn has_content(&self) -> bool {
        self.node.is_some()
    }

    /// Copies the selected node. Returns the text to put on the system clipboard.
    pub fn copy(&mut self, doc: &Document, selected: Option<u64>) -> Option<String> {
        let node = doc.node(selected?)?.clone();
        let text = format!("{HEADER}{}", serde_json::to_string(&node).ok()?);
        *self = Self {
            node: Some(node),
            text: text.clone(),
            source: selected,
            pastes: 0,
            in_place: false,
        };
        Some(text)
    }

    /// Copies the selected node, then deletes it. Returns the text to put on the system
    /// clipboard.
    pub fn cut(&mut self, doc: &mut Document, selected: &mut Option<u64>) -> Option<String> {
        let text = self.copy(doc, *selected)?;
        layers_ui::delete(doc, selected);
        self.in_place = true;
        Some(text)
    }

    /// Pastes the layers `text` (the system clipboard's) holds: above the selection, or
    /// at the top of a selected group, then selects them. `None` if the text holds none.
    pub fn paste(
        &mut self,
        doc: &mut Document,
        selected: &mut Option<u64>,
        text: &str,
    ) -> Option<Pasted> {
        let foreign = text != self.text || self.node.is_none();
        let mut node = if foreign {
            let node: Node = serde_json::from_str(text.strip_prefix(HEADER)?).ok()?;
            // Copied in another window: pasted where it was there, then a step further.
            if text != self.text {
                *self = Self {
                    node: None,
                    text: text.to_owned(),
                    source: None,
                    pastes: 0,
                    in_place: true,
                };
            }
            node
        } else {
            self.node.clone()?
        };
        let steps = self.pastes + u32::from(!self.in_place);
        self.pastes += 1;
        let d = STEP_MM * steps as f32;
        node.for_each_layer_mut(&mut |l| {
            l.transform.center[0] += d;
            l.transform.center[1] += d;
        });
        layers_ui::give_fresh_ids(doc, &mut node);
        layers_ui::name_copy(doc, &mut node);
        // From another project, its links name color and text variables this one may
        // not have.
        crate::colors::adopt_links(&mut doc.variables, &mut node);
        crate::text_links::adopt_links(&mut doc.variables, &mut node);
        // Over the copied group itself, the copy goes next to it rather than inside.
        let path = match selected.filter(|&s| Some(s) == self.source) {
            Some(s) => doc.path_of(s).map(|mut p| {
                *p.last_mut().unwrap() += 1;
                p
            }),
            None => None,
        }
        .unwrap_or_else(|| layers_ui::insertion_path(doc, *selected));
        let id = node.id();
        doc.insert(&path, node);
        *selected = Some(id);
        Some(Pasted { id, foreign })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Group, Layer};

    fn layer(id: u64, x: f32) -> Node {
        let mut l = Layer::new(id, format!("l{id}"), "x.png".into());
        l.transform.center = [x, 0.0];
        Node::Layer(Box::new(l))
    }

    fn doc_with(nodes: Vec<Node>) -> Document {
        Document {
            next_id: 100,
            layers: nodes,
            ..Document::default()
        }
    }

    fn ids(nodes: &[Node]) -> Vec<u64> {
        nodes.iter().map(Node::id).collect()
    }

    fn x(doc: &Document, id: u64) -> f32 {
        doc.layer(id).unwrap().transform.center[0]
    }

    #[test]
    fn copies_step_away_and_cut_pastes_in_place() {
        let mut d = doc_with(vec![layer(1, 10.0), layer(2, 20.0)]);
        let mut clip = Clipboard::default();
        let text = clip.copy(&d, Some(1)).unwrap();
        assert!(text.starts_with(HEADER));
        let mut sel = Some(1);
        let p = clip.paste(&mut d, &mut sel, &text).unwrap();
        assert!(!p.foreign);
        assert_eq!((p.id, sel), (100, Some(100)));
        assert_eq!(ids(&d.layers), [1, 100, 2], "just above the selection");
        assert_eq!(x(&d, 100), 15.0);
        clip.paste(&mut d, &mut sel, &text).unwrap();
        assert_eq!(x(&d, 101), 20.0, "each paste a step further");
        assert_eq!(d.layer(100).unwrap().name, "l1 copy");
        assert_eq!(d.layer(101).unwrap().name, "l1 copy (2)");

        let mut sel = Some(2);
        let text = clip.cut(&mut d, &mut sel).unwrap();
        assert!(d.node(2).is_none());
        clip.paste(&mut d, &mut sel, &text).unwrap();
        assert_eq!(x(&d, 102), 20.0, "the first paste of a cut is in place");
        assert_eq!(d.layer(102).unwrap().name, "l2", "the original is gone");
        clip.paste(&mut d, &mut sel, &text).unwrap();
        assert_eq!(x(&d, 103), 25.0);
    }

    #[test]
    fn other_text_pastes_nothing() {
        let mut d = doc_with(vec![layer(1, 0.0)]);
        let mut clip = Clipboard::default();
        clip.copy(&d, Some(1)).unwrap();
        let mut sel = Some(1);
        assert!(clip.paste(&mut d, &mut sel, "some text").is_none());
        assert!(
            clip.paste(&mut d, &mut sel, &format!("{HEADER}not json"))
                .is_none()
        );
        assert_eq!(ids(&d.layers), [1]);
    }

    #[test]
    fn pastes_layers_copied_in_another_window() {
        let other = doc_with(vec![layer(7, 30.0)]);
        let text = Clipboard::default().copy(&other, Some(7)).unwrap();
        let mut d = doc_with(vec![layer(1, 0.0)]);
        let mut clip = Clipboard::default();
        let mut sel = None;
        let p = clip.paste(&mut d, &mut sel, &text).unwrap();
        assert!(p.foreign);
        assert_eq!(ids(&d.layers), [1, 100], "on top without a selection");
        assert_eq!(x(&d, 100), 30.0, "in place first");
        clip.paste(&mut d, &mut sel, &text).unwrap();
        assert_eq!(x(&d, 101), 35.0);
    }

    #[test]
    fn a_copied_group_pastes_beside_itself_with_its_texts_following_the_copied_shapes() {
        let mut shape = Layer::new(2, "shape".into(), "".into());
        shape.shape = Some(Box::default());
        let mut follower = Layer::new(3, "text".into(), "".into());
        let mut t = crate::text::Text::default();
        t.path.kind = crate::text::PathKind::Shape;
        t.path.shape = 2;
        follower.text = Some(Box::new(t));
        let group = Node::Group(Group {
            id: 1,
            name: "g".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            expanded: true,
            children: vec![
                Node::Layer(Box::new(shape)),
                Node::Layer(Box::new(follower)),
            ],
        });
        let mut d = doc_with(vec![group]);
        let mut clip = Clipboard::default();
        let text = clip.copy(&d, Some(1)).unwrap();
        let mut sel = Some(1);
        clip.paste(&mut d, &mut sel, &text).unwrap();
        assert_eq!(ids(&d.layers), [1, 100], "beside the group, not inside");
        let followed = |d: &Document, id| d.layer(id).unwrap().text.as_ref().unwrap().path.shape;
        assert_eq!(followed(&d, 102), 101, "follows the pasted shape");

        // Alone, a text keeps following its shape here, and drops one missing elsewhere.
        let text = clip.copy(&d, Some(3)).unwrap();
        let mut sel = Some(3);
        let p = clip.paste(&mut d, &mut sel, &text).unwrap();
        assert_eq!(followed(&d, p.id), 2);
        let mut elsewhere = doc_with(Vec::new());
        let p = Clipboard::default()
            .paste(&mut elsewhere, &mut None, &text)
            .unwrap();
        assert_eq!(followed(&elsewhere, p.id), 0);
    }
}
