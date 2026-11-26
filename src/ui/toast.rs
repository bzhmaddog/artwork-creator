//! Toasts: short notices in a card at the bottom right of the main window, above the
//! status bar, newest at the bottom. Each has an intent (its color and icon), a title, a
//! text and maybe buttons (its calls to action). One goes away after [`SECONDS`], or when
//! one of its buttons or its close button is clicked; the pointer over it keeps it up. An
//! error stays up until it is closed.

use egui::{Color32, Frame, Margin, Pos2, Rect, Sense, Stroke, Ui, vec2};

use crate::i18n::tr;
use crate::ui::callout::{self, Tone};

/// How long a toast stays up, in seconds, from when it was shown or the pointer left it.
pub const SECONDS: f64 = 8.0;
/// Width of a toast's card.
const WIDTH: f32 = 340.0;
/// Room between the toasts and the window's edges, and between two toasts.
const GAP: f32 = 12.0;
/// Width of the accent bar along a toast's left edge.
const BAR: f32 = 3.0;

/// What a toast says, which sets its color and icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // Success not used yet; the gallery shows all.
pub enum Intent {
    /// Something to know: blue, an "i" in a circle.
    Info,
    /// Something done: green, a check mark in a circle.
    Success,
    /// Something to look out for: amber, a triangle with "!".
    Warning,
    /// Something that failed: red, a triangle with "!". Stays up until closed.
    Error,
}

impl Intent {
    /// Whether its toasts stay up until closed, instead of going away after [`SECONDS`].
    pub fn lasts(self) -> bool {
        self == Intent::Error
    }

    pub fn color(self, visuals: &egui::Visuals) -> Color32 {
        match self {
            Intent::Info => visuals.hyperlink_color,
            Intent::Success => crate::ui::tokens::tip(visuals),
            Intent::Warning => visuals.warn_fg_color,
            Intent::Error => visuals.error_fg_color,
        }
    }
}

/// A notice to show as a toast.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// Tells it from the others: what [`Toasts::show`] returns for its buttons. A toast
    /// pushed with the key of one still up replaces it.
    pub key: &'static str,
    pub intent: Intent,
    pub title: String,
    pub text: String,
    /// Its buttons, left to right.
    pub actions: Vec<String>,
}

impl Toast {
    /// A toast without buttons; `text` may be empty (the title says it all).
    pub fn new(key: &'static str, intent: Intent, title: &str, text: &str) -> Self {
        Self {
            key,
            intent,
            title: title.to_owned(),
            text: text.to_owned(),
            actions: Vec::new(),
        }
    }

    /// With a button `label` after the others.
    pub fn action(mut self, label: &str) -> Self {
        self.actions.push(label.to_owned());
        self
    }
}

fn queue_id() -> egui::Id {
    egui::Id::unique("toast_queue")
}

/// Shows `toast` from the next frame on, from code that has the context but not the
/// app's [`Toasts`].
pub fn push(ctx: &egui::Context, toast: Toast) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Toast>>(queue_id())
            .push(toast)
    });
}

/// The toasts pushed with [`push`] and not shown yet.
#[cfg(test)]
pub fn queued(ctx: &egui::Context) -> Vec<Toast> {
    ctx.data(|d| d.get_temp::<Vec<Toast>>(queue_id()).unwrap_or_default())
}

/// The toasts up, with when each goes away (in the context's time).
#[derive(Default)]
pub struct Toasts {
    up: Vec<(Toast, Option<f64>)>,
}

impl Toasts {
    /// Shows `toast` from the next frame on.
    pub fn push(&mut self, toast: Toast) {
        self.up.retain(|(t, _)| t.key != toast.key);
        self.up.push((toast, None));
    }

    /// Whether the toast `key` is up.
    pub fn is_up(&self, key: &str) -> bool {
        self.up.iter().any(|(t, _)| t.key == key)
    }

    /// Draws the toasts, their bottom right corner at `corner`; the buttons clicked, by
    /// toast key and index in its actions, their toasts gone.
    pub fn show(&mut self, ctx: &egui::Context, corner: Pos2) -> Vec<(&'static str, usize)> {
        let queued = ctx.data_mut(|d| d.remove_temp::<Vec<Toast>>(queue_id()));
        for toast in queued.unwrap_or_default() {
            self.push(toast);
        }
        if self.up.is_empty() {
            return Vec::new();
        }
        let now = ctx.input(|i| i.time);
        let mut clicked = Vec::new();
        let mut closed = Vec::new();
        let area = egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(corner - vec2(GAP, GAP))
            .interactable(true);
        area.show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = GAP;
            // Top down, the area's bottom at the corner: the newest at the bottom. A
            // bottom-up layout would stretch the cards to the window's height.
            for (toast, until) in &mut self.up {
                let (r, action, close) = card(ui, toast);
                if let Some(i) = action {
                    clicked.push((toast.key, i));
                }
                if close {
                    closed.push(toast.key);
                }
                if !toast.intent.lasts() && (r.contains_pointer() || until.is_none()) {
                    *until = Some(now + SECONDS);
                }
            }
        });
        self.up.retain(|(t, until)| {
            !clicked.iter().any(|(k, _)| *k == t.key)
                && !closed.contains(&t.key)
                && until.is_none_or(|u| u > now)
        });
        if let Some(next) = self.up.iter().filter_map(|(_, u)| *u).reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.0)));
        }
        clicked
    }
}

