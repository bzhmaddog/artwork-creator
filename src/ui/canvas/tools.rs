//! The canvas tool bar, the eyedropper and the paint brushes.

use super::*;

/// The canvas' tools, with their commands: selecting and moving, then painting.
pub const TOOLS: [(
    crate::ui::shortcuts::Command,
    Option<crate::document::paint::Tool>,
); 4] = [
    (crate::ui::shortcuts::Command::SelectTool, None),
    (
        crate::ui::shortcuts::Command::PencilTool,
        Some(crate::document::paint::Tool::Pencil),
    ),
    (
        crate::ui::shortcuts::Command::BrushTool,
        Some(crate::document::paint::Tool::Brush),
    ),
    (
        crate::ui::shortcuts::Command::EraserTool,
        Some(crate::document::paint::Tool::Eraser),
    ),
];

/// The tool bar at the canvas' top left: Select and the eyedropper; while a canvas
/// is selected (`paintable`), the paint tools between them, the button clearing the
/// shape's paint (`can_clear` says whether it has any), then, for a paint tool, its
/// color (not the eraser's) and width. Returns true if the clear button was clicked.
pub(super) fn tool_bar(
    ui: &egui::Ui,
    state: &mut CanvasState,
    rect: Rect,
    paintable: bool,
    can_clear: bool,
) -> bool {
    use crate::ui::layers::Tool as Icon;
    let mut clear = false;
    egui::Area::new(ui.scope_id().with("tool_bar"))
        .fixed_pos(rect.min + vec2(8.0, 8.0))
        .order(egui::Order::Middle)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let shown = TOOLS.iter().filter(|(_, t)| paintable || t.is_none());
                        for &(command, tool) in shown {
                            let icon = match tool {
                                None => Icon::Select,
                                Some(crate::document::paint::Tool::Pencil) => Icon::Pencil,
                                Some(crate::document::paint::Tool::Brush) => {
                                    Icon::Brush(state.brushes.brush)
                                }
                                Some(crate::document::paint::Tool::Eraser) => Icon::Eraser,
                            };
                            let mut tip = crate::ui::shortcuts::with_key(
                                command.label(),
                                state.keys.get(&command),
                            );
                            if tool == Some(crate::document::paint::Tool::Brush) {
                                tip = format!("{tip}\n{}", crate::i18n::tr!("BRUSH_KIND_HINT"));
                            }
                            // The tool in use stands out.
                            let on = !state.eyedropper
                                && (state.brushes.tool == tool || !paintable && tool.is_none());
                            let button = tool_toggle(ui, icon, on, &tip);
                            if button.clicked() {
                                state.brushes.tool = tool;
                                state.eyedropper = false;
                                state.painting.clear();
                            }
                            if tool == Some(crate::document::paint::Tool::Brush) {
                                brush_menu(ui, &button, &mut state.brushes);
                            }
                        }
                        let c = crate::ui::shortcuts::Command::EyedropperTool;
                        let tip = crate::ui::shortcuts::with_key(c.label(), state.keys.get(&c));
                        if tool_toggle(ui, Icon::Eyedropper, state.eyedropper, &tip).clicked() {
                            state.eyedropper = true;
                            state.painting.clear();
                        }
                        if !paintable {
                            return;
                        }
                        // Emptying the shape of its paint, with any tool.
                        ui.add_space(4.0);
                        let c = crate::ui::shortcuts::Command::ClearPaint;
                        let tip = crate::ui::shortcuts::with_key(c.label(), state.keys.get(&c));
                        clear =
                            crate::ui::layers::tool_button(ui, Icon::ClearPaint, can_clear, &tip);
                        let Some(tool) = state.brushes.tool.filter(|_| !state.eyedropper) else {
                            return;
                        };
                        ui.separator();
                        if tool != crate::document::paint::Tool::Eraser {
                            crate::ui::colors::button(ui, &mut state.brushes.color, true, None)
                                .on_hover_text(crate::i18n::tr!("PAINT_COLOR_HINT"));
                        }
                        ui.add(
                            egui::DragValue::new(state.brushes.width_mut(tool))
                                .range(0.05..=100.0)
                                .speed(0.05)
                                .max_decimals(2),
                        )
                        .on_hover_text(crate::i18n::tr!("PAINT_WIDTH_HINT"));
                        crate::ui::layers::unit(ui, "mm");
                        if tool == crate::document::paint::Tool::Brush {
                            brush_options(ui, &mut state.brushes);
                        }
                    });
                });
        });
    clear
}

