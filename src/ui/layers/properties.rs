//! The Properties panel of the selected layer or group, and its shared field
//! helpers.

use super::*;

/// Gap between the rows of the property grids.
pub(super) const PROP_ROW_GAP: f32 = 6.0;

/// Labels on the left of the property grids: the label column is as wide as the longest,
/// so the controls line up from one section to the next, in every language.
pub(super) const PROP_LABELS: [&str; 30] = [
    tr_noop!("LAYER_NAME"),
    tr_noop!("OPACITY"),
    tr_noop!("LAYER_POSITION"),
    tr_noop!("LAYER_SIZE"),
    tr_noop!("LAYER_ROTATION"),
    tr_noop!("TEXT_FONT"),
    tr_noop!("TEXT_STYLE"),
    tr_noop!("TEXT_LETTER_SPACING"),
    tr_noop!("TEXT_LINE_SPACING"),
    tr_noop!("TEXT_COLOR"),
    tr_noop!("TEXT_FONT_SIZE"),
    tr_noop!("TEXT_OUTLINE"),
    tr_noop!("SHAPE_KIND"),
    tr_noop!("SHAPE_CORNER_RADIUS"),
    tr_noop!("SHAPE_SIDES"),
    tr_noop!("SHAPE_POINTS"),
    tr_noop!("SHAPE_POINT_COUNT"),
    tr_noop!("SHAPE_INNER_RADIUS"),
    tr_noop!("SHAPE_FILL"),
    tr_noop!("SHAPE_STROKE"),
    tr_noop!("STROKE_DASHES"),
    tr_noop!("STROKE_CAPS"),
    tr_noop!("STROKE_JOINS"),
    tr_noop!("TEXT_PATH"),
    tr_noop!("TEXT_PATH_BEND"),
    tr_noop!("TEXT_PATH_RADIUS"),
    tr_noop!("TEXT_PATH_SHAPE_LAYER"),
    tr_noop!("TEXT_PATH_POSITION"),
    tr_noop!("TEXT_PATH_FLIP"),
    tr_noop!("TEXT_PATH_SHIFT"),
];

/// A two-column grid of labels and controls, lined up with the other property grids.
/// Gives the fields of a popup (a picker, an editor) their frames back: egui draws the
/// widgets of popup menus without frames, which makes a text or number field there
/// look like plain text.
pub fn field_frames(ui: &mut Ui) {
    ui.visuals_mut().widgets = ui.ctx().global_style().visuals.widgets.clone();
}

/// A field's unit, shown after it rather than inside it: "mm", "°", "pt", "%"...
pub fn unit(ui: &mut Ui, unit: &str) -> Response {
    let unit = unit.trim();
    if unit.is_empty() {
        return ui.response();
    }
    ui.label(unit)
}

pub(super) fn prop_grid(ui: &Ui, id: &str) -> egui::Grid {
    let label_width = PROP_LABELS
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
        .spacing([8.0, PROP_ROW_GAP])
        .min_col_width(label_width.ceil())
}

/// What a text style toggle shows.
#[derive(Clone, Copy)]
pub(super) enum StyleIcon {
    Bold,
    Italic,
    Align(TextAlign),
}

/// A square toggle of the text style row, highlighted when `on`.
pub(super) fn style_toggle(ui: &mut Ui, on: bool, icon: StyleIcon, tip: &str) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, r) = ui.allocate_exact_size(vec2(side + 2.0, side), Sense::click());
    let r = r.on_hover_text(tip);
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let visuals = ui.style().interact_selectable(&r, on);
    let p = ui.painter();
    if on || r.hovered() || r.is_pointer_button_down_on() {
        p.rect(
            rect,
            3.0,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
    }
    let c = visuals.fg_stroke.color;
    let k = rect.center();
    match icon {
        StyleIcon::Bold | StyleIcon::Italic => {
            let text = match icon {
                StyleIcon::Bold => RichText::new("B").strong(),
                _ => RichText::new("I").italics(),
            };
            let galley = egui::WidgetText::from(text).into_galley(
                ui,
                None,
                f32::INFINITY,
                egui::TextStyle::Button,
            );
            p.galley(k - galley.size() / 2.0, galley, c);
        }
        StyleIcon::Align(a) => {
            // Four lines of different lengths, lined up on one side or centered.
            let s = Stroke::new(1.4, c);
            for (y, len) in [(-4.5, 12.0), (-1.5, 8.0), (1.5, 12.0), (4.5, 6.0)] {
                let x0 = match a {
                    TextAlign::Left => -6.0,
                    TextAlign::Center => -len / 2.0,
                    TextAlign::Right => 6.0 - len,
                };
                p.line_segment([k + vec2(x0, y), k + vec2(x0 + len, y)], s);
            }
        }
    }
    r
}