/// A toast's card: the accent bar and icon of its intent, its title and text, and its
/// buttons at the bottom right, its close button at the top right; which button was
/// clicked, and whether the close button was.
pub fn card(ui: &mut Ui, toast: &Toast) -> (egui::Response, Option<usize>, bool) {
    let color = toast.intent.color(ui.visuals());
    let (mut action, mut close) = (None, false);
    let frame = Frame::popup(ui.style()).inner_margin(Margin {
        left: BAR as i8 + 12,
        right: 12,
        top: 10,
        bottom: 10,
    });
    let inner = frame.show(ui, |ui| {
        ui.set_width(WIDTH);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let line = ui.text_style_height(&egui::TextStyle::Body);
            let (rect, _) = ui.allocate_exact_size(vec2(16.0, line.max(16.0)), Sense::hover());
            let rect = Rect::from_center_size(rect.center(), vec2(16.0, 16.0));
            paint_icon(ui, rect, toast.intent, color);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.strong(&toast.title);
                    let right = egui::Layout::right_to_left(egui::Align::Center);
                    ui.with_layout(right, |ui| {
                        close = crate::ui::panels::cross_button(ui, tr!("CLOSE")).clicked();
                    });
                });
                if !toast.text.is_empty() {
                    ui.add(egui::Label::new(&toast.text).wrap());
                }
                if !toast.actions.is_empty() {
                    ui.add_space(4.0);
                    let row = vec2(ui.available_width(), ui.spacing().interact_size.y);
                    let right = egui::Layout::right_to_left(egui::Align::Center);
                    ui.allocate_ui_with_layout(row, right, |ui| {
                        for (i, a) in toast.actions.iter().enumerate().rev() {
                            if ui.button(a).clicked() {
                                action = Some(i);
                            }
                        }
                    });
                }
            });
        });
    });
    let r = inner.response.rect;
    let radius = ui.visuals().menu_corner_radius;
    let round = egui::CornerRadius {
        nw: radius.nw,
        sw: radius.sw,
        ne: 0,
        se: 0,
    };
    let bar = Rect::from_min_max(r.min, egui::pos2(r.left() + BAR, r.bottom()));
    ui.painter().rect_filled(bar, round, color);
    (inner.response.interact(Sense::hover()), action, close)
}

