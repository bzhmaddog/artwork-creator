//! Keyboard shortcuts the user can change: each command's keys, kept between sessions,
//! and the View > Keyboard shortcuts window that edits them.
//!
//! Cut, copy and paste stay Ctrl+X, C and V (egui turns those keys into clipboard
//! events), and the arrow keys nudge the selection: those are listed in the window but
//! cannot be changed or given to another command. A shortcut belongs to one command: a
//! key in use is refused until it is removed from the command that has it.

use std::collections::{BTreeMap, HashMap};

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::i18n::{tr, tr_noop, translate};

/// Key in eframe's storage.
const STORAGE_KEY: &str = "shortcuts";
/// Width of the window's contents.
const WINDOW_WIDTH: f32 = 560.0;
/// Width of the commands' names.
const LABEL_WIDTH: f32 = 220.0;
/// Height of a command's row.
const ROW: f32 = 28.0;
/// Size of the icon buttons.
const ICON: f32 = 24.0;
/// Seconds the row of a command with a refused key stays highlighted, then fades out.
const CONFLICT_HOLD: f64 = 2.0;
const CONFLICT_FADE: f64 = 1.0;

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Command {
    NewProject,
    OpenProject,
    Save,
    SaveAs,
    Export,
    Print,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    Nudge,
    AddImages,
    AddText,
    AddShape,
    AddCanvas,
    Group,
    Ungroup,
    FitPage,
    RealSize,
    ShowTemplate,
    PreviewCuts,
    PreviewBleed,
    PreviewSafeArea,
    SnapToAnchors,
    SnapToEdges,
    SnapToGuides,
    ShowRulers,
    ShowGuides,
    LockGuides,
    ClearGuides,
    KeyboardShortcuts,
    SelectTool,
    PencilTool,
    BrushTool,
    EraserTool,
    ClearPaint,
    About,
}

/// The menu (or panel) a command is listed under in the window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    File,
    Edit,
    Layers,
    /// The View panel, the zoom buttons in the status bar and the View menu's Keyboard
    /// shortcuts.
    View,
    Tools,
    Help,
}

impl Group {
    const ALL: [Group; 6] = [
        Group::File,
        Group::Edit,
        Group::Layers,
        Group::View,
        Group::Tools,
        Group::Help,
    ];

    fn title(self) -> &'static str {
        translate(match self {
            Group::File => tr_noop!("MENU_FILE"),
            Group::Edit => tr_noop!("MENU_EDIT"),
            Group::Layers => tr_noop!("LAYERS_SECTION"),
            Group::View => tr_noop!("MENU_VIEW"),
            Group::Tools => tr_noop!("MENU_TOOLS"),
            Group::Help => tr_noop!("MENU_HELP"),
        })
    }
}

