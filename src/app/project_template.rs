//! Choosing the project's template, comparing it with the installed one and
//! linking layers to template points.

use super::*;

/// The field picking a template, showing `current`: a list of `templates` by vendor, only
/// as wide as its longest choice. Returns the folder of the one picked, or `chosen` as it
/// was, and the field.
pub(crate) fn template_selector(
    ui: &mut egui::Ui,
    current: &str,
    templates: &[TemplateEntry],
    chosen: &Option<String>,
) -> (Option<String>, egui::Response) {
    let mut choice = chosen.clone();
    // A field as wide as the panel, like a combo box, opening a list only as wide as
    // its longest choice.
    let arrow = egui::Atom::paint(egui::Vec2::splat(ui.spacing().icon_width), |ui, args| {
        let r = args.rect;
        let (w, h) = (r.width() * 0.7, r.height() * 0.45);
        let c = r.center();
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                c + egui::vec2(-w / 2.0, -h / 2.0),
                c + egui::vec2(w / 2.0, -h / 2.0),
                c + egui::vec2(0.0, h / 2.0),
            ],
            ui.visuals().widgets.inactive.fg_stroke.color,
            egui::Stroke::NONE,
        ));
    });
    let field = ui.add(
        egui::Button::new((egui::RichText::new(current), egui::Atom::grow(), arrow))
            .truncate()
            .min_size(egui::vec2(ui.available_width(), 0.0)),
    );
    // Wide enough for the longest choice, its version and the space between them.
    let list_width = {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let width = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        };
        let names = std::iter::once(width(tr!("NO_TEMPLATE"))).chain(templates.iter().map(|t| {
            let indent = if t.vendor.is_some() {
                ui.spacing().indent
            } else {
                0.0
            };
            let vendor = t.vendor.as_deref().map_or(0.0, width);
            (indent + width(&t.name)).max(vendor)
        }));
        let versions = templates
            .iter()
            .filter_map(|t| t.version.as_deref())
            .map(width)
            .fold(0.0, f32::max);
        // Each template's origin has a mark before its name.
        let origin = 14.0 + ui.spacing().item_spacing.x;
        // A template with problems has a mark after its name.
        let mark = if templates.iter().any(|t| !t.problems.is_empty()) {
            14.0 + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        names.fold(0.0, f32::max)
            + origin
            + mark
            + versions
            + 2.0 * ui.spacing().button_padding.x
            + 3.0 * ui.spacing().item_spacing.x
    };
    egui::Popup::menu(&field)
        .width(list_width.min(field.rect.width()))
        .show(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            // Each choice as wide as the list, all of it clickable; `indent` sets its
            // text in, inside the row; the mark of its origin before it; a mark after it when the template diagnostic
            // found problems, listed in its tooltip; its version, if any, at its right
            // end.
            let mut row = |ui: &mut egui::Ui,
                           id: Option<String>,
                           text: &str,
                           version: Option<&str>,
                           indent: f32,
                           origin: Option<template::Origin>,
                           problems: &[Problem]| {
                let gap = egui::Atom::paint(egui::vec2(indent, 0.0), |_, _| {});
                let origin_mark = origin_mark(ui, origin);
                let version = egui::RichText::new(version.unwrap_or_default()).weak();
                let worst = if crate::template::check::has_errors(problems) {
                    Severity::Error
                } else {
                    Severity::Warning
                };
                let mark = if problems.is_empty() {
                    egui::Atom::paint(egui::Vec2::ZERO, |_, _| {})
                } else {
                    crate::ui::callout::warning_mark(problem_color(ui, worst))
                };
                let button = egui::Button::selectable(
                    choice == id,
                    (gap, origin_mark, text, mark, egui::Atom::grow(), version),
                )
                .min_size(egui::vec2(ui.available_width(), 0.0));
                let mut response = ui.add(button);
                if origin.is_some() || !problems.is_empty() {
                    response = response.on_hover_ui(|ui| {
                        match origin {
                            Some(template::Origin::Provided) => {
                                ui.label(tr!("TEMPLATE_PROVIDED_HINT"));
                            }
                            Some(template::Origin::User) => {
                                ui.label(tr!("TEMPLATE_USER_HINT"));
                            }
                            Some(template::Origin::Project) => {
                                ui.label(tr!("TEMPLATE_PROJECT_HINT"));
                            }
                            None => {}
                        }
                        if !problems.is_empty() {
                            problems_tooltip(ui, problems);
                        }
                    });
                }
                if response.clicked() {
                    choice = id;
                }
            };
            row(ui, None, tr!("NO_TEMPLATE"), None, 0.0, None, &[]);
            // Listed by vendor (see `template::scan`): each vendor's name over its
            // models, set in a little.
            let mut vendor: Option<String> = None;
            for t in templates {
                let key = t.vendor.as_ref().map(|v| v.to_lowercase());
                if key.is_some() && key != vendor {
                    ui.add_space(2.0);
                    ui.weak(t.vendor.as_deref().unwrap_or_default());
                }
                vendor = key;
                let indent = if t.vendor.is_some() {
                    ui.spacing().indent
                } else {
                    0.0
                };
                row(
                    ui,
                    Some(t.id.clone()),
                    &t.name,
                    t.version.as_deref(),
                    indent,
                    Some(t.origin),
                    &t.problems,
                );
            }
        });
    (choice, field)
}

