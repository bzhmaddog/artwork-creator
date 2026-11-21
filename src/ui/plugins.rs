//! The plugin manager (Tools > Plugins > Manage plugins...), also opened at the start
//! when plugins wait to be allowed: every plugin file found, to allow (loaded at once),
//! enable or disable (at the next start), and a Python one's file to open in an editor.

use egui::Ui;

use crate::app::{dialog, dialog_button_row};
use crate::i18n::tr;
use crate::render::effects::plugin::{PluginFile, Settings, Status, user_plugins_dir};
use crate::render::effects::registry;
use crate::ui::callout::{Tone, callout};

/// The open window.
pub struct Manager {
    files: Vec<PluginFile>,
    settings: Settings,
    /// Something changed: it applies at the next start.
    changed: bool,
}

/// What became of the window this frame.
pub enum Outcome {
    Open,
    Closed,
    /// Restart the app, to apply the changes.
    Restart,
    /// It stays open; this went wrong.
    Error(String),
}

impl Manager {
    pub fn new() -> Self {
        Self {
            files: registry().files(),
            settings: Settings::read(),
            changed: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let (mut close, mut restart) = (false, false);
        let mut error = None;
        let response = dialog(ctx, "plugins").show(ctx, |ui| {
            ui.set_min_width(520.0);
            ui.heading(tr!("PLUGINS_TITLE"));
            ui.add_space(4.0);
            if self.files.is_empty() {
                let folder = user_plugins_dir().map(|d| d.display().to_string());
                callout(
                    ui,
                    Tone::Info,
                    &tr!("NO_PLUGINS", folder = folder.unwrap_or_default()),
                );
            } else {
                if self.files.iter().any(|f| f.status == Status::Waiting) {
                    callout(ui, Tone::Warning, tr!("PLUGINS_WAITING"));
                    ui.add_space(4.0);
                }
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.6)
                    .show(ui, |ui| error = self.list(ui).err());
            }
            if self.changed {
                ui.add_space(4.0);
                callout(ui, Tone::Info, tr!("PLUGINS_RESTART"));
            }
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                restart = ui
                    .button(tr!("RESTART_APP"))
                    .on_hover_text(tr!("RESTART_APP_HINT"))
                    .clicked();
                close = ui.button(tr!("CLOSE")).clicked();
            });
        });
        if let Some(e) = error {
            return Outcome::Error(e);
        }
        if restart {
            return Outcome::Restart;
        }
        if close || response.should_close() {
            Outcome::Closed
        } else {
            Outcome::Open
        }
    }

    fn list(&mut self, ui: &mut Ui) -> Result<(), String> {
        let mut allow = None;
        let mut edit = None;
        egui::Grid::new("plugin_list")
            .num_columns(6)
            .spacing([12.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                for header in [
                    tr!("PLUGIN_ENABLED"),
                    tr!("PLUGIN_NAME"),
                    tr!("PLUGIN_VERSION"),
                    tr!("PLUGIN_KIND"),
                    tr!("PLUGIN_STATUS"),
                ] {
                    ui.strong(header);
                }
                // The last column takes the rest of the width: the rows go across the
                // window, Edit at their right end.
                ui.allocate_space(egui::vec2(ui.available_width(), 0.0));
                ui.end_row();
                for f in &self.files {
                    let mut enabled = !self.settings.is_disabled(&f.path);
                    if ui.checkbox(&mut enabled, "").changed() {
                        self.settings.set_disabled(&f.path, !enabled);
                        self.settings.save();
                        self.changed = true;
                    }
                    ui.label(f.name())
                        .on_hover_text(f.path.display().to_string());
                    // Not known until the plugin has run.
                    match &f.status {
                        Status::Loaded { version, .. } if !version.is_empty() => ui.label(version),
                        _ => ui.label("?"),
                    };
                    ui.label(if f.python {
                        tr!("PLUGIN_KIND_PYTHON")
                    } else {
                        tr!("PLUGIN_KIND_LIBRARY")
                    });
                    if f.status == Status::Waiting {
                        ui.horizontal(|ui| {
                            status(ui, &f.status);
                            if ui.button(tr!("ALLOW_PLUGIN")).clicked() {
                                allow = Some(f.path.clone());
                            }
                        });
                    } else {
                        status(ui, &f.status);
                    }
                    let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                    let right = egui::Layout::right_to_left(egui::Align::Center);
                    ui.allocate_ui_with_layout(row, right, |ui| {
                        if f.python
                            && ui
                                .button(tr!("EDIT_PLUGIN"))
                                .on_hover_text(tr!("EDIT_PLUGIN_HINT"))
                                .clicked()
                        {
                            edit = Some(f.path.clone());
                        }
                    });
                    ui.end_row();
                }
            });
        if let Some(path) = allow {
            registry().allow(&path, &mut self.settings);
            self.files = registry().files();
        }
        edit.map_or(Ok(()), |path| crate::ui::file_dialog::edit_file(&path))
    }
}

/// What became of a plugin at startup; why it failed in the tooltip.
fn status(ui: &mut Ui, status: &Status) {
    let text = match status {
        Status::Loaded { .. } => tr!("PLUGIN_STATUS_LOADED"),
        Status::Disabled => tr!("PLUGIN_STATUS_DISABLED"),
        Status::Waiting => tr!("PLUGIN_STATUS_WAITING"),
        Status::Failed(_) => tr!("PLUGIN_STATUS_FAILED"),
    };
    let r = ui.weak(text);
    if let Status::Failed(e) = status {
        r.on_hover_text(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// `PLUGINS_PREVIEW=out.png cargo test plugin_manager_preview -- --ignored` draws
    /// the window in both themes.
    #[test]
    #[ignore]
    fn plugin_manager_preview() {
        let Ok(path) = std::env::var("PLUGINS_PREVIEW") else {
            return;
        };
        let file = |name: &str, python: bool, status: Status| PluginFile {
            path: PathBuf::from("/home/me/.local/share/artwork-creator/plugins").join(name),
            python,
            status,
        };
        let files = vec![
            file(
                "libhalftone.so",
                false,
                Status::Loaded {
                    name: "Halftone".into(),
                    version: "0.1.0".into(),
                },
            ),
            file(
                "posterize.py",
                true,
                Status::Loaded {
                    name: "Python example".into(),
                    version: "1.0".into(),
                },
            ),
            file("new.py", true, Status::Waiting),
            file("old.py", true, Status::Disabled),
            file("broken.py", true, Status::Failed("no plugin".into())),
        ];
        crate::ui::preview::render(&path, egui::vec2(720.0, 420.0), |ui| {
            let mut m = Manager {
                files: files.clone(),
                settings: Settings::default(),
                changed: true,
            };
            m.settings.set_disabled(&files[3].path, true);
            m.show(ui.ctx());
        });
    }
}
