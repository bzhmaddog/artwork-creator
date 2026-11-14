//! Templates in the app's UI: the dialogs naming a new one and deleting the one edited,
//! and the form of its settings (its `template.json`) in the Template properties panel.
//! Provided templates are never changed: editing one makes a copy in the user's
//! templates when saved.

use egui::Ui;

use crate::app::{dialog, dialog_button_row};
use crate::document::{CUSTOM_PAGE_RANGE_MM, Orientation, P2, PageSize};
use crate::i18n::tr;
use crate::template::{PageConfig, PageSizeConfig, TemplateConfig, TemplateEntry};

/// Width of the dialogs' fields.
const FIELD_WIDTH: f32 = 220.0;

/// A dialog about templates, open over the window.
pub enum Dialog {
    /// Names a new template before it is edited.
    New { name: String, vendor: String },
    /// Asks before deleting the user template being edited.
    Delete {
        /// As shown.
        name: String,
        /// The templates the open project may use instead, when it uses this one.
        others: Option<Vec<TemplateEntry>>,
        /// The one chosen among them; `None` for no template.
        replacement: Option<String>,
    },
    /// Says, before editing a template that can't be changed (provided with the app, or
    /// only in the project), that a copy is edited.
    EditCopy {
        /// The template's name, as shown.
        name: String,
        /// The copy's.
        copy: String,
        /// Provided with the app (else only the project has it).
        provided: bool,
    },
}

/// What the user decided in a dialog.
pub enum Outcome {
    /// Still open.
    Open,
    /// Closed without doing anything.
    Canceled,
    /// Start a new template of this name and vendor.
    New {
        name: String,
        vendor: Option<String>,
    },
    /// Delete the template being edited; the project, if it used it, uses `replacement`
    /// (`None`: no template).
    Delete { replacement: Option<String> },
    /// Edit a copy of the template.
    EditCopy,
}

impl Dialog {
    /// Shows the dialog and returns what was decided.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        match self {
            Dialog::New { name, vendor } => new_dialog(ctx, name, vendor),
            Dialog::Delete {
                name,
                others,
                replacement,
            } => delete_dialog(ctx, name, others.as_deref(), replacement),
            Dialog::EditCopy {
                name,
                copy,
                provided,
            } => edit_copy_dialog(ctx, name, copy, *provided),
        }
    }
}

/// `text` trimmed, `None` when nothing is left.
fn non_empty(text: &str) -> Option<String> {
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// A text field of the dialogs' width (or less, in a narrow panel), with a hint shown
/// while it is empty.
fn text_field(ui: &mut Ui, text: &mut String, hint: &str) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(FIELD_WIDTH.min(ui.available_width())),
    )
}

/// Asks a new template's name and vendor.
fn new_dialog(ctx: &egui::Context, name: &mut String, vendor: &mut String) -> Outcome {
    let mut outcome = Outcome::Open;
    let response = dialog(ctx, "name_template").show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.strong(tr!("NEW_TEMPLATE_TITLE"));
        ui.add_space(4.0);
        crate::callout::callout(ui, crate::callout::Tone::Info, tr!("NEW_TEMPLATE_INFO"));
        ui.add_space(6.0);
        let mut enter = false;
        egui::Grid::new("new_template_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(tr!("TEMPLATE_NAME"));
                let field = text_field(ui, name, "");
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    enter = true;
                }
                // Typed over at once.
                if ui.memory(|m| m.focused().is_none()) {
                    field.request_focus();
                }
                ui.end_row();
                ui.label(tr!("TEMPLATE_VENDOR"));
                text_field(ui, vendor, tr!("TEMPLATE_VENDOR_HINT"));
                ui.end_row();
            });
        ui.add_space(8.0);
        let can = non_empty(name).is_some();
        let (ok, cancel) = crate::app::dialog_buttons(ui, tr!("NEW_TEMPLATE_BUTTON"), can);
        if (ok || (enter && can))
            && let Some(name) = non_empty(name)
        {
            let vendor = non_empty(vendor);
            outcome = Outcome::New { name, vendor };
        } else if cancel {
            outcome = Outcome::Canceled;
        }
    });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Canceled;
    }
    outcome
}

