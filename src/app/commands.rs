//! The menu bar, keyboard shortcuts and the commands they run.

use super::*;

/// The key of the toast saying new plugin versions were found.
const PLUGIN_UPDATES_TOAST: &str = "plugin_updates";
/// The key of the toast saying a newer version of the app is out.
const APP_UPDATE_TOAST: &str = "app_update";

impl ArtworkApp {
    /// File, Edit and View menus.
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr!("MENU_FILE"), |ui| {
                let ctx = ui.ctx().clone();
                let keys = self.shortcuts.texts(&ctx);
                // While a template is edited, only Save (it) and Quit.
                let editing = self.editing.is_some();
                let item = |ui: &mut egui::Ui, label: &str, c: Command| {
                    let can = !editing || Self::works_while_editing(c);
                    ui.add_enabled(can, menu_entry(label, keys.get(&c)))
                        .clicked()
                };
                if item(ui, tr!("NEW_PROJECT"), Command::NewProject) {
                    self.leave(&ctx, Leave::New);
                }
                if item(ui, tr!("OPEN_PROJECT"), Command::OpenProject) {
                    self.leave(&ctx, Leave::Open);
                }
                ui.add_enabled_ui(!editing && !self.recent.paths().is_empty(), |ui| {
                    ui.menu_button(tr!("MENU_OPEN_RECENT"), |ui| self.recent_menu(ui));
                });
                if item(ui, tr!("SAVE"), Command::Save) {
                    self.run_command(&ctx, Command::Save);
                }
                if item(ui, tr!("SAVE_PROJECT_AS"), Command::SaveAs) {
                    self.save_project(&ctx, true);
                }
                ui.menu_button(tr!("MENU_AUTOSAVE"), |ui| self.autosave_menu(ui))
                    .response
                    .on_hover_text(tr!("AUTOSAVE_HINT"));
                ui.separator();
                if item(ui, tr!("EXPORT"), Command::Export) {
                    self.open_export();
                }
                if item(ui, tr!("PRINT"), Command::Print) {
                    self.open_print();
                }
                ui.separator();
                if item(ui, tr!("QUIT"), Command::Quit) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(tr!("MENU_EDIT"), |ui| {
                let ctx = ui.ctx().clone();
                let keys = self.shortcuts.texts(&ctx);
                let undo = menu_entry(tr!("UNDO"), keys.get(&Command::Undo));
                if ui.add_enabled(self.history.can_undo(), undo).clicked() {
                    self.undo();
                }
                let redo = menu_entry(tr!("REDO"), keys.get(&Command::Redo));
                if ui.add_enabled(self.history.can_redo(), redo).clicked() {
                    self.redo();
                }
                ui.separator();
                let has = self.selected.is_some();
                let entry =
                    |key: &'static str, c: Command| menu_entry(translate(key), keys.get(&c));
                if ui
                    .add_enabled(has, entry(tr_noop!("CUT_LAYER"), Command::Cut))
                    .clicked()
                {
                    self.edit(&ctx, Edit::Cut);
                }
                if ui
                    .add_enabled(has, entry(tr_noop!("COPY_LAYER"), Command::Copy))
                    .clicked()
                {
                    self.edit(&ctx, Edit::Copy);
                }
                let paste = entry(tr_noop!("PASTE_LAYER"), Command::Paste);
                if ui
                    .add_enabled(self.clipboard.has_content(), paste)
                    .clicked()
                {
                    self.edit(&ctx, Edit::Paste);
                }
                let duplicate = entry(tr_noop!("DUPLICATE_LAYER"), Command::Duplicate);
                if ui.add_enabled(has, duplicate).clicked() {
                    self.dirty |= layers::duplicate(&mut self.doc, &mut self.selected);
                }
                if ui
                    .add_enabled(has, entry(tr_noop!("DELETE_LAYER"), Command::Delete))
                    .clicked()
                {
                    self.dirty |= layers::delete(&mut self.doc, &mut self.selected);
                }
            });
            ui.menu_button(tr!("MENU_VIEW"), |ui| {
                let keys = self.shortcuts.texts(ui.ctx());
                ui.menu_button(tr!("MENU_PANELS"), |ui| self.panels_menu(ui));
                ui.menu_button(tr!("MENU_THEME"), crate::ui::theme::menu);
                ui.menu_button(tr!("MENU_LANGUAGE"), |ui| {
                    i18n::menu(ui, &mut self.language)
                });
                let entry = menu_entry(
                    tr!("MENU_KEYBOARD_SHORTCUTS"),
                    keys.get(&Command::KeyboardShortcuts),
                );
                if ui.add(entry).clicked() {
                    self.open_shortcuts();
                }
            });
            ui.menu_button(tr!("MENU_TOOLS"), |ui| {
                let keys = self.shortcuts.texts(ui.ctx());
                let c = Command::EyedropperTool;
                let entry = menu_entry(c.label(), keys.get(&c));
                if ui.add_enabled(self.editing.is_none(), entry).clicked() {
                    self.run_command(ui.ctx(), c);
                    ui.close();
                }
                ui.menu_button(tr!("MENU_TEMPLATES"), |ui| {
                    let keys = self.shortcuts.texts(ui.ctx());
                    let idle = self.editing.is_none();
                    // Properties and Delete are about the template being edited.
                    for (command, can) in [
                        (Command::NewTemplate, idle),
                        (
                            Command::EditTemplate,
                            idle && self.template_to_edit().is_some(),
                        ),
                        (
                            Command::DeleteTemplate,
                            self.edited_user_template().is_some(),
                        ),
                        (
                            Command::OpenTemplateFolder,
                            self.edited_template_folder().is_some(),
                        ),
                        (Command::CheckTemplate, !idle),
                        (Command::ChooseReference, !idle),
                        (
                            Command::CompareTemplateSource,
                            self.template_source_entry().is_some(),
                        ),
                    ] {
                        let entry = menu_entry(command.label(), keys.get(&command));
                        if ui.add_enabled(can, entry).clicked() {
                            self.run_command(ui.ctx(), command);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui
                        .button(tr!("RESCAN_TEMPLATES"))
                        .on_hover_text(tr!("RESCAN_TEMPLATES_HINT"))
                        .clicked()
                    {
                        self.rescan_templates();
                        ui.close();
                    }
                });
                ui.menu_button(tr!("MENU_PLUGINS"), |ui| {
                    let keys = self.shortcuts.texts(ui.ctx());
                    let c = Command::ManagePlugins;
                    if ui.add(menu_entry(c.label(), keys.get(&c))).clicked() {
                        self.run_command(ui.ctx(), c);
                        ui.close();
                    }
                });
            });
            ui.menu_button(tr!("MENU_HELP"), |ui| {
                let keys = self.shortcuts.texts(ui.ctx());
                let entry = menu_entry(tr!("MENU_ABOUT"), keys.get(&Command::About));
                if ui.add(entry).clicked() {
                    self.about = true;
                }
            });
        });
    }