/// The Properties panel: what can be set on the selected layer or group.
/// `template_bounds` is the template's axis-aligned box on the page: (center, size).
pub fn properties_panel(
    ui: &mut Ui,
    doc: &mut Document,
    selected: Option<u64>,
    state: &mut LayersPanel,
    template_bounds: Option<(P2, P2)>,
    fonts: &FontLibrary,
) -> LayersOutput {
    let mut out = LayersOutput::default();
    let sel = selected.filter(|&id| doc.path_of(id).is_some());
    properties(ui, doc, sel, state, template_bounds, fonts, &mut out);
    out
}

/// A titled section of a side panel, with a line above it, folded by clicking its arrow
/// or title. `extra`
/// adds widgets at the right of the header and returns true to open the section (e.g.
/// after adding something to it). The open state is shared by all layers and kept
/// between runs.
pub fn section(
    ui: &mut Ui,
    id_salt: &str,
    title: RichText,
    extra: impl FnOnce(&mut Ui) -> bool,
    body: impl FnOnce(&mut Ui),
) {
    let id = ui.make_persistent_id(("section", id_salt));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
    // A line above each section, so sections stand apart, folded or not.
    ui.add_space(2.0);
    ui.separator();
    ui.horizontal(|ui| {
        state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
        let title = ui.add(egui::Label::new(title).sense(Sense::click()));
        if title.clicked() {
            state.toggle(ui);
        }
        let open = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), extra)
            .inner;
        if open {
            state.set_open(true);
        }
    });
    state.show_body_unindented(ui, body);
}

/// A position or size field in mm, from `min` (up to 5000 mm for a size), named by
/// `prefix`. Its row ends with the unit (see [`unit`]).
pub(super) fn mm_value<'a>(d: DragValue<'a>, prefix: &str, min: f32) -> DragValue<'a> {
    let d = d.fixed_decimals(1).speed(0.5).prefix(format!("{prefix} "));
    if min.is_finite() {
        d.range(min..=5000.0)
    } else {
        d
    }
}

/// A drop-down list of a shape setting's values, named by their translation keys.
/// Returns true if another was picked.
pub(super) fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    all: &[T],
    name: impl Fn(T) -> &'static str,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(130.0)
        .selected_text(translate(name(*value)))
        .show_ui(ui, |ui| {
            for &v in all {
                changed |= ui.selectable_value(value, v, translate(name(v))).changed();
            }
        });
    changed
}

/// The part of Properties for an image layer: its print resolution, flagged below
/// `min_dpi`, and source size.
pub(super) fn image_section(ui: &mut Ui, layer: &Layer, min_dpi: f32) {
    section(
        ui,
        "image_props",
        RichText::new(tr!("IMAGE_SECTION")),
        |_| false,
        |ui| {
            // Print resolution of this layer, from its pixel count and printed size.
            let dpi = layer.print_dpi().unwrap_or(f32::INFINITY);
            let shown = format!("{dpi:.0}");
            if dpi < min_dpi {
                crate::ui::callout::callout(
                    ui,
                    crate::ui::callout::Tone::Warning,
                    &tr!("PRINT_RESOLUTION_LOW", dpi = shown),
                )
                .on_hover_text(tr!("LOW_RESOLUTION_HINT", min = format!("{min_dpi:.0}")));
            } else {
                ui.weak(tr!("PRINT_RESOLUTION", dpi = shown));
            }
            ui.weak(tr!(
                "IMAGE_SOURCE_SIZE",
                w = layer.size_px[0],
                h = layer.size_px[1]
            ));
        },
    );
}

