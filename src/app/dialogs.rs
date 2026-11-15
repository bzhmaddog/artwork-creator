//! Dialogs, notices and menu entries shared by the whole app.

use super::*;

/// A two-column grid of labels and fields in the Page, Template or Print panel, its
/// labels' column as wide as the widest of `PANEL_LABELS`.
pub(super) fn panel_grid(ui: &egui::Ui, id: &str) -> egui::Grid {
    let width = PANEL_LABELS
        .iter()
        .map(|k| {
            egui::WidgetText::from(translate(k))
                .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 4.0])
        .min_col_width(width.ceil())
}

/// How the UI reacts to the mouse, for the app and the tests that drive its panels.
pub fn set_interaction(ctx: &egui::Context) {
    // Text fields stop editing as soon as the mouse is pressed anywhere else,
    // not only on a clean click (egui's default ignores presses that turn into drags).
    ctx.options_mut(|o| {
        o.input_options.surrender_focus_on = egui::SurrenderFocusOn::Presses;
    });
    ctx.all_styles_mut(|s| {
        // Labels can't be selected, so the text cursor only shows over what can be edited.
        s.interaction.selectable_labels = false;
    });
}

/// How long a notice stays in the status bar, in seconds.
pub(crate) const NOTICE_SECONDS: f64 = 6.0;

pub(super) fn notice_id() -> egui::Id {
    egui::Id::unique("status_notice")
}

/// Shows `text` as a warning in the status bar for a few seconds, without stopping
/// what the user is doing (unlike a message, which waits for OK).
pub(crate) fn notify(ctx: &egui::Context, text: String) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(notice_id(), (text, now)));
}

/// The notice shown in the status bar, if one is still up.
pub(crate) fn notice(ctx: &egui::Context) -> Option<String> {
    let (text, at) = ctx.data(|d| d.get_temp::<(String, f64)>(notice_id()))?;
    let left = at + NOTICE_SECONDS - ctx.input(|i| i.time);
    if left <= 0.0 {
        ctx.data_mut(|d| d.remove::<(String, f64)>(notice_id()));
        return None;
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    Some(text)
}

/// Room between a window's or dialog's border and what it holds, in points (egui's is 6).
pub(super) const DIALOG_MARGIN: i8 = 14;

/// A menu entry with its command's shortcut, if it has one, at the right.
pub(super) fn menu_entry<'a>(label: &'a str, shortcut: Option<&String>) -> egui::Button<'a> {
    let b = egui::Button::new(label);
    match shortcut {
        Some(s) => b.shortcut_text(s.as_str()),
        None => b,
    }
}

/// A modal dialog, framed like a window but with more room: egui frames modals like
/// menus.
pub(crate) fn dialog(ctx: &egui::Context, id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(id)).frame(
        egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(DIALOG_MARGIN)),
    )
}

/// The bottom row of a window: `ok` (enabled when `can`) at the right edge of what is
/// above it, Cancel on its left. Returns which was clicked: (ok, cancel).
pub(crate) fn dialog_buttons(ui: &mut egui::Ui, ok: &str, can: bool) -> (bool, bool) {
    dialog_button_row(ui, |ui| {
        let ok = ui.add_enabled(can, egui::Button::new(ok)).clicked();
        let cancel = ui.button(tr!("CANCEL")).clicked();
        (ok, cancel)
    })
}

/// The bottom row of a window or dialog, as wide as what is above it: `add` lays its
/// buttons out from the right edge, the main one first (the rightmost), then Cancel. A
/// button that should sit at the left edge (such as "Close without saving") goes last, in
/// [`at_left_edge`].
pub(crate) fn dialog_button_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // As wide as the rows above, not more: the window sizes itself to its contents.
    let width = ui.min_rect().width();
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}

/// In a [`dialog_button_row`], a button at the row's left edge, after the others.
pub(crate) fn at_left_edge<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), add)
        .inner
}

/// A window's contents with `DIALOG_MARGIN` of room around them. egui's window margin
/// also sets its title bar's, which stays as it is.
pub(super) fn padded<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let extra = (DIALOG_MARGIN - ui.spacing().window_margin.left).max(0);
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(extra))
        .show(ui, add)
        .inner
}

impl ArtworkApp {
    /// Help > About: the name, version and build date.
    pub(super) fn about_modal(&mut self, ctx: &egui::Context) {
        if !self.about {
            return;
        }
        let response = dialog(ctx, "about").show(ctx, |ui| {
            ui.set_min_width(260.0);
            ui.heading("Artwork Creator");
            ui.add_space(4.0);
            ui.label(tr!("ABOUT_VERSION", version = env!("CARGO_PKG_VERSION")));
            ui.label(tr!("ABOUT_BUILD_DATE", date = env!("BUILD_DATE")));
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| ui.button(tr!("OK")).clicked())
        });
        if response.inner || response.should_close() {
            self.about = false;
        }
    }

    /// Shows queued messages (errors, a canceled export) one at a time.
    pub(super) fn message_modal(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.messages.first().cloned() else {
            return;
        };
        let response = dialog(ctx, "message").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(msg);
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| ui.button(tr!("OK")).clicked())
        });
        if response.inner || response.should_close() {
            self.messages.remove(0);
        }
    }
}
