//! Moving the view: fitting the page, the comparison's divider, zooming and panning.

use super::*;

/// Keeps the view centered when the window resizes, and fits the page on demand.
/// `full` is the canvas with its rulers, `rect` without.
pub(super) fn fit(state: &mut CanvasState, page: P2, full: Rect, rect: Rect) {
    if state.fit_pending || !state.rect.is_positive() {
        state.camera.fit(rect, page);
        state.fit_pending = false;
    } else if full == state.full_rect && rect != state.rect {
        // The rulers were shown or hidden: the page stays where it is on screen.
        state.camera.pan -= rect.min - state.rect.min;
    } else if state.rect.size() != rect.size() {
        state.camera.pan += (rect.size() - state.rect.size()) / 2.0;
    }
    state.rect = rect;
    state.full_rect = full;
}

/// The comparison's divider, dragged across the canvas. Returns true while the pointer
/// is on it.
pub(super) fn divider(ui: &egui::Ui, state: &mut CanvasState, rect: Rect) -> bool {
    if state.compare.is_none() {
        state.compare_cut_preview = None;
        state.compare_safe_area = None;
        return false;
    }
    let x = rect.left() + rect.width() * state.compare_split;
    // Along the line, and a dead area around the handle wider than it: there, the
    // divider takes the press (click or drag), never what is drawn under it.
    let line = Rect::from_x_y_ranges(x - DIVIDER_GRAB..=x + DIVIDER_GRAB, rect.y_range());
    let handle = Rect::from_center_size(
        pos2(x, rect.center().y),
        Vec2::splat(2.0 * HANDLE_DEAD_ZONE),
    );
    let id = ui.scope_id().with("compare_divider");
    let r = ui.interact(line, id, Sense::click_and_drag())
        | ui.interact(handle, id.with("handle"), Sense::click_and_drag());
    if r.dragged()
        && let Some(p) = r.interact_pointer_pos()
    {
        state.compare_split = ((p.x - rect.left()) / rect.width()).clamp(0.02, 0.98);
    }
    let on = r.hovered() || r.dragged();
    if on {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    on
}

/// Wheel with the pointer at `pos`: a mouse wheel or Ctrl+scroll zooms, a trackpad
/// scroll pans, a pinch zooms.
pub(super) fn wheel(ui: &egui::Ui, cam: &mut Camera, rect: Rect, pos: Pos2) {
    let events = ui.input(|i| i.events.clone());
    for ev in events {
        match ev {
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                source,
                ..
            } => {
                let is_wheel = match source {
                    MouseWheelSource::Wheel => true,
                    MouseWheelSource::Unknown => unit != MouseWheelUnit::Point,
                    _ => false,
                };
                if modifiers.command || modifiers.ctrl || is_wheel {
                    let steps = match unit {
                        MouseWheelUnit::Point => delta.y / 40.0,
                        MouseWheelUnit::Line => delta.y,
                        MouseWheelUnit::Page => delta.y * 5.0,
                    };
                    cam.zoom_at(rect, pos, 1.15f32.powf(steps));
                } else {
                    cam.pan += delta;
                }
            }
            Event::Zoom(factor) => cam.zoom_at(rect, pos, factor),
            _ => {}
        }
    }
}

/// Panning: left drag on empty canvas, middle or right drag, or Space (`space`) + left
/// drag. Returns true while the view is panned.
pub(super) fn pan(
    ui: &egui::Ui,
    state: &mut CanvasState,
    response: &egui::Response,
    space: bool,
    hover: Option<Pos2>,
) -> bool {
    if !ui.input(|i| i.pointer.primary_down()) {
        state.panning = false;
    }
    let panning = response.dragged_by(PointerButton::Middle)
        || response.dragged_by(PointerButton::Secondary)
        || (response.dragged_by(PointerButton::Primary) && (space || state.panning));
    if panning {
        state.camera.pan += response.drag_delta();
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if space && hover.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    panning
}
