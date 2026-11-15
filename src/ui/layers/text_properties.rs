//! The text settings in Properties: what the text says, its font and look, and its path.

use super::*;

/// The part of Properties for a text layer: what it says and how it looks. Returns true
/// if it changed.
pub(super) fn text_section(
    ui: &mut Ui,
    layer: &mut Layer,
    state: &mut LayersPanel,
    fonts: &FontLibrary,
    shapes: &[(u64, String)],
) -> bool {
    let mut changed = false;
    // A new text layer opens the section, so its settings show while it is typed.
    let open = state.show_text;
    let text_scale = layer.transform.scale[1].abs();
    section(
        ui,
        "text_props",
        RichText::new(tr!("TEXT_SECTION")),
        move |_| open,
        |ui| {
            let Some(t) = layer.text.as_deref_mut() else {
                return;
            };
            // The name follows the text until it is renamed.
            let auto = layer.name == text::auto_name(&t.content);
            let id = ui.make_persistent_id("text_content");
            // The text box, then its link button to a text variable at its top right.
            let r = ui
                .horizontal_top(|ui| {
                    let side = ui.spacing().interact_size.y;
                    let width = ui.available_width() - side - ui.spacing().item_spacing.x;
                    let r = ui
                        .add(
                            egui::TextEdit::multiline(&mut t.content)
                                .id(id)
                                .desired_rows(3)
                                .desired_width(width)
                                .hint_text(tr!("TEXT_PLACEHOLDER")),
                        )
                        .on_hover_text(if t.path.kind == PathKind::Straight {
                            tr!("TYPE_ON_CANVAS_HINT")
                        } else {
                            tr!("TEXT_ON_PATH_TYPE_HINT")
                        });
                    if crate::ui::text_links::link_button(ui, &mut t.link) {
                        changed = true;
                    }
                    r
                })
                .inner;
            // Typed in, the text is no longer its variable's.
            if r.changed() {
                crate::ui::text_links::typed_in(ui.ctx(), &mut t.link);
            }
            if std::mem::take(&mut state.show_text) {
                r.scroll_to_me(Some(egui::Align::Center));
            }
            if r.changed() {
                changed = true;
                if auto {
                    layer.name = text::auto_name(&t.content);
                }
            }
            ui.add_space(4.0);

            let mut font_changed = false;
            prop_grid(ui, "text_props").show(ui, |ui| {
                ui.label(tr!("TEXT_FONT"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    font_changed |= font_menu(ui, &mut t.family, state, fonts);
                    let missing = layer.broken;
                    if missing {
                        ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                            .on_hover_text(tr!("TEXT_FONT_NOT_INSTALLED"));
                    }
                    let chars = &layer.text_state.missing;
                    if !chars.is_empty() {
                        let chars: Vec<String> = chars.iter().map(char::to_string).collect();
                        ui.colored_label(ui.visuals().warn_fg_color, "⚠")
                            .on_hover_text(tr!("TEXT_MISSING_CHARACTERS", chars = chars.join(" ")));
                    }
                });
                ui.end_row();

                // Fonts the characters this one lacks come from.
                if !t.fallbacks.is_empty() {
                    ui.label(tr!("TEXT_FALLBACK_FONTS"))
                        .on_hover_text(tr!("TEXT_FALLBACK_FONTS_HINT"));
                    let names: Vec<&str> = t.fallbacks.iter().map(|f| f.family.as_str()).collect();
                    ui.add(egui::Label::new(RichText::new(names.join(", ")).weak()).wrap())
                        .on_hover_text(tr!("TEXT_FALLBACK_FONTS_HINT"));
                    ui.end_row();
                }

                ui.label(tr!("TEXT_STYLE"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if style_toggle(ui, t.bold, StyleIcon::Bold, tr!("TEXT_BOLD")).clicked() {
                        t.bold ^= true;
                        font_changed = true;
                    }
                    let italic = style_toggle(ui, t.italic, StyleIcon::Italic, tr!("TEXT_ITALIC"));
                    if italic.clicked() {
                        t.italic ^= true;
                        font_changed = true;
                    }
                    ui.add_space(8.0);
                    for (a, tip) in [
                        (TextAlign::Left, tr!("TEXT_ALIGN_LEFT")),
                        (TextAlign::Center, tr!("TEXT_ALIGN_CENTER")),
                        (TextAlign::Right, tr!("TEXT_ALIGN_RIGHT")),
                    ] {
                        if style_toggle(ui, t.align == a, StyleIcon::Align(a), tip).clicked() {
                            t.align = a;
                            changed = true;
                        }
                    }
                });
                ui.end_row();

                ui.label(tr!("TEXT_LETTER_SPACING"));
                changed |= ui
                    .add(
                        DragValue::new(&mut t.letter_spacing)
                            .range(-500.0..=2000.0)
                            .speed(2.0)
                            .fixed_decimals(0),
                    )
                    .on_hover_text(tr!("TEXT_LETTER_SPACING_HINT"))
                    .changed();
                ui.end_row();

                ui.label(tr!("TEXT_LINE_SPACING"));
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(
                            DragValue::new(&mut t.line_height)
                                .range(0.5..=5.0)
                                .speed(0.01)
                                .fixed_decimals(2),
                        )
                        .on_hover_text(tr!("TEXT_LINE_SPACING_HINT"))
                        .changed();
                    unit(ui, "x");
                });
                ui.end_row();

                ui.label(tr!("TEXT_COLOR"));
                let mut link = layer.color_links.remove(&ColorField::Text);
                let mut fill = crate::ui::gradient::Fill {
                    color: &mut t.color,
                    link: &mut link,
                    gradient: &mut t.gradient,
                    pattern: Some(&mut t.pattern),
                    // The text's units are its scale's mm.
                    units_per_mm: 1.0 / text_scale.max(1e-6),
                    alpha: true,
                    pick_pattern: false,
                };
                ui.horizontal(|ui| {
                    changed |= crate::ui::gradient::inline(ui, ("text", layer.id), &mut fill);
                });
                ui.end_row();
                if fill.has_rows() {
                    ui.label("");
                    changed |= crate::ui::gradient::rows(ui, ("text", layer.id), &mut fill);
                    ui.end_row();
                }
                if fill.pick_pattern {
                    state.pick_pattern = Some(layer.id);
                }
                if let Some(name) = link {
                    layer.color_links.insert(ColorField::Text, name);
                }
            });
            if font_changed {
                text::font_changed(layer);
                changed = true;
            }

            // Size and outline width are in mm on the page, so they need the whole layer.
            // Same gap as between the rows above.
            ui.add_space(PROP_ROW_GAP - ui.spacing().item_spacing.y);
            prop_grid(ui, "text_size").show(ui, |ui| {
                ui.label(tr!("TEXT_FONT_SIZE"));
                let mut pt = text::size_pt(layer);
                ui.horizontal(|ui| {
                    if variables_ui::field(ui, &mut layer.bindings, Field::TextSize, &mut pt, |d| {
                        d.range(1.0..=2000.0).speed(0.2).fixed_decimals(1)
                    })
                    .on_hover_text(tr!("TEXT_FONT_SIZE_HINT"))
                    .changed()
                    {
                        text::set_size_pt(layer, pt);
                        changed = true;
                    }
                    unit(ui, "pt");
                });
                ui.end_row();

                ui.label(tr!("TEXT_OUTLINE"));
                // The outline's thickness is kept in ems, shown in mm.
                let em_mm = text::em_to_mm(layer, 1.0).max(1e-6);
                let Some(t) = layer.text.as_deref_mut() else {
                    return;
                };
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut t.outline, "").changed();
                    ui.add_enabled_ui(t.outline, |ui| {
                        let mut mm = t.outline_width * em_mm;
                        if ui
                            .add(
                                DragValue::new(&mut mm)
                                    .range(0.01..=50.0)
                                    .speed(0.02)
                                    .fixed_decimals(2),
                            )
                            .on_hover_text(tr!("TEXT_OUTLINE_HINT"))
                            .changed()
                        {
                            t.outline_width = mm / em_mm;
                            changed = true;
                        }
                        unit(ui, "mm");
                        changed |= crate::ui::colors::linked_button(
                            ui,
                            &mut layer.color_links,
                            ColorField::TextOutline,
                            &mut t.outline_color,
                            true,
                        )
                        .changed();
                    });
                });
                ui.end_row();

                changed |= text_path_rows(ui, t, em_mm, layer.size_px, shapes);
            });
        },
    );
    changed
}