/// The mark before a template's name in the template list: the app's icon for a
/// template provided with it, a person for one of the user's, a sheet for one only the
/// project keeps; drawn, so it shows
/// whatever the font has. Without an origin, an empty space as wide, to line names up.
pub(super) fn origin_mark(ui: &egui::Ui, origin: Option<template::Origin>) -> egui::Atom<'static> {
    let size = egui::Vec2::splat(14.0);
    match origin {
        None => egui::Atom::paint(size, |_, _| {}),
        Some(template::Origin::Provided) => {
            const PNG: &[u8] = include_bytes!("../../assets/icon/png/artwork-creator-32.png");
            // Loaded once, kept in the context's memory.
            let id = egui::Id::unique("template_provided_icon");
            let texture = ui
                .data(|d| d.get_temp::<egui::TextureHandle>(id))
                .unwrap_or_else(|| {
                    let image = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
                        .map(|i| i.into_rgba8())
                        .map(|i| {
                            ColorImage::from_rgba_unmultiplied(
                                [i.width() as usize, i.height() as usize],
                                i.as_raw(),
                            )
                        })
                        .unwrap_or_default();
                    let texture = ui.ctx().load_texture(
                        "template-provided-icon",
                        image,
                        TextureOptions::LINEAR,
                    );
                    ui.data_mut(|d| d.insert_temp(id, texture.clone()));
                    texture
                });
            egui::Atom::paint(size, move |ui, args| {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                let rect = egui::Rect::from_center_size(args.rect.center(), size);
                ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
            })
        }
        Some(template::Origin::User) => egui::Atom::paint(size, |ui, args| {
            let c = args.rect.center();
            let color = ui.visuals().widgets.inactive.fg_stroke.color;
            let painter = ui.painter();
            // A head over shoulders.
            painter.circle_filled(c + egui::vec2(0.0, -3.0), 3.0, color);
            let shoulders: Vec<egui::Pos2> = (0..=12)
                .map(|k| {
                    let a = std::f32::consts::PI * k as f32 / 12.0;
                    c + egui::vec2(-5.5 * a.cos(), 6.5 - 5.0 * a.sin())
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(
                shoulders,
                color,
                egui::Stroke::NONE,
            ));
        }),
        Some(template::Origin::Project) => egui::Atom::paint(size, |ui, args| {
            // A sheet with a folded corner: the project's file.
            let c = args.rect.center();
            let stroke = egui::Stroke::new(1.2, ui.visuals().widgets.inactive.fg_stroke.color);
            let (l, r, t, b, fold) = (c.x - 4.5, c.x + 4.5, c.y - 6.0, c.y + 6.0, 3.5);
            let p = egui::pos2;
            let painter = ui.painter();
            painter.add(egui::Shape::closed_line(
                vec![p(l, t), p(r - fold, t), p(r, t + fold), p(r, b), p(l, b)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(r - fold, t), p(r - fold, t + fold), p(r, t + fold)],
                stroke,
            ));
        }),
    }
}

