//! Icons and small toggles of the layer list, and the tool icons used across the
//! app.

use super::*;

/// Color of a toggle icon: follows hover, faded when off, dimmer when `dim`.
pub(super) fn icon_color(ui: &Ui, r: &Response, on: bool, dim: bool) -> Color32 {
    let c = ui.style().interact(r).fg_stroke.color;
    let c = if on { c } else { c.gamma_multiply(0.35) };
    if dim { c.gamma_multiply(0.45) } else { c }
}

/// An eye that toggles `visible`. `dim` grays it out when a parent group is hidden.
pub(super) fn eye_toggle(ui: &mut Ui, visible: &mut bool, dim: bool) -> Response {
    let (rect, mut r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    if r.clicked() {
        *visible = !*visible;
        r.mark_changed();
    }
    let c = icon_color(ui, &r, *visible, dim);
    let p = ui.painter();
    let (cx, cy) = (rect.center().x, rect.center().y);
    let (w, h) = (7.0, 5.5);
    let stroke = Stroke::new(1.3, c);
    let (left, right) = (pos2(cx - w, cy), pos2(cx + w, cy));
    let curve = |dy: f32| {
        Shape::QuadraticBezier(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [left, pos2(cx, cy + dy), right],
            false,
            Color32::TRANSPARENT,
            stroke,
        ))
    };
    if *visible {
        p.add(curve(-h * 1.5));
        p.add(curve(h * 1.5));
        p.circle_filled(rect.center(), 2.4, c);
    } else {
        // Closed eye: the lower lid with lashes.
        p.add(curve(h * 1.2));
        for (dx, lx) in [(-4.0, -1.5), (0.0, 0.0), (4.0, 1.5)] {
            let base = pos2(cx + dx, cy + h * 0.6 - dx.abs() * 0.12);
            p.line_segment([base, base + vec2(lx, 2.5)], stroke);
        }
    }
    r.on_hover_text(if *visible {
        tr!("HIDE_LAYER")
    } else {
        tr!("SHOW_LAYER")
    })
}

/// A padlock that toggles `locked`.
pub(super) fn lock_toggle(ui: &mut Ui, locked: &mut bool, dim: bool) -> Response {
    let (rect, mut r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    if r.clicked() {
        *locked = !*locked;
        r.mark_changed();
    }
    let c = icon_color(ui, &r, *locked, dim);
    paint_lock(ui.painter(), rect, *locked, c);
    r.on_hover_text(if *locked {
        tr!("UNLOCK_LAYER")
    } else {
        tr!("LOCK_LAYER")
    })
}

/// A padlock centered in `rect`, closed when `locked`.
pub(super) fn paint_lock(p: &egui::Painter, rect: Rect, locked: bool, c: Color32) {
    let body = Rect::from_center_size(rect.center() + vec2(0.0, 3.0), vec2(10.0, 7.0));
    if locked {
        p.rect_filled(body, 1.5, c);
    } else {
        p.rect_stroke(body, 1.5, Stroke::new(1.2, c), StrokeKind::Inside);
    }
    // Shackle: a half circle above the body, lifted and shifted when unlocked.
    let (lift, shift) = if locked { (0.0, 0.0) } else { (2.0, 2.5) };
    let center = pos2(body.center().x + shift, body.top() - 1.0 - lift);
    let pts: Vec<Pos2> = (0..=10)
        .map(|k| {
            let a = std::f32::consts::PI * (1.0 + k as f32 / 10.0);
            center + vec2(a.cos(), a.sin()) * 3.3
        })
        .collect();
    let mut shackle = vec![pts[0] + vec2(0.0, 1.0 + lift)];
    shackle.extend(pts.iter().copied());
    shackle.push(pts[10] + vec2(0.0, 1.0 + lift * 0.3));
    p.add(Shape::line(shackle, Stroke::new(1.3, c)));
}

/// An error mark: a red disc with a white exclamation point, as tall as the row.
pub(super) fn error_mark(ui: &mut Ui, tip: &str) -> Response {
    // As tall as the row, to be seen at once.
    let size = ui.spacing().interact_size.y.max(ICON);
    let (rect, r) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let c = ui.visuals().error_fg_color;
    let p = ui.painter();
    let center = rect.center();
    let radius = size / 2.0 - 0.5;
    p.circle_filled(center, radius, c);
    let ink = Color32::WHITE;
    let stroke = Stroke::new(radius * 0.26, ink);
    p.line_segment(
        [
            center + vec2(0.0, -radius * 0.58),
            center + vec2(0.0, radius * 0.18),
        ],
        stroke,
    );
    p.circle_filled(center + vec2(0.0, radius * 0.52), radius * 0.15, ink);
    r.on_hover_text(tip)
}

/// Padlock button keeping a layer's width and height in proportion.
pub(super) fn aspect_lock(ui: &mut Ui, free_aspect: &mut bool) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    if r.clicked() {
        *free_aspect = !*free_aspect;
    }
    let locked = !*free_aspect;
    if ui.is_rect_visible(rect) {
        if r.hovered() {
            let bg = ui.style().interact(&r).weak_bg_fill;
            ui.painter().rect_filled(rect, 3.0, bg);
        }
        // Full strength when locked, faint when width and height are free.
        let c = icon_color(ui, &r, locked, false);
        paint_lock(ui.painter(), rect, locked, c);
    }
    r.on_hover_text(if locked {
        tr!("ASPECT_RATIO_LOCKED")
    } else {
        tr!("ASPECT_RATIO_UNLOCKED")
    })
}