/// The icon of `intent` in `rect`, in `color`.
fn paint_icon(ui: &Ui, rect: Rect, intent: Intent, color: Color32) {
    let tone = match intent {
        Intent::Info => Tone::Info,
        Intent::Warning => Tone::Warning,
        Intent::Error => Tone::Error,
        Intent::Success => {
            let (c, s) = (rect.center(), rect.width());
            let p = ui.painter();
            p.circle_filled(c, s / 2.0, color);
            let ink = Stroke::new(s * 0.12, ui.visuals().panel_fill);
            let points = vec![
                c + vec2(-s * 0.24, 0.0),
                c + vec2(-s * 0.06, s * 0.18),
                c + vec2(s * 0.25, -s * 0.16),
            ];
            p.line(points, ink);
            return;
        }
    };
    callout::paint_icon(ui.painter(), rect, tone, color, ui.visuals());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(key: &'static str) -> Toast {
        Toast {
            key,
            intent: Intent::Info,
            title: "Title".into(),
            text: "Text".into(),
            actions: vec!["Review".into()],
        }
    }

    /// The texts drawn, with where.
    type Texts = Vec<(String, Rect)>;

    /// One frame at `time`, the pointer at `pointer` (pressed and released if `click`).
    fn frame(
        ctx: &egui::Context,
        toasts: &mut Toasts,
        time: f64,
        pointer: Pos2,
        click: bool,
    ) -> (Vec<(&'static str, usize)>, Texts) {
        let mut events = vec![egui::Event::PointerMoved(pointer)];
        if click {
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut clicked = Vec::new();
        let mut out = ctx.run_ui(input, |ui| {
            clicked = toasts.show(ui.ctx(), egui::pos2(800.0, 580.0));
        });
        out.textures_delta.clear();
        let texts = (out.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    Rect::from_min_size(t.pos, t.galley.size()),
                )),
                _ => None,
            })
            .collect();
        (clicked, texts)
    }

    #[test]
    fn a_toast_goes_away_unless_hovered_and_on_its_button() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push(toast("a"));
        let away = egui::pos2(10.0, 10.0);
        // An area is laid out on its first frame, drawn from the next one.
        frame(&ctx, &mut toasts, 0.0, away, false);
        let (_, texts) = frame(&ctx, &mut toasts, 0.0, away, false);
        let title = texts.iter().find(|(t, _)| t == "Title").unwrap().1;
        // Bottom right, above the corner given, as tall as what it holds.
        assert!(title.left() > 400.0 && title.bottom() < 580.0, "{title:?}");
        assert!(title.top() > 450.0, "{title:?}");
        frame(&ctx, &mut toasts, SECONDS - 1.0, away, false);
        assert!(toasts.is_up("a"));
        // The pointer over it keeps it up.
        frame(&ctx, &mut toasts, SECONDS + 5.0, title.center(), false);
        frame(&ctx, &mut toasts, 2.0 * SECONDS + 4.0, away, false);
        assert!(toasts.is_up("a"));
        frame(&ctx, &mut toasts, 2.0 * SECONDS + 6.0, away, false);
        assert!(!toasts.is_up("a"));
        // Its button: reported, and the toast gone.
        toasts.push(toast("b"));
        let (_, texts) = frame(&ctx, &mut toasts, 30.0, away, false);
        let review = texts.iter().find(|(t, _)| t == "Review").unwrap().1;
        frame(&ctx, &mut toasts, 30.1, review.center(), false);
        let (clicked, _) = frame(&ctx, &mut toasts, 30.2, review.center(), true);
        assert_eq!(clicked, [("b", 0)]);
        assert!(!toasts.is_up("b"));
    }

    #[test]
    fn toasts_of_each_intent_and_from_the_context_are_shown() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        let away = egui::pos2(10.0, 10.0);
        let (clicked, texts) = frame(&ctx, &mut toasts, 0.0, away, false);
        assert!(
            clicked.is_empty() && texts.is_empty(),
            "none up: nothing drawn"
        );
        let keys = ["info", "success", "warning", "error"];
        let intents = [
            Intent::Info,
            Intent::Success,
            Intent::Warning,
            Intent::Error,
        ];
        for (key, intent) in keys.into_iter().zip(intents) {
            let toast = Toast::new(key, intent, key, "").action("OK");
            push(&ctx, toast);
        }
        assert_eq!(queued(&ctx).len(), 4);
        frame(&ctx, &mut toasts, 0.0, away, false);
        assert!(queued(&ctx).is_empty());
        let (_, texts) = frame(&ctx, &mut toasts, 0.1, away, false);
        for key in keys {
            assert!(toasts.is_up(key));
            assert!(texts.iter().any(|(t, _)| t == key), "{key}");
        }
        assert_eq!(texts.iter().filter(|(t, _)| t == "OK").count(), 4);
        let visuals = egui::Visuals::dark();
        let colors: std::collections::HashSet<_> = intents.map(|i| i.color(&visuals)).into();
        assert_eq!(colors.len(), 4, "one color each");
    }

    /// Where the close button of the toast whose title is at `title` is.
    fn cross(title: Rect) -> Pos2 {
        // The text column is the card's width less the icon's (16) and the gap (10).
        egui::pos2(title.left() + WIDTH - 26.0 - 6.0, title.center().y)
    }

    #[test]
    fn any_toast_closes_with_its_close_button() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push(toast("a"));
        let away = egui::pos2(10.0, 10.0);
        frame(&ctx, &mut toasts, 0.0, away, false);
        let (_, texts) = frame(&ctx, &mut toasts, 0.1, away, false);
        let title = texts.iter().find(|(t, _)| t == "Title").unwrap().1;
        frame(&ctx, &mut toasts, 0.2, cross(title), false);
        let (clicked, _) = frame(&ctx, &mut toasts, 0.3, cross(title), true);
        assert!(clicked.is_empty(), "closing is not a call to action");
        assert!(!toasts.is_up("a"));
    }

    #[test]
    fn an_error_toast_stays_until_closed() {
        assert!(Intent::Error.lasts());
        assert!(
            ![Intent::Info, Intent::Success, Intent::Warning]
                .iter()
                .any(|i| i.lasts())
        );
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.push(Toast::new("a", Intent::Error, "Title", "Text"));
        let away = egui::pos2(10.0, 10.0);
        frame(&ctx, &mut toasts, 0.0, away, false);
        frame(&ctx, &mut toasts, 0.1, away, false);
        frame(&ctx, &mut toasts, 10.0 * SECONDS, away, false);
        let (_, texts) = frame(&ctx, &mut toasts, 10.0 * SECONDS, away, false);
        assert!(toasts.is_up("a"));
        let title = texts.iter().find(|(t, _)| t == "Title").unwrap().1;
        frame(&ctx, &mut toasts, 10.0 * SECONDS + 0.1, cross(title), false);
        frame(&ctx, &mut toasts, 10.0 * SECONDS + 0.2, cross(title), true);
        assert!(!toasts.is_up("a"));
    }

    #[test]
    fn a_toast_pushed_again_replaces_the_one_up() {
        let mut toasts = Toasts::default();
        toasts.push(toast("a"));
        toasts.push(toast("a"));
        assert_eq!(toasts.up.len(), 1);
    }
}