/// The color of the mark of a problem of `severity` found in a template.
pub(super) fn problem_color(ui: &egui::Ui, severity: Severity) -> Color32 {
    match severity {
        Severity::Error => ui.visuals().error_fg_color,
        Severity::Warning => ui.visuals().warn_fg_color,
    }
}

/// The problems found in a template, errors first, one per line after its mark: for a
/// tooltip.
pub(super) fn problems_tooltip(ui: &mut egui::Ui, problems: &[Problem]) {
    let mut sorted: Vec<&Problem> = problems.iter().collect();
    sorted.sort_by_key(|p| p.severity);
    ui.label(tr!("TEMPLATE_CHECK_LIST_HINT"));
    for p in sorted {
        ui.horizontal_top(|ui| {
            crate::ui::callout::warning_icon(ui, problem_color(ui, p.severity));
            ui.label(&p.text);
        });
    }
}

/// The Template check panel's contents: the problems found, counted, errors first; one
/// about a shape is a button selecting it. Returns the layer clicked.
pub(super) fn problem_list(
    ui: &mut egui::Ui,
    problems: &[Problem],
    selected: Option<u64>,
) -> Option<u64> {
    if problems.is_empty() {
        crate::ui::callout::callout(ui, crate::ui::callout::Tone::Info, tr!("TEMPLATE_CHECK_OK"));
        return None;
    }
    let mut sorted: Vec<&Problem> = problems.iter().collect();
    sorted.sort_by_key(|p| p.severity);
    let errors = sorted
        .iter()
        .filter(|p| p.severity == Severity::Error)
        .count();
    let warnings = sorted.len() - errors;
    let counts: Vec<String> = [
        (errors > 0).then(|| tr_n!("TEMPLATE_CHECK_ERRORS", errors)),
        (warnings > 0).then(|| tr_n!("TEMPLATE_CHECK_WARNINGS", warnings)),
    ]
    .into_iter()
    .flatten()
    .collect();
    ui.weak(counts.join(", "));
    ui.add_space(4.0);
    let mut clicked = None;
    for p in sorted {
        ui.horizontal_top(|ui| {
            crate::ui::callout::warning_icon(ui, problem_color(ui, p.severity));
            match p.layer {
                Some(id) => {
                    let button = egui::Button::selectable(selected == Some(id), p.text.as_str())
                        .wrap_mode(egui::TextWrapMode::Wrap);
                    if ui
                        .add(button)
                        .on_hover_text(tr!("TEMPLATE_CHECK_SELECT_HINT"))
                        .clicked()
                    {
                        clicked = Some(id);
                    }
                }
                None => {
                    ui.add(egui::Label::new(p.text.as_str()).wrap());
                }
            }
        });
    }
    clicked
}

/// The bar under the canvas while comparing templates: two large buttons, Use installed
/// (the main one, in the accent color) right of Keep project's, in a frame outlined in
/// the accent color. Returns which was clicked: (keep, use installed).
pub(super) fn compare_bar_body(ui: &mut egui::Ui) -> (bool, bool) {
    let accent = ui.visuals().selection.bg_fill;
    let frame = egui::Frame::popup(ui.style())
        .inner_margin(egui::Margin::same(10))
        .stroke(egui::Stroke::new(crate::ui::tokens::STROKE, accent))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 2,
            color: egui::Color32::from_black_alpha(110),
        });
    frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.spacing_mut().button_padding = egui::vec2(16.0, 8.0);
                let text = |key: &str| egui::RichText::new(i18n::translate(key)).size(16.0);
                let keep = ui
                    .button(text(tr_noop!("KEEP_PROJECT_TEMPLATE")))
                    .on_hover_text(tr!("KEEP_PROJECT_TEMPLATE_HINT"))
                    .clicked();
                let use_installed = ui
                    .add(
                        egui::Button::new(
                            text(tr_noop!("USE_INSTALLED_TEMPLATE"))
                                .color(ui.visuals().selection.stroke.color),
                        )
                        .fill(accent),
                    )
                    .on_hover_text(tr!("USE_INSTALLED_TEMPLATE_HINT"))
                    .clicked();
                (keep, use_installed)
            })
            .inner
        })
        .inner
}