    /// View > Panels: each panel shown or hidden, and the default layout.
    pub(super) fn panels_menu(&mut self, ui: &mut egui::Ui) {
        for id in PanelId::ALL {
            if self.hidden_panels().contains(&id) {
                continue;
            }
            let mut open = self.panels.is_open(id);
            if ui.checkbox(&mut open, id.title()).changed() {
                self.panels.set_open(id, open);
            }
        }
        ui.separator();
        if ui.button(tr!("RESET_PANEL_LAYOUT")).clicked() {
            self.panels.reset();
            ui.close();
        }
    }

    /// Keyboard shortcuts. Only the File ones work while a text field has focus. Off
    /// while the Keyboard shortcuts window is open, which takes the keys.
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.shortcuts_editor.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        for command in self.shortcuts.pressed(ctx, typing) {
            self.run_command(ctx, command);
        }
        if typing {
            return;
        }
        // egui turns Ctrl+X, C and V (and their Windows alternatives) into events.
        let edits: Vec<_> = ctx.input_mut(|i| {
            let mut edits = Vec::new();
            i.events.retain(|e| {
                match e {
                    egui::Event::Cut => edits.push((Edit::Cut, None)),
                    egui::Event::Copy => edits.push((Edit::Copy, None)),
                    egui::Event::Paste(text) => edits.push((Edit::Paste, Some(text.clone()))),
                    _ => return true,
                }
                false
            });
            edits
        });
        for (edit, text) in edits {
            match text {
                Some(text) => self.paste(ctx, &text),
                None => self.edit(ctx, edit),
            }
        }
        // The arrow keys nudge; they cannot be changed.
        let arrows = ctx.input(|i| {
            let step = if i.modifiers.shift { 10.0 } else { 1.0 };
            let mut d = [0.0f32, 0.0];
            for (key, v) in [
                (Key::ArrowLeft, [-step, 0.0]),
                (Key::ArrowRight, [step, 0.0]),
                (Key::ArrowUp, [0.0, -step]),
                (Key::ArrowDown, [0.0, step]),
            ] {
                if i.key_pressed(key) {
                    d = [d[0] + v[0], d[1] + v[1]];
                }
            }
            d
        });
        // The layer list clicked last: Up and Down select the row above or below.
        let mut arrows = arrows;
        if self.layers_panel.list_focused && !self.point_picked() && arrows[1] != 0.0 {
            if layers::step_selection(&self.doc, &mut self.selected, arrows[1] > 0.0) {
                self.layers_panel.scroll_to_selected = true;
            }
            arrows[1] = 0.0;
        }
        if arrows != [0.0, 0.0] {
            self.dirty |= if self.point_picked() {
                points::nudge_selected(&self.canvas.points, &mut self.doc, arrows)
            } else {
                // A position linked to a variable stays where its formula puts it.
                let linked = layers::linked_axes(&self.doc, self.selected);
                let d = [0, 1].map(|k| if linked[k] { 0.0 } else { arrows[k] });
                d != [0.0, 0.0] && layers::nudge(&mut self.doc, self.selected, d)
            };
        }
    }

    /// While a shape's points are edited, Delete and the arrow keys act on the point
    /// picked last.
    pub(super) fn point_picked(&self) -> bool {
        self.canvas
            .points
            .as_ref()
            .is_some_and(|e| e.selected.is_some())
    }

    /// The template has shapes with a center to snap to: closed ones.
    pub(super) fn has_centers(&self) -> bool {
        self.template
            .as_ref()
            .is_some_and(|t| t.elements.iter().any(|e| !e.centers.is_empty()))
    }

    /// The command does something while a template is edited.
    pub(super) fn works_while_editing(command: Command) -> bool {
        !matches!(
            command,
            Command::NewProject
                | Command::OpenProject
                | Command::SaveAs
                | Command::Export
                | Command::Print
                | Command::AddImages
                | Command::AddText
                | Command::AddCanvas
                | Command::AddBackground
                | Command::Group
                | Command::Ungroup
                | Command::Clip
                | Command::ShowTemplate
                | Command::PreviewCuts
                | Command::PreviewBleed
                | Command::PreviewSafeArea
                | Command::SnapToCenters
                | Command::NewTemplate
                | Command::EditTemplate
                | Command::PencilTool
                | Command::BrushTool
                | Command::EraserTool
                | Command::EyedropperTool
                | Command::ClearPaint
        )
    }

    /// Does what a shortcut's command does.
    pub(super) fn run_command(&mut self, ctx: &egui::Context, command: Command) {
        // While a template is edited: Save saves it; the commands about the project, its
        // template, images, texts and paint do nothing.
        if self.editing.is_some() {
            if command == Command::Save {
                self.save_template_edit();
                return;
            }
            if !Self::works_while_editing(command) {
                return;
            }
        }
        match command {
            Command::NewProject => self.leave(ctx, Leave::New),
            Command::OpenProject => self.leave(ctx, Leave::Open),
            Command::Save => self.save_project(ctx, false),
            Command::SaveAs => self.save_project(ctx, true),
            Command::Export => self.open_export(),
            Command::Print => self.open_print(),
            // The app's window, also from a panel's own window.
            Command::Quit => {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close)
            }
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::Duplicate => {
                self.dirty |= layers::duplicate(&mut self.doc, &mut self.selected);
            }
            Command::Delete => {
                self.dirty |= if self.point_picked() {
                    points::delete_selected(&mut self.canvas.points, &mut self.doc)
                } else {
                    layers::delete(&mut self.doc, &mut self.selected)
                };
            }
            Command::AddImages => self.pick_images(ctx),
            Command::AddText => self.add_text(ctx),
            Command::AddShape => self.add_shape(ctx, self.layers_panel.shape_kind),
            Command::AddCanvas => self.add_canvas(ctx, self.layers_panel.canvas_kind),
            Command::AddBackground => self.add_background(),
            Command::Group => self.dirty |= layers::group(&mut self.doc, &mut self.selected),
            Command::Ungroup => {
                self.dirty |= layers::ungroup(&mut self.doc, &mut self.selected);
            }
            Command::Clip => self.dirty |= layers::toggle_clip(&mut self.doc, self.selected),
            Command::ZoomIn => self.canvas.zoom_centered(ZOOM_STEP),
            Command::ZoomOut => self.canvas.zoom_centered(1.0 / ZOOM_STEP),
            Command::FitPage => self.canvas.fit_pending = true,
            Command::RealSize => self.canvas.set_zoom(REAL_SIZE_ZOOM),
            Command::ShowTemplate => self.canvas.show_template ^= true,
            Command::PreviewCuts => self.canvas.preview_cuts ^= true,
            Command::PreviewBleed => self.canvas.preview_bleed ^= true,
            Command::PreviewSafeArea => self.canvas.preview_safe_area ^= true,
            Command::SnapToCenters => {
                if self.has_centers() {
                    self.canvas.snap_to_centers ^= true;
                }
            }
            Command::SnapToEdges => self.canvas.snap_to_edges ^= true,
            Command::SnapToGuides => self.canvas.snap_to_guides ^= true,
            Command::ShowRulers => self.canvas.show_rulers ^= true,
            Command::ShowGuides => self.canvas.show_guides ^= true,
            Command::LockGuides => self.canvas.lock_guides ^= true,
            Command::ClearGuides => self.dirty |= canvas::clear_guides(&mut self.doc),
            Command::KeyboardShortcuts => self.open_shortcuts(),
            Command::NewTemplate => {
                self.template_dialog = Some(crate::ui::template::Dialog::New {
                    name: String::new(),
                    vendor: String::new(),
                });
            }
            Command::EditTemplate => self.edit_template(None),
            Command::DeleteTemplate => self.ask_delete_template(),
            Command::OpenTemplateFolder => self.open_template_folder(),
            Command::CheckTemplate => self.show_template_check(),
            Command::CompareTemplateSource => self.toggle_source_compare(),
            Command::ChooseReference => self.pick_template_reference(ctx),
            Command::About => self.about = true,
            Command::SelectTool => {
                self.canvas.brushes.tool = None;
                self.canvas.eyedropper = false;
            }
            Command::EyedropperTool => self.canvas.eyedropper = true,
            Command::ClearPaint => self.dirty |= canvas::clear_paint(&mut self.doc, self.selected),
            Command::ManagePlugins => {
                self.plugin_manager = Some(crate::ui::plugins::Manager::new(None));
            }
            // Only while a canvas is selected, to paint in.
            Command::PencilTool | Command::BrushTool | Command::EraserTool => {
                if canvas::can_paint(&self.doc, self.selected) {
                    self.canvas.eyedropper = false;
                    self.canvas.brushes.tool = canvas::TOOLS
                        .iter()
                        .find(|(c, _)| *c == command)
                        .and_then(|(_, t)| *t);
                }
            }
            // Handled as clipboard events and arrow keys.
            Command::Cut | Command::Copy | Command::Paste | Command::Nudge => {}
        }
    }

    pub(super) fn open_shortcuts(&mut self) {
        self.shortcuts_editor = Some(crate::ui::shortcuts::Editor::new(&self.shortcuts));
    }

    /// Opens the plugin manager at the start when Python plugins wait to be allowed,
    /// after the recovery questions.
    pub(super) fn offer_plugins(&mut self) {
        if self.plugins_offered
            || !self.orphans.is_empty()
            || self.confirm_leave.is_some()
            || self.leave_after_save.is_some()
        {
            return;
        }
        self.plugins_offered = true;
        if !crate::render::effects::registry().pending().is_empty() {
            self.plugin_manager = Some(crate::ui::plugins::Manager::new(None));
        }
    }

    /// New plugin versions found at startup: a toast says so, its Review opening the
    /// plugin manager on its Installed tab, the new versions highlighted. A plugin manager
    /// already open just shows them. Waits for `offer_plugins`, which may open one.
    pub(super) fn plugin_updates(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.plugin_check else {
            return;
        };
        if !check.is_finished() || !self.plugins_offered {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            return;
        }
        let check = self.plugin_check.take().unwrap();
        let (catalog, n) = check.join().unwrap_or_default();
        if n == 0 {
            return;
        }
        if let Some(manager) = &mut self.plugin_manager {
            manager.set_catalog(catalog);
            return;
        }
        self.plugin_updates = Some((catalog, n));
        let text = tr_n!("PLUGIN_UPDATES_TEXT", n);
        let title = tr!("PLUGIN_UPDATES_TITLE");
        let toast = Toast::new(PLUGIN_UPDATES_TOAST, Intent::Info, title, &text);
        self.toasts.push(toast.action(tr!("REVIEW_PLUGIN_UPDATES")));
    }

    /// A newer version of the app found at startup: a toast says so, its Download button
    /// opening the download page.
    pub(super) fn app_update(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.version_check else {
            return;
        };
        if !check.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
            return;
        }
        let check = self.version_check.take().unwrap();
        if let Some(version) = check.join().ok().flatten() {
            let text = tr!("APP_UPDATE_TEXT", version = version);
            let toast = Toast::new(
                APP_UPDATE_TOAST,
                Intent::Info,
                tr!("APP_UPDATE_TITLE"),
                &text,
            );
            self.toasts.push(toast.action(tr!("DOWNLOAD_APP_UPDATE")));
        }
    }

    /// The toasts, their bottom right corner at `corner`, and what their buttons do.
    pub(super) fn toasts(&mut self, ctx: &egui::Context, corner: egui::Pos2) {
        for (key, _) in self.toasts.show(ctx, corner) {
            if key == APP_UPDATE_TOAST {
                ctx.open_url(egui::OpenUrl::new_tab(version_check::DOWNLOAD_PAGE));
            }
            if key == PLUGIN_UPDATES_TOAST
                && let Some((catalog, _)) = self.plugin_updates.take()
            {
                // One opened since keeps what it is doing.
                match &mut self.plugin_manager {
                    Some(manager) => manager.set_catalog(catalog),
                    None => {
                        let manager = crate::ui::plugins::Manager::new(Some(catalog));
                        self.plugin_manager = Some(manager);
                    }
                }
            }
            // Still the open project's template (another may have been opened since),
            // still not installed.
            if key == project_template::TEMPLATE_INSTALL_TOAST
                && let Some(id) = self.install_toast.take()
                && self.doc.template.as_ref() == Some(&id)
                && self.installed(&id).is_none()
            {
                self.install_project_template(&id);
            }
        }
        if !self.toasts.is_up(PLUGIN_UPDATES_TOAST) {
            self.plugin_updates = None;
        }
        if !self.toasts.is_up(project_template::TEMPLATE_INSTALL_TOAST) {
            self.install_toast = None;
        }
    }

    /// The plugin manager, while open.
    pub(super) fn plugins_window(&mut self, ctx: &egui::Context) {
        let Some(manager) = &mut self.plugin_manager else {
            return;
        };
        match manager.show(ctx) {
            crate::ui::plugins::Outcome::Open => {}
            crate::ui::plugins::Outcome::Closed => self.plugin_manager = None,
            crate::ui::plugins::Outcome::Restart => {
                // Closed as by the window's close button: unsaved changes are asked
                // about first, and canceling that cancels the restart.
                self.plugin_manager = None;
                self.restarting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            crate::ui::plugins::Outcome::Error(e) => self.messages.push(e),
        }
    }

    /// The Keyboard shortcuts window, while open. OK uses the shortcuts it shows.
    pub(super) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.shortcuts_editor else {
            return;
        };
        match editor.show(ctx) {
            crate::ui::shortcuts::Outcome::Open => {}
            crate::ui::shortcuts::Outcome::Apply(s) => {
                self.shortcuts = s;
                self.shortcuts_editor = None;
            }
            crate::ui::shortcuts::Outcome::Cancel => self.shortcuts_editor = None,
        }
    }
}
