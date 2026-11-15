//! Rulers and the guides dragged out of them.

use super::*;

/// The rulers' bands: along the top of the canvas, then along its left edge, leaving
/// the corner where they meet out.
pub(super) fn ruler_bands(full: Rect, rect: Rect) -> (Rect, Rect) {
    (
        Rect::from_min_max(
            pos2(rect.left(), full.top()),
            pos2(full.right(), rect.top()),
        ),
        Rect::from_min_max(
            pos2(full.left(), rect.top()),
            pos2(rect.left(), full.bottom()),
        ),
    )
}

/// A drag out of a ruler adds a guide: a horizontal one from the top ruler, a vertical
/// one from the left ruler. Hidden guides are shown again. Returns true if one was added.
pub(super) fn ruler_input(
    ui: &egui::Ui,
    doc: &mut Document,
    state: &mut CanvasState,
    full: Rect,
    rect: Rect,
) -> bool {
    let mut added = false;
    let (top, left) = ruler_bands(full, rect);
    for (band, vertical) in [(top, false), (left, true)] {
        let r = ui.interact(band, ui.scope_id().with(("ruler", vertical)), Sense::drag());
        if r.drag_started_by(PointerButton::Primary)
            && let Some(p) = r.interact_pointer_pos()
        {
            let at = state.camera.to_mm(rect, p)[if vertical { 0 } else { 1 }];
            doc.guides.push(Guide { vertical, at });
            state.guide_drag = Some(doc.guides.len() - 1);
            state.show_guides = true;
            added = true;
        }
    }
    added
}

/// Millimeters between two numbered marks on a ruler at `zoom`, so that they are at least
/// `RULER_LABEL_SPACING` apart on screen, and how many parts the marks between them cut
/// that into.
pub(super) fn ruler_step(zoom: f32) -> (f32, i64) {
    let mut base = 1.0;
    loop {
        for (m, parts) in [(1.0, 10), (2.0, 4), (5.0, 5)] {
            let step = base * m;
            if step * zoom >= RULER_LABEL_SPACING || step >= 10_000.0 {
                return (step, parts);
            }
        }
        base *= 10.0;
    }
}

/// The rulers along the canvas' top and left edges, in mm from the page's top-left
/// corner, the page's extent lighter and the pointer's position marked.
pub(super) fn paint_rulers(ui: &egui::Ui, state: &CanvasState, page: P2, full: Rect, rect: Rect) {
    let painter = ui.painter_at(full);
    let visuals = ui.visuals();
    let ink = visuals.weak_text_color();
    let text = visuals.text_color();
    let border = visuals.widgets.noninteractive.bg_stroke;
    let cam = state.camera;
    let (top, left) = ruler_bands(full, rect);
    for band in [top, left, Rect::from_min_max(full.min, rect.min)] {
        painter.rect_filled(band, 0.0, visuals.panel_fill);
    }
    let (p0, p1) = (cam.to_screen(rect, [0.0, 0.0]), cam.to_screen(rect, page));
    let (step, parts) = ruler_step(cam.zoom);
    let minor = step / parts as f32;
    let font = egui::FontId::proportional(10.0);
    for (axis, band) in [(0, top), (1, left)] {
        let clip = painter.with_clip_rect(band);
        let along = |a: f32, b: f32| {
            if axis == 0 {
                Rect::from_min_max(pos2(a, band.top()), pos2(b, band.bottom()))
            } else {
                Rect::from_min_max(pos2(band.left(), a), pos2(band.right(), b))
            }
        };
        clip.rect_filled(along(p0[axis], p1[axis]), 0.0, visuals.extreme_bg_color);
        let screen = |mm: f32| rect.min[axis] + cam.pan[axis] + mm * cam.zoom;
        let to_mm = |s: f32| (s - rect.min[axis] - cam.pan[axis]) / cam.zoom;
        let (from, to) = (band.min[axis], band.max[axis]);
        let (k0, k1) = (
            (to_mm(from) / minor).floor() as i64,
            (to_mm(to) / minor).ceil() as i64,
        );
        for k in k0..=k1 {
            let s = screen(k as f32 * minor);
            let len = if k % parts == 0 {
                RULER * 0.6
            } else if parts % 2 == 0 && k % (parts / 2) == 0 {
                RULER * 0.4
            } else {
                RULER * 0.25
            };
            // Marks stand on the canvas' edge.
            let mark = if axis == 0 {
                [pos2(s, band.bottom()), pos2(s, band.bottom() - len)]
            } else {
                [pos2(band.right(), s), pos2(band.right() - len, s)]
            };
            clip.line_segment(mark, Stroke::new(1.0, ink));
            if k % parts == 0 {
                let label = format!("{}", ((k / parts) as f32 * step).round() as i64);
                if axis == 0 {
                    clip.text(
                        pos2(s + 3.0, band.top() + 1.0),
                        egui::Align2::LEFT_TOP,
                        label,
                        font.clone(),
                        text,
                    );
                } else {
                    // Read from the bottom up, just above the mark.
                    let galley = clip.layout_no_wrap(label, font.clone(), text);
                    let at = pos2(band.left() + 1.0, s - 3.0);
                    clip.add(
                        egui::epaint::TextShape::new(at, galley, text)
                            .with_angle(-std::f32::consts::FRAC_PI_2),
                    );
                }
            }
        }
        if let Some(c) = state.cursor_mm {
            let s = screen(c[axis]);
            let line = if axis == 0 {
                [pos2(s, band.top()), pos2(s, band.bottom())]
            } else {
                [pos2(band.left(), s), pos2(band.right(), s)]
            };
            clip.line_segment(line, Stroke::new(1.0, ACCENT));
        }
    }
    painter.line_segment(
        [
            pos2(full.left(), rect.top()),
            pos2(full.right(), rect.top()),
        ],
        border,
    );
    painter.line_segment(
        [
            pos2(rect.left(), full.top()),
            pos2(rect.left(), full.bottom()),
        ],
        border,
    );
}

