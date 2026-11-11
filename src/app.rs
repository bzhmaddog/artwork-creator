use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, ColorImage, Key, TextureFilter, TextureOptions};

use crate::canvas_points;
use crate::clipboard::Clipboard;
use crate::document::{self, Document, Layer, Node, Orientation, P2, PageSize};
use crate::effects::cache::EffectCache;
use crate::export::{self, ExportSettings};
use crate::file_dialog::{self, FileBrowser, Filter, Purpose, Request};
use crate::history::History;
use crate::i18n::{self, tr, tr_n, tr_noop, translate};
use crate::layers_ui::{self, Edit, LayersPanel};
use crate::panels::{PanelId, Side};
use crate::project;
use crate::recent::{self, RecentFiles};
use crate::render::{self, PlacedElement, Scene};
use crate::shape::{self, ShapeKind};
use crate::shortcuts::{Command, Shortcuts};
use crate::template::{self, ElementKind, Template, TemplateEntry};
use crate::text::{self, FontLibrary};
use crate::variables;
use crate::view::{self, CanvasState, REAL_SIZE_ZOOM};

/// Longest side of the on-screen texture. Exports always use the full-resolution image.
const MAX_TEXTURE_SIDE: u32 = 4096;
/// Longest side of the copy used to preview semi-transparent groups on screen.
const MAX_PREVIEW_SIDE: u32 = 2048;
/// Width of the preview in the Export window, in points.
const EXPORT_PREVIEW_WIDTH: f32 = 360.0;
/// Outline of the sheets over the export preview, apart from the template element colors.
const TILE_COLOR: Color32 = Color32::from_rgb(220, 20, 60);
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const IMAGE_FILTERS: &[Filter] = &[Filter::new(tr_noop!("FILTER_IMAGES"), IMAGE_EXTENSIONS)];
const EXPORT_FILTERS: &[Filter] = &[Filter::new("PNG / PDF", &["png", "pdf"])];
/// Projects that can be opened: archives and linked JSON projects.
const OPEN_PROJECT_FILTERS: &[Filter] = &[Filter::new(
    tr_noop!("FILTER_ARTWORK_PROJECT"),
    &[project::EXTENSION, "json"],
)];
/// The two formats a project can be saved in, the default first.
const SAVE_PROJECT_FILTERS: &[Filter] = &[
    Filter::new(
        tr_noop!("FILTER_ARTWORK_PROJECT_EMBEDDED"),
        &[project::EXTENSION],
    ),
    Filter::new(tr_noop!("FILTER_LINKED_JSON_PROJECT"), &["json"]),
];

/// Background export thread; yields the written files or an error.
type ExportJob = JoinHandle<Result<Vec<PathBuf>, String>>;

/// The application: the open project, its template, and all UI state.
pub struct ArtworkApp {
    doc: Document,
    templates: Vec<TemplateEntry>,
    templates_dir: Option<PathBuf>,
    template: Option<Template>,
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
    printers: Option<crate::printing::Printers>,
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
    messages: Vec<String>,
    title: String,
    /// Native dialogs when the system has them, otherwise the built-in browser.
    native_dialogs: bool,
    browser: FileBrowser,
    /// A save or export waiting for the user to agree to replace existing files.
    overwrite: Option<Overwrite>,
    /// Modification times of the current template's files, to reload on change.
    template_stamp: Vec<Option<SystemTime>>,
    last_template_check: Instant,
    /// Interface language: the system's or a chosen one.
    language: i18n::Preference,
    /// Where each panel is: docked on a side, floating or in its own window.
    panels: crate::panels::Layout,
    /// Each command's keys.
    shortcuts: Shortcuts,
    /// The Keyboard shortcuts window, while open.
    shortcuts_editor: Option<crate::shortcuts::Editor>,
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
}

/// Labels of the rows of the Page, Template and Print panels: the first column of
/// their grids is as wide as the widest, so that the fields line up within a panel and
/// from one to the next.
const PANEL_LABELS: [&str; 13] = [
    tr_noop!("PAGE_BACKGROUND"),
    tr_noop!("TEMPLATE_ROTATION"),
    tr_noop!("TEMPLATE_FLIP"),
    tr_noop!("TEMPLATE_OFFSET"),
    tr_noop!("EXPORT_RESOLUTION"),
    tr_noop!("EXPORT_MIN_IMAGE_DPI"),
    tr_noop!("EXPORT_BLANK_INSIDE"),
    tr_noop!("BLEED"),
    tr_noop!("EXPORT_MARKS"),
    tr_noop!("EXPORT_SHEET"),
    tr_noop!("EXPORT_SHEET_MARGIN"),
    tr_noop!("EXPORT_TILE_OVERLAP"),
    tr_noop!("EXPORT_TILE_POSITION"),
];

/// A two-column grid of labels and fields in the Page, Template or Print panel, its
/// labels' column as wide as the widest of `PANEL_LABELS`.
fn panel_grid(ui: &egui::Ui, id: &str) -> egui::Grid {
    let width = PANEL_LABELS
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
        .spacing([8.0, 4.0])
        .min_col_width(width.ceil())
}

/// Where the print settings are kept with the app's preferences.
const PRINT_SETTINGS_KEY: &str = "print_settings";

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
}

/// A save or export held until the user agrees to replace existing files.
struct Overwrite {
    purpose: Purpose,
    /// Where to save or export, as `on_files` resolved it.
    path: PathBuf,
    /// The files that would be replaced.
    existing: Vec<PathBuf>,
}

/// The file a project is saved to: the extension picks the format; without one of the
/// project extensions, it's an archive.
fn project_save_path(path: PathBuf) -> PathBuf {
    let json = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if json || project::is_archive(&path) {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(project::EXTENSION);
    name.into()
}

/// How the UI reacts to the mouse, for the app and the tests that drive its panels.
pub fn set_interaction(ctx: &egui::Context) {
    // Text fields stop editing as soon as the mouse is pressed anywhere else,
    // not only on a clean click (egui's default ignores presses that turn into drags).
    ctx.options_mut(|o| {
        o.input_options.surrender_focus_on = egui::SurrenderFocusOn::Presses;
    });
    ctx.all_styles_mut(|s| {
        // Labels can't be selected, so the text cursor only shows over what can be edited.
        s.interaction.selectable_labels = false;
    });
}

/// How long a notice stays in the status bar, in seconds.
pub(crate) const NOTICE_SECONDS: f64 = 6.0;

fn notice_id() -> egui::Id {
    egui::Id::unique("status_notice")
}

/// Shows `text` as a warning in the status bar for a few seconds, without stopping
/// what the user is doing (unlike a message, which waits for OK).
pub(crate) fn notify(ctx: &egui::Context, text: String) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(notice_id(), (text, now)));
}

/// The notice shown in the status bar, if one is still up.
pub(crate) fn notice(ctx: &egui::Context) -> Option<String> {
    let (text, at) = ctx.data(|d| d.get_temp::<(String, f64)>(notice_id()))?;
    let left = at + NOTICE_SECONDS - ctx.input(|i| i.time);
    if left <= 0.0 {
        ctx.data_mut(|d| d.remove::<(String, f64)>(notice_id()));
        return None;
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
    Some(text)
}

/// Room between a window's or dialog's border and what it holds, in points (egui's is 6).
const DIALOG_MARGIN: i8 = 14;

/// A menu entry with its command's shortcut, if it has one, at the right.
fn menu_entry<'a>(label: &'a str, shortcut: Option<&String>) -> egui::Button<'a> {
    let b = egui::Button::new(label);
    match shortcut {
        Some(s) => b.shortcut_text(s.as_str()),
        None => b,
    }
}

/// A menu checkbox, with its command's shortcut at the right as entries show theirs.
fn menu_checkbox(
    ui: &mut egui::Ui,
    enabled: bool,
    value: &mut bool,
    label: &str,
    shortcut: Option<&String>,
) -> egui::Response {
    ui.horizontal(|ui| {
        let r = ui.add_enabled(enabled, egui::Checkbox::new(value, label));
        if let Some(s) = shortcut {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(s.as_str())
            });
        }
        r
    })
    .inner
}