const fn sc(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const NONE: Modifiers = Modifiers::NONE;

impl Command {
    /// In the order of the window.
    pub const ALL: [Command; 41] = [
        Command::NewProject,
        Command::OpenProject,
        Command::Save,
        Command::SaveAs,
        Command::Export,
        Command::Print,
        Command::Quit,
        Command::Undo,
        Command::Redo,
        Command::Cut,
        Command::Copy,
        Command::Paste,
        Command::Duplicate,
        Command::Delete,
        Command::Nudge,
        Command::AddImages,
        Command::AddText,
        Command::AddShape,
        Command::AddCanvas,
        Command::Group,
        Command::Ungroup,
        Command::FitPage,
        Command::RealSize,
        Command::ShowTemplate,
        Command::PreviewCuts,
        Command::PreviewBleed,
        Command::PreviewSafeArea,
        Command::SnapToAnchors,
        Command::SnapToEdges,
        Command::SnapToGuides,
        Command::ShowRulers,
        Command::ShowGuides,
        Command::LockGuides,
        Command::ClearGuides,
        Command::KeyboardShortcuts,
        Command::SelectTool,
        Command::PencilTool,
        Command::BrushTool,
        Command::EraserTool,
        Command::ClearPaint,
        Command::About,
    ];

    /// Its name in the window.
    pub fn label(self) -> &'static str {
        translate(match self {
            Command::NewProject => tr_noop!("NEW_PROJECT"),
            Command::OpenProject => tr_noop!("OPEN_PROJECT"),
            Command::Save => tr_noop!("SAVE"),
            Command::SaveAs => tr_noop!("SAVE_PROJECT_AS"),
            Command::Export => tr_noop!("EXPORT"),
            Command::Print => tr_noop!("PRINT"),
            Command::Quit => tr_noop!("QUIT"),
            Command::Undo => tr_noop!("UNDO"),
            Command::Redo => tr_noop!("REDO"),
            Command::Cut => tr_noop!("CUT_LAYER"),
            Command::Copy => tr_noop!("COPY_LAYER"),
            Command::Paste => tr_noop!("PASTE_LAYER"),
            Command::Duplicate => tr_noop!("DUPLICATE_LAYER"),
            Command::Delete => tr_noop!("DELETE_LAYER"),
            Command::Nudge => tr_noop!("COMMAND_NUDGE"),
            Command::AddImages => tr_noop!("COMMAND_ADD_IMAGES"),
            Command::AddText => tr_noop!("ADD_TEXT_LAYER"),
            Command::AddShape => tr_noop!("COMMAND_ADD_SHAPE"),
            Command::AddCanvas => tr_noop!("COMMAND_ADD_CANVAS"),
            Command::Group => tr_noop!("COMMAND_GROUP"),
            Command::Ungroup => tr_noop!("UNGROUP_LAYERS"),
            Command::FitPage => tr_noop!("FIT_PAGE_IN_VIEW"),
            Command::RealSize => tr_noop!("ZOOM_REAL_SIZE"),
            Command::ShowTemplate => tr_noop!("SHOW_TEMPLATE_LINES"),
            Command::PreviewCuts => tr_noop!("PREVIEW_CUTS"),
            Command::PreviewBleed => tr_noop!("PREVIEW_BLEED"),
            Command::PreviewSafeArea => tr_noop!("PREVIEW_SAFE_AREA"),
            Command::SnapToAnchors => tr_noop!("SNAP_TO_ANCHORS"),
            Command::SnapToEdges => tr_noop!("SNAP_TO_EDGES"),
            Command::SnapToGuides => tr_noop!("SNAP_TO_GUIDES"),
            Command::ShowRulers => tr_noop!("SHOW_RULERS"),
            Command::ShowGuides => tr_noop!("SHOW_GUIDES"),
            Command::LockGuides => tr_noop!("LOCK_GUIDES"),
            Command::ClearGuides => tr_noop!("CLEAR_GUIDES"),
            Command::KeyboardShortcuts => tr_noop!("MENU_KEYBOARD_SHORTCUTS"),
            Command::SelectTool => tr_noop!("TOOL_SELECT"),
            Command::PencilTool => tr_noop!("TOOL_PENCIL"),
            Command::BrushTool => tr_noop!("TOOL_BRUSH"),
            Command::EraserTool => tr_noop!("TOOL_ERASER"),
            Command::ClearPaint => tr_noop!("CLEAR_PAINT"),
            Command::About => tr_noop!("MENU_ABOUT"),
        })
    }

    fn group(self) -> Group {
        match self {
            Command::NewProject
            | Command::OpenProject
            | Command::Save
            | Command::SaveAs
            | Command::Export
            | Command::Print
            | Command::Quit => Group::File,
            Command::Undo
            | Command::Redo
            | Command::Cut
            | Command::Copy
            | Command::Paste
            | Command::Duplicate
            | Command::Delete
            | Command::Nudge => Group::Edit,
            Command::AddImages
            | Command::AddText
            | Command::AddShape
            | Command::AddCanvas
            | Command::Group
            | Command::Ungroup => Group::Layers,
            Command::FitPage
            | Command::RealSize
            | Command::ShowTemplate
            | Command::PreviewCuts
            | Command::PreviewBleed
            | Command::PreviewSafeArea
            | Command::SnapToAnchors
            | Command::SnapToEdges
            | Command::SnapToGuides
            | Command::ShowRulers
            | Command::ShowGuides
            | Command::LockGuides
            | Command::ClearGuides
            | Command::KeyboardShortcuts => Group::View,
            Command::SelectTool
            | Command::PencilTool
            | Command::BrushTool
            | Command::EraserTool
            | Command::ClearPaint => Group::Tools,
            Command::About => Group::Help,
        }
    }

    /// Its keys before the user changes them.
    pub fn defaults(self) -> Vec<KeyboardShortcut> {
        match self {
            Command::NewProject => vec![sc(CMD, Key::N)],
            Command::OpenProject => vec![sc(CMD, Key::O)],
            Command::Save => vec![sc(CMD, Key::S)],
            Command::Export => vec![sc(CMD, Key::E)],
            Command::Print => vec![sc(CMD, Key::P)],
            Command::Undo => vec![sc(CMD, Key::Z)],
            Command::Redo => vec![sc(CMD_SHIFT, Key::Z), sc(CMD, Key::Y)],
            Command::Duplicate => vec![sc(CMD, Key::D)],
            Command::Delete => vec![sc(NONE, Key::Delete), sc(NONE, Key::Backspace)],
            Command::AddImages => vec![sc(CMD, Key::I)],
            Command::AddText => vec![sc(CMD, Key::T)],
            Command::AddShape => vec![sc(CMD, Key::R)],
            Command::AddCanvas => vec![sc(CMD, Key::K)],
            Command::Group => vec![sc(CMD, Key::G)],
            Command::Ungroup => vec![sc(CMD_SHIFT, Key::G)],
            Command::FitPage => vec![sc(NONE, Key::F)],
            Command::SelectTool => vec![sc(NONE, Key::V)],
            Command::PencilTool => vec![sc(NONE, Key::P)],
            Command::BrushTool => vec![sc(NONE, Key::B)],
            Command::EraserTool => vec![sc(NONE, Key::E)],
            Command::ClearPaint => vec![sc(CMD_SHIFT, Key::Delete)],
            _ => fixed(self),
        }
    }

    /// Cut, copy, paste and nudging keep their keys.
    pub fn is_fixed(self) -> bool {
        matches!(
            self,
            Command::Cut | Command::Copy | Command::Paste | Command::Nudge
        )
    }

    /// Works while a text field has the focus (with Ctrl or Alt): the File commands.
    fn works_while_typing(self) -> bool {
        self.group() == Group::File || self == Command::KeyboardShortcuts
    }

    /// Its name in the saved settings; stays the same across versions.
    fn id(self) -> &'static str {
        match self {
            Command::NewProject => "new_project",
            Command::OpenProject => "open_project",
            Command::Save => "save",
            Command::SaveAs => "save_as",
            Command::Export => "export",
            Command::Print => "print",
            Command::Quit => "quit",
            Command::Undo => "undo",
            Command::Redo => "redo",
            Command::Cut => "cut",
            Command::Copy => "copy",
            Command::Paste => "paste",
            Command::Duplicate => "duplicate",
            Command::Delete => "delete",
            Command::Nudge => "nudge",
            Command::AddImages => "add_images",
            Command::AddText => "add_text",
            Command::AddShape => "add_shape",
            Command::AddCanvas => "add_canvas",
            Command::Group => "group",
            Command::Ungroup => "ungroup",
            Command::FitPage => "fit_page",
            Command::RealSize => "real_size",
            Command::ShowTemplate => "show_template",
            Command::PreviewCuts => "preview_cuts",
            Command::PreviewBleed => "preview_bleed",
            Command::PreviewSafeArea => "preview_safe_area",
            Command::SnapToAnchors => "snap_to_anchors",
            Command::SnapToEdges => "snap_to_edges",
            Command::SnapToGuides => "snap_to_guides",
            Command::ShowRulers => "show_rulers",
            Command::ShowGuides => "show_guides",
            Command::LockGuides => "lock_guides",
            Command::ClearGuides => "clear_guides",
            Command::KeyboardShortcuts => "keyboard_shortcuts",
            Command::SelectTool => "select_tool",
            Command::PencilTool => "pencil_tool",
            Command::BrushTool => "brush_tool",
            Command::EraserTool => "eraser_tool",
            Command::ClearPaint => "clear_paint",
            Command::About => "about",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Command::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// The keys of the commands that keep theirs. egui also turns Shift+Del, Ctrl+Insert
/// and Shift+Insert into cut, copy and paste on Windows.
fn fixed(command: Command) -> Vec<KeyboardShortcut> {
    let windows = cfg!(target_os = "windows");
    let shift = Modifiers::SHIFT;
    match command {
        Command::Cut => {
            let mut v = vec![sc(CMD, Key::X)];
            if windows {
                v.push(sc(shift, Key::Delete));
            }
            v
        }
        Command::Copy => {
            let mut v = vec![sc(CMD, Key::C)];
            if windows {
                v.push(sc(Modifiers::CTRL, Key::Insert));
            }
            v
        }
        Command::Paste => {
            let mut v = vec![sc(CMD, Key::V)];
            if windows {
                v.push(sc(shift, Key::Insert));
            }
            v
        }
        Command::Nudge => [
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::ArrowUp,
            Key::ArrowDown,
        ]
        .into_iter()
        .flat_map(|k| [sc(NONE, k), sc(shift, k)])
        .collect(),
        _ => Vec::new(),
    }
}

/// Each command's keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Shortcuts {
    keys: HashMap<Command, Vec<KeyboardShortcut>>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            keys: Command::ALL.iter().map(|&c| (c, c.defaults())).collect(),
        }
    }
}

