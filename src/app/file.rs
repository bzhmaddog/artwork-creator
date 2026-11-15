//! File menu actions: new, open and save projects, recovery copies, recent
//! projects, file dialogs, and leaving a modified project.

use super::*;

/// When a file was last written, if it can be told.
pub(super) fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The file a project is saved to: with the project extension added if it lacks it.
pub(super) fn project_save_path(path: PathBuf) -> PathBuf {
    if project::is_archive(&path) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(project::EXTENSION);
    name.into()
}

impl ArtworkApp {
    /// Asks for files with a native dialog, or with the built-in browser where there is
    /// none.
    pub(super) fn request_file(&mut self, ctx: &egui::Context, req: Request) {
        if self.browser.is_open() {
            return;
        }
        if self.native_dialogs {
            let paths = file_dialog::native(&req);
            if !paths.is_empty() {
                self.on_files(ctx, req.purpose, paths, true);
            }
        } else {
            self.browser.open(req);
        }
    }

    /// Acts on the paths chosen in a file dialog. Before a save or export replaces
    /// existing files, asks the user; `native` dialogs have already asked about the
    /// chosen file itself.
    pub(super) fn on_files(
        &mut self,
        ctx: &egui::Context,
        purpose: Purpose,
        mut paths: Vec<PathBuf>,
        native: bool,
    ) {
        if paths.is_empty() {
            return;
        }
        let chosen = paths.swap_remove(0);
        let path = match purpose {
            Purpose::AddImages => {
                paths.insert(0, chosen);
                return self.add_images(ctx, paths);
            }
            Purpose::ImportSvg => {
                paths.insert(0, chosen);
                for path in paths {
                    self.add_svg(ctx, &path);
                }
                return;
            }
            Purpose::TemplateReference => return self.set_template_reference(ctx, &chosen),
            Purpose::Pattern(target) => return self.set_pattern(target, chosen),
            Purpose::OpenProject => return self.open_project_path(ctx, chosen),
            Purpose::SaveProject => project_save_path(chosen.clone()),
            Purpose::Export => chosen.clone(),
        };
        let existing: Vec<PathBuf> = self
            .files_written(purpose, &path)
            .into_iter()
            .filter(|p| p.exists() && !(native && *p == chosen))
            .collect();
        if existing.is_empty() {
            self.write_files(purpose, path);
        } else {
            self.overwrite = Some(Overwrite {
                purpose,
                path,
                existing,
            });
        }
    }

    /// The files a save or export to `path` writes.
    pub(super) fn files_written(&self, purpose: Purpose, path: &Path) -> Vec<PathBuf> {
        match purpose {
            Purpose::Export => self.export_settings.files(path, self.doc.page_mm()),
            _ => vec![path.to_path_buf()],
        }
    }

    /// Saves or exports to `path`, once any overwrite is agreed.
    pub(super) fn write_files(&mut self, purpose: Purpose, path: PathBuf) {
        match purpose {
            Purpose::SaveProject => self.save_to(path),
            Purpose::Export => self.start_export(path),
            Purpose::AddImages
            | Purpose::ImportSvg
            | Purpose::TemplateReference
            | Purpose::Pattern(_)
            | Purpose::OpenProject => {}
        }
    }