/// The rows of a text's path: what it follows and how the text sits on it. `em_mm` is
/// the length of an em on the page, `size_px` the layer's box. Returns true if it
/// changed.
pub(super) fn text_path_rows(
    ui: &mut Ui,
    t: &mut text::Text,
    em_mm: f32,
    size_px: [u32; 2],
    shapes: &[(u64, String)],
) -> bool {
    let mut changed = false;
    let p = &mut t.path;
    ui.label(tr!("TEXT_PATH"));
    let mut kind = p.kind;
    if choice(
        ui,
        "text_path",
        &mut kind,
        &PathKind::ALL,
        PathKind::name_key,
    ) {
        // A circle about three times as long as the text; the first shape to follow.
        if kind == PathKind::Circle && p.kind != PathKind::Circle {
            let width = size_px[0] as f32 / text::EM;
            p.radius = (width * 3.0 / std::f32::consts::TAU).max(1.0);
        }
        if kind == PathKind::Shape && !shapes.iter().any(|(id, _)| *id == p.shape) {
            p.shape = shapes.first().map_or(0, |(id, _)| *id);
        }
        p.kind = kind;
        changed = true;
    }
    ui.end_row();

    // A length kept in ems, shown in mm.
    let mm_value = |ui: &mut Ui, ems: &mut f32, range, hint: &str| {
        let mut mm = *ems * em_mm;
        let r = ui
            .horizontal(|ui| {
                let r = ui.add(
                    DragValue::new(&mut mm)
                        .range(range)
                        .speed(0.1)
                        .fixed_decimals(1),
                );
                unit(ui, "mm");
                r
            })
            .inner
            .on_hover_text(hint);
        if r.changed() {
            *ems = mm / em_mm;
            true
        } else {
            false
        }
    };
    match p.kind {
        PathKind::Straight => return changed,
        PathKind::Arc => {
            ui.label(tr!("TEXT_PATH_BEND"));
            ui.horizontal(|ui| {
                changed |= ui
                    .add(Slider::new(&mut p.bend, -100.0..=100.0).fixed_decimals(0))
                    .on_hover_text(tr!("TEXT_PATH_BEND_HINT"))
                    .changed();
                unit(ui, "%");
            });
            ui.end_row();
        }
        PathKind::Circle => {
            ui.label(tr!("TEXT_PATH_RADIUS"));
            changed |= mm_value(
                ui,
                &mut p.radius,
                0.5..=1000.0,
                tr!("TEXT_PATH_RADIUS_HINT"),
            );
            ui.end_row();
        }
        PathKind::Shape => {
            ui.label(tr!("TEXT_PATH_SHAPE_LAYER"));
            if shapes.is_empty() {
                crate::ui::callout::callout_small(
                    ui,
                    crate::ui::callout::Tone::Info,
                    tr!("TEXT_PATH_NO_SHAPES"),
                );
            } else {
                let name = shapes
                    .iter()
                    .find(|(id, _)| *id == p.shape)
                    .map_or(tr!("TEXT_PATH_PICK_SHAPE"), |(_, n)| n.as_str());
                egui::ComboBox::from_id_salt("text_path_shape")
                    .width(130.0)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for (id, n) in shapes {
                            changed |= ui.selectable_value(&mut p.shape, *id, n).changed();
                        }
                    });
            }
            ui.end_row();
        }
    }
    if matches!(p.kind, PathKind::Circle | PathKind::Shape) {
        ui.label(tr!("TEXT_PATH_POSITION"));
        ui.horizontal(|ui| {
            changed |= ui
                .add(Slider::new(&mut p.position, 0.0..=100.0).fixed_decimals(1))
                .on_hover_text(tr!("TEXT_PATH_POSITION_HINT"))
                .changed();
            unit(ui, "%");
        });
        ui.end_row();

        ui.label(tr!("TEXT_PATH_FLIP"));
        changed |= ui
            .checkbox(&mut p.flip, "")
            .on_hover_text(tr!("TEXT_PATH_FLIP_HINT"))
            .changed();
        ui.end_row();
    }
    ui.label(tr!("TEXT_PATH_SHIFT"));
    changed |= mm_value(
        ui,
        &mut p.shift,
        -1000.0..=1000.0,
        tr!("TEXT_PATH_SHIFT_HINT"),
    );
    ui.end_row();
    changed
}