/// Alignment, properties and effects of the selected layer or group.
pub(super) fn properties(
    ui: &mut Ui,
    doc: &mut Document,
    sel: Option<u64>,
    state: &mut LayersPanel,
    template_bounds: Option<(P2, P2)>,
    fonts: &FontLibrary,
    out: &mut LayersOutput,
) {
    let Some(id) = sel else {
        let text = if state.template_mode {
            tr!("NO_SHAPE_SELECTED")
        } else {
            tr!("NO_LAYER_SELECTED")
        };
        crate::ui::callout::callout(ui, crate::ui::callout::Tone::Info, text);
        return;
    };
    let page_center = doc.page_center();
    let page = doc.page_mm();
    // A background stays on the outline: placed like a locked layer.
    let fixed = doc
        .layer(id)
        .is_some_and(crate::document::background::is_fixed);
    let locked = doc.effectively_locked(id) || fixed;
    // Align the selection with the page or the template outline.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.label(tr!("ALIGN_TO"));
        if template_bounds.is_none() {
            state.align_to = AlignTo::Page;
        }
        egui::ComboBox::from_id_salt("align_to")
            .width(80.0)
            .selected_text(match state.align_to {
                AlignTo::Page => tr!("ALIGN_TO_PAGE"),
                AlignTo::Template => tr!("ALIGN_TO_TEMPLATE"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.align_to, AlignTo::Page, tr!("ALIGN_TO_PAGE"));
                ui.add_enabled_ui(template_bounds.is_some(), |ui| {
                    ui.selectable_value(
                        &mut state.align_to,
                        AlignTo::Template,
                        tr!("ALIGN_TO_TEMPLATE"),
                    );
                });
            });
        ui.add_space(4.0);
        let area = match (state.align_to, template_bounds) {
            (AlignTo::Template, Some((c, s))) => (
                [c[0] - s[0] / 2.0, c[1] - s[1] / 2.0],
                [c[0] + s[0] / 2.0, c[1] + s[1] / 2.0],
            ),
            _ => ([0.0, 0.0], page),
        };
        for (k, a) in Align::ALL.into_iter().enumerate() {
            if k == 3 {
                ui.add_space(4.0);
            }
            if tool_button(ui, Tool::Align(a), !locked, a.tip()) {
                out.changed |= align(doc, sel, a, area);
            }
        }
    });
    ui.add_space(4.0);
    if fixed {
        crate::ui::callout::callout(
            ui,
            crate::ui::callout::Tone::Info,
            tr!("BACKGROUND_LAYER_NOTE"),
        );
        ui.add_space(4.0);
    }

    // Shape layers a text can follow, by name.
    let shapes: Vec<(u64, String)> = doc
        .leaves()
        .iter()
        .filter(|l| l.layer.shape.is_some())
        .map(|l| (l.layer.id, l.layer.name.clone()))
        .collect();
    // In the template editor: the element the selection is in.
    // A template's shapes, in an element or not, are plain lines.
    let plain = state.template_mode || crate::template::edit::element_of(doc, id).is_some();
    // The template's reference: its place, size, turn, opacity and visibility, saved with
    // the template; nothing it wouldn't keep (flips, distortion, effects).
    let reference = crate::template::edit::is_reference(doc, id);
    let can_clip = can_clip(doc, id);
    let clip_hint = state.hint(tr!("CLIP_TO_LAYER_BELOW_HINT"), Command::Clip);
    // How the layer or group mixes with what lies below.
    let blend_row = |ui: &mut Ui, blend: &mut BlendMode| {
        ui.label(tr!("BLEND_MODE"));
        let mut changed = false;
        egui::ComboBox::from_id_salt("blend_mode")
            .selected_text(translate(blend.name_key()))
            .show_ui(ui, |ui| {
                for (k, mode) in BlendMode::ALL.into_iter().enumerate() {
                    // A line between the families of `BlendMode::ALL`.
                    if matches!(k, 1 | 4 | 7 | 10 | 12) {
                        ui.separator();
                    }
                    changed |= ui
                        .selectable_value(blend, mode, translate(mode.name_key()))
                        .changed();
                }
            })
            .response
            .on_hover_text(tr!("BLEND_MODE_HINT"));
        ui.end_row();
        changed
    };
    // Turns clipping off, or on when there is a layer below.
    let clip_row = |ui: &mut Ui, clip: &mut bool| {
        ui.label("");
        let r = ui
            .add_enabled(
                *clip || can_clip,
                egui::Checkbox::new(clip, tr!("CLIP_TO_LAYER_BELOW")),
            )
            .on_hover_text(&clip_hint)
            .on_disabled_hover_text(&clip_hint);
        ui.end_row();
        r.changed()
    };
    // A group's Flip H or V, or turn, done once it is no longer borrowed.
    let mut group_flip = None;
    let mut group_turn = None;
    match doc.node_mut(id) {
        Some(Node::Group(g)) => {
            // A group has only the properties all layers have.
            section(
                ui,
                "layer_props",
                RichText::new(tr!("LAYER_GENERAL_SECTION")).strong(),
                |_| false,
                |ui| {
                    prop_grid(ui, "group_props").show(ui, |ui| {
                        ui.label(tr!("LAYER_NAME"));
                        out.changed |= ui.text_edit_singleline(&mut g.name).changed();
                        ui.end_row();

                        // A template element: its kind and line color.
                        if let Some(tag) = &mut g.element {
                            ui.label(tr!("ELEMENT_KIND"));
                            ui.label(translate(tag.kind.label()));
                            ui.end_row();
                            ui.label(tr!("ELEMENT_COLOR"));
                            let [r, g, b] = tag.color;
                            let mut c = [r, g, b, 255];
                            if crate::document::colors::button(ui, &mut c, false, None)
                                .on_hover_text(tr!("ELEMENT_LINE_COLOR"))
                                .changed()
                            {
                                tag.color = [c[0], c[1], c[2]];
                                out.changed = true;
                            }
                            ui.end_row();
                            return;
                        }
                        if plain {
                            return;
                        }

                        ui.label(tr!("OPACITY"));
                        out.changed |= ui
                            .add(Slider::new(&mut g.opacity, 0.0..=1.0).fixed_decimals(2))
                            .on_hover_text(tr!("GROUP_OPACITY_HINT"))
                            .changed();
                        ui.end_row();
                        out.changed |= blend_row(ui, &mut g.blend);
                        out.changed |= clip_row(ui, &mut g.clip);
                    });
                    if plain {
                        return;
                    }
                    ui.add_space(4.0);
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            if ui
                                .button(tr!("FLIP_LAYER_HORIZONTALLY"))
                                .on_hover_text(tr!("FLIP_GROUP_HORIZONTALLY_HINT"))
                                .clicked()
                            {
                                group_flip = Some(true);
                            }
                            if ui
                                .button(tr!("FLIP_LAYER_VERTICALLY"))
                                .on_hover_text(tr!("FLIP_GROUP_VERTICALLY_HINT"))
                                .clicked()
                            {
                                group_flip = Some(false);
                            }
                            ui.add_space(4.0);
                            for (deg, hint) in [
                                (-90.0, tr!("ROTATE_GROUP_LEFT_HINT")),
                                (90.0, tr!("ROTATE_GROUP_RIGHT_HINT")),
                            ] {
                                if ui.button(format!("{deg:+}°")).on_hover_text(hint).clicked() {
                                    group_turn = Some(deg);
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(tr!("ROTATE_GROUP_BY"));
                            ui.add(
                                DragValue::new(&mut state.group_turn_deg)
                                    .range(-360.0..=360.0)
                                    .speed(0.5)
                                    .fixed_decimals(1),
                            )
                            .on_hover_text(tr!("ROTATE_GROUP_BY_HINT"));
                            unit(ui, "°");
                            if ui
                                .add_enabled(
                                    state.group_turn_deg % 360.0 != 0.0,
                                    Button::new(tr!("ROTATE_GROUP")),
                                )
                                .on_hover_text(tr!("ROTATE_GROUP_BY_HINT"))
                                .clicked()
                            {
                                group_turn = Some(state.group_turn_deg);
                            }
                        });
                    });
                },
            );
        }
        Some(Node::Layer(layer)) => {
            // Common properties first, then what only this kind of layer has.
            let open = state.show_text;
            section(
                ui,
                "layer_props",
                RichText::new(tr!("LAYER_GENERAL_SECTION")).strong(),
                move |_| open,
                |ui| {
                    prop_grid(ui, "layer_props").show(ui, |ui| {
                        ui.label(tr!("LAYER_NAME"));
                        out.changed |= ui.text_edit_singleline(&mut layer.name).changed();
                        ui.end_row();

                        if !plain || reference {
                            ui.label(tr!("OPACITY"));
                            out.changed |= ui
                                .add(Slider::new(&mut layer.opacity, 0.0..=1.0).fixed_decimals(2))
                                .changed();
                            ui.end_row();
                        }
                        if !plain && !reference {
                            out.changed |= blend_row(ui, &mut layer.blend);
                            out.changed |= clip_row(ui, &mut layer.clip);
                        }

                        ui.add_enabled_ui(!locked, |ui| ui.label(tr!("LAYER_POSITION")));
                        ui.add_enabled_ui(!locked, |ui| {
                            ui.horizontal(|ui| {
                                let (t, b) = (&mut layer.transform, &mut layer.bindings);
                                let [x, y] = &mut t.center;
                                out.changed |= variables::field(ui, b, Field::X, x, |d| {
                                    mm_value(d, "x", f32::NEG_INFINITY)
                                })
                                .changed();
                                out.changed |= variables::field(ui, b, Field::Y, y, |d| {
                                    mm_value(d, "y", f32::NEG_INFINITY)
                                })
                                .changed();
                                unit(ui, "mm");
                            });
                        });
                        ui.end_row();

                        ui.label(tr!("LAYER_SIZE"));
                        ui.add_enabled_ui(!locked, |ui| {
                            ui.horizontal(|ui| {
                                if layer.shape.is_some() {
                                    out.changed |= shape_size(ui, layer, state);
                                    return;
                                }
                                let [px_w, px_h] = layer.size_px.map(|v| v.max(1) as f32);
                                let [mut w, mut h] = layer.size_mm();
                                let old = [w, h];
                                let b = &mut layer.bindings;
                                let links =
                                    [Field::Width, Field::Height].map(|f| b.get(&f).cloned());
                                let rw = variables::field(ui, b, Field::Width, &mut w, |d| {
                                    mm_value(d, tr!("LAYER_WIDTH_PREFIX"), 0.01)
                                });
                                let rh = variables::field(ui, b, Field::Height, &mut h, |d| {
                                    mm_value(d, tr!("LAYER_HEIGHT_PREFIX"), 0.01)
                                });
                                unit(ui, "mm");
                                aspect_lock(ui, &mut state.free_aspect);
                                if !state.free_aspect {
                                    variables::link_aspect(
                                        b,
                                        links,
                                        old,
                                        &variables::published(ui.ctx()),
                                    );
                                }
                                let s = &mut layer.transform.scale;
                                if rw.changed() {
                                    let k = w / px_w / s[0];
                                    s[0] = w / px_w;
                                    if !state.free_aspect {
                                        s[1] *= k;
                                    }
                                    out.changed = true;
                                }
                                if rh.changed() {
                                    let k = h / px_h / s[1];
                                    s[1] = h / px_h;
                                    if !state.free_aspect {
                                        s[0] *= k;
                                    }
                                    out.changed = true;
                                }
                            });
                        });
                        ui.end_row();

                        ui.label(tr!("LAYER_ROTATION"));
                        ui.add_enabled_ui(!locked, |ui| {
                            ui.horizontal(|ui| {
                                let (t, b) = (&mut layer.transform, &mut layer.bindings);
                                let rot = &mut t.rotation_deg;
                                // The slider's own value box can't take a formula: the
                                // field next to it does.
                                if variables::field(ui, b, Field::Rotation, rot, |d| {
                                    d.fixed_decimals(1).speed(0.5)
                                })
                                .changed()
                                {
                                    *rot = variables::wrap_degrees(*rot);
                                    out.changed = true;
                                }
                                unit(ui, "°");
                                out.changed |= ui
                                    .add(Slider::new(rot, -180.0..=180.0).show_value(false))
                                    .changed();
                            });
                        });
                        ui.end_row();

                        // Turned in 3D: not for the template's reference, which keeps
                        // only its place, size and turn.
                        if !reference {
                            ui.label(tr!("LAYER_TILT"))
                                .on_hover_text(tr!("LAYER_TILT_HINT"));
                            ui.add_enabled_ui(!locked, |ui| {
                                ui.horizontal(|ui| {
                                    let (t, b) = (&mut layer.transform, &mut layer.bindings);
                                    for (field, v, prefix) in [
                                        (Field::TiltX, &mut t.tilt_x_deg, "x"),
                                        (Field::TiltY, &mut t.tilt_y_deg, "y"),
                                    ] {
                                        if variables::field(ui, b, field, v, |d| {
                                            d.fixed_decimals(1)
                                                .speed(0.5)
                                                .prefix(format!("{prefix} "))
                                        })
                                        .changed()
                                        {
                                            *v = variables::clamp_tilt(*v);
                                            out.changed = true;
                                        }
                                    }
                                    unit(ui, "°");
                                });
                            });
                            ui.end_row();
                            ui.label(tr!("LAYER_PERSPECTIVE"))
                                .on_hover_text(tr!("LAYER_PERSPECTIVE_HINT"));
                            ui.add_enabled_ui(!locked && layer.transform.is_tilted(), |ui| {
                                out.changed |= ui
                                    .add(
                                        Slider::new(&mut layer.transform.perspective, 0.0..=1.0)
                                            .fixed_decimals(2),
                                    )
                                    .on_hover_text(tr!("LAYER_PERSPECTIVE_HINT"))
                                    .changed();
                            });
                            ui.end_row();
                        }
                    });

                    ui.add_space(4.0);
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            if ui
                                .button(tr!("FIT_LAYER_TO_PAGE"))
                                .on_hover_text(tr!("FIT_LAYER_TO_PAGE_HINT"))
                                .clicked()
                            {
                                layer.fit_to(page_center, page, true);
                                out.changed = true;
                            }
                            if ui
                                .add_enabled(
                                    template_bounds.is_some(),
                                    Button::new(tr!("FIT_LAYER_TO_TEMPLATE")),
                                )
                                .on_hover_text(tr!("FIT_LAYER_TO_TEMPLATE_HINT"))
                                .clicked()
                            {
                                let (c, s) = template_bounds.unwrap_or((page_center, page));
                                layer.fit_to(c, s, true);
                                out.changed = true;
                            }
                            if ui
                                .button(tr!("RESET_LAYER_TRANSFORM"))
                                .on_hover_text(tr!("RESET_LAYER_TRANSFORM_HINT"))
                                .clicked()
                            {
                                layer.fit_to(page_center, page, false);
                                layer.transform.flip_x = false;
                                layer.transform.flip_y = false;
                                layer.transform.distort = Default::default();
                                layer.transform.tilt_x_deg = 0.0;
                                layer.transform.tilt_y_deg = 0.0;
                                layer.transform.perspective = crate::document::DEFAULT_PERSPECTIVE;
                                out.changed = true;
                            }
                            if reference {
                                return;
                            }
                            if ui
                                .button(tr!("FLIP_LAYER_HORIZONTALLY"))
                                .on_hover_text(tr!("FLIP_LAYER_HORIZONTALLY_HINT"))
                                .clicked()
                            {
                                layer.transform.flip_x ^= true;
                                out.changed = true;
                            }
                            if ui
                                .button(tr!("FLIP_LAYER_VERTICALLY"))
                                .on_hover_text(tr!("FLIP_LAYER_VERTICALLY_HINT"))
                                .clicked()
                            {
                                layer.transform.flip_y ^= true;
                                out.changed = true;
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.add_enabled_ui(!locked && !reference, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            if ui
                                .add_enabled(
                                    layer.transform.is_distorted(),
                                    Button::new(tr!("RESET_DISTORTION")),
                                )
                                .clicked()
                            {
                                layer.transform.distort = Default::default();
                                out.changed = true;
                            }
                            if ui
                                .add_enabled(
                                    layer.transform.is_tilted(),
                                    Button::new(tr!("RESET_TILT")),
                                )
                                .clicked()
                            {
                                layer.transform.tilt_x_deg = 0.0;
                                layer.transform.tilt_y_deg = 0.0;
                                layer.transform.perspective = crate::document::DEFAULT_PERSPECTIVE;
                                out.changed = true;
                            }
                        });
                    });

                    if layer.text.is_some() {
                        out.changed |= text_section(ui, layer, state, fonts, &shapes);
                    } else if layer.shape.is_some() {
                        // Without a shortcut, the command's name.
                        let delete = state
                            .keys
                            .get(&Command::Delete)
                            .cloned()
                            .unwrap_or_else(|| tr!("DELETE_LAYER").to_owned());
                        out.changed |= shape_section(
                            ui,
                            layer,
                            locked,
                            &mut state.edit_points,
                            &mut state.pick_pattern,
                            &delete,
                            plain,
                        );
                    } else {
                        image_section(ui, layer, state.min_dpi);
                    }
                },
            );
            // Effect sizes of drawn layers are shown in mm: a text's follow its font
            // size, a shape's stay the same when it is resized.
            let px_mm = if layer.text.is_some() {
                Some((layer.transform.scale[1].abs(), tr!("TEXT_EFFECT_SIZE_HINT")))
            } else if layer.shape.is_some() {
                Some((1.0 / shape::UNITS_PER_MM, tr!("SHAPE_EFFECT_SIZE_HINT")))
            } else {
                None
            };
            if !plain {
                out.changed |= effects_ui::show(ui, layer.id, &mut layer.effects, px_mm);
            }
        }
        None => {}
    }
    if let Some(horizontally) = group_flip {
        out.changed |= flip_group(doc, id, horizontally);
    }
    if let Some(deg) = group_turn {
        out.changed |= rotate_group(doc, id, deg);
    }
}
