use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, ColorImage, Key, TextureFilter, TextureOptions};

use crate::canvas_points;
use crate::clipboard::Clipboard;
use crate::document::{self, Document, Layer, Node, Orientation, P2, PageSize, TemplateCopy};
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
use crate::template_check::{Problem, Severity};
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
/// Fill of a button that deletes something, under white text, in both themes.
const DANGER_COLOR: Color32 = Color32::from_rgb(200, 40, 40);
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const IMAGE_FILTERS: &[Filter] = &[Filter::new(tr_noop!("FILTER_IMAGES"), IMAGE_EXTENSIONS)];
/// SVG files, whose shapes are imported as path layers (see `svg`).
const SVG_EXTENSIONS: &[&str] = &["svg", "svgz"];
const SVG_FILTERS: &[Filter] = &[Filter::new(tr_noop!("FILTER_SVG"), SVG_EXTENSIONS)];
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
    template_dialog: Option<crate::template_ui::Dialog>,
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
    /// Modification times of the installed template's files, to notice a change.
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
    /// Copies of the project with unsaved changes, for recovery after a crash.
    autosave: crate::recovery::Autosave,
    /// Copies left by runs that crashed, offered back one at a time.
    orphans: Vec<crate::recovery::Orphan>,
    /// When the project file was last written, for the status bar: by this run, or
    /// before it was opened.
    last_saved: Option<SystemTime>,
}

/// Labels of the rows of the Page, Template and Print panels: the first column of
/// their grids is as wide as the widest, so that the fields line up within a panel and
/// from one to the next.
const PANEL_LABELS: [&str; 10] = [
    tr_noop!("PAGE_BACKGROUND"),
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
        // A template with problems has a mark after its name.
        let mark = if templates.iter().any(|t| !t.problems.is_empty()) {
            14.0 + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        names.fold(0.0, f32::max)
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
            // text in, inside the row; a mark after it when the template diagnostic
            // found problems, listed in its tooltip; its version, if any, at its right
            // end.
            let mut row = |ui: &mut egui::Ui,
                           id: Option<String>,
                           text: &str,
                           version: Option<&str>,
                           indent: f32,
                           problems: &[Problem]| {
                let gap = egui::Atom::paint(egui::vec2(indent, 0.0), |_, _| {});
                let version = egui::RichText::new(version.unwrap_or_default()).weak();
                let worst = if crate::template_check::has_errors(problems) {
                    Severity::Error
                } else {
                    Severity::Warning
                };
                let mark = if problems.is_empty() {
                    egui::Atom::paint(egui::Vec2::ZERO, |_, _| {})
                } else {
                    crate::callout::warning_mark(problem_color(ui, worst))
                };
                let button = egui::Button::selectable(
                    choice == id,
                    (gap, text, mark, egui::Atom::grow(), version),
                )
                .min_size(egui::vec2(ui.available_width(), 0.0));
                let mut response = ui.add(button);
                if !problems.is_empty() {
                    response = response.on_hover_ui(|ui| problems_tooltip(ui, problems));
                }
                if response.clicked() {
                    choice = id;
                }
            };
            row(ui, None, tr!("NO_TEMPLATE"), None, 0.0, &[]);
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
                    &t.problems,
                );
            }
        });
    (choice, field)
}

/// The color of the mark of a problem of `severity` found in a template.
fn problem_color(ui: &egui::Ui, severity: Severity) -> Color32 {
    match severity {
        Severity::Error => ui.visuals().error_fg_color,
        Severity::Warning => ui.visuals().warn_fg_color,
    }
}

/// The problems found in a template, errors first, one per line after its mark: for a
/// tooltip.
fn problems_tooltip(ui: &mut egui::Ui, problems: &[Problem]) {
    let mut sorted: Vec<&Problem> = problems.iter().collect();
    sorted.sort_by_key(|p| p.severity);
    ui.label(tr!("TEMPLATE_CHECK_LIST_HINT"));
    for p in sorted {
        ui.horizontal_top(|ui| {
            crate::callout::warning_icon(ui, problem_color(ui, p.severity));
            ui.label(&p.text);
        });
    }
}