/// The popup asking what to do with a changed installed template: what changed, both
/// versions, and the buttons. Returns which was clicked: (compare, keep the project's,
/// use the installed one).
pub(super) fn template_differs_body(
    ui: &mut egui::Ui,
    name: &str,
    project: &str,
    installed: &str,
) -> (bool, bool, bool) {
    let (mut compare, mut keep, mut use_installed) = (false, false, false);
    ui.set_width(420.0);
    ui.strong(tr!("TEMPLATE_DIFFERS_TITLE"));
    ui.add_space(4.0);
    ui.label(tr!("TEMPLATE_DIFFERS_TEXT", name = name));
    ui.add_space(4.0);
    egui::Grid::new("template_versions").show(ui, |ui| {
        ui.label(tr!("TEMPLATE_PROJECT_VERSION"));
        ui.strong(project);
        ui.end_row();
        ui.label(tr!("TEMPLATE_INSTALLED_VERSION"));
        ui.strong(installed);
        ui.end_row();
    });
    ui.add_space(8.0);
    dialog_button_row(ui, |ui| {
        use_installed = ui
            .button(tr!("USE_INSTALLED_TEMPLATE"))
            .on_hover_text(tr!("USE_INSTALLED_TEMPLATE_HINT"))
            .clicked();
        keep = ui
            .button(tr!("KEEP_PROJECT_TEMPLATE"))
            .on_hover_text(tr!("KEEP_PROJECT_TEMPLATE_HINT"))
            .clicked();
        at_left_edge(ui, |ui| {
            compare = ui
                .button(tr!("COMPARE_TEMPLATES"))
                .on_hover_text(tr!("COMPARE_TEMPLATES_HINT"))
                .clicked();
        });
    });
    (compare, keep, use_installed)
}

impl ArtworkApp {
    /// Picks an installed template by folder id: the project gets its own copy of it, then
    /// uses it.
    pub(super) fn select_template(&mut self, id: Option<String>) {
        self.doc.template = id.clone();
        self.doc.template_copy = None;
        if let Some(id) = &id {
            match self.installed(id).map(TemplateCopy::of) {
                Some(Ok(copy)) => self.doc.template_copy = Some(copy),
                Some(Err(e)) => self.messages.push(e),
                None => self.messages.push(tr!("TEMPLATE_NOT_INSTALLED", id = id)),
            }
        }
        self.load_template();
    }

