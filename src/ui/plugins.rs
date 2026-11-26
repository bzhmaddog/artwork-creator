//! The plugin manager (Tools > Plugins > Manage plugins...), also opened at the start
//! when plugins wait to be allowed. Its Installed tab lists every plugin file found, to
//! allow (loaded at once), enable or disable (at the next start), update or uninstall. Allowing a Python plugin first installs the packages it needs,
//! with the app's own pip. Its Available tab lists the plugins of the repositories (the
//! official one until removed, and those the user added), to install: downloaded into the user's plugins
//! folder, allowed and enabled, and loaded at once.

use std::path::{Path, PathBuf};

use egui::Ui;

use std::thread::JoinHandle;

use crate::app::{danger_button, dialog, dialog_button_row};
use crate::i18n::{tr, tr_n};
use crate::render::effects::plugin::{self, PluginFile, Settings, Status, user_plugins_dir};
use crate::render::effects::repository::{
    self, Catalog, Download, Entry, Kind, Known, Repository, Version,
};
use crate::render::effects::{PluginRef, python, registry};
use crate::ui::callout::{Tone, callout};

/// Width of the window.
const WINDOW_WIDTH: f32 = 860.0;
/// Height of the tabs' contents, as a share of the app's window, within these bounds:
/// the same for both tabs, the lists scrolling inside.
const BODY_SHARE: f32 = 0.65;
const BODY_HEIGHT: std::ops::RangeInclusive<f32> = 320.0..=720.0;
/// Width of the field taking a repository's address.
const URL_WIDTH: f32 = 420.0;
/// Width of the Available tab's drop-down of repositories.
const SOURCE_WIDTH: f32 = 260.0;
/// Width of the Available tab's drop-downs of versions.
const VERSION_WIDTH: f32 = 130.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tab {
    Installed,
    Available,
    Repositories,
}

/// A repository of the Available tab.
struct Repo {
    url: String,
    list: Fetch,
}

/// A plugin file being downloaded and saved, in the background.
/// A plugin to install: its name, file, kind, repository name and address, what to
/// record of it, and the file of another version of it that it replaces, if any.
type InstallRequest = (
    String,
    Download,
    Kind,
    String,
    String,
    plugin::Installed,
    Option<PathBuf>,
);

struct Job {
    /// The plugin's name in its repository.
    name: String,
    /// The name of the folder it goes in: its origin, for one from a repository.
    folder: String,
    /// For an update: the file it replaces, and the version it brings.
    update: Option<(PathBuf, String)>,
    handle: JoinHandle<Result<PathBuf, String>>,
}

/// A repository's list: being downloaded, read, or why it could not be.
enum Fetch {
    Loading(JoinHandle<Result<Repository, String>>),
    Ready(Repository),
    Failed(String),
}

impl Fetch {
    fn start(url: &str) -> Self {
        let url = url.to_owned();
        Fetch::Loading(std::thread::spawn(move || repository::fetch(&url)))
    }

    /// Takes the result of a finished download.
    fn update(&mut self) {
        if let Fetch::Loading(h) = self
            && h.is_finished()
        {
            let Fetch::Loading(h) = std::mem::replace(self, Fetch::Failed(String::new())) else {
                unreachable!()
            };
            *self = match h.join().unwrap_or_else(|_| Err("stopped".into())) {
                Ok(r) => Fetch::Ready(r),
                Err(e) => Fetch::Failed(e),
            };
        }
    }
}

/// The open window.
pub struct Manager {
    files: Vec<PluginFile>,
    settings: Settings,
    /// Something changed: it applies at the next start.
    changed: bool,
    /// pip came with the app's Python: what plugins need can be installed.
    pip: bool,
    /// pip installing what this plugin needs, before it is allowed.
    install: Option<(PathBuf, std::thread::JoinHandle<Result<(), String>>)>,
    /// Why installing what this plugin needs failed.
    install_error: Option<(String, String)>,
    tab: Tab,
    /// The repositories, once the Available tab was opened.
    repos: Vec<Repo>,
    /// The address typed in, of a repository to add.
    new_repo: String,
    /// The list of the repository being added, being downloaded.
    adding: Option<(String, JoinHandle<Result<Repository, String>>)>,
    /// Why the repository typed in could not be added.
    add_error: Option<String>,
    /// The address of the repository whose plugins are listed, if picked.
    source: String,
    /// What the plugins listed must contain.
    filter: String,
    /// The version picked to install, by repository address and plugin id; the latest
    /// that works here if none is.
    versions: std::collections::BTreeMap<(String, String), String>,
    /// A plugin being installed or updated.
    download: Option<Job>,
    /// Why installing this plugin failed.
    download_error: Option<(String, String)>,
    /// The plugin files installed while the window is open.
    installed: Vec<PathBuf>,
    /// The SHA-256 of each plugin file.
    hashes: Vec<(PathBuf, String)>,
    /// The repository (name and address) each plugin file was installed from, if any.
    sources: std::collections::BTreeMap<PathBuf, (String, String)>,
    /// What the repositories say about the plugin files they list.
    known: std::collections::BTreeMap<PathBuf, Known>,
    /// Plugins updated while the window is open, with the version they got.
    updated: Vec<(PathBuf, String)>,
    /// Plugin files uninstalled while the window is open: no longer listed.
    uninstalled: Vec<PathBuf>,
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
    /// The window, with the repositories' lists of `catalog` if given (already downloaded
    /// at startup), else downloading them.
    pub fn new(catalog: Option<Catalog>) -> Self {
        let mut m = Self::with(registry().files(), Settings::read());
        m.pip = python::pip().is_some();
        m.repos = match catalog {
            Some(c) => c
                .into_iter()
                .map(|(url, list)| Repo {
                    url,
                    list: list.map_or_else(Fetch::Failed, Fetch::Ready),
                })
                .collect(),
            None => (m.settings.repositories().into_iter())
                .map(|url| Repo {
                    list: Fetch::start(&url),
                    url,
                })
                .collect(),
        };
        m.refresh_known();
        m
    }

    /// The window with these plugin files and settings, no repository.
    fn with(files: Vec<PluginFile>, settings: Settings) -> Self {
        let hashes = repository::hashes(files.iter().map(|f| f.path.as_path()));
        let sources = sources(&files);
        Self {
            files,
            settings,
            changed: false,
            pip: false,
            install: None,
            install_error: None,
            tab: Tab::Installed,
            repos: Vec::new(),
            new_repo: String::new(),
            adding: None,
            add_error: None,
            source: String::new(),
            filter: String::new(),
            versions: Default::default(),
            download: None,
            download_error: None,
            installed: Vec::new(),
            sources,
            hashes,
            known: Default::default(),
            updated: Vec::new(),
            uninstalled: Vec::new(),
        }
    }

    /// The repositories' lists given to a window already open (by the startup check).
    pub fn set_catalog(&mut self, catalog: Catalog) {
        for (url, list) in catalog {
            if let Some(r) = self.repos.iter_mut().find(|r| r.url == url)
                && !matches!(r.list, Fetch::Ready(_))
            {
                r.list = list.map_or_else(Fetch::Failed, Fetch::Ready);
            }
        }
        self.refresh_known();
    }

    /// What the repositories read say about the plugin files.
    fn refresh_known(&mut self) {
        let lists = self.repos.iter().filter_map(|r| match &r.list {
            Fetch::Ready(list) => Some((r.url.as_str(), list)),
            _ => None,
        });
        plugin::Source::name_folders(lists.clone().map(|(url, l)| (url, l.name.as_str())));
        self.known = repository::known(&self.hashes, lists);
    }

