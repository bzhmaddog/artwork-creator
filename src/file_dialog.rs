//! File dialogs. Native dialogs (via `rfd`) are used where the system provides them.
//! On Linux systems without an xdg-desktop-portal or zenity, such as WSLg, a built-in
//! browser is shown instead.

use std::path::{Path, PathBuf};

use egui::{Key, RichText};

/// What the dialog was opened for; returned with the chosen paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    AddImages,
    OpenProject,
    SaveProject,
    Export,
}

/// What to ask the user for; used by both the native and the built-in dialog.
pub struct Request {
    /// Passed back with the result.
    pub purpose: Purpose,
    /// Dialog title.
    pub title: &'static str,
    /// Ask for a new file name instead of picking existing files.
    pub save: bool,
    /// Allow picking several files.
    pub multiple: bool,
    /// Label of the file type filter.
    pub filter_name: &'static str,
    /// Lower-case extensions without dots. Empty means any file.
    pub extensions: &'static [&'static str],
    /// Suggested file name when saving.
    pub default_name: String,
}

/// True when native dialogs can work. Can be forced with
/// `ARTWORK_FILE_DIALOG=builtin` or `ARTWORK_FILE_DIALOG=native`.
pub fn native_available() -> bool {
    match std::env::var("ARTWORK_FILE_DIALOG").as_deref() {
        Ok("builtin") => return false,
        Ok("native") => return true,
        _ => {}
    }
    if !cfg!(target_os = "linux") {
        return true;
    }
    // rfd talks to xdg-desktop-portal over D-Bus and falls back to zenity.
    let on_path = |bin: &str| {
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
    };
    let mut data_dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    if let Some(home) = std::env::var_os("HOME") {
        data_dirs.push(Path::new(&home).join(".local/share"));
    }
    let portal = data_dirs.iter().any(|d| {
        d.join("dbus-1/services/org.freedesktop.portal.Desktop.service")
            .is_file()
    });
    portal || on_path("zenity")
}

/// Shows a native dialog and returns the result immediately.
pub fn native(req: &Request) -> Vec<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(req.title);
    if !req.extensions.is_empty() {
        d = d.add_filter(req.filter_name, req.extensions);
    }
    if req.save {
        d.set_file_name(&req.default_name)
            .save_file()
            .into_iter()
            .collect()
    } else if req.multiple {
        d.pick_files().unwrap_or_default()
    } else {
        d.pick_file().into_iter().collect()
    }
}

/// A file or folder listed in the built-in browser.
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// In-app file browser.
pub struct FileBrowser {
    request: Option<Request>,
    dir: PathBuf,
    path_text: String,
    entries: Vec<Entry>,
    selected: Vec<PathBuf>,
    file_name: String,
    error: Option<String>,
}

impl Default for FileBrowser {
    fn default() -> Self {
        let dir = std::env::current_dir()
            .ok()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            request: None,
            path_text: dir.display().to_string(),
            dir,
            entries: Vec::new(),
            selected: Vec::new(),
            file_name: String::new(),
            error: None,
        }
    }
}

/// Shortcuts in the browser's side bar: home folders, the current folder and the Windows
/// drives under /mnt.
fn places() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        out.push(("Home".to_owned(), home.clone()));
        for sub in ["Desktop", "Documents", "Pictures", "Downloads"] {
            let p = home.join(sub);
            if p.is_dir() {
                out.push((sub.to_owned(), p));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(("Current folder".to_owned(), cwd));
    }
    // Windows drives inside WSL.
    if let Ok(read) = std::fs::read_dir("/mnt") {
        let mut drives: Vec<PathBuf> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.len() == 1) && p.is_dir())
            .collect();
        drives.sort();
        for d in drives {
            let letter = d
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_uppercase();
            out.push((format!("Windows {letter}:"), d));
        }
    }
    out
}

impl FileBrowser {
    /// The browser is waiting for the user.
    pub fn is_open(&self) -> bool {
        self.request.is_some()
    }

    /// Shows the browser for a request, in the last folder visited.
    pub fn open(&mut self, req: Request) {
        self.file_name = req.default_name.clone();
        self.selected.clear();
        self.request = Some(req);
        let dir = self.dir.clone();
        self.navigate(dir);
    }

