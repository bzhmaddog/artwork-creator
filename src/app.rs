use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, ColorImage, Key, KeyboardShortcut, Modifiers, TextureFilter, TextureOptions};

use crate::document::{Document, Layer, Node, Orientation, P2, PageSize};
use crate::export::{self, ExportSettings};
use crate::file_dialog::{self, FileBrowser, Purpose, Request};
use crate::history::History;
use crate::layers_ui::{self, LayersPanel};
use crate::render::{self, PlacedElement, Scene};
use crate::template::{self, ElementKind, Template, TemplateEntry};
use crate::view::{self, CanvasState, REAL_SIZE_ZOOM};

/// Longest side of the on-screen texture. Exports always use the full-resolution image.
const MAX_TEXTURE_SIDE: u32 = 4096;
/// Longest side of the copy used to preview semi-transparent groups on screen.
const MAX_PREVIEW_SIDE: u32 = 2048;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const PROJECT_EXTENSIONS: &[&str] = &["json"];

const SHORTCUT_NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
const SHORTCUT_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const SHORTCUT_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SHORTCUT_EXPORT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::E);
const SHORTCUT_DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
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
    export_settings: ExportSettings,
    export_open: bool,
    export_job: Option<ExportJob>,
    messages: Vec<String>,
    title: String,
    /// Native dialogs when the system has them, otherwise the built-in browser.
    native_dialogs: bool,
    browser: FileBrowser,
    /// Modification times of the current template's files, to reload on change.
    template_stamp: Vec<Option<SystemTime>>,
    last_template_check: Instant,
}

