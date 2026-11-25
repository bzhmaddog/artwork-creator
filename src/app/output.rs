//! Export and print: the output scene, background jobs, previews and the Export
//! and Print windows and panel.

use super::*;

impl ArtworkApp {
    /// What the export cuts, with its bleed: the print settings'.
    pub(super) fn cuts(&self) -> render::Cuts {
        self.export_settings.cuts()
    }

    /// What gets exported or printed, with text and shapes drawn for `dpi`.
    pub(super) fn output_scene(&self, dpi: f32) -> Scene {
        let colors: Vec<[u8; 3]> = self
            .canvas
            .element_color
            .iter()
            .map(|c| [c.r(), c.g(), c.b()])
            .collect();
        // Text and shapes are drawn again at the output resolution.
        let mut doc = self.doc.clone();
        text::prepare_export(&mut doc, dpi);
        shape::prepare_export(&mut doc, dpi);
        let s = &self.export_settings;
        let scene = Scene::new(
            &doc,
            self.template.as_ref(),
            &colors,
            s.print_lines,
            self.cuts(),
        )
        .with_marks(s.crop_marks, s.registration_marks);
        scene.mirrored(s.mirror, s.mirror_vertical)
    }

    /// Runs `job` on a background thread, with the progress window.
    pub(super) fn start_job(
        &mut self,
        prints: bool,
        job: impl FnOnce(&export::Progress) -> Result<Vec<PathBuf>, String> + Send + 'static,
    ) {
        self.export_open = false;
        self.print_open = false;
        self.job_prints = prints;
        self.export_progress = export::Progress::default();
        self.export_started = Instant::now();
        let progress = self.export_progress.clone();
        self.export_job = Some(std::thread::spawn(move || job(&progress)));
    }

    /// Renders and writes the export on a background thread.
    pub(super) fn start_export(&mut self, base: PathBuf) {
        let scene = self.output_scene(self.export_settings.dpi.clamp(72, 1200) as f32);
        let settings = self.export_settings.clone();
        self.start_job(false, move |progress| {
            export::run(&scene, &settings, &base, progress)
        });
    }

    /// Opens the system print dialog over the app's window, then prints in the
    /// background.
    #[cfg(windows)]
    pub(super) fn choose_printer(&mut self, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let owner = frame.window_handle().ok().and_then(|h| match h.as_raw() {
            RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
            _ => None,
        });
        let paper = crate::export::printing::Paper::of(&self.doc, &self.export_settings);
        match crate::export::printing::choose(owner, &paper) {
            Ok(Some(printer)) => {
                let scene = self.output_scene(crate::export::printing::MAX_PRINT_DPI);
                let settings = self.export_settings.clone();
                let title = self.project_name();
                self.start_job(true, move |progress| {
                    crate::export::printing::print(
                        printer, &scene, &settings, &title, None, progress,
                    )
                    .map(|()| Vec::new())
                });
            }
            Ok(None) => {}
            Err(e) => self.messages.push(e),
        }
    }

    /// Prints through CUPS in the background.
    #[cfg(not(windows))]
    pub(super) fn start_print(&mut self) {
        let (printer, copies) = self.print_to.clone();
        let paper = crate::export::printing::Paper::of(&self.doc, &self.export_settings);
        let scene = self.output_scene(self.export_settings.dpi.clamp(72, 1200) as f32);
        let settings = self.export_settings.clone();
        let title = self.project_name();
        self.start_job(true, move |progress| {
            crate::export::printing::print(
                &printer, copies, &paper, &scene, &settings, &title, progress,
            )
            .map(|()| Vec::new())
        });
    }

