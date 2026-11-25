//! The link button of a text layer linked to a text variable. The links themselves are
//! in `document::text_links`.

use crate::document::text_links::find;
use crate::document::variables::{Kind, Variable};
use crate::i18n::tr;
use crate::ui::variables as variables_ui;
use crate::ui::variables::LinkAction;
use egui::Ui;

/// Unlinks a text typed in (`link` being its text's link), with a toast saying so.
pub fn typed_in(ctx: &egui::Context, link: &mut Option<String>) {
    use crate::ui::toast::{self, Intent, Toast};
    if let Some(name) = link.take() {
        let text = tr!("TEXT_UNLINKED_BY_TYPING", name = name);
        let title = tr!("TEXT_UNLINKED_TITLE");
        toast::push(ctx, Toast::new("text_unlinked", Intent::Info, title, &text));
    }
}

/// The link button of a text layer's text: the text variables, a click on one linking
/// the text to it, then Unlink, which keeps the text. Returns true if the link changed.
pub fn link_button(ui: &mut Ui, link: &mut Option<String>) -> bool {
    let vars = variables_ui::published(ui.ctx());
    let state = match link.as_deref() {
        None => variables_ui::Link::None,
        Some(name) if find(&vars, name).is_some() => variables_ui::Link::Linked(String::new()),
        Some(name) => variables_ui::Link::Broken(tr!("BROKEN_TEXT_LINK_HINT", name = name)),
    };
    let candidates: Vec<&Variable> = vars
        .iter()
        .filter(|v| v.value.kind() == Kind::Text)
        .collect();
    match variables_ui::link_button(ui, &state, link.as_deref(), &candidates) {
        LinkAction::Pick(name) => {
            *link = Some(name);
            true
        }
        LinkAction::Unlink => {
            *link = None;
            true
        }
        LinkAction::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_in_a_linked_text_unlinks_it_with_a_toast() {
        let ctx = egui::Context::default();
        let mut link = None;
        typed_in(&ctx, &mut link);
        assert!(
            crate::ui::toast::queued(&ctx).is_empty(),
            "not linked: nothing to say"
        );
        let mut link = Some("title".to_owned());
        typed_in(&ctx, &mut link);
        assert_eq!(link, None);
        let queued = crate::ui::toast::queued(&ctx);
        assert!(
            queued.len() == 1 && queued[0].text.contains("title"),
            "{queued:?}"
        );
    }
}
