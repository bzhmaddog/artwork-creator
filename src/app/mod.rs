mod add;
mod commands;
mod dialogs;
mod file;
mod output;
mod panels;
mod project_template;
mod template_edit;
mod version_check;
pub use dialogs::set_interaction;
pub(crate) use dialogs::{at_left_edge, danger_button, dialog, dialog_button_row, dialog_buttons};
use dialogs::{menu_entry, padded, panel_grid};
use file::modified_time;
use project_template::problem_list;
pub(crate) use project_template::template_selector;
use template_edit::edited_name;

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, ColorImage, Key, TextureFilter, TextureOptions};

use crate::document::clipboard::Clipboard;
use crate::document::history::History;
use crate::document::shape::{self, ShapeKind};
use crate::document::text::{self, FontLibrary};
use crate::document::variables;
use crate::document::{self, Document, Layer, Node, Orientation, P2, PageSize, TemplateCopy};
use crate::export::{self, ExportSettings};
use crate::files::project;
use crate::files::recent::{self, RecentFiles};
use crate::i18n::{self, tr, tr_n, tr_noop, translate};
use crate::render::effects::cache::EffectCache;
use crate::render::{self, PlacedElement, Scene};
use crate::template::check::{Problem, Severity};
use crate::template::{self, ElementKind, Template, TemplateEntry};
use crate::ui::canvas::points;
use crate::ui::canvas::{self, CanvasState, REAL_SIZE_ZOOM};
use crate::ui::file_dialog::{self, FileBrowser, Filter, Purpose, Request};
use crate::ui::layers::{self, Edit, LayersPanel};
use crate::ui::panels::{PanelId, Side};
use crate::ui::shortcuts::{Command, Shortcuts};
use crate::ui::toast::{Intent, Toast};
use crate::ui::variables as variables_ui;

/// Longest side of the on-screen texture. Exports always use the full-resolution image.
const MAX_TEXTURE_SIDE: u32 = 4096;
/// Longest side of the copy used to preview semi-transparent groups on screen.
const MAX_PREVIEW_SIDE: u32 = 2048;
/// Width of the preview in the Export window, in points.
const EXPORT_PREVIEW_WIDTH: f32 = 360.0;
/// Outline of the sheets over the export preview, apart from the template element colors.
const TILE_COLOR: Color32 = crate::ui::tokens::ACCENT;
/// How much the zoom buttons and keys zoom in or out at a time.
const ZOOM_STEP: f32 = 1.25;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const IMAGE_FILTERS: &[Filter] = &[Filter::new(tr_noop!("FILTER_IMAGES"), IMAGE_EXTENSIONS)];
/// SVG files, whose shapes are imported as path layers (see `svg`).
const SVG_EXTENSIONS: &[&str] = &["svg", "svgz"];
const SVG_FILTERS: &[Filter] = &[Filter::new(tr_noop!("FILTER_SVG"), SVG_EXTENSIONS)];
const REFERENCE_FILTERS: &[Filter] = &[Filter::new(
    tr_noop!("FILTER_TEMPLATE_REFERENCE"),
    crate::template::reference::EXTENSIONS,
)];
const EXPORT_FILTERS: &[Filter] = &[Filter::new("PNG / PDF / SVG", &["png", "pdf", "svg"])];
/// Project files, to open and save.
const PROJECT_FILTERS: &[Filter] = &[Filter::new(
    tr_noop!("FILTER_ARTWORK_PROJECT"),
    &[project::EXTENSION],
)];

/// Background export thread; yields the written files or an error.
type ExportJob = JoinHandle<Result<Vec<PathBuf>, String>>;