/// A modal dialog, framed like a window but with more room: egui frames modals like
/// menus.
pub(crate) fn dialog(ctx: &egui::Context, id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::unique(id)).frame(
        egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(DIALOG_MARGIN)),
    )
}

/// The bottom row of a window: `ok` (enabled when `can`) at the right edge of what is
/// above it, Cancel on its left. Returns which was clicked: (ok, cancel).
pub(crate) fn dialog_buttons(ui: &mut egui::Ui, ok: &str, can: bool) -> (bool, bool) {
    dialog_button_row(ui, |ui| {
        let ok = ui.add_enabled(can, egui::Button::new(ok)).clicked();
        let cancel = ui.button(tr!("CANCEL")).clicked();
        (ok, cancel)
    })
}

/// The bottom row of a window or dialog, as wide as what is above it: `add` lays its
/// buttons out from the right edge, the main one first (the rightmost), then Cancel. A
/// button that should sit at the left edge (such as "Close without saving") goes last, in
/// [`at_left_edge`].
pub(crate) fn dialog_button_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // As wide as the rows above, not more: the window sizes itself to its contents.
    let width = ui.min_rect().width();
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}

/// In a [`dialog_button_row`], a button at the row's left edge, after the others.
pub(crate) fn at_left_edge<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), add)
        .inner
}

/// A window's contents with `DIALOG_MARGIN` of room around them. egui's window margin
/// also sets its title bar's, which stays as it is.
fn padded<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let extra = (DIALOG_MARGIN - ui.spacing().window_margin.left).max(0);
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(extra))
        .show(ui, add)
        .inner
}

/// A new canvas' width and height, as a fraction of the page's.
const CANVAS_PAGE_FRACTION: f32 = 0.5;

