//! The template editing mode: starting, saving and leaving it, its panels and
//! the check of the edited template.

use super::*;

/// The name of the template edited in `doc`, as shown: vendor and name.
pub(super) fn edited_name(doc: &Document) -> String {
    let Some(config) = crate::template::edit::config(doc) else {
        return String::new();
    };
    let name = config.name.clone().unwrap_or_default();
    match &config.vendor {
        Some(v) => format!("{v} - {name}"),
        None => name,
    }
}

impl ArtworkApp {
    /// The Template properties panel: the settings of the template being edited, taken
    /// at once (Undo goes back; written when it is saved). A new page moves the shapes
    /// with the template, centered on it.
    pub(super) fn template_settings_panel(&mut self, ui: &mut egui::Ui) {
        let compare = self
            .template_source_entry()
            .map(|_| self.editing.as_ref().is_some_and(|e| e.comparing.is_some()));
        let keys = self.shortcuts.texts(ui.ctx());
        let compare_hint = crate::ui::shortcuts::with_key(
            tr!("COMPARE_TEMPLATE_SOURCE_HINT"),
            keys.get(&Command::CompareTemplateSource),
        );
        let (Some(editing), Some(config)) =
            (&mut self.editing, crate::template::edit::config(&self.doc))
        else {
            return;
        };
        if editing.form.as_ref().is_none_or(|(_, seen)| seen != config) {
            editing.form = Some((
                Box::new(crate::ui::template::PropertiesForm::new(config)),
                config.clone(),
            ));
        }
        let Some((form, seen)) = &mut editing.form else {
            return;
        };
        let (config, compare_clicked) = form.show(ui, compare, &compare_hint);
        if let Some(config) = &config {
            *seen = config.clone();
        }
        if compare_clicked {
            self.toggle_source_compare();
        }
        self.template_folder_buttons(ui);
        let Some(config) = config else {
            return;
        };
        let page = self.doc.page_mm();
        crate::template::edit::set_config(&mut self.doc, config);
        // Moved with their shapes, linked positions are not changes by hand.
        self.applied.clear();
        if page != self.doc.page_mm() {
            self.canvas.fit_pending = true;
        }
        self.dirty = true;
    }

