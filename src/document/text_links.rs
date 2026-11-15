//! Text layers linked to text variables (see `variables`), by name, like colors linked
//! to color variables (see `colors`): each frame, [`apply`] gives every linked text its
//! variable's text, so changing a text variable changes all the texts using it. Typing
//! in a linked text unlinks it. A text whose variable is gone keeps its link, broken,
//! and its last text; a text variable of that name links it again.

use crate::document::text;
use crate::document::variables::{Value, Variable};
use crate::document::{Document, Layer, Node};

/// The text variable `name`'s text, if there is one.
pub fn find<'a>(vars: &'a [Variable], name: &str) -> Option<&'a str> {
    vars.iter().find(|v| v.name == name)?.value.text()
}

/// Gives every linked text its text variable's text; the layer's name follows when it
/// was named after its text. Returns true if the document changed.
pub fn apply(doc: &mut Document) -> bool {
    let vars = std::mem::take(&mut doc.variables);
    let mut changed = false;
    doc.for_each_layer_mut(|layer| {
        let Some(t) = layer.text.as_deref_mut() else {
            return;
        };
        let Some(new) = t.link.as_deref().and_then(|name| find(&vars, name)) else {
            return;
        };
        if t.content != new {
            if layer.name == text::auto_name(&t.content) {
                layer.name = text::auto_name(new);
            }
            t.content = new.to_owned();
            changed = true;
        }
    });
    doc.variables = vars;
    changed
}

/// Every text layer's link, `f` returning false for one to remove.
fn for_each_link(doc: &mut Document, mut f: impl FnMut(&mut String) -> bool) {
    doc.for_each_layer_mut(|layer| {
        if let Some(t) = layer.text.as_deref_mut()
            && let Some(name) = &mut t.link
            && !f(name)
        {
            t.link = None;
        }
    });
}

/// How many texts are linked to the variable `name`.
pub fn use_count(doc: &mut Document, name: &str) -> usize {
    let mut n = 0;
    for_each_link(doc, |link| {
        n += usize::from(link == name);
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

/// Fits the text links of `node`, pasted from the clipboard, to the variables `vars`,
/// as `colors::adopt_links` does for colors: a link stays when there is a text variable
/// of that name and text; it moves to the text variable of that text when there is one;
/// otherwise its text is saved as a variable of its name. A link whose name another
/// variable has is dropped, its text kept as it is.
pub fn adopt_links(vars: &mut Vec<Variable>, node: &mut Node) {
    node.for_each_layer_mut(&mut |l: &mut Layer| {
        let Some(t) = l.text.as_deref_mut() else {
            return;
        };
        let Some(name) = t.link.clone() else {
            return;
        };
        if find(vars, &name) == Some(t.content.as_str()) {
            return;
        }
        if let Some(v) = vars
            .iter()
            .find(|v| v.value.text() == Some(t.content.as_str()))
        {
            t.link = Some(v.name.clone());
        } else if vars.iter().any(|v| v.name == name) {
            t.link = None;
        } else {
            vars.push(Variable {
                formula: None,
                name,
                value: Value::Text(t.content.clone()),
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tv(name: &str, text: &str) -> Variable {
        Variable {
            formula: None,
            name: name.into(),
            value: Value::Text(text.into()),
        }
    }

    fn doc_with_text(content: &str, link: Option<&str>) -> Document {
        let mut layer = text::new_layer(1, content, "x".into(), Default::default());
        layer.text.as_mut().unwrap().link = link.map(Into::into);
        Document {
            layers: vec![Node::Layer(Box::new(layer))],
            variables: vec![tv("title", "Hello")],
            ..Default::default()
        }
    }

    fn content(doc: &Document) -> (&str, &str) {
        let l = doc.layer(1).unwrap();
        (l.text.as_ref().unwrap().content.as_str(), l.name.as_str())
    }

    #[test]
    fn linked_texts_follow_their_variable_and_keep_their_text_when_it_is_gone() {
        let mut doc = doc_with_text("Old", Some("title"));
        assert!(apply(&mut doc));
        assert!(!apply(&mut doc), "nothing more to do");
        assert_eq!(content(&doc), ("Hello", "Hello"), "the name follows too");
        assert_eq!(use_count(&mut doc, "title"), 1);

        doc.variables[0] = tv("title", "Bye");
        apply(&mut doc);
        assert_eq!(content(&doc).0, "Bye");

        rename_links(&mut doc, "title", "heading");
        doc.variables[0].name = "heading".into();
        doc.variables[0].value = Value::Text("Hi".into());
        apply(&mut doc);
        assert_eq!(content(&doc).0, "Hi");

        // Removed: the text stays, its link kept, broken.
        doc.variables.clear();
        assert!(!apply(&mut doc));
        assert_eq!(content(&doc).0, "Hi");
        let link = doc.layer(1).unwrap().text.as_ref().unwrap().link.clone();
        assert_eq!(link.as_deref(), Some("heading"));
    }

    #[test]
    fn pasted_text_links_fit_the_variables_of_the_project_they_land_in() {
        let pasted = |content: &str, vars: &mut Vec<Variable>| {
            let mut doc = doc_with_text(content, Some("title"));
            let mut node = doc.layers.remove(0);
            adopt_links(vars, &mut node);
            match node {
                Node::Layer(l) => l.text.unwrap().link,
                _ => unreachable!(),
            }
        };
        // Same name and text: kept.
        let mut vars = vec![tv("title", "Hello")];
        assert_eq!(pasted("Hello", &mut vars).as_deref(), Some("title"));
        // Another variable has the text: moved to it.
        let mut vars = vec![tv("greeting", "Hello")];
        assert_eq!(pasted("Hello", &mut vars).as_deref(), Some("greeting"));
        // Neither: the text is saved under its name.
        let mut vars = Vec::new();
        assert_eq!(pasted("Hello", &mut vars).as_deref(), Some("title"));
        assert_eq!(vars, [tv("title", "Hello")]);
        // The name is another variable's: unlinked.
        let mut vars = vec![tv("title", "Other")];
        assert_eq!(pasted("Hello", &mut vars), None);
    }

    #[test]
    fn text_variables_are_saved_with_their_kind() {
        let vars = vec![tv("title", "Two\nlines")];
        let json = serde_json::to_string(&vars).unwrap();
        assert_eq!(
            json,
            r#"[{"name":"title","kind":"text","value":"Two\nlines"}]"#
        );
        let back: Vec<Variable> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, vars);
        assert_eq!(vars[0].value.short_text(), "\"Two...\"");
    }
}