    /// Loads the project's copy of its template. A project saved before projects kept
    /// their template gets a copy of the installed one, kept from then on.
    pub(super) fn load_template(&mut self) {
        self.template = None;
        self.template_entry = None;
        self.doc.template_variables.clear();
        self.comparing = None;
        self.installed_fingerprint = None;
        self.template_stamp = Vec::new();
        self.dirty = true;
        let Some(id) = self.doc.template.clone() else {
            return;
        };
        self.refresh_installed();
        if self
            .doc
            .template_copy
            .as_ref()
            .is_none_or(|c| c.files.is_empty())
        {
            let Some(installed) = self.installed(&id) else {
                self.messages.push(tr!("TEMPLATE_NOT_INSTALLED", id = id));
                return;
            };
            let fresh = match TemplateCopy::of(installed) {
                Ok(fresh) => fresh,
                Err(e) => {
                    self.messages.push(e);
                    return;
                }
            };
            self.doc.template_copy = Some(fresh);
        }
        let Some(copy) = &self.doc.template_copy else {
            return;
        };
        let entry = match copy.files.unpack(&id) {
            Ok(dir) => template::read_entry(&dir),
            Err(e) => {
                self.messages.push(e);
                return;
            }
        };
        let Some(entry) = entry else {
            self.messages.push(tr!("TEMPLATE_NOT_INSTALLED", id = id));
            return;
        };
        if let Some(e) = &entry.config_error {
            self.messages.push(tr!("INVALID_TEMPLATE_CONFIG", e = e));
        }
        match template::load(&entry) {
            Ok(t) => {
                // Each element's lines shown, in its color.
                self.canvas.element_show = vec![true; t.elements.len()];
                self.canvas.element_color = canvas::element_colors(&t);
                self.template = Some(t);
                // Its variables, which the project's layers can use.
                self.doc.template_variables = entry.config.variables.clone();
                self.template_entry = Some(entry);
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// The installed template of folder `id`.
    pub(super) fn installed(&self, id: &str) -> Option<&TemplateEntry> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// The installed template differs from the project's copy (and was not set aside):
    /// its fingerprint.
    pub(super) fn installed_differs(&self) -> Option<String> {
        let fp = self.installed_fingerprint.clone()?;
        (Some(&fp) != self.copy_fingerprint().as_ref()
            && self.kept_fingerprint.as_ref() != Some(&fp))
        .then_some(fp)
    }

    /// Replaces the project's copy of its template with the installed one, placed as it
    /// says. Undo brings the copy back.
    pub(super) fn use_installed_template(&mut self) {
        let Some(id) = self.doc.template.clone() else {
            return;
        };
        match self.installed(&id).map(TemplateCopy::of) {
            Some(Ok(copy)) => {
                self.doc.template_copy = Some(copy);
                self.load_template();
            }
            Some(Err(e)) => self.messages.push(e),
            None => {}
        }
    }

    /// Starts comparing the installed template with the project's copy on the canvas.
    pub(super) fn compare_installed_template(&mut self) {
        let Some(entry) = self
            .doc
            .template
            .as_deref()
            .and_then(|id| self.installed(id))
        else {
            return;
        };
        match template::load(entry) {
            Ok(t) => self.comparing = Some((entry.clone(), t)),
            Err(e) => self.messages.push(e),
        }
    }

    /// In the Template panel, after the user put off deciding about the installed
    /// template: a reminder, and a button to open the popup again.
    pub(super) fn template_differs_note(&mut self, ui: &mut egui::Ui) {
        let Some(fp) = self.installed_differs() else {
            self.comparing = None;
            return;
        };
        if self.postponed_fingerprint.as_ref() != Some(&fp) || self.comparing.is_some() {
            return;
        }
        ui.add_space(4.0);
        crate::ui::callout::callout_small(
            ui,
            crate::ui::callout::Tone::Info,
            tr!("TEMPLATE_DIFFERS_TITLE"),
        );
        if ui.button(tr!("REVIEW_TEMPLATE_CHANGE")).clicked() {
            self.postponed_fingerprint = None;
        }
    }

    /// The popup asking what to do when the installed template differs from the
    /// project's copy: compare them on the canvas, keep the project's, or use the
    /// installed one. Esc puts it off (see `template_differs_note`).
    pub(super) fn template_differs_modal(&mut self, ctx: &egui::Context) {
        let Some(fp) = self.installed_differs().filter(|_| self.editing.is_none()) else {
            return;
        };
        if self.comparing.is_some()
            || self.postponed_fingerprint.as_ref() == Some(&fp)
            || !self.messages.is_empty()
            || !self.orphans.is_empty()
            || self.confirm_leave.is_some()
        {
            return;
        }
        let (project, installed) = self.template_versions();
        let name = self
            .current_entry()
            .map(TemplateEntry::full_name)
            .unwrap_or_default();
        let response = dialog(ctx, "template_differs").show(ctx, |ui| {
            template_differs_body(ui, &name, &project, &installed)
        });
        let (compare, keep, use_installed) = response.inner;
        self.template_choice(compare, keep, use_installed, fp.clone());
        if response.should_close() && !(compare || keep || use_installed) {
            self.postponed_fingerprint = Some(fp);
        }
    }

    /// The popup offering to install the template of the project just opened, which is
    /// not installed, from the project's copy of it: Install, or Not now (Esc too).
    pub(super) fn template_install_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.offer_install.clone() else {
            return;
        };
        // Still the project's template, still not installed.
        if self.doc.template.as_ref() != Some(&id) || self.installed(&id).is_some() {
            self.offer_install = None;
            return;
        }
        if self.editing.is_some()
            || !self.messages.is_empty()
            || !self.orphans.is_empty()
            || self.confirm_leave.is_some()
        {
            return;
        }
        let name = self
            .current_entry()
            .map(TemplateEntry::full_name)
            .unwrap_or_else(|| id.clone());
        let (mut install, mut later) = (false, false);
        let response = dialog(ctx, "install_template").show(ctx, |ui| {
            ui.set_width(420.0);
            ui.strong(tr!("TEMPLATE_NOT_INSTALLED_TITLE"));
            ui.add_space(4.0);
            ui.label(tr!("INSTALL_PROJECT_TEMPLATE_TEXT", name = name));
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                install = ui.button(tr!("INSTALL_TEMPLATE")).clicked();
                later = ui.button(tr!("NOT_NOW")).clicked();
            });
        });
        if install {
            self.install_project_template(&id);
        }
        if install || later || response.should_close() {
            self.offer_install = None;
        }
    }