    /// The Template properties panel's buttons about the template's folder: Open folder,
    /// and Delete... in red (it asks first).
    pub(super) fn template_folder_buttons(&mut self, ui: &mut egui::Ui) {
        let keys = self.shortcuts.texts(ui.ctx());
        let tip =
            |hint: &str, command: Command| crate::ui::shortcuts::with_key(hint, keys.get(&command));
        let can_delete = self.edited_user_template().is_some();
        let can_open = self.edited_template_folder().is_some();
        let (mut open, mut delete) = (false, false);
        ui.add_space(8.0);
        // At the panel's right edge, Delete last.
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                delete = ui
                    .add_enabled(can_delete, danger_button(tr!("DELETE_TEMPLATE_ELLIPSIS")))
                    .on_hover_text(tip(tr!("DELETE_TEMPLATE_HINT"), Command::DeleteTemplate))
                    .on_disabled_hover_text(tr!("DELETE_TEMPLATE_UNSAVED_HINT"))
                    .clicked();
                open = ui
                    .add_enabled(
                        can_open,
                        egui::Button::new(tr!("OPEN_TEMPLATE_FOLDER_BUTTON")),
                    )
                    .on_hover_text(tip(
                        tr!("OPEN_TEMPLATE_FOLDER_HINT"),
                        Command::OpenTemplateFolder,
                    ))
                    .on_disabled_hover_text(tr!("OPEN_TEMPLATE_FOLDER_UNSAVED_HINT"))
                    .clicked();
            },
        );
        if open {
            self.open_template_folder();
        }
        if delete {
            self.ask_delete_template();
        }
    }

    /// What the template diagnostic finds in the template being edited, checked again
    /// after each change.
    pub(super) fn edited_problems(&mut self) -> Vec<Problem> {
        let Some(editing) = &mut self.editing else {
            return Vec::new();
        };
        let key = self.history.key();
        match &editing.check {
            Some((checked, problems)) if checked == key => problems.clone(),
            _ => {
                let problems = crate::template::check::check_document(&self.doc);
                editing.check = Some((key.to_owned(), problems.clone()));
                problems
            }
        }
    }

    /// The Template check panel: what makes the template being edited invalid, errors
    /// first; a click on a problem about a shape selects it.
    /// It is checked again after every change; Check, at its bottom, checks it again now.
    pub(super) fn template_check_panel(&mut self, ui: &mut egui::Ui) {
        let problems = self.edited_problems();
        if let Some(id) = problem_list(ui, &problems, self.selected) {
            self.selected = Some(id);
        }
        ui.add_space(6.0);
        let mut check = false;
        ui.vertical_centered(|ui| {
            check = ui
                .button(tr!("CHECK_TEMPLATE_BUTTON"))
                .on_hover_text(tr!("CHECK_TEMPLATE_AGAIN_HINT"))
                .clicked();
        });
        if check && let Some(editing) = &mut self.editing {
            editing.check = None;
        }
    }

    /// The template the one being edited was made from, when it is installed: the
    /// provided one it is a copy of, or else the one its `source` names (by vendor, if
    /// any, and name).
    pub(super) fn template_source_entry(&self) -> Option<&TemplateEntry> {
        let editing = self.editing.as_ref()?;
        if let Some(e) = &editing.copy_of {
            return Some(e);
        }
        let source = crate::template::edit::config(&self.doc)?.source.as_ref()?;
        self.templates.iter().find(|t| {
            editing.id.as_ref() != Some(&t.id)
                && t.name.eq_ignore_ascii_case(source.name.trim())
                && t.vendor.as_deref().map(str::trim).map(str::to_lowercase)
                    == source
                        .vendor
                        .as_deref()
                        .map(str::trim)
                        .map(str::to_lowercase)
        })
    }

    /// Starts or ends comparing the template being edited with its source on the canvas.
    pub(super) fn toggle_source_compare(&mut self) {
        let Some(editing) = &self.editing else {
            return;
        };
        if editing.comparing.is_some() {
            if let Some(editing) = &mut self.editing {
                editing.comparing = None;
            }
            return;
        }
        let Some(entry) = self.template_source_entry() else {
            return;
        };
        let name =
            template::TemplateSource::of(entry).map_or_else(|| entry.full_name(), |s| s.describe());
        match template::load(entry) {
            Ok(t) => {
                if let Some(editing) = &mut self.editing {
                    editing.comparing = Some((name, t));
                }
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// While the template being edited is compared with its source: a bar at the bottom
    /// of the canvas to end it; Esc ends it too.
    pub(super) fn source_compare_bar(&mut self, ctx: &egui::Context) {
        if !self.editing.as_ref().is_some_and(|e| e.comparing.is_some()) {
            return;
        }
        let rect = self.canvas.rect;
        let mut end = false;
        egui::Area::new(egui::Id::unique("source_compare_bar"))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(rect.center_bottom() - egui::vec2(0.0, 16.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let accent = ui.visuals().selection.bg_fill;
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(10))
                    .stroke(egui::Stroke::new(crate::ui::tokens::STROKE, accent))
                    .show(ui, |ui| {
                        ui.spacing_mut().button_padding = egui::vec2(16.0, 8.0);
                        end = ui
                            .button(egui::RichText::new(tr!("END_COMPARE")).size(16.0))
                            .on_hover_text(crate::ui::shortcuts::with_key(
                                tr!("END_COMPARE_HINT"),
                                Some(&"Esc".to_owned()),
                            ))
                            .clicked();
                    });
            });
        // Not while typing in a field, which Esc leaves.
        end |= !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if end {
            self.toggle_source_compare();
        }
    }

    /// Shows the Template check panel of the template being edited.
    pub(super) fn show_template_check(&mut self) {
        if self.editing.is_some() {
            self.panels.set_open(PanelId::TemplateCheck, true);
        }
    }

    /// The panels left out now.
    pub(super) fn hidden_panels(&self) -> &'static [PanelId] {
        if self.editing.is_some() {
            &Self::HIDDEN_WHILE_EDITING
        } else {
            &Self::ONLY_WHILE_EDITING
        }
    }

    /// The template "Edit template" opens: the installed one the project uses, or its
    /// own copy when it isn't installed.
    pub(super) fn template_to_edit(&self) -> Option<&TemplateEntry> {
        self.doc
            .template
            .as_deref()
            .and_then(|id| self.installed(id))
            .or_else(|| self.current_entry())
    }

    /// The name of a copy of the template `name` of `vendor`: "Copy of name", or "Copy
    /// (2) of name" and so on, so that no two templates of the vendor are listed alike.
    pub(super) fn copy_name(&self, name: &str, vendor: Option<&str>) -> String {
        let key = |v: Option<&str>| v.map(|v| v.trim().to_lowercase());
        let taken = |n: &str| {
            self.templates
                .iter()
                .any(|t| key(t.vendor.as_deref()) == key(vendor) && t.name.eq_ignore_ascii_case(n))
        };
        let first = tr!("TEMPLATE_COPY_NAME", name = name).to_owned();
        if !taken(&first) {
            return first;
        }
        (2..)
            .map(|n| tr!("TEMPLATE_COPY_NAME_NUMBERED", name = name, n = n).to_owned())
            .find(|c| !taken(c))
            .expect("some number is free")
    }

    /// Edits the project's template in the window. A provided template, or one the
    /// project alone has, is copied into the user's templates when saved, named as a copy.
    /// A copy is edited only once the user agreed to and named it (`copy_name`): asked
    /// first.
    pub(super) fn edit_template(&mut self, copy_name: Option<String>) {
        let Some(entry) = self.template_to_edit() else {
            return;
        };
        let user = entry.origin == template::Origin::User
            && self
                .installed(&entry.id)
                .is_some_and(|e| e.folder == entry.folder);
        if !user && copy_name.is_none() {
            let provided = entry.origin == template::Origin::Provided
                && self
                    .installed(&entry.id)
                    .is_some_and(|e| e.folder == entry.folder);
            self.template_dialog = Some(crate::ui::template::Dialog::EditCopy {
                name: entry.full_name(),
                copy: self.copy_name(&entry.name, entry.vendor.as_deref()),
                provided,
            });
            return;
        }
        let mut config = entry.config.clone();
        let (id, copy_of) = if user {
            (Some(entry.id.clone()), None)
        } else {
            config.name = copy_name;
            config.vendor = entry.vendor.clone();
            // Said where it comes from (when that is fully known); its own versions
            // start from its source's.
            config.source = template::TemplateSource::of(entry);
            config.version = Some(
                entry
                    .version
                    .clone()
                    .filter(|v| template::is_valid_version(v))
                    .unwrap_or_else(|| template::FIRST_VERSION.to_owned()),
            );
            (None, Some(entry.clone()))
        };
        let doc = match crate::template::edit::open(entry, config) {
            Ok(doc) => doc,
            Err(e) => {
                self.messages.push(e);
                return;
            }
        };
        // A reference whose file couldn't be read: said, and shown crossed out.
        if let Some(l) = crate::template::edit::reference(&doc).filter(|l| l.broken) {
            self.messages.push(tr!(
                "TEMPLATE_REFERENCE_NOT_LOADED",
                path = l.source.display()
            ));
        }
        let from = self.doc.template.clone();
        self.start_template_editing(id, copy_of, from, doc);
    }

    /// Starts a new template named `name`: an empty outline to draw in.
    pub(super) fn new_template(
        &mut self,
        _ctx: &egui::Context,
        name: String,
        vendor: Option<String>,
    ) {
        let config = template::TemplateConfig {
            name: Some(name),
            vendor,
            version: Some(template::FIRST_VERSION.to_owned()),
            ..Default::default()
        };
        self.start_template_editing(
            None,
            None,
            None,
            crate::template::edit::new_document(config),
        );
    }

    /// Puts the project aside and edits `doc`, the template of folder `id`.
    pub(super) fn start_template_editing(
        &mut self,
        id: Option<String>,
        copy_of: Option<TemplateEntry>,
        from: Option<String>,
        doc: Document,
    ) {
        if self.editing.is_some() {
            return;
        }
        let config_before = crate::template::edit::config(&doc)
            .cloned()
            .unwrap_or_default();
        let last_saved = id
            .as_deref()
            .and_then(|id| self.installed(id))
            .and_then(|e| modified_time(&e.config_path()));
        let history = History::new(&doc);
        let project = std::mem::replace(&mut self.doc, doc);
        let history = std::mem::replace(&mut self.history, history);
        self.editing = Some(TemplateEditing {
            id,
            copy_of,
            from,
            config_before,
            form: None,
            saved: false,
            project,
            history,
            selected: self.selected.take(),
            confirm_end: false,
            then_quit: false,
            check: None,
            comparing: None,
            last_saved,
        });
        self.after_switching_documents();
    }

    /// Writes the template being edited into its folder of the user's templates (a new
    /// one into a new folder, named after it).
    pub(super) fn save_template_edit(&mut self) {
        let Some(editing) = &self.editing else {
            return;
        };
        let Some(dir) = self.user_templates_dir.clone() else {
            self.messages
                .push(tr!("NO_USER_TEMPLATES_FOLDER").to_owned());
            return;
        };
        let Some(config) = crate::template::edit::config(&self.doc).cloned() else {
            return;
        };
        if config.name.is_none() {
            self.messages.push(tr!("TEMPLATE_NAME_REQUIRED").to_owned());
            return;
        }
        let id = editing.id.clone().unwrap_or_else(|| {
            let taken: Vec<String> = self.templates.iter().map(|t| t.id.clone()).collect();
            template::unique_folder_name(&edited_name(&self.doc), &taken, &dir)
        });
        // A provided template: its files copied first, so what the editor doesn't
        // write is kept.
        let folder = match (&editing.id, &editing.copy_of) {
            (None, Some(source)) => {
                let taken: Vec<String> = self.templates.iter().map(|t| t.id.clone()).collect();
                let name = config.name.clone().unwrap_or_default();
                match source.duplicate(&dir, &name, config.vendor.as_deref(), &taken) {
                    Ok(folder) => folder,
                    Err(e) => {
                        self.messages.push(e);
                        return;
                    }
                }
            }
            _ => dir.join(&id),
        };
        let id = folder
            .file_name()
            .map_or(id, |f| f.to_string_lossy().into_owned());
        match crate::template::edit::save(&self.doc, &folder) {
            Ok(saved) => {
                // The files it lists now: the next save deletes those it no longer does.
                // Its shapes keep their ids.
                if let Some(e) = &mut self.doc.edited_template {
                    e.config = saved.config;
                    e.shape_ids = saved.shape_ids;
                }
                self.history.mark_saved(&self.doc);
                if let Some(editing) = &mut self.editing {
                    editing.id = Some(id);
                    editing.copy_of = None;
                    editing.saved = true;
                    editing.last_saved = Some(SystemTime::now());
                }
                // Saved anyway: an unfinished template is worth keeping. Said, with
                // what is wrong in sight.
                let errors = self
                    .edited_problems()
                    .iter()
                    .filter(|p| p.severity == Severity::Error)
                    .count();
                if errors > 0 {
                    self.messages
                        .push(tr_n!("TEMPLATE_SAVED_WITH_ERRORS", errors));
                    self.show_template_check();
                }
                // Listed at once, a new folder too: Delete and Open folder find it. The
                // project's template is checked again when editing ends.
                self.templates = template::scan_all(
                    self.templates_dir.as_deref(),
                    self.user_templates_dir.as_deref(),
                );
                self.dirty = self.is_modified();
            }
            Err(e) => {
                // A copy made just now goes again: the next save copies anew.
                if self.editing.as_ref().is_some_and(|e| e.id.is_none()) {
                    let _ = std::fs::remove_dir_all(&folder);
                }
                self.messages.push(e)
            }
        }
    }

    /// Ends editing the template, first asking whether to save its unsaved changes.
    pub(super) fn end_template_editing(&mut self) {
        if self.dirty {
            if let Some(editing) = &mut self.editing {
                editing.confirm_end = true;
            }
        } else {
            self.end_template_editing_now();
        }
    }

    /// Ends editing the template, saved or not: the project comes back. When it uses
    /// the template saved, it takes the saved version (Undo goes back).
    pub(super) fn end_template_editing_now(&mut self) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let config = crate::template::edit::config(&self.doc)
            .cloned()
            .unwrap_or_default();
        self.doc = editing.project;
        self.history = editing.history;
        self.selected = editing.selected;
        self.after_switching_documents();
        self.rescan_templates();
        // The project uses the template saved: the saved version, or the copy made of a
        // provided one.
        if editing.saved
            && editing.id.is_some()
            && editing.from.is_some()
            && self.doc.template == editing.from
        {
            if editing.from == editing.id {
                self.use_installed_template();
            } else {
                self.select_template(editing.id.clone());
            }
            if config.page != editing.config_before.page {
                self.use_template_page();
            }
        }
        // Closing the window ended it: on with closing, the project now.
        self.quit_after_editing = editing.then_quit;
    }

    /// The bar under the menus while a template is edited: which one, and its buttons.
    pub(super) fn template_bar(&mut self, ui: &mut egui::Ui) {
        let Some(editing) = &self.editing else {
            return;
        };
        let name = edited_name(&self.doc);
        let accent = ui.visuals().selection.bg_fill;
        let on_accent = ui.visuals().selection.stroke.color;
        let copy_of = editing.copy_of.as_ref().map(|e| e.full_name());
        let mut close = false;
        egui::Frame::NONE
            .fill(accent)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tr!("EDITING_TEMPLATE", name = name))
                            .strong()
                            .color(on_accent),
                    );
                    if let Some(source) = &copy_of {
                        ui.label(
                            egui::RichText::new(tr!("EDITING_TEMPLATE_COPY", name = source))
                                .color(on_accent),
                        )
                        .on_hover_text(tr!("EDITING_TEMPLATE_COPY_HINT"));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui
                            .button(tr!("CLOSE_TEMPLATE"))
                            .on_hover_text(tr!("CLOSE_TEMPLATE_HINT"))
                            .clicked();
                    });
                });
            });
        if close {
            self.end_template_editing();
        }
    }

    /// The question asked before ending template editing with unsaved changes: save,
    /// don't save or cancel.
    pub(super) fn template_end_modal(&mut self, ctx: &egui::Context) {
        if !self.editing.as_ref().is_some_and(|e| e.confirm_end) {
            return;
        }
        let name = edited_name(&self.doc);
        let (mut save, mut discard, mut cancel) = (false, false, false);
        let response = dialog(ctx, "confirm_end_template").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.strong(tr!("CONFIRM_SAVE_TEMPLATE", name = name));
            ui.add_space(4.0);
            crate::ui::callout::callout(
                ui,
                crate::ui::callout::Tone::Warning,
                tr!("UNSAVED_TEMPLATE_WARNING"),
            );
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                save |= ui.button(tr!("SAVE")).clicked();
                cancel |= ui.button(tr!("CANCEL")).clicked();
                at_left_edge(ui, |ui| {
                    discard |= ui.add(danger_button(tr!("DONT_SAVE"))).clicked();
                });
            });
        });
        cancel |= response.should_close();
        if let Some(editing) = &mut self.editing
            && (save || discard || cancel)
        {
            editing.confirm_end = false;
            if cancel {
                editing.then_quit = false;
            }
        }
        if save {
            self.save_template_edit();
            // Saved: done; failed: still editing, the message says why.
            if !self.dirty {
                self.end_template_editing_now();
            }
        } else if discard {
            self.end_template_editing_now();
        }
    }

    /// The user template being edited, as installed: one that can be deleted.
    pub(super) fn edited_user_template(&self) -> Option<&TemplateEntry> {
        let id = self.editing.as_ref()?.id.as_deref()?;
        self.installed(id)
            .filter(|e| e.origin == template::Origin::User)
    }

    /// The folder of the template being edited: its saved one in your templates, or the
    /// provided template it is a copy of until saved. `None` for a new one not saved yet.
    pub(super) fn edited_template_folder(&self) -> Option<&Path> {
        let editing = self.editing.as_ref()?;
        match editing.id.as_deref() {
            Some(id) => self.installed(id).map(|e| e.folder.as_path()),
            None => editing.copy_of.as_ref().map(|e| e.folder.as_path()),
        }
    }

    /// Shows the folder of the template being edited in the system's file manager.
    pub(super) fn open_template_folder(&mut self) {
        if let Some(folder) = self.edited_template_folder()
            && let Err(e) = crate::ui::file_dialog::open_folder(folder)
        {
            self.messages.push(e);
        }
    }

    /// Deletes the user template being edited, and ends editing it. Projects that use it
    /// keep their own copy of it.
    pub(super) fn delete_template(&mut self, replacement: Option<String>) -> Result<(), String> {
        let entry = self
            .edited_user_template()
            .ok_or_else(|| tr!("NO_TEMPLATE_TO_DELETE").to_owned())?;
        entry.delete()?;
        let in_use = self.project_uses_edited_template();
        if let Some(editing) = &mut self.editing {
            // Nothing left for the project to take.
            editing.saved = false;
        }
        self.end_template_editing_now();
        // The project used it: the template chosen in the dialog now, with its page, or
        // none (Undo goes back).
        if in_use {
            let page = replacement.is_some();
            self.select_template(replacement);
            if page {
                self.use_template_page();
            }
        }
        Ok(())
    }

    /// The project set aside uses the saved template being edited.
    pub(super) fn project_uses_edited_template(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|e| e.id.is_some() && e.project.template == e.id)
    }

    /// The question asked before deleting the template being edited.
    pub(super) fn ask_delete_template(&mut self) {
        let in_use = self.project_uses_edited_template();
        let Some(entry) = self.edited_user_template() else {
            return;
        };
        // What the project may use instead: any other template.
        let others = in_use.then(|| {
            self.templates
                .iter()
                .filter(|t| t.id != entry.id)
                .cloned()
                .collect()
        });
        self.template_dialog = Some(crate::ui::template::Dialog::Delete {
            name: entry.full_name(),
            others,
            replacement: None,
        });
    }

    /// Opens the dialog to choose the reference of the template being edited.
    pub(super) fn pick_template_reference(&mut self, ctx: &egui::Context) {
        if self.editing.is_none() {
            return;
        }
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::TemplateReference,
                title: tr!("TEMPLATE_REFERENCE_DIALOG_TITLE"),
                save: false,
                multiple: false,
                filters: REFERENCE_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Makes the file at `path` the reference of the template being edited, in place of
    /// the one it had, and selects it.
    pub(super) fn set_template_reference(&mut self, ctx: &egui::Context, path: &Path) {
        if self.editing.is_none() {
            return;
        }
        let id = self.doc.alloc_id();
        let mut layer = match crate::template::reference::layer(id, path) {
            Ok(layer) => layer,
            Err(e) => return self.messages.push(e),
        };
        if let Err(e) = Self::load_image(ctx, &mut layer) {
            return self.messages.push(e);
        }
        self.selected = Some(crate::template::edit::set_reference(&mut self.doc, layer));
        self.dirty = true;
    }

    /// The Template panel's Edit button, next to the template field. A provided template
    /// is copied into the user's templates when saved.
    pub(super) fn edit_template_button(&mut self, ui: &mut egui::Ui) {
        let Some(entry) = self.template_to_edit() else {
            return;
        };
        let user = entry.origin == template::Origin::User;
        let keys = self.shortcuts.texts(ui.ctx());
        let hint = if user {
            tr!("EDIT_TEMPLATE_HINT")
        } else {
            tr!("EDIT_PROVIDED_TEMPLATE_HINT")
        };
        if ui
            .button(tr!("EDIT_TEMPLATE_BUTTON"))
            .on_hover_text(crate::ui::shortcuts::with_key(
                hint,
                keys.get(&Command::EditTemplate),
            ))
            .clicked()
        {
            self.run_command(ui.ctx(), Command::EditTemplate);
        }
    }
}