fn delete_dialog(
    ctx: &egui::Context,
    name: &str,
    others: Option<&[TemplateEntry]>,
    replacement: &mut Option<String>,
) -> Outcome {
    let mut outcome = Outcome::Open;
    let response = dialog(ctx, "delete_template").show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.strong(tr!("CONFIRM_DELETE_TEMPLATE", name = name));
        ui.add_space(4.0);
        crate::callout::callout(
            ui,
            crate::callout::Tone::Warning,
            tr!("DELETE_TEMPLATE_WARNING"),
        );
        if let Some(others) = others {
            ui.add_space(8.0);
            ui.label(tr!("DELETE_TEMPLATE_IN_USE"));
            let current = replacement
                .as_deref()
                .and_then(|id| others.iter().find(|t| t.id == id))
                .map_or_else(|| tr!("NO_TEMPLATE").to_owned(), |t| t.full_name());
            *replacement = crate::app::template_selector(ui, &current, others, replacement).0;
        }
        ui.add_space(8.0);
        dialog_button_row(ui, |ui| {
            if ui.button(tr!("DELETE_TEMPLATE_BUTTON")).clicked() {
                outcome = Outcome::Delete {
                    replacement: replacement.clone(),
                };
            }
            if ui.button(tr!("CANCEL")).clicked() {
                outcome = Outcome::Canceled;
            }
        });
    });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Canceled;
    }
    outcome
}

/// Says that editing the template `name` edits a copy named `copy`, saved into the
/// user's templates; Edit a copy goes on.
fn edit_copy_dialog(ctx: &egui::Context, name: &str, copy: &str, provided: bool) -> Outcome {
    let mut outcome = Outcome::Open;
    let response = dialog(ctx, "edit_template_copy").show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.strong(tr!("EDIT_TEMPLATE_COPY_TITLE", name = name));
        ui.add_space(4.0);
        let why = if provided {
            tr!("EDIT_TEMPLATE_COPY_PROVIDED", name = name)
        } else {
            tr!("EDIT_TEMPLATE_COPY_PROJECT_ONLY", name = name)
        };
        crate::callout::callout(ui, crate::callout::Tone::Info, &why);
        ui.add_space(4.0);
        ui.label(tr!("EDIT_TEMPLATE_COPY_INFO", copy = copy));
        ui.add_space(8.0);
        let (ok, cancel) = crate::app::dialog_buttons(ui, tr!("EDIT_TEMPLATE_COPY_BUTTON"), true);
        if ok {
            outcome = Outcome::EditCopy;
        } else if cancel {
            outcome = Outcome::Canceled;
        }
    });
    if response.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Canceled;
    }
    outcome
}

/// The settings of the template being edited, as changed in the Template properties
/// panel.
pub struct PropertiesForm {
    /// Its settings as read: what the form doesn't show is kept.
    config: TemplateConfig,
    vendor: String,
    name: String,
    version: String,
    /// The page it is made for; `None` leaves the project's as it is.
    page: Option<PageSize>,
    /// For a custom page: width and height as given, in mm.
    custom_mm: P2,
    /// `None` leaves the project's, or follows a custom size's.
    orientation: Option<Orientation>,
    /// A page setting the form can't show (an unknown name): kept unless changed.
    page_error: Option<String>,
}

impl PropertiesForm {
    pub fn new(config: &TemplateConfig) -> Self {
        let config = config.clone();
        let page_cfg = config.page.clone().unwrap_or_default();
        let (page, page_error) = match page_cfg.setup() {
            Ok(setup) => (setup.size, None),
            Err(value) => (None, Some(value)),
        };
        let custom_mm = match &page_cfg.size {
            Some(PageSizeConfig::Mm(mm)) => *mm,
            _ => [250.0, 400.0],
        };
        let orientation = match page_cfg.orientation.as_deref().map(str::trim) {
            Some(o) if o.eq_ignore_ascii_case("landscape") => Some(Orientation::Landscape),
            Some(o) if o.eq_ignore_ascii_case("portrait") => Some(Orientation::Portrait),
            _ => None,
        };
        Self {
            vendor: config.vendor.clone().unwrap_or_default(),
            name: config.name.clone().unwrap_or_default(),
            version: config.version.clone().unwrap_or_default(),
            page,
            custom_mm,
            orientation,
            page_error,
            config,
        }
    }