/// A tool bar button with `icon`, standing out when `on`, the tool in hand.
pub(super) fn tool_toggle(
    ui: &mut egui::Ui,
    icon: crate::ui::layers::Tool,
    on: bool,
    tip: &str,
) -> egui::Response {
    if on {
        let at = ui.cursor().min;
        ui.painter().rect_filled(
            Rect::from_min_size(at, vec2(24.0, 24.0)),
            3.0,
            ui.visuals().selection.bg_fill,
        );
    }
    crate::ui::layers::tool_icon(ui, icon, true, tip)
}

/// With the eyedropper in hand: the artwork's color under the pointer is sampled when it
/// moves, and a click takes it: it becomes the paint's color and its hex code goes to the
/// clipboard. A Ctrl+click also adds it to the picked ones, first among the colors in use.
pub(super) fn eyedropper_input(
    ui: &egui::Ui,
    doc: &Document,
    state: &mut CanvasState,
    cam: Camera,
    rect: Rect,
    response: &egui::Response,
) {
    let Some(p) = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|_| ui.rect_contains_pointer(rect))
    else {
        state.dropper_sample = None;
        return;
    };
    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    let at = cam.to_mm(rect, p);
    if state.dropper_sample.is_none_or(|(q, _)| q != at) {
        // One pixel at the zoom's resolution: the color as shown.
        let px_per_mm = (cam.zoom * ui.ctx().pixels_per_point()).max(1e-3);
        let color = render::sample(
            &render::scene_nodes(&doc.layers, true),
            at,
            px_per_mm,
            doc.page_mm(),
            doc.background,
            doc.background_gradient.as_ref(),
            doc.background_pattern.as_ref(),
        );
        state.dropper_sample = Some((at, color));
    }
    if response.clicked_by(PointerButton::Primary)
        && let Some((_, Some(c))) = state.dropper_sample
    {
        state.brushes.color = c;
        if ui.input(|i| i.modifiers.command) {
            state.picked.retain(|&p| p != c);
            state.picked.insert(0, c);
        }
        let [r, g, b, _] = c;
        ui.ctx().copy_text(format!("#{r:02X}{g:02X}{b:02X}"));
    }
}

/// Next to the pointer, with the eyedropper in hand: a swatch of the color under it and
/// its code.
pub(super) fn paint_dropper_sample(
    ui: &egui::Ui,
    state: &CanvasState,
    rect: Rect,
    hover: Option<Pos2>,
) {
    let (Some(p), Some((_, Some(c)))) = (hover, state.dropper_sample) else {
        return;
    };
    let painter = ui.painter_at(rect);
    let [r, g, b, a] = c;
    let side = 28.0;
    let swatch = Rect::from_min_size(p + vec2(14.0, 14.0), vec2(side, side));
    let frame = Stroke::new(1.0, ui.visuals().widgets.noninteractive.fg_stroke.color);
    painter.rect_filled(swatch.expand(2.0), 4.0, ui.visuals().extreme_bg_color);
    painter.rect_filled(swatch, 3.0, Color32::from_rgba_unmultiplied(r, g, b, a));
    painter.rect_stroke(swatch, 3.0, frame, egui::StrokeKind::Outside);
    let text = format!("#{r:02X}{g:02X}{b:02X}");
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = painter.layout_no_wrap(text, font, ui.visuals().text_color());
    let at = pos2(
        swatch.right() + 6.0,
        swatch.center().y - galley.size().y / 2.0,
    );
    painter.rect_filled(
        Rect::from_min_size(at, galley.size()).expand(3.0),
        3.0,
        ui.visuals().extreme_bg_color,
    );
    painter.galley(at, galley, Color32::PLACEHOLDER);
}