/// Square thumbnail of a layer: the middle of the image, cropped to fill the square.
/// `image_uv` is the part of the texture holding the image, without margins drawn by
/// effects.
pub(super) fn thumbnail(
    ui: &mut Ui,
    tex: Option<egui::TextureId>,
    size_px: [u32; 2],
    image_uv: [f32; 4],
) {
    let (rect, _) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::hover());
    let radius = 3.0;
    if let Some(tex) = tex {
        let [w, h] = size_px.map(|v| v.max(1) as f32);
        // Fraction of the image kept along each axis.
        let (kx, ky) = if w > h { (h / w, 1.0) } else { (1.0, w / h) };
        let [u0, v0, u1, v1] = image_uv;
        let uv = Rect::from_center_size(
            pos2((u0 + u1) / 2.0, (v0 + v1) / 2.0),
            vec2(kx * (u1 - u0), ky * (v1 - v0)),
        );
        egui::Image::new((tex, rect.size()))
            .uv(uv)
            .corner_radius(radius)
            .paint_at(ui, rect);
    }
    let border = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.painter()
        .rect_stroke(rect, radius, border, StrokeKind::Inside);
}

/// Folder icon shown in place of a group's thumbnail.
pub(super) fn folder_icon(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::hover());
    let c = ui.visuals().widgets.inactive.fg_stroke.color;
    let body = Rect::from_center_size(rect.center() + vec2(0.0, 2.0), vec2(22.0, 15.0));
    let tab = Rect::from_min_size(body.min - vec2(0.0, 3.0), vec2(9.0, 4.0));
    let p = ui.painter();
    p.rect_filled(tab, 1.0, c.gamma_multiply(0.7));
    p.rect_filled(body, 2.0, c.gamma_multiply(0.7));
}

/// Bent arrow before a clipped node's thumbnail, pointing down to the layer it is
/// clipped to.
pub(super) fn clip_mark(ui: &mut Ui) {
    let (rect, r) = ui.allocate_exact_size(vec2(ICON * 0.8, ICON), Sense::hover());
    r.on_hover_text(tr!("CLIPPED_TO_LAYER_BELOW"));
    let c = ui.visuals().widgets.inactive.fg_stroke.color;
    let stroke = Stroke::new(crate::ui::tokens::STROKE, c);
    let k = rect.center();
    let p = ui.painter();
    p.line_segment([k + vec2(-5.0, -5.0), k + vec2(1.0, -5.0)], stroke);
    p.line_segment([k + vec2(1.0, -5.0), k + vec2(1.0, 5.0)], stroke);
    p.line_segment([k + vec2(-2.5, 2.0), k + vec2(1.0, 5.5)], stroke);
    p.line_segment([k + vec2(4.5, 2.0), k + vec2(1.0, 5.5)], stroke);
}