/// The application: the open project, its template, and all UI state.
pub struct ArtworkApp {
    doc: Document,
    templates: Vec<TemplateEntry>,
    /// The provided templates' folder, next to the executable.
    templates_dir: Option<PathBuf>,
    /// The user's templates folder, in the app's data folder.
    user_templates_dir: Option<PathBuf>,
    /// Duplicating, changing or deleting a template.
    template_dialog: Option<crate::ui::template::Dialog>,
    /// A template being edited instead of the project.
    editing: Option<TemplateEditing>,
    /// Template editing was ended to close the window: close it, asking about the
    /// project first.
    quit_after_editing: bool,
    /// The template in use, loaded from the project's copy of it.
    template: Option<Template>,
    /// That copy, unpacked and read as an installed template is.
    template_entry: Option<TemplateEntry>,
    /// The fingerprint of the installed template of the same name, when there is one.
    installed_fingerprint: Option<String>,
    /// An installed version the user chose not to use: it is not offered again.
    kept_fingerprint: Option<String>,
    /// An installed version the user will decide about later: no popup for it, only a
    /// note in the Template panel.
    postponed_fingerprint: Option<String>,
    /// The template of the project just opened, which is not installed: the user is
    /// asked whether to install the project's copy of it.
    offer_install: Option<String>,
    /// The template the toast offering to install it is up for.
    install_toast: Option<String>,
    /// The installed template, loaded to compare it with the project's copy on the canvas.
    comparing: Option<(TemplateEntry, Template)>,
    canvas: CanvasState,
    layers_panel: LayersPanel,
    selected: Option<u64>,
    project_path: Option<PathBuf>,
    /// The document differs from the saved project; follows `history`.
    dirty: bool,
    history: History,
    /// The layer or group last cut or copied.
    clipboard: Clipboard,
    /// What linked fields were last set to, to tell when one is changed by hand.
    applied: variables::Applied,
    /// Recently opened or saved projects, for File > Open Recent.
    recent: RecentFiles,
    /// What to export, chosen in the Export and Print windows and saved with the
    /// project, and the print settings, set in the Print panel and kept with the app's
    /// preferences (see `ExportSettings`).
    export_settings: ExportSettings,
    /// The export settings as last saved or opened with the project: their choices of
    /// what to export count as an unsaved change when they differ, without being undo
    /// steps.
    saved_export: ExportSettings,
    export_open: bool,
    export_job: Option<ExportJob>,
    /// Progress of the running export, shared with its thread.
    export_progress: export::Progress,
    export_started: Instant,
    /// The running job prints rather than exports: for its title and messages.
    job_prints: bool,
    /// The Print window is open.
    print_open: bool,
    /// Print was asked for: the system print dialog opens on the next frame, which has
    /// the window to put it over.
    #[cfg(windows)]
    print_pending: bool,
    /// The printers CUPS offers, asked for when the Print window opens.
    #[cfg(not(windows))]
    printers: Option<crate::export::printing::Printers>,
    /// The printer picked in the Print window, and the number of copies.
    #[cfg(not(windows))]
    print_to: (String, u32),
    /// Small render of the export shown in the Export window, with the hash of what it
    /// shows.
    export_preview: Option<(u64, egui::TextureHandle)>,
    /// Layer images with their effects applied, for the screen.
    effect_cache: EffectCache,
    /// Installed fonts, for text layers.
    fonts: FontLibrary,
    /// The outline grown by the bleed, which background layers fill (see `background`),
    /// with the hash of what it was made from.
    background_rings: (u64, Option<Vec<Vec<P2>>>),
    messages: Vec<String>,
    title: String,
    /// Native dialogs when the system has them, otherwise the built-in browser.
    native_dialogs: bool,
    browser: FileBrowser,
    /// A save or export waiting for the user to agree to replace existing files.
    overwrite: Option<Overwrite>,
    /// Modification times of the installed template's files, to notice a change.
    template_stamp: Vec<Option<SystemTime>>,
    last_template_check: Instant,
    /// Interface language: the system's or a chosen one.
    language: i18n::Preference,
    /// Where each panel is: docked on a side, floating or in its own window.
    panels: crate::ui::panels::Layout,
    /// Each command's keys.
    shortcuts: Shortcuts,
    /// The Keyboard shortcuts window, while open.
    shortcuts_editor: Option<crate::ui::shortcuts::Editor>,
    /// Tools > Plugins > Manage plugins, while open.
    plugin_manager: Option<crate::ui::plugins::Manager>,
    /// The plugin manager was opened at the start for Python plugins waiting to be
    /// allowed (once).
    plugins_offered: bool,
    /// Which new versions of the app to tell about (Help > Update channel).
    update_channel: version_check::Channel,
    /// Looking for a newer version of the app, from the start (or a change of channel)
    /// until it's done.
    version_check: Option<JoinHandle<Option<String>>>,
    /// Looking for new plugin versions, from the start until it's done.
    plugin_check:
        Option<std::thread::JoinHandle<(crate::render::effects::repository::Catalog, usize)>>,
    /// New plugin versions found at startup, to ask about: the repositories' lists, and
    /// how many plugins have one.
    plugin_updates: Option<(crate::render::effects::repository::Catalog, usize)>,
    /// The toasts up, at the bottom right of the window.
    toasts: crate::ui::toast::Toasts,
    /// Closing to start again (the plugin manager's Restart); canceled with the close.
    restarting: bool,
    /// The project is about to be closed or replaced with unsaved changes: ask whether
    /// to save them first.
    confirm_leave: Option<Leave>,
    /// Help > About is open.
    about: bool,
    /// What to do once the project is saved (Save in the unsaved changes question, with
    /// a file still to choose).
    leave_after_save: Option<Leave>,
    /// Closing is agreed: let the next close request through.
    closing: bool,
    /// Copies of the project with unsaved changes, for recovery after a crash.
    autosave: crate::files::recovery::Autosave,
    /// Copies left by runs that crashed, offered back one at a time.
    orphans: Vec<crate::files::recovery::Orphan>,
    /// When the project file was last written, for the status bar: by this run, or
    /// before it was opened.
    last_saved: Option<SystemTime>,
}