/// The brush button's corner mark, saying it has more kinds, and the menu of them
/// below it, opened by a click (which also takes the brush): each kind's icon and name.
pub(super) fn brush_menu(
    ui: &egui::Ui,
    button: &egui::Response,
    brushes: &mut crate::document::paint::Brushes,
) {
    use crate::i18n::translate;
    use crate::ui::layers::{Tool as Icon, icon_entry};
    let r = button.rect;
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    ui.painter().add(Shape::convex_polygon(
        vec![
            r.right_bottom() - vec2(1.0, 1.0),
            r.right_bottom() - vec2(1.0, 5.0),
            r.right_bottom() - vec2(5.0, 1.0),
        ],
        color,
        Stroke::NONE,
    ));
    let labels = crate::document::paint::BrushKind::ALL.map(|k| translate(k.name_key()));
    egui::Popup::menu(button).show(|ui| {
        for kind in crate::document::paint::BrushKind::ALL {
            let on = brushes.brush == kind;
            let name = translate(kind.name_key());
            if icon_entry(ui, Icon::Brush(kind), name, &labels, on).clicked() {
                brushes.brush = kind;
                ui.close();
            }
        }
    });
}

/// The settings of the brush kinds that have some: the calligraphy nib's angle, the
/// spray's density.
pub(super) fn brush_options(ui: &mut egui::Ui, brushes: &mut crate::document::paint::Brushes) {
    use crate::document::paint::BrushKind;
    use crate::i18n::tr;
    match brushes.brush {
        BrushKind::Calligraphy => {
            let default = crate::document::paint::Brushes::default().nib_angle;
            let r = crate::ui::dial::angle_dial(
                ui,
                &mut brushes.nib_angle,
                crate::ui::dial::Turn::FromRight,
                default,
            )
            .on_hover_text(tr!("NIB_ANGLE_HINT"));
            // A faint line through its middle: the nib.
            let a = brushes.nib_angle.to_radians();
            let dir = vec2(a.cos(), -a.sin()) * (r.rect.width() / 2.0 - 1.5);
            let faint = ui.style().interact(&r).fg_stroke.color.gamma_multiply(0.4);
            let c = r.rect.center();
            ui.painter()
                .line_segment([c - dir, c + dir], Stroke::new(1.0, faint));
        }
        BrushKind::Spray => {
            let mut percent = brushes.spray_density * 100.0;
            if ui
                .add(egui::Slider::new(&mut percent, 1.0..=100.0).max_decimals(0))
                .on_hover_text(tr!("SPRAY_DENSITY_HINT"))
                .changed()
            {
                brushes.spray_density = percent / 100.0;
            }
            crate::ui::layers::unit(ui, "%");
        }
        _ => {}
    }
}

/// The selected shape can be painted in and has paint to clear.
pub fn has_paint(doc: &Document, selected: Option<u64>) -> bool {
    can_paint(doc, selected)
        && selected
            .and_then(|id| doc.layer(id))
            .and_then(|l| l.shape.as_deref())
            .is_some_and(|s| !s.strokes.is_empty())
}

/// Empties the selected shape of its paint, if it can be painted in. Returns true if
/// anything was cleared.
pub fn clear_paint(doc: &mut Document, selected: Option<u64>) -> bool {
    has_paint(doc, selected)
        && selected
            .and_then(|id| doc.layer_mut(id))
            .is_some_and(crate::document::shape::clear_paint)
}

/// The paint tool in hand: the one picked last, while a shape that can be painted in is
/// selected.
pub fn active_tool(
    doc: &Document,
    selected: Option<u64>,
    brushes: &crate::document::paint::Brushes,
) -> Option<crate::document::paint::Tool> {
    brushes.tool.filter(|_| can_paint(doc, selected))
}

/// The selected layer, if it is a shape that can be painted in: closed, visible and
/// unlocked.
pub fn can_paint(doc: &Document, selected: Option<u64>) -> bool {
    selected.is_some_and(|id| {
        // A template's shapes are lines only.
        crate::template::edit::element_of(doc, id).is_none()
            && doc.layer(id).is_some_and(crate::document::shape::paintable)
            && !doc.effectively_locked(id)
            && doc.effectively_visible(id)
    })
}