    /// Asks whether to replace the files a save or export would overwrite.
    pub(super) fn overwrite_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.overwrite else {
            return;
        };
        let names: Vec<String> = pending
            .existing
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or(p.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let folder = pending
            .existing
            .first()
            .and_then(|p| p.parent())
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        let mut replace = false;
        let mut cancel = false;
        let response = dialog(ctx, "overwrite").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.strong(if names.len() == 1 {
                tr!("CONFIRM_REPLACE_FILE")
            } else {
                tr!("CONFIRM_REPLACE_FILES")
            });
            ui.add_space(6.0);
            for name in &names {
                ui.label(name);
            }
            crate::ui::callout::callout(
                ui,
                crate::ui::callout::Tone::Warning,
                &tr_n!("FILES_ALREADY_EXIST", names.len(), folder = folder),
            );
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                replace |= ui.button(tr!("REPLACE")).clicked();
                cancel |= ui.button(tr!("CANCEL")).clicked();
            });
        });
        cancel |= response.should_close();
        if replace && !cancel {
            if let Some(Overwrite { purpose, path, .. }) = self.overwrite.take() {
                self.write_files(purpose, path);
            }
        } else if cancel {
            self.overwrite = None;
        }
    }

    /// Stores the app's own settings that are not the project's: the Print panel's and
    /// the View panel's display settings.
    pub(super) fn save_settings(&self, storage: &mut dyn eframe::Storage) {
        // Without the project's choices of what to export, which aren't the app's.
        let print = self
            .export_settings
            .with_choices_of(&ExportSettings::default());
        eframe::set_value(storage, PRINT_SETTINGS_KEY, &print);
        self.canvas.save(storage);
    }

    /// Closes the window or replaces the project, first asking whether to save it when
    /// it has unsaved changes.
    pub(super) fn leave(&mut self, ctx: &egui::Context, leave: Leave) {
        if self.dirty {
            self.confirm_leave = Some(leave);
        } else {
            self.leave_now(ctx, leave);
        }
    }

    /// Closes the window or replaces the project, unsaved changes or not.
    pub(super) fn leave_now(&mut self, ctx: &egui::Context, leave: Leave) {
        match leave {
            Leave::Close => self.closing = true,
            Leave::New => self.new_project(),
            Leave::Open => self.open_project(ctx),
            Leave::OpenPath(path) => self.open_project_path(ctx, path),
            Leave::Recover => self.recover(ctx),
        }
    }

    /// Opens the first copy left by a crash, as the project it was made from with its
    /// changes unsaved. A copy that can't be opened is kept, to be offered again at the
    /// next start.
    pub(super) fn recover(&mut self, ctx: &egui::Context) {
        if self.orphans.is_empty() {
            return;
        }
        let orphan = self.orphans.remove(0);
        let copy = orphan.copy_path();
        self.open_project_path(ctx, copy.clone());
        self.recent.remove(&copy);
        if self.project_path.as_deref() != Some(copy.as_path()) {
            return;
        }
        self.project_path = orphan.info.original.clone();
        self.last_saved = self.project_path.as_deref().and_then(modified_time);
        self.history.mark_unsaved();
        self.dirty = true;
        self.autosave
            .adopt(orphan, self.history.key(), &self.export_settings);
    }

    /// The question asked at the start for each copy left by a crash: recover it, keep
    /// it for later, or delete it.
    pub(super) fn recovery_modal(&mut self, ctx: &egui::Context) {
        if self.confirm_leave.is_some() || self.leave_after_save.is_some() {
            return;
        }
        let Some(orphan) = self.orphans.first() else {
            return;
        };
        let (mut recover, mut later, mut discard) = (false, false, false);
        let more = self.orphans.len() - 1;
        let response = dialog(ctx, "recovery").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.strong(tr!("RECOVERY_TITLE"));
            ui.add_space(4.0);
            ui.label(tr!(
                "RECOVERY_TEXT",
                name = orphan.info.name,
                age = i18n::time_ago(orphan.info.saved_time())
            ));
            if let Some(path) = &orphan.info.original {
                ui.weak(path.display().to_string());
            }
            if more > 0 {
                ui.weak(tr_n!("RECOVERY_MORE", more));
            }
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                recover |= ui.button(tr!("RECOVER")).clicked();
                later |= ui
                    .button(tr!("RECOVER_LATER"))
                    .on_hover_text(tr!("RECOVER_LATER_HINT"))
                    .clicked();
                at_left_edge(ui, |ui| {
                    discard |= ui
                        .button(tr!("DISCARD_RECOVERY"))
                        .on_hover_text(tr!("DISCARD_RECOVERY_HINT"))
                        .clicked();
                });
            });
        });
        later |= response.should_close();
        if discard {
            self.orphans.remove(0).discard();
        } else if recover {
            self.leave(ctx, Leave::Recover);
        } else if later {
            // Let go: offered again at the next start.
            self.orphans.remove(0);
        }
    }

    /// File > Autosave: how often a copy of unsaved changes is kept, or off.
    pub(super) fn autosave_menu(&mut self, ui: &mut egui::Ui) {
        for minutes in crate::files::recovery::INTERVALS {
            let label = if minutes == 0 {
                tr!("AUTOSAVE_OFF").to_owned()
            } else {
                tr_n!("AUTOSAVE_EVERY", minutes)
            };
            ui.radio_value(&mut self.autosave.minutes, minutes, label);
        }
    }

    /// Starts an empty project, keeping the current template.
    pub(super) fn new_project(&mut self) {
        let template = self.doc.template.clone();
        self.doc = Document::default();
        self.selected = None;
        self.project_path = None;
        self.select_template(template);
        self.use_template_page();
        self.canvas.fit_pending = true;
        // The print settings stay: they are the app's.
        self.export_settings = self
            .export_settings
            .with_choices_of(&ExportSettings::default());
        self.saved_export = self.export_settings.clone();
        self.dirty = false;
        self.last_saved = None;
        self.applied.clear();
        self.history.reset(&self.doc);
    }

    /// Opens the dialog to choose a project file.
    pub(super) fn open_project(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::OpenProject,
                title: tr!("OPEN_PROJECT_DIALOG_TITLE"),
                save: false,
                multiple: false,
                filters: PROJECT_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Loads a project and its images. Missing images are reported and drawn crossed out.
    pub(super) fn open_project_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        match project::load(&path) {
            Ok((doc, export)) => {
                self.doc = doc;
                // Its choices of what to export, with the app's print settings.
                self.export_settings = self.export_settings.with_choices_of(&export);
                self.saved_export = self.export_settings.clone();
                let mut missing = Vec::new();
                self.doc.for_each_layer_mut(|layer| {
                    if layer.is_drawn() {
                        return;
                    }
                    if Self::load_image(ctx, layer).is_err() {
                        layer.broken = true;
                        missing.push(layer.source.display().to_string());
                    }
                });
                // Pattern tiles; a missing one leaves its color or gradient.
                self.doc.for_each_pattern_mut(|p| {
                    if p.load().is_err() {
                        missing.push(p.source.display().to_string());
                    }
                });
                if !missing.is_empty() {
                    self.messages
                        .push(tr!("IMAGES_NOT_LOADED", files = missing.join("\n")));
                }
                // Text and shapes are laid out now, so opening doesn't count as a change.
                shape::sync(ctx, &mut self.doc, true);
                text::sync(ctx, &mut self.doc, &self.fonts);
                let mut fonts = Vec::new();
                for leaf in self.doc.leaves() {
                    if let Some(t) = leaf.layer.text.as_ref().filter(|_| leaf.layer.broken)
                        && !fonts.contains(&t.family)
                    {
                        fonts.push(t.family.clone());
                    }
                }
                if !fonts.is_empty() {
                    self.messages
                        .push(tr!("FONTS_NOT_INSTALLED", fonts = fonts.join("\n")));
                }
                // Its own copy of its template, placed as that says.
                self.kept_fingerprint = None;
                self.postponed_fingerprint = None;
                self.load_template();
                // A template no longer installed (deleted, or from another computer):
                // offered from the project's copy.
                self.offer_install = self
                    .doc
                    .template
                    .clone()
                    .filter(|id| self.installed(id).is_none() && self.template_entry.is_some());
                self.selected = None;
                self.recent.add(&path);
                self.last_saved = modified_time(&path);
                self.project_path = Some(path);
                self.canvas.fit_pending = true;
                self.dirty = false;
                self.applied.clear();
                self.history.reset(&self.doc);
            }
            Err(e) => {
                // A file that can't be opened has no place in Open Recent.
                self.recent.remove(&path);
                self.messages.push(e);
            }
        }
    }

    /// Saves to the current file, or asks for one (always with `save_as`).
    pub(super) fn save_project(&mut self, ctx: &egui::Context, save_as: bool) {
        match (&self.project_path, save_as) {
            (Some(p), false) => self.save_to(p.clone()),
            _ => self.request_file(
                ctx,
                Request {
                    purpose: Purpose::SaveProject,
                    title: tr!("SAVE_PROJECT_DIALOG_TITLE"),
                    save: true,
                    multiple: false,
                    filters: PROJECT_FILTERS,
                    default_name: format!("{}.{}", self.project_name(), project::EXTENSION),
                },
            ),
        }
    }

    /// Writes the project to `path` and marks it as saved.
    pub(super) fn save_to(&mut self, path: PathBuf) {
        match project::save(&self.doc, &self.export_settings, &path) {
            Ok(missing) => {
                if !missing.is_empty() {
                    self.messages.push(tr!(
                        "SAVED_WITH_MISSING_IMAGES",
                        layers = missing.join("\n")
                    ));
                }
                self.recent.add(&path);
                self.project_path = Some(path);
                self.history.mark_saved(&self.doc);
                self.saved_export = self.export_settings.clone();
                self.dirty = false;
                self.last_saved = Some(SystemTime::now());
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// Project file name without extensions, or "Untitled". Used in the title and for
    /// exports.
    pub(super) fn project_name(&self) -> String {
        self.project_path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| {
                n.to_string_lossy()
                    .trim_end_matches(".json")
                    .trim_end_matches(".artwork")
                    .to_owned()
            })
            .unwrap_or_else(|| tr!("UNTITLED_PROJECT").into())
    }

    /// Entries of File > Open Recent. Projects that are missing (e.g. on an unplugged
    /// drive) stay listed but grayed out.
    pub(super) fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        // Projects of the linked JSON format, no longer read, are left out.
        for path in self
            .recent
            .paths()
            .iter()
            .filter(|p| project::is_archive(p))
        {
            let exists = path.is_file();
            let button = egui::Button::new(recent::label(path));
            let r = ui
                .add_enabled(exists, button)
                .on_hover_text(path.display().to_string())
                .on_disabled_hover_text(tr!("RECENT_FILE_NOT_FOUND", path = path.display()));
            if r.clicked() {
                open = Some(path.clone());
            }
        }
        ui.separator();
        if ui.button(tr!("CLEAR_RECENT_LIST")).clicked() {
            self.recent.clear();
        }
        if let Some(path) = open {
            let ctx = ui.ctx().clone();
            self.leave(&ctx, Leave::OpenPath(path));
        }
    }

    /// Keeps the window open when it is asked to close with unsaved changes, and asks
    /// whether to save them first.
    pub(super) fn guard_close(&mut self, ctx: &egui::Context) {
        // While a template is edited, editing ends first (asking about it), then the
        // project is closed as usual.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.closing
            && let Some(editing) = &mut self.editing
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            editing.then_quit = true;
            self.end_template_editing();
            return;
        }
        if std::mem::take(&mut self.quit_after_editing) {
            self.leave(ctx, Leave::Close);
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_leave = Some(Leave::Close);
        }
    }

    /// The question asked before closing or replacing the project with unsaved changes:
    /// save, don't save or cancel.
    pub(super) fn close_modal(&mut self, ctx: &egui::Context) {
        if let Some(leave) = self.confirm_leave.clone() {
            let closing = leave == Leave::Close;
            let (mut save, mut discard, mut cancel) = (false, false, false);
            let response = dialog(ctx, "confirm_leave").show(ctx, |ui| {
                ui.set_max_width(420.0);
                let name = self.project_name();
                ui.strong(if closing {
                    tr!("CONFIRM_SAVE_BEFORE_CLOSING", name = name)
                } else {
                    tr!("CONFIRM_SAVE_BEFORE_REPLACING", name = name)
                });
                ui.add_space(4.0);
                crate::ui::callout::callout(
                    ui,
                    crate::ui::callout::Tone::Warning,
                    tr!("UNSAVED_CHANGES_WARNING"),
                );
                ui.add_space(8.0);
                // Save (and close) at the right edge, Cancel (back to the project) on its
                // left, the one that loses work apart at the left. Closing, both buttons
                // that close the app say so.
                let (save_label, discard_label) = if closing {
                    (tr!("SAVE_AND_CLOSE"), tr!("CLOSE_WITHOUT_SAVING"))
                } else {
                    (tr!("SAVE"), tr!("DONT_SAVE"))
                };
                dialog_button_row(ui, |ui| {
                    save |= ui.button(save_label).clicked();
                    cancel |= ui.button(tr!("CANCEL")).clicked();
                    at_left_edge(ui, |ui| {
                        discard |= ui.button(discard_label).clicked();
                    });
                });
            });
            cancel |= response.should_close();
            if save || discard || cancel {
                self.confirm_leave = None;
            }
            if discard {
                self.leave_now(ctx, leave);
            } else if save {
                self.leave_after_save = Some(leave);
                self.save_project(ctx, false);
            }
        }
        // Once the save is done: saved, go on; given up (no file chosen, the overwrite
        // question canceled) or failed, stay.
        let waiting = self.browser.is_open() || self.overwrite.is_some();
        if !waiting
            && let Some(leave) = self.leave_after_save.take()
            && !self.dirty
        {
            self.leave_now(ctx, leave);
        }
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