/// Labels of the rows of the Page, Template and Print panels: the first column of
/// their grids is as wide as the widest, so that the fields line up within a panel and
/// from one to the next.
const PANEL_LABELS: [&str; 11] = [
    tr_noop!("PAGE_BACKGROUND"),
    tr_noop!("EXPORT_RESOLUTION"),
    tr_noop!("EXPORT_MIN_IMAGE_DPI"),
    tr_noop!("EXPORT_MIRROR"),
    tr_noop!("EXPORT_LEAVE_BLANK"),
    tr_noop!("BLEED"),
    tr_noop!("EXPORT_MARKS"),
    tr_noop!("EXPORT_SHEET"),
    tr_noop!("EXPORT_SHEET_MARGIN"),
    tr_noop!("EXPORT_TILE_OVERLAP"),
    tr_noop!("EXPORT_TILE_POSITION"),
];

/// Where the print settings are kept with the app's preferences.
const PRINT_SETTINGS_KEY: &str = "print_settings";

/// A template being edited in the window (see `template::edit`), the project set aside
/// meanwhile.
struct TemplateEditing {
    /// Its folder in the user's templates; `None` until a new template or the copy of a
    /// provided one is first saved.
    id: Option<String>,
    /// The provided template it was opened from: never changed, it is copied into the
    /// user's templates when first saved.
    copy_of: Option<TemplateEntry>,
    /// The project's template it was opened from: the project takes the saved version.
    from: Option<String>,
    /// Its settings when editing began (they are in the edited document).
    config_before: template::TemplateConfig,
    /// The Template properties panel's form, and the settings it was last made from or
    /// changed to: made again when they change otherwise (Undo).
    form: Option<(
        Box<crate::ui::template::PropertiesForm>,
        template::TemplateConfig,
    )>,
    /// It was saved: the templates are read again when editing ends.
    saved: bool,
    /// The project, back when editing ends.
    project: Document,
    history: History,
    selected: Option<u64>,
    /// Ending was asked with unsaved changes: ask whether to save them first.
    confirm_end: bool,
    /// The window closes once editing has ended.
    then_quit: bool,
    /// What the template diagnostic found, with the history key of the document it
    /// checked: checked again when it changes.
    check: Option<(String, Vec<Problem>)>,
    /// The template it was made from, compared with it on the canvas: its name as shown,
    /// and its lines.
    comparing: Option<(String, Template)>,
    /// When it was last saved: its `template.json`'s time for one of the user's
    /// templates; `None` for a new template or a copy not saved yet.
    last_saved: Option<SystemTime>,
}