/// Painting with the pencil or the brush, or erasing, in the selected canvas: a
/// drag on the page paints a stroke, added when the button is released (what goes past
/// the shape's outline doesn't show); a click, a dot. The eraser cuts its strokes
/// as it goes. Returns true if the document changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_input(
    ui: &egui::Ui,
    doc: &mut Document,
    selected: Option<u64>,
    state: &mut CanvasState,
    cam: Camera,
    rect: Rect,
    response: &egui::Response,
    tool: crate::document::paint::Tool,
) -> bool {
    use crate::document::paint::Tool;
    let Some(id) = selected.filter(|_| can_paint(doc, selected)) else {
        return false;
    };
    // A pen's pressure comes with touch events.
    let force = ui.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    });
    if force.is_some() {
        state.pen_pressure = force;
    }
    let pressure = state.pen_pressure.unwrap_or(1.0);
    let mut changed = false;
    let at = |p: Pos2| {
        let m = cam.to_mm(rect, p);
        [m[0], m[1], pressure]
    };
    let eraser = state.brushes.width(Tool::Eraser) / 2.0;
    let erase = |doc: &mut Document, path: &[P2]| {
        doc.layer_mut(id)
            .is_some_and(|l| crate::document::shape::erase(l, path, eraser))
    };
    if response.drag_started_by(PointerButton::Primary)
        && let Some(p) = ui
            .input(|i| i.pointer.press_origin())
            .or(response.interact_pointer_pos())
    {
        state.painting = vec![at(p)];
    }
    let pointer = response.interact_pointer_pos();
    if response.dragged_by(PointerButton::Primary)
        && let Some(p) = pointer
        && !state.painting.is_empty()
    {
        let s = at(p);
        let last = *state.painting.last().unwrap_or(&s);
        if last[0] != s[0] || last[1] != s[1] {
            state.painting.push(s);
            if tool == Tool::Eraser {
                changed |= erase(doc, &[[last[0], last[1]], [s[0], s[1]]]);
            }
        }
        ui.ctx().request_repaint();
    }
    let done = response.drag_stopped_by(PointerButton::Primary)
        || (response.clicked_by(PointerButton::Primary) && state.painting.is_empty());
    if done {
        let mut samples = std::mem::take(&mut state.painting);
        if samples.is_empty()
            && let Some(p) = pointer
        {
            samples.push(at(p));
        }
        if !samples.is_empty() {
            changed |= match tool {
                Tool::Eraser => {
                    let path: Vec<P2> = samples.iter().map(|s| [s[0], s[1]]).collect();
                    erase(doc, &path)
                }
                _ => {
                    let pen = state.brushes.pen(tool, &samples);
                    doc.layer_mut(id)
                        .is_some_and(|l| crate::document::shape::add_stroke(l, &samples, &pen))
                }
            };
        }
        state.pen_pressure = None;
    }
    changed
}

/// The outlines of the visible closed shapes with neither fill nor stroke, faint and
/// dashed so that one added to paint in shows (not printed).
pub(super) fn canvas_frames(
    doc: &Document,
    painter: &egui::Painter,
    state: &CanvasState,
    rect: Rect,
) {
    let cam = state.camera;
    let frame = Stroke::new(1.0, Color32::from_gray(140));
    for leaf in doc.leaves() {
        let unseen = leaf
            .layer
            .shape
            .as_deref()
            .is_some_and(|s| s.kind.closed() && !s.fill && !(s.stroke && s.stroke_width > 0.0));
        if !leaf.visible || !unseen {
            continue;
        }
        let Some((outline, _)) = crate::document::shape::page_outline(leaf.layer) else {
            continue;
        };
        let mut pts: Vec<Pos2> = outline.iter().map(|&p| cam.to_screen(rect, p)).collect();
        pts.extend(pts.first().copied());
        painter.extend(Shape::dashed_line(&pts, frame, 4.0, 4.0));
    }
}