/// The guide drawn within `GUIDE_GRAB` of screen point `p` (the nearest one), if `p` is
/// on the canvas.
pub(super) fn guide_at(doc: &Document, cam: Camera, rect: Rect, p: Pos2) -> Option<usize> {
    if !rect.contains(p) {
        return None;
    }
    let m = cam.to_mm(rect, p);
    doc.guides
        .iter()
        .enumerate()
        .map(|(i, g)| {
            (
                i,
                (m[if g.vertical { 0 } else { 1 }] - g.at).abs() * cam.zoom,
            )
        })
        .filter(|&(_, d)| d <= GUIDE_GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The cursor over a guide: it moves across its length.
pub(super) fn guide_cursor(vertical: bool) -> CursorIcon {
    if vertical {
        CursorIcon::ResizeHorizontal
    } else {
        CursorIcon::ResizeVertical
    }
}

/// A guide's line across the canvas, on screen.
pub(super) fn guide_line(cam: Camera, rect: Rect, g: &Guide) -> [Pos2; 2] {
    if g.vertical {
        let x = cam.to_screen(rect, [g.at, 0.0]).x;
        [pos2(x, rect.top()), pos2(x, rect.bottom())]
    } else {
        let y = cam.to_screen(rect, [0.0, g.at]).y;
        [pos2(rect.left(), y), pos2(rect.right(), y)]
    }
}

/// What a dragged guide snaps to along `axis` (0: x, 1: y): the page's and the outline's
/// lines in `edges`, and the edges and middle of each visible layer.
pub(super) fn guide_targets(doc: &Document, edges: &Guides, axis: usize) -> Vec<f32> {
    let mut targets = if axis == 0 {
        edges.x.clone()
    } else {
        edges.y.clone()
    };
    for leaf in doc.leaves().into_iter().filter(|l| l.visible) {
        if let Some((lo, hi)) = layers::layers_bounds(doc, &[leaf.layer.id]) {
            targets.extend([lo[axis], (lo[axis] + hi[axis]) / 2.0, hi[axis]]);
        }
    }
    targets
}

/// The window where the position of the guide double-clicked on the canvas is typed, or
/// the guide deleted. Returns true if it changed the guides.
pub(super) fn guide_dialog(
    ctx: &egui::Context,
    doc: &mut Document,
    state: &mut CanvasState,
) -> bool {
    use crate::app::{at_left_edge, dialog, dialog_button_row};
    let Some(mut edit) = state.guide_edit else {
        return false;
    };
    let Some(&guide) = doc.guides.get(edit.index) else {
        state.guide_edit = None;
        return false;
    };
    let mut entered = false;
    let response = dialog(ctx, "guide_position").show(ctx, |ui| {
        ui.heading(if guide.vertical {
            tr!("VERTICAL_GUIDE")
        } else {
            tr!("HORIZONTAL_GUIDE")
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(if guide.vertical {
                tr!("GUIDE_FROM_LEFT")
            } else {
                tr!("GUIDE_FROM_TOP")
            });
            let r = ui.add(
                egui::DragValue::new(&mut edit.at)
                    .speed(0.1)
                    .max_decimals(2)
                    .suffix(" mm"),
            );
            if std::mem::take(&mut edit.focus) {
                r.request_focus();
            }
            entered = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        });
        ui.add_space(8.0);
        dialog_button_row(ui, |ui| {
            let ok = ui.button(tr!("OK")).clicked();
            let cancel = ui.button(tr!("CANCEL")).clicked();
            let delete = at_left_edge(ui, |ui| ui.button(tr!("DELETE_GUIDE")).clicked());
            (ok, cancel, delete)
        })
    });
    let (ok, cancel, delete) = response.inner;
    if delete {
        doc.guides.remove(edit.index);
        state.guide_edit = None;
        true
    } else if ok || entered {
        let changed = doc.guides[edit.index].at != edit.at;
        doc.guides[edit.index].at = edit.at;
        state.guide_edit = None;
        changed
    } else {
        state.guide_edit = (!cancel && !response.should_close()).then_some(edit);
        false
    }
}

/// Removes all the guides. Returns true if there were any.
pub fn clear_guides(doc: &mut Document) -> bool {
    !std::mem::take(&mut doc.guides).is_empty()
}
