//! The shape settings in Properties: kind, size, fill and stroke.

use super::*;

/// The Size row of a shape layer: the size of its geometry, stroke left out. Returns
/// true if it changed.
pub(super) fn shape_size(ui: &mut Ui, layer: &mut Layer, state: &mut LayersPanel) -> bool {
    let Some(old) = layer.shape.as_ref().map(|s| s.geometry_size()) else {
        return false;
    };
    let [mut w, mut h] = old;
    let b = &mut layer.bindings;
    let links = [Field::Width, Field::Height].map(|f| b.get(&f).cloned());
    let rw = prefixed(ui, tr!("LAYER_WIDTH_PREFIX"), |ui| {
        variables_ui::field(ui, b, Field::Width, &mut w, |d| mm_value(d, 0.0))
    });
    let rh = prefixed(ui, tr!("LAYER_HEIGHT_PREFIX"), |ui| {
        variables_ui::field(ui, b, Field::Height, &mut h, |d| mm_value(d, 0.0))
    });
    unit(ui, "mm");
    aspect_lock(ui, &mut state.free_aspect);
    if !state.free_aspect {
        variables::link_aspect(b, links, old, &variables_ui::published(ui.ctx()));
    }
    let mut size = old;
    if rw.changed() {
        size[0] = w;
        if !state.free_aspect && old[0] > 1e-6 {
            size[1] = old[1] * w / old[0];
        }
    } else if rh.changed() {
        size[1] = h;
        if !state.free_aspect && old[1] > 1e-6 {
            size[0] = old[0] * h / old[1];
        }
    } else {
        return false;
    }
    shape::set_geometry_size(layer, size);
    true
}