    /// Writes the project's copy of its template, of folder `id`, into the user's
    /// templates, where it is listed from then on.
    pub(super) fn install_project_template(&mut self, id: &str) {
        let Some(dir) = self.user_templates_dir.clone() else {
            return self
                .messages
                .push(tr!("NO_USER_TEMPLATES_FOLDER").to_owned());
        };
        let Some(copy) = &self.doc.template_copy else {
            return;
        };
        // A plain folder name: it comes from the project file.
        let Some(name) = template::normal_path(id).filter(|p| p.components().count() == 1) else {
            return;
        };
        if let Err(e) = copy.files.write_into(&dir.join(name)) {
            return self.messages.push(e);
        }
        self.rescan_templates();
    }

    /// While comparing the templates: a bar at the bottom of the canvas with the popup's
    /// choices (the labels at the top of the canvas say which side is which).
    pub(super) fn compare_bar(&mut self, ctx: &egui::Context) {
        let Some(fp) = self
            .installed_differs()
            .filter(|_| self.comparing.is_some() && self.editing.is_none())
        else {
            return;
        };
        let rect = self.canvas.rect;
        let (mut keep, mut use_installed) = (false, false);
        egui::Area::new(egui::Id::unique("compare_bar"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(rect.center_bottom() - egui::vec2(0.0, 16.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| (keep, use_installed) = compare_bar_body(ui));
        self.template_choice(false, keep, use_installed, fp);
    }

    /// Carries out a choice about the installed template of fingerprint `fp`.
    pub(super) fn template_choice(
        &mut self,
        compare: bool,
        keep: bool,
        use_installed: bool,
        fp: String,
    ) {
        if compare {
            self.compare_installed_template();
        }
        if keep {
            self.kept_fingerprint = Some(fp);
            self.comparing = None;
        }
        if use_installed {
            self.use_installed_template();
        }
    }

    /// The versions of the project's copy of its template and of the installed one, as
    /// shown to the user.
    pub(super) fn template_versions(&self) -> (String, String) {
        let shown = |v: Option<&String>| {
            v.cloned()
                .unwrap_or_else(|| tr!("TEMPLATE_VERSION_UNKNOWN").to_owned())
        };
        let project = self
            .doc
            .template_copy
            .as_ref()
            .and_then(|c| c.version.as_ref());
        let installed = self
            .doc
            .template
            .as_deref()
            .and_then(|id| self.installed(id))
            .and_then(|e| e.version.as_ref());
        (shown(project), shown(installed))
    }

    /// While comparing: the installed template's elements, placed on the page as it says,
    /// and their colors.
    pub(super) fn compared_elements(&self) -> Option<(Vec<PlacedElement>, Vec<Color32>)> {
        let (_, t) = self.comparing.as_ref()?;
        Some((
            render::placed_elements(&self.doc, t),
            canvas::element_colors(t),
        ))
    }

    /// Sets the page the current template asks for in its `template.json` (what it
    /// leaves out stays); the template is centered on it. Done when a template is picked
    /// or a project started, not when one is opened: a project keeps its page.
    pub(super) fn use_template_page(&mut self) {
        let Some(entry) = self.current_entry() else {
            return;
        };
        let setup = entry.page;
        if setup == template::PageSetup::default() {
            return;
        }
        if let Some(size) = setup.size {
            self.doc.page = size;
        }
        if let Some(mm) = setup.custom_mm {
            self.doc.custom_page_mm = mm;
        }
        if let Some(o) = setup.orientation {
            self.doc.orientation = o;
        }
        self.canvas.fit_pending = true;
        self.dirty = true;
    }

    /// The template the project uses, as read from its copy.
    pub(super) fn current_entry(&self) -> Option<&TemplateEntry> {
        self.template_entry.as_ref()
    }

    /// The dialog naming, changing or deleting a template, and what it decides.
    /// After a layer was dropped on a template shape's center: asks whether its position
    /// should follow that shape, which links its X and Y to the shape's (see
    /// `variables::point_names`), its offset from it kept.
    pub(super) fn link_position_modal(&mut self, ctx: &egui::Context) {
        let Some(offer) = &self.canvas.link_offer else {
            return;
        };
        let mut done = false;
        let mut link = false;
        let response = dialog(ctx, "link_position").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.strong(tr!("LINK_POSITION_TITLE"));
            ui.add_space(4.0);
            ui.label(tr!("LINK_POSITION_INFO"));
            ui.add_space(8.0);
            let (ok, cancel) = dialog_buttons(ui, tr!("LINK_POSITION_BUTTON"), true);
            link = ok || ui.input(|i| i.key_pressed(egui::Key::Enter));
            done = link || cancel;
        });
        if link {
            let names = variables::point_names(&offer.point);
            let (id, offset) = (offer.layer, offer.offset);
            if let Some(layer) = self.doc.layer_mut(id) {
                for (k, field) in [variables::Field::X, variables::Field::Y]
                    .into_iter()
                    .enumerate()
                {
                    layer
                        .bindings
                        .insert(field, variables::offset_formula(&names[k], offset[k]));
                }
            }
        }
        if done || response.should_close() {
            self.canvas.link_offer = None;
        }
    }

    pub(super) fn template_dialog_modal(&mut self, ctx: &egui::Context) {
        use crate::ui::template::Outcome;
        let Some(dialog) = &mut self.template_dialog else {
            return;
        };
        let outcome = dialog.show(ctx);
        if !matches!(outcome, Outcome::Open) {
            self.template_dialog = None;
        }
        let result = match outcome {
            Outcome::Open | Outcome::Canceled => Ok(()),
            Outcome::New { name, vendor } => {
                self.new_template(ctx, name, vendor);
                Ok(())
            }
            Outcome::Delete { replacement } => self.delete_template(replacement),
            Outcome::EditCopy { name } => {
                self.edit_template(Some(name));
                Ok(())
            }
        };
        if let Err(e) = result {
            self.messages.push(e);
        }
    }

    /// Notices when the installed template of the project's changes on disk (its
    /// version is then compared with the project's copy, see `installed_differs`).
    pub(super) fn watch_template(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.last_template_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_template_check = Instant::now();
        let Some(entry) = self
            .doc
            .template
            .as_deref()
            .and_then(|id| self.installed(id))
        else {
            return;
        };
        if entry.stamp() == self.template_stamp {
            return;
        }
        let folder = entry.folder.clone();
        log::info!("installed template files changed: {}", folder.display());
        if let Some(fresh) = template::read_entry(&folder)
            && let Some(slot) = self.templates.iter_mut().find(|t| t.id == fresh.id)
        {
            // From the same folder, checked again.
            *slot = TemplateEntry {
                origin: slot.origin,
                problems: crate::template::check::check_entry(&fresh),
                ..fresh
            };
        }
        self.refresh_installed();
    }

    /// Reads again the installed template of the project's: its stamp, its fingerprint,
    /// and what is compared with it.
    pub(super) fn refresh_installed(&mut self) {
        let installed = self
            .doc
            .template
            .as_deref()
            .and_then(|id| self.installed(id));
        let stamp = installed.map(TemplateEntry::stamp).unwrap_or_default();
        let fingerprint = installed
            .and_then(|e| template::TemplateFiles::read(e).ok())
            .map(|f| f.fingerprint());
        self.template_stamp = stamp;
        self.installed_fingerprint = fingerprint;
        if self.comparing.is_some() {
            self.comparing = None;
            if self.installed_differs().is_some() {
                self.compare_installed_template();
            }
        }
    }

    /// Lists the templates folders again and reloads the current template.
    pub(super) fn rescan_templates(&mut self) {
        self.templates = template::scan_all(
            self.templates_dir.as_deref(),
            self.user_templates_dir.as_deref(),
        );
        self.refresh_installed();
        // A project whose template could not be found may find it now.
        if self.template.is_none() {
            self.load_template();
        }
    }

    /// The Template panel: the template to work on, and its elements' lines on the
    /// canvas. How it sits on the page comes from the template itself.
    pub(super) fn template_panel(&mut self, ui: &mut egui::Ui) {
        // The template in use, with its version: the project's copy's.
        let current = self
            .current_entry()
            .map(|t| match &t.version {
                Some(v) => tr!("TEMPLATE_WITH_VERSION", name = t.full_name(), version = v),
                None => t.full_name(),
            })
            .unwrap_or_else(|| tr!("NO_TEMPLATE").into());
        // The field, with Edit at its right; the field takes the rest of the width.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        let mut choice = self.doc.template.clone();
        // A template the project keeps but that isn't installed is listed too, as the
        // project's.
        let project_only = match (&self.doc.template, self.current_entry()) {
            (Some(id), Some(entry)) if self.installed(id).is_none() => Some(TemplateEntry {
                id: id.clone(),
                origin: template::Origin::Project,
                ..entry.clone()
            }),
            _ => None,
        };
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                self.edit_template_button(ui);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let listed: std::borrow::Cow<[TemplateEntry]> = match &project_only {
                        Some(entry) => {
                            let mut list = self.templates.clone();
                            list.push(entry.clone());
                            template::sort_entries(&mut list);
                            list.into()
                        }
                        None => self.templates.as_slice().into(),
                    };
                    choice = template_selector(ui, &current, &listed, &self.doc.template).0;
                });
            },
        );
        if choice != self.doc.template {
            self.select_template(choice);
            self.use_template_page();
        }
        self.template_differs_note(ui);
        let Some(t) = &self.template else {
            return;
        };
        ui.add_space(4.0);
        for (k, e) in t.elements.iter().enumerate() {
            if k >= self.canvas.element_show.len() || k >= self.canvas.element_color.len() {
                continue;
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.canvas.show_template, |ui| {
                    ui.checkbox(&mut self.canvas.element_show[k], "")
                        .on_hover_text(tr!("SHOW_ELEMENT_LINES"));
                });
                let line = &mut self.canvas.element_color[k];
                let mut c = line.to_srgba_unmultiplied();
                if crate::ui::colors::button(ui, &mut c, true, None)
                    .on_hover_text(tr!("ELEMENT_LINE_COLOR"))
                    .changed()
                {
                    let [r, g, b, a] = c;
                    *line = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
                }
                ui.label(translate(&e.name));
                ui.weak(tr_n!("SHAPE_COUNT", e.polylines.len()));
            });
        }
    }
}