/// The stroke being painted, as it goes, and the tool's size around the pointer.
pub(super) fn paint_brush(
    ui: &egui::Ui,
    painter: &egui::Painter,
    doc: &Document,
    selected: Option<u64>,
    state: &mut CanvasState,
    rect: Rect,
) {
    use crate::document::paint::{BrushKind, Tool};
    let Some(tool) = active_tool(doc, selected, &state.brushes) else {
        return;
    };
    let cam = state.camera;
    let width = state.brushes.width(tool) * cam.zoom;
    if tool != Tool::Eraser && !state.painting.is_empty() {
        // Cut off at the canvas' box on screen, as the stroke will be.
        let clip = selected
            .and_then(|id| doc.layer(id))
            .and_then(crate::document::shape::page_outline)
            .map(|(o, _)| {
                Rect::from_points(
                    &o.iter()
                        .map(|&p| cam.to_screen(rect, p))
                        .collect::<Vec<_>>(),
                )
            });
        let painter = match clip {
            Some(c) => painter.with_clip_rect(c.intersect(painter.clip_rect())),
            None => painter.clone(),
        };
        let pen = state.brushes.pen(tool, &state.painting);
        let strokes = crate::document::paint::PaintStroke::paint(&state.painting, &pen);
        stroke_preview(
            ui.ctx(),
            &painter,
            &mut state.paint_preview,
            &strokes,
            cam,
            rect,
        );
    }
    // Only over the bare canvas, not the tool bar, a window or a popup over it.
    if let Some(p) = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|_| ui.rect_contains_pointer(rect))
    {
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        // The tool's reach, in black and white so it shows on anything: the calligraphy
        // nib as a line at its angle.
        if tool == Tool::Brush && state.brushes.brush == BrushKind::Calligraphy {
            let a = state.brushes.nib_angle.to_radians();
            let h = vec2(a.cos(), -a.sin()) * (width / 2.0).max(4.0);
            painter.line_segment([p - h, p + h], Stroke::new(3.0, Color32::WHITE));
            painter.line_segment([p - h, p + h], Stroke::new(1.0, Color32::BLACK));
        } else {
            let r = (width / 2.0).max(2.0);
            painter.circle_stroke(p, r + 1.0, Stroke::new(1.0, Color32::WHITE));
            painter.circle_stroke(p, r, Stroke::new(1.0, Color32::BLACK));
        }
    }
}

/// Draws `strokes` (page mm) as they will be filled, over the part of the screen they
/// cover within the painter's clip, through `texture`.
pub(super) fn stroke_preview(
    ctx: &egui::Context,
    painter: &egui::Painter,
    texture: &mut Option<egui::TextureHandle>,
    strokes: &[crate::document::paint::PaintStroke],
    cam: Camera,
    rect: Rect,
) {
    let Some((lo, hi)) = strokes
        .iter()
        .filter_map(crate::document::paint::PaintStroke::bounds)
        .reduce(|a, b| {
            (
                [a.0[0].min(b.0[0]), a.0[1].min(b.0[1])],
                [a.1[0].max(b.1[0]), a.1[1].max(b.1[1])],
            )
        })
    else {
        return;
    };
    let area = Rect::from_min_max(cam.to_screen(rect, lo), cam.to_screen(rect, hi))
        .expand(1.0)
        .intersect(painter.clip_rect());
    if !area.is_positive() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let size = [area.width(), area.height()].map(|v| (v * ppp).ceil().clamp(1.0, 8192.0) as u32);
    let Some(mut pixmap) = tiny_skia::Pixmap::new(size[0], size[1]) else {
        return;
    };
    // From page mm to the pixmap's pixels.
    let o = cam.to_screen(rect, [0.0; 2]);
    let k = cam.zoom * ppp;
    let ts = tiny_skia::Transform::from_scale(k, k)
        .post_translate((o.x - area.min.x) * ppp, (o.y - area.min.y) * ppp);
    for s in strokes {
        if let Some(path) = s.outline() {
            let ink = crate::document::gradient::Ink::Solid(s.color);
            pixmap.fill_path(&path, &ink.paint(), tiny_skia::FillRule::Winding, ts, None);
        }
    }
    let image = egui::ColorImage::from_rgba_premultiplied(
        [size[0] as usize, size[1] as usize],
        pixmap.data(),
    );
    let options = egui::TextureOptions::LINEAR;
    let id = match texture {
        Some(t) => {
            t.set(image, options);
            t.id()
        }
        None => texture
            .insert(ctx.load_texture("paint-preview", image, options))
            .id(),
    };
    let shown = Rect::from_min_size(area.min, vec2(size[0] as f32, size[1] as f32) / ppp);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    painter.image(id, shown, uv, Color32::WHITE);
}