impl Shortcuts {
    /// The shortcuts saved by a previous session, or the default ones. Only the commands
    /// the user changed are saved, so new commands and defaults show up.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let saved: BTreeMap<String, Vec<KeyboardShortcut>> = storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default();
        Self::from_saved(saved)
    }

    fn from_saved(saved: BTreeMap<String, Vec<KeyboardShortcut>>) -> Self {
        let mut out = Self::default();
        for (id, keys) in saved {
            if let Some(c) = Command::from_id(&id).filter(|c| !c.is_fixed()) {
                // A modifier alone, recorded before modifiers were skipped, is no shortcut.
                let keys = keys
                    .into_iter()
                    .filter(|s| !is_modifier(s.logical_key))
                    .collect();
                out.keys.insert(c, keys);
            }
        }
        out
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.to_saved());
    }

    fn to_saved(&self) -> BTreeMap<String, Vec<KeyboardShortcut>> {
        Command::ALL
            .iter()
            .filter(|c| !c.is_fixed() && self.get(**c) != c.defaults())
            .map(|&c| (c.id().to_owned(), self.get(c).to_vec()))
            .collect()
    }

    pub fn get(&self, command: Command) -> &[KeyboardShortcut] {
        self.keys.get(&command).map_or(&[], Vec::as_slice)
    }

    /// The command a shortcut belongs to.
    fn owner(&self, shortcut: KeyboardShortcut) -> Option<Command> {
        Command::ALL
            .into_iter()
            .find(|&c| self.get(c).contains(&shortcut))
    }

    /// The first of a command's keys, as shown in menus and tooltips.
    pub fn text(&self, ctx: &egui::Context, command: Command) -> Option<String> {
        self.get(command).first().map(|s| ctx.format_shortcut(s))
    }

    /// Each command's first key, as text, for code that only shows them.
    pub fn texts(&self, ctx: &egui::Context) -> HashMap<Command, String> {
        Command::ALL
            .into_iter()
            .filter_map(|c| Some((c, self.text(ctx, c)?)))
            .collect()
    }

    /// Gives `shortcut` to `command` in place of its key at `index` (added when `None`).
    /// A shortcut belongs to one command: when another one has it, nothing changes and
    /// that command is returned.
    fn assign(
        &mut self,
        command: Command,
        index: Option<usize>,
        shortcut: KeyboardShortcut,
    ) -> Result<(), Command> {
        match self.owner(shortcut) {
            Some(c) if c == command => return Ok(()),
            Some(c) => return Err(c),
            None => {}
        }
        let keys = self.keys.entry(command).or_default();
        match index.filter(|&i| i < keys.len()) {
            Some(i) => keys[i] = shortcut,
            None => keys.push(shortcut),
        }
        Ok(())
    }

    fn remove(&mut self, command: Command, index: usize) {
        if let Some(keys) = self.keys.get_mut(&command)
            && index < keys.len()
        {
            keys.remove(index);
        }
    }

    /// Gives a command its default keys back, taking them from the commands that have
    /// them now. Returns those commands.
    fn reset(&mut self, command: Command) -> Vec<(KeyboardShortcut, Command)> {
        let mut kept = Vec::new();
        self.keys.insert(command, Vec::new());
        for s in command.defaults() {
            if let Err(c) = self.assign(command, None, s) {
                kept.push((s, c));
            }
        }
        kept
    }

    /// The commands whose keys were pressed this frame, their key events consumed. A
    /// shortcut matches its modifiers exactly, so Ctrl+Z does not fire on Ctrl+Shift+Z.
    /// While a text field has the focus (`typing`), only the File commands work, and only
    /// with Ctrl or Alt held, so typed letters stay letters.
    pub fn pressed(&self, ctx: &egui::Context, typing: bool) -> Vec<Command> {
        let mut out = Vec::new();
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                else {
                    return true;
                };
                let held = modifiers.command || modifiers.ctrl || modifiers.alt;
                let hit = Command::ALL.into_iter().find(|&c| {
                    !c.is_fixed()
                        && (!typing || (c.works_while_typing() && held))
                        && self
                            .get(c)
                            .iter()
                            .any(|s| s.logical_key == *key && modifiers.matches_exact(s.modifiers))
                });
                match hit {
                    Some(c) => {
                        out.push(c);
                        false
                    }
                    None => true,
                }
            });
        });
        out
    }
}