    /// The template's settings with the form's.
    fn config(&self) -> TemplateConfig {
        let mut config = self.config.clone();
        config.vendor = non_empty(&self.vendor);
        config.name = non_empty(&self.name);
        config.version = non_empty(&self.version);
        if self.page_error.is_none() {
            let size = self.page.map(|p| match p {
                PageSize::Custom => PageSizeConfig::Mm(self.custom_mm),
                p => PageSizeConfig::Named(format!("{p:?}")),
            });
            let orientation = self.orientation.map(|o| match o {
                Orientation::Landscape => "landscape".to_owned(),
                Orientation::Portrait => "portrait".to_owned(),
            });
            config.page = (size.is_some() || orientation.is_some())
                .then_some(PageConfig { size, orientation });
        }
        config
    }

    /// The page rows of the grid.
    fn page_rows(&mut self, ui: &mut Ui) {
        ui.label(tr!("PAGE_SECTION"));
        ui.vertical(|ui| {
            if let Some(value) = &self.page_error {
                crate::callout::callout_small(
                    ui,
                    crate::callout::Tone::Warning,
                    &tr!("TEMPLATE_PAGE_NOT_UNDERSTOOD", value = value),
                );
                if ui.button(tr!("TEMPLATE_PAGE_RESET")).clicked() {
                    self.page_error = None;
                }
                return;
            }
            let custom = self.custom_mm;
            let label = |p: Option<PageSize>| match p {
                Some(p) => p.label(custom),
                None => tr!("TEMPLATE_PAGE_NOT_SET").to_owned(),
            };
            egui::ComboBox::from_id_salt("template_page")
                .width(FIELD_WIDTH.min(ui.available_width()))
                .selected_text(label(self.page))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.page, None, label(None));
                    for p in PageSize::ALL {
                        ui.selectable_value(&mut self.page, Some(p), label(Some(p)));
                    }
                });
            if self.page == Some(PageSize::Custom) {
                ui.horizontal(|ui| {
                    for (i, prefix) in [tr!("PAGE_CUSTOM_WIDTH"), tr!("PAGE_CUSTOM_HEIGHT")]
                        .into_iter()
                        .enumerate()
                    {
                        ui.add(
                            egui::DragValue::new(&mut self.custom_mm[i])
                                .range(CUSTOM_PAGE_RANGE_MM)
                                .speed(1.0)
                                .fixed_decimals(1)
                                .prefix(prefix),
                        );
                    }
                    crate::layers_ui::unit(ui, "mm");
                });
            }
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.orientation, None, tr!("TEMPLATE_PAGE_NOT_SET"));
                ui.radio_value(
                    &mut self.orientation,
                    Some(Orientation::Landscape),
                    tr!("PAGE_LANDSCAPE"),
                );
                ui.radio_value(
                    &mut self.orientation,
                    Some(Orientation::Portrait),
                    tr!("PAGE_PORTRAIT"),
                );
            });
        });
        ui.end_row();
    }
}