    /// Reports a failed or canceled export once its thread has finished.
    pub(super) fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.export_job else { return };
        if !job.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let result = self.export_job.take().map(|j| j.join());
        match result {
            // Success is silent: the progress window closing is enough.
            Some(Ok(Ok(paths))) => log::info!("exported {paths:?}"),
            Some(Ok(Err(e))) if e == render::CANCELED => {
                let title = match self.job_prints {
                    true => tr!("PRINT_CANCELED"),
                    false => tr!("EXPORT_CANCELED"),
                };
                (self.toasts).push(Toast::new("output_canceled", Intent::Info, title, ""));
            }
            Some(Ok(Err(e))) if self.job_prints => self.messages.push(tr!("PRINT_FAILED", e = e)),
            Some(Ok(Err(e))) => self.messages.push(tr!("EXPORT_FAILED", e = e)),
            Some(Err(_)) => self.messages.push(tr!("EXPORT_FAILED_UNEXPECTEDLY").into()),
            None => {}
        }
    }

    /// Re-renders the Export window's preview when the project or the export options
    /// changed. Uses the layers' preview images, so it stays quick.
    pub(super) fn update_export_preview(&mut self, ctx: &egui::Context) {
        let colors: Vec<[u8; 3]> = self
            .canvas
            .element_color
            .iter()
            .map(|c| [c.r(), c.g(), c.b()])
            .collect();
        let s = &self.export_settings;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.doc)
            .unwrap_or_default()
            .hash(&mut h);
        let cuts = self.cuts();
        (&colors, s.print_lines, cuts, &self.template_stamp).hash(&mut h);
        (
            s.crop_marks,
            s.registration_marks,
            s.mirror,
            s.mirror_vertical,
        )
            .hash(&mut h);
        // Images loaded or reloaded change the pixels, not the saved project.
        for leaf in self.doc.leaves() {
            leaf.layer.shown_preview().map(Arc::as_ptr).hash(&mut h);
        }
        let key = h.finish();
        if self.export_preview.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let scene = Scene::preview(
            &self.doc,
            self.template.as_ref(),
            &colors,
            s.print_lines,
            cuts,
        )
        .with_marks(s.crop_marks, s.registration_marks);
        let scene = scene.mirrored(s.mirror, s.mirror_vertical);
        let [pw, ph] = self.doc.page_mm();
        // Rendered at twice the shown width, for high-DPI screens.
        let dpi = EXPORT_PREVIEW_WIDTH * 2.0 / (pw.max(ph) / 25.4);
        match render::compose(&scene, dpi, true) {
            Ok(pixmap) => {
                let image = ColorImage::from_rgba_premultiplied(
                    [pixmap.width() as usize, pixmap.height() as usize],
                    pixmap.data(),
                );
                let texture = match self.export_preview.take() {
                    Some((_, mut t)) => {
                        t.set(image, TextureOptions::LINEAR);
                        t
                    }
                    None => ctx.load_texture("export-preview", image, TextureOptions::LINEAR),
                };
                self.export_preview = Some((key, texture));
            }
            Err(e) => log::warn!("export preview: {e}"),
        }
    }

    /// Progress window shown while an export runs, with a Cancel button.
    pub(super) fn export_progress_window(&mut self, ctx: &egui::Context) {
        if self.export_job.is_none() {
            return;
        }
        let (fraction, stage) = self.export_progress.get();
        let canceling = self.export_progress.is_canceled();
        dialog(ctx, "export-progress").show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading(if self.job_prints {
                tr!("PRINT_PROGRESS_TITLE")
            } else {
                tr!("EXPORT_PROGRESS_TITLE")
            });
            ui.add_space(4.0);
            ui.label(if canceling {
                tr!("EXPORT_CANCELING")
            } else {
                stage.as_str()
            });
            ui.add(
                egui::ProgressBar::new(fraction)
                    .show_percentage()
                    .animate(true),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.weak(format!(
                    "{:.0} s",
                    self.export_started.elapsed().as_secs_f32().floor()
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!canceling, egui::Button::new(tr!("CANCEL")))
                        .clicked()
                    {
                        self.export_progress.cancel();
                    }
                });
            });
        });
    }

    /// The export preview, with the sheets over it when the page is split; they are
    /// dragged on it to move them. Shared by the Export and Print windows.
    pub(super) fn output_preview(&mut self, ui: &mut egui::Ui) {
        let Some((tex, size)) = self
            .export_preview
            .as_ref()
            .map(|(_, t)| (t.id(), t.size_vec2()))
        else {
            return;
        };
        let page_mm = self.doc.page_mm();
        let tiling = self.export_settings.tiling();
        let size = size * (EXPORT_PREVIEW_WIDTH / size.x);
        ui.vertical(|ui| {
            // When tiled, the sheets are dragged around on the preview.
            let sense = if tiling.is_some() {
                egui::Sense::drag()
            } else {
                egui::Sense::hover()
            };
            let image = egui::Frame::new()
                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                .show(ui, |ui| ui.add(egui::Image::new((tex, size)).sense(sense)))
                .inner;
            let shown = image.rect;
            if let Some(t) = tiling {
                let k = shown.width() / page_mm[0];
                let offset = &mut self.export_settings.tile_offset_mm;
                if image.dragged() {
                    let d = image.drag_delta() / k;
                    *offset = [offset[0] + d.x, offset[1] + d.y];
                }
                image
                    .on_hover_cursor(egui::CursorIcon::Grab)
                    .on_hover_text(tr!("EXPORT_TILE_DRAG_HINT"));
                // Each sheet's artwork, where the overlaps show as
                // bands between double lines.
                let [cw, ch] = t.content_mm();
                let painter = ui.painter_at(shown);
                for tile in t.tiles(page_mm) {
                    let [x, y] = tile.origin_mm;
                    let r = egui::Rect::from_min_size(
                        shown.min + egui::vec2(x * k, y * k),
                        egui::vec2(cw * k, ch * k),
                    );
                    painter.rect_stroke(
                        r,
                        0.0,
                        egui::Stroke::new(crate::ui::tokens::STROKE, TILE_COLOR),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            ui.weak(tr!("EXPORT_PREVIEW"));
        });
        ui.add_space(8.0);
    }

    /// What to export, in the Export and Print windows: the formats (`printing` leaves
    /// them out) and the template lines printed. The rest is in the Print panel.
    pub(super) fn output_settings(&mut self, ui: &mut egui::Ui, printing: bool) {
        let s = &mut self.export_settings;
        if !printing {
            ui.horizontal(|ui| {
                ui.label(tr!("EXPORT_FORMATS"));
                ui.checkbox(&mut s.png, "PNG");
                ui.checkbox(&mut s.pdf, "PDF");
                ui.checkbox(&mut s.svg, "SVG")
                    .on_hover_text(tr!("EXPORT_SVG_HINT"));
            });
        }
        let template = self.template.as_ref();
        let has = |k: ElementKind| template.is_some_and(|t| t.has(k));
        if template.is_some() {
            ui.horizontal(|ui| {
                ui.label(tr!("EXPORT_PRINT_LINES"));
                for k in ElementKind::ALL.into_iter().filter(|&k| has(k)) {
                    ui.checkbox(&mut s.print_lines[k.index()], translate(k.label()));
                }
            });
        }
        self.print_warnings(ui);
        ui.weak(tr!("PRINT_SETTINGS_ELSEWHERE"));
    }

    /// What the print settings may leave wrong: images printed at too low a
    /// resolution, marks falling off the page or the sheets.
    pub(super) fn print_warnings(&self, ui: &mut egui::Ui) {
        let s = &self.export_settings;
        let page_mm = self.doc.page_mm();
        let low = self.doc.low_resolution(s.min_image_dpi as f32);
        if !low.is_empty() {
            let names: Vec<String> = low
                .iter()
                .map(|(l, dpi)| format!("{} ({dpi:.0} DPI)", l.name))
                .collect();
            crate::ui::callout::callout(
                ui,
                crate::ui::callout::Tone::Warning,
                &tr_n!("EXPORT_LOW_RESOLUTION", low.len()),
            )
            .on_hover_text(names.join("\n"));
        }
        let Some(template) = self.template.as_ref() else {
            return;
        };
        if !template.has(ElementKind::Outline) || !(s.crop_marks || s.registration_marks) {
            return;
        }
        let elements = render::placed_elements(&self.doc, template);
        let marks = render::trim_box(elements.iter()).map(|trim| {
            let mut marks =
                crate::export::marks::marks(trim, s.bleed_mm, s.crop_marks, s.registration_marks);
            // Where they print: the sheets lie over the mirrored page.
            for p in marks.iter_mut().flat_map(|m| &mut m.points) {
                if s.mirror {
                    p[0] = page_mm[0] - p[0];
                }
                if s.mirror_vertical {
                    p[1] = page_mm[1] - p[1];
                }
            }
            marks
        });
        // What gets printed: the page, or when split, the sheets' artwork, which may
        // reach past the page.
        let (lo, hi) = match s.tiling() {
            Some(t) => t.covered(page_mm),
            None => ([0.0, 0.0], page_mm),
        };
        let off = marks
            .as_ref()
            .and_then(crate::export::marks::bounds)
            .is_some_and(|(mn, mx)| {
                mn[0] < lo[0] || mn[1] < lo[1] || mx[0] > hi[0] || mx[1] > hi[1]
            });
        if off {
            crate::ui::callout::callout(
                ui,
                crate::ui::callout::Tone::Warning,
                if s.tile {
                    tr!("EXPORT_MARKS_OFF_SHEETS")
                } else {
                    tr!("EXPORT_MARKS_OFF_PAGE")
                },
            );
        }
    }

    /// The Print panel: how every project is exported and printed (resolution, bleed,
    /// blank areas, marks, mirroring, sheets), kept with the app's preferences.
    pub(super) fn print_panel(&mut self, ui: &mut egui::Ui) {
        let template = self.template.as_ref();
        let has = |k: ElementKind| template.is_some_and(|t| t.has(k));
        let s = &mut self.export_settings;
        panel_grid(ui, "print-resolution").show(ui, |ui| {
            ui.label(tr!("EXPORT_RESOLUTION"))
                .on_hover_text(tr!("EXPORT_RESOLUTION_HINT"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.dpi).range(72..=1200))
                    .on_hover_text(tr!("EXPORT_RESOLUTION_HINT"));
                crate::ui::layers::unit(ui, "DPI");
            });
            ui.end_row();
            ui.label(tr!("EXPORT_MIN_IMAGE_DPI"))
                .on_hover_text(tr!("EXPORT_MIN_IMAGE_DPI_HINT"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.min_image_dpi).range(0..=1200))
                    .on_hover_text(tr!("EXPORT_MIN_IMAGE_DPI_HINT"));
                crate::ui::layers::unit(ui, "DPI");
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        panel_grid(ui, "print-cuts").show(ui, |ui| {
            ui.label(tr!("EXPORT_MIRROR"))
                .on_hover_text(tr!("EXPORT_MIRROR_HINT"));
            ui.vertical(|ui| {
                ui.checkbox(&mut s.mirror, tr!("EXPORT_MIRROR_HORIZONTALLY"))
                    .on_hover_text(tr!("EXPORT_MIRROR_HINT"));
                ui.checkbox(&mut s.mirror_vertical, tr!("EXPORT_MIRROR_VERTICALLY"))
                    .on_hover_text(tr!("EXPORT_MIRROR_HINT"));
            });
            ui.end_row();
            ui.label(tr!("EXPORT_LEAVE_BLANK"));
            ui.vertical(|ui| {
                ui.add_enabled(
                    has(ElementKind::Outline) || template.is_none(),
                    egui::Checkbox::new(
                        &mut s.cuts.clip_to_outline,
                        tr!("EXPORT_BLANK_OUTSIDE_OUTLINE"),
                    ),
                );
                ui.checkbox(&mut s.cuts.blank_holes, tr!("EXPORT_BLANK_INSIDE_HOLES"));
            });
            ui.end_row();
            ui.label(tr!("BLEED")).on_hover_text(tr!("BLEED_HINT"));
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut s.bleed_mm)
                        .range(0.0..=crate::export::marks::MAX_MARGIN_MM)
                        .speed(0.1)
                        .fixed_decimals(1),
                )
                .on_hover_text(tr!("BLEED_HINT"));
                crate::ui::layers::unit(ui, "mm");
            });
            ui.end_row();
            ui.label(tr!("EXPORT_MARKS"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.crop_marks, tr!("EXPORT_CROP_MARKS"))
                    .on_hover_text(tr!("EXPORT_CROP_MARKS_HINT"));
                ui.checkbox(&mut s.registration_marks, tr!("EXPORT_REGISTRATION_MARKS"))
                    .on_hover_text(tr!("EXPORT_REGISTRATION_MARKS_HINT"));
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.checkbox(&mut s.tile, tr!("EXPORT_TILE"))
            .on_hover_text(tr!("EXPORT_TILE_HINT"));
        if s.tile {
            panel_grid(ui, "tiling").show(ui, |ui| {
                ui.label(tr!("EXPORT_SHEET"));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("sheet")
                        .selected_text(s.sheet.label([0.0; 2]))
                        .show_ui(ui, |ui| {
                            for p in PageSize::ALL
                                .into_iter()
                                .filter(|p| p.portrait_mm().is_some())
                            {
                                ui.selectable_value(&mut s.sheet, p, p.label([0.0; 2]));
                            }
                        });
                    ui.radio_value(
                        &mut s.sheet_orientation,
                        Orientation::Portrait,
                        tr!("PAGE_PORTRAIT"),
                    );
                    ui.radio_value(
                        &mut s.sheet_orientation,
                        Orientation::Landscape,
                        tr!("PAGE_LANDSCAPE"),
                    );
                });
                ui.end_row();
                fn mm(v: &mut f32) -> egui::DragValue<'_> {
                    egui::DragValue::new(v)
                        .range(0.0..=50.0)
                        .speed(0.2)
                        .fixed_decimals(1)
                }
                ui.label(tr!("EXPORT_SHEET_MARGIN"))
                    .on_hover_text(tr!("EXPORT_SHEET_MARGIN_HINT"));
                ui.horizontal(|ui| {
                    ui.add(mm(&mut s.sheet_margin_mm))
                        .on_hover_text(tr!("EXPORT_SHEET_MARGIN_HINT"));
                    crate::ui::layers::unit(ui, "mm");
                });
                ui.end_row();
                ui.label(tr!("EXPORT_TILE_OVERLAP"))
                    .on_hover_text(tr!("EXPORT_TILE_OVERLAP_HINT"));
                ui.horizontal(|ui| {
                    ui.add(mm(&mut s.tile_overlap_mm))
                        .on_hover_text(tr!("EXPORT_TILE_OVERLAP_HINT"));
                    crate::ui::layers::unit(ui, "mm");
                });
                ui.end_row();
                ui.label(tr!("EXPORT_TILE_POSITION"))
                    .on_hover_text(tr!("EXPORT_TILE_POSITION_HINT"));
                ui.horizontal(|ui| {
                    for (v, prefix) in s.tile_offset_mm.iter_mut().zip(["x ", "y "]) {
                        crate::ui::layers::prefixed(ui, prefix, |ui| {
                            ui.add(egui::DragValue::new(v).speed(0.5).fixed_decimals(1))
                        })
                        .on_hover_text(tr!("EXPORT_TILE_POSITION_HINT"));
                    }
                    crate::ui::layers::unit(ui, "mm");
                    if ui
                        .add_enabled(
                            s.tile_offset_mm != [0.0, 0.0],
                            egui::Button::new(tr!("EXPORT_TILE_CENTER")).small(),
                        )
                        .clicked()
                    {
                        s.tile_offset_mm = [0.0, 0.0];
                    }
                });
                ui.end_row();
            });
            ui.checkbox(&mut s.tile_marks, tr!("EXPORT_TILE_MARKS"))
                .on_hover_text(tr!("EXPORT_TILE_MARKS_HINT"));
        }
        self.print_warnings(ui);
    }

    /// Export options; the Export button opens the save dialog.
    pub(super) fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export_open {
            return;
        }
        let page_mm = self.doc.page_mm();
        let mut open = self.export_open;
        let mut go = false;
        let mut close = false;
        egui::Window::new(tr!("EXPORT_WINDOW_TITLE"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                padded(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        self.output_preview(ui);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            // At least wide enough that the checkbox labels don't wrap; rows
                            // that need more widen the column.
                            ui.set_min_width(250.0);
                            self.output_settings(ui, false);
                            let s = &self.export_settings;
                            let dpi = s.dpi as f32;
                            if let Some(t) = s.tiling() {
                                let [cols, rows] = t.grid(page_mm);
                                let [w, h] = render::pixel_size(t.sheet_mm, dpi);
                                ui.weak(tr_n!(
                                    "EXPORT_SHEET_COUNT",
                                    cols * rows,
                                    cols = cols,
                                    rows = rows,
                                    w = w,
                                    h = h
                                ));
                            } else {
                                let [w, h] = render::pixel_size(page_mm, dpi);
                                ui.weak(tr!(
                                    "EXPORT_PIXEL_SIZE",
                                    page = self.doc.page.label(self.doc.custom_page_mm),
                                    w = w,
                                    h = h
                                ));
                            }
                            ui.add_space(10.0);
                            let can = s.any_format() && self.export_job.is_none();
                            let (ok, cancel) = dialog_buttons(ui, tr!("START_EXPORT"), can);
                            go = ok;
                            close = cancel;
                        });
                    })
                });
            });
        self.export_open = open && !close;
        if go {
            let name = self.project_name();
            self.request_file(
                ctx,
                Request {
                    purpose: Purpose::Export,
                    title: tr!("EXPORT_DIALOG_TITLE"),
                    save: true,
                    multiple: false,
                    filters: EXPORT_FILTERS,
                    default_name: name,
                },
            );
        }
    }

    /// Opens the Export window, in place of the Print window: they share their settings.
    pub(super) fn open_export(&mut self) {
        self.export_open = true;
        self.print_open = false;
    }

    /// Opens the Print window, in place of the Export window; on Linux, asks CUPS for its
    /// printers again, in case one was added.
    pub(super) fn open_print(&mut self) {
        self.print_open = true;
        self.export_open = false;
        #[cfg(not(windows))]
        {
            let printers = crate::export::printing::printers();
            if !printers.names.contains(&self.print_to.0) {
                self.print_to.0 = printers
                    .default
                    .clone()
                    .or_else(|| printers.names.first().cloned())
                    .unwrap_or_default();
            }
            self.printers = Some(printers);
        }
    }

    /// What the print will be, and the printer to use where the app picks it. Returns
    /// whether it can be printed.
    pub(super) fn print_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let s = &self.export_settings;
        let paper = crate::export::printing::Paper::of(&self.doc, s);
        let paper_name = match paper.preset {
            Some(p) => p.label([0.0; 2]),
            None => format!("{:.0} x {:.0} mm", paper.size_mm[0], paper.size_mm[1]),
        };
        let orientation = match paper.orientation() {
            Orientation::Landscape => tr!("PAGE_LANDSCAPE"),
            Orientation::Portrait => tr!("PAGE_PORTRAIT"),
        };
        match s.tiling() {
            Some(t) => {
                let [cols, rows] = t.grid(self.doc.page_mm());
                ui.label(tr_n!(
                    "PRINT_SHEETS",
                    cols * rows,
                    cols = cols,
                    rows = rows,
                    paper = paper_name,
                    orientation = orientation
                ));
            }
            None => {
                ui.label(tr!(
                    "PRINT_PAGE",
                    paper = paper_name,
                    orientation = orientation
                ));
            }
        }
        ui.add_space(4.0);
        #[cfg(not(windows))]
        {
            let printers = self.printers.clone().unwrap_or_default();
            if printers.names.is_empty() {
                crate::ui::callout::callout(
                    ui,
                    crate::ui::callout::Tone::Warning,
                    tr!("PRINT_NO_PRINTERS"),
                );
            } else {
                egui::Grid::new("print-to").show(ui, |ui| {
                    ui.label(tr!("PRINT_PRINTER"));
                    egui::ComboBox::from_id_salt("printer")
                        .selected_text(self.print_to.0.as_str())
                        .show_ui(ui, |ui| {
                            for name in &printers.names {
                                ui.selectable_value(
                                    &mut self.print_to.0,
                                    name.clone(),
                                    name.as_str(),
                                );
                            }
                        });
                    ui.end_row();
                    ui.label(tr!("PRINT_COPIES"));
                    ui.add(egui::DragValue::new(&mut self.print_to.1).range(1..=99));
                    ui.end_row();
                });
            }
            !self.print_to.0.is_empty() && self.export_job.is_none()
        }
        #[cfg(windows)]
        {
            crate::ui::callout::callout(
                ui,
                crate::ui::callout::Tone::Info,
                tr!("PRINT_SYSTEM_DIALOG_HINT"),
            );
            self.export_job.is_none()
        }
    }

    /// The Print window: the preview and the export settings, as in the Export window,
    /// then what will be printed and where.
    pub(super) fn print_window(&mut self, ctx: &egui::Context) {
        if !self.print_open {
            return;
        }
        let mut open = true;
        let mut go = false;
        let mut close = false;
        egui::Window::new(tr!("PRINT_WINDOW_TITLE"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                padded(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        self.output_preview(ui);
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_min_width(250.0);
                            self.output_settings(ui, true);
                            crate::ui::callout::callout(
                                ui,
                                crate::ui::callout::Tone::Info,
                                tr!("PRINT_SETTINGS_SHARED"),
                            );
                            ui.add_space(6.0);
                            let can = self.print_controls(ui);
                            ui.add_space(10.0);
                            let (ok, cancel) = dialog_buttons(ui, tr!("START_PRINT"), can);
                            go = ok;
                            close = cancel;
                        });
                    })
                });
            });
        self.print_open = open && !close;
        if go {
            #[cfg(windows)]
            {
                self.print_pending = true;
            }
            #[cfg(not(windows))]
            self.start_print();
        }
    }
}