/// Triangle that opens or closes a group in the list.
pub(super) fn expander(ui: &mut Ui, expanded: &mut bool) {
    let (rect, r) = ui.allocate_exact_size(vec2(ICON * 0.8, ICON), Sense::click());
    if r.clicked() {
        *expanded = !*expanded;
    }
    let c = ui.style().interact(&r).fg_stroke.color;
    let k = rect.center();
    let pts = if *expanded {
        vec![
            k + vec2(-4.0, -2.0),
            k + vec2(4.0, -2.0),
            k + vec2(0.0, 3.0),
        ]
    } else {
        vec![
            k + vec2(-2.0, -4.0),
            k + vec2(3.0, 0.0),
            k + vec2(-2.0, 4.0),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(pts, c, Stroke::NONE));
}

/// Icons of the toolbar above the layer list.
#[derive(Clone, Copy)]
pub enum Tool {
    Add,
    /// A template element: an outline with a button in it, and a plus.
    AddElement,
    AddImage,
    AddText,
    AddShape,
    AddCanvas,
    Group,
    Ungroup,
    Duplicate,
    Delete,
    Top,
    Up,
    Down,
    Bottom,
    /// Back to the defaults: a circular arrow, turning left.
    Reset,
    /// Selecting and moving layers: an arrow pointer.
    Select,
    /// Painting even lines: a pencil.
    Pencil,
    /// The brush of a kind: a round brush, a calligraphy nib, a marker, a spray can, a
    /// flat bristle brush.
    Brush(crate::document::paint::BrushKind),
    /// Cutting painted strokes: an eraser.
    Eraser,
    /// Picking a color on the canvas: a pipette.
    Eyedropper,
    /// Emptying a shape of its paint: a frame with a stroke in it, crossed out.
    ClearPaint,
    Align(Align),
}

pub(super) const TOOL: f32 = crate::ui::tokens::ICON_TOOL;

/// A flat icon button of the layer toolbar; returns whether it was clicked.
pub fn tool_button(ui: &mut Ui, tool: Tool, enabled: bool, tip: &str) -> bool {
    let r = tool_icon(ui, tool, enabled, tip);
    enabled && r.clicked()
}

/// A flat icon button of the layer toolbar, e.g. to open a menu from it.
pub fn tool_icon(ui: &mut Ui, tool: Tool, enabled: bool, tip: &str) -> Response {
    let back = ui.visuals().panel_fill;
    flat_button(ui, vec2(TOOL, TOOL), enabled, tip, |p, rect, c| {
        draw_tool(p, rect, tool, c, back);
    })
}

/// A flat button of `size`, its icon drawn by `paint` in the color to use: nothing
/// behind it until hovered or pressed, faded when not `enabled` (then it can't be
/// clicked), `tip` as its tooltip. The app's icon buttons are all made with it.
pub fn flat_button(
    ui: &mut Ui,
    size: egui::Vec2,
    enabled: bool,
    tip: &str,
    paint: impl FnOnce(&egui::Painter, Rect, Color32),
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(size, sense);
    let r = r.on_hover_text(tip);
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let visuals = ui.style().interact(&r);
    let p = ui.painter();
    if enabled && (r.hovered() || r.is_pointer_button_down_on()) {
        p.rect_filled(rect, crate::ui::tokens::RADIUS, visuals.weak_bg_fill);
    }
    let c = if enabled {
        visuals.fg_stroke.color
    } else {
        ui.visuals()
            .widgets
            .noninteractive
            .fg_stroke
            .color
            .gamma_multiply(0.4)
    };
    paint(p, rect, c);
    r
}

/// A menu entry with `tool`'s icon before `label`; `selected` stands out. Returns its
/// response, clicked when picked.
/// It is as wide as the widest of `labels`, the entries of its menu, so they line up.
pub fn icon_entry(
    ui: &mut Ui,
    tool: Tool,
    label: &str,
    labels: &[&str],
    selected: bool,
) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let layout = |t: &str| {
        ui.painter()
            .layout_no_wrap(t.to_owned(), font.clone(), Color32::PLACEHOLDER)
    };
    let text = layout(label);
    let widest = labels
        .iter()
        .map(|l| layout(l).size().x)
        .fold(text.size().x, f32::max);
    let width = TOOL + 6.0 + widest + 10.0;
    let (rect, r) = ui.allocate_exact_size(vec2(width, TOOL), Sense::click());
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let visuals = ui.visuals();
    let p = ui.painter();
    let fg = if selected {
        p.rect_filled(rect, crate::ui::tokens::RADIUS, visuals.selection.bg_fill);
        visuals.selection.stroke.color
    } else {
        if r.hovered() {
            p.rect_filled(
                rect,
                crate::ui::tokens::RADIUS,
                visuals.widgets.hovered.weak_bg_fill,
            );
        }
        visuals.text_color()
    };
    let back = if selected {
        visuals.selection.bg_fill
    } else {
        visuals.panel_fill
    };
    draw_tool(
        p,
        Rect::from_min_size(rect.min, vec2(TOOL, TOOL)),
        tool,
        fg,
        back,
    );
    p.galley(
        egui::pos2(
            rect.min.x + TOOL + 6.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        fg,
    );
    r
}

/// Draws `tool`'s icon in `rect`, in color `c`; parts cut out of it show `back`.
pub fn draw_tool(p: &egui::Painter, rect: Rect, tool: Tool, c: Color32, back: Color32) {
    let s = Stroke::new(1.4, c);
    let k = rect.center();
    let at = |x: f32, y: f32| k + vec2(x, y);
    let folder = |dx: f32| {
        let body = Rect::from_center_size(at(dx, 1.5), vec2(13.0, 9.0));
        let tab = Rect::from_min_size(body.min - vec2(0.0, 2.5), vec2(5.5, 3.0));
        p.rect_filled(tab, 1.0, c);
        p.rect_stroke(body, 1.5, s, StrokeKind::Inside);
    };
    let chevron = |dy: f32, up: bool| {
        let d = if up { -3.0 } else { 3.0 };
        p.add(Shape::line(
            vec![at(-5.0, dy - d), at(0.0, dy + d), at(5.0, dy - d)],
            s,
        ));
    };
    match tool {
        Tool::Add => {
            p.line_segment([at(-6.0, 0.0), at(6.0, 0.0)], s);
            p.line_segment([at(0.0, -6.0), at(0.0, 6.0)], s);
        }
        Tool::AddElement => {
            // A rounded outline with a button in it, and a plus at its corner.
            p.rect_stroke(
                Rect::from_min_size(at(-8.0, -7.0), vec2(13.0, 11.0)),
                3.0,
                s,
                StrokeKind::Inside,
            );
            p.circle_stroke(at(-1.5, -1.5), 2.5, s);
            p.circle_filled(at(5.0, 4.5), 5.0, back);
            p.line_segment([at(1.5, 4.5), at(8.5, 4.5)], s);
            p.line_segment([at(5.0, 1.0), at(5.0, 8.0)], s);
        }
        Tool::AddImage => {
            // A framed picture: hills under a sun.
            p.rect_stroke(
                Rect::from_center_size(k, vec2(15.0, 12.0)),
                1.5,
                s,
                StrokeKind::Inside,
            );
            p.add(Shape::line(
                vec![
                    at(-6.0, 4.5),
                    at(-2.0, 0.0),
                    at(1.0, 3.0),
                    at(3.0, 1.0),
                    at(6.0, 4.5),
                ],
                s,
            ));
            p.circle_filled(at(3.0, -2.5), 1.4, c);
        }
        Tool::AddText => {
            // A capital T next to a text cursor.
            let bold = Stroke::new(1.8, c);
            p.line_segment([at(-7.0, -5.5), at(2.0, -5.5)], bold);
            p.line_segment([at(-2.5, -5.5), at(-2.5, 6.0)], bold);
            p.line_segment([at(6.0, -6.0), at(6.0, 6.0)], s);
            p.line_segment([at(4.5, -6.0), at(7.5, -6.0)], s);
            p.line_segment([at(4.5, 6.0), at(7.5, 6.0)], s);
        }
        Tool::AddShape => {
            // A square behind a circle.
            p.rect_stroke(
                Rect::from_min_size(at(-7.0, -7.0), vec2(9.0, 9.0)),
                1.0,
                s,
                StrokeKind::Inside,
            );
            p.circle_filled(at(2.5, 2.5), 5.5, back);
            p.circle_stroke(at(2.5, 2.5), 5.0, s);
        }
        Tool::AddCanvas => {
            // A frame with a painted wave across it.
            p.rect_stroke(
                Rect::from_center_size(k, vec2(16.0, 13.0)),
                1.0,
                s,
                StrokeKind::Inside,
            );
            p.add(Shape::line(
                (0..=12)
                    .map(|i| {
                        let x = -5.0 + i as f32 * 10.0 / 12.0;
                        at(x, -1.5 * (x * 0.6).sin() + 0.5)
                    })
                    .collect(),
                Stroke::new(2.0, c),
            ));
        }
        Tool::Group => {
            folder(-2.0);
            p.line_segment([at(4.0, 4.5), at(10.0, 4.5)], s);
            p.line_segment([at(7.0, 1.5), at(7.0, 7.5)], s);
        }
        Tool::Ungroup => {
            folder(-2.0);
            p.rect_filled(
                Rect::from_center_size(at(7.0, 4.5), vec2(8.0, 4.0)),
                0.0,
                back,
            );
            p.line_segment([at(4.0, 4.5), at(10.0, 4.5)], s);
        }
        Tool::Duplicate => {
            p.rect_stroke(
                Rect::from_center_size(at(-2.0, -2.0), vec2(10.0, 10.0)),
                1.5,
                s,
                StrokeKind::Inside,
            );
            let front = Rect::from_center_size(at(2.5, 2.5), vec2(10.0, 10.0));
            p.rect_filled(front, 1.5, back);
            p.rect_stroke(front, 1.5, s, StrokeKind::Inside);
        }
        Tool::Delete => {
            // Trash can: lid, handle and a tapered bin with ribs.
            p.line_segment([at(-6.5, -4.5), at(6.5, -4.5)], s);
            p.add(Shape::line(
                vec![at(-2.5, -4.5), at(-2.0, -7.0), at(2.0, -7.0), at(2.5, -4.5)],
                s,
            ));
            p.add(Shape::closed_line(
                vec![at(-5.0, -4.5), at(-4.0, 7.0), at(4.0, 7.0), at(5.0, -4.5)],
                s,
            ));
            for x in [-1.8, 1.8] {
                p.line_segment([at(x, -2.0), at(x, 4.5)], s);
            }
        }
        Tool::Reset => {
            // Most of a circle, open at the top, the arrowhead at its right end pointing
            // back up and left.
            let r = 6.0;
            let start = -std::f32::consts::FRAC_PI_2 + 0.6;
            let sweep = std::f32::consts::TAU - 1.2;
            let point = |a: f32| at(r * a.cos(), r * a.sin() + 0.5);
            let arc: Vec<Pos2> = (0..=24)
                .map(|i| point(start + sweep * i as f32 / 24.0))
                .collect();
            p.add(Shape::line(arc, s));
            // Against the way the arc goes from its start.
            let dir = -vec2(-start.sin(), start.cos());
            let side = vec2(-dir.y, dir.x);
            let tip = point(start) + dir * 2.5;
            let back = point(start) - dir * 1.5;
            p.add(Shape::convex_polygon(
                vec![tip, back + side * 3.0, back - side * 3.0],
                c,
                Stroke::NONE,
            ));
        }
        Tool::Select => {
            p.add(Shape::convex_polygon(
                vec![at(-4.5, -7.0), at(5.0, 2.5), at(0.5, 2.5), at(-4.5, 7.0)],
                c,
                Stroke::NONE,
            ));
            p.line_segment([at(0.5, 2.0), at(3.0, 7.0)], Stroke::new(2.0, c));
        }
        Tool::Pencil => {
            // Along the diagonal: the body, then the sharpened tip at the bottom left.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let tip = at(-7.0, 7.0);
            let wood = tip + d * 4.0;
            let end = tip + d * 18.0;
            p.add(Shape::closed_line(
                vec![wood + n * 2.5, end + n * 2.5, end - n * 2.5, wood - n * 2.5],
                s,
            ));
            p.add(Shape::convex_polygon(
                vec![tip, wood + n * 2.5, wood - n * 2.5],
                c,
                Stroke::NONE,
            ));
            p.line_segment([end - d * 3.0 + n * 2.5, end - d * 3.0 - n * 2.5], s);
        }
        Tool::Brush(crate::document::paint::BrushKind::Calligraphy) => {
            // A pen nib pointing to the bottom left, its slit and breather hole, then the
            // holder.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let tip = at(-7.5, 7.5);
            let shoulder = tip + d * 8.0;
            let back_end = tip + d * 11.5;
            p.add(Shape::convex_polygon(
                vec![tip, shoulder + n * 3.8, back_end, shoulder - n * 3.8],
                c,
                Stroke::NONE,
            ));
            p.line_segment([tip, tip + d * 5.5], Stroke::new(1.0, back));
            p.circle_filled(tip + d * 6.3, 1.1, back);
            p.line_segment([back_end - d * 0.5, at(7.5, -7.5)], Stroke::new(2.6, c));
        }
        Tool::Brush(crate::document::paint::BrushKind::Marker) => {
            // A marker on the diagonal, its chisel tip filled at the bottom left.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let tip = at(-7.0, 7.0);
            let neck = tip + d * 4.5;
            let end = tip + d * 19.0;
            p.add(Shape::convex_polygon(
                vec![
                    tip + n * 2.0,
                    neck + n * 2.0,
                    neck - n * 2.0,
                    tip - n * 0.5 + d * 2.0,
                ],
                c,
                Stroke::NONE,
            ));
            p.add(Shape::closed_line(
                vec![neck + n * 3.2, end + n * 3.2, end - n * 3.2, neck - n * 3.2],
                s,
            ));
            p.line_segment([neck + d * 6.0 + n * 3.2, neck + d * 6.0 - n * 3.2], s);
        }
        Tool::Brush(crate::document::paint::BrushKind::Spray) => {
            // A spray can, its nozzle at the top, dots spraying to the top right.
            p.rect_stroke(
                Rect::from_min_max(at(-7.0, -1.5), at(1.0, 8.0)),
                1.5,
                s,
                StrokeKind::Inside,
            );
            p.rect_filled(Rect::from_min_max(at(-5.0, -4.5), at(-1.0, -1.5)), 0.5, c);
            for (x, y) in [
                (2.5, -6.0),
                (5.0, -7.5),
                (7.5, -5.5),
                (4.5, -3.5),
                (7.0, -2.0),
                (3.5, -9.0),
                (8.0, -9.0),
            ] {
                p.circle_filled(at(x, y), 0.95, c);
            }
        }
        Tool::Brush(crate::document::paint::BrushKind::Bristle) => {
            // A handle and a flat ferrule, then bristles spreading apart at the bottom
            // left.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let base = at(-2.0, 2.0);
            p.line_segment([base + d * 4.0, at(7.5, -7.5)], Stroke::new(2.2, c));
            p.add(Shape::convex_polygon(
                vec![
                    base + n * 3.2,
                    base + d * 4.0 + n * 2.2,
                    base + d * 4.0 - n * 2.2,
                    base - n * 3.2,
                ],
                c,
                Stroke::NONE,
            ));
            for k in [-2.6f32, -0.9, 0.9, 2.6] {
                let from = base + n * k;
                let to = base - d * 7.5 + n * (k * 1.45);
                p.line_segment([from, to], Stroke::new(1.1, c));
            }
        }
        Tool::Brush(crate::document::paint::BrushKind::Round) => {
            // A handle, then the bristles at the bottom left in a soft drop.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let base = at(-3.0, 3.0);
            p.line_segment([base + d * 2.0, at(7.5, -7.5)], Stroke::new(2.2, c));
            p.add(Shape::convex_polygon(
                vec![
                    base + n * 2.6,
                    base + d * 2.0 + n * 2.0,
                    base + d * 2.0 - n * 2.0,
                    base - n * 2.6,
                    at(-7.5, 7.5),
                ],
                c,
                Stroke::NONE,
            ));
        }
        Tool::ClearPaint => {
            // A frame with a painted wave in it, a cross over its bottom right corner.
            let frame = Rect::from_min_max(at(-7.5, -6.5), at(5.0, 4.5));
            p.rect_stroke(frame, 1.5, s, StrokeKind::Inside);
            p.add(Shape::line(
                vec![at(-5.0, 1.0), at(-3.0, -2.5), at(-0.5, 0.5), at(2.0, -3.0)],
                Stroke::new(1.8, c),
            ));
            p.circle_filled(at(4.5, 4.5), 4.6, back);
            let x = Stroke::new(1.6, c);
            p.line_segment([at(2.0, 2.0), at(7.0, 7.0)], x);
            p.line_segment([at(7.0, 2.0), at(2.0, 7.0)], x);
        }
        Tool::Eyedropper => {
            // Along the diagonal: the bulb at the top right, the collar across, then the
            // glass tube down to the tip at the bottom left.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let tip = at(-7.0, 7.0);
            let collar = tip + d * 11.0;
            p.add(Shape::closed_line(
                vec![tip + d * 1.5, collar + n * 1.8, collar - n * 1.8],
                s,
            ));
            p.line_segment([collar + n * 3.5, collar - n * 3.5], Stroke::new(2.0, c));
            p.circle_filled(collar + d * 3.5, 2.8, c);
        }
        Tool::Eraser => {
            // A block on the diagonal, its lower part filled.
            let d = vec2(1.0, -1.0).normalized();
            let n = d.rot90();
            let a = at(-6.0, 6.0);
            let m = a + d * 7.0;
            let b = a + d * 15.0;
            p.add(Shape::convex_polygon(
                vec![a + n * 3.5, m + n * 3.5, m - n * 3.5, a - n * 3.5],
                c,
                Stroke::NONE,
            ));
            p.add(Shape::closed_line(
                vec![a + n * 3.5, b + n * 3.5, b - n * 3.5, a - n * 3.5],
                s,
            ));
        }
        Tool::Up => chevron(0.0, true),
        Tool::Down => chevron(0.0, false),
        Tool::Top => {
            p.line_segment([at(-6.0, -5.5), at(6.0, -5.5)], s);
            chevron(1.5, true);
        }
        Tool::Bottom => {
            p.line_segment([at(-6.0, 5.5), at(6.0, 5.5)], s);
            chevron(-1.5, false);
        }
        Tool::Align(a) => {
            // A guide line and two bars of different lengths lined up against it.
            // Drawn for horizontal alignment, then swapped for vertical.
            let (vertical, pos) = match a {
                Align::Left => (false, -1.0),
                Align::CenterX => (false, 0.0),
                Align::Right => (false, 1.0),
                Align::Top => (true, -1.0),
                Align::CenterY => (true, 0.0),
                Align::Bottom => (true, 1.0),
            };
            let pt = |x: f32, y: f32| if vertical { at(y, x) } else { at(x, y) };
            let guide = pos * 8.0;
            p.line_segment([pt(guide, -8.0), pt(guide, 8.0)], s);
            for (y, len) in [(-3.0, 12.0), (3.0, 7.0)] {
                let x0 = match pos {
                    p if p < 0.0 => guide + 1.5,
                    p if p > 0.0 => guide - 1.5 - len,
                    _ => -len / 2.0,
                };
                let bar = if vertical {
                    Rect::from_min_size(at(y - 1.5, x0), vec2(3.0, len))
                } else {
                    Rect::from_min_size(at(x0, y - 1.5), vec2(len, 3.0))
                };
                p.rect_filled(bar, 1.0, c);
            }
        }
    }
}