/// The Template check panel's contents: the problems found, counted, errors first; one
/// about a shape is a button selecting it. Returns the layer clicked.
fn problem_list(ui: &mut egui::Ui, problems: &[Problem], selected: Option<u64>) -> Option<u64> {
    if problems.is_empty() {
        crate::callout::callout(ui, crate::callout::Tone::Info, tr!("TEMPLATE_CHECK_OK"));
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
            crate::callout::warning_icon(ui, problem_color(ui, p.severity));
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
fn compare_bar_body(ui: &mut egui::Ui) -> (bool, bool) {
    let accent = ui.visuals().selection.bg_fill;
    let frame = egui::Frame::popup(ui.style())
        .inner_margin(egui::Margin::same(10))
        .stroke(egui::Stroke::new(1.5, accent))
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
fn template_differs_body(
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

/// When a file was last written, if it can be told.
fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A template being edited in the window (see `template_edit`), the project set aside
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
        Box<crate::template_ui::PropertiesForm>,
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

/// The name of the template edited in `doc`, as shown: vendor and name.
fn edited_name(doc: &Document) -> String {
    let Some(config) = crate::template_edit::config(doc) else {
        return String::new();
    };
    let name = config.name.clone().unwrap_or_default();
    match &config.vendor {
        Some(v) => format!("{v} - {name}"),
        None => name,
    }
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

/// The file a project is saved to: with the project extension added if it lacks it.
fn project_save_path(path: PathBuf) -> PathBuf {
    if project::is_archive(&path) {
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
        let user_templates_dir = template::user_templates_dir();
        let templates = template::scan_all(templates_dir.as_deref(), user_templates_dir.as_deref());
        let mut messages = Vec::new();
        if templates.is_empty() {
            messages.push(tr!("NO_TEMPLATES_FOUND").to_owned());
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
            autosave: crate::recovery::Autosave::new(
                crate::recovery::root(),
                cc.storage
                    .and_then(|s| eframe::get_value(s, crate::recovery::STORAGE_KEY))
                    .unwrap_or(crate::recovery::DEFAULT_INTERVAL),
            ),
            orphans: crate::recovery::root()
                .as_deref()
                .map(crate::recovery::orphans)
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

    /// Picks an installed template by folder id: the project gets its own copy of it, then
    /// uses it.
    fn select_template(&mut self, id: Option<String>) {
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
    fn load_template(&mut self) {
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
                // Line colors set in template.json.
                for e in &t.elements {
                    if let Some([r, g, b]) = e.color {
                        self.canvas.element_color[e.kind.index()] = Color32::from_rgb(r, g, b);
                    }
                }
                self.template = Some(t);
                // Its variables, which the project's layers can use.
                self.doc.template_variables = entry.config.variables.clone();
                self.template_entry = Some(entry);
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// The installed template of folder `id`.
    fn installed(&self, id: &str) -> Option<&TemplateEntry> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// The installed template differs from the project's copy (and was not set aside):
    /// its fingerprint.
    fn installed_differs(&self) -> Option<String> {
        let fp = self.installed_fingerprint.clone()?;
        (Some(&fp) != self.copy_fingerprint().as_ref()
            && self.kept_fingerprint.as_ref() != Some(&fp))
        .then_some(fp)
    }

    /// Replaces the project's copy of its template with the installed one, placed as it
    /// says. Undo brings the copy back.
    fn use_installed_template(&mut self) {
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
    fn compare_installed_template(&mut self) {
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
    fn template_differs_note(&mut self, ui: &mut egui::Ui) {
        let Some(fp) = self.installed_differs() else {
            self.comparing = None;
            return;
        };
        if self.postponed_fingerprint.as_ref() != Some(&fp) || self.comparing.is_some() {
            return;
        }
        ui.add_space(4.0);
        crate::callout::callout_small(
            ui,
            crate::callout::Tone::Info,
            tr!("TEMPLATE_DIFFERS_TITLE"),
        );
        if ui.button(tr!("REVIEW_TEMPLATE_CHANGE")).clicked() {
            self.postponed_fingerprint = None;
        }
    }

    /// The popup asking what to do when the installed template differs from the
    /// project's copy: compare them on the canvas, keep the project's, or use the
    /// installed one. Esc puts it off (see `template_differs_note`).
    fn template_differs_modal(&mut self, ctx: &egui::Context) {
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

    /// While comparing the templates: a bar at the bottom of the canvas with the popup's
    /// choices (the labels at the top of the canvas say which side is which).
    fn compare_bar(&mut self, ctx: &egui::Context) {
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
    fn template_choice(&mut self, compare: bool, keep: bool, use_installed: bool, fp: String) {
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
    fn template_versions(&self) -> (String, String) {
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

    /// While comparing: the installed template's elements, placed on the page as it says.
    fn compared_elements(&self) -> Option<Vec<PlacedElement>> {
        let (_, t) = self.comparing.as_ref()?;
        Some(render::placed_elements(&self.doc, t))
    }

    /// Sets the page the current template asks for in its `template.json` (what it
    /// leaves out stays); the template is centered on it. Done when a template is picked
    /// or a project started, not when one is opened: a project keeps its page.
    fn use_template_page(&mut self) {
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
    fn current_entry(&self) -> Option<&TemplateEntry> {
        self.template_entry.as_ref()
    }

    /// The dialog naming, changing or deleting a template, and what it decides.
    fn template_dialog_modal(&mut self, ctx: &egui::Context) {
        use crate::template_ui::Outcome;
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
            Outcome::EditCopy => {
                self.edit_template(true);
                Ok(())
            }
        };
        if let Err(e) = result {
            self.messages.push(e);
        }
    }

    /// The Template properties panel: the settings of the template being edited, taken
    /// at once (Undo goes back; written when it is saved). A new page moves the shapes
    /// with the template, centered on it.
    fn template_settings_panel(&mut self, ui: &mut egui::Ui) {
        let compare = self
            .template_source_entry()
            .map(|_| self.editing.as_ref().is_some_and(|e| e.comparing.is_some()));
        let keys = self.shortcuts.texts(ui.ctx());
        let compare_hint = crate::shortcuts::with_key(
            tr!("COMPARE_TEMPLATE_SOURCE_HINT"),
            keys.get(&Command::CompareTemplateSource),
        );
        let (Some(editing), Some(config)) =
            (&mut self.editing, crate::template_edit::config(&self.doc))
        else {
            return;
        };
        if editing.form.as_ref().is_none_or(|(_, seen)| seen != config) {
            editing.form = Some((
                Box::new(crate::template_ui::PropertiesForm::new(config)),
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
        crate::template_edit::set_config(&mut self.doc, config);
        // Moved with their shapes, linked positions are not changes by hand.
        self.applied.clear();
        if page != self.doc.page_mm() {
            self.canvas.fit_pending = true;
        }
        self.dirty = true;
    }

    /// The Template properties panel's buttons about the template's folder: Open folder,
    /// and Delete... in red (it asks first).
    fn template_folder_buttons(&mut self, ui: &mut egui::Ui) {
        let keys = self.shortcuts.texts(ui.ctx());
        let tip =
            |hint: &str, command: Command| crate::shortcuts::with_key(hint, keys.get(&command));
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
                    .add_enabled(
                        can_delete,
                        egui::Button::new(
                            egui::RichText::new(tr!("DELETE_TEMPLATE_ELLIPSIS"))
                                .color(Color32::WHITE),
                        )
                        .fill(DANGER_COLOR),
                    )
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
    fn edited_problems(&mut self) -> Vec<Problem> {
        let Some(editing) = &mut self.editing else {
            return Vec::new();
        };
        let key = self.history.key();
        match &editing.check {
            Some((checked, problems)) if checked == key => problems.clone(),
            _ => {
                let problems = crate::template_check::check_document(&self.doc);
                editing.check = Some((key.to_owned(), problems.clone()));
                problems
            }
        }
    }

    /// The Template check panel: what makes the template being edited invalid, errors
    /// first; a click on a problem about a shape selects it.
    /// It is checked again after every change; Check, at its bottom, checks it again now.
    fn template_check_panel(&mut self, ui: &mut egui::Ui) {
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
    fn template_source_entry(&self) -> Option<&TemplateEntry> {
        let editing = self.editing.as_ref()?;
        if let Some(e) = &editing.copy_of {
            return Some(e);
        }
        let source = crate::template_edit::config(&self.doc)?.source.as_ref()?;
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
    fn toggle_source_compare(&mut self) {
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
    /// of the canvas to end it.
    fn source_compare_bar(&mut self, ctx: &egui::Context) {
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
                    .stroke(egui::Stroke::new(1.5, accent))
                    .show(ui, |ui| {
                        ui.spacing_mut().button_padding = egui::vec2(16.0, 8.0);
                        end = ui
                            .button(egui::RichText::new(tr!("END_COMPARE")).size(16.0))
                            .clicked();
                    });
            });
        if end {
            self.toggle_source_compare();
        }
    }

    /// Shows the Template check panel of the template being edited.
    fn show_template_check(&mut self) {
        if self.editing.is_some() {
            self.panels.set_open(PanelId::TemplateCheck, true);
        }
    }

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

    /// The panels left out now.
    fn hidden_panels(&self) -> &'static [PanelId] {
        if self.editing.is_some() {
            &Self::HIDDEN_WHILE_EDITING
        } else {
            &Self::ONLY_WHILE_EDITING
        }
    }

    /// The template "Edit template" opens: the installed one the project uses, or its
    /// own copy when it isn't installed.
    fn template_to_edit(&self) -> Option<&TemplateEntry> {
        self.doc
            .template
            .as_deref()
            .and_then(|id| self.installed(id))
            .or_else(|| self.current_entry())
    }

    /// The name of a copy of the template `name` of `vendor`: "name copy", or "name copy
    /// (2)" and so on, so that no two templates of the vendor are listed alike.
    fn copy_name(&self, name: &str, vendor: Option<&str>) -> String {
        let key = |v: Option<&str>| v.map(|v| v.trim().to_lowercase());
        let taken = |n: &str| {
            self.templates
                .iter()
                .any(|t| key(t.vendor.as_deref()) == key(vendor) && t.name.eq_ignore_ascii_case(n))
        };
        let first = tr!("DUPLICATE_LAYER_NAME", name = name).to_owned();
        if !taken(&first) {
            return first;
        }
        (2..)
            .map(|n| tr!("DUPLICATE_LAYER_NAME_NUMBERED", name = name, n = n).to_owned())
            .find(|c| !taken(c))
            .expect("some number is free")
    }

    /// Edits the project's template in the window. A provided template, or one the
    /// project alone has, is copied into the user's templates when saved, named as a copy.
    /// A copy is edited only once the user agreed to (`confirmed`): asked first.
    fn edit_template(&mut self, confirmed: bool) {
        let Some(entry) = self.template_to_edit() else {
            return;
        };
        let user = entry.origin == template::Origin::User
            && self
                .installed(&entry.id)
                .is_some_and(|e| e.folder == entry.folder);
        if !user && !confirmed {
            let provided = entry.origin == template::Origin::Provided
                && self
                    .installed(&entry.id)
                    .is_some_and(|e| e.folder == entry.folder);
            self.template_dialog = Some(crate::template_ui::Dialog::EditCopy {
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
            config.name = Some(self.copy_name(&entry.name, entry.vendor.as_deref()));
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
        let doc = match crate::template_edit::open(entry, config) {
            Ok(doc) => doc,
            Err(e) => {
                self.messages.push(e);
                return;
            }
        };
        let from = self.doc.template.clone();
        self.start_template_editing(id, copy_of, from, doc);
    }

    /// Starts a new template named `name`: an empty outline to draw in.
    fn new_template(&mut self, _ctx: &egui::Context, name: String, vendor: Option<String>) {
        let config = template::TemplateConfig {
            name: Some(name),
            vendor,
            version: Some(template::FIRST_VERSION.to_owned()),
            ..Default::default()
        };
        self.start_template_editing(None, None, None, crate::template_edit::new_document(config));
    }

    /// Puts the project aside and edits `doc`, the template of folder `id`.
    fn start_template_editing(
        &mut self,
        id: Option<String>,
        copy_of: Option<TemplateEntry>,
        from: Option<String>,
        doc: Document,
    ) {
        if self.editing.is_some() {
            return;
        }
        let config_before = crate::template_edit::config(&doc)
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

    /// What follows a change of the document in hand: nothing selected or being drawn,
    /// the panels that fit it, the page in view.
    fn after_switching_documents(&mut self) {
        let editing = self.editing.is_some();
        self.canvas.points = None;
        self.canvas.drawing = None;
        self.canvas.brushes.tool = None;
        self.layers_panel.edit_points = None;
        self.layers_panel.template_mode = editing;
        self.comparing = None;
        self.applied.clear();
        self.canvas.fit_pending = true;
        self.dirty = self.is_modified();
    }

    /// Writes the template being edited into its folder of the user's templates (a new
    /// one into a new folder, named after it).
    fn save_template_edit(&mut self) {
        let Some(editing) = &self.editing else {
            return;
        };
        let Some(dir) = self.user_templates_dir.clone() else {
            self.messages
                .push(tr!("NO_USER_TEMPLATES_FOLDER").to_owned());
            return;
        };
        let Some(config) = crate::template_edit::config(&self.doc).cloned() else {
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
        match crate::template_edit::save(&self.doc, &folder) {
            Ok(config) => {
                // The files it lists now: the next save deletes those it no longer does.
                if let Some(e) = &mut self.doc.edited_template {
                    e.config = config;
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
    fn end_template_editing(&mut self) {
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
    fn end_template_editing_now(&mut self) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let config = crate::template_edit::config(&self.doc)
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
    fn template_bar(&mut self, ui: &mut egui::Ui) {
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
    fn template_end_modal(&mut self, ctx: &egui::Context) {
        if !self.editing.as_ref().is_some_and(|e| e.confirm_end) {
            return;
        }
        let name = edited_name(&self.doc);
        let (mut save, mut discard, mut cancel) = (false, false, false);
        let response = dialog(ctx, "confirm_end_template").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.strong(tr!("CONFIRM_SAVE_TEMPLATE", name = name));
            ui.add_space(4.0);
            crate::callout::callout(
                ui,
                crate::callout::Tone::Warning,
                tr!("UNSAVED_TEMPLATE_WARNING"),
            );
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                save |= ui.button(tr!("SAVE")).clicked();
                cancel |= ui.button(tr!("CANCEL")).clicked();
                at_left_edge(ui, |ui| {
                    discard |= ui.button(tr!("DONT_SAVE")).clicked();
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
    fn edited_user_template(&self) -> Option<&TemplateEntry> {
        let id = self.editing.as_ref()?.id.as_deref()?;
        self.installed(id)
            .filter(|e| e.origin == template::Origin::User)
    }

    /// The folder of the template being edited: its saved one in your templates, or the
    /// provided template it is a copy of until saved. `None` for a new one not saved yet.
    fn edited_template_folder(&self) -> Option<&Path> {
        let editing = self.editing.as_ref()?;
        match editing.id.as_deref() {
            Some(id) => self.installed(id).map(|e| e.folder.as_path()),
            None => editing.copy_of.as_ref().map(|e| e.folder.as_path()),
        }
    }

    /// Shows the folder of the template being edited in the system's file manager.
    fn open_template_folder(&mut self) {
        if let Some(folder) = self.edited_template_folder()
            && let Err(e) = crate::file_dialog::open_folder(folder)
        {
            self.messages.push(e);
        }
    }

    /// Deletes the user template being edited, and ends editing it. Projects that use it
    /// keep their own copy of it.
    fn delete_template(&mut self, replacement: Option<String>) -> Result<(), String> {
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
    fn project_uses_edited_template(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|e| e.id.is_some() && e.project.template == e.id)
    }

    /// The question asked before deleting the template being edited.
    fn ask_delete_template(&mut self) {
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
        self.template_dialog = Some(crate::template_ui::Dialog::Delete {
            name: entry.full_name(),
            others,
            replacement: None,
        });
    }

    /// Notices when the installed template of the project's changes on disk (its
    /// version is then compared with the project's copy, see `installed_differs`).
    fn watch_template(&mut self, ctx: &egui::Context) {
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
                problems: crate::template_check::check_entry(&fresh),
                ..fresh
            };
        }
        self.refresh_installed();
    }

    /// Reads again the installed template of the project's: its stamp, its fingerprint,
    /// and what is compared with it.
    fn refresh_installed(&mut self) {
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
        // A random word to type over, which names it until it is renamed.
        let word = layers_ui::random_name(&self.doc);
        let layer = text::new_layer(id, &word, self.fonts.default_family(), center);
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
    /// polylines and polygons are drawn point by point on the canvas instead; paths come
    /// from an SVG file, asked for.
    fn add_shape(&mut self, ctx: &egui::Context, kind: ShapeKind) {
        // In a template, only into the element selected (or the one of the shape selected).
        if self.editing.is_some()
            && self
                .selected
                .and_then(|id| crate::template_edit::element_of(&self.doc, id))
                .is_none()
        {
            return;
        }
        self.layers_panel.shape_kind = kind;
        if kind == ShapeKind::Path {
            self.pick_svg(ctx);
            return;
        }
        if kind.has_points() {
            self.canvas.drawing = Some(view::Drawing::new(kind));
            self.canvas.points = None;
            self.layers_panel.edit_points = None;
            return;
        }
        let id = self.doc.alloc_id();
        let center = self.new_layer_center();
        let mut layer = shape::new_layer(id, kind, center);
        layer.name = layers_ui::random_name(&self.doc);
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
        let mut layer =
            shape::new_canvas_layer(id, kind, [c[0] - w, c[1] - h], [c[0] + w, c[1] + h]);
        layer.name = layers_ui::random_name(&self.doc);
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

    /// Adds files dropped on the window: SVG files as shapes (see `add_svg`), others as
    /// images.
    fn add_dropped(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let (svgs, images): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| SVG_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        });
        for path in svgs {
            self.add_svg(ctx, &path);
        }
        // A template is made of shapes only.
        if !images.is_empty() && self.editing.is_none() {
            self.add_images(ctx, images);
        }
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

    /// Adds an SVG file's shapes (and its groups) above the selection, at its true size
    /// unless larger than the page.
    fn add_svg(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        // In a template, only into the element selected (or the one of the shape selected).
        if self.editing.is_some()
            && self
                .selected
                .and_then(|id| crate::template_edit::element_of(&self.doc, id))
                .is_none()
        {
            self.messages
                .push(tr!("ADD_SHAPE_SELECT_ELEMENT").to_owned());
            return;
        }
        let (page, center) = (self.doc.page_mm(), self.doc.page_center());
        let doc = &mut self.doc;
        match crate::svg::import(path, page, center, &mut || doc.alloc_id()) {
            Ok(imported) => {
                // Bottom first, each just above the one before (not inside it, when it
                // is a group); the top one ends selected.
                let mut at = layers_ui::insertion_path(&mut self.doc, self.selected);
                for node in imported.nodes {
                    self.selected = Some(node.id());
                    self.doc.insert(&at, node);
                    if let Some(last) = at.last_mut() {
                        *last += 1;
                    }
                }
                self.dirty = true;
                self.messages.extend(imported.notices);
                shape::sync(ctx, &mut self.doc, true);
            }
            Err(e) => self.messages.push(e),
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
            Purpose::ImportSvg => {
                paths.insert(0, chosen);
                for path in paths {
                    self.add_svg(ctx, &path);
                }
                return;
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
            Purpose::AddImages | Purpose::ImportSvg | Purpose::OpenProject => {}
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

    /// Opens the dialog to add the shapes of SVG files as path layers.
    fn pick_svg(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::ImportSvg,
                title: tr!("IMPORT_SVG_DIALOG_TITLE"),
                save: false,
                multiple: true,
                filters: SVG_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Stores the app's own settings that are not the project's: the Print panel's and
    /// the View panel's display settings.
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
            Leave::Recover => self.recover(ctx),
        }
    }

    /// Opens the first copy left by a crash, as the project it was made from with its
    /// changes unsaved. A copy that can't be opened is kept, to be offered again at the
    /// next start.
    fn recover(&mut self, ctx: &egui::Context) {
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
    fn recovery_modal(&mut self, ctx: &egui::Context) {
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
    fn autosave_menu(&mut self, ui: &mut egui::Ui) {
        for minutes in crate::recovery::INTERVALS {
            let label = if minutes == 0 {
                tr!("AUTOSAVE_OFF").to_owned()
            } else {
                tr_n!("AUTOSAVE_EVERY", minutes)
            };
            ui.radio_value(&mut self.autosave.minutes, minutes, label);
        }
    }

    /// Starts an empty project, keeping the current template.
    fn new_project(&mut self) {
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
    fn open_project(&mut self, ctx: &egui::Context) {
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
                // Its own copy of its template, placed as that says.
                self.kept_fingerprint = None;
                self.postponed_fingerprint = None;
                self.load_template();
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
                    filters: PROJECT_FILTERS,
                    default_name: format!("{}.{}", self.project_name(), project::EXTENSION),
                },
            ),
        }
    }

    /// Writes the project to `path` and marks it as saved.
    fn save_to(&mut self, path: PathBuf) {
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

    /// File, Edit and View menus.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
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

    /// Lists the templates folders again and reloads the current template.
    fn rescan_templates(&mut self) {
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
    }

    /// The View panel: how the canvas shows the template and what is cut out of the
    /// panel, what layers snap to, and the rulers and guides. Kept with the app's
    /// preferences, not the project.
    fn view_panel(&mut self, ui: &mut egui::Ui) {
        // Neither the template nor the export changes with the previews. The slider sets
        // how much the cut preview covers.
        let keys = self.shortcuts.texts(ui.ctx());
        let tip =
            |hint: &str, command: Command| crate::shortcuts::with_key(hint, keys.get(&command));
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
                    .range(0.0..=crate::print::MAX_MARGIN_MM)
                    .speed(0.1)
                    .fixed_decimals(1),
            )
            .on_hover_text(tr!("SAFE_MARGIN_HINT"));
            crate::layers_ui::unit(ui, "mm");
        });

        // What layers snap to while dragged.
        let has_centers = self.has_centers();
        let canvas = &mut self.canvas;
        crate::layers_ui::section(
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
        crate::layers_ui::section(
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
                        cleared = view::clear_guides(doc);
                    }
                });
            },
        );
        self.dirty |= cleared;
    }

    /// A panel's contents; the Layers panel returns the height it would take.
    fn panel_body(&mut self, ui: &mut egui::Ui, id: PanelId) -> Option<f32> {
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
                self.dirty |= variables::variables_ui(ui, &mut self.doc, kinds)
            }
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
        if let Some(kind) = out.add_element {
            let id = crate::template_edit::add_element(&mut self.doc, kind);
            self.selected = Some(id);
            self.dirty = true;
        }
    }

    /// View > Panels: each panel shown or hidden, and the default layout.
    fn panels_menu(&mut self, ui: &mut egui::Ui) {
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

    /// The Template panel: the template to work on, and its elements' lines on the
    /// canvas. How it sits on the page comes from the template itself.
    fn template_panel(&mut self, ui: &mut egui::Ui) {
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
        ui.allocate_ui_with_layout(
            row,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                self.edit_template_button(ui);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    choice = template_selector(ui, &current, &self.templates, &self.doc.template).0;
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
                ui.weak(tr_n!("SHAPE_COUNT", e.polylines.len()));
            });
        }
    }

    /// The Template panel's Edit button, next to the template field. A provided template
    /// is copied into the user's templates when saved.
    fn edit_template_button(&mut self, ui: &mut egui::Ui) {
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
            .on_hover_text(crate::shortcuts::with_key(
                hint,
                keys.get(&Command::EditTemplate),
            ))
            .clicked()
        {
            self.run_command(ui.ctx(), Command::EditTemplate);
        }
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
                .on_hover_text(crate::shortcuts::with_key(
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
    fn save_status(&self, ui: &mut egui::Ui) {
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
        let before = self.selected;
        let Some(pasted) = self
            .clipboard
            .paste(&mut self.doc, &mut self.selected, text)
        else {
            return;
        };
        // In a template, shapes go into an element and elements at the top of the list.
        if self.editing.is_some() {
            let parent = self.doc.path_of(pasted.id).map(|mut p| {
                p.pop();
                p
            });
            if !parent.is_some_and(|p| crate::template_edit::fits_in(&self.doc, pasted.id, &p)) {
                self.doc.remove(pasted.id);
                self.selected = before;
                self.messages
                    .push(tr!("TEMPLATE_PASTE_OUTSIDE_ELEMENT").to_owned());
                return;
            }
        }
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

    /// The template has shapes with a center to snap to: closed ones.
    fn has_centers(&self) -> bool {
        self.template
            .as_ref()
            .is_some_and(|t| t.elements.iter().any(|e| !e.centers.is_empty()))
    }

    /// The command does something while a template is edited.
    fn works_while_editing(command: Command) -> bool {
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
                | Command::Group
                | Command::Ungroup
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
                | Command::ClearPaint
        )
    }

    /// Does what a shortcut's command does.
    fn run_command(&mut self, ctx: &egui::Context, command: Command) {
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
            Command::ClearGuides => self.dirty |= view::clear_guides(&mut self.doc),
            Command::KeyboardShortcuts => self.open_shortcuts(),
            Command::NewTemplate => {
                self.template_dialog = Some(crate::template_ui::Dialog::New {
                    name: String::new(),
                    vendor: String::new(),
                });
            }
            Command::EditTemplate => self.edit_template(false),
            Command::DeleteTemplate => self.ask_delete_template(),
            Command::OpenTemplateFolder => self.open_template_folder(),
            Command::CheckTemplate => self.show_template_check(),
            Command::CompareTemplateSource => self.toggle_source_compare(),
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

impl crate::panels::Contents for ArtworkApp {
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
        self.panels.save(storage);
        self.shortcuts.save(storage);
        self.save_settings(storage);
        eframe::set_value(
            storage,
            crate::recovery::STORAGE_KEY,
            &self.autosave.minutes,
        );
    }

    /// Closing normally: nothing to recover, so the autosave copy goes.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.autosave.end();
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
        if self.editing.is_some() {
            egui::Panel::top("template_bar")
                .frame(egui::Frame::NONE)
                .show(ui, |ui| self.template_bar(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, &elements));
        // Taken out while drawn, as it draws the panels with the app's contents.
        let mut panels = std::mem::take(&mut self.panels);
        // Set here, not when editing starts: a panel's button may start it while the
        // layout is taken out.
        panels.set_hidden(self.hidden_panels());
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
        // While comparing, the installed template left of the divider; in the template
        // editor, the source of the template being edited.
        self.canvas.compare = match self.editing.as_ref().map(|e| &e.comparing) {
            Some(Some((name, t))) => Some(view::Compare {
                elements: render::placed_elements(&self.doc, t),
                labels: [
                    tr!("COMPARE_SOURCE_LABEL", name = name),
                    tr!("COMPARE_EDITED_LABEL").to_owned(),
                ],
            }),
            Some(None) => None,
            None => self.compared_elements().map(|elements| {
                let (project, installed) = self.template_versions();
                view::Compare {
                    elements,
                    labels: [
                        tr!("COMPARE_INSTALLED_LABEL", version = installed),
                        tr!("COMPARE_PROJECT_LABEL", version = project),
                    ],
                }
            }),
        };
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
        self.template_differs_modal(&ctx);
        self.template_dialog_modal(&ctx);
        self.compare_bar(&ctx);
        self.source_compare_bar(&ctx);
        self.close_modal(&ctx);
        self.template_end_modal(&ctx);
        self.recovery_modal(&ctx);
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
        // A template's shapes take their element's look.
        if self.editing.is_some() {
            crate::template_edit::sync(&mut self.doc);
        }
        self.history.update(&self.doc, finished);
        self.dirty = self.is_modified();
        self.update_title(&ctx);
        let project = crate::recovery::Project {
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
            self.messages.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the template selector with its list open, to look at it:
    /// `TEMPLATE_SELECTOR_PREVIEW=out.png cargo test template_selector_preview -- --ignored`.
    #[test]
    #[ignore]
    fn template_selector_preview() {
        let Ok(path) = std::env::var("TEMPLATE_SELECTOR_PREVIEW") else {
            return;
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let templates = template::scan(&dir);
        let chosen = templates.first().map(|t| t.id.clone());
        crate::preview::render(&path, egui::vec2(320.0, 200.0), |ui| {
            ui.set_width(300.0);
            let (_, field) = template_selector(ui, "Cosmox - T16 (1.0)", &templates, &chosen);
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&field));
        });
    }

    /// Draws a template's problems as the Template check panel and the template list's
    /// tooltip show them, to look at them:
    /// `TEMPLATE_CHECK_PREVIEW=out.png cargo test template_check_preview -- --ignored`.
    #[test]
    #[ignore]
    fn template_check_preview() {
        let Ok(path) = std::env::var("TEMPLATE_CHECK_PREVIEW") else {
            return;
        };
        let problem = |severity, text: &str, layer| Problem {
            severity,
            text: text.to_owned(),
            layer,
        };
        let problems = [
            problem(Severity::Warning, "Menu buttons: no shapes", None),
            problem(
                Severity::Error,
                "Action buttons: Circle 30.0 mm crosses the outline",
                Some(1),
            ),
            problem(
                Severity::Error,
                "Circle 24.0 mm (Action buttons) and Circle 30.0 mm (Action buttons) overlap",
                Some(2),
            ),
        ];
        crate::preview::render(&path, egui::vec2(560.0, 260.0), |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(260.0);
                    problem_list(ui, &problems, Some(1));
                    ui.add_space(12.0);
                    problem_list(ui, &[], None);
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(260.0);
                        problems_tooltip(ui, &problems);
                    });
                    ui.add_space(12.0);
                    let mark = crate::callout::warning_mark(problem_color(ui, Severity::Error));
                    let _ = ui.add(egui::Button::selectable(
                        false,
                        (
                            "T16",
                            mark,
                            egui::Atom::grow(),
                            egui::RichText::new("1.2").weak(),
                        ),
                    ));
                    let mark = crate::callout::warning_mark(problem_color(ui, Severity::Error));
                    let _ = ui.add(egui::Button::new((mark, "Check")));
                });
            });
        });
    }

    /// Draws the bar shown while comparing templates, to look at it:
    /// `COMPARE_BAR_PREVIEW=out.png cargo test compare_bar_preview -- --ignored`.
    #[test]
    #[ignore]
    fn compare_bar_preview() {
        let Ok(path) = std::env::var("COMPARE_BAR_PREVIEW") else {
            return;
        };
        crate::preview::render(&path, egui::vec2(520.0, 110.0), |ui| {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                compare_bar_body(ui);
            });
        });
    }

    /// Draws the popup about a changed installed template, to look at it:
    /// `TEMPLATE_DIFFERS_PREVIEW=out.png cargo test template_differs_preview -- --ignored`.
    #[test]
    #[ignore]
    fn template_differs_preview() {
        let Ok(path) = std::env::var("TEMPLATE_DIFFERS_PREVIEW") else {
            return;
        };
        crate::preview::render(&path, egui::vec2(520.0, 260.0), |ui| {
            egui::Frame::window(ui.style())
                .inner_margin(egui::Margin::same(DIALOG_MARGIN))
                .show(ui, |ui| {
                    template_differs_body(ui, "Cosmox - T16", "1.0", "1.1")
                });
        });
    }

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
        let ours = format!("/a/stick.{}", project::EXTENSION);
        assert_eq!(p(&ours), PathBuf::from(&ours));
        // A .json file is no longer a project: saved as an archive beside it.
        assert_eq!(
            p("/a/stick.json"),
            PathBuf::from(format!("/a/stick.json.{}", project::EXTENSION))
        );
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