    /// The plugin files again, after one was allowed or added.
    fn refresh_files(&mut self) {
        self.files = registry().files();
        self.hashes = repository::hashes(self.files.iter().map(|f| f.path.as_path()));
        self.sources = sources(&self.files);
        self.refresh_known();
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let (mut close, mut restart) = (false, false);
        let mut error = None;
        let response = dialog(ctx, "plugins").show(ctx, |ui| {
            ui.set_width(WINDOW_WIDTH);
            ui.horizontal(|ui| {
                ui.heading(tr!("PLUGINS_TITLE"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = crate::ui::panels::cross_button(ui, tr!("CLOSE")).clicked();
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let installed = tr!("PLUGINS_INSTALLED_TAB");
                ui.selectable_value(&mut self.tab, Tab::Installed, installed);
                let available = tr!("PLUGINS_AVAILABLE_TAB");
                let available = ui.selectable_value(&mut self.tab, Tab::Available, available);
                let repositories = tr!("PLUGIN_REPOSITORIES");
                let repositories =
                    ui.selectable_value(&mut self.tab, Tab::Repositories, repositories);
                let _ = (available, repositories);
            });
            self.update_downloads();
            let busy = self.adding.is_some() || self.download.is_some();
            if busy
                || self
                    .repos
                    .iter()
                    .any(|r| matches!(r.list, Fetch::Loading(_)))
            {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
            ui.separator();
            let (min, max) = BODY_HEIGHT.into_inner();
            let body = (ctx.content_rect().height() * BODY_SHARE).clamp(min, max);
            ui.allocate_ui(egui::vec2(WINDOW_WIDTH, body), |ui| {
                ui.set_min_size(egui::vec2(WINDOW_WIDTH, body));
                if self.tab == Tab::Available {
                    self.available(ui);
                } else if self.tab == Tab::Repositories {
                    self.repositories(ui);
                } else if self.files.is_empty() {
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
                        .max_height(ui.available_height())
                        .auto_shrink([false, true])
                        .show(ui, |ui| error = self.list(ui).err());
                }
            });
            if let Some((plugin, e)) = &self.install_error {
                ui.add_space(4.0);
                callout(
                    ui,
                    Tone::Warning,
                    &tr!("PLUGIN_PACKAGES_FAILED", plugin = plugin, error = e),
                );
            }
            if let Some((plugin, e)) = &self.download_error {
                ui.add_space(4.0);
                callout(
                    ui,
                    Tone::Warning,
                    &tr!("PLUGIN_INSTALL_FAILED", plugin = plugin, error = e),
                );
            }
            if self.changed {
                ui.add_space(4.0);
                callout(ui, Tone::Info, tr!("PLUGINS_RESTART"));
                ui.add_space(8.0);
                dialog_button_row(ui, |ui| {
                    restart = ui
                        .button(tr!("RESTART_APP"))
                        .on_hover_text(tr!("RESTART_APP_HINT"))
                        .clicked();
                });
            }
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

    /// A finished install: the plugin allowed, or why it failed.
    fn installed(&mut self) {
        if !self.install.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            return;
        }
        let (path, handle) = self.install.take().unwrap();
        match handle.join().unwrap_or_else(|_| Err("pip stopped".into())) {
            Ok(()) => self.allow(&path),
            Err(e) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.install_error = Some((name.into_owned(), e));
            }
        }
    }

    fn allow(&mut self, path: &Path) {
        registry().allow(path, &mut self.settings);
        self.refresh_files();
    }

    /// Takes the results of the downloads that finished.
    fn update_downloads(&mut self) {
        let loading = |m: &Self| {
            let loading = m
                .repos
                .iter()
                .filter(|r| matches!(r.list, Fetch::Loading(_)));
            loading.count()
        };
        let before = loading(self);
        for r in &mut self.repos {
            r.list.update();
        }
        if loading(self) < before {
            self.refresh_known();
        }
        if self.adding.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            let (url, handle) = self.adding.take().unwrap();
            match handle.join().unwrap_or_else(|_| Err("stopped".into())) {
                Ok(list) => {
                    self.settings.add_repository(&url);
                    self.settings.save();
                    self.repos.push(Repo {
                        url,
                        list: Fetch::Ready(list),
                    });
                    self.new_repo.clear();
                    self.refresh_known();
                }
                Err(e) => {
                    self.add_error = Some(tr!("REPOSITORY_INVALID", url = url, error = e));
                }
            }
        }
        if self
            .download
            .as_ref()
            .is_some_and(|j| j.handle.is_finished())
        {
            let job = self.download.take().unwrap();
            match job.handle.join().unwrap_or_else(|_| Err("stopped".into())) {
                Ok(path) => match job.update {
                    Some((old, version)) => self.updated_file(&old, &path, version),
                    None => self.installed_file(&path),
                },
                Err(e) => self.download_error = Some((job.name, e)),
            }
        }
    }

    /// The plugin just saved at `path`: allowed, as the user chose to install it, enabled,
    /// and loaded at once if it can be.
    fn installed_file(&mut self, path: &Path) {
        if !enable_installed(&mut self.settings, path) {
            self.changed = true;
        }
        self.uninstalled.retain(|p| p != path);
        self.refresh_files();
        self.installed.push(path.to_owned());
    }

    /// The plugin file `old` replaced with `path`, of `version`: allowed, as the user
    /// chose to update it. The old one stays loaded until the next start.
    fn updated_file(&mut self, old: &Path, path: &Path, version: String) {
        match plugin::pending(path) {
            Ok(p) => self.settings.allow(&p),
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
        if self.settings.is_disabled(old) {
            self.settings.set_disabled(path, true);
        }
        self.settings.save();
        self.changed = true;
        self.updated.push((old.to_owned(), version));
        if let Ok(p) = plugin::pending(path)
            && let Some(h) = self.hashes.iter_mut().find(|(f, _)| f == old)
        {
            *h = (old.to_owned(), p.hash);
        }
        self.refresh_known();
    }

    /// Removes the plugin file `path` (set aside, as a library in use can't be deleted on
    /// Windows) and what the settings say of it, and unloads it.
    fn uninstall(&mut self, path: &Path) -> Result<(), String> {
        repository::set_aside(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.settings.forget(path);
        self.settings.save();
        registry().remove(path);
        self.installed.retain(|p| p != path);
        self.uninstalled.push(path.to_owned());
        Ok(())
    }

    /// Adds the repository typed in, once its list is read.
    fn add_repository(&mut self) {
        let url = self.new_repo.trim().to_owned();
        self.add_error = None;
        if url.is_empty() {
            return;
        }
        if self.repos.iter().any(|r| r.url == url) {
            self.add_error = Some(tr!("REPOSITORY_ALREADY_ADDED").to_owned());
            return;
        }
        let fetch = url.clone();
        self.adding = Some((url, std::thread::spawn(move || repository::fetch(&fetch))));
    }

    /// The Repositories tab: the repositories, to add or remove.
    fn repositories(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let field = ui.add_enabled(
                self.adding.is_none(),
                egui::TextEdit::singleline(&mut self.new_repo)
                    .hint_text(tr!("REPOSITORY_URL_HINT"))
                    .desired_width(URL_WIDTH),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if self.adding.is_some() {
                ui.spinner();
            } else if ui.button(tr!("ADD_REPOSITORY")).clicked() || enter {
                self.add_repository();
            }
        });
        if let Some(e) = &self.add_error {
            callout(ui, Tone::Warning, e);
        }
        ui.add_space(8.0);
        if self.repos.is_empty() {
            callout(ui, Tone::Info, tr!("NO_REPOSITORIES"));
            return;
        }
        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height())
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("plugin_repositories")
                    .num_columns(4)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for header in [
                            tr!("PLUGIN_NAME"),
                            tr!("REPOSITORY_ADDRESS"),
                            tr!("REPOSITORY_PLUGINS"),
                        ] {
                            ui.strong(header);
                        }
                        // The last column takes the rest of the width, Remove at its right.
                        ui.allocate_space(egui::vec2(ui.available_width(), 0.0));
                        ui.end_row();
                        for (i, r) in self.repos.iter().enumerate() {
                            match &r.list {
                                Fetch::Ready(list) => ui.label(&list.name),
                                _ => ui.label("?"),
                            };
                            let size = egui::vec2(URL_WIDTH, ui.spacing().interact_size.y);
                            let left = egui::Layout::left_to_right(egui::Align::Center);
                            ui.allocate_ui_with_layout(size, left, |ui| {
                                let url = egui::RichText::new(&r.url).weak();
                                ui.add(egui::Label::new(url).truncate())
                                    .on_hover_text(&r.url);
                            });
                            match &r.list {
                                Fetch::Loading(_) => {
                                    ui.spinner();
                                }
                                Fetch::Ready(list) => {
                                    ui.weak(tr_n!("REPOSITORY_PLUGIN_COUNT", list.plugins.len()));
                                }
                                Fetch::Failed(e) => {
                                    ui.weak(tr!("REPOSITORY_STATUS_FAILED")).on_hover_text(e);
                                }
                            }
                            let row =
                                egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                            let right = egui::Layout::right_to_left(egui::Align::Center);
                            ui.allocate_ui_with_layout(row, right, |ui| {
                                if ui.add(danger_button(tr!("REMOVE_REPOSITORY"))).clicked() {
                                    remove = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(i) = remove {
            // Its plugins stay installed.
            let r = self.repos.remove(i);
            self.settings.remove_repository(&r.url);
            self.settings.save();
            self.refresh_known();
        }
    }

    /// The repository whose plugins the Available tab lists: the one picked, else the
    /// official one, else the first.
    fn source(&self) -> Option<&Repo> {
        let find = |url: &str| self.repos.iter().find(|r| r.url == url);
        find(&self.source)
            .or_else(|| find(repository::OFFICIAL))
            .or(self.repos.first())
    }

    /// The Available tab: the plugins of the repository picked.
    fn available(&mut self, ui: &mut Ui) {
        callout(ui, Tone::Warning, tr!("PLUGINS_TRUST"));
        ui.add_space(8.0);
        let Some(current) = self.source().map(|r| r.url.clone()) else {
            callout(ui, Tone::Info, tr!("NO_REPOSITORIES"));
            return;
        };
        let name = |r: &Repo| match &r.list {
            Fetch::Ready(list) => list.name.clone(),
            _ => r.url.clone(),
        };
        ui.horizontal(|ui| {
            let selected = self.repos.iter().find(|r| r.url == current).map(name);
            egui::ComboBox::from_id_salt("plugin_source")
                .width(SOURCE_WIDTH)
                .selected_text(selected.unwrap_or_default())
                .show_ui(ui, |ui| {
                    for r in &self.repos {
                        let on = r.url == current;
                        if ui
                            .selectable_label(on, name(r))
                            .on_hover_text(&r.url)
                            .clicked()
                        {
                            self.source = r.url.clone();
                        }
                    }
                })
                .response
                .on_hover_text(&current);
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text(tr!("FILTER_PLUGINS_HINT"))
                    .desired_width(f32::INFINITY),
            );
        });
        ui.add_space(4.0);
        let Some(r) = self.source() else { return };
        let list = match &r.list {
            Fetch::Loading(_) => {
                ui.spinner();
                return;
            }
            Fetch::Failed(e) => {
                let text = tr!("REPOSITORY_INVALID", url = r.url, error = e);
                callout(ui, Tone::Warning, &text);
                return;
            }
            Fetch::Ready(list) => list,
        };
        let filter = self.filter.trim().to_lowercase();
        let (repo, url, origin) = (list.name.as_str(), r.url.as_str(), plugin::origin(&r.url));
        let shown: Vec<&Entry> = list
            .plugins
            .iter()
            .filter(|p| {
                let provides = self
                    .picked(url, p, &p.compatible())
                    .into_iter()
                    .flat_map(|(v, _)| v.effects.iter().chain(&v.generators).map(String::as_str));
                [&p.id, &p.name, &p.description, &p.author]
                    .into_iter()
                    .map(String::as_str)
                    .chain(provides)
                    .any(|t| t.to_lowercase().contains(&filter))
            })
            .collect();
        if shown.is_empty() {
            callout(ui, Tone::Info, tr!("NO_AVAILABLE_PLUGINS"));
            return;
        }
        let mut install = None;
        let mut pick = None;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height())
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let right = ui.max_rect().right();
                egui::Grid::new("available_plugins")
                    .num_columns(5)
                    .spacing([12.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for header in [
                            tr!("PLUGIN_NAME"),
                            tr!("PLUGIN_VERSION"),
                            tr!("PLUGIN_KIND"),
                            tr!("PLUGIN_PROVIDES"),
                        ] {
                            ui.strong(header);
                        }
                        // The description takes the rest of the width, Install at its right.
                        description_header(ui, right);
                        ui.end_row();
                        for p in &shown {
                            let compatible = p.compatible();
                            let best = self.picked(url, p, &compatible);
                            let name = ui.label(&p.name);
                            if !p.author.is_empty() {
                                name.on_hover_text(&p.author);
                            }
                            let picked = best.map(|(v, _)| v);
                            version_choice(ui, url, p, &compatible, picked, &mut pick);
                            ui.label(match p.kind {
                                Kind::Python => tr!("PLUGIN_KIND_PYTHON"),
                                Kind::Library => tr!("PLUGIN_KIND_LIBRARY"),
                            });
                            match best {
                                Some((v, _)) => provides(ui, &v.effects, &v.generators),
                                None => provides(ui, &[], &[]),
                            }
                            let row = egui::vec2(
                                right - ui.cursor().left(),
                                ui.spacing().interact_size.y,
                            );
                            let right = egui::Layout::right_to_left(egui::Align::Center);
                            ui.allocate_ui_with_layout(row, right, |ui| {
                                let source = (repo, url, origin.as_str());
                                self.install_cell(ui, p, source, best, &mut install);
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let text = egui::RichText::new(&p.description).weak();
                                        let r = ui.add(egui::Label::new(text).truncate());
                                        r.on_hover_text(&p.description);
                                    },
                                );
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some((key, version)) = pick {
            self.versions.insert(key, version);
        }
        if let Some((name, d, kind, repo, url, entry, old)) = install {
            match old {
                // Another version of it is installed: replaced, as an update would.
                Some(old) => {
                    let dir = old.parent().unwrap_or(Path::new(".")).to_owned();
                    let version = entry.entry.version.clone();
                    self.start_download(name, d, kind, dir, entry, Some((old, version)));
                }
                None => self.install(name, d, kind, (&repo, &url), entry),
            }
        }
    }

    /// The version of the plugin `p` of the repository at `url` to install, and its file
    /// for this system: the one picked if it still works here, else the latest that does.
    /// `compatible` is [`Entry::compatible`] of `p`.
    fn picked<'a>(
        &self,
        url: &str,
        p: &Entry,
        compatible: &[(&'a Version, &'a Download)],
    ) -> Option<(&'a Version, &'a Download)> {
        let key = (url.to_owned(), p.id.clone());
        let picked = self.versions.get(&key);
        (picked.and_then(|want| compatible.iter().find(|(v, _)| v.version == *want)))
            .or(compatible.first())
            .copied()
    }

    /// The Install button of plugin `p` at the version `picked` (with its file for this
    /// system), or what stands in its place; `source` is its repository's name, address
    /// and origin.
    fn install_cell(
        &self,
        ui: &mut Ui,
        p: &Entry,
        (repo, url, origin): (&str, &str, &str),
        picked: Option<(&Version, &Download)>,
        install: &mut Option<InstallRequest>,
    ) {
        let Some((v, d)) = picked else {
            ui.weak(tr!("PLUGIN_UNAVAILABLE"))
                .on_hover_text(tr!("PLUGIN_UNAVAILABLE_HINT"));
            return;
        };
        if (self.download.as_ref()).is_some_and(|j| j.name == p.name && j.folder == origin) {
            ui.spinner();
        } else {
            let old = self.installed_of(origin, p, d.file_name());
            if old
                .as_ref()
                .is_some_and(|(_, at)| at.as_ref().is_none_or(|at| *at == v.version))
            {
                ui.weak(tr!("PLUGIN_INSTALLED"));
                return;
            }
            let busy = self.download.is_some() || self.install.is_some();
            let hint = match &old {
                Some(_) => tr!("UPDATE_PLUGIN_HINT", version = &v.version),
                None => tr!("INSTALL_PLUGIN_HINT").to_owned(),
            };
            let button = ui
                .add_enabled(!busy, egui::Button::new(tr!("INSTALL_PLUGIN")))
                .on_hover_text(hint);
            if button.clicked() {
                let source = (repo.to_owned(), url.to_owned());
                let entry = installed_entry(p, v);
                let old = old.map(|(path, _)| path);
                let (name, d) = (p.name.clone(), d.clone());
                *install = Some((name, d, p.kind, source.0, source.1, entry, old));
            }
        }
    }

    /// The file of the plugin `p` installed from `origin` (named `name`, or recorded as
    /// `p`), with its version if known, in any plugins folder.
    fn installed_of(
        &self,
        origin: &str,
        p: &Entry,
        name: &str,
    ) -> Option<(PathBuf, Option<String>)> {
        let files = (self.files.iter())
            .map(|f| &f.path)
            .filter(|path| !self.uninstalled.contains(path));
        let mut paths = self.installed.iter().chain(files);
        paths.find_map(|path| {
            if plugin::origin_of(path) != Some(origin) {
                return None;
            }
            // What the repositories say of it, else what its install recorded (only
            // read for a file they don't list).
            let recorded = match self.known.get(path) {
                Some(k) => Some((k.id.clone(), k.version.clone())),
                None => plugin::recorded(path).map(|r| (r.entry.id, r.entry.version)),
            };
            let named = path.file_name().is_some_and(|n| n == name);
            if !named && recorded.as_ref().is_none_or(|(id, _)| *id != p.id) {
                return None;
            }
            let updated = self.updated.iter().find(|(old, _)| old == path);
            let version = updated.map(|(_, v)| v.clone()).or(recorded.map(|(_, v)| v));
            Some((path.clone(), version))
        })
    }

    /// Installs the plugin `name`, `entry` of the repository `repo` at `url`, from `d`
    /// into its origin's folder in the user's plugins folder, in the background.
    fn install(
        &mut self,
        name: String,
        d: Download,
        kind: Kind,
        (repo, url): (&str, &str),
        entry: plugin::Installed,
    ) {
        match origin_dir(repo, url) {
            Ok(dir) => self.start_download(name, d, kind, dir, entry, None),
            Err(e) => self.download_error = Some((name, e)),
        }
    }

    /// Updates the plugin file `old` to `u`: its new file goes in the same folder, the
    /// old one set aside if its name differs.
    fn update(&mut self, name: String, old: PathBuf, u: repository::Update) {
        let dir = old.parent().unwrap_or(Path::new(".")).to_owned();
        let entry = plugin::Installed {
            entry: PluginRef {
                id: u.id,
                version: u.version.clone(),
            },
            compatible_from: u.compatible_from,
            min_sdk: u.min_sdk,
        };
        let update = Some((old, u.version));
        self.start_download(name, u.download, u.kind, dir, entry, update);
    }

    /// Downloads `d` into `dir`, with the packages a Python plugin needs if the app's
    /// pip can install them (if not, the file is not kept).
    fn start_download(
        &mut self,
        name: String,
        d: Download,
        kind: Kind,
        dir: PathBuf,
        entry: plugin::Installed,
        update: Option<(PathBuf, String)>,
    ) {
        self.download_error = None;
        let pip = self.pip;
        let old = update.as_ref().map(|(old, _)| old.clone());
        let folder = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let handle = std::thread::spawn(move || fetch_plugin(&d, kind, &dir, pip, entry, old));
        self.download = Some(Job {
            name,
            folder,
            update,
            handle,
        });
    }

    fn list(&mut self, ui: &mut Ui) -> Result<(), String> {
        self.installed();
        let busy = self.install.is_some();
        if busy {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut allow = None;
        let mut update_to = None;
        let mut uninstall = None;
        // Only the user's plugins can be uninstalled, not those that come with the app.
        let user_dir = user_plugins_dir().and_then(|d| d.canonicalize().ok());
        // Where the rows end: the last column goes up to it.
        let right = ui.max_rect().right();
        egui::Grid::new("plugin_list")
            .num_columns(8)
            .spacing([12.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                for header in [
                    tr!("PLUGIN_ENABLED"),
                    tr!("PLUGIN_NAME"),
                    tr!("PLUGIN_VERSION"),
                    tr!("PLUGIN_KIND"),
                    tr!("PLUGIN_PROVIDES"),
                    tr!("PLUGIN_STATUS"),
                ] {
                    ui.strong(header);
                }
                // The description takes the rest of the width: the rows go across the
                // window, Update and Edit at their right end.
                description_header(ui, right);
                ui.end_row();
                // By repository, in the order of their names; those from none last.
                let mut files: Vec<_> = (self.files.iter())
                    .filter(|f| !self.uninstalled.contains(&f.path))
                    .map(|f| (self.sources.get(&f.path), f))
                    .collect();
                files.sort_by_key(|(s, _)| (s.is_none(), s.map(|(name, _)| name.to_lowercase())));
                let mut group = None;
                for (source, f) in files {
                    if group != Some(source) {
                        group = Some(source);
                        group_header(ui, source);
                    }
                    let mut enabled = !self.settings.is_disabled(&f.path);
                    if ui.checkbox(&mut enabled, "").changed() {
                        self.settings.set_disabled(&f.path, !enabled);
                        self.settings.save();
                        self.changed = true;
                    }
                    ui.label(f.name())
                        .on_hover_text(f.path.display().to_string());
                    let known = self.known.get(&f.path);
                    let updated = self.updated.iter().find(|(p, _)| *p == f.path);
                    ui.horizontal(|ui| {
                        // Not known until the plugin has run, unless a repository lists it.
                        let loaded = match &f.status {
                            Status::Loaded { version, .. } => Some(version.as_str()),
                            _ => None,
                        };
                        let recorded = known.map(|k| k.version.as_str());
                        ui.label(shown_version(loaded, recorded));
                        if let Some((_, version)) = updated {
                            updated_mark(ui)
                                .on_hover_text(tr!("PLUGIN_UPDATED", version = version));
                        } else if let Some(u) = known.and_then(|k| k.update.as_ref()) {
                            if self.download.as_ref().is_some_and(|j| {
                                j.update.as_ref().is_some_and(|(p, _)| *p == f.path)
                            }) {
                                ui.spinner();
                            } else if new_version(ui, u, !busy && self.download.is_none()).clicked()
                            {
                                update_to = Some((f.name(), f.path.clone(), u.clone()));
                            }
                        }
                    });
                    ui.label(if f.python {
                        tr!("PLUGIN_KIND_PYTHON")
                    } else {
                        tr!("PLUGIN_KIND_LIBRARY")
                    });
                    match known {
                        Some(k) => provides(ui, &k.effects, &k.generators),
                        None => provides(ui, &[], &[]),
                    }
                    if f.status == Status::Waiting {
                        let needs = needs(&f.path);
                        ui.horizontal(|ui| {
                            status(ui, &f.status);
                            if self.install.as_ref().is_some_and(|(p, _)| *p == f.path) {
                                ui.spinner();
                                return;
                            }
                            let button =
                                ui.add_enabled(!busy, egui::Button::new(tr!("ALLOW_PLUGIN")));
                            let button = match (needs.is_empty(), self.pip) {
                                (true, _) => button,
                                (false, true) => button.on_hover_text(tr!(
                                    "ALLOW_PLUGIN_INSTALLS_HINT",
                                    packages = needs.join(", ")
                                )),
                                (false, false) => button.on_hover_text(tr!(
                                    "PLUGIN_NEEDS_PACKAGES_HINT",
                                    packages = needs.join(", ")
                                )),
                            };
                            if button.clicked() {
                                allow = Some((f.path.clone(), needs));
                            }
                        });
                    } else {
                        status(ui, &f.status);
                    }
                    let row = egui::vec2(right - ui.cursor().left(), ui.spacing().interact_size.y);
                    let right = egui::Layout::right_to_left(egui::Align::Center);
                    ui.allocate_ui_with_layout(row, right, |ui| {
                        if user_dir.as_ref().is_some_and(|d| f.path.starts_with(d))
                            && ui
                                .add(danger_button(tr!("UNINSTALL_PLUGIN")))
                                .on_hover_text(tr!("UNINSTALL_PLUGIN_HINT"))
                                .clicked()
                        {
                            uninstall = Some(f.path.clone());
                        }
                        let description = known.map_or("", |k| k.description.as_str());
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let text = egui::RichText::new(description).weak();
                            let r = ui.add(egui::Label::new(text).truncate());
                            if !description.is_empty() {
                                r.on_hover_text(description);
                            }
                        });
                    });
                    ui.end_row();
                }
            });
        if let Some((path, needs)) = allow {
            self.install_error = None;
            if needs.is_empty() || !self.pip {
                self.allow(&path);
            } else {
                let handle = std::thread::spawn(move || python::install_dependencies(&needs));
                self.install = Some((path, handle));
            }
        }
        if let Some((name, path, u)) = update_to {
            self.update(name, path, u);
        }
        if let Some(path) = uninstall {
            self.uninstall(&path)?;
        }
        Ok(())
    }
}

/// Looks for new versions of the installed plugins, in the background, at startup:
/// the repositories' lists, and how many plugins have a new version. Nothing without
/// plugins or repositories; lists that can't be read are left out.
pub fn check_updates() -> Option<JoinHandle<(Catalog, usize)>> {
    updates_for(registry().files(), Settings::read().repositories())
}

/// [`check_updates`] for the plugin `files` and the repositories at `urls`.
fn updates_for(files: Vec<PluginFile>, urls: Vec<String>) -> Option<JoinHandle<(Catalog, usize)>> {
    if files.is_empty() || urls.is_empty() {
        return None;
    }
    Some(std::thread::spawn(move || {
        let catalog = repository::fetch_all(&urls);
        let hashes = repository::hashes(files.iter().map(|f| f.path.as_path()));
        let lists = catalog
            .iter()
            .filter_map(|(url, l)| Some((url.as_str(), l.as_ref().ok()?)));
        plugin::Source::name_folders(lists.clone().map(|(url, l)| (url, l.name.as_str())));
        let known = repository::known(&hashes, lists);
        let n = known.values().filter(|k| k.update.is_some()).count();
        (catalog, n)
    }))
}

/// The repository (name, else address, and address) each of `files` was installed from.
fn sources(files: &[PluginFile]) -> std::collections::BTreeMap<PathBuf, (String, String)> {
    (files.iter())
        .filter_map(|f| {
            let s = plugin::source_of(&f.path)?;
            let name = if s.name.is_empty() {
                s.url.clone()
            } else {
                s.name
            };
            Some((f.path.clone(), (name, s.url)))
        })
        .collect()
}

/// The row above the plugins of the repository `source` (name and address), or of
/// those from none: its name as `ui.strong` shows it, across the row from the first column (it takes no
/// width there), its address in the tooltip.
fn group_header(ui: &mut Ui, source: Option<&(String, String)>) {
    let name = source.map_or(tr!("PLUGINS_NOT_FROM_REPOSITORY"), |(name, _)| {
        name.as_str()
    });
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().strong_text_color();
    let galley = (ui.painter()).layout_no_wrap(name.to_owned(), font, color);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(0.0, galley.size().y), egui::Sense::hover());
    let text = egui::Rect::from_min_size(rect.left_top(), galley.size());
    ui.painter().galley(text.min, galley, color);
    if let Some((_, url)) = source {
        ui.interact(text, ui.scope_id().with(url), egui::Sense::hover())
            .on_hover_text(url);
    }
    ui.end_row();
}

/// The Description column's header, up to `right`: the rows go across the window.
fn description_header(ui: &mut Ui, right: f32) {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let size = egui::vec2(right - ui.cursor().left(), height);
    let left = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, left, |ui| {
        ui.strong(tr!("PLUGIN_DESCRIPTION"));
        ui.allocate_space(ui.available_size());
    });
}

/// What a plugin adds, as counts ("1 effect, 1 generator"), the names in the tooltip;
/// empty when its repository does not say.
fn provides(ui: &mut Ui, effects: &[String], generators: &[String]) {
    let mut counts = Vec::new();
    let mut names = Vec::new();
    if !effects.is_empty() {
        counts.push(tr_n!("PLUGIN_EFFECT_COUNT", effects.len()));
        names.push(tr!("PLUGIN_EFFECTS_LIST", names = effects.join(", ")));
    }
    if !generators.is_empty() {
        counts.push(tr_n!("PLUGIN_GENERATOR_COUNT", generators.len()));
        names.push(tr!("PLUGIN_GENERATORS_LIST", names = generators.join(", ")));
    }
    let r = ui.label(counts.join(", "));
    if !names.is_empty() {
        r.on_hover_text(names.join("\n"));
    }
}

/// The version shown for an installed plugin: the one it gives itself once loaded, else the
/// one recorded when installed from a repository, else "?".
///
/// TEMPORARY (see TODO.md): the recorded one also replaces a loaded version that isn't
/// X.Y.Z, as the Python plugins released before the plugin repository checked it give
/// "1.0" for 1.0.0 and 1.0.1.
fn shown_version<'a>(loaded: Option<&'a str>, recorded: Option<&'a str>) -> &'a str {
    let loaded = loaded.filter(|v| !v.is_empty());
    let valid = loaded.filter(|v| semver::Version::parse(v).is_ok());
    valid.or(recorded).or(loaded).unwrap_or("?")
}

/// A check mark: the plugin was updated, the new version used from the next start.
fn updated_mark(ui: &mut Ui) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let c = rect.center();
    let stroke = egui::Stroke::new(1.5, ui.visuals().weak_text_color());
    let points = [
        c + egui::vec2(-5.0, 0.5),
        c + egui::vec2(-1.5, 4.0),
        c + egui::vec2(5.0, -4.0),
    ];
    ui.painter().line(points.to_vec(), stroke);
    r
}

/// That the version `u` is out, to click to update to it: an up arrow in a disc (in the
/// warning color if it breaks with the installed one, faded when not `enabled`), what it
/// does, what changed in it, and that warning in its tooltip.
fn new_version(ui: &mut Ui, u: &repository::Update, enabled: bool) -> egui::Response {
    let color = if u.breaking {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().selection.bg_fill
    };
    let color = if enabled {
        color
    } else {
        color.gamma_multiply(0.4)
    };
    let size = 16.0;
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(egui::vec2(size, size), sense);
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    let (c, radius) = (rect.center(), size / 2.0 - 0.5);
    let p = ui.painter();
    p.circle_filled(c, radius, color);
    let stroke = egui::Stroke::new(1.5, egui::Color32::WHITE);
    let (top, bottom) = (c + egui::vec2(0.0, -4.0), c + egui::vec2(0.0, 4.0));
    p.line_segment([top, bottom], stroke);
    p.line_segment([top, top + egui::vec2(-3.5, 3.5)], stroke);
    p.line_segment([top, top + egui::vec2(3.5, 3.5)], stroke);
    let tip = [
        tr!("UPDATE_PLUGIN_HINT", version = &u.version).to_owned(),
        u.changes.clone(),
        breaking_warning(u),
    ];
    let tip = tip.into_iter().filter(|t| !t.is_empty());
    r.on_hover_text(tip.collect::<Vec<_>>().join("\n\n"))
}

/// That the update `u` can't read what the installed version saved; empty if it can.
fn breaking_warning(u: &repository::Update) -> String {
    if u.breaking {
        tr!("PLUGIN_UPDATE_BREAKS").to_owned()
    } else {
        String::new()
    }
}

/// What to record of the plugin `p` installed at its version `v`.
fn installed_entry(p: &Entry, v: &Version) -> plugin::Installed {
    plugin::Installed {
        entry: PluginRef {
            id: p.id.clone(),
            version: v.version.clone(),
        },
        compatible_from: v.compatible_from.clone(),
        min_sdk: v.min_sdk(),
    }
}

/// The Version cell of the plugin `p` of the repository at `url`: a drop-down of the
/// versions that work here (`picked` shown, what changed in each in its tooltip),
/// disabled when there is only one, "-" if none; the version picked goes in `pick`, with
/// its key.
fn version_choice(
    ui: &mut Ui,
    url: &str,
    p: &Entry,
    compatible: &[(&Version, &Download)],
    picked: Option<&Version>,
    pick: &mut Option<((String, String), String)>,
) {
    let Some(picked) = picked else {
        ui.label("-");
        return;
    };
    let latest = compatible.first().map(|(v, _)| v.version.as_str());
    let name = |v: &Version| match Some(v.version.as_str()) == latest {
        true => tr!("PLUGIN_LATEST_VERSION", version = &v.version),
        false => v.version.clone(),
    };
    ui.add_enabled_ui(compatible.len() > 1, |ui| {
        egui::ComboBox::from_id_salt(("plugin_version", url, &p.id))
            .width(VERSION_WIDTH)
            .selected_text(name(picked))
            .show_ui(ui, |ui| {
                for (v, _) in compatible {
                    let on = v.version == picked.version;
                    let r = ui.selectable_label(on, name(v));
                    let r = match v.changes.is_empty() {
                        true => r,
                        false => r.on_hover_text(&v.changes),
                    };
                    if r.clicked() {
                        let key = (url.to_owned(), p.id.clone());
                        *pick = Some((key, v.version.clone()));
                    }
                }
            });
    });
}

/// The packages the Python plugin `path` needs; none for a library.
/// The folder of the repository `repo` at `url` in the user's plugins folder (see
/// [`plugin::origin`]), with the `source.json` naming it.
fn origin_dir(repo: &str, url: &str) -> Result<PathBuf, String> {
    let dir = user_plugins_dir().ok_or("no data folder")?;
    let dir = dir.join(plugin::origin(url));
    plugin::Source::write(&dir, repo, url).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Downloads `d`, the plugin `entry`, into `dir` and records it there, with the
/// packages a Python plugin needs if `pip` can install them (if not, the file is not
/// kept); the file `old` it replaces is set aside.
fn fetch_plugin(
    d: &Download,
    kind: Kind,
    dir: &Path,
    pip: bool,
    entry: plugin::Installed,
    old: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let path = repository::download(d, dir)?;
    let needs = needs(&path);
    if kind == Kind::Python
        && pip
        && !needs.is_empty()
        && let Err(e) = python::install_dependencies(&needs)
    {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    let name = |p: &Path| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let replaces = old.as_deref().map(name);
    if let Err(e) = plugin::Source::record(dir, &name(&path), entry, replaces.as_deref()) {
        log::warn!("cannot record {}: {e}", path.display());
    }
    if let Some(old) = old.filter(|o| *o != path) {
        repository::set_aside(&old).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// The plugin just saved at `path` allowed, as the user chose to install it, and
/// enabled in `settings`; false if it can't be loaded until the next start.
fn enable_installed(settings: &mut Settings, path: &Path) -> bool {
    match plugin::pending(path) {
        Ok(p) => settings.allow(&p),
        Err(e) => log::warn!("{}: {e}", path.display()),
    }
    settings.set_disabled(path, false);
    settings.save();
    registry().add_installed(path)
}

/// The repositories' lists and installs of the Install buttons of missing effects,
/// for the whole run of the app.
#[derive(Default)]
struct Missing {
    /// The user's repositories, once a missing effect asked for them.
    repos: Option<Vec<Repo>>,
    /// By origin and plugin id.
    installs: Vec<((String, String), MissingInstall)>,
}

enum MissingInstall {
    Running(JoinHandle<Result<PathBuf, String>>),
    /// Installed: loaded, or not until the next start.
    Done(bool),
    Failed(String),
}

static MISSING: std::sync::Mutex<Option<Missing>> = std::sync::Mutex::new(None);

/// What a repository of `repos` (address and list) offers for the missing effect
/// `effect` (`<origin>/<id>`) of the plugin `entry`: its list's name and address, the
/// plugin, and its version and file for this system, if the repository of the same origin lists
/// that plugin at the same version or a later one that works here.
fn offer<'a>(
    repos: impl IntoIterator<Item = (&'a str, &'a Repository)>,
    effect: &str,
    entry: &PluginRef,
) -> Option<(
    &'a str,
    &'a Repository,
    &'a Entry,
    &'a Version,
    &'a Download,
)> {
    let (origin, _) = effect.split_once('/')?;
    let (url, list) = repos
        .into_iter()
        .find(|(url, _)| plugin::origin(url) == origin)?;
    let p = list.plugins.iter().find(|p| p.id == entry.id)?;
    let (v, d) = p.best()?;
    let newer = repository::version_parts(&v.version) >= repository::version_parts(&entry.version);
    newer.then_some((url, list, p, v, d))
}

/// The Install button of the missing effect `effect`, of the plugin `entry`, when a
/// repository of the user has it (see [`offer`]): installs it like the plugin manager.
/// Returns what to say about it: the install failed, or needs a restart.
pub fn install_missing(ui: &mut Ui, effect: &str, entry: &PluginRef) -> Option<(Tone, String)> {
    let origin = (effect.split_once('/').map(|(o, _)| o)).filter(|o| plugin::is_origin(o))?;
    let mut guard = MISSING.lock().unwrap_or_else(|e| e.into_inner());
    let missing = guard.get_or_insert_with(Missing::default);
    let repos = missing.repos.get_or_insert_with(|| {
        let urls = Settings::read().repositories();
        (urls.into_iter())
            .map(|url| Repo {
                list: Fetch::start(&url),
                url,
            })
            .collect()
    });
    for r in repos.iter_mut() {
        r.list.update();
    }
    if repos.iter().any(|r| matches!(r.list, Fetch::Loading(_))) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    let key = (origin.to_owned(), entry.id.clone());
    let mut note = None;
    let install = missing.installs.iter_mut().find(|(k, _)| *k == key);
    if let Some((_, state)) = install {
        if let MissingInstall::Running(h) = state {
            if !h.is_finished() {
                ui.spinner();
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200));
                return None;
            }
            let MissingInstall::Running(h) = std::mem::replace(state, MissingInstall::Done(false))
            else {
                unreachable!()
            };
            *state = match h.join().unwrap_or_else(|_| Err("stopped".into())) {
                Ok(path) => MissingInstall::Done(enable_installed(&mut Settings::read(), &path)),
                Err(e) => MissingInstall::Failed(e),
            };
            ui.ctx().request_repaint();
        }
        match state {
            MissingInstall::Done(true) | MissingInstall::Running(_) => return None,
            MissingInstall::Done(false) => {
                return Some((Tone::Info, tr!("PLUGINS_RESTART").to_owned()));
            }
            MissingInstall::Failed(e) => {
                let text = tr!("PLUGIN_INSTALL_FAILED", plugin = entry.id, error = e);
                note = Some((Tone::Warning, text));
            }
        }
    }
    let lists = repos.iter().filter_map(|r| match &r.list {
        Fetch::Ready(list) => Some((r.url.as_str(), list)),
        _ => None,
    });
    let Some((url, list, p, v, d)) = offer(lists, effect, entry) else {
        return note;
    };
    let button = ui
        .button(tr!("INSTALL_PLUGIN"))
        .on_hover_text(tr!("INSTALL_PLUGIN_HINT"));
    if !button.clicked() {
        return note;
    }
    let found = installed_entry(p, v);
    let (d, kind, pip) = (d.clone(), p.kind, python::pip().is_some());
    let state = match origin_dir(&list.name, url) {
        Ok(dir) => MissingInstall::Running(std::thread::spawn(move || {
            fetch_plugin(&d, kind, &dir, pip, found, None)
        })),
        Err(e) => MissingInstall::Failed(e),
    };
    missing.installs.retain(|(k, _)| *k != key);
    missing.installs.push((key, state));
    None
}

fn needs(path: &Path) -> Vec<String> {
    let python = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("py"));
    match std::fs::read_to_string(path) {
        Ok(source) if python => python::dependencies(&source),
        _ => Vec::new(),
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

    impl Manager {
        /// Whether the plugin file `name` of `origin` (see [`plugin::origin`]) is there
        /// already, in any plugins folder.
        fn is_installed(&self, origin: &str, name: &str) -> bool {
            let is = |p: &Path| {
                plugin::origin_of(p) == Some(origin) && p.file_name().is_some_and(|n| n == name)
            };
            self.installed.iter().any(|p| is(p))
                || self
                    .files
                    .iter()
                    .any(|f| is(&f.path) && !self.uninstalled.contains(&f.path))
        }
    }

    /// One plugin in each state, as found in the user's folder.
    fn sample() -> Vec<PluginFile> {
        let file = |name: &str, python: bool, status: Status| PluginFile {
            path: PathBuf::from("/home/me/.local/share/artwork-creator/plugins").join(name),
            python,
            status,
        };
        let loaded = |name: &str, version: &str| Status::Loaded {
            name: name.into(),
            version: version.into(),
        };
        vec![
            file("libhalftone.so", false, loaded("Halftone", "0.1.0")),
            file("posterize.py", true, loaded("Python example", "1.0")),
            file("new.py", true, Status::Waiting),
            file("old.py", true, Status::Disabled),
            file("broken.py", true, Status::Failed("no plugin".into())),
            file("noversion.py", true, loaded("No version", "")),
        ]
    }

    /// Runs one frame of the window; returns what it gave and where each text is.
    fn frame(
        ctx: &egui::Context,
        m: &mut Manager,
        events: Vec<egui::Event>,
    ) -> (Outcome, Vec<(String, egui::Rect)>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let mut outcome = Outcome::Open;
        let mut out = ctx.run_ui(input, |ui| outcome = m.show(ui.ctx()));
        out.textures_delta.clear();
        let texts = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    egui::Rect::from_min_size(t.pos, t.galley.size()),
                )),
                _ => None,
            })
            .collect();
        (outcome, texts)
    }