impl ArtworkApp {
    /// `project` is an optional project file to open, e.g. from the command line.
    pub fn new(cc: &eframe::CreationContext<'_>, project: Option<PathBuf>) -> Self {
        // Text fields stop editing as soon as the mouse is pressed anywhere else,
        // not only on a clean click (egui's default ignores presses that turn into drags).
        cc.egui_ctx.options_mut(|o| {
            o.input_options.surrender_focus_on = egui::SurrenderFocusOn::Presses;
        });
        let templates_dir = template::templates_dir();
        let templates = templates_dir
            .as_deref()
            .map(template::scan)
            .unwrap_or_default();
        let mut messages = Vec::new();
        if templates.is_empty() {
            messages.push(
                "No controller templates found. Put a folder containing a .dxf file in a \"templates\" \
                 folder next to the program."
                    .to_owned(),
            );
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
            export_settings: ExportSettings::default(),
            export_open: false,
            export_job: None,
            messages,
            title: String::new(),
            native_dialogs: file_dialog::native_available(),
            browser: FileBrowser::default(),
            template_stamp: Vec::new(),
            last_template_check: Instant::now(),
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
        self.layers_panel.renaming = None;
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
            self.messages
                .push(format!("Template \"{id}\" is not installed."));
            return;
        };
        if let Some(e) = &entry.config_error {
            self.messages.push(format!(
                "Invalid template configuration, using defaults.\n{e}"
            ));
        }
        match template::load(entry) {
            Ok(t) => {
                if apply_defaults {
                    self.doc.template_placement =
                        entry.default_placement(t.size_mm, self.doc.page_mm());
                }
                self.template_stamp = entry.stamp();
                // Line colours set in template.json.
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

    /// Axis-aligned (centre, size) of the placed outline on the page, or of the whole
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

    /// Shrinks an image so its longest side is at most `max_side`; borrows it if it's
    /// already small enough.
    fn downscale(img: &image::RgbaImage, max_side: u32) -> std::borrow::Cow<'_, image::RgbaImage> {
        let (w, h) = img.dimensions();
        if w.max(h) <= max_side {
            return std::borrow::Cow::Borrowed(img);
        }
        let k = max_side as f32 / w.max(h) as f32;
        let (dw, dh) = (
            ((w as f32 * k).round() as u32).max(1),
            ((h as f32 * k).round() as u32).max(1),
        );
        std::borrow::Cow::Owned(image::imageops::resize(
            img,
            dw,
            dh,
            image::imageops::FilterType::Triangle,
        ))
    }

    /// Decodes a layer's image file into its full-size pixmap, preview pixmap and screen
    /// texture.
    fn load_image(ctx: &egui::Context, layer: &mut Layer) -> Result<(), String> {
        let img = image::open(&layer.source)
            .map_err(|e| format!("Cannot open {}: {e}", layer.source.display()))?
            .to_rgba8();
        let invalid = || format!("{} has no pixels", layer.source.display());
        let (w, h) = img.dimensions();
        layer.size_px = [w, h];
        let display = Self::downscale(&img, MAX_TEXTURE_SIDE);
        let color = ColorImage::from_rgba_unmultiplied(
            [display.width() as usize, display.height() as usize],
            display.as_raw(),
        );
        let preview = render::premultiplied_pixmap(&Self::downscale(&display, MAX_PREVIEW_SIDE))
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

    /// Adds one layer per image file, fitted to the page, above the selection.
    fn add_images(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        for path in paths {
            let id = self.doc.alloc_id();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Image".into());
            let mut layer = Layer::new(id, name, path);
            match Self::load_image(ctx, &mut layer) {
                Ok(()) => {
                    layer.fit_to(self.doc.page_center(), self.doc.page_mm(), false);
                    let at = layers_ui::insertion_path(&mut self.doc, self.selected);
                    self.doc.insert(&at, Node::Layer(layer));
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
                self.on_files(ctx, req.purpose, paths);
            }
        } else {
            self.browser.open(req);
        }
    }

    /// Acts on the paths chosen in a file dialog.
    fn on_files(&mut self, ctx: &egui::Context, purpose: Purpose, mut paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        match purpose {
            Purpose::AddImages => self.add_images(ctx, paths),
            Purpose::OpenProject => self.open_project_path(ctx, paths.swap_remove(0)),
            Purpose::SaveProject => {
                let mut path = paths.swap_remove(0);
                if path.extension().is_none() {
                    path.set_extension("artwork.json");
                }
                self.save_to(path);
            }
            Purpose::Export => self.start_export(paths.swap_remove(0)),
        }
    }

    /// Opens the dialog to add image layers.
    fn pick_images(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::AddImages,
                title: "Add images",
                save: false,
                multiple: true,
                filter_name: "Images",
                extensions: IMAGE_EXTENSIONS,
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
                title: "Open project",
                save: false,
                multiple: false,
                filter_name: "Artwork project",
                extensions: PROJECT_EXTENSIONS,
                default_name: String::new(),
            },
        );
    }

    /// Loads a project and its images. Missing images are reported and drawn crossed out.
    fn open_project_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        match Document::load(&path) {
            Ok(doc) => {
                self.doc = doc;
                let mut missing = Vec::new();
                self.doc.for_each_layer_mut(|layer| {
                    if Self::load_image(ctx, layer).is_err() {
                        layer.broken = true;
                        missing.push(layer.source.display().to_string());
                    }
                });
                if !missing.is_empty() {
                    self.messages.push(format!(
                        "These images could not be loaded:\n{}",
                        missing.join("\n")
                    ));
                }
                let template = self.doc.template.clone();
                self.select_template(template, false);
                self.selected = None;
                self.project_path = Some(path);
                self.canvas.fit_pending = true;
                self.dirty = false;
                self.history.reset(&self.doc);
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// Saves to the current file, or asks for one (always with `save_as`).
    fn save_project(&mut self, ctx: &egui::Context, save_as: bool) {
        match (&self.project_path, save_as) {
            (Some(p), false) => self.save_to(p.clone()),
            _ => self.request_file(
                ctx,
                Request {
                    purpose: Purpose::SaveProject,
                    title: "Save project",
                    save: true,
                    multiple: false,
                    filter_name: "Artwork project",
                    extensions: PROJECT_EXTENSIONS,
                    default_name: format!("{}.artwork.json", self.project_name()),
                },
            ),
        }
    }

    /// Writes the project to `path` and marks it as saved.
    fn save_to(&mut self, path: PathBuf) {
        match self.doc.save(&path) {
            Ok(()) => {
                self.project_path = Some(path);
                self.history.mark_saved(&self.doc);
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
            .unwrap_or_else(|| "Untitled".into())
    }

    // ---- Export ----

    /// Renders and writes the export on a background thread.
    fn start_export(&mut self, base: PathBuf) {
        let colors = self.canvas.element_color.map(|c| [c.r(), c.g(), c.b()]);
        let scene = Scene::new(
            &self.doc,
            self.template.as_ref(),
            colors,
            self.export_settings.print_lines,
            self.export_settings.cuts,
        );
        let settings = self.export_settings.clone();
        self.export_open = false;
        self.export_job = Some(std::thread::spawn(move || {
            export::run(&scene, &settings, &base)
        }));
    }

    /// Reports the export's result once its thread has finished.
    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.export_job else { return };
        if !job.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let result = self.export_job.take().map(|j| j.join());
        match result {
            Some(Ok(Ok(paths))) => self.messages.push(format!(
                "Exported:\n{}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            Some(Ok(Err(e))) => self.messages.push(format!("Export failed: {e}")),
            Some(Err(_)) => self.messages.push("Export failed unexpectedly.".into()),
            None => {}
        }
    }

    // ---- UI ----

    /// File, Edit and View menus.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                let ctx = ui.ctx().clone();
                let item = |ui: &mut egui::Ui, label: &str, sc: Option<&KeyboardShortcut>| {
                    let mut b = egui::Button::new(label);
                    if let Some(sc) = sc {
                        b = b.shortcut_text(ctx.format_shortcut(sc));
                    }
                    ui.add(b).clicked()
                };
                if item(ui, "New", Some(&SHORTCUT_NEW)) {
                    self.new_project();
                }
                if item(ui, "Open…", Some(&SHORTCUT_OPEN)) {
                    self.open_project(&ctx);
                }
                if item(ui, "Save", Some(&SHORTCUT_SAVE)) {
                    self.save_project(&ctx, false);
                }
                if item(ui, "Save As…", None) {
                    self.save_project(&ctx, true);
                }
                ui.separator();
                if item(ui, "Add images…", None) {
                    self.pick_images(&ctx);
                }
                if item(ui, "Export…", Some(&SHORTCUT_EXPORT)) {
                    self.export_open = true;
                }
                ui.separator();
                if item(ui, "Quit", None) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let ctx = ui.ctx().clone();
                let undo =
                    egui::Button::new("Undo").shortcut_text(ctx.format_shortcut(&SHORTCUT_UNDO));
                if ui.add_enabled(self.history.can_undo(), undo).clicked() {
                    self.undo();
                }
                let redo =
                    egui::Button::new("Redo").shortcut_text(ctx.format_shortcut(&SHORTCUT_REDO));
                if ui.add_enabled(self.history.can_redo(), redo).clicked() {
                    self.redo();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Fit page").clicked() {
                    self.canvas.fit_pending = true;
                }
                if ui.button("Real size (100%)").clicked() {
                    self.canvas.set_zoom(REAL_SIZE_ZOOM);
                }
                ui.checkbox(&mut self.canvas.show_template, "Show template lines");
                ui.checkbox(&mut self.canvas.preview_cuts, "Preview cuts");
            });
        });
    }

    /// Left panel: template, page, background, template placement and elements.
    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Controller");
        let current = self
            .current_entry()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "None".into());
        let mut choice = self.doc.template.clone();
        egui::ComboBox::from_id_salt("template")
            .width(ui.available_width())
            .selected_text(current)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, None, "None");
                for t in &self.templates {
                    ui.selectable_value(&mut choice, Some(t.id.clone()), &t.name);
                }
            });
        if choice != self.doc.template {
            self.select_template(choice, true);
        }
        if ui.small_button("Rescan templates").clicked() {
            self.templates = self
                .templates_dir
                .as_deref()
                .map(template::scan)
                .unwrap_or_default();
            let id = self.doc.template.clone();
            self.select_template(id, true);
        }

        ui.separator();
        ui.heading("Page");
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
                "Landscape",
            );
            ui.radio_value(&mut self.doc.orientation, Orientation::Portrait, "Portrait");
        });
        if before != (self.doc.page, self.doc.orientation) {
            if let (Some(t), Some(e)) = (&self.template, self.current_entry()) {
                self.doc.template_placement = e.default_placement(t.size_mm, self.doc.page_mm());
            }
            self.canvas.fit_pending = true;
            self.dirty = true;
        }
        ui.horizontal(|ui| {
            ui.label("Background");
            let [r, g, b] = self.doc.background;
            let mut c = Color32::from_rgb(r, g, b);
            if ui.color_edit_button_srgba(&mut c).changed() {
                self.doc.background = [c.r(), c.g(), c.b()];
                self.dirty = true;
            }
        });

        if let Some(t) = &self.template {
            let size = t.size_mm;
            ui.separator();
            ui.heading("Template");
            ui.weak(format!("{:.1} x {:.1} mm", size[0], size[1]));
            let before = self.doc.template_placement;
            let p = &mut self.doc.template_placement;
            ui.horizontal(|ui| {
                ui.label("Rotation");
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
                ui.label("Flip");
                ui.checkbox(&mut p.flip_x, "Horizontal");
                ui.checkbox(&mut p.flip_y, "Vertical");
            });
            ui.horizontal(|ui| {
                ui.label("Offset");
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
            if ui.small_button("Centre on page").clicked() {
                p.offset_mm = [0.0, 0.0];
            }

            ui.add_space(4.0);
            ui.strong("Elements");
            for e in &t.elements {
                let k = e.kind.index();
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.canvas.show_template, |ui| {
                        ui.checkbox(&mut self.canvas.element_show[k], "")
                            .on_hover_text("Show its lines on the canvas");
                    });
                    ui.color_edit_button_srgba(&mut self.canvas.element_color[k])
                        .on_hover_text("Line colour, on screen and in the export");
                    ui.label(&e.name);
                    ui.weak(format!(
                        "{} shape{}",
                        e.polylines.len(),
                        if e.polylines.len() == 1 { "" } else { "s" }
                    ));
                });
            }
            ui.checkbox(&mut self.canvas.preview_cuts, "Preview cuts")
                .on_hover_text(
                    "Cover what the export leaves blank: outside the outline and inside holes",
                );

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
                        egui::Button::new("Reset to default").small(),
                    )
                    .on_hover_text("Use the placement from template.json")
                    .clicked()
                    && let Some(d) = default
                {
                    self.doc.template_placement = d;
                }
                if ui
                    .add_enabled(
                        default.is_some_and(|d| d != current),
                        egui::Button::new("Save as default").small(),
                    )
                    .on_hover_text("Write this placement into the template's template.json")
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
                ui.weak(format!("Config: {}", e.config_path().display()));
            }
            if self.doc.template_placement != before {
                self.dirty = true;
            }
        }
    }

    /// Bottom bar: zoom, page size, cursor position and warnings.
    fn status_bar(&mut self, ui: &mut egui::Ui, elements: &[PlacedElement]) {
        ui.horizontal(|ui| {
            if ui.small_button("−").on_hover_text("Zoom out").clicked() {
                self.canvas.zoom_centered(1.0 / 1.25);
            }
            ui.label(format!(
                "{:.0}%",
                self.canvas.camera.zoom / REAL_SIZE_ZOOM * 100.0
            ));
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                self.canvas.zoom_centered(1.25);
            }
            if ui
                .small_button("Fit")
                .on_hover_text("Fit page (F)")
                .clicked()
            {
                self.canvas.fit_pending = true;
            }
            if ui.small_button("100%").on_hover_text("Real size").clicked() {
                self.canvas.set_zoom(REAL_SIZE_ZOOM);
            }
            ui.separator();
            let [pw, ph] = self.doc.page_mm();
            ui.label(format!("Page {pw:.0} x {ph:.0} mm"));
            if let Some([x, y]) = self.canvas.cursor_mm {
                ui.separator();
                ui.label(format!("x {x:.1}  y {y:.1} mm"));
            }
            if let Some((c, s)) = Self::outline_bounds(elements) {
                let off = c[0] - s[0] / 2.0 < -0.01
                    || c[1] - s[1] / 2.0 < -0.01
                    || c[0] + s[0] / 2.0 > pw + 0.01
                    || c[1] + s[1] / 2.0 > ph + 0.01;
                if off {
                    ui.separator();
                    ui.colored_label(
                        Color32::from_rgb(230, 160, 40),
                        "Template does not fit on this page",
                    );
                }
            }
            if self.export_job.is_some() {
                ui.separator();
                ui.spinner();
                ui.label("Exporting…");
            }
        });
    }

    /// Export options; the Export button opens the save dialog.
    fn export_window(&mut self, ctx: &egui::Context) {
        let mut open = self.export_open;
        let mut go = false;
        egui::Window::new("Export")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let s = &mut self.export_settings;
                ui.horizontal(|ui| {
                    ui.label("Formats");
                    ui.checkbox(&mut s.png, "PNG");
                    ui.checkbox(&mut s.pdf, "PDF");
                });
                ui.horizontal(|ui| {
                    ui.label("Resolution");
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
                    ui.strong("Template");
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Print lines");
                        for k in ElementKind::ALL.into_iter().filter(|&k| has(k)) {
                            ui.checkbox(&mut s.print_lines[k.index()], k.label());
                        }
                    });
                    ui.add_enabled(
                        has(ElementKind::Outline),
                        egui::Checkbox::new(
                            &mut s.cuts.clip_to_outline,
                            "Leave blank outside the outline",
                        ),
                    );
                    ui.add_enabled(
                        has(ElementKind::Buttons) || has(ElementKind::Cutouts),
                        egui::Checkbox::new(
                            &mut s.cuts.blank_holes,
                            "Leave buttons and cutouts blank",
                        ),
                    );
                    ui.add_space(4.0);
                }
                let [w, h] = render::pixel_size(self.doc.page_mm(), s.dpi as f32);
                ui.weak(format!("{} at {w} x {h} px", self.doc.page.label()));
                ui.add_space(6.0);
                let can = (s.png || s.pdf) && self.export_job.is_none();
                if ui.add_enabled(can, egui::Button::new("Export…")).clicked() {
                    go = true;
                }
            });
        self.export_open = open;
        if go {
            let name = self.project_name();
            self.request_file(
                ctx,
                Request {
                    purpose: Purpose::Export,
                    title: "Export artwork",
                    save: true,
                    multiple: false,
                    filter_name: "PNG / PDF",
                    extensions: &["png", "pdf"],
                    default_name: name,
                },
            );
        }
    }

    /// Shows queued messages (errors, export results) one at a time.
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
        if del {
            self.dirty |= layers_ui::delete(&mut self.doc, &mut self.selected);
        }
        if fit {
            self.canvas.fit_pending = true;
        }
        if arrows != [0.0, 0.0] {
            self.dirty |= layers_ui::nudge(&mut self.doc, self.selected, arrows);
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
        self.shortcuts(&ctx);
        self.poll_export(&ctx);
        self.watch_template(&ctx);

        let elements = self.elements();

        egui::Panel::top("menu_bar").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &elements));
        egui::Panel::left("settings")
            .resizable(true)
            .default_size(230.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
            });
        egui::Panel::right("layers")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                let bounds = Self::outline_bounds(&elements);
                let out = layers_ui::show(
                    ui,
                    &mut self.doc,
                    &mut self.selected,
                    &mut self.layers_panel,
                    bounds,
                );
                self.dirty |= out.changed;
                if out.add_images {
                    self.pick_images(&ctx);
                }
            });

        // Settings may have moved the template this frame.
        let elements = self.elements();
        self.canvas.distort_mode = self.layers_panel.distort;
        self.canvas.cuts = self.export_settings.cuts;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.dirty |= view::show(
                ui,
                &mut self.doc,
                &elements,
                &mut self.selected,
                &mut self.canvas,
            );
        });

        self.export_window(&ctx);
        if let Some((purpose, paths)) = self.browser.show(&ctx) {
            self.on_files(&ctx, purpose, paths);
        }
        self.message_modal(&ctx);

        // Record a step once the edit is over: no button held, no text being typed.
        let finished = !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input();
        self.history.update(&self.doc, finished);
        self.dirty = self.history.is_modified();
        self.update_title(&ctx);
    }
}