/// The part of Properties for a shape layer: its kind, fill and stroke. Returns true if
/// it changed. `delete_key` is the Delete command's shortcut, named in a tooltip.
pub(super) fn shape_section(
    ui: &mut Ui,
    layer: &mut Layer,
    locked: bool,
    edit_points: &mut Option<u64>,
    pick_pattern: &mut Option<u64>,
    delete_key: &str,
    plain: bool,
) -> bool {
    let id = layer.id;
    let mut changed = false;
    section(
        ui,
        "shape_props",
        RichText::new(tr!("SHAPE_SECTION")),
        |_| false,
        |ui| {
            let Some(s) = layer.shape.as_deref_mut() else {
                return;
            };
            prop_grid(ui, "shape_props").show(ui, |ui| {
                // A shape keeps the kind it was made as: shown, not offered.
                ui.label(tr!("SHAPE_KIND"));
                ui.label(translate(s.kind.name_key()));
                ui.end_row();

                match s.kind {
                    ShapeKind::Rectangle => {
                        ui.label(tr!("SHAPE_CORNER_RADIUS"));
                        let max = s.size[0].min(s.size[1]) / 2.0;
                        ui.horizontal(|ui| {
                            changed |= variables_ui::field(
                                ui,
                                &mut layer.bindings,
                                Field::CornerRadius,
                                &mut s.corner_radius,
                                |d| d.range(0.0..=max).speed(0.1).fixed_decimals(1),
                            )
                            .changed();
                            unit(ui, "mm");
                        });
                        ui.end_row();
                    }
                    ShapeKind::RegularPolygon | ShapeKind::Star => {
                        let star = s.kind == ShapeKind::Star;
                        ui.label(if star {
                            tr!("SHAPE_POINTS")
                        } else {
                            tr!("SHAPE_SIDES")
                        });
                        changed |= ui
                            .add(DragValue::new(&mut s.sides).range(3..=100).speed(0.1))
                            .changed();
                        ui.end_row();
                        if star {
                            ui.label(tr!("SHAPE_INNER_RADIUS"));
                            let mut pct = s.inner_ratio * 100.0;
                            ui.horizontal(|ui| {
                                if ui
                                    .add(Slider::new(&mut pct, 1.0..=100.0).fixed_decimals(0))
                                    .on_hover_text(tr!("SHAPE_INNER_RADIUS_HINT"))
                                    .changed()
                                {
                                    s.inner_ratio = pct / 100.0;
                                    changed = true;
                                }
                                unit(ui, "%");
                            });
                            ui.end_row();
                        }
                    }
                    k if k.has_points() => {
                        // Shows what the canvas does now: a double click there turns it on
                        // as well.
                        ui.label(tr!("SHAPE_POINT_COUNT"));
                        ui.horizontal(|ui| {
                            ui.label(s.points.len().to_string());
                            let on = *edit_points == Some(id);
                            let r = ui
                                .add_enabled(
                                    !locked,
                                    egui::Button::selectable(on, tr!("EDIT_POINTS")),
                                )
                                .on_hover_text(tr!("EDIT_POINTS_HINT", delete = delete_key));
                            if r.clicked() {
                                *edit_points = if on { None } else { Some(id) };
                            }
                        });
                        ui.end_row();
                        if k == ShapeKind::Path {
                            ui.label(tr!("SHAPE_FILL_RULE"));
                            changed |= choice(
                                ui,
                                "shape_fill_rule",
                                &mut s.even_odd,
                                &[false, true],
                                |even_odd| {
                                    if even_odd {
                                        tr_noop!("FILL_RULE_EVEN_ODD")
                                    } else {
                                        tr_noop!("FILL_RULE_NONZERO")
                                    }
                                },
                            );
                            ui.end_row();
                        }
                    }
                    _ => {}
                }

                // A template's shapes are lines of their element's color.
                if plain {
                    return;
                }
                if s.kind.can_fill() {
                    ui.label(tr!("SHAPE_FILL"));
                    let mut link = layer.color_links.remove(&ColorField::Fill);
                    let mut fill = crate::ui::gradient::Fill {
                        color: &mut s.fill_color,
                        link: &mut link,
                        gradient: &mut s.fill_gradient,
                        pattern: Some(&mut s.fill_pattern),
                        units_per_mm: 1.0,
                        alpha: true,
                        pick_pattern: false,
                    };
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut s.fill, "").changed();
                        ui.add_enabled_ui(s.fill, |ui| {
                            changed |= crate::ui::gradient::inline(ui, ("fill", id), &mut fill);
                        });
                    });
                    ui.end_row();
                    if fill.has_rows() {
                        ui.label("");
                        ui.add_enabled_ui(s.fill, |ui| {
                            changed |= crate::ui::gradient::rows(ui, ("fill", id), &mut fill);
                        });
                        ui.end_row();
                    }
                    if fill.pick_pattern {
                        *pick_pattern = Some(id);
                    }
                    if let Some(name) = link {
                        layer.color_links.insert(ColorField::Fill, name);
                    }

                    // What is painted in it, when it is resized.
                    ui.label(tr!("SHAPE_STRETCH_PAINT"));
                    changed |= ui
                        .checkbox(&mut s.stretch_paint, "")
                        .on_hover_text(tr!("SHAPE_STRETCH_PAINT_HINT"))
                        .changed();
                    ui.end_row();
                }

                ui.label(tr!("SHAPE_STROKE"));
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut s.stroke, "").changed();
                    ui.add_enabled_ui(s.stroke, |ui| {
                        changed |= variables_ui::field(
                            ui,
                            &mut layer.bindings,
                            Field::StrokeWidth,
                            &mut s.stroke_width,
                            |d| d.range(0.01..=100.0).speed(0.02).fixed_decimals(2),
                        )
                        .on_hover_text(tr!("SHAPE_STROKE_WIDTH_HINT"))
                        .changed();
                        unit(ui, "mm");
                    });
                });
                ui.end_row();
                // Its color or gradient on a row of its own, as for the fill.
                ui.add_enabled_ui(s.stroke, |ui| ui.label(tr!("SHAPE_STROKE_COLOR")));
                let mut link = layer.color_links.remove(&ColorField::Stroke);
                let mut stroke = crate::ui::gradient::Fill {
                    color: &mut s.stroke_color,
                    link: &mut link,
                    gradient: &mut s.stroke_gradient,
                    pattern: None,
                    units_per_mm: 1.0,
                    alpha: true,
                    pick_pattern: false,
                };
                ui.add_enabled_ui(s.stroke, |ui| {
                    ui.horizontal(|ui| {
                        changed |= crate::ui::gradient::inline(ui, ("stroke", id), &mut stroke);
                    });
                });
                ui.end_row();
                if stroke.gradient.is_some() {
                    ui.label("");
                    ui.add_enabled_ui(s.stroke, |ui| {
                        changed |= crate::ui::gradient::rows(ui, ("stroke", id), &mut stroke);
                    });
                    ui.end_row();
                }
                if let Some(name) = link {
                    layer.color_links.insert(ColorField::Stroke, name);
                }

                ui.add_enabled_ui(s.stroke, |ui| ui.label(tr!("STROKE_DASHES")));
                ui.add_enabled_ui(s.stroke, |ui| {
                    changed |= choice(
                        ui,
                        "stroke_dash",
                        &mut s.dash,
                        &DashStyle::ALL,
                        DashStyle::name_key,
                    );
                });
                ui.end_row();

                // Caps show at the ends of open lines and of dashes.
                let caps = s.stroke && (s.has_open_ends() || s.dash != DashStyle::Solid);
                ui.add_enabled_ui(caps, |ui| ui.label(tr!("STROKE_CAPS")));
                ui.add_enabled_ui(caps, |ui| {
                    changed |= choice(ui, "stroke_cap", &mut s.cap, &Cap::ALL, Cap::name_key);
                });
                ui.end_row();

                ui.add_enabled_ui(s.stroke, |ui| ui.label(tr!("STROKE_JOINS")));
                ui.add_enabled_ui(s.stroke, |ui| {
                    changed |= choice(ui, "stroke_join", &mut s.join, &Join::ALL, Join::name_key);
                });
                ui.end_row();
            });
        },
    );
    changed
}
