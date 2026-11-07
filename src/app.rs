use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use egui::{
    Color32, ColorImage, Key, KeyboardShortcut, Modifiers, RichText, TextureFilter, TextureOptions,
};

use crate::canvas_points;
use crate::document::{Document, Layer, Node, Orientation, P2, PageSize};
use crate::effects::cache::EffectCache;
use crate::export::{self, ExportSettings};
use crate::file_dialog::{self, FileBrowser, Filter, Purpose, Request};
use crate::history::History;
use crate::i18n::{self, tr, tr_n, tr_noop, translate};
use crate::layers_ui::{self, LayersPanel};
use crate::project;
use crate::recent::{self, RecentFiles};
use crate::render::{self, PlacedElement, Scene};
use crate::shape::{self, ShapeKind};
use crate::template::{self, ElementKind, Template, TemplateEntry};
use crate::text::{self, FontLibrary};
use crate::view::{self, CanvasState, REAL_SIZE_ZOOM};

/// Longest side of the on-screen texture. Exports always use the full-resolution image.
const MAX_TEXTURE_SIDE: u32 = 4096;
/// Longest side of the copy used to preview semi-transparent groups on screen.
const MAX_PREVIEW_SIDE: u32 = 2048;
/// Width of the preview in the Export window, in points.
const EXPORT_PREVIEW_WIDTH: f32 = 360.0;
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

