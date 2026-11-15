//! The Page and View panels, panel contents and the status bar.

use super::*;

impl ArtworkApp {
    /// The Page panel: size, orientation and background.
    pub(super) fn page_panel(&mut self, ui: &mut egui::Ui) {
        let before = (self.doc.page, self.doc.orientation);
        let custom = self.doc.custom_page_mm;
        egui::ComboBox::from_id_salt("page")
            .width(ui.available_width())
            .selected_text(self.doc.page.label(custom))
            .show_ui(ui, |ui| {
                for p in PageSize::ALL {
                    ui.selectable_value(&mut self.doc.page, p, p.label(custom));
                }
            });
        if self.doc.page == PageSize::Custom {
            if before.0 != PageSize::Custom
                && let Some(size) = before.0.portrait_mm()
            {
                // Starts from the size picked before, to adjust from there.
                self.doc.custom_page_mm = size;
            }
            // Width and height as the page shows them, orientation applied.
            let [mut w, mut h] = self.doc.page_mm();
            let changed = ui
                .horizontal(|ui| {
                    let field = |ui: &mut egui::Ui, v: &mut f32, prefix: &str| {
                        ui.add(
                            egui::DragValue::new(v)
                                .range(document::CUSTOM_PAGE_RANGE_MM)
                                .speed(1.0)
                                .fixed_decimals(1)
                                .prefix(prefix),
                        )
                        .changed()
                    };
                    let a = field(ui, &mut w, tr!("PAGE_CUSTOM_WIDTH"));
                    let b = field(ui, &mut h, tr!("PAGE_CUSTOM_HEIGHT"));
                    crate::ui::layers::unit(ui, "mm");
                    a || b
                })
                .inner;
            if changed {
                self.doc.custom_page_mm = match self.doc.orientation {
                    Orientation::Portrait => [w, h],
                    Orientation::Landscape => [h, w],
                };
                self.canvas.fit_pending = true;
                self.dirty = true;
            }
        }
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut self.doc.orientation,
                Orientation::Landscape,
                tr!("PAGE_LANDSCAPE"),
            );
            ui.radio_value(
                &mut self.doc.orientation,
                Orientation::Portrait,
                tr!("PAGE_PORTRAIT"),
            );
        });
        if before != (self.doc.page, self.doc.orientation) {
            self.canvas.fit_pending = true;
            self.dirty = true;
        }
        // One color, a gradient over the whole page or a pattern; opaque, the page being
        // so.
        let doc = &mut self.doc;
        let [r, g, b] = doc.background;
        let mut color = [r, g, b, 255];
        let mut fill = crate::ui::gradient::Fill {
            color: &mut color,
            link: &mut doc.background_link,
            gradient: &mut doc.background_gradient,
            pattern: Some(&mut doc.background_pattern),
            units_per_mm: 1.0,
            alpha: false,
            pick_pattern: false,
        };
        let mut changed = false;
        panel_grid(ui, "page-background").show(ui, |ui| {
            ui.label(tr!("PAGE_BACKGROUND"));
            ui.horizontal(|ui| {
                changed |= crate::ui::gradient::inline(ui, "background", &mut fill);
            });
            ui.end_row();
        });
        changed |= crate::ui::gradient::rows(ui, "background", &mut fill);
        let pick = fill.pick_pattern;
        doc.background = [color[0], color[1], color[2]];
        self.dirty |= changed;
        if pick {
            self.pick_pattern(ui.ctx(), crate::document::pattern::Target::Background);
        }
    }

    /// The View panel: how the canvas shows the template and what is cut out of the
    /// panel, what layers snap to, and the rulers and guides. Kept with the app's
    /// preferences, not the project.
    pub(super) fn view_panel(&mut self, ui: &mut egui::Ui) {
        // Neither the template nor the export changes with the previews. The slider sets
        // how much the cut preview covers.
        let keys = self.shortcuts.texts(ui.ctx());
        let tip =
            |hint: &str, command: Command| crate::ui::shortcuts::with_key(hint, keys.get(&command));
        ui.checkbox(&mut self.canvas.show_template, tr!("SHOW_TEMPLATE_LINES"))
            .on_hover_text(tip(tr!("SHOW_TEMPLATE_LINES_HINT"), Command::ShowTemplate));
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.canvas.preview_cuts, tr!("PREVIEW_CUTS"))
                .on_hover_text(tip(tr!("PREVIEW_CUTS_HINT"), Command::PreviewCuts));
            // In percent, the value shown only in its tooltip.
            let mut percent = (self.canvas.cut_opacity * 100.0).round();
            let r = ui
                .add_enabled(
                    self.canvas.preview_cuts,
                    egui::Slider::new(&mut percent, 0.0..=100.0)
                        .step_by(1.0)
                        .show_value(false),
                )
                .on_hover_text(tr!(
                    "CUT_PREVIEW_OPACITY_HINT",
                    percent = format!("{percent:.0}")
                ));
            if r.changed() {
                self.canvas.cut_opacity = percent / 100.0;
            }
        });
        ui.add_enabled(
            self.canvas.preview_cuts,
            egui::Checkbox::new(&mut self.canvas.preview_bleed, tr!("PREVIEW_BLEED")),
        )
        .on_hover_text(tip(tr!("PREVIEW_BLEED_HINT"), Command::PreviewBleed));
        // A guide inside the cut lines, never printed, and how far inside: kept with the
        // app's preferences, like the previews above.
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.canvas.preview_safe_area, tr!("PREVIEW_SAFE_AREA"))
                .on_hover_text(tip(tr!("PREVIEW_SAFE_AREA_HINT"), Command::PreviewSafeArea));
            ui.add_enabled(
                self.canvas.preview_safe_area,
                egui::DragValue::new(&mut self.canvas.safe_margin_mm)
                    .range(0.0..=crate::export::marks::MAX_MARGIN_MM)
                    .speed(0.1)
                    .fixed_decimals(1),
            )
            .on_hover_text(tr!("SAFE_MARGIN_HINT"));
            crate::ui::layers::unit(ui, "mm");
        });

        // What layers snap to while dragged.
        let has_centers = self.has_centers();
        let canvas = &mut self.canvas;
        crate::ui::layers::section(
            ui,
            "page_snapping",
            egui::RichText::new(tr!("SNAPPING_SECTION")),
            |_| false,
            |ui| {
                ui.add_enabled(
                    has_centers,
                    egui::Checkbox::new(&mut canvas.snap_to_centers, tr!("SNAP_TO_CENTERS")),
                )
                .on_hover_text(tip(tr!("SNAP_TO_CENTERS_HINT"), Command::SnapToCenters))
                .on_disabled_hover_text(tr!("TEMPLATE_HAS_NO_CENTERS"));
                ui.checkbox(&mut canvas.snap_to_edges, tr!("SNAP_TO_EDGES"))
                    .on_hover_text(tip(tr!("SNAP_TO_EDGES_HINT"), Command::SnapToEdges));
                ui.checkbox(&mut canvas.snap_to_guides, tr!("SNAP_TO_GUIDES"))
                    .on_hover_text(tip(tr!("SNAP_TO_GUIDES_HINT"), Command::SnapToGuides));
            },
        );

        // The rulers, and the guides dragged out of them.
        let doc = &mut self.doc;
        let mut cleared = false;
        crate::ui::layers::section(
            ui,
            "page_rulers",
            egui::RichText::new(tr!("RULERS_AND_GUIDES_SECTION")),
            |_| false,
            |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut canvas.show_rulers, tr!("SHOW_RULERS"))
                        .on_hover_text(tip(tr!("SHOW_RULERS_HINT"), Command::ShowRulers));
                    ui.checkbox(&mut canvas.show_guides, tr!("SHOW_GUIDES"))
                        .on_hover_text(tip(tr!("SHOW_GUIDES_HINT"), Command::ShowGuides));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut canvas.lock_guides, tr!("LOCK_GUIDES"))
                        .on_hover_text(tip(tr!("LOCK_GUIDES_HINT"), Command::LockGuides));
                    if ui
                        .add_enabled(
                            !doc.guides.is_empty(),
                            egui::Button::new(tr!("CLEAR_GUIDES")),
                        )
                        .on_hover_text(tip(tr!("CLEAR_GUIDES_HINT"), Command::ClearGuides))
                        .clicked()
                    {
                        cleared = canvas::clear_guides(doc);
                    }
                });
            },
        );
        self.dirty |= cleared;
    }

    /// A panel's contents; the Layers panel returns the height it would take.
    pub(super) fn panel_body(&mut self, ui: &mut egui::Ui, id: PanelId) -> Option<f32> {
        match id {
            PanelId::TemplateSettings => self.template_settings_panel(ui),
            PanelId::TemplateCheck => self.template_check_panel(ui),
            PanelId::Template => self.template_panel(ui),
            PanelId::Page => self.page_panel(ui),
            PanelId::View => self.view_panel(ui),
            PanelId::Print => self.print_panel(ui),
            PanelId::Removed => {}
            PanelId::Variables => {
                // A template's shapes take lengths, angles and numbers alone.
                let kinds: &[variables::Kind] = if self.editing.is_some() {
                    &variables::TEMPLATE_KINDS
                } else {
                    &variables::Kind::ALL
                };
                self.dirty |= variables_ui::variables_ui(ui, &mut self.doc, kinds)
            }
            PanelId::ColorsInUse => {
                self.dirty |= crate::ui::colors::in_use_ui(ui, &mut self.doc, &self.canvas.picked)
            }
            PanelId::Layers => {
                self.layers_panel.can_paste = self.clipboard.has_content();
                let out = layers::show(
                    ui,
                    &mut self.doc,
                    &mut self.selected,
                    &mut self.layers_panel,
                );
                let natural = out.natural_height;
                self.layers_output(ui.ctx(), out);
                return Some(natural);
            }
            PanelId::Properties => {
                let bounds = Self::outline_bounds(&self.elements());
                let out = layers::properties_panel(
                    ui,
                    &mut self.doc,
                    self.selected,
                    &mut self.layers_panel,
                    bounds,
                    &self.fonts,
                );
                self.layers_output(ui.ctx(), out);
                if let Some(id) = self.layers_panel.pick_pattern.take() {
                    self.pick_pattern(ui.ctx(), crate::document::pattern::Target::Layer(id));
                }
            }
        }
        None
    }

    /// Carries out what was asked in the Layers or Properties panel.
    pub(super) fn layers_output(&mut self, ctx: &egui::Context, out: layers::LayersOutput) {
        self.dirty |= out.changed;
        if out.add_images {
            self.pick_images(ctx);
        }
        if out.add_text {
            self.add_text(ctx);
        }
        if let Some(kind) = out.add_canvas {
            self.add_canvas(ctx, kind);
        }
        if out.add_background {
            self.add_background();
        }
        if let Some(kind) = out.add_shape {
            self.add_shape(ctx, kind);
        }
        if let Some(edit) = out.edit {
            self.edit(ctx, edit);
        }
        if let Some(kind) = out.add_element {
            let id = crate::template::edit::add_element(&mut self.doc, kind);
            self.selected = Some(id);
            self.dirty = true;
        }
        if out.choose_reference {
            self.pick_template_reference(ctx);
        }
    }

    /// Bottom bar: zoom, page size, cursor position and warnings.
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui, elements: &[PlacedElement]) {
        ui.horizontal(|ui| {
            if ui
                .small_button("−")
                .on_hover_text(tr!("ZOOM_OUT"))
                .clicked()
            {
                self.canvas.zoom_centered(1.0 / 1.25);
            }
            ui.label(format!(
                "{:.0}%",
                self.canvas.camera.zoom / REAL_SIZE_ZOOM * 100.0
            ));
            if ui.small_button("+").on_hover_text(tr!("ZOOM_IN")).clicked() {
                self.canvas.zoom_centered(1.25);
            }
            if ui
                .small_button(tr!("FIT_PAGE_IN_VIEW_SHORT"))
                .on_hover_text(crate::ui::shortcuts::with_key(
                    tr!("FIT_PAGE_IN_VIEW"),
                    self.shortcuts.text(ui.ctx(), Command::FitPage).as_ref(),
                ))
                .clicked()
            {
                self.canvas.fit_pending = true;
            }
            if ui
                .small_button("100%")
                .on_hover_text(crate::ui::shortcuts::with_key(
                    tr!("ZOOM_REAL_SIZE_HINT"),
                    self.shortcuts.text(ui.ctx(), Command::RealSize).as_ref(),
                ))
                .clicked()
            {
                self.canvas.set_zoom(REAL_SIZE_ZOOM);
            }
            ui.separator();
            let [pw, ph] = self.doc.page_mm();
            ui.label(tr!(
                "PAGE_SIZE",
                w = format!("{pw:.0}"),
                h = format!("{ph:.0}")
            ));
            if let Some([x, y]) = self.canvas.cursor_mm {
                ui.separator();
                ui.label(format!("x {x:.1}  y {y:.1} mm"));
            }
            if let Some(d) = &self.canvas.drawing {
                ui.separator();
                ui.colored_label(
                    ui.visuals().selection.stroke.color,
                    match d.kind {
                        ShapeKind::Line => tr!("DRAW_LINE_HINT"),
                        _ => tr!("DRAW_POINTS_HINT"),
                    },
                );
            }
            if let Some((c, s)) = Self::outline_bounds(elements) {
                let off = c[0] - s[0] / 2.0 < -0.01
                    || c[1] - s[1] / 2.0 < -0.01
                    || c[0] + s[0] / 2.0 > pw + 0.01
                    || c[1] + s[1] / 2.0 > ph + 0.01;
                if off {
                    ui.separator();
                    ui.colored_label(ui.visuals().warn_fg_color, tr!("TEMPLATE_DOES_NOT_FIT"));
                }
            }
            if self.export_job.is_some() {
                ui.separator();
                ui.spinner();
                ui.label(tr!("EXPORT_IN_PROGRESS"));
            }
            if let Some(text) = notice(ui.ctx()) {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, text);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.save_status(ui)
            });
        });
    }

    /// At the right of the status bar: when the project was last saved and, with
    /// unsaved changes, when they were last autosaved. Kept up to date as time passes.
    pub(super) fn save_status(&self, ui: &mut egui::Ui) {
        // While a template is edited: when it was saved, its folder in the tooltip.
        if let Some(editing) = &self.editing {
            let saved = match editing.last_saved {
                Some(t) => tr!("LAST_SAVED", age = i18n::time_ago(t)),
                None => tr!("NOT_SAVED_YET").to_owned(),
            };
            let label = ui.weak(saved);
            if editing.id.is_some()
                && let Some(folder) = self.edited_template_folder()
            {
                label.on_hover_text(folder.display().to_string());
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(20));
            return;
        }
        if self.dirty
            && let Some(t) = self.autosave.last_copy
        {
            ui.weak(tr!("LAST_AUTOSAVED", age = i18n::time_ago(t)))
                .on_hover_text(tr!("LAST_AUTOSAVED_HINT"));
            ui.separator();
        }
        let saved = match self.last_saved {
            Some(t) => tr!("LAST_SAVED", age = i18n::time_ago(t)),
            None => tr!("NOT_SAVED_YET").to_owned(),
        };
        let label = ui.weak(saved);
        if let Some(path) = &self.project_path {
            label.on_hover_text(path.display().to_string());
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(20));
    }
}