/// The font family list, with a search field. Returns true if another was picked.
pub(super) fn font_menu(
    ui: &mut Ui,
    family: &mut String,
    state: &mut LayersPanel,
    fonts: &FontLibrary,
) -> bool {
    let mut picked = None;
    egui::ComboBox::from_id_salt("text_font")
        .width(170.0)
        .selected_text(family.as_str())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut state.font_filter)
                    .hint_text(tr!("SEARCH_FONTS"))
                    .desired_width(f32::INFINITY),
            );
            if !search.has_focus() && state.font_filter.is_empty() {
                search.request_focus();
            }
            let filter = state.font_filter.to_lowercase();
            let list = fonts.with_families(|all| {
                all.iter()
                    .filter(|f| f.to_lowercase().contains(&filter))
                    .cloned()
                    .collect::<Vec<_>>()
            });
            let Some(list) = list else {
                ui.weak(tr!("LOADING_FONTS"));
                ui.ctx().request_repaint();
                return;
            };
            let row = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical().max_height(300.0).show_rows(
                ui,
                row,
                list.len(),
                |ui, range| {
                    for f in &list[range] {
                        if ui.selectable_label(f == family, f.as_str()).clicked() {
                            picked = Some(f.clone());
                            ui.close();
                        }
                    }
                },
            );
        });
    match picked {
        Some(f) if f != *family => {
            *family = f;
            true
        }
        _ => false,
    }
}