/// A tooltip with the shortcut of its command after it, if it has one.
pub fn with_key(text: &str, shortcut: Option<&String>) -> String {
    match shortcut {
        Some(s) => tr!("WITH_SHORTCUT", text = text, shortcut = s),
        None => text.to_owned(),
    }
}

/// Ctrl, Shift, Alt or Super pressed on its own: egui reports those as keys too, but a
/// shortcut is held modifiers and another key.
fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
}

/// The shortcut pressed, with the modifiers as shortcuts are written: Ctrl as the
/// command key outside macOS, Cmd on it.
fn recorded(key: Key, m: Modifiers) -> KeyboardShortcut {
    let modifiers = Modifiers {
        alt: m.alt,
        shift: m.shift,
        ctrl: m.ctrl && !m.command,
        mac_cmd: false,
        command: m.command,
    };
    KeyboardShortcut::new(modifiers, key)
}

/// The Keyboard shortcuts window: a copy of the shortcuts, applied by OK.
pub struct Editor {
    draft: Shortcuts,
    filter: String,
    /// The command waiting for keys, and which of its keys they replace (added when
    /// `None`).
    recording: Option<(Command, Option<usize>)>,
    /// Why the last key pressed or reset was refused.
    notice: Option<String>,
    /// The commands that have the keys refused, with those keys: their rows are
    /// highlighted for a moment, or until the keys are freed.
    conflicts: Vec<(Command, KeyboardShortcut)>,
    /// When the highlight started, set on the first frame that shows it.
    conflict_since: Option<f64>,
    /// How strong the highlight is this frame, from 1 down to 0 as it fades.
    conflict_strength: f32,
    /// Scroll to the first of them, once.
    scroll_to_conflict: bool,
}

/// What the window asks the app to do.
pub enum Outcome {
    Open,
    /// OK: use these.
    Apply(Shortcuts),
    Cancel,
}

impl Editor {
    pub fn new(shortcuts: &Shortcuts) -> Self {
        Self {
            draft: shortcuts.clone(),
            filter: String::new(),
            recording: None,
            notice: None,
            conflicts: Vec::new(),
            conflict_since: None,
            conflict_strength: 0.0,
            scroll_to_conflict: false,
        }
    }

    /// Shows which commands have the keys refused, or nothing.
    fn set_conflicts(&mut self, conflicts: Vec<(Command, KeyboardShortcut)>) {
        self.scroll_to_conflict = !conflicts.is_empty();
        self.conflicts = conflicts;
        self.conflict_since = None;
    }