    /// A click at `pos`, over three frames; what the last gave.
    fn click(ctx: &egui::Context, m: &mut Manager, pos: egui::Pos2) -> Outcome {
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(ctx, m, vec![egui::Event::PointerMoved(pos)]);
        frame(ctx, m, vec![press(true)]);
        frame(ctx, m, vec![press(false)]).0
    }

    fn find(texts: &[(String, egui::Rect)], text: &str) -> Option<egui::Rect> {
        texts.iter().find(|(t, _)| t == text).map(|(_, r)| *r)
    }

    #[test]
    fn the_window_lists_plugins_and_its_buttons_work() {
        let dir = std::env::temp_dir().join(format!("artwork-manager-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = dir.join("plugins.json");
        let files = sample();
        let manager = || Manager::with(files.clone(), Settings::read_from(Some(saved.clone())));
        let ctx = egui::Context::default();
        let mut m = manager();
        // The window sizes itself on its first frame, and is drawn on the next.
        frame(&ctx, &mut m, Vec::new());
        let (outcome, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(matches!(outcome, Outcome::Open));
        for text in [
            "Plugin manager",
            "new.py",
            "?",
            "Halftone",
            "Disabled",
            "0.1.0",
            "Library",
            "Loaded",
            "Waiting to be allowed",
            "Failed to load",
        ] {
            assert!(find(&texts, text).is_some(), "{text}");
        }
        assert!(find(&texts, "Restart Artwork Creator to apply the changes.").is_none());
        assert!(find(&texts, "Restart").is_none());
        // Unticking a plugin's box disables it, saved at once.
        let enabled = find(&texts, "Enabled").unwrap();
        let row = find(&texts, "new.py").unwrap();
        click(
            &ctx,
            &mut m,
            egui::pos2(enabled.left() + 9.0, row.center().y),
        );
        assert!(Settings::read_from(Some(saved.clone())).is_disabled(&files[2].path));
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "Restart Artwork Creator to apply the changes.").is_some());
        // Allow: the plugin isn't pending in this run's registry.
        let allow = find(&texts, "Allow").unwrap();
        click(&ctx, &mut m, allow.center());
        // Restart, then Close, each from a new window. The window, now without plugins,
        // sizes itself again first.
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        // This run's plugins (none, unless another test installed one).
        let none = texts
            .iter()
            .any(|(t, _)| t.starts_with("No plugins found."));
        // Not compared with registry() again: tests running alongside install into it.
        assert!(
            !m.files
                .iter()
                .any(|f| files.iter().any(|s| s.path == f.path)),
            "the sample's plugins are gone"
        );
        assert_eq!(none, m.files.is_empty());
        let restart = find(&texts, "Restart").unwrap();
        assert!(matches!(
            click(&ctx, &mut m, restart.center()),
            Outcome::Restart
        ));
        let ctx = egui::Context::default();
        let mut m = manager();
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        // The cross at the right end of the title's row.
        let title = find(&texts, tr!("PLUGINS_TITLE")).unwrap();
        let close = egui::pos2(title.left() + WINDOW_WIDTH - 10.0, title.center().y);
        assert!(matches!(click(&ctx, &mut m, close), Outcome::Closed));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_uninstalled_plugin_is_set_aside_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("artwork-uninstall-{}", std::process::id()));
        let origin = plugin::origin("https://example.com/plugins.json");
        std::fs::create_dir_all(dir.join(&origin)).unwrap();
        let path = dir.join(&origin).join("posterize.py");
        std::fs::write(&path, "# plugin").unwrap();
        let mut settings = Settings::read_from(Some(dir.join("plugins.json")));
        settings.allow(&plugin::pending(&path).unwrap());
        settings.set_disabled(&path, true);
        let file = PluginFile {
            path: path.clone(),
            python: true,
            status: Status::Disabled,
        };
        let mut m = Manager::with(vec![file], settings);
        assert!(m.is_installed(&origin, "posterize.py"));
        // The same plugin from another repository is another plugin.
        assert!(!m.is_installed(
            &plugin::origin("http://localhost:8000/plugins.json"),
            "posterize.py"
        ));
        m.uninstall(&path).unwrap();
        assert!(!path.exists() && dir.join(&origin).join(".posterize.py.old").exists());
        assert!(!m.is_installed(&origin, "posterize.py") && !m.changed);
        let saved = Settings::read_from(Some(dir.join("plugins.json")));
        assert!(!saved.is_disabled(&path));
        assert!(
            !saved.is_allowed(
                &plugin::pending(&dir.join(&origin).join(".posterize.py.old"))
                    .map(|mut p| {
                        p.path = path.clone();
                        p
                    })
                    .unwrap()
            )
        );
        // Its row is gone.
        let ctx = egui::Context::default();
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "posterize.py").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_window_shows_this_run_s_plugins() {
        // This run's registry has only the built-in effects: no plugin files.
        assert!(Manager::new(Some(Vec::new())).files.is_empty());
    }