    /// Lists a folder: subfolders first, then files matching the request's extensions.
    /// Hidden files are skipped.
    fn navigate(&mut self, dir: PathBuf) {
        let exts = self.request.as_ref().map(|r| r.extensions).unwrap_or(&[]);
        match std::fs::read_dir(&dir) {
            Ok(read) => {
                let mut entries: Vec<Entry> = read
                    .flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            return None;
                        }
                        let path = e.path();
                        let is_dir = path.is_dir();
                        let ext_ok = exts.is_empty()
                            || path.extension().is_some_and(|x| {
                                exts.contains(&x.to_string_lossy().to_lowercase().as_str())
                            });
                        (is_dir || ext_ok).then_some(Entry { name, path, is_dir })
                    })
                    .collect();
                entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                self.entries = entries;
                self.path_text = dir.display().to_string();
                self.dir = dir;
                self.selected.clear();
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Cannot open {}: {e}", dir.display())),
        }
    }

    /// Shows the browser if open, in its own OS window when the platform allows it.
    /// Returns the purpose and chosen paths when the user confirms.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<(Purpose, Vec<PathBuf>)> {
        let req = self.request.as_ref()?;
        let (title, purpose) = (req.title, req.purpose);
        let viewport = egui::ViewportId::from_hash_of("file_browser");
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([760.0, 500.0])
            .with_min_inner_size([520.0, 340.0]);
        let separate = !ctx.embed_viewports();

        let action = ctx.show_viewport_immediate(viewport, builder, |ui, class| {
            let closed = ui.input(|i| i.viewport().close_requested());
            let action = if class == egui::ViewportClass::EmbeddedWindow {
                self.contents(ui)
            } else {
                egui::CentralPanel::default()
                    .show(ui, |ui| self.contents(ui))
                    .inner
            };
            if closed { Action::Cancel } else { action }
        });

        // Like a native dialog, block the main window while the browser is open: an
        // invisible modal dims it and catches clicks, which bring the browser to the front.
        if separate {
            let blocker = egui::Modal::new(egui::Id::unique("file_browser_blocker"))
                .frame(egui::Frame::NONE)
                .show(ctx, |_| {});
            if blocker.backdrop_response.clicked() {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            }
        }

        match action {
            Action::None => None,
            Action::Cancel => {
                self.request = None;
                None
            }
            Action::Confirm(paths) => {
                self.request = None;
                Some((purpose, paths))
            }
        }
    }

    /// The browser's widgets.
    fn contents(&mut self, ui: &mut egui::Ui) -> Action {
        let Some(req) = self.request.as_ref() else {
            return Action::Cancel;
        };
        let (save, multiple) = (req.save, req.multiple);
        let filter = if req.extensions.is_empty() {
            "All files".to_owned()
        } else {
            format!("{} (*.{})", req.filter_name, req.extensions.join(", *."))
        };
        let mut go_to: Option<PathBuf> = None;
        let mut confirm = false;
        let mut cancel = ui.input(|i| i.key_pressed(Key::Escape));

        ui.horizontal(|ui| {
            if ui.button("Up").clicked()
                && let Some(parent) = self.dir.parent()
            {
                go_to = Some(parent.to_path_buf());
            }
            let r = ui
                .add(egui::TextEdit::singleline(&mut self.path_text).desired_width(f32::INFINITY));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                go_to = Some(PathBuf::from(self.path_text.trim()));
            }
        });
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        ui.separator();

        // Buttons pinned to the bottom; the file list takes the rest.
        egui::Panel::bottom("file_browser_buttons").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if save {
                    ui.label("File name");
                    let r = ui
                        .add(egui::TextEdit::singleline(&mut self.file_name).desired_width(260.0));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        confirm = !self.file_name.trim().is_empty();
                    }
                } else if multiple {
                    ui.weak("Ctrl+click to select several files");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok_label = if save { "Save" } else { "Open" };
                    let can = if save {
                        !self.file_name.trim().is_empty()
                    } else {
                        !self.selected.is_empty()
                    };
                    if ui.add_enabled(can, egui::Button::new(ok_label)).clicked() {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
            ui.weak(filter);
        });

        let list_height = ui.available_height().max(80.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(140.0);
                for (label, path) in places() {
                    if ui.selectable_label(self.dir == path, label).clicked() {
                        go_to = Some(path);
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("files")
                .max_height(list_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_height(list_height);
                    // One entry per row, full width.
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                        if self.entries.is_empty() {
                            ui.weak("No matching files here.");
                        }
                        for e in &self.entries {
                            let is_sel = self.selected.contains(&e.path);
                            let text = if e.is_dir {
                                RichText::new(format!("{}/", e.name)).strong()
                            } else {
                                RichText::new(&e.name)
                            };
                            let r = ui.selectable_label(is_sel, text);
                            if r.double_clicked() {
                                if e.is_dir {
                                    go_to = Some(e.path.clone());
                                } else {
                                    self.selected = vec![e.path.clone()];
                                    self.file_name = e.name.clone();
                                    confirm = true;
                                }
                            } else if r.clicked() && !e.is_dir {
                                let add = multiple
                                    && ui.input(|i| i.modifiers.command || i.modifiers.shift);
                                if add {
                                    if is_sel {
                                        self.selected.retain(|p| p != &e.path);
                                    } else {
                                        self.selected.push(e.path.clone());
                                    }
                                } else {
                                    self.selected = vec![e.path.clone()];
                                }
                                self.file_name = e.name.clone();
                            }
                        }
                    });
                });
        });

        if let Some(dir) = go_to {
            self.navigate(dir);
        }
        if cancel {
            Action::Cancel
        } else if confirm {
            Action::Confirm(if save {
                vec![self.dir.join(self.file_name.trim())]
            } else {
                std::mem::take(&mut self.selected)
            })
        } else {
            Action::None
        }
    }
}

/// What the user did in the browser this frame.
enum Action {
    None,
    Cancel,
    Confirm(Vec<PathBuf>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_lists_dirs_first_and_filters_extensions() {
        let dir = std::env::temp_dir().join(format!("artwork-fd-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["b.png", "a.JPG", "notes.txt", ".hidden.png"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut fb = FileBrowser {
            dir: dir.clone(),
            ..Default::default()
        };
        fb.open(Request {
            purpose: Purpose::AddImages,
            title: "t",
            save: false,
            multiple: true,
            filter_name: "Images",
            extensions: &["png", "jpg"],
            default_name: String::new(),
        });
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "a.JPG", "b.png"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