impl ArtworkApp {
    /// `project` is an optional project file to open, e.g. from the command line.
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<PathBuf>) -> Self {
        set_interaction(&cc.egui_ctx);
        crate::theme::detect_in_background(&cc.egui_ctx);
        let language = i18n::Preference::load(cc.storage);
        i18n::set(language.language());
        let templates_dir = template::templates_dir();
        let templates = templates_dir
            .as_deref()
            .map(template::scan)
            .unwrap_or_default();
        let mut messages = Vec::new();
        if templates.is_empty() {
            messages.push(tr!("NO_TEMPLATES_FOUND").to_owned());
        }
        let mut app = Self {
            doc: Document::default(),
            templates,
            templates_dir,
            template: None,
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
            messages,
            title: String::new(),
            native_dialogs: file_dialog::native_available(),
            browser: FileBrowser::default(),
            overwrite: None,
            template_stamp: Vec::new(),
            last_template_check: Instant::now(),
            language,
            panels: crate::panels::Layout::load(cc.storage),
            shortcuts: Shortcuts::load(cc.storage),
            shortcuts_editor: None,
            confirm_leave: None,
            about: false,
            leave_after_save: None,
            closing: false,
        };
        log::info!("native file dialogs: {}", app.native_dialogs);
        if let Some(first) = app.templates.first().map(|t| t.id.clone()) {
            app.select_template(Some(first), true);
            app.dirty = false;
        }
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

    /// Template id and page size before an undo or redo, so `after_restore` knows what to
    /// refresh.
    fn snapshot_info(&self) -> (Option<String>, P2) {
        (self.doc.template.clone(), self.doc.page_mm())
    }

    /// Brings the rest of the app in line with a document restored by undo or redo.
    fn after_restore(&mut self, (template, page): (Option<String>, P2)) {
        // The restored fields were not changed by hand.
        self.applied.clear();
        if self.doc.template != template {
            // Keep the restored placement rather than the template's defaults.
            let placement = self.doc.template_placement;
            self.select_template(self.doc.template.clone(), false);
            self.doc.template_placement = placement;
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
        self.history.is_modified() || !self.export_settings.same_choices(&self.saved_export)
    }

    // ---- Templates ----

    /// Selects a template by folder id. With `apply_defaults`, its configured
    /// (or automatic) placement replaces the current one.
    fn select_template(&mut self, id: Option<String>, apply_defaults: bool) {
        self.template = None;
        self.doc.template = id.clone();
        let Some(id) = id else { return };
        let Some(entry) = self.templates.iter().find(|t| t.id == id) else {
            self.messages.push(tr!("TEMPLATE_NOT_INSTALLED", id = id));
            return;
        };
        if let Some(e) = &entry.config_error {
            self.messages.push(tr!("INVALID_TEMPLATE_CONFIG", e = e));
        }
        match template::load(entry) {
            Ok(t) => {
                if apply_defaults {
                    self.doc.template_placement =
                        entry.default_placement(t.size_mm, self.doc.page_mm());
                }
                self.template_stamp = entry.stamp();
                // Line colors set in template.json.
                for e in &t.elements {
                    if let Some([r, g, b]) = e.color {
                        self.canvas.element_color[e.kind.index()] = Color32::from_rgb(r, g, b);
                    }
                }
                self.template = Some(t);
            }
            Err(e) => self.messages.push(e),
        }
        self.dirty = true;
    }

    /// The installed template the project uses, if any.
    fn current_entry(&self) -> Option<&TemplateEntry> {
        let id = self.doc.template.as_ref()?;
        self.templates.iter().find(|t| &t.id == id)
    }

    /// Reloads the current template when its DXF or `template.json` changes on disk.
    fn watch_template(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.last_template_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_template_check = Instant::now();
        let Some(entry) = self.current_entry() else {
            return;
        };
        if entry.stamp() == self.template_stamp {
            return;
        }
        let folder = entry.folder.clone();
        if let Some(fresh) = template::read_entry(&folder)
            && let Some(slot) = self.templates.iter_mut().find(|t| t.id == fresh.id)
        {
            *slot = fresh;
        }
        log::info!("template files changed, reloading {}", folder.display());
        let id = self.doc.template.clone();
        self.select_template(id, true);
    }

    /// The template's elements placed on the page.
    fn elements(&self) -> Vec<PlacedElement> {
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

    /// Decodes a layer's image file into its full-size pixmap, preview pixmap and screen
    /// texture.
    fn load_image(ctx: &egui::Context, layer: &mut Layer) -> Result<(), String> {
        // The original bytes are kept, to be saved into project archives as they are.
        let data = match &layer.data {
            Some(data) => data.clone(),
            None => {
                let bytes = std::fs::read(&layer.source)
                    .map_err(|e| tr!("CANNOT_OPEN_FILE", path = layer.source.display(), e = e))?;
                let data = Arc::new(bytes);
                layer.data = Some(data.clone());
                data
            }
        };
        let img = image::load_from_memory(&data)
            .map_err(|e| tr!("CANNOT_OPEN_FILE", path = layer.source.display(), e = e))?
            .to_rgba8();
        let invalid = || tr!("IMAGE_HAS_NO_PIXELS", path = layer.source.display());
        let (w, h) = img.dimensions();
        layer.size_px = [w, h];
        let display = render::downscale(&img, MAX_TEXTURE_SIDE);
        let color = ColorImage::from_rgba_unmultiplied(
            [display.width() as usize, display.height() as usize],
            display.as_raw(),
        );
        let preview = render::premultiplied_pixmap(&render::downscale(&display, MAX_PREVIEW_SIDE))
            .ok_or_else(invalid)?;
        let options = TextureOptions {
            mipmap_mode: Some(TextureFilter::Linear),
            ..TextureOptions::LINEAR
        };
        layer.texture = Some(ctx.load_texture(format!("layer-{}", layer.id), color, options));
        layer.preview = Some(Arc::new(preview));
        layer.pixmap = Some(Arc::new(
            render::premultiplied_pixmap(&img).ok_or_else(invalid)?,
        ));
        layer.broken = false;
        Ok(())
    }

    /// Where a new text or shape layer goes: the template's center (the page's without
    /// a template), or the middle of the view when that is out of sight.
    fn new_layer_center(&self) -> P2 {
        let center = Self::outline_bounds(&self.elements())
            .map(|(c, _)| c)
            .unwrap_or_else(|| self.doc.page_center());
        self.canvas.in_view(center)
    }

    /// Adds a text layer above the selection, centered on the template, and puts the
    /// keyboard in its text.
    fn add_text(&mut self, ctx: &egui::Context) {
        let id = self.doc.alloc_id();
        let center = self.new_layer_center();
        let layer = text::new_layer(
            id,
            tr!("DEFAULT_TEXT_LAYER_NAME"),
            self.fonts.default_family(),
            center,
        );
        let at = layers_ui::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        self.layers_panel.show_text = true;
        text::sync(ctx, &mut self.doc, &self.fonts);
        // Typed in right away on the canvas, its placeholder text selected.
        if let Some(layer) = self.doc.layer(id) {
            self.canvas.type_in(layer);
        }
    }

    /// Adds a shape layer of `kind` above the selection, centered on the template, or on
    /// the view when the template's center is out of sight (see `new_layer_center`). Lines,
    /// polylines and polygons are drawn point by point on the canvas instead.
    fn add_shape(&mut self, ctx: &egui::Context, kind: ShapeKind) {
        self.layers_panel.shape_kind = kind;
        if kind.has_points() {
            self.canvas.drawing = Some(view::Drawing::new(kind));
            self.canvas.points = None;
            self.layers_panel.edit_points = None;
            return;
        }
        let id = self.doc.alloc_id();
        let center = self.new_layer_center();
        let layer = shape::new_layer(id, kind, center);
        let at = layers_ui::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        shape::sync(ctx, &mut self.doc, true);
    }

    /// Adds a closed shape of `kind` to paint in, without fill or stroke, above the
    /// selection, half the page's size (an ellipse, regular polygon or star: in a square
    /// half its shorter side across) and centered like a new text (see `new_layer_center`), with the Select
    /// tool in hand. A polygon is drawn point by point on the canvas instead, as from
    /// the Shape menu.
    fn add_canvas(&mut self, ctx: &egui::Context, kind: ShapeKind) {
        self.layers_panel.canvas_kind = kind;
        self.canvas.brushes.tool = None;
        if kind.has_points() {
            let mut drawing = view::Drawing::new(kind);
            drawing.canvas = true;
            self.canvas.drawing = Some(drawing);
            self.canvas.points = None;
            self.layers_panel.edit_points = None;
            return;
        }
        let id = self.doc.alloc_id();
        let c = self.new_layer_center();
        let [mut w, mut h] = self.doc.page_mm().map(|v| v * CANVAS_PAGE_FRACTION / 2.0);
        if matches!(
            kind,
            ShapeKind::Ellipse | ShapeKind::RegularPolygon | ShapeKind::Star
        ) {
            // In a square box, as from the Shape menu (a circle, an unsquished star or
            // polygon): its handles stretch it.
            (w, h) = (w.min(h), w.min(h));
        }
        let layer = shape::new_canvas_layer(id, kind, [c[0] - w, c[1] - h], [c[0] + w, c[1] + h]);
        let at = layers_ui::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        self.canvas.drawing = None;
        self.canvas.points = None;
        self.layers_panel.edit_points = None;
        // Selecting and moving first: it is placed before it is painted in.
        self.canvas.brushes.tool = None;
        shape::sync(ctx, &mut self.doc, true);
    }

    /// Adds one layer per image file, fitted to the page, above the selection.
    fn add_images(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        for path in paths {
            let id = self.doc.alloc_id();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| tr!("DEFAULT_IMAGE_LAYER_NAME").into());
            let mut layer = Layer::new(id, name, path);
            match Self::load_image(ctx, &mut layer) {
                Ok(()) => {
                    layer.fit_to(self.doc.page_center(), self.doc.page_mm(), false);
                    let at = layers_ui::insertion_path(&mut self.doc, self.selected);
                    self.doc.insert(&at, Node::Layer(Box::new(layer)));
                    self.selected = Some(id);
                    self.dirty = true;
                }
                Err(e) => self.messages.push(e),
            }
        }
    }

    // ---- File dialogs ----

    /// Asks for files with a native dialog, or with the built-in browser where there is
    /// none.
    fn request_file(&mut self, ctx: &egui::Context, req: Request) {
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
    fn on_files(
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
    fn files_written(&self, purpose: Purpose, path: &Path) -> Vec<PathBuf> {
        match purpose {
            Purpose::Export => self.export_settings.files(path, self.doc.page_mm()),
            _ => vec![path.to_path_buf()],
        }
    }

    /// Saves or exports to `path`, once any overwrite is agreed.
    fn write_files(&mut self, purpose: Purpose, path: PathBuf) {
        match purpose {
            Purpose::SaveProject => self.save_to(path),
            Purpose::Export => self.start_export(path),
            Purpose::AddImages | Purpose::OpenProject => {}
        }
    }

    /// Asks whether to replace the files a save or export would overwrite.
    fn overwrite_modal(&mut self, ctx: &egui::Context) {
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
            crate::callout::callout(
                ui,
                crate::callout::Tone::Warning,
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

    /// Opens the dialog to add image layers.
    fn pick_images(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::AddImages,
                title: tr!("ADD_IMAGES_DIALOG_TITLE"),
                save: false,
                multiple: true,
                filters: IMAGE_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Stores the app's own settings that are not the project's: the Print panel's and
    /// the Page panel's display settings.
    fn save_settings(&self, storage: &mut dyn eframe::Storage) {
        // Without the project's choices of what to export, which aren't the app's.
        let print = self
            .export_settings
            .with_choices_of(&ExportSettings::default());
        eframe::set_value(storage, PRINT_SETTINGS_KEY, &print);
        self.canvas.save(storage);
    }

    // ---- Project files ----

    /// Closes the window or replaces the project, first asking whether to save it when
    /// it has unsaved changes.
    fn leave(&mut self, ctx: &egui::Context, leave: Leave) {
        if self.dirty {
            self.confirm_leave = Some(leave);
        } else {
            self.leave_now(ctx, leave);
        }
    }

    /// Closes the window or replaces the project, unsaved changes or not.
    fn leave_now(&mut self, ctx: &egui::Context, leave: Leave) {
        match leave {
            Leave::Close => self.closing = true,
            Leave::New => self.new_project(),
            Leave::Open => self.open_project(ctx),
            Leave::OpenPath(path) => self.open_project_path(ctx, path),
        }
    }

    /// Starts an empty project, keeping the current template.
    fn new_project(&mut self) {
        let template = self.doc.template.clone();
        self.doc = Document::default();
        self.selected = None;
        self.project_path = None;
        self.select_template(template, true);
        self.canvas.fit_pending = true;
        // The print settings stay: they are the app's.
        self.export_settings = self
            .export_settings
            .with_choices_of(&ExportSettings::default());
        self.saved_export = self.export_settings.clone();
        self.dirty = false;
        self.applied.clear();
        self.history.reset(&self.doc);
    }

    /// Opens the dialog to choose a project file.
    fn open_project(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::OpenProject,
                title: tr!("OPEN_PROJECT_DIALOG_TITLE"),
                save: false,
                multiple: false,
                filters: OPEN_PROJECT_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Loads a project and its images. Missing images are reported and drawn crossed out.
    fn open_project_path(&mut self, ctx: &egui::Context, path: PathBuf) {
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
                let template = self.doc.template.clone();
                self.select_template(template, false);
                self.selected = None;
                self.recent.add(&path);
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

    /// Saves to the current file in its format, or asks for a file and format (always
    /// with `save_as`).
    fn save_project(&mut self, ctx: &egui::Context, save_as: bool) {
        match (&self.project_path, save_as) {
            (Some(p), false) => self.save_to(p.clone()),
            _ => self.request_file(
                ctx,
                Request {
                    purpose: Purpose::SaveProject,
                    title: tr!("SAVE_PROJECT_DIALOG_TITLE"),
                    save: true,
                    multiple: false,
                    filters: SAVE_PROJECT_FILTERS,
                    // Suggests the current format.
                    default_name: format!(
                        "{}.{}",
                        self.project_name(),
                        if self
                            .project_path
                            .as_deref()
                            .is_some_and(|p| !project::is_archive(p))
                        {
                            "json"
                        } else {
                            project::EXTENSION
                        }
                    ),
                },
            ),
        }
    }

    /// Writes the project to `path`, as an archive or as linked JSON depending on its
    /// extension, and marks it as saved.
    fn save_to(&mut self, path: PathBuf) {
        let saved = if project::is_archive(&path) {
            project::save(&self.doc, &self.export_settings, &path)
        } else {
            project::save_json(&self.doc, &self.export_settings, &path)
        };
        match saved {
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
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// Project file name without extensions, or "Untitled". Used in the title and for
    /// exports.
    fn project_name(&self) -> String {
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

    // ---- Export ----

    /// What the export cuts, with its bleed: the print settings'.
    fn cuts(&self) -> render::Cuts {
        self.export_settings.cuts()
    }

    /// What gets exported or printed, with text and shapes drawn for `dpi`.
    fn output_scene(&self, dpi: f32) -> Scene {
        let colors = self.canvas.element_color.map(|c| [c.r(), c.g(), c.b()]);
        // Text and shapes are drawn again at the output resolution.
        let mut doc = self.doc.clone();
        text::prepare_export(&mut doc, dpi);
        shape::prepare_export(&mut doc, dpi);
        let s = &self.export_settings;
        let scene = Scene::new(
            &doc,
            self.template.as_ref(),
            colors,
            s.print_lines,
            self.cuts(),
        )
        .with_marks(s.crop_marks, s.registration_marks);
        if s.mirror { scene.mirrored() } else { scene }
    }

    /// Runs `job` on a background thread, with the progress window.
    fn start_job(
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
    fn start_export(&mut self, base: PathBuf) {
        let scene = self.output_scene(self.export_settings.dpi.clamp(72, 1200) as f32);
        let settings = self.export_settings.clone();
        self.start_job(false, move |progress| {
            export::run(&scene, &settings, &base, progress)
        });
    }

    /// Opens the system print dialog over the app's window, then prints in the
    /// background.
    #[cfg(windows)]
    fn choose_printer(&mut self, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let owner = frame.window_handle().ok().and_then(|h| match h.as_raw() {
            RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
            _ => None,
        });
        let paper = crate::printing::Paper::of(&self.doc, &self.export_settings);
        match crate::printing::choose(owner, &paper) {
            Ok(Some(printer)) => {
                let scene = self.output_scene(crate::printing::MAX_PRINT_DPI);
                let settings = self.export_settings.clone();
                let title = self.project_name();
                self.start_job(true, move |progress| {
                    crate::printing::print(printer, &scene, &settings, &title, None, progress)
                        .map(|()| Vec::new())
                });
            }
            Ok(None) => {}
            Err(e) => self.messages.push(e),
        }
    }

    /// Prints through CUPS in the background.
    #[cfg(not(windows))]
    fn start_print(&mut self) {
        let (printer, copies) = self.print_to.clone();
        let paper = crate::printing::Paper::of(&self.doc, &self.export_settings);
        let scene = self.output_scene(self.export_settings.dpi.clamp(72, 1200) as f32);
        let settings = self.export_settings.clone();
        let title = self.project_name();
        self.start_job(true, move |progress| {
            crate::printing::print(
                &printer, copies, &paper, &scene, &settings, &title, progress,
            )
            .map(|()| Vec::new())
        });
    }

    /// Reports a failed or canceled export once its thread has finished.
    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.export_job else { return };
        if !job.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let result = self.export_job.take().map(|j| j.join());
        match result {
            // Success is silent: the progress window closing is enough.
            Some(Ok(Ok(paths))) => log::info!("exported {paths:?}"),
            Some(Ok(Err(e))) if e == render::CANCELED => self.messages.push(if self.job_prints {
                tr!("PRINT_CANCELED").into()
            } else {
                tr!("EXPORT_CANCELED").into()
            }),
            Some(Ok(Err(e))) if self.job_prints => self.messages.push(tr!("PRINT_FAILED", e = e)),
            Some(Ok(Err(e))) => self.messages.push(tr!("EXPORT_FAILED", e = e)),
            Some(Err(_)) => self.messages.push(tr!("EXPORT_FAILED_UNEXPECTEDLY").into()),
            None => {}
        }
    }

    // ---- UI ----

    /// Entries of File > Open Recent. Projects that are missing (e.g. on an unplugged
    /// drive) stay listed but grayed out.
    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        for path in self.recent.paths() {
            let exists = path.is_file();
            // The format, shown on the right like a shortcut.
            let kind = if project::is_archive(path) {
                tr!("RECENT_FORMAT_ARTWORK")
            } else {
                tr!("RECENT_FORMAT_LINKED_JSON")
            };
            let button = egui::Button::new(recent::label(path)).shortcut_text(kind);
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

    /// File, Edit and View menus.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr!("MENU_FILE"), |ui| {
                let ctx = ui.ctx().clone();
                let keys = self.shortcuts.texts(&ctx);
                let item = |ui: &mut egui::Ui, label: &str, c: Command| {
                    ui.add(menu_entry(label, keys.get(&c))).clicked()
                };
                if item(ui, tr!("NEW_PROJECT"), Command::NewProject) {
                    self.leave(&ctx, Leave::New);
                }
                if item(ui, tr!("OPEN_PROJECT"), Command::OpenProject) {
                    self.leave(&ctx, Leave::Open);
                }
                ui.add_enabled_ui(!self.recent.paths().is_empty(), |ui| {
                    ui.menu_button(tr!("MENU_OPEN_RECENT"), |ui| self.recent_menu(ui));
                });
                if item(ui, tr!("SAVE"), Command::Save) {
                    self.save_project(&ctx, false);
                }
                if item(ui, tr!("SAVE_PROJECT_AS"), Command::SaveAs) {
                    self.save_project(&ctx, true);
                }
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
                    self.dirty |= layers_ui::duplicate(&mut self.doc, &mut self.selected);
                }
                if ui
                    .add_enabled(has, entry(tr_noop!("DELETE_LAYER"), Command::Delete))
                    .clicked()
                {
                    self.dirty |= layers_ui::delete(&mut self.doc, &mut self.selected);
                }
            });
            ui.menu_button(tr!("MENU_VIEW"), |ui| {
                let keys = self.shortcuts.texts(ui.ctx());
                if ui
                    .add(menu_entry(
                        tr!("FIT_PAGE_IN_VIEW"),
                        keys.get(&Command::FitPage),
                    ))
                    .clicked()
                {
                    self.canvas.fit_pending = true;
                }
                if ui
                    .add(menu_entry(
                        tr!("ZOOM_REAL_SIZE"),
                        keys.get(&Command::RealSize),
                    ))
                    .clicked()
                {
                    self.canvas.set_zoom(REAL_SIZE_ZOOM);
                }
                menu_checkbox(
                    ui,
                    true,
                    &mut self.canvas.show_template,
                    tr!("SHOW_TEMPLATE_LINES"),
                    keys.get(&Command::ShowTemplate),
                );
                menu_checkbox(
                    ui,
                    true,
                    &mut self.canvas.preview_cuts,
                    tr!("PREVIEW_CUTS"),
                    keys.get(&Command::PreviewCuts),
                );
                let cuts = self.canvas.preview_cuts;
                menu_checkbox(
                    ui,
                    cuts,
                    &mut self.canvas.preview_bleed,
                    tr!("PREVIEW_BLEED"),
                    keys.get(&Command::PreviewBleed),
                )
                .on_hover_text(tr!("PREVIEW_BLEED_HINT"));
                menu_checkbox(
                    ui,
                    true,
                    &mut self.canvas.preview_safe_area,
                    tr!("PREVIEW_SAFE_AREA"),
                    keys.get(&Command::PreviewSafeArea),
                )
                .on_hover_text(tr!("PREVIEW_SAFE_AREA_HINT"));
                let has_anchors = self.has_anchors();
                menu_checkbox(
                    ui,
                    has_anchors,
                    &mut self.canvas.snap_to_anchors,
                    tr!("SNAP_TO_ANCHORS"),
                    keys.get(&Command::SnapToAnchors),
                )
                .on_hover_text(tr!("SNAP_TO_ANCHORS_HINT"))
                .on_disabled_hover_text(tr!("TEMPLATE_HAS_NO_ANCHORS"));
                menu_checkbox(
                    ui,
                    true,
                    &mut self.canvas.snap_to_edges,
                    tr!("SNAP_TO_EDGES"),
                    keys.get(&Command::SnapToEdges),
                )
                .on_hover_text(tr!("SNAP_TO_EDGES_HINT"));
                ui.separator();
                ui.menu_button(tr!("MENU_PANELS"), |ui| self.panels_menu(ui));
                ui.menu_button(tr!("MENU_THEME"), crate::theme::menu);
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
                // The canvas' tools, as on its tool bar.
                let keys = self.shortcuts.texts(ui.ctx());
                let ctx = ui.ctx().clone();
                // Painting needs a closed shape selected.
                let paint = view::can_paint(&self.doc, self.selected);
                for (c, tool) in crate::view::TOOLS {
                    let on =
                        view::active_tool(&self.doc, self.selected, &self.canvas.brushes) == tool;
                    let entry = menu_entry(c.label(), keys.get(&c)).selected(on);
                    if ui.add_enabled(paint || tool.is_none(), entry).clicked() {
                        self.run_command(&ctx, c);
                    }
                }
                // The brush's kinds, as on the tool bar: picking one takes the brush.
                ui.add_enabled_ui(paint, |ui| {
                    ui.menu_button(tr!("MENU_BRUSH_KIND"), |ui| {
                        for kind in crate::paint::BrushKind::ALL {
                            let on = self.canvas.brushes.brush == kind;
                            let entry = egui::Button::new(crate::i18n::translate(kind.name_key()))
                                .selected(on);
                            if ui.add(entry).clicked() {
                                self.canvas.brushes.brush = kind;
                                self.run_command(&ctx, Command::BrushTool);
                                ui.close();
                            }
                        }
                    });
                });
                let c = Command::ClearPaint;
                let clear = view::has_paint(&self.doc, self.selected);
                if ui
                    .add_enabled(clear, menu_entry(c.label(), keys.get(&c)))
                    .clicked()
                {
                    self.run_command(&ctx, c);
                }
                ui.separator();
                ui.menu_button(tr!("MENU_TEMPLATES"), |ui| {
                    if ui
                        .button(tr!("RESCAN_TEMPLATES"))
                        .on_hover_text(tr!("RESCAN_TEMPLATES_HINT"))
                        .clicked()
                    {
                        self.rescan_templates();
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

    /// Lists the templates folder again and reloads the current template.
    fn rescan_templates(&mut self) {
        self.templates = self
            .templates_dir
            .as_deref()
            .map(template::scan)
            .unwrap_or_default();
        let id = self.doc.template.clone();
        self.select_template(id, true);
    }

    /// The Controller panel: the template to work on.
    fn controller_panel(&mut self, ui: &mut egui::Ui) {
        let current = self
            .current_entry()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| tr!("NO_TEMPLATE").into());
        let mut choice = self.doc.template.clone();
        egui::ComboBox::from_id_salt("template")
            .width(ui.available_width())
            .selected_text(current)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, None, tr!("NO_TEMPLATE"));
                for t in &self.templates {
                    ui.selectable_value(&mut choice, Some(t.id.clone()), &t.name);
                }
            });
        if choice != self.doc.template {
            self.select_template(choice, true);
        }
    }

    /// The Page panel: size, orientation and background.
    fn page_panel(&mut self, ui: &mut egui::Ui) {
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
                    crate::layers_ui::unit(ui, "mm");
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
            if let (Some(t), Some(e)) = (&self.template, self.current_entry()) {
                self.doc.template_placement = e.default_placement(t.size_mm, self.doc.page_mm());
            }
            self.canvas.fit_pending = true;
            self.dirty = true;
        }
        // One color, or a gradient over the whole page; opaque, the page being so.
        let doc = &mut self.doc;
        let [r, g, b] = doc.background;
        let mut color = [r, g, b, 255];
        let mut fill = crate::gradient_ui::Fill {
            color: &mut color,
            link: &mut doc.background_link,
            gradient: &mut doc.background_gradient,
            alpha: false,
        };
        let mut changed = false;
        panel_grid(ui, "page-background").show(ui, |ui| {
            ui.label(tr!("PAGE_BACKGROUND"));
            ui.horizontal(|ui| {
                changed |= crate::gradient_ui::inline(ui, "background", &mut fill);
            });
            ui.end_row();
        });
        changed |= crate::gradient_ui::rows(ui, "background", &mut fill);
        doc.background = [color[0], color[1], color[2]];
        self.dirty |= changed;
        // How the canvas shows what is cut out of the panel: neither the template nor the
        // export changes with it. The slider sets how much it covers.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.canvas.preview_cuts, tr!("PREVIEW_CUTS"))
                .on_hover_text(tr!("PREVIEW_CUTS_HINT"));
            ui.add_enabled(
                self.canvas.preview_cuts,
                egui::Slider::new(&mut self.canvas.cut_opacity, 0.0..=1.0).fixed_decimals(2),
            )
            .on_hover_text(tr!("CUT_PREVIEW_OPACITY_HINT"));
        });
        ui.add_enabled(
            self.canvas.preview_cuts,
            egui::Checkbox::new(&mut self.canvas.preview_bleed, tr!("PREVIEW_BLEED")),
        )
        .on_hover_text(tr!("PREVIEW_BLEED_HINT"));
        // A guide inside the cut lines, never printed, and how far inside: kept with the
        // app's preferences, like the previews above.
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.canvas.preview_safe_area, tr!("PREVIEW_SAFE_AREA"))
                .on_hover_text(tr!("PREVIEW_SAFE_AREA_HINT"));
            ui.add_enabled(
                self.canvas.preview_safe_area,
                egui::DragValue::new(&mut self.canvas.safe_margin_mm)
                    .range(0.0..=crate::print::MAX_MARGIN_MM)
                    .speed(0.1)
                    .fixed_decimals(1),
            )
            .on_hover_text(tr!("SAFE_MARGIN_HINT"));
            crate::layers_ui::unit(ui, "mm");
        });
    }

    /// A panel's contents; the Layers panel returns the height it would take.
    fn panel_body(&mut self, ui: &mut egui::Ui, id: PanelId) -> Option<f32> {
        match id {
            PanelId::Controller => self.controller_panel(ui),
            PanelId::Page => self.page_panel(ui),
            PanelId::Print => self.print_panel(ui),
            PanelId::Template if self.template.is_some() => self.template_settings(ui),
            PanelId::Template => {
                crate::callout::callout(ui, crate::callout::Tone::Info, tr!("NO_TEMPLATE_CHOSEN"));
            }
            PanelId::Variables => self.dirty |= variables::variables_ui(ui, &mut self.doc),
            PanelId::ColorsInUse => self.dirty |= crate::colors::in_use_ui(ui, &mut self.doc),
            PanelId::Layers => {
                self.layers_panel.can_paste = self.clipboard.has_content();
                let out = layers_ui::show(
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
                let out = layers_ui::properties_panel(
                    ui,
                    &mut self.doc,
                    self.selected,
                    &mut self.layers_panel,
                    bounds,
                    &self.fonts,
                );
                self.layers_output(ui.ctx(), out);
            }
        }
        None
    }

    /// Carries out what was asked in the Layers or Properties panel.
    fn layers_output(&mut self, ctx: &egui::Context, out: layers_ui::LayersOutput) {
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
        if let Some(kind) = out.add_shape {
            self.add_shape(ctx, kind);
        }
        if let Some(edit) = out.edit {
            self.edit(ctx, edit);
        }
    }

    /// View > Panels: each panel shown or hidden, and the default layout.
    fn panels_menu(&mut self, ui: &mut egui::Ui) {
        for id in PanelId::ALL {
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

    /// Body of the Template section: placement, elements and default placement.
    fn template_settings(&mut self, ui: &mut egui::Ui) {
        // Taken out while shown, so the rest of `self` can change; put back at the end.
        let Some(t) = self.template.take() else {
            return;
        };
        let size = t.size_mm;
        ui.weak(format!("{:.1} x {:.1} mm", size[0], size[1]));
        let before = self.doc.template_placement;
        let p = &mut self.doc.template_placement;
        panel_grid(ui, "template-placement").show(ui, |ui| {
            ui.label(tr!("TEMPLATE_ROTATION"));
            ui.horizontal(|ui| {
                for deg in [0, 90, 180, 270] {
                    if ui
                        .selectable_label(p.rotation == deg, format!("{deg}°"))
                        .clicked()
                    {
                        p.rotation = deg;
                    }
                }
            });
            ui.end_row();
            ui.label(tr!("TEMPLATE_FLIP"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut p.flip_x, tr!("TEMPLATE_FLIP_HORIZONTAL"));
                ui.checkbox(&mut p.flip_y, tr!("TEMPLATE_FLIP_VERTICAL"));
            });
            ui.end_row();
            ui.label(tr!("TEMPLATE_OFFSET"));
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut p.offset_mm[0])
                        .speed(0.2)
                        .fixed_decimals(1)
                        .prefix("x "),
                );
                ui.add(
                    egui::DragValue::new(&mut p.offset_mm[1])
                        .speed(0.2)
                        .fixed_decimals(1)
                        .prefix("y "),
                );
                crate::layers_ui::unit(ui, "mm");
            });
            ui.end_row();
            // Under the offset it resets.
            ui.label("");
            if ui.small_button(tr!("CENTER_TEMPLATE_ON_PAGE")).clicked() {
                p.offset_mm = [0.0, 0.0];
            }
            ui.end_row();
        });

        ui.add_space(4.0);
        ui.strong(tr!("TEMPLATE_ELEMENTS"));
        for e in &t.elements {
            let k = e.kind.index();
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.canvas.show_template, |ui| {
                    ui.checkbox(&mut self.canvas.element_show[k], "")
                        .on_hover_text(tr!("SHOW_ELEMENT_LINES"));
                });
                ui.color_edit_button_srgba(&mut self.canvas.element_color[k])
                    .on_hover_text(tr!("ELEMENT_LINE_COLOR"));
                ui.label(translate(&e.name));
                ui.weak(if e.kind.has_lines() {
                    tr_n!("SHAPE_COUNT", e.polylines.len())
                } else {
                    tr_n!("ANCHOR_COUNT", e.anchors.len())
                });
            });
        }

        ui.add_space(4.0);
        let page = self.doc.page_mm();
        let current = self.doc.template_placement;
        let default = self
            .current_entry()
            .map(|e| e.default_placement(size, page));
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    default.is_some_and(|d| d != current),
                    egui::Button::new(tr!("RESET_TEMPLATE_PLACEMENT")).small(),
                )
                .on_hover_text(tr!("RESET_TEMPLATE_PLACEMENT_HINT"))
                .clicked()
                && let Some(d) = default
            {
                self.doc.template_placement = d;
            }
            if ui
                .add_enabled(
                    default.is_some_and(|d| d != current),
                    egui::Button::new(tr!("SAVE_TEMPLATE_PLACEMENT")).small(),
                )
                .on_hover_text(tr!("SAVE_TEMPLATE_PLACEMENT_HINT"))
                .clicked()
            {
                let id = self.doc.template.clone();
                if let Some(entry) = self
                    .templates
                    .iter_mut()
                    .find(|t| Some(&t.id) == id.as_ref())
                {
                    match entry.save_placement(current) {
                        Ok(()) => self.template_stamp = entry.stamp(),
                        Err(e) => self.messages.push(e),
                    }
                }
            }
        });
        if let Some(e) = self.current_entry() {
            ui.weak(tr!(
                "TEMPLATE_CONFIG_PATH",
                path = e.config_path().display()
            ));
        }
        if self.doc.template_placement != before {
            self.dirty = true;
        }
        self.template = Some(t);
    }

    /// Bottom bar: zoom, page size, cursor position and warnings.
    fn status_bar(&mut self, ui: &mut egui::Ui, elements: &[PlacedElement]) {
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
                .on_hover_text(crate::shortcuts::with_key(
                    tr!("FIT_PAGE_IN_VIEW"),
                    self.shortcuts.text(ui.ctx(), Command::FitPage).as_ref(),
                ))
                .clicked()
            {
                self.canvas.fit_pending = true;
            }
            if ui
                .small_button("100%")
                .on_hover_text(tr!("ZOOM_REAL_SIZE_HINT"))
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
        });
    }

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

    /// Re-renders the Export window's preview when the project or the export options
    /// changed. Uses the layers' preview images, so it stays quick.
    fn update_export_preview(&mut self, ctx: &egui::Context) {
        let colors = self.canvas.element_color.map(|c| [c.r(), c.g(), c.b()]);
        let s = &self.export_settings;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.doc)
            .unwrap_or_default()
            .hash(&mut h);
        let cuts = self.cuts();
        (colors, s.print_lines, cuts, &self.template_stamp).hash(&mut h);
        (s.crop_marks, s.registration_marks, s.mirror).hash(&mut h);
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
            colors,
            s.print_lines,
            cuts,
        )
        .with_marks(s.crop_marks, s.registration_marks);
        let scene = if s.mirror { scene.mirrored() } else { scene };
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
    fn export_progress_window(&mut self, ctx: &egui::Context) {
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
    fn output_preview(&mut self, ui: &mut egui::Ui) {
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
                        egui::Stroke::new(1.5, TILE_COLOR),
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
    fn output_settings(&mut self, ui: &mut egui::Ui, printing: bool) {
        let s = &mut self.export_settings;
        if !printing {
            ui.horizontal(|ui| {
                ui.label(tr!("EXPORT_FORMATS"));
                ui.checkbox(&mut s.png, "PNG");
                ui.checkbox(&mut s.pdf, "PDF");
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
    fn print_warnings(&self, ui: &mut egui::Ui) {
        let s = &self.export_settings;
        let page_mm = self.doc.page_mm();
        let low = self.doc.low_resolution(s.min_image_dpi as f32);
        if !low.is_empty() {
            let names: Vec<String> = low
                .iter()
                .map(|(l, dpi)| format!("{} ({dpi:.0} DPI)", l.name))
                .collect();
            crate::callout::callout(
                ui,
                crate::callout::Tone::Warning,
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
                crate::print::marks(trim, s.bleed_mm, s.crop_marks, s.registration_marks);
            // Where they print: the sheets lie over the mirrored page.
            if s.mirror {
                for p in marks.iter_mut().flat_map(|m| &mut m.points) {
                    p[0] = page_mm[0] - p[0];
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
            .and_then(crate::print::bounds)
            .is_some_and(|(mn, mx)| {
                mn[0] < lo[0] || mn[1] < lo[1] || mx[0] > hi[0] || mx[1] > hi[1]
            });
        if off {
            crate::callout::callout(
                ui,
                crate::callout::Tone::Warning,
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
    fn print_panel(&mut self, ui: &mut egui::Ui) {
        let template = self.template.as_ref();
        let has = |k: ElementKind| template.is_some_and(|t| t.has(k));
        let s = &mut self.export_settings;
        panel_grid(ui, "print-resolution").show(ui, |ui| {
            ui.label(tr!("EXPORT_RESOLUTION"))
                .on_hover_text(tr!("EXPORT_RESOLUTION_HINT"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.dpi).range(72..=1200))
                    .on_hover_text(tr!("EXPORT_RESOLUTION_HINT"));
                crate::layers_ui::unit(ui, "DPI");
            });
            ui.end_row();
            ui.label(tr!("EXPORT_MIN_IMAGE_DPI"))
                .on_hover_text(tr!("EXPORT_MIN_IMAGE_DPI_HINT"));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.min_image_dpi).range(0..=1200))
                    .on_hover_text(tr!("EXPORT_MIN_IMAGE_DPI_HINT"));
                crate::layers_ui::unit(ui, "DPI");
            });
            ui.end_row();
        });
        ui.checkbox(&mut s.mirror, tr!("EXPORT_MIRROR"))
            .on_hover_text(tr!("EXPORT_MIRROR_HINT"));
        ui.add_space(4.0);
        ui.add_enabled(
            has(ElementKind::Outline) || template.is_none(),
            egui::Checkbox::new(
                &mut s.cuts.clip_to_outline,
                tr!("EXPORT_BLANK_OUTSIDE_OUTLINE"),
            ),
        );
        panel_grid(ui, "print-cuts").show(ui, |ui| {
            ui.label(tr!("EXPORT_BLANK_INSIDE"));
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut s.cuts.blank_buttons,
                    tr!("EXPORT_BLANK_INSIDE_BUTTONS"),
                );
                ui.checkbox(
                    &mut s.cuts.blank_cutouts,
                    tr!("EXPORT_BLANK_INSIDE_CUTOUTS"),
                );
            });
            ui.end_row();
            ui.label(tr!("BLEED")).on_hover_text(tr!("BLEED_HINT"));
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut s.bleed_mm)
                        .range(0.0..=crate::print::MAX_MARGIN_MM)
                        .speed(0.1)
                        .fixed_decimals(1),
                )
                .on_hover_text(tr!("BLEED_HINT"));
                crate::layers_ui::unit(ui, "mm");
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
                    crate::layers_ui::unit(ui, "mm");
                });
                ui.end_row();
                ui.label(tr!("EXPORT_TILE_OVERLAP"))
                    .on_hover_text(tr!("EXPORT_TILE_OVERLAP_HINT"));
                ui.horizontal(|ui| {
                    ui.add(mm(&mut s.tile_overlap_mm))
                        .on_hover_text(tr!("EXPORT_TILE_OVERLAP_HINT"));
                    crate::layers_ui::unit(ui, "mm");
                });
                ui.end_row();
                ui.label(tr!("EXPORT_TILE_POSITION"))
                    .on_hover_text(tr!("EXPORT_TILE_POSITION_HINT"));
                ui.horizontal(|ui| {
                    for (v, prefix) in s.tile_offset_mm.iter_mut().zip(["x ", "y "]) {
                        ui.add(
                            egui::DragValue::new(v)
                                .speed(0.5)
                                .fixed_decimals(1)
                                .prefix(prefix),
                        )
                        .on_hover_text(tr!("EXPORT_TILE_POSITION_HINT"));
                    }
                    crate::layers_ui::unit(ui, "mm");
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
    fn export_window(&mut self, ctx: &egui::Context) {
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
                            let can = (s.png || s.pdf) && self.export_job.is_none();
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
    fn open_export(&mut self) {
        self.export_open = true;
        self.print_open = false;
    }

    /// Opens the Print window, in place of the Export window; on Linux, asks CUPS for its
    /// printers again, in case one was added.
    fn open_print(&mut self) {
        self.print_open = true;
        self.export_open = false;
        #[cfg(not(windows))]
        {
            let printers = crate::printing::printers();
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
    fn print_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let s = &self.export_settings;
        let paper = crate::printing::Paper::of(&self.doc, s);
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
                crate::callout::callout(
                    ui,
                    crate::callout::Tone::Warning,
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
            crate::callout::callout(
                ui,
                crate::callout::Tone::Info,
                tr!("PRINT_SYSTEM_DIALOG_HINT"),
            );
            self.export_job.is_none()
        }
    }

    /// The Print window: the preview and the export settings, as in the Export window,
    /// then what will be printed and where.
    fn print_window(&mut self, ctx: &egui::Context) {
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
                            crate::callout::callout(
                                ui,
                                crate::callout::Tone::Info,
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

    /// Help > About: the name, version and build date.
    fn about_modal(&mut self, ctx: &egui::Context) {
        if !self.about {
            return;
        }
        let response = dialog(ctx, "about").show(ctx, |ui| {
            ui.set_min_width(260.0);
            ui.heading("Artwork Creator");
            ui.add_space(4.0);
            ui.label(tr!("ABOUT_VERSION", version = env!("CARGO_PKG_VERSION")));
            ui.label(tr!("ABOUT_BUILD_DATE", date = env!("BUILD_DATE")));
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| ui.button(tr!("OK")).clicked())
        });
        if response.inner || response.should_close() {
            self.about = false;
        }
    }

    /// Shows queued messages (errors, a canceled export) one at a time.
    fn message_modal(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.messages.first().cloned() else {
            return;
        };
        let response = dialog(ctx, "message").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(msg);
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| ui.button(tr!("OK")).clicked())
        });
        if response.inner || response.should_close() {
            self.messages.remove(0);
        }
    }

    // ---- Clipboard ----

    /// Cuts or copies the selection onto the clipboard. Paste asks for the system
    /// clipboard's text, which comes back next frame as a paste event, as with Ctrl+V.
    fn edit(&mut self, ctx: &egui::Context, edit: Edit) {
        let text = match edit {
            Edit::Cut => {
                let text = self.clipboard.cut(&mut self.doc, &mut self.selected);
                self.dirty |= text.is_some();
                text
            }
            Edit::Copy => self.clipboard.copy(&self.doc, self.selected),
            Edit::Paste => {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                None
            }
        };
        if let Some(text) = text {
            ctx.copy_text(text);
        }
    }

    /// Pastes the layers the system clipboard's `text` holds, if it holds any. Layers
    /// copied in another window come without their images, which are loaded here.
    fn paste(&mut self, ctx: &egui::Context, text: &str) {
        let Some(pasted) = self
            .clipboard
            .paste(&mut self.doc, &mut self.selected, text)
        else {
            return;
        };
        self.dirty = true;
        if !pasted.foreign {
            return;
        }
        let mut missing = Vec::new();
        if let Some(node) = self.doc.node_mut(pasted.id) {
            node.for_each_layer_mut(&mut |layer| {
                if !layer.is_drawn() && Self::load_image(ctx, layer).is_err() {
                    layer.broken = true;
                    missing.push(layer.source.display().to_string());
                }
            });
        }
        if !missing.is_empty() {
            self.messages
                .push(tr!("IMAGES_NOT_LOADED", files = missing.join("\n")));
        }
    }

    /// Keyboard shortcuts. Only the File ones work while a text field has focus. Off
    /// while the Keyboard shortcuts window is open, which takes the keys.
    fn shortcuts(&mut self, ctx: &egui::Context) {
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
        if arrows != [0.0, 0.0] {
            self.dirty |= if self.point_picked() {
                canvas_points::nudge_selected(&self.canvas.points, &mut self.doc, arrows)
            } else {
                layers_ui::nudge(&mut self.doc, self.selected, arrows)
            };
        }
    }

    /// While a shape's points are edited, Delete and the arrow keys act on the point
    /// picked last.
    fn point_picked(&self) -> bool {
        self.canvas
            .points
            .as_ref()
            .is_some_and(|e| e.selected.is_some())
    }

    /// The template has anchors to snap to.
    fn has_anchors(&self) -> bool {
        self.template
            .as_ref()
            .is_some_and(|t| t.has(ElementKind::Anchors))
    }

    /// Does what a shortcut's command does.
    fn run_command(&mut self, ctx: &egui::Context, command: Command) {
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
                self.dirty |= layers_ui::duplicate(&mut self.doc, &mut self.selected);
            }
            Command::Delete => {
                self.dirty |= if self.point_picked() {
                    canvas_points::delete_selected(&mut self.canvas.points, &mut self.doc)
                } else {
                    layers_ui::delete(&mut self.doc, &mut self.selected)
                };
            }
            Command::AddImages => self.pick_images(ctx),
            Command::AddText => self.add_text(ctx),
            Command::AddShape => self.add_shape(ctx, self.layers_panel.shape_kind),
            Command::AddCanvas => self.add_canvas(ctx, self.layers_panel.canvas_kind),
            Command::Group => self.dirty |= layers_ui::group(&mut self.doc, &mut self.selected),
            Command::Ungroup => {
                self.dirty |= layers_ui::ungroup(&mut self.doc, &mut self.selected);
            }
            Command::FitPage => self.canvas.fit_pending = true,
            Command::RealSize => self.canvas.set_zoom(REAL_SIZE_ZOOM),
            Command::ShowTemplate => self.canvas.show_template ^= true,
            Command::PreviewCuts => self.canvas.preview_cuts ^= true,
            Command::PreviewBleed => self.canvas.preview_bleed ^= true,
            Command::PreviewSafeArea => self.canvas.preview_safe_area ^= true,
            Command::SnapToAnchors => {
                if self.has_anchors() {
                    self.canvas.snap_to_anchors ^= true;
                }
            }
            Command::SnapToEdges => self.canvas.snap_to_edges ^= true,
            Command::KeyboardShortcuts => self.open_shortcuts(),
            Command::About => self.about = true,
            Command::SelectTool => self.canvas.brushes.tool = None,
            Command::ClearPaint => self.dirty |= view::clear_paint(&mut self.doc, self.selected),
            // Only while a closed shape is selected, to paint in.
            Command::PencilTool | Command::BrushTool | Command::EraserTool => {
                if view::can_paint(&self.doc, self.selected) {
                    self.canvas.brushes.tool = view::TOOLS
                        .iter()
                        .find(|(c, _)| *c == command)
                        .and_then(|(_, t)| *t);
                }
            }
            // Handled as clipboard events and arrow keys.
            Command::Cut | Command::Copy | Command::Paste | Command::Nudge => {}
        }
    }

    fn open_shortcuts(&mut self) {
        self.shortcuts_editor = Some(crate::shortcuts::Editor::new(&self.shortcuts));
    }

    /// The Keyboard shortcuts window, while open. OK uses the shortcuts it shows.
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.shortcuts_editor else {
            return;
        };
        match editor.show(ctx) {
            crate::shortcuts::Outcome::Open => {}
            crate::shortcuts::Outcome::Apply(s) => {
                self.shortcuts = s;
                self.shortcuts_editor = None;
            }
            crate::shortcuts::Outcome::Cancel => self.shortcuts_editor = None,
        }
    }

    /// Keeps the window open when it is asked to close with unsaved changes, and asks
    /// whether to save them first.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_leave = Some(Leave::Close);
        }
    }

    /// The question asked before closing or replacing the project with unsaved changes:
    /// save, don't save or cancel.
    fn close_modal(&mut self, ctx: &egui::Context) {
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
                crate::callout::callout(
                    ui,
                    crate::callout::Tone::Warning,
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

    /// Window title: project name, with * when there are unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = format!(
            "{}{} - Artwork Creator",
            self.project_name(),
            if self.dirty { "*" } else { "" }
        );
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

impl crate::panels::Contents for ArtworkApp {
    fn contents(&mut self, ui: &mut egui::Ui, id: PanelId) -> Option<f32> {
        self.panel_body(ui, id)
    }

    /// As before panels could move, the Template panel shows once there is a template.
    fn skip_docked(&self, id: PanelId) -> bool {
        id == PanelId::Template && self.template.is_none()
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
        self.panels.save(storage);
        self.shortcuts.save(storage);
        self.save_settings(storage);
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
            self.add_images(&ctx, dropped);
        }
        self.guard_close(&ctx);
        crate::colors::publish(&ctx, &self.doc);
        variables::publish(&ctx, &self.doc);
        self.shortcuts(&ctx);
        self.layers_panel.keys = self.shortcuts.texts(&ctx);
        self.canvas.keys = self.layers_panel.keys.clone();
        self.poll_export(&ctx);
        self.watch_template(&ctx);
        // Undo, redo or a reload may have changed effects or images.
        self.sync_effects(&ctx);

        let elements = self.elements();

        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &elements));
        // Taken out while drawn, as it draws the panels with the app's contents.
        let mut panels = std::mem::take(&mut self.panels);
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
        self.canvas.distort_mode = self.layers_panel.distort;
        self.layers_panel.min_dpi = self.export_settings.min_image_dpi as f32;
        // The panel is cut along the outline and through every hole, whatever the export
        // prints there.
        self.canvas.cuts = render::Cuts {
            clip_to_outline: true,
            blank_buttons: true,
            blank_cutouts: true,
            ..self.cuts()
        };
        // Edit points is turned on and off in Properties as well as on the canvas.
        let panel_points = self.layers_panel.edit_points;
        if panel_points != self.canvas.points.as_ref().map(|e| e.layer) {
            self.canvas.points = panel_points.map(view::PointEdit::new);
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.dirty |= view::show(
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
        for var in crate::colors::take_saved(&ctx) {
            self.dirty |= crate::colors::save(&mut self.doc, var);
        }
        self.overwrite_modal(&ctx);
        self.message_modal(&ctx);
        self.about_modal(&ctx);
        self.close_modal(&ctx);
        self.shortcuts_window(&ctx);

        // Record a step once the edit is over: no button held, no text being typed. A
        // shape scaled by a drag that just ended takes its new size first, so the drag
        // and that are one step.
        let finished = !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input();
        // Linked fields follow their variables, and linked colors their color variables,
        // before shapes and texts are laid out.
        variables::apply(&mut self.doc, &mut self.applied);
        crate::colors::apply(&mut self.doc);
        crate::text_links::apply(&mut self.doc);
        shape::sync(&ctx, &mut self.doc, !ctx.input(|i| i.pointer.any_down()));
        // Texts following a shape edited on the canvas go with it in the same step.
        text::sync(&ctx, &mut self.doc, &self.fonts);
        self.history.update(&self.doc, finished);
        self.dirty = self.is_modified();
        self.update_title(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_buttons_sit_right_without_widening_the_window() {
        let ctx = egui::Context::default();
        let mut found = Vec::new();
        for _ in 0..4 {
            let mut window = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let shown = egui::Window::new("w")
                    .resizable(false)
                    .show(ui.ctx(), |ui| {
                        padded(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(250.0);
                                ui.label("A row of settings, longer than the minimum width");
                                ui.add_space(10.0);
                                dialog_buttons(ui, "Export…", true);
                            })
                        })
                    });
                window = shown.unwrap().response.rect;
            });
            out.textures_delta.clear();
            // Where the button texts were drawn: Cancel then Export, at the right edge.
            let mut texts: Vec<(String, f32)> = Vec::new();
            for clipped in &out.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    texts.push((t.galley.text().to_owned(), t.pos.x + t.galley.size().x));
                }
            }
            found.push((window.width(), texts));
        }
        let (width, texts) = found.last().unwrap();
        assert_eq!(found[1].0, *width, "the window keeps its width");
        let x = |name: &str| texts.iter().find(|(t, _)| t == name).unwrap().1;
        assert!(x("Cancel") < x("Export…"));
        let content_right = texts
            .iter()
            .find(|(t, _)| t.starts_with("A row"))
            .unwrap()
            .1;
        assert!((x("Export…") - content_right).abs() < 12.0, "{texts:?}");
    }

    #[test]
    fn a_dialog_with_three_buttons_keeps_the_main_one_right_and_one_apart_left() {
        let ctx = egui::Context::default();
        let mut found = Vec::new();
        for _ in 0..4 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                dialog(ui.ctx(), "t").show(ui.ctx(), |ui| {
                    ui.set_max_width(420.0);
                    ui.label("Save the changes before closing?");
                    dialog_button_row(ui, |ui| {
                        let _ = ui.button("Save and close");
                        let _ = ui.button("Cancel");
                        at_left_edge(ui, |ui| ui.button("Close without saving"));
                    });
                });
            });
            out.textures_delta.clear();
            let mut texts: Vec<(String, f32, f32)> = Vec::new();
            for clipped in &out.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    texts.push((
                        t.galley.text().to_owned(),
                        t.pos.x,
                        t.pos.x + t.galley.size().x,
                    ));
                }
            }
            found.push(texts);
        }
        let texts = found.last().unwrap();
        let at = |name: &str| {
            texts
                .iter()
                .find(|(t, ..)| t == name)
                .map(|(_, a, b)| ((), *a, *b))
                .unwrap()
        };
        let (_, label_left, label_right) = at("Save the changes before closing?");
        assert!(
            at("Save and close").1 > at("Cancel").1
                && at("Cancel").1 > at("Close without saving").1
        );
        assert!(
            (at("Save and close").2 - label_right).abs() < 12.0,
            "Save at the right: {texts:?}"
        );
        assert!(
            (at("Close without saving").1 - label_left).abs() < 12.0,
            "Close without saving at the left"
        );
    }

    #[test]
    fn project_save_path_adds_archive_extension() {
        let p = |s: &str| project_save_path(PathBuf::from(s));
        assert_eq!(p("/a/stick.json"), PathBuf::from("/a/stick.json"));
        assert_eq!(p("/a/stick.JSON"), PathBuf::from("/a/stick.JSON"));
        assert_eq!(
            p("/a/stick"),
            PathBuf::from(format!("/a/stick.{}", project::EXTENSION))
        );
        // A dot in the name is not an extension of ours.
        assert_eq!(
            p("/a/v1.2"),
            PathBuf::from(format!("/a/v1.2.{}", project::EXTENSION))
        );
    }
}