/// What ends the project in hand, once its unsaved changes are saved or let go.
#[derive(Clone, Debug, PartialEq)]
enum Leave {
    /// Closing the window.
    Close,
    /// File > New.
    New,
    /// File > Open: the dialog to choose a project.
    Open,
    /// Opening this project, from Open Recent.
    OpenPath(PathBuf),
    /// Recovering the first copy left by a crash.
    Recover,
}

/// A save or export held until the user agrees to replace existing files.
struct Overwrite {
    purpose: Purpose,
    /// Where to save or export, as `on_files` resolved it.
    path: PathBuf,
    /// The files that would be replaced.
    existing: Vec<PathBuf>,
}

/// A new canvas' width and height, as a fraction of the page's.
const CANVAS_PAGE_FRACTION: f32 = 0.5;

impl ArtworkApp {
    /// `project` is an optional project file to open, e.g. from the command line.
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<PathBuf>) -> Self {
        set_interaction(&cc.egui_ctx);
        crate::ui::theme::detect_in_background(&cc.egui_ctx);
        let language = i18n::Preference::load(cc.storage);
        let update_channel = version_check::Channel::load(cc.storage);
        i18n::set(language.language());
        let templates_dir = template::templates_dir();
        let user_templates_dir = template::user_templates_dir();
        let templates = template::scan_all(templates_dir.as_deref(), user_templates_dir.as_deref());
        let mut toasts = crate::ui::toast::Toasts::default();
        if templates.is_empty() {
            toasts.push(Toast::new(
                "no_templates",
                Intent::Warning,
                tr!("NO_TEMPLATES_FOUND_TITLE"),
                tr!("NO_TEMPLATES_FOUND"),
            ));
        }
        let mut app = Self {
            doc: Document::default(),
            templates,
            templates_dir,
            user_templates_dir,
            template_dialog: None,
            editing: None,
            quit_after_editing: false,
            template: None,
            template_entry: None,
            installed_fingerprint: None,
            kept_fingerprint: None,
            postponed_fingerprint: None,
            offer_install: None,
            install_toast: None,
            comparing: None,
            canvas: CanvasState::load(cc.storage),
            layers_panel: LayersPanel::default(),
            selected: None,
            project_path: None,
            dirty: false,
            history: History::new(&Document::default()),
            clipboard: Clipboard::default(),
            applied: variables::Applied::new(),
            recent: RecentFiles::load(cc.storage),
            export_settings: cc
                .storage
                .and_then(|s| eframe::get_value::<ExportSettings>(s, PRINT_SETTINGS_KEY))
                .unwrap_or_default()
                .with_choices_of(&ExportSettings::default()),
            saved_export: ExportSettings::default(),
            export_open: false,
            export_job: None,
            export_progress: export::Progress::default(),
            export_started: Instant::now(),
            job_prints: false,
            print_open: false,
            #[cfg(windows)]
            print_pending: false,
            #[cfg(not(windows))]
            printers: None,
            #[cfg(not(windows))]
            print_to: (String::new(), 1),
            export_preview: None,
            effect_cache: EffectCache::default(),
            fonts: FontLibrary::new(),
            background_rings: (0, None),
            messages: Vec::new(),
            title: String::new(),
            native_dialogs: file_dialog::native_available(),
            browser: FileBrowser::default(),
            overwrite: None,
            template_stamp: Vec::new(),
            last_template_check: Instant::now(),
            language,
            panels: crate::ui::panels::Layout::load(cc.storage),
            shortcuts: Shortcuts::load(cc.storage),
            shortcuts_editor: None,
            plugin_manager: None,
            plugins_offered: false,
            update_channel,
            version_check: version_check::start(update_channel),
            plugin_check: crate::ui::plugins::check_updates(),
            plugin_updates: None,
            toasts,
            restarting: false,
            confirm_leave: None,
            about: false,
            leave_after_save: None,
            closing: false,
            autosave: crate::files::recovery::Autosave::new(
                crate::files::recovery::root(),
                cc.storage
                    .and_then(|s| eframe::get_value(s, crate::files::recovery::STORAGE_KEY))
                    .unwrap_or(crate::files::recovery::DEFAULT_INTERVAL),
            ),
            orphans: crate::files::recovery::root()
                .as_deref()
                .map(crate::files::recovery::orphans)
                .unwrap_or_default(),
            last_saved: None,
        };
        log::info!("native file dialogs: {}", app.native_dialogs);
        app.history.reset(&app.doc);
        if let Some(path) = project {
            app.open_project_path(&cc.egui_ctx, path);
        }
        app
    }

    // ---- Undo ----

    /// Steps the project back one change (Ctrl+Z).
    fn undo(&mut self) {
        let before = self.snapshot_info();
        if self.history.undo(&mut self.doc) {
            self.after_restore(before);
        }
    }

    /// Re-applies the last undone change.
    fn redo(&mut self) {
        let before = self.snapshot_info();
        if self.history.redo(&mut self.doc) {
            self.after_restore(before);
        }
    }

    /// Template id, its copy's fingerprint and page size before an undo or redo, so
    /// `after_restore` knows what to refresh.
    fn snapshot_info(&self) -> (Option<String>, Option<String>, P2) {
        (
            self.doc.template.clone(),
            self.copy_fingerprint(),
            self.doc.page_mm(),
        )
    }

    /// The fingerprint of the project's copy of its template.
    fn copy_fingerprint(&self) -> Option<String> {
        self.doc
            .template_copy
            .as_ref()
            .map(|c| c.fingerprint.clone())
    }

    /// Brings the rest of the app in line with a document restored by undo or redo.
    fn after_restore(
        &mut self,
        (template, fingerprint, page): (Option<String>, Option<String>, P2),
    ) {
        // The restored fields were not changed by hand.
        self.applied.clear();
        // The template's variables come from the template loaded, not the step restored.
        if self.editing.is_none() {
            self.doc.template_variables = self
                .template_entry
                .as_ref()
                .map(|e| e.config.variables.clone())
                .unwrap_or_default();
        }
        if self.doc.template != template || self.copy_fingerprint() != fingerprint {
            self.load_template();
        }
        if self.doc.page_mm() != page {
            self.canvas.fit_pending = true;
        }
        if self
            .selected
            .is_some_and(|id| self.doc.path_of(id).is_none())
        {
            self.selected = None;
        }
        self.dirty = self.is_modified();
    }

    /// The project differs from the saved file: its document or its export settings.
    fn is_modified(&self) -> bool {
        if self.editing.is_some() {
            return self.history.is_modified();
        }
        self.history.is_modified() || !self.export_settings.same_choices(&self.saved_export)
    }

    // ---- Templates ----

    // ---- Template editing ----

    /// Panels with no use while a template is edited: its page is in its settings.
    const HIDDEN_WHILE_EDITING: [PanelId; 5] = [
        PanelId::Template,
        PanelId::Page,
        PanelId::View,
        PanelId::Print,
        PanelId::ColorsInUse,
    ];

    /// Panels shown only while a template is edited.
    const ONLY_WHILE_EDITING: [PanelId; 2] = [PanelId::TemplateSettings, PanelId::TemplateCheck];

    /// What follows a change of the document in hand: nothing selected or being drawn,
    /// the panels that fit it, the page in view.
    fn after_switching_documents(&mut self) {
        let editing = self.editing.is_some();
        self.canvas.points = None;
        self.canvas.drawing = None;
        self.canvas.brushes.tool = None;
        self.canvas.eyedropper = false;
        self.layers_panel.edit_points = None;
        self.layers_panel.template_mode = editing;
        self.comparing = None;
        self.applied.clear();
        self.canvas.fit_pending = true;
        self.dirty = self.is_modified();
    }

    /// The template's elements placed on the page.
    fn elements(&self) -> Vec<PlacedElement> {
        // The template being edited is the document itself.
        if self.editing.is_some() {
            return Vec::new();
        }
        self.template
            .as_ref()
            .map(|t| render::placed_elements(&self.doc, t))
            .unwrap_or_default()
    }

    /// Axis-aligned (center, size) of the placed outline on the page, or of the whole
    /// template if it has no outline.
    fn outline_bounds(elements: &[PlacedElement]) -> Option<(P2, P2)> {
        let has_outline = elements.iter().any(|e| e.kind == ElementKind::Outline);
        let mut pts = elements
            .iter()
            .filter(|e| !has_outline || e.kind == ElementKind::Outline)
            .flat_map(|e| &e.polylines)
            .flat_map(|p| &p.points);
        let first = *pts.next()?;
        let (min, max) = pts.fold((first, first), |(mn, mx), p| {
            (
                [mn[0].min(p[0]), mn[1].min(p[1])],
                [mx[0].max(p[0]), mx[1].max(p[1])],
            )
        });
        Some((
            [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0],
            [max[0] - min[0], max[1] - min[1]],
        ))
    }

    // ---- Images ----

    // ---- File dialogs ----

    // ---- Project files ----

    // ---- Export ----

    // ---- UI ----

    /// Brings the layers' on-screen images up to date with their effects.
    fn sync_effects(&mut self, ctx: &egui::Context) {
        // Quick, smaller results while a slider or value is being dragged.
        let quick = ctx.input(|i| i.pointer.any_down());
        // A scale given to a shape by its handles goes into its geometry once the drag
        // is over. Shapes first: texts can follow them.
        shape::sync(ctx, &mut self.doc, !quick);
        text::sync(ctx, &mut self.doc, &self.fonts);
        self.effect_cache.sync(ctx, &mut self.doc, quick);
    }

    // ---- Clipboard ----

    /// Window title: project name, with * when there are unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let name = match &self.editing {
            Some(_) => tr!("EDITING_TEMPLATE", name = edited_name(&self.doc)),
            None => self.project_name(),
        };
        let title = format!(
            "{}{} - Artwork Creator",
            name,
            if self.dirty { "*" } else { "" }
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

impl crate::ui::panels::Contents for ArtworkApp {
    fn contents(&mut self, ui: &mut egui::Ui, id: PanelId) -> Option<f32> {
        self.panel_body(ui, id)
    }

    /// Shortcuts work while a panel's own window has the focus too.
    fn window_frame(&mut self, ctx: &egui::Context) {
        self.shortcuts(ctx);
    }
}

impl eframe::App for ArtworkApp {
    /// Keeps app settings between sessions (eframe also stores the theme and window).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.recent.save(storage);
        self.language.save(storage);
        self.update_channel.save(storage);
        self.panels.save(storage);
        self.shortcuts.save(storage);
        self.save_settings(storage);
        eframe::set_value(
            storage,
            crate::files::recovery::STORAGE_KEY,
            &self.autosave.minutes,
        );
    }

    /// Closing normally: nothing to recover, so the autosave copy goes.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.autosave.end();
        if self.restarting {
            let _ = crate::RESTART.set(self.project_path.clone());
        }
    }

    /// One frame: input, panels, canvas and dialogs, then an undo step if the project
    /// changed.
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(windows)]
        if std::mem::take(&mut self.print_pending) {
            self.choose_printer(frame);
        }
        let ctx = ui.ctx().clone();
        // The Page and Print panels' settings are saved as soon as they change, once
        // the button is let go (not at each step of a slider's drag): written only
        // when one differs from what is stored.
        if !ctx.input(|i| i.pointer.any_down())
            && let Some(storage) = frame.storage_mut()
        {
            self.save_settings(storage);
            storage.flush();
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_dropped(&ctx, dropped);
        }
        self.guard_close(&ctx);
        crate::ui::colors::publish(&ctx, &self.doc, &self.canvas.picked);
        // The template's points on the page, which layer positions can be linked to.
        self.doc.template_points = match (&self.template, &self.editing) {
            (Some(t), None) => {
                let m = self.doc.template_affine(t.size_mm);
                t.elements
                    .iter()
                    .flat_map(|e| &e.points)
                    .map(|(id, p)| (id.clone(), m.apply(*p)))
                    .collect()
            }
            _ => Vec::new(),
        };
        variables_ui::publish(&ctx, &self.doc);
        self.shortcuts(&ctx);
        self.layers_panel.keys = self.shortcuts.texts(&ctx);
        self.canvas.keys = self.layers_panel.keys.clone();
        self.poll_export(&ctx);
        self.watch_template(&ctx);
        // Undo, redo or a reload may have changed effects or images.
        self.sync_effects(&ctx);

        let elements = self.elements();

        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        if self.editing.is_some() {
            egui::Panel::top("template_bar")
                .frame(egui::Frame::NONE)
                .show(ui, |ui| self.template_bar(ui));
        }
        let status = egui::Panel::bottom("status")
            .show(ui, |ui| self.status_bar(ui, &elements))
            .response
            .rect;
        // Taken out while drawn, as it draws the panels with the app's contents.
        let mut panels = std::mem::take(&mut self.panels);
        // Set here, not when editing starts: a panel's button may start it while the
        // layout is taken out.
        panels.set_hidden(self.hidden_panels());
        crate::ui::panels::Layout::set_template_editing(self.editing.is_some());
        panels.begin_frame(ui.available_rect_before_wrap());
        panels.show_dock(ui, Side::Left, self);
        panels.show_dock(ui, Side::Right, self);
        // Over the canvas, under the dialogs.
        panels.show_floating(&ctx, self);
        panels.show_windows(&ctx, self);
        self.panels = panels;

        // Show effect changes made in the layer panel on this frame's canvas.
        self.sync_effects(&ctx);

        // Settings may have moved the template this frame.
        let elements = self.elements();
        // Background layers stay on the outline grown by the bleed.
        if self.editing.is_none() {
            let rings = self.background_rings(&elements);
            crate::document::background::sync(&mut self.doc, rings.as_deref());
        }
        self.layers_panel.min_dpi = self.export_settings.min_image_dpi as f32;
        // The panel is cut along the outline and through every hole, whatever the export
        // prints there.
        self.canvas.cuts = render::Cuts {
            clip_to_outline: true,
            blank_holes: true,
            ..self.cuts()
        };
        // Edit points is turned on and off in Properties as well as on the canvas.
        let panel_points = self.layers_panel.edit_points;
        if panel_points != self.canvas.points.as_ref().map(|e| e.layer) {
            self.canvas.points = panel_points.map(canvas::PointEdit::new);
        }
        // While comparing, the installed template right of the divider, the project's
        // copy, usually older, left of it; in the template editor, the source of the
        // template being edited left of it: before, then after.
        self.canvas.compare = match self.editing.as_ref().map(|e| &e.comparing) {
            Some(Some((name, t))) => Some(canvas::Compare {
                elements: render::placed_elements(&self.doc, t),
                colors: canvas::element_colors(t),
                labels: [
                    tr!("COMPARE_SOURCE_LABEL", name = name),
                    tr!("COMPARE_EDITED_LABEL").to_owned(),
                ],
                on_left: true,
            }),
            Some(None) => None,
            None => self.compared_elements().map(|(elements, colors)| {
                let (project, installed) = self.template_versions();
                canvas::Compare {
                    elements,
                    colors,
                    labels: [
                        tr!("COMPARE_PROJECT_LABEL", version = project),
                        tr!("COMPARE_INSTALLED_LABEL", version = installed),
                    ],
                    on_left: false,
                }
            }),
        };
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.dirty |= canvas::show(
                ui,
                &mut self.doc,
                &elements,
                &mut self.selected,
                &mut self.canvas,
            );
        });
        self.layers_panel.edit_points = self.canvas.points.as_ref().map(|e| e.layer);
        self.panels.drag_overlay(&ctx);

        // The preview both windows show, freed while neither is open.
        if self.export_open || self.print_open {
            self.update_export_preview(&ctx);
        } else {
            self.export_preview = None;
        }
        self.export_window(&ctx);
        self.print_window(&ctx);
        self.export_progress_window(&ctx);
        if let Some((purpose, paths)) = self.browser.show(&ctx) {
            self.on_files(&ctx, purpose, paths, false);
        }
        // Colors saved from a picker become color variables.
        for var in crate::ui::colors::take_saved(&ctx) {
            self.dirty |= crate::document::colors::save(&mut self.doc, var);
        }
        self.overwrite_modal(&ctx);
        self.message_modal(&ctx);
        self.about_modal(&ctx);
        self.template_differs_modal(&ctx);
        self.template_install_offer();
        self.template_dialog_modal(&ctx);
        self.link_position_modal(&ctx);
        self.compare_bar(&ctx);
        self.source_compare_bar(&ctx);
        self.close_modal(&ctx);
        self.template_end_modal(&ctx);
        self.recovery_modal(&ctx);
        self.offer_plugins();
        self.plugin_updates(&ctx);
        self.app_update(&ctx);
        self.shortcuts_window(&ctx);
        self.plugins_window(&ctx);
        self.toasts(&ctx, status.right_top());

        // Record a step once the edit is over: no button held, no text being typed. A
        // shape scaled by a drag that just ended takes its new size first, so the drag
        // and that are one step.
        let finished = !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input();
        // Linked fields follow their variables, and linked colors their color variables,
        // before shapes and texts are laid out.
        variables::apply(&mut self.doc, &mut self.applied, &self.canvas.held_layers());
        crate::document::colors::apply(&mut self.doc);
        crate::document::text_links::apply(&mut self.doc);
        shape::sync(&ctx, &mut self.doc, !ctx.input(|i| i.pointer.any_down()));
        // Texts following a shape edited on the canvas go with it in the same step.
        text::sync(&ctx, &mut self.doc, &self.fonts);
        // A template's shapes take their element's look; its reference's image, rendered
        // when the template was opened, goes to the screen.
        if self.editing.is_some() {
            crate::template::edit::sync(&mut self.doc);
            let reference = crate::template::edit::reference_mut(&mut self.doc)
                .filter(|l| l.texture.is_none() && !l.broken && l.data.is_some());
            if let Some(l) = reference {
                if let Err(e) = Self::load_image(&ctx, l) {
                    l.broken = true;
                    self.messages.push(e);
                }
                ctx.request_repaint();
            }
        }
        self.history.update(&self.doc, finished);
        self.dirty = self.is_modified();
        self.update_title(&ctx);
        let project = crate::files::recovery::Project {
            doc: &self.doc,
            key: self.history.key(),
            export: &self.export_settings,
            path: self.project_path.as_deref(),
            name: self.project_name(),
            modified: self.dirty,
        };
        // Recovery copies are of the project, not of a template being edited.
        if self.editing.is_none()
            && let Some(e) = self.autosave.tick(&ctx, project, !finished)
        {
            let title = tr!("AUTOSAVE_FAILED");
            let toast = Toast::new("autosave_failed", Intent::Error, title, &e);
            self.toasts.push(toast);
        }
    }
}

#[cfg(test)]
mod tests;