    #[test]
    fn without_plugins_it_says_where_to_put_them() {
        let ctx = egui::Context::default();
        let mut m = Manager::with(Vec::new(), Settings::default());
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(
            texts
                .iter()
                .any(|(t, _)| t.starts_with("No plugins found."))
        );
    }

    #[test]
    fn a_failed_install_leaves_the_plugin_waiting() {
        let ctx = egui::Context::default();
        let mut m = Manager::with(sample(), Settings::default());
        m.pip = true;
        let handle = std::thread::spawn(|| Err("No matching distribution".to_owned()));
        while !handle.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        m.install = Some((m.files[2].path.clone(), handle));
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(m.install.is_none() && m.files[2].status == Status::Waiting);
        assert!(
            texts.iter().any(|(t, _)| t
                == "Could not install the packages new.py needs: No matching distribution")
        );
    }

    #[test]
    fn a_python_plugin_s_needs_are_read_from_its_file() {
        let dir = std::env::temp_dir().join(format!("artwork-needs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = "# /// script\n# dependencies = [\"numpy\"]\n# ///\n";
        for name in ["a.py", "a.so"] {
            std::fs::write(dir.join(name), source).unwrap();
        }
        assert_eq!(needs(&dir.join("a.py")), ["numpy"]);
        assert!(needs(&dir.join("a.so")).is_empty());
        assert!(needs(&dir.join("missing.py")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A repository listing two Python plugins, posterize.py being in `sample`.
    fn repo() -> Repo {
        let plugin = |id: &str, name: &str, file: &str| {
            format!(
                r#"{{"id": "{id}", "name": "{name}", "description": "About {name}", "kind": "python",
                "versions": [{{"version": "1.0.0", "effects": ["{name}"], "files": {{"any": {{
                "url": "https://example.com/{file}", "sha256": "{}", "size": 1}}}}}}]}}"#,
                "0".repeat(64)
            )
        };
        let json = format!(
            r#"{{"format": 1, "name": "Test plugins", "plugins": [{}, {}]}}"#,
            plugin("posterize", "Posterize", "posterize.py"),
            plugin("swap", "Swap channels", "swap_channels.py")
        );
        Repo {
            url: "https://example.com/plugins.json".into(),
            list: Fetch::Ready(repository::parse(&json).unwrap()),
        }
    }

    /// Draws frames until `done` (a background job finished), at most for 10 s.
    fn wait(ctx: &egui::Context, m: &mut Manager, done: impl Fn(&Manager) -> bool) {
        for _ in 0..1000 {
            frame(ctx, m, Vec::new());
            if done(m) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the background job did not finish");
    }

    #[test]
    fn repositories_and_updates_are_downloaded_in_the_background() {
        let new = b"# 1.1\nplugin = None\n".to_vec();
        let base = repository::serve(vec![("new.py", new.clone())]);
        let dir = std::env::temp_dir().join(format!("artwork-update-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.canonicalize().unwrap().join("old.py");
        std::fs::write(&old, "plugin = None\n").unwrap();
        let settings = Settings::read_from(Some(dir.join("plugins.json")));
        let ctx = egui::Context::default();
        let mut m = Manager::with(sample(), settings);
        // A repository whose list is not there, shown as such.
        m.tab = Tab::Available;
        let gone = format!("{base}gone.json");
        m.repos = vec![Repo {
            url: gone.clone(),
            list: Fetch::start(&gone),
        }];
        wait(&ctx, &mut m, |m| {
            !matches!(m.repos[0].list, Fetch::Loading(_))
        });
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(texts.iter().any(|(t, _)| t.contains(&gone)));
        // Added again, or another that can't be read.
        m.new_repo = gone.clone();
        m.add_repository();
        assert!(m.add_error.is_some() && m.adding.is_none());
        m.new_repo = format!("{base}other.json");
        m.add_repository();
        wait(&ctx, &mut m, |m| m.adding.is_none());
        assert!(m.add_error.is_some() && m.repos.len() == 1);
        // An update: the new file beside the old one, allowed.
        let update = |download| repository::Update {
            id: "posterize".into(),
            version: "1.1".into(),
            changes: String::new(),
            kind: Kind::Python,
            download,
            compatible_from: None,
            min_sdk: None,
            breaking: false,
        };
        let d = repository::listed(format!("{base}new.py"), &new);
        m.update("Posterize".into(), old.clone(), update(d.clone()));
        wait(&ctx, &mut m, |m| m.download.is_none());
        assert_eq!(m.download_error, None);
        let path = old.with_file_name("new.py");
        assert_eq!(std::fs::read(&path).unwrap(), new);
        assert_eq!(m.updated, [(old.clone(), "1.1".to_owned())]);
        assert!(m.changed && m.settings.is_allowed(&plugin::pending(&path).unwrap()));
        // One that fails says why.
        let missing = repository::Download {
            url: format!("{base}missing.py"),
            ..d
        };
        m.update("Posterize".into(), path, update(missing));
        wait(&ctx, &mut m, |m| m.download.is_none());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(m.download_error.is_some());
        assert!(texts.iter().any(|(t, _)| t.contains("404")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_python_plugin_whose_packages_can_t_be_installed_is_not_kept() {
        let source = b"# /// script\n# dependencies = [\"numpy\"]\n# ///\nplugin = None\n".to_vec();
        let base = repository::serve(vec![("needs.py", source.clone())]);
        let dir = std::env::temp_dir().join(format!("artwork-fetch-needs-{}", std::process::id()));
        let d = repository::listed(format!("{base}needs.py"), &source);
        let entry = plugin::Installed {
            entry: PluginRef {
                id: "needs".into(),
                version: "1.0".into(),
            },
            compatible_from: None,
            min_sdk: None,
        };
        // This run's Python is not the app's own: no pip.
        assert!(fetch_plugin(&d, Kind::Python, &dir, true, entry.clone(), None).is_err());
        assert!(!dir.join("needs.py").exists());
        // Without pip, kept: the user installs the packages.
        let path = fetch_plugin(&d, Kind::Python, &dir, false, entry, None).unwrap();
        assert!(path.is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_effect_is_offered_from_its_repository_at_its_version_or_later() {
        let r = repo();
        let origin = plugin::origin(&r.url);
        let Fetch::Ready(list) = &r.list else {
            unreachable!()
        };
        let lists = || [(r.url.as_str(), list)];
        let entry = |id: &str, version: &str| PluginRef {
            id: id.into(),
            version: version.into(),
        };
        let effect = format!("{origin}/posterize");
        let found = offer(lists(), &effect, &entry("posterize", "1.0.0"));
        assert_eq!(
            found.map(|(_, _, p, _, _)| p.name.as_str()),
            Some("Posterize")
        );
        assert!(offer(lists(), &effect, &entry("posterize", "0.9")).is_some());
        // Not an older version, another plugin, or from another repository.
        assert!(offer(lists(), &effect, &entry("posterize", "1.1.0")).is_none());
        assert!(offer(lists(), &effect, &entry("blur", "1.0.0")).is_none());
        let elsewhere = format!("{}/posterize", plugin::origin("https://example.org/x.json"));
        assert!(offer(lists(), &elsewhere, &entry("posterize", "1.0.0")).is_none());
    }

    #[test]
    fn the_available_tab_lists_one_repository_at_a_time() {
        let ctx = egui::Context::default();
        let mut m = Manager::with(Vec::new(), Settings::default());
        m.tab = Tab::Available;
        let json = format!(
            r#"{{"format": 1, "name": "Other plugins", "plugins": [{{"id": "blur",
            "name": "Blur", "description": "", "kind": "python", "versions": [{{
            "version": "1.0.0", "files": {{"any": {{"url": "https://example.org/blur.py",
            "sha256": "{}", "size": 1}}}}}}]}}]}}"#,
            "0".repeat(64)
        );
        let other = Repo {
            url: "https://example.org/plugins.json".into(),
            list: Fetch::Ready(repository::parse(&json).unwrap()),
        };
        m.repos = vec![repo(), other];
        frame(&ctx, &mut m, Vec::new());
        // The first one by default, without the official one.
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "Posterize").is_some() && find(&texts, "Blur").is_none());
        // The search looks only at the repository picked.
        m.source = "https://example.org/plugins.json".into();
        m.filter = "posterize".into();
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "No plugins to show.").is_some());
        m.filter = "blur".into();
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "Blur").is_some() && find(&texts, "Posterize").is_none());
    }