impl PropertiesForm {
    /// The form's rows. `compare` is whether the template is being compared with its
    /// source, `None` when the source isn't installed; `compare_hint` the Compare
    /// button's tooltip. Returns the settings when they were changed, and whether Compare
    /// was clicked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        compare: Option<bool>,
        compare_hint: &str,
    ) -> (Option<TemplateConfig>, bool) {
        let mut compare_clicked = false;
        let before = self.config();
        egui::Grid::new("template_properties_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(tr!("TEMPLATE_NAME"));
                text_field(ui, &mut self.name, "");
                ui.end_row();
                ui.label(tr!("TEMPLATE_VENDOR"));
                text_field(ui, &mut self.vendor, tr!("TEMPLATE_VENDOR_HINT"));
                ui.end_row();
                ui.label(tr!("TEMPLATE_VERSION"));
                text_field(ui, &mut self.version, tr!("TEMPLATE_VERSION_HINT"));
                ui.end_row();
                // A copy: the template it was made from, kept as it is.
                if let Some(source) = &self.config.source {
                    ui.label(tr!("TEMPLATE_SOURCE"));
                    ui.horizontal(|ui| {
                        ui.label(source.describe())
                            .on_hover_text(tr!("TEMPLATE_SOURCE_HINT"));
                        compare_clicked = ui
                            .add_enabled(
                                compare.is_some(),
                                egui::Button::new(tr!("COMPARE_TEMPLATE_SOURCE_BUTTON"))
                                    .selected(compare == Some(true)),
                            )
                            .on_hover_text(compare_hint)
                            .on_disabled_hover_text(tr!("COMPARE_TEMPLATE_SOURCE_NONE_HINT"))
                            .clicked();
                    });
                    ui.end_row();
                }
                self.page_rows(ui);
            });
        if non_empty(&self.name).is_none() {
            crate::callout::callout_small(
                ui,
                crate::callout::Tone::Warning,
                tr!("TEMPLATE_NAME_REQUIRED"),
            );
        }
        match non_empty(&self.version) {
            None => {
                crate::callout::callout_small(
                    ui,
                    crate::callout::Tone::Warning,
                    tr!("TEMPLATE_VERSION_REQUIRED"),
                );
            }
            Some(v) if !crate::template::is_valid_version(&v) => {
                crate::callout::callout_small(
                    ui,
                    crate::callout::Tone::Warning,
                    &tr!("TEMPLATE_VERSION_INVALID", version = v),
                );
            }
            Some(_) => {}
        }
        let after = self.config();
        ((after != before).then_some(after), compare_clicked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> TemplateConfig {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn form_keeps_what_it_does_not_show() {
        let json = r#"{ "name": "T16", "units": "inch", "dxf": "a.dxf",
            "page": { "size": "A4", "orientation": "landscape" } }"#;
        let form = PropertiesForm::new(&config(json));
        assert_eq!(form.config(), serde_json::from_str(json).unwrap());
    }

    #[test]
    fn form_writes_custom_page() {
        let mut form = PropertiesForm::new(&config(r#"{ "name": "X" }"#));
        form.page = Some(PageSize::Custom);
        form.custom_mm = [300.0, 200.0];
        form.vendor = "  ".into();
        form.version = " 1.1 ".into();
        let config = form.config();
        assert_eq!(config.vendor, None);
        assert_eq!(config.version.as_deref(), Some("1.1"));
        assert_eq!(
            config.page.unwrap().size,
            Some(PageSizeConfig::Mm([300.0, 200.0]))
        );
    }

    #[test]
    fn form_keeps_a_page_it_does_not_understand() {
        let json = r#"{ "page": { "size": "B5" } }"#;
        let form = PropertiesForm::new(&config(json));
        assert!(form.page_error.is_some());
        assert_eq!(form.config().page, config(json).page);
    }
}

#[cfg(test)]
mod previews {
    use super::*;

    /// Draws the template dialogs and the settings form, to look at them:
    /// `TEMPLATE_DIALOGS_PREVIEW=out cargo test template_dialogs_preview -- --ignored`
    /// writes out-new.png, out-properties.png and out-delete.png.
    #[test]
    #[ignore]
    fn template_dialogs_preview() {
        let Ok(base) = std::env::var("TEMPLATE_DIALOGS_PREVIEW") else {
            return;
        };
        let config: TemplateConfig = serde_json::from_str(
            r#"{ "vendor": "Cosmox", "name": "T16 copy", "version": "1.0b",
                 "source": { "name": "T16", "vendor": "Cosmox", "version": "1.1" },
                 "page": { "size": "A4", "orientation": "landscape" } }"#,
        )
        .unwrap();
        let dialogs = [
            (
                "new",
                Dialog::New {
                    name: "T16".into(),
                    vendor: String::new(),
                },
            ),
            (
                "delete",
                Dialog::Delete {
                    name: "Cosmox - T16".into(),
                    others: Some(crate::template::scan(
                        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates"),
                    )),
                    replacement: None,
                },
            ),
        ];
        let dialogs = dialogs.into_iter().chain([(
            "edit-copy",
            Dialog::EditCopy {
                name: "Cosmox - T16".into(),
                copy: "T16 copy".into(),
                provided: true,
            },
        )]);
        for (name, dialog) in dialogs {
            let dialog = std::cell::RefCell::new(dialog);
            crate::preview::render(
                &format!("{base}-{name}.png"),
                egui::vec2(560.0, 480.0),
                |ui| {
                    dialog.borrow_mut().show(ui.ctx());
                },
            );
        }
        // The panel's form, as wide as the panel is at least.
        let form = std::cell::RefCell::new(PropertiesForm::new(&config));
        crate::preview::render(
            &format!("{base}-properties.png"),
            egui::vec2(320.0, 330.0),
            |ui| {
                ui.set_width(300.0);
                form.borrow_mut().show(ui, Some(false), "");
            },
        );
    }
}
