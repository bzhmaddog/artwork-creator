//! A dial to set an angle by turning it.

use egui::{Sense, Ui, vec2};

/// Its width and height, in points.
const SIDE: f32 = crate::ui::tokens::DIAL;

/// Where a dial's 0° is, and which way its angle turns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    /// 0° to the right, counterclockwise: a direction (a gradient's, a light's, a nib's).
    FromRight,
    /// 0° at the top, clockwise: a rotation, as the rotation handle above a layer turns.
    FromTop,
}

/// A dial for an angle in degrees, counted as `turn` says: a ring with a dot at the
/// angle. Dragging or clicking on it turns it there, in whole degrees, or in 15° steps
/// with Shift; a double click puts it back to `default`. The angle it sets is between
/// -180 and 180.
pub fn angle_dial(ui: &mut Ui, deg: &mut f32, turn: Turn, default: f32) -> egui::Response {
    let (rect, mut r) = ui.allocate_exact_size(vec2(SIDE, SIDE), Sense::click_and_drag());
    let c = rect.center();
    if r.double_clicked() {
        *deg = default;
        r.mark_changed();
    } else if (r.dragged() || r.clicked())
        && let Some(p) = r.interact_pointer_pos()
    {
        let v = p - c;
        if v.length() > 1.0 {
            let step = if ui.input(|i| i.modifiers.shift) {
                15.0
            } else {
                1.0
            };
            // Up on screen is counterclockwise.
            let a = match turn {
                Turn::FromRight => (-v.y).atan2(v.x),
                Turn::FromTop => v.x.atan2(-v.y),
            }
            .to_degrees();
            let a = (a / step).round() * step;
            *deg = if a > 180.0 { a - 360.0 } else { a };
            r.mark_changed();
        }
    }
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&r);
        let radius = SIDE / 2.0 - 1.5;
        let a = deg.to_radians();
        let dir = match turn {
            Turn::FromRight => vec2(a.cos(), -a.sin()),
            Turn::FromTop => vec2(a.sin(), -a.cos()),
        };
        let p = ui.painter();
        let ring = egui::Stroke::new(1.0, visuals.fg_stroke.color);
        p.circle(c, radius, visuals.bg_fill, ring);
        p.circle_filled(c + dir * (radius - 3.0), 2.5, visuals.fg_stroke.color);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, Rect};

    /// A click at `offset` from the middle of a dial counting as `turn`, from 0°: the
    /// angle it sets.
    fn click(turn: Turn, offset: egui::Vec2, shift: bool) -> f32 {
        let ctx = egui::Context::default();
        let mut deg = 0.0;
        let modifiers = egui::Modifiers {
            shift,
            ..Default::default()
        };
        let frame = |mut events: Vec<Event>, deg: &mut f32| {
            events.insert(0, Event::ModifiersChanged(modifiers));
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0))),
                events,
                ..Default::default()
            };
            let mut middle = Pos2::ZERO;
            ctx.run_ui(input, |ui| {
                middle = angle_dial(ui, deg, turn, 0.0).rect.center();
            })
            .textures_delta
            .clear();
            middle
        };
        let middle = frame(vec![], &mut deg);
        let at = middle + offset;
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(vec![Event::PointerMoved(at), button(true)], &mut deg);
        frame(vec![button(false)], &mut deg);
        deg
    }

    #[test]
    fn turns_toward_the_click_either_way() {
        use Turn::{FromRight, FromTop};
        // Above the middle: a quarter turn from the right, 0° from the top.
        assert_eq!(click(FromRight, vec2(0.0, -8.0), false), 90.0);
        assert_eq!(click(FromTop, vec2(0.0, -8.0), false), 0.0);
        // Right of it: 0° from the right, a quarter turn clockwise from the top.
        assert_eq!(click(FromRight, vec2(8.0, 0.0), false), 0.0);
        assert_eq!(click(FromTop, vec2(8.0, 0.0), false), 90.0);
        // Below and left.
        assert_eq!(click(FromRight, vec2(-6.0, 6.0), false), -135.0);
        assert_eq!(click(FromTop, vec2(-6.0, 6.0), false), -135.0);
        // Shift: 15° steps (atan(3/8) is about 20.6°).
        assert_eq!(click(FromRight, vec2(8.0, -3.0), false), 21.0);
        assert_eq!(click(FromRight, vec2(8.0, -3.0), true), 15.0);
    }
}