    #[test]
    fn the_available_tab_lists_the_repositories_plugins() {
        let dir = std::env::temp_dir().join(format!("artwork-available-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = dir.join("plugins.json");
        let mut settings = Settings::read_from(Some(saved.clone()));
        settings.add_repository("https://example.com/plugins.json");
        let ctx = egui::Context::default();
        let mut m = Manager::with(sample(), settings);
        m.tab = Tab::Available;
        m.repos = vec![repo()];
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        for text in [
            "Test plugins",
            "Posterize",
            "About Swap channels",
            "1.0.0 (latest)",
            "Python",
            "Provides",
            "1 effect",
            "Installed",
            "Install",
        ] {
            assert!(find(&texts, text).is_some(), "{text}");
        }
        // The repositories are on a tab of their own.
        assert!(find(&texts, "Add repository").is_none());
        // The filter keeps the plugins whose name, description... contain it.
        m.filter = "SWAP".into();
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "Posterize").is_none() && find(&texts, "Swap channels").is_some());
        m.filter = "nothing like it".into();
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "No plugins to show.").is_some());
        // ...or the name of one of their effects.
        m.filter = "swap c".into();
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "Posterize").is_none() && find(&texts, "Swap channels").is_some());
        m.tab = Tab::Repositories;
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        for text in [
            "Test plugins",
            "https://example.com/plugins.json",
            "2 plugins",
            "Add repository",
        ] {
            assert!(find(&texts, text).is_some(), "{text}");
        }
        assert!(find(&texts, "Posterize").is_none());
        // An address already listed is not added again; Remove takes a repository out.
        m.new_repo = "https://example.com/plugins.json".into();
        m.add_repository();
        assert_eq!(
            m.add_error.as_deref(),
            Some("This repository is already in the list.")
        );
        // The window grows with the message, and is centered again on the next frame.
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        let remove = find(&texts, "Remove").unwrap();
        click(&ctx, &mut m, remove.center());
        assert!(m.repos.is_empty());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(texts.iter().any(|(t, _)| t.starts_with("No repositories.")));
        let official = vec![repository::OFFICIAL.to_owned()];
        assert_eq!(
            Settings::read_from(Some(saved.clone())).repositories(),
            official
        );
        // The official one is there until removed, like any other.
        let mut settings = Settings::read_from(Some(saved.clone()));
        settings.remove_repository(repository::OFFICIAL);
        settings.save();
        assert!(Settings::read_from(Some(saved)).repositories().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_available_tab_offers_each_version_that_works_here() {
        let ctx = egui::Context::default();
        let mut m = Manager::with(Vec::new(), Settings::read_from(None));
        m.tab = Tab::Available;
        let file = |v: &str| {
            format!(
                r#"{{"any": {{"url": "https://example.com/{v}/posterize.py", "sha256": "{}", "size": 1}}}}"#,
                "0".repeat(64)
            )
        };
        let json = format!(
            r#"{{"format": 1, "name": "Test plugins", "plugins": [{{
            "id": "posterize", "name": "Posterize", "kind": "python", "versions": [
              {{"version": "1.0.0", "effects": ["Posterize"], "files": {}}},
              {{"version": "1.1.0", "changes": "Faster", "effects": ["Posterize", "Bands"],
                "files": {}}},
              {{"version": "2.0.0", "engines": {{"app": ">=99.0.0"}}, "files": {}}}
            ]}}]}}"#,
            file("1.0.0"),
            file("1.1.0"),
            file("2.0.0")
        );
        let url = "https://example.com/plugins.json";
        m.repos = vec![Repo {
            url: url.into(),
            list: Fetch::Ready(repository::parse(&json).unwrap()),
        }];
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        // The latest that works here by default, not one too new for this app.
        assert!(find(&texts, "1.1.0 (latest)").is_some() && find(&texts, "2 effects").is_some());
        let Fetch::Ready(list) = &m.repos[0].list else {
            unreachable!()
        };
        let p = list.plugins[0].clone();
        let picked = |m: &Manager| {
            m.picked(url, &p, &p.compatible())
                .map(|(v, _)| v.version.clone())
        };
        assert_eq!(picked(&m).as_deref(), Some("1.1.0"));
        // The drop-down lists the others; picking one shows it, and installs it.
        click(
            &ctx,
            &mut m,
            find(&texts, "1.1.0 (latest)").unwrap().center(),
        );
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "2.0.0").is_none());
        click(&ctx, &mut m, find(&texts, "1.0.0").unwrap().center());
        assert_eq!(picked(&m).as_deref(), Some("1.0.0"));
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(find(&texts, "1.0.0").is_some() && find(&texts, "1 effect").is_some());
        let (v, _) = m.picked(url, &p, &p.compatible()).unwrap();
        assert_eq!(installed_entry(&p, v).entry.version, "1.0.0");
        // A version picked that no longer works here gives the latest again.
        m.versions
            .insert((url.into(), "posterize".into()), "2.0.0".into());
        assert_eq!(picked(&m).as_deref(), Some("1.1.0"));
        // With 1.1.0 installed (whatever its file's name), it says so, and another
        // version picked can be installed in its place.
        let path = Path::new("plugins")
            .join(plugin::origin(url))
            .join("old_name.py");
        m.files = vec![PluginFile {
            path: path.clone(),
            python: true,
            status: Status::Disabled,
        }];
        let known = |version: &str| Known {
            id: "posterize".into(),
            description: String::new(),
            version: version.into(),
            effects: Vec::new(),
            generators: Vec::new(),
            update: None,
        };
        m.known.insert(path.clone(), known("1.1.0"));
        // "Installed" is also the name of a tab.
        let count = |texts: &[(String, egui::Rect)], text: &str| {
            texts.iter().filter(|(t, _)| t == text).count()
        };
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(count(&texts, "Installed") == 2 && find(&texts, "Install").is_none());
        m.versions
            .insert((url.into(), "posterize".into()), "1.0.0".into());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(count(&texts, "Installed") == 1 && find(&texts, "Install").is_some());
        let origin = plugin::origin(url);
        let old = m.installed_of(&origin, &p, "posterize.py");
        assert_eq!(old, Some((path, Some("1.1.0".into()))));
    }

    /// A repository listing `path`'s content as Posterize 1.0.0, and a version 1.1.0.
    fn repo_with_update(path: &Path) -> Repo {
        let hash = plugin::pending(path).unwrap().hash;
        let json = format!(
            r#"{{"format": 1, "name": "Test plugins", "plugins": [{{
            "id": "posterize", "name": "Posterize", "description": "Fewer colors",
            "kind": "python", "versions": [
              {{"version": "1.0.0", "effects": ["Posterize"], "generators": ["Bands", "Steps"],
                "files": {{"any": {{
                "url": "https://example.com/1.0.0/posterize.py", "sha256": "{hash}", "size": 2}}}}}},
              {{"version": "1.1.0", "changes": "Faster", "files": {{"any": {{
                "url": "https://example.com/1.1.0/posterize.py", "sha256": "{}", "size": 2}}}}}}
            ]}}]}}"#,
            "1".repeat(64)
        );
        Repo {
            url: "https://example.com/plugins.json".into(),
            list: Fetch::Ready(repository::parse(&json).unwrap()),
        }
    }

    #[test]
    fn the_installed_tab_groups_plugins_by_repository() {
        let dir = std::env::temp_dir().join(format!("artwork-groups-{}", std::process::id()));
        let url = "https://example.com/plugins.json";
        let folder = dir.join(plugin::origin(url));
        plugin::Source::write(&folder, "Test plugins", url).unwrap();
        let file = |path: PathBuf, name: &str| PluginFile {
            path,
            python: true,
            status: Status::Loaded {
                name: name.into(),
                version: "1.0".into(),
            },
        };
        let files = vec![
            file(dir.join("mine.py"), "Mine"),
            file(folder.join("theirs.py"), "Theirs"),
        ];
        let mut m = Manager::with(files, Settings::default());
        let ctx = egui::Context::default();
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        let y = |t: &str| find(&texts, t).unwrap().top();
        // The repository's plugins first, under its name; the others under theirs.
        assert!(y("Test plugins") < y("Theirs"));
        assert!(y("Theirs") < y("Not from a repository"));
        assert!(y("Not from a repository") < y("Mine"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_installed_tab_shows_descriptions_and_new_versions() {
        let dir = std::env::temp_dir().join(format!("artwork-updates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("posterize.py");
        std::fs::write(&path, "v1").unwrap();
        let files = vec![PluginFile {
            path: path.clone(),
            python: true,
            status: Status::Loaded {
                name: "Posterize".into(),
                version: "1.0.0".into(),
            },
        }];
        let ctx = egui::Context::default();
        let mut m = Manager::with(files, Settings::read_from(Some(dir.join("plugins.json"))));
        m.repos = vec![repo_with_update(&path)];
        m.refresh_known();
        frame(&ctx, &mut m, Vec::new());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        for text in [
            "Description",
            "Fewer colors",
            "1.0.0",
            "1 effect, 2 generators",
        ] {
            assert!(find(&texts, text).is_some(), "{text}");
        }
        // Updated: the new file is allowed, the row says so, the restart is asked for.
        std::fs::write(&path, "v2").unwrap();
        m.updated_file(&path, &path, "1.1.0".into());
        let (_, texts) = frame(&ctx, &mut m, Vec::new());
        assert!(m.updated.iter().any(|(p, v)| *p == path && v == "1.1.0"));
        assert!(find(&texts, "Restart Artwork Creator to apply the changes.").is_some());
        let settings = Settings::read_from(Some(dir.join("plugins.json")));
        assert!(settings.is_allowed(&plugin::pending(&path).unwrap()));
        // Its repository removed, it stays installed, but nothing is known of it.
        m.tab = Tab::Repositories;
        m.repos.clear();
        m.refresh_known();
        assert!(m.known.is_empty() && m.files.len() == 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shown_version_prefers_a_valid_one() {
        assert_eq!(shown_version(Some("1.2.0"), Some("1.0.0")), "1.2.0");
        // Not X.Y.Z (a Python plugin's "1.0"): the recorded one, if any (temporary, TODO.md).
        assert_eq!(shown_version(Some("1.0"), Some("1.0.1")), "1.0.1");
        assert_eq!(shown_version(Some("1.0"), None), "1.0");
        assert_eq!(shown_version(Some(""), Some("1.0.1")), "1.0.1");
        assert_eq!(shown_version(None, Some("1.0.1")), "1.0.1");
        assert_eq!(shown_version(None, None), "?");
        assert_eq!(shown_version(Some(""), None), "?");
    }

    #[test]
    fn without_plugins_or_repositories_no_update_is_looked_for() {
        assert!(updates_for(Vec::new(), vec![repository::OFFICIAL.into()]).is_none());
        assert!(updates_for(sample(), Vec::new()).is_none());
    }

    #[test]
    fn a_plugin_installed_is_allowed_and_enabled() {
        let dir = std::env::temp_dir().join(format!("artwork-installed-{}", std::process::id()));
        let origin = plugin::origin("https://example.com/plugins.json");
        std::fs::create_dir_all(dir.join(&origin)).unwrap();
        let path = dir
            .canonicalize()
            .unwrap()
            .join(&origin)
            .join("swap_channels.py");
        std::fs::write(&path, "plugin = None\n").unwrap();
        let mut settings = Settings::read_from(Some(dir.join("plugins.json")));
        settings.set_disabled(&path, true);
        let mut m = Manager::with(sample(), settings);
        assert!(!m.is_installed(&origin, "swap_channels.py"));
        m.installed_file(&path);
        assert!(m.is_installed(&origin, "swap_channels.py"));
        let settings = Settings::read_from(Some(dir.join("plugins.json")));
        assert!(!settings.is_disabled(&path));
        assert!(settings.is_allowed(&plugin::pending(&path).unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `PLUGINS_PREVIEW=out.png cargo test plugin_manager_preview -- --ignored` draws
    /// the window in both themes.
    #[test]
    #[ignore]
    fn plugin_manager_preview() {
        let Ok(path) = std::env::var("PLUGINS_PREVIEW") else {
            return;
        };
        let files = sample();
        crate::ui::preview::render(&path, egui::vec2(940.0, 700.0), |ui| {
            let mut m = Manager::with(files.clone(), Settings::default());
            m.changed = true;
            m.pip = true;
            // The first two installed from the official repository.
            for f in &files[..2] {
                let official = (
                    "Artwork Creator community plugins".into(),
                    repository::OFFICIAL.into(),
                );
                m.sources.insert(f.path.clone(), official);
            }
            m.settings.set_disabled(&files[3].path, true);
            // Python example, a new version out.
            let u = repository::Update {
                id: "posterize".into(),
                version: "1.1".into(),
                changes: "Faster".into(),
                kind: Kind::Python,
                download: Download {
                    url: "https://example.com/posterize.py".into(),
                    sha256: "0".repeat(64),
                    size: 1,
                },
                compatible_from: None,
                min_sdk: None,
                breaking: false,
            };
            let known = Known {
                id: "posterize".into(),
                description: "Posterize effect and bands generator".into(),
                version: "1.0".into(),
                effects: vec!["Posterize".into()],
                generators: vec!["Bands".into()],
                update: Some(u),
            };
            m.known.insert(files[1].path.clone(), known);
            m.show(ui.ctx());
        });
        // The Available tab, beside it.
        let available = path.replace(".png", "-available.png");
        crate::ui::preview::render(&available, egui::vec2(940.0, 700.0), |ui| {
            let mut m = Manager::with(files.clone(), Settings::default());
            m.tab = Tab::Available;
            m.repos = vec![repo()];
            m.show(ui.ctx());
        });
        let repositories = path.replace(".png", "-repositories.png");
        crate::ui::preview::render(&repositories, egui::vec2(940.0, 700.0), |ui| {
            let mut m = Manager::with(files.clone(), Settings::default());
            m.tab = Tab::Repositories;
            m.repos = vec![repo()];
            m.show(ui.ctx());
        });
    }
}