    /// The window, once a frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        if self.recording.is_some() {
            self.record(ctx);
        }
        // A key freed from the command that had it: no conflict left to show.
        let before = self.conflicts.len();
        let draft = &self.draft;
        self.conflicts.retain(|(c, s)| draft.get(*c).contains(s));
        if before > 0 && self.conflicts.is_empty() {
            self.notice = None;
        }
        // Held, then faded out; the warning stays.
        if !self.conflicts.is_empty() {
            let now = ctx.input(|i| i.time);
            let since = *self.conflict_since.get_or_insert(now);
            let fade = ((now - since - CONFLICT_HOLD) / CONFLICT_FADE).clamp(0.0, 1.0);
            self.conflict_strength = 1.0 - fade as f32;
            if fade >= 1.0 {
                self.conflicts.clear();
            } else if fade > 0.0 {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after_secs((since + CONFLICT_HOLD - now) as f32);
            }
        }
        let mut ok = false;
        let mut cancel = false;
        let response = crate::app::dialog(ctx, "keyboard_shortcuts").show(ctx, |ui| {
            ui.set_width(WINDOW_WIDTH);
            ui.heading(tr!("KEYBOARD_SHORTCUTS_TITLE"));
            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text(tr!("FILTER_SHORTCUTS_HINT"))
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.6)
                .auto_shrink([false, true])
                .show(ui, |ui| self.list(ui));
            ui.add_space(4.0);
            if let Some(n) = &self.notice {
                // A key refused: in use by another command.
                crate::callout::callout(ui, crate::callout::Tone::Warning, n);
                ui.add_space(4.0);
            }
            ui.separator();
            crate::app::dialog_button_row(ui, |ui| {
                ok = ui.button(tr!("OK")).clicked();
                cancel = ui.button(tr!("CANCEL")).clicked();
                crate::app::at_left_edge(ui, |ui| {
                    let changed = self.draft != Shortcuts::default();
                    if ui
                        .add_enabled(changed, egui::Button::new(tr!("RESET_ALL_SHORTCUTS")))
                        .clicked()
                    {
                        self.draft = Shortcuts::default();
                        self.set_conflicts(Vec::new());
                        self.recording = None;
                        self.notice = None;
                    }
                });
            });
        });
        // Esc while waiting for keys only stops waiting.
        if response.should_close() && self.recording.is_none() {
            cancel = true;
        }
        if ok {
            Outcome::Apply(self.draft.clone())
        } else if cancel {
            Outcome::Cancel
        } else {
            Outcome::Open
        }
    }

    /// Takes the keys pressed for the command waiting for them. Esc stops waiting.
    fn record(&mut self, ctx: &egui::Context) {
        let Some((command, index)) = self.recording else {
            return;
        };
        let pressed = ctx.input_mut(|i| {
            let mut pressed = None;
            i.events.retain(|e| {
                if pressed.is_some() {
                    return true;
                }
                match e {
                    // Waits for the key the modifiers are held with.
                    egui::Event::Key { key, .. } if is_modifier(*key) => false,
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => {
                        pressed = Some(if *key == Key::Escape {
                            None
                        } else {
                            Some(recorded(*key, *modifiers))
                        });
                        false
                    }
                    // Kept by Cut, Copy and Paste.
                    egui::Event::Cut => {
                        pressed = Some(Some(fixed(Command::Cut)[0]));
                        false
                    }
                    egui::Event::Copy => {
                        pressed = Some(Some(fixed(Command::Copy)[0]));
                        false
                    }
                    egui::Event::Paste(_) => {
                        pressed = Some(Some(fixed(Command::Paste)[0]));
                        false
                    }
                    _ => true,
                }
            });
            pressed
        });
        let Some(pressed) = pressed else {
            return;
        };
        self.recording = None;
        let Some(shortcut) = pressed else {
            return;
        };
        let name = ctx.format_shortcut(&shortcut);
        let result = self.draft.assign(command, index, shortcut);
        self.set_conflicts(result.err().map(|by| (by, shortcut)).into_iter().collect());
        self.notice = match result {
            Ok(()) => None,
            Err(by) if by.is_fixed() => Some(tr!(
                "SHORTCUT_KEPT_BY",
                shortcut = name,
                command = by.label()
            )),
            Err(by) => Some(tr!(
                "SHORTCUT_IN_USE",
                shortcut = name,
                command = by.label()
            )),
        };
    }

    /// The commands, grouped as in the menus, those matching the filter.
    fn list(&mut self, ui: &mut egui::Ui) {
        let filter = self.filter.trim().to_lowercase();
        let ctx = ui.ctx().clone();
        let matches = |c: Command| {
            filter.is_empty()
                || c.label().to_lowercase().contains(&filter)
                || self
                    .draft
                    .get(c)
                    .iter()
                    .any(|s| ctx.format_shortcut(s).to_lowercase().contains(&filter))
        };
        // A command with a refused key shows whatever the filter.
        let shown: Vec<Command> = Command::ALL
            .into_iter()
            .filter(|&c| matches(c) || self.conflicts.iter().any(|(k, _)| *k == c))
            .collect();
        if shown.is_empty() {
            ui.weak(tr!("NO_MATCHING_SHORTCUTS"));
            return;
        }
        for group in Group::ALL {
            let rows: Vec<Command> = shown
                .iter()
                .copied()
                .filter(|c| c.group() == group)
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            ui.strong(group.title());
            // Rows one against the other, for the stripes.
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (k, c) in rows.into_iter().enumerate() {
                    self.row(ui, c, k % 2 == 1);
                }
            });
        }
    }

    /// A command, in columns the same for all rows: its name; its keys (each one clicked
    /// to change it, right-clicked to remove it) and a button to add one; a button to
    /// remove them all; one to give it its default keys back.
    fn row(&mut self, ui: &mut egui::Ui, c: Command, striped: bool) {
        use crate::layers_ui::{Tool, tool_button, tool_icon};
        // Clear of the scroll bar, which floats over the right edge of the list.
        let bar = &ui.spacing().scroll;
        let room = bar.bar_width + bar.bar_inner_margin + bar.bar_outer_margin;
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(WINDOW_WIDTH - room, ROW), egui::Sense::hover());
        // The keys refused elsewhere because this command has them.
        let conflicting: Vec<KeyboardShortcut> = self
            .conflicts
            .iter()
            .filter(|(k, _)| *k == c)
            .map(|(_, s)| *s)
            .collect();
        let warn = ui
            .visuals()
            .warn_fg_color
            .gamma_multiply(self.conflict_strength);
        if striped {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().faint_bg_color);
        }
        if !conflicting.is_empty() {
            ui.painter()
                .rect_filled(rect, 3.0, warn.gamma_multiply(0.18));
            ui.painter().rect_stroke(
                rect,
                3.0,
                egui::Stroke::new(1.0, warn),
                egui::StrokeKind::Inside,
            );
            if std::mem::take(&mut self.scroll_to_conflict) {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
        }
        // A key in conflict is outlined too.
        let outline = |b: egui::Button<'static>, hit: bool| {
            if hit {
                b.stroke(egui::Stroke::new(1.5, warn))
            } else {
                b
            }
        };
        let gap = ui.spacing().item_spacing.x;
        let reset_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - ICON / 2.0, rect.center().y),
            egui::vec2(ICON, ICON),
        );
        let trash_rect = reset_rect.translate(egui::vec2(-(ICON + gap), 0.0));
        let label_rect = egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(4.0, 0.0),
            egui::pos2(rect.left() + LABEL_WIDTH, rect.bottom()),
        );
        let keys_rect = egui::Rect::from_min_max(
            egui::pos2(label_rect.right() + gap, rect.top()),
            egui::pos2(trash_rect.left() - gap, rect.bottom()),
        );
        // Inside the row, which alone takes room in the list.
        fn cell(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui)) {
            let builder = egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center));
            add(&mut ui.new_child(builder));
        }
        cell(ui, label_rect, |ui| {
            ui.add(egui::Label::new(c.label()).truncate());
        });
        let ctx = ui.ctx().clone();
        let keys = self.draft.get(c).to_vec();
        cell(ui, keys_rect, |ui| {
            if c.is_fixed() {
                // The arrows show as one button, in conflict when any of them is.
                let texts: Vec<(String, bool)> = if c == Command::Nudge {
                    vec![(tr!("ARROW_KEYS").to_owned(), !conflicting.is_empty())]
                } else {
                    keys.iter()
                        .map(|s| (ctx.format_shortcut(s), conflicting.contains(s)))
                        .collect()
                };
                for (t, hit) in texts {
                    ui.add_enabled(false, outline(egui::Button::new(t), hit))
                        .on_disabled_hover_text(tr!("SHORTCUT_FIXED_HINT"));
                }
                return;
            }
            for (k, s) in keys.iter().enumerate() {
                let waiting = self.recording == Some((c, Some(k)));
                let text = if waiting {
                    tr!("PRESS_SHORTCUT_KEYS").to_owned()
                } else {
                    ctx.format_shortcut(s)
                };
                let button = outline(egui::Button::new(text), conflicting.contains(s));
                let r = ui
                    .add(button.selected(waiting))
                    .on_hover_text(tr!("CHANGE_SHORTCUT_HINT"));
                if r.clicked() {
                    self.recording = if waiting { None } else { Some((c, Some(k))) };
                }
                let mut remove = false;
                r.context_menu(|ui| {
                    remove = ui.button(tr!("REMOVE_SHORTCUT")).clicked();
                });
                if remove {
                    self.draft.remove(c, k);
                    self.recording = None;
                }
            }
            if self.recording == Some((c, None)) {
                if ui
                    .add(egui::Button::new(tr!("PRESS_SHORTCUT_KEYS")).selected(true))
                    .clicked()
                {
                    self.recording = None;
                }
            } else if tool_button(ui, Tool::Add, true, tr!("ADD_SHORTCUT")) {
                self.recording = Some((c, None));
            }
        });
        if c.is_fixed() {
            return;
        }
        // Always in their columns, grayed out when they would do nothing.
        cell(ui, trash_rect, |ui| {
            let r = tool_icon(
                ui,
                Tool::Delete,
                !keys.is_empty(),
                tr!("REMOVE_ALL_SHORTCUTS"),
            );
            if !keys.is_empty() && r.clicked() {
                self.draft.keys.insert(c, Vec::new());
                self.recording = None;
            }
        });
        let changed = self.draft.get(c) != c.defaults();
        let defaults: Vec<String> = c
            .defaults()
            .iter()
            .map(|s| ctx.format_shortcut(s))
            .collect();
        let tip = if defaults.is_empty() {
            tr!("RESET_SHORTCUT_TO_NONE").to_owned()
        } else {
            tr!("RESET_SHORTCUT_HINT", keys = defaults.join(", "))
        };
        cell(ui, reset_rect, |ui| {
            if tool_icon(ui, Tool::Reset, changed, &tip).clicked() && changed {
                let kept = self.draft.reset(c);
                self.recording = None;
                self.set_conflicts(kept.iter().map(|(s, by)| (*by, *s)).collect());
                self.notice = (!kept.is_empty()).then(|| {
                    let list: Vec<String> = kept
                        .iter()
                        .map(|(s, by)| format!("{} ({})", ctx.format_shortcut(s), by.label()))
                        .collect();
                    tr!("DEFAULT_SHORTCUTS_IN_USE", shortcuts = list.join(", "))
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        }
    }

    /// The commands pressed in one frame with these keys held.
    fn fire(s: &Shortcuts, key: Key, modifiers: Modifiers, typing: bool) -> Vec<Command> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        let mut output = ctx.run_ui(press(key, modifiers), |ui| {
            out = s.pressed(ui.ctx(), typing);
        });
        output.textures_delta.clear();
        out
    }

    /// Ctrl as egui reports it outside macOS.
    const CTRL: Modifiers = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };

    #[test]
    fn shortcuts_match_their_modifiers_exactly() {
        let s = Shortcuts::default();
        assert_eq!(fire(&s, Key::Z, CTRL, false), [Command::Undo]);
        let ctrl_shift = Modifiers {
            shift: true,
            ..CTRL
        };
        assert_eq!(fire(&s, Key::Z, ctrl_shift, false), [Command::Redo]);
        assert_eq!(fire(&s, Key::Y, CTRL, false), [Command::Redo]);
        assert_eq!(fire(&s, Key::F, Modifiers::NONE, false), [Command::FitPage]);
        assert!(fire(&s, Key::F, Modifiers::SHIFT, false).is_empty());
        // Typing in a field: File commands still work, the others do not.
        assert_eq!(fire(&s, Key::S, CTRL, true), [Command::Save]);
        assert!(fire(&s, Key::Z, CTRL, true).is_empty());
        assert!(fire(&s, Key::F, Modifiers::NONE, true).is_empty());
    }

    #[test]
    fn a_plain_key_given_to_a_file_command_does_not_fire_while_typing() {
        let mut s = Shortcuts::default();
        assert_eq!(s.assign(Command::SaveAs, None, sc(NONE, Key::A)), Ok(()));
        assert_eq!(fire(&s, Key::A, Modifiers::NONE, false), [Command::SaveAs]);
        assert!(fire(&s, Key::A, Modifiers::NONE, true).is_empty());
    }

    #[test]
    fn a_key_in_use_is_refused() {
        let mut s = Shortcuts::default();
        let ctrl_t = sc(CMD, Key::T);
        assert_eq!(
            s.assign(Command::SaveAs, None, ctrl_t),
            Err(Command::AddText)
        );
        assert!(s.get(Command::SaveAs).is_empty());
        assert_eq!(s.get(Command::AddText), [ctrl_t]);
        // A command's own key again: nothing to do.
        assert_eq!(s.assign(Command::AddText, None, ctrl_t), Ok(()));
        assert_eq!(s.get(Command::AddText), [ctrl_t]);
        // Replacing one of several keys.
        assert_eq!(s.assign(Command::Redo, Some(1), sc(CMD, Key::U)), Ok(()));
        assert_eq!(
            s.get(Command::Redo),
            [sc(CMD_SHIFT, Key::Z), sc(CMD, Key::U)]
        );
        // Ctrl+C stays Copy's, the arrows nudge.
        assert_eq!(
            s.assign(Command::Quit, None, sc(CMD, Key::C)),
            Err(Command::Copy)
        );
        assert_eq!(
            s.assign(Command::Quit, None, sc(NONE, Key::ArrowUp)),
            Err(Command::Nudge)
        );
        assert!(s.get(Command::Quit).is_empty());
        // Removed from Add a text layer, Ctrl+T can go to Save As; Reset then gives Add a
        // text layer the default keys that are free, which is none.
        s.remove(Command::AddText, 0);
        assert_eq!(s.assign(Command::SaveAs, None, ctrl_t), Ok(()));
        assert_eq!(s.reset(Command::AddText), [(ctrl_t, Command::SaveAs)]);
        assert!(s.get(Command::AddText).is_empty());
        assert_eq!(s.get(Command::SaveAs), [ctrl_t]);
    }

    #[test]
    fn only_changed_commands_are_saved_and_unknown_ones_ignored() {
        let mut s = Shortcuts::default();
        assert!(s.to_saved().is_empty());
        assert_eq!(s.assign(Command::Quit, None, sc(CMD, Key::Q)), Ok(()));
        s.remove(Command::FitPage, 0);
        let mut saved = s.to_saved();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved["fit_page"], []);
        // From an older or newer version: unknown names and fixed commands are skipped.
        saved.insert("teleport".into(), vec![sc(CMD, Key::K)]);
        saved.insert("copy".into(), vec![sc(CMD, Key::K)]);
        // A modifier alone, saved by an earlier build, is dropped.
        saved
            .get_mut("fit_page")
            .unwrap()
            .push(sc(Modifiers::CTRL, Key::ControlLeft));
        let json = serde_json::to_string(&saved).unwrap();
        let back = Shortcuts::from_saved(serde_json::from_str(&json).unwrap());
        assert_eq!(back, s);
    }

    #[test]
    fn recorded_keys_use_the_command_modifier() {
        let s = recorded(Key::K, CTRL);
        assert_eq!(s, sc(CMD, Key::K));
        let mac_cmd = Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        };
        assert_eq!(recorded(Key::K, mac_cmd), sc(CMD, Key::K));
        assert_eq!(recorded(Key::K, Modifiers::ALT), sc(Modifiers::ALT, Key::K));
    }

    /// Draws the window, a key refused and another waiting for keys, to look at it:
    /// `SHORTCUTS_PREVIEW=out.png cargo test shortcuts_preview -- --ignored`.
    #[test]
    #[ignore]
    fn shortcuts_preview() {
        let Ok(path) = std::env::var("SHORTCUTS_PREVIEW") else {
            return;
        };
        let mut editor = Editor::new(&Shortcuts::default());
        let _ = editor
            .draft
            .assign(Command::SaveAs, None, sc(CMD_SHIFT, Key::S));
        editor.notice = Some(tr!(
            "SHORTCUT_IN_USE",
            shortcut = "Ctrl+D",
            command = Command::Duplicate.label()
        ));
        editor.set_conflicts(vec![(Command::Duplicate, sc(CMD, Key::D))]);
        editor.recording = Some((Command::Quit, None));
        let editor = std::cell::RefCell::new(editor);
        crate::preview::render(&path, egui::vec2(800.0, 900.0), |ui| {
            editor.borrow_mut().show(ui.ctx());
        });
    }

    #[test]
    fn the_window_records_keys_and_ok_applies_them() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new(&Shortcuts::default());
        editor.recording = Some((Command::Quit, None));
        let frame = |editor: &mut Editor, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            let mut outcome = None;
            let mut output = ctx.run_ui(input, |ui| {
                outcome = Some(editor.show(ui.ctx()));
            });
            output.textures_delta.clear();
            outcome.unwrap()
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        // Ctrl pressed first comes as a key of its own: the window waits for Q.
        frame(&mut editor, vec![key(Key::ControlLeft, CTRL)]);
        assert!(editor.draft.get(Command::Quit).is_empty());
        assert_eq!(editor.recording, Some((Command::Quit, None)));
        frame(&mut editor, vec![key(Key::Q, CTRL)]);
        assert_eq!(editor.draft.get(Command::Quit), [sc(CMD, Key::Q)]);
        assert!(editor.recording.is_none());
        // Esc while waiting stops waiting, without closing the window.
        editor.recording = Some((Command::Print, Some(0)));
        let outcome = frame(&mut editor, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(editor.recording.is_none());
        assert_eq!(editor.draft.get(Command::Print), [sc(CMD, Key::P)]);
        assert!(matches!(outcome, Outcome::Open));
        // A key in use is refused, saying by which command.
        editor.recording = Some((Command::Quit, Some(0)));
        frame(&mut editor, vec![key(Key::P, CTRL)]);
        assert_eq!(editor.draft.get(Command::Print), [sc(CMD, Key::P)]);
        assert_eq!(editor.draft.get(Command::Quit), [sc(CMD, Key::Q)]);
        assert!(editor.notice.as_deref().unwrap().contains("Print"));
        // Print's row is highlighted, shown even when the filter leaves it out.
        assert_eq!(editor.conflicts, [(Command::Print, sc(CMD, Key::P))]);
        editor.filter = "quit".into();
        let shown = |editor: &mut Editor| {
            let mut out = Vec::new();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                editor.show(ui.ctx());
            });
            output.textures_delta.clear();
            for c in output.shapes {
                if let egui::Shape::Text(t) = c.shape {
                    out.push(t.galley.text().to_owned());
                }
            }
            out
        };
        assert!(shown(&mut editor).iter().any(|t| t == "Print…"));
        // The highlight fades out after a moment; the warning stays.
        let at = |editor: &mut Editor, time: f64| {
            let input = egui::RawInput {
                time: Some(time),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                editor.show(ui.ctx());
            });
            output.textures_delta.clear();
        };
        editor.set_conflicts(vec![(Command::Print, sc(CMD, Key::P))]);
        at(&mut editor, 100.0);
        assert_eq!(editor.conflict_strength, 1.0);
        at(&mut editor, 100.0 + CONFLICT_HOLD + CONFLICT_FADE / 2.0);
        assert!((editor.conflict_strength - 0.5).abs() < 0.01);
        assert_eq!(editor.conflicts.len(), 1);
        at(&mut editor, 100.0 + CONFLICT_HOLD + CONFLICT_FADE);
        assert!(editor.conflicts.is_empty());
        assert!(editor.notice.is_some());
        assert!(!shown(&mut editor).iter().any(|t| t == "Print…"));
        // Freeing the key ends the highlight and the warning at once.
        editor.set_conflicts(vec![(Command::Print, sc(CMD, Key::P))]);
        editor.draft.remove(Command::Print, 0);
        assert!(!shown(&mut editor).iter().any(|t| t == "Print…"));
        assert!(editor.conflicts.is_empty());
        assert!(editor.notice.is_none());
    }
}