const SHORTCUT_NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const SHORTCUT_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SHORTCUT_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SHORTCUT_EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const SHORTCUT_DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
const SHORTCUT_TEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::T);
const SHORTCUT_IMAGE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);
/// Adds a shape of the kind added last.
const SHORTCUT_SHAPE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
const SHORTCUT_GROUP: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::G);
const SHORTCUT_UNGROUP: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
const SHORTCUT_UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
const SHORTCUT_REDO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
const SHORTCUT_REDO_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);

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
    /// Recently opened or saved projects, for File > Open Recent.
    recent: RecentFiles,
    export_settings: ExportSettings,
    export_open: bool,
    export_job: Option<ExportJob>,
    /// Progress of the running export, shared with its thread.
    export_progress: export::Progress,
    export_started: Instant,
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
    /// The window was asked to close with unsaved changes: ask what to do.
    confirm_close: bool,
    /// Close once the project is saved (Save in the close question, with a file still to
    /// choose).
    close_after_save: bool,
    /// Closing is agreed: let the next close request through.
    closing: bool,
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
    // Labels can't be selected, so the text cursor only shows over what can be edited.
    ctx.all_styles_mut(|s| s.interaction.selectable_labels = false);
}

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
            canvas: CanvasState::default(),
            layers_panel: LayersPanel::default(),
            selected: None,
            project_path: None,
            dirty: false,
            history: History::new(&Document::default()),
            recent: RecentFiles::load(cc.storage),
            export_settings: ExportSettings::default(),
            export_open: false,
            export_job: None,
            export_progress: export::Progress::default(),
            export_started: Instant::now(),
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
            confirm_close: false,
            close_after_save: false,
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
        self.dirty = self.history.is_modified();
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

    /// Adds a text layer above the selection, centered on the template, and puts the
    /// keyboard in its text.
    fn add_text(&mut self, ctx: &egui::Context) {
        let id = self.doc.alloc_id();
        let center = Self::outline_bounds(&self.elements())
            .map(|(c, _)| c)
            .unwrap_or_else(|| self.doc.page_center());
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

    /// Adds a shape layer of `kind` above the selection, centered on the template. Lines,
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
        let center = Self::outline_bounds(&self.elements())
            .map(|(c, _)| c)
            .unwrap_or_else(|| self.doc.page_center());
        let layer = shape::new_layer(id, kind, center);
        let at = layers_ui::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
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
            Purpose::Export => {
                let s = &self.export_settings;
                [(s.png, "png"), (s.pdf, "pdf")]
                    .into_iter()
                    .filter(|&(on, _)| on)
                    .map(|(_, ext)| path.with_extension(ext))
                    .collect()
            }
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
        let response = egui::Modal::new(egui::Id::unique("overwrite")).show(ctx, |ui| {
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
            ui.weak(tr_n!("FILES_ALREADY_EXIST", names.len(), folder = folder));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
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

    // ---- Project files ----

    /// Starts an empty project, keeping the current template.
    fn new_project(&mut self) {
        let template = self.doc.template.clone();
        self.doc = Document::default();
        self.selected = None;
        self.project_path = None;
        self.select_template(template, true);
        self.canvas.fit_pending = true;
        self.dirty = false;
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
            Ok(doc) => {
                self.doc = doc;
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
            project::save(&self.doc, &path)
        } else {
            project::save_json(&self.doc, &path)
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
                self.dirty = false;
                if std::mem::take(&mut self.close_after_save) {
                    self.closing = true;
                }
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

    /// Renders and writes the export on a background thread.
    fn start_export(&mut self, base: PathBuf) {
        let colors = self.canvas.element_color.map(|c| [c.r(), c.g(), c.b()]);
        // Text and shapes are drawn again at the export resolution.
        let mut doc = self.doc.clone();
        let dpi = self.export_settings.dpi.clamp(72, 1200) as f32;
        text::prepare_export(&mut doc, dpi);
        shape::prepare_export(&mut doc, dpi);
        let scene = Scene::new(
            &doc,
            self.template.as_ref(),
            colors,
            self.export_settings.print_lines,
            self.export_settings.cuts,
        );
        let settings = self.export_settings.clone();
        self.export_open = false;
        self.export_progress = export::Progress::default();
        self.export_started = Instant::now();
        let progress = self.export_progress.clone();
        self.export_job = Some(std::thread::spawn(move || {
            export::run(&scene, &settings, &base, &progress)
        }));
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
            Some(Ok(Err(e))) if e == render::CANCELED => {
                self.messages.push(tr!("EXPORT_CANCELED").into())
            }
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
            self.open_project_path(&ctx, path);
        }
    }

    /// File, Edit and View menus.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr!("MENU_FILE"), |ui| {
                let ctx = ui.ctx().clone();
                let item = |ui: &mut egui::Ui, label: &str, sc: Option<&KeyboardShortcut>| {
                    let mut b = egui::Button::new(label);
                    if let Some(sc) = sc {
                        b = b.shortcut_text(ctx.format_shortcut(sc));
                    }
                    ui.add(b).clicked()
                };
                if item(ui, tr!("NEW_PROJECT"), Some(&SHORTCUT_NEW)) {
                    self.new_project();
                }
                if item(ui, tr!("OPEN_PROJECT"), Some(&SHORTCUT_OPEN)) {
                    self.open_project(&ctx);
                }
                ui.add_enabled_ui(!self.recent.paths().is_empty(), |ui| {
                    ui.menu_button(tr!("MENU_OPEN_RECENT"), |ui| self.recent_menu(ui));
                });
                if item(ui, tr!("SAVE"), Some(&SHORTCUT_SAVE)) {
                    self.save_project(&ctx, false);
                }
                if item(ui, tr!("SAVE_PROJECT_AS"), None) {
                    self.save_project(&ctx, true);
                }
                ui.separator();
                if item(ui, tr!("EXPORT"), Some(&SHORTCUT_EXPORT)) {
                    self.export_open = true;
                }
                ui.separator();
                if item(ui, tr!("QUIT"), None) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(tr!("MENU_EDIT"), |ui| {
                let ctx = ui.ctx().clone();
                let undo = egui::Button::new(tr!("UNDO"))
                    .shortcut_text(ctx.format_shortcut(&SHORTCUT_UNDO));
                if ui.add_enabled(self.history.can_undo(), undo).clicked() {
                    self.undo();
                }
                let redo = egui::Button::new(tr!("REDO"))
                    .shortcut_text(ctx.format_shortcut(&SHORTCUT_REDO));
                if ui.add_enabled(self.history.can_redo(), redo).clicked() {
                    self.redo();
                }
            });
            ui.menu_button(tr!("MENU_VIEW"), |ui| {
                if ui.button(tr!("FIT_PAGE_IN_VIEW")).clicked() {
                    self.canvas.fit_pending = true;
                }
                if ui.button(tr!("ZOOM_REAL_SIZE")).clicked() {
                    self.canvas.set_zoom(REAL_SIZE_ZOOM);
                }
                ui.checkbox(&mut self.canvas.show_template, tr!("SHOW_TEMPLATE_LINES"));
                ui.checkbox(&mut self.canvas.preview_cuts, tr!("PREVIEW_CUTS"));
                let has_anchors = self
                    .template
                    .as_ref()
                    .is_some_and(|t| t.has(ElementKind::Anchors));
                ui.add_enabled(
                    has_anchors,
                    egui::Checkbox::new(&mut self.canvas.snap_to_anchors, tr!("SNAP_TO_ANCHORS")),
                )
                .on_hover_text(tr!("SNAP_TO_ANCHORS_HINT"))
                .on_disabled_hover_text(tr!("TEMPLATE_HAS_NO_ANCHORS"));
                ui.separator();
                ui.menu_button(tr!("MENU_THEME"), crate::theme::menu);
                ui.menu_button(tr!("MENU_LANGUAGE"), |ui| {
                    i18n::menu(ui, &mut self.language)
                });
            });
            ui.menu_button(tr!("MENU_TOOLS"), |ui| {
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

    /// Left panel: template, page, background, template placement and elements.
    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr!("CONTROLLER"));
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

        ui.separator();
        ui.heading(tr!("PAGE_SECTION"));
        let before = (self.doc.page, self.doc.orientation);
        egui::ComboBox::from_id_salt("page")
            .width(ui.available_width())
            .selected_text(self.doc.page.label())
            .show_ui(ui, |ui| {
                for p in PageSize::ALL {
                    ui.selectable_value(&mut self.doc.page, p, p.label());
                }
            });
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
        ui.horizontal(|ui| {
            ui.label(tr!("PAGE_BACKGROUND"));
            let [r, g, b] = self.doc.background;
            let mut c = Color32::from_rgb(r, g, b);
            if ui.color_edit_button_srgba(&mut c).changed() {
                self.doc.background = [c.r(), c.g(), c.b()];
                self.dirty = true;
            }
        });

        if self.template.is_some() {
            crate::layers_ui::section(
                ui,
                "template",
                RichText::new(tr!("TEMPLATE_SECTION")).heading(),
                |_| false,
                |ui| self.template_settings(ui),
            );
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
        ui.horizontal(|ui| {
            ui.label(tr!("TEMPLATE_ROTATION"));
            for deg in [0, 90, 180, 270] {
                if ui
                    .selectable_label(p.rotation == deg, format!("{deg}°"))
                    .clicked()
                {
                    p.rotation = deg;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(tr!("TEMPLATE_FLIP"));
            ui.checkbox(&mut p.flip_x, tr!("TEMPLATE_FLIP_HORIZONTAL"));
            ui.checkbox(&mut p.flip_y, tr!("TEMPLATE_FLIP_VERTICAL"));
        });
        ui.horizontal(|ui| {
            ui.label(tr!("TEMPLATE_OFFSET"));
            ui.add(
                egui::DragValue::new(&mut p.offset_mm[0])
                    .speed(0.2)
                    .fixed_decimals(1)
                    .prefix("x ")
                    .suffix(" mm"),
            );
            ui.add(
                egui::DragValue::new(&mut p.offset_mm[1])
                    .speed(0.2)
                    .fixed_decimals(1)
                    .prefix("y ")
                    .suffix(" mm"),
            );
        });
        if ui.small_button(tr!("CENTER_TEMPLATE_ON_PAGE")).clicked() {
            p.offset_mm = [0.0, 0.0];
        }

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
        ui.checkbox(&mut self.canvas.preview_cuts, tr!("PREVIEW_CUTS"))
            .on_hover_text(tr!("PREVIEW_CUTS_HINT"));
        ui.add_enabled_ui(self.canvas.preview_cuts, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr!("CUT_PREVIEW_OPACITY"));
                ui.add(
                    egui::Slider::new(&mut self.canvas.cut_opacity, 0.0..=1.0).fixed_decimals(2),
                )
                .on_hover_text(tr!("CUT_PREVIEW_OPACITY_HINT"));
            });
        });

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
                .on_hover_text(tr!("FIT_PAGE_IN_VIEW_HINT"))
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
        (colors, s.print_lines, s.cuts, &self.template_stamp).hash(&mut h);
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
            s.cuts,
        );
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
        egui::Modal::new(egui::Id::unique("export-progress")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading(tr!("EXPORT_PROGRESS_TITLE"));
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

    /// Export options; the Export button opens the save dialog.
    fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export_open {
            // Free the preview texture while the window is closed.
            self.export_preview = None;
            return;
        }
        self.update_export_preview(ctx);
        let preview = self
            .export_preview
            .as_ref()
            .map(|(_, t)| (t.id(), t.size_vec2()));
        let mut open = self.export_open;
        let mut go = false;
        egui::Window::new(tr!("EXPORT_WINDOW_TITLE"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    if let Some((tex, size)) = preview {
                        let size = size * (EXPORT_PREVIEW_WIDTH / size.x);
                        ui.vertical(|ui| {
                            egui::Frame::new()
                                .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                .show(ui, |ui| ui.image((tex, size)));
                            ui.weak(tr!("EXPORT_PREVIEW"));
                        });
                        ui.add_space(8.0);
                    }
                    ui.vertical(|ui| {
                        // At least wide enough that the checkbox labels don't wrap; rows
                        // that need more widen the column.
                        ui.set_min_width(250.0);
                        let s = &mut self.export_settings;
                        ui.horizontal(|ui| {
                            ui.label(tr!("EXPORT_FORMATS"));
                            ui.checkbox(&mut s.png, "PNG");
                            ui.checkbox(&mut s.pdf, "PDF");
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr!("EXPORT_RESOLUTION"));
                            ui.add(
                                egui::DragValue::new(&mut s.dpi)
                                    .range(72..=1200)
                                    .suffix(" DPI"),
                            );
                        });
                        let template = self.template.as_ref();
                        let has = |k: ElementKind| template.is_some_and(|t| t.has(k));
                        if template.is_some() {
                            ui.add_space(4.0);
                            ui.strong(tr!("EXPORT_TEMPLATE_SECTION"));
                            ui.horizontal(|ui| {
                                ui.label(tr!("EXPORT_PRINT_LINES"));
                                for k in ElementKind::ALL.into_iter().filter(|&k| has(k)) {
                                    ui.checkbox(
                                        &mut s.print_lines[k.index()],
                                        translate(k.label()),
                                    );
                                }
                            });
                            ui.add_enabled(
                                has(ElementKind::Outline),
                                egui::Checkbox::new(
                                    &mut s.cuts.clip_to_outline,
                                    tr!("EXPORT_BLANK_OUTSIDE_OUTLINE"),
                                ),
                            );
                            if has(ElementKind::Buttons) || has(ElementKind::Cutouts) {
                                ui.horizontal(|ui| {
                                    ui.label(tr!("EXPORT_BLANK_INSIDE"));
                                    if has(ElementKind::Buttons) {
                                        ui.checkbox(
                                            &mut s.cuts.blank_buttons,
                                            tr!("EXPORT_BLANK_INSIDE_BUTTONS"),
                                        );
                                    }
                                    if has(ElementKind::Cutouts) {
                                        ui.checkbox(
                                            &mut s.cuts.blank_cutouts,
                                            tr!("EXPORT_BLANK_INSIDE_CUTOUTS"),
                                        );
                                    }
                                });
                            }
                            ui.add_space(4.0);
                        }
                        let [w, h] = render::pixel_size(self.doc.page_mm(), s.dpi as f32);
                        ui.weak(tr!(
                            "EXPORT_PIXEL_SIZE",
                            page = self.doc.page.label(),
                            w = w,
                            h = h
                        ));
                        ui.add_space(6.0);
                        let can = (s.png || s.pdf) && self.export_job.is_none();
                        if ui
                            .add_enabled(can, egui::Button::new(tr!("START_EXPORT")))
                            .clicked()
                        {
                            go = true;
                        }
                    });
                });
            });
        self.export_open = open;
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

    /// Shows queued messages (errors, a canceled export) one at a time.
    fn message_modal(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.messages.first().cloned() else {
            return;
        };
        let response = egui::Modal::new(egui::Id::unique("message")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(msg);
            ui.add_space(8.0);
            ui.button("OK").clicked()
        });
        if response.inner || response.should_close() {
            self.messages.remove(0);
        }
    }

    /// Keyboard shortcuts. Editing ones are skipped while a text field has focus.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = |sc: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(sc));
        if pressed(&SHORTCUT_NEW) {
            self.new_project();
        }
        if pressed(&SHORTCUT_OPEN) {
            self.open_project(ctx);
        }
        if pressed(&SHORTCUT_SAVE) {
            self.save_project(ctx, false);
        }
        if pressed(&SHORTCUT_EXPORT) {
            self.export_open = true;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // Ctrl+Z also matches Ctrl+Shift+Z, so test the longer one first.
        if pressed(&SHORTCUT_REDO) || pressed(&SHORTCUT_REDO_ALT) {
            self.redo();
        } else if pressed(&SHORTCUT_UNDO) {
            self.undo();
        }
        if pressed(&SHORTCUT_TEXT) {
            self.add_text(ctx);
        }
        if pressed(&SHORTCUT_IMAGE) {
            self.pick_images(ctx);
        }
        if pressed(&SHORTCUT_SHAPE) {
            self.add_shape(ctx, self.layers_panel.shape_kind);
        }
        if pressed(&SHORTCUT_DUPLICATE) {
            self.dirty |= layers_ui::duplicate(&mut self.doc, &mut self.selected);
        }
        // Ctrl+G also matches Ctrl+Shift+G, so test the longer one first.
        if pressed(&SHORTCUT_UNGROUP) {
            self.dirty |= layers_ui::ungroup(&mut self.doc, &mut self.selected);
        } else if pressed(&SHORTCUT_GROUP) {
            self.dirty |= layers_ui::group(&mut self.doc, &mut self.selected);
        }
        let (del, fit, arrows) = ctx.input(|i| {
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
            (
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                i.key_pressed(Key::F) && i.modifiers.is_none(),
                d,
            )
        });
        // While a shape's points are edited, the keys act on the point picked last.
        let point = self
            .canvas
            .points
            .as_ref()
            .is_some_and(|e| e.selected.is_some());
        if del {
            self.dirty |= if point {
                canvas_points::delete_selected(&mut self.canvas.points, &mut self.doc)
            } else {
                layers_ui::delete(&mut self.doc, &mut self.selected)
            };
        }
        if fit {
            self.canvas.fit_pending = true;
        }
        if arrows != [0.0, 0.0] {
            self.dirty |= if point {
                canvas_points::nudge_selected(&self.canvas.points, &mut self.doc, arrows)
            } else {
                layers_ui::nudge(&mut self.doc, self.selected, arrows)
            };
        }
    }

    /// Keeps the window open when it is asked to close with unsaved changes, and asks
    /// whether to save them first.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
    }

    /// The question asked before closing with unsaved changes: save, don't save or cancel.
    fn close_modal(&mut self, ctx: &egui::Context) {
        if self.confirm_close {
            let (mut save, mut discard, mut cancel) = (false, false, false);
            let response = egui::Modal::new(egui::Id::unique("confirm_close")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.strong(tr!(
                    "CONFIRM_SAVE_BEFORE_CLOSING",
                    name = self.project_name()
                ));
                ui.add_space(4.0);
                ui.weak(tr!("UNSAVED_CHANGES_WARNING"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    save |= ui.button(tr!("SAVE")).clicked();
                    discard |= ui.button(tr!("DONT_SAVE")).clicked();
                    cancel |= ui.button(tr!("CANCEL")).clicked();
                });
            });
            cancel |= response.should_close();
            if save || discard || cancel {
                self.confirm_close = false;
            }
            if discard {
                self.closing = true;
            } else if save {
                self.close_after_save = true;
                self.save_project(ctx, false);
            }
        }
        // The save was given up: no file chosen, or the overwrite question canceled.
        if self.close_after_save && !self.browser.is_open() && self.overwrite.is_none() {
            self.close_after_save = false;
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

impl eframe::App for ArtworkApp {
    /// Keeps app settings between sessions (eframe also stores the theme and window).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.recent.save(storage);
        self.language.save(storage);
    }

    /// One frame: input, panels, canvas and dialogs, then an undo step if the project
    /// changed.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

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
        self.shortcuts(&ctx);
        self.poll_export(&ctx);
        self.watch_template(&ctx);
        // Undo, redo or a reload may have changed effects or images.
        self.sync_effects(&ctx);

        let elements = self.elements();

        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &elements));
        egui::Panel::left("settings")
            .resizable(true)
            .default_size(240.0)
            .min_size(240.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
            });
        // The layer panel's rows (toolbar, align, properties) need 310 points.
        egui::Panel::right("layers")
            .resizable(true)
            .default_size(320.0)
            .min_size(310.0)
            .show(ui, |ui| {
                let bounds = Self::outline_bounds(&elements);
                let out = layers_ui::show(
                    ui,
                    &mut self.doc,
                    &mut self.selected,
                    &mut self.layers_panel,
                    bounds,
                    &self.fonts,
                );
                self.dirty |= out.changed;
                if out.add_images {
                    self.pick_images(&ctx);
                }
                if out.add_text {
                    self.add_text(&ctx);
                }
                if let Some(kind) = out.add_shape {
                    self.add_shape(&ctx, kind);
                }
            });

        // Show effect changes made in the layer panel on this frame's canvas.
        self.sync_effects(&ctx);

        // Settings may have moved the template this frame.
        let elements = self.elements();
        self.canvas.distort_mode = self.layers_panel.distort;
        self.canvas.cuts = self.export_settings.cuts;
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

        self.export_window(&ctx);
        self.export_progress_window(&ctx);
        if let Some((purpose, paths)) = self.browser.show(&ctx) {
            self.on_files(&ctx, purpose, paths, false);
        }
        self.overwrite_modal(&ctx);
        self.message_modal(&ctx);
        self.close_modal(&ctx);

        // Record a step once the edit is over: no button held, no text being typed. A
        // shape scaled by a drag that just ended takes its new size first, so the drag
        // and that are one step.
        let finished = !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input();
        shape::sync(&ctx, &mut self.doc, !ctx.input(|i| i.pointer.any_down()));
        // Texts following a shape edited on the canvas go with it in the same step.
        text::sync(&ctx, &mut self.doc, &self.fonts);
        self.history.update(&self.doc, finished);
        self.dirty = self.history.is_modified();
        self.update_title(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
