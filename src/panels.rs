//! Where the panels go: docked on the left or right of the window in any order, floating
//! over the canvas, or each in a window of its own (to put it on another screen). The
//! layout is kept between sessions.
//!
//! A panel is moved by dragging its title: onto a side to dock it there (a line shows
//! where it goes among the panels of that side), anywhere else to float it. Its menu (the
//! button at the right of its title) docks it, floats it, opens it in its own window or
//! hides it; View > Panels shows hidden panels again and resets the layout. The line
//! between two docked panels is dragged to share the side's height differently.
//!
//! The app draws the panels' contents; this module keeps the layout, draws the titles and
//! handles the drag.

use std::collections::HashMap;

use egui::{Color32, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::i18n::{tr, tr_noop, translate};

/// Key in eframe's storage.
const STORAGE_KEY: &str = "panel_layout";
/// The smallest height a docked panel's contents get, when the side is crowded.
const MIN_BODY: f32 = 60.0;
/// Height of the line between two docked panels, dragged to resize them.
const SPLITTER: f32 = 9.0;
/// Width of the zone at an empty side that a panel is dropped on to dock it there.
const EDGE_ZONE: f32 = 48.0;
/// Size of a panel floated for the first time.
const FLOAT_SIZE: Vec2 = vec2(320.0, 420.0);

/// The panels, each with its title and contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelId {
    Controller,
    Page,
    Template,
    Variables,
    ColorsInUse,
    Layers,
    Properties,
}

impl PanelId {
    /// In the order of the default layout and of View > Panels.
    pub const ALL: [PanelId; 7] = [
        PanelId::Controller,
        PanelId::Page,
        PanelId::Template,
        PanelId::Variables,
        PanelId::ColorsInUse,
        PanelId::Layers,
        PanelId::Properties,
    ];

    pub fn title(self) -> &'static str {
        translate(match self {
            PanelId::Controller => tr_noop!("CONTROLLER"),
            PanelId::Page => tr_noop!("PAGE_SECTION"),
            PanelId::Template => tr_noop!("TEMPLATE_SECTION"),
            PanelId::Variables => tr_noop!("VARIABLES_SECTION"),
            PanelId::ColorsInUse => tr_noop!("COLORS_IN_USE_SECTION"),
            PanelId::Layers => tr_noop!("LAYERS_SECTION"),
            PanelId::Properties => tr_noop!("PROPERTIES_SECTION"),
        })
    }

    /// The narrowest width its contents fit in. The layer toolbar and the rows of
    /// Properties need 310 points.
    pub fn min_width(self) -> f32 {
        match self {
            PanelId::Layers | PanelId::Properties => 310.0,
            _ => 240.0,
        }
    }

    /// The side it is docked on in the default layout.
    fn default_side(self) -> Side {
        match self {
            PanelId::Layers | PanelId::Properties => Side::Right,
            _ => Side::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];

    fn index(self) -> usize {
        self as usize
    }

    /// Width of the side panel before the user resizes it.
    pub fn default_width(self) -> f32 {
        match self {
            Side::Left => 240.0,
            Side::Right => 320.0,
        }
    }
}

/// Where a panel is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    Docked(Side),
    /// Over the canvas, in the app's window.
    Floating,
    /// In a window of its own.
    Window,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    id: PanelId,
    place: Place,
    /// Shown, or hidden from View > Panels or its menu.
    open: bool,
    /// Docked, only its title shows.
    folded: bool,
    /// Docked, the height its contents were given by dragging the line below it; `None`
    /// for the height they need.
    #[serde(default)]
    height: Option<f32>,
    /// Where it floats over the canvas: x, y, width, height.
    float_rect: Option<[f32; 4]>,
    /// Where its own window is on the screen, frame included.
    window_rect: Option<[f32; 4]>,
}

/// What a dragged panel is dropped on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// A side, above the given panel docked there, or below the last one.
    Dock(Side, Option<PanelId>),
    /// Over the canvas, its top-left corner there.
    Float(Pos2),
    /// A window of its own.
    Window,
}

/// What a panel's title does in each place.
#[derive(Clone, Copy, PartialEq)]
pub enum Header {
    /// Folds the panel; dragging it moves the panel to another place.
    Docked,
    /// Dragging it moves the floating panel; dropped on a side, it docks there.
    Floating,
    /// Its own window: only the menu moves it.
    Window,
}

/// A panel being dragged by its title.
#[derive(Clone, Copy, Debug)]
struct Drag {
    id: PanelId,
    /// From the panel's top-left corner to the pointer.
    grab: Vec2,
    /// The panel's size when the drag started, for the outline that follows the pointer.
    size: Vec2,
    from: Place,
}

/// What was measured this frame, to lay out the next one.
struct Frame {
    /// The area the side panels and the canvas share.
    area: Rect,
    /// Each side panel, when shown.
    docks: [Option<Rect>; 2],
    /// Each docked panel, title and contents, top to bottom.
    slots: Vec<(Side, PanelId, Rect)>,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            area: Rect::NOTHING,
            docks: [None; 2],
            slots: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Runtime {
    /// The height each panel's contents would take without a limit.
    natural: HashMap<PanelId, f32>,
    /// Height of each side used by titles and spacing, besides the contents.
    overhead: [f32; 2],
    this: Frame,
    last: Frame,
    drag: Option<Drag>,
    /// Floating panels to put at a place on the next frame.
    move_to: HashMap<PanelId, Pos2>,
    /// The rectangle each own window was opened with. The window is then left where
    /// the user puts it: a changed position would move it back.
    spawn: HashMap<PanelId, [f32; 4]>,
}

#[derive(Serialize, Deserialize)]
pub struct Layout {
    /// In order: docked panels are shown top to bottom in this order.
    entries: Vec<Entry>,
    #[serde(skip)]
    runtime: Runtime,
}

impl Default for Layout {
    /// Controller, Page, Template, Variables and Colors in use on the left; Layers and
    /// Properties on the right.
    fn default() -> Self {
        Self {
            entries: PanelId::ALL
                .iter()
                .map(|&id| Entry {
                    id,
                    place: Place::Docked(id.default_side()),
                    open: true,
                    folded: false,
                    height: None,
                    float_rect: None,
                    window_rect: None,
                })
                .collect(),
            runtime: Runtime::default(),
        }
    }
}

impl Layout {
    /// The layout saved by a previous session, or the default one.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut layout: Self = storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default();
        layout.normalize();
        layout
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, self);
    }

    /// Each panel once, those a newer version added in their default place.
    fn normalize(&mut self) {
        let mut seen = Vec::new();
        self.entries.retain(|e| {
            let new = !seen.contains(&e.id);
            seen.push(e.id);
            new
        });
        for e in Layout::default().entries {
            if !seen.contains(&e.id) {
                self.entries.push(e);
            }
        }
    }

    pub fn reset(&mut self) {
        let runtime = std::mem::take(&mut self.runtime);
        *self = Self {
            runtime,
            ..Self::default()
        };
        self.runtime.spawn.clear();
        self.runtime.move_to.clear();
    }

    fn entry(&self, id: PanelId) -> &Entry {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .expect("every panel has an entry")
    }

    fn entry_mut(&mut self, id: PanelId) -> &mut Entry {
        self.entries
            .iter_mut()
            .find(|e| e.id == id)
            .expect("every panel has an entry")
    }

    fn place(&self, id: PanelId) -> Place {
        self.entry(id).place
    }

    pub fn is_open(&self, id: PanelId) -> bool {
        self.entry(id).open
    }

    pub fn set_open(&mut self, id: PanelId, open: bool) {
        self.entry_mut(id).open = open;
    }

    fn is_folded(&self, id: PanelId) -> bool {
        self.entry(id).folded
    }

    /// The panels shown on a side, top to bottom.
    pub fn docked(&self, side: Side) -> Vec<PanelId> {
        self.entries
            .iter()
            .filter(|e| e.open && e.place == Place::Docked(side))
            .map(|e| e.id)
            .collect()
    }

    /// The panels shown in `place` (floating or in their own window).
    fn shown_in(&self, place: Place) -> Vec<PanelId> {
        self.entries
            .iter()
            .filter(|e| e.open && e.place == place)
            .map(|e| e.id)
            .collect()
    }

    /// Puts a panel somewhere, and shows it.
    pub fn move_to(&mut self, id: PanelId, target: Target) {
        let i = self.entries.iter().position(|e| e.id == id).unwrap();
        let mut e = self.entries.remove(i);
        e.open = true;
        // The height it was given suits the panels it was with.
        if !matches!(target, Target::Dock(side, _) if e.place == Place::Docked(side)) {
            e.height = None;
        }
        match target {
            Target::Dock(side, before) => {
                e.place = Place::Docked(side);
                // Above `before`, or below the last panel of that side.
                let at = before
                    .and_then(|b| self.entries.iter().position(|x| x.id == b))
                    .or_else(|| {
                        self.entries
                            .iter()
                            .rposition(|x| x.place == Place::Docked(side))
                            .map(|k| k + 1)
                    })
                    .unwrap_or(self.entries.len());
                self.entries.insert(at, e);
            }
            Target::Float(pos) => {
                e.place = Place::Floating;
                let size = e.float_rect.map_or(FLOAT_SIZE, |r| vec2(r[2], r[3]));
                e.float_rect = Some([pos.x, pos.y, size.x, size.y]);
                self.runtime.move_to.insert(id, pos);
                self.entries.insert(i, e);
            }
            Target::Window => {
                e.place = Place::Window;
                self.runtime.spawn.remove(&id);
                self.entries.insert(i, e);
            }
        }
    }

    /// Starts a frame: what was measured on the last one is used to lay out this one.
    /// `area` is what the side panels and the canvas share.
    pub fn begin_frame(&mut self, area: Rect) {
        let r = &mut self.runtime;
        r.last = std::mem::take(&mut r.this);
        r.this.area = area;
    }

    /// Height of each docked panel's contents on a side `height` high, `None` for a
    /// folded one. A panel resized by dragging the line below it gets that height (no
    /// more than it needs). The others get the height they need when they all fit;
    /// otherwise the small ones still do, and the others share what is left equally and
    /// scroll.
    fn heights(&self, side: Side, height: f32, ids: &[PanelId]) -> Vec<(PanelId, Option<f32>)> {
        let unfolded: Vec<PanelId> = ids
            .iter()
            .copied()
            .filter(|&id| !self.is_folded(id))
            .collect();
        let available = (height - self.runtime.overhead[side.index()]).max(0.0);
        let natural: Vec<f32> = unfolded
            .iter()
            .map(|id| {
                self.runtime
                    .natural
                    .get(id)
                    .copied()
                    .unwrap_or(f32::INFINITY)
            })
            .collect();
        let set: Vec<Option<f32>> = unfolded.iter().map(|&id| self.entry(id).height).collect();
        let shares = share_set(&natural, &set, available, MIN_BODY);
        ids.iter()
            .map(|&id| {
                let h = unfolded.iter().position(|&u| u == id).map(|k| shares[k]);
                (id, h)
            })
            .collect()
    }

    /// The height a panel's contents took, unlimited, to lay out the next frame. Asks
    /// for another frame when it changed.
    fn set_natural(&mut self, ctx: &egui::Context, id: PanelId, height: f32) {
        let before = self.runtime.natural.insert(id, height);
        if before.is_none_or(|h| (h - height).abs() > 0.5) {
            ctx.request_repaint();
        }
    }

    /// The height a side used besides the panels' contents, measured after drawing it.
    fn set_overhead(&mut self, ctx: &egui::Context, side: Side, height: f32) {
        let o = &mut self.runtime.overhead[side.index()];
        if (*o - height).abs() > 0.5 {
            *o = height;
            ctx.request_repaint();
        }
    }

    /// Notes where a side panel was drawn, for drops.
    fn set_dock_rect(&mut self, side: Side, rect: Rect) {
        self.runtime.this.docks[side.index()] = Some(rect);
    }

    /// Notes where a docked panel was drawn, for drops.
    fn set_slot(&mut self, side: Side, id: PanelId, rect: Rect) {
        self.runtime.this.slots.push((side, id, rect));
    }

    /// The floating panel's rectangle to show it at: where it was dragged or dropped,
    /// once (`true`), or where it was last.
    fn float_rect(&mut self, id: PanelId) -> (Rect, bool) {
        let size = self
            .entry(id)
            .float_rect
            .map_or(FLOAT_SIZE, |r| vec2(r[2], r[3]));
        if let Some(p) = self.runtime.move_to.remove(&id) {
            return (Rect::from_min_size(p, size), true);
        }
        let rect = self.entry(id).float_rect.map_or_else(
            || {
                let area = self.runtime.last.area;
                Rect::from_center_size(area.center(), size)
            },
            |r| Rect::from_min_size(pos2(r[0], r[1]), size),
        );
        (rect, false)
    }

    /// Where a floating panel was shown, kept for the next session.
    fn set_float_rect(&mut self, id: PanelId, rect: Rect) {
        self.entry_mut(id).float_rect = Some([rect.min.x, rect.min.y, rect.width(), rect.height()]);
    }

    /// The position and size to open a panel's own window with. They stay the same
    /// while it is open, so the window is not moved back.
    fn window_spawn(&mut self, id: PanelId) -> (Option<Pos2>, Vec2) {
        let saved = self.entry(id).window_rect;
        let fallback = self.entry(id).float_rect;
        let r = *self.runtime.spawn.entry(id).or_insert_with(|| {
            saved
                .or(fallback)
                .unwrap_or([f32::NAN, f32::NAN, FLOAT_SIZE.x, FLOAT_SIZE.y])
        });
        let pos = (r[0].is_finite() && r[1].is_finite()).then(|| pos2(r[0], r[1]));
        (pos, vec2(r[2], r[3]))
    }

    /// Where the user put a panel's own window, kept for the next session.
    fn set_window_rect(&mut self, id: PanelId, outer: Option<Rect>, inner_size: Option<Vec2>) {
        let e = self.entry_mut(id);
        let old = e
            .window_rect
            .unwrap_or([f32::NAN, f32::NAN, FLOAT_SIZE.x, FLOAT_SIZE.y]);
        let pos = outer.map_or([old[0], old[1]], |r| [r.min.x, r.min.y]);
        let size = inner_size.map_or([old[2], old[3]], |s| [s.x, s.y]);
        e.window_rect = Some([pos[0], pos[1], size[0], size[1]]);
    }

    /// The panel's title row: a fold arrow when docked, the title (dragged to move the
    /// panel), its menu and, outside the sides, a button to hide it.
    fn header(&mut self, ui: &mut Ui, id: PanelId, kind: Header, panel_rect: Rect) {
        let multi = !ui.ctx().embed_viewports();
        ui.horizontal(|ui| {
            if kind == Header::Docked {
                let folded = self.is_folded(id);
                if fold_arrow(ui, folded).clicked() {
                    self.entry_mut(id).folded = !folded;
                }
            }
            let sense = if kind == Header::Window {
                Sense::hover()
            } else {
                Sense::click_and_drag()
            };
            let right = 2.0 * (ICON + ui.spacing().item_spacing.x);
            let width = (ui.available_width() - right).max(0.0);
            let title = egui::RichText::new(id.title()).heading();
            let galley = egui::WidgetText::from(title).into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                width,
                egui::TextStyle::Heading,
            );
            let (rect, r) = ui.allocate_exact_size(vec2(width, galley.size().y), sense);
            ui.painter()
                .galley(rect.left_top(), galley, ui.visuals().text_color());
            if kind != Header::Window {
                let r = r.on_hover_text(tr!("PANEL_DRAG_HINT"));
                // As on the sections inside the panels, a click on the title folds it.
                if kind == Header::Docked && r.clicked() {
                    let e = self.entry_mut(id);
                    e.folded = !e.folded;
                }
                if r.hovered() && !r.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
                self.title_drag(ui, id, kind, &r, panel_rect);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kind != Header::Docked && close_button(ui).clicked() {
                    self.set_open(id, false);
                }
                let menu = menu_button(ui);
                egui::Popup::menu(&menu).show(|ui| self.panel_menu(ui, id, multi));
            });
        });
    }

    /// Dock left / Dock right / Float / Own window / Hide.
    fn panel_menu(&mut self, ui: &mut Ui, id: PanelId, multi: bool) {
        let place = self.place(id);
        let item = |ui: &mut Ui, enabled: bool, key: &str| {
            ui.add_enabled(enabled, egui::Button::new(translate(key)))
                .clicked()
        };
        let mut target = None;
        if item(
            ui,
            place != Place::Docked(Side::Left),
            tr_noop!("DOCK_PANEL_LEFT"),
        ) {
            target = Some(Target::Dock(Side::Left, None));
        }
        if item(
            ui,
            place != Place::Docked(Side::Right),
            tr_noop!("DOCK_PANEL_RIGHT"),
        ) {
            target = Some(Target::Dock(Side::Right, None));
        }
        if item(ui, place != Place::Floating, tr_noop!("FLOAT_PANEL")) {
            let area = self.runtime.last.area;
            let size = self
                .entry(id)
                .float_rect
                .map_or(FLOAT_SIZE, |r| vec2(r[2], r[3]));
            target = Some(Target::Float(area.center() - size / 2.0));
        }
        // Only where the platform can open more windows.
        if multi && item(ui, place != Place::Window, tr_noop!("PANEL_OWN_WINDOW")) {
            target = Some(Target::Window);
        }
        ui.separator();
        if item(ui, true, tr_noop!("HIDE_PANEL")) {
            self.set_open(id, false);
        }
        if let Some(t) = target {
            self.move_to(id, t);
            ui.ctx().request_repaint();
        }
    }

    /// Starts, follows and ends a drag of a panel's title.
    fn title_drag(&mut self, ui: &Ui, id: PanelId, kind: Header, r: &Response, panel_rect: Rect) {
        let ctx = ui.ctx();
        // From where the button was pressed: the drag starts once the pointer has moved.
        if r.drag_started()
            && let Some(p) = ctx.input(|i| i.pointer.press_origin())
        {
            self.runtime.drag = Some(Drag {
                id,
                grab: p - panel_rect.min,
                size: panel_rect.size(),
                from: self.place(id),
            });
        }
        let Some(drag) = self.runtime.drag.filter(|d| d.id == id) else {
            return;
        };
        if r.dragged() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            // A floating panel moves with the pointer.
            if kind == Header::Floating
                && let Some(p) = ctx.pointer_interact_pos()
            {
                self.runtime.move_to.insert(id, p - drag.grab);
            }
        }
    }

    /// Where a panel dropped at `p` goes: a side when over it (or over the zone at an
    /// empty side's edge), above the panel whose middle is below `p`.
    pub fn drop_target(&self, p: Pos2) -> Option<(Side, Option<PanelId>, f32)> {
        let last = &self.runtime.last;
        for side in Side::BOTH {
            let zone = last.docks[side.index()].unwrap_or_else(|| edge_zone(last.area, side));
            if !zone.contains(p) {
                continue;
            }
            let slots: Vec<_> = last.slots.iter().filter(|(s, ..)| *s == side).collect();
            let before = slots.iter().find(|(_, _, r)| r.center().y > p.y);
            let y = match (before, slots.last()) {
                (Some((_, _, r)), _) => r.top(),
                (None, Some((_, _, r))) => r.bottom(),
                (None, None) => zone.top(),
            };
            return Some((side, before.map(|(_, id, _)| *id), y));
        }
        None
    }

    /// While a panel is dragged: the drop zones, the line where it would be docked or
    /// the outline of where it would float. Drops it when the button is released. Call
    /// once a frame, after the panels.
    pub fn drag_overlay(&mut self, ctx: &egui::Context) {
        let Some(drag) = self.runtime.drag else {
            return;
        };
        let Some(p) = ctx.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let target = self.drop_target(p);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::unique("panel_drop"),
        ));
        let visuals = ctx.global_style().visuals.clone();
        let accent = visuals.selection.stroke.color;
        let fill = accent.gamma_multiply(0.15);
        let last = &self.runtime.last;
        // Zones at the sides with no panel.
        for side in Side::BOTH {
            if last.docks[side.index()].is_none() {
                let zone = edge_zone(last.area, side);
                let over = target.is_some_and(|t| t.0 == side);
                painter.rect_filled(
                    zone,
                    0.0,
                    if over { fill } else { fill.gamma_multiply(0.5) },
                );
                painter.rect_stroke(
                    zone,
                    0.0,
                    Stroke::new(1.0, accent),
                    egui::StrokeKind::Inside,
                );
            }
        }
        match target {
            Some((side, _, y)) => {
                let zone = last.docks[side.index()].unwrap_or_else(|| edge_zone(last.area, side));
                painter.rect_filled(zone, 0.0, fill);
                painter.hline(zone.x_range(), y, Stroke::new(3.0, accent));
            }
            // A docked panel would float here.
            None if drag.from != Place::Floating => {
                let rect = Rect::from_min_size(p - drag.grab, drag.size.min(FLOAT_SIZE));
                painter.rect_filled(rect, 4.0, fill);
                painter.rect_stroke(
                    rect,
                    4.0,
                    Stroke::new(1.5, accent),
                    egui::StrokeKind::Inside,
                );
            }
            None => {}
        }
        if ctx.input(|i| !i.pointer.primary_down()) {
            self.runtime.drag = None;
            match target {
                Some((side, before, _)) => {
                    if before != Some(drag.id) {
                        self.move_to(drag.id, Target::Dock(side, before));
                    }
                }
                None if drag.from != Place::Floating => {
                    self.move_to(drag.id, Target::Float(p - drag.grab));
                }
                None => {}
            }
            ctx.request_repaint();
        } else {
            ctx.request_repaint();
        }
    }
}

/// What the app puts in the panels.
pub trait Contents {
    /// Draws a panel's contents. A panel that scrolls on its own (see
    /// [`PanelId::scrolls_itself`]) fits in the height `ui` has, and returns the height
    /// it would take without a limit.
    fn contents(&mut self, ui: &mut Ui, id: PanelId) -> Option<f32>;

    /// Whether a docked panel is left out for now (the Template panel, until there is
    /// a template).
    fn skip_docked(&self, _id: PanelId) -> bool {
        false
    }

    /// Called on each frame of a panel's own window, e.g. for keyboard shortcuts.
    fn window_frame(&mut self, _ctx: &egui::Context) {}
}

impl PanelId {
    /// Its contents have a part that scrolls (the layer list) and one that does not
    /// (the toolbar above it), so they are not put in a scroll area.
    pub fn scrolls_itself(self) -> bool {
        self == PanelId::Layers
    }
}

impl Layout {
    /// A panel's contents `height` high at most (all the height left when `None`). They
    /// scroll when taller.
    fn contents(&mut self, ui: &mut Ui, app: &mut dyn Contents, id: PanelId, height: Option<f32>) {
        let height = height.unwrap_or_else(|| ui.available_height());
        let natural = if id.scrolls_itself() {
            let size = vec2(ui.available_width(), height);
            ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_max_height(height);
                app.contents(ui, id)
            })
            .inner
            .unwrap_or(height)
        } else {
            egui::ScrollArea::vertical()
                .id_salt(("panel", id))
                .max_height(height)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    app.contents(ui, id);
                })
                .content_size
                .y
        };
        self.set_natural(ui.ctx(), id, natural);
    }

    /// The panels docked on a side, top to bottom, sharing its height. Nothing when it
    /// has none.
    pub fn show_dock(&mut self, ui: &mut Ui, side: Side, app: &mut dyn Contents) {
        let shown: Vec<PanelId> = self
            .docked(side)
            .into_iter()
            .filter(|&id| !app.skip_docked(id))
            .collect();
        if shown.is_empty() {
            return;
        }
        // The ids keep the widths the side panels had before panels could move.
        let panel = match side {
            Side::Left => egui::Panel::left("settings"),
            Side::Right => egui::Panel::right("layers"),
        };
        let min_width = shown.iter().map(|id| id.min_width()).fold(0.0, f32::max);
        let response = panel
            .resizable(true)
            .default_size(side.default_width())
            .min_size(min_width)
            .show(ui, |ui| {
                let top = ui.cursor().top();
                let mut contents = 0.0;
                let heights = self.heights(side, ui.available_height(), &shown);
                for (k, &(id, height)) in heights.iter().enumerate() {
                    let start = ui.cursor().top();
                    if k > 0 {
                        let (above, above_height) = heights[k - 1];
                        self.splitter(ui, above_height.map(|h| (above, h)), (id, height));
                    }
                    // The panel's size is known once drawn: as wide as the side, and a
                    // guess at its height for the outline that follows a drag.
                    let guess =
                        Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 200.0));
                    self.header(ui, id, Header::Docked, guess);
                    if let Some(h) = height {
                        let before = ui.cursor().top();
                        self.contents(ui, app, id, Some(h));
                        contents += ui.cursor().top() - before;
                    }
                    let rect =
                        Rect::from_x_y_ranges(ui.max_rect().x_range(), start..=ui.cursor().top());
                    self.set_slot(side, id, rect);
                }
                let used = ui.cursor().top() - top;
                self.set_overhead(ui.ctx(), side, used - contents);
            });
        self.set_dock_rect(side, response.response.rect);
    }

    /// The line between two docked panels. Dragged, it resizes the panel above (and the
    /// one below, when it was resized too, so its bottom stays); double-clicked, the panel
    /// above gets the height it needs again. `above` is the panel above and its contents'
    /// height, `None` when folded: the line is then only a separator.
    fn splitter(
        &mut self,
        ui: &mut Ui,
        above: Option<(PanelId, f32)>,
        below: (PanelId, Option<f32>),
    ) {
        let sense = if above.is_some() {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), SPLITTER), sense);
        let active = above.is_some() && (r.hovered() || r.dragged());
        let stroke = if active {
            ui.visuals().selection.stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        ui.painter().hline(rect.x_range(), rect.center().y, stroke);
        let Some((above, shown)) = above else {
            return;
        };
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let r = if r.dragged() {
            r
        } else {
            r.on_hover_text(tr!("PANEL_SPLITTER_HINT"))
        };
        if r.double_clicked() {
            self.entry_mut(above).height = None;
            ui.ctx().request_repaint();
        } else if r.dragged() {
            let mut h = (shown + r.drag_delta().y).max(MIN_BODY);
            // The panel below, resized too, gives or takes what the one above takes or
            // gives, so the panels further down stay.
            if let (b, Some(below_shown)) = below
                && self.entry(b).height.is_some()
            {
                let b_h = (below_shown - (h - shown)).max(MIN_BODY);
                h = shown + below_shown - b_h;
                self.entry_mut(b).height = Some(b_h);
            }
            self.entry_mut(above).height = Some(h);
            ui.ctx().request_repaint();
        }
    }

    /// The panels floating over the canvas.
    pub fn show_floating(&mut self, ctx: &egui::Context, app: &mut dyn Contents) {
        for id in self.shown_in(Place::Floating) {
            let (rect, moved) = self.float_rect(id);
            let mut window = egui::Window::new(id.title())
                .id(Id::unique(("floating_panel", id)))
                .title_bar(false)
                .movable(false)
                .resizable(true)
                .min_width(id.min_width())
                .min_height(80.0)
                .default_rect(rect);
            if moved {
                window = window.current_pos(rect.min);
            }
            let shown = window.show(ctx, |ui| {
                let rect = ui.max_rect();
                self.header(ui, id, Header::Floating, rect);
                ui.separator();
                self.contents(ui, app, id, None);
            });
            if let Some(shown) = shown {
                self.set_float_rect(id, shown.response.rect);
            }
        }
    }

    /// The panels in windows of their own. Closing one hides it.
    pub fn show_windows(&mut self, ctx: &egui::Context, app: &mut dyn Contents) {
        for id in self.shown_in(Place::Window) {
            let (pos, size) = self.window_spawn(id);
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("{} - Artwork Creator", id.title()))
                .with_inner_size(size)
                .with_min_inner_size([id.min_width(), 120.0]);
            if let Some(pos) = pos {
                builder = builder.with_position(pos);
            }
            let viewport = egui::ViewportId::from_hash_of(("panel_window", id));
            ctx.show_viewport_immediate(viewport, builder, |ui, _class| {
                let ctx = ui.ctx().clone();
                app.window_frame(&ctx);
                egui::CentralPanel::default().show(ui, |ui| {
                    let rect = ui.max_rect();
                    self.header(ui, id, Header::Window, rect);
                    ui.separator();
                    self.contents(ui, app, id, None);
                });
                let (outer, inner, close) = ctx.input(|i| {
                    let v = i.viewport();
                    (
                        v.outer_rect,
                        v.inner_rect.map(|r| r.size()),
                        v.close_requested(),
                    )
                });
                self.set_window_rect(id, outer, inner);
                if close {
                    self.set_open(id, false);
                }
            });
        }
    }
}

/// Heights for contents that would take `natural`, sharing `available`: all of it when
/// they fit, otherwise each one needing less than an equal share of what is left gets
/// what it needs, the others that share. Never below `min`.
pub fn share(natural: &[f32], available: f32, min: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..natural.len()).collect();
    order.sort_by(|&a, &b| natural[a].total_cmp(&natural[b]));
    let mut out = vec![0.0; natural.len()];
    let mut left = available;
    for (k, &i) in order.iter().enumerate() {
        let share = left / (order.len() - k) as f32;
        let h = natural[i].min(share);
        out[i] = h.max(min);
        left -= h;
    }
    out
}

/// Heights for contents that would take `natural`, sharing `available`, some of them
/// resized to `set`: those get it, top to bottom, no more than they need and leaving `min`
/// for each other one. The others share what is left as [`share`] does.
pub fn share_set(natural: &[f32], set: &[Option<f32>], available: f32, min: f32) -> Vec<f32> {
    let mut out = vec![0.0; natural.len()];
    let mut left = available;
    let mut pending = natural.len();
    for (i, s) in set.iter().enumerate() {
        if let Some(s) = *s {
            pending -= 1;
            let h = s.min(natural[i]).min(left - min * pending as f32).max(min);
            out[i] = h;
            left -= h;
        }
    }
    let free: Vec<usize> = (0..natural.len()).filter(|&i| set[i].is_none()).collect();
    let wanted: Vec<f32> = free.iter().map(|&i| natural[i]).collect();
    for (&i, h) in free.iter().zip(share(&wanted, left.max(0.0), min)) {
        out[i] = h;
    }
    out
}

/// The zone at the edge of `area` a panel is dropped on to dock it on an empty side.
fn edge_zone(area: Rect, side: Side) -> Rect {
    match side {
        Side::Left => Rect::from_min_max(area.min, pos2(area.left() + EDGE_ZONE, area.bottom())),
        Side::Right => Rect::from_min_max(pos2(area.right() - EDGE_ZONE, area.top()), area.max),
    }
}

const ICON: f32 = 20.0;

/// A small flat button, its icon drawn by `paint` in the given color.
fn icon_button(
    ui: &mut Ui,
    tip: &str,
    paint: impl FnOnce(&egui::Painter, Rect, Color32),
) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    let r = r.on_hover_text(tip);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&r);
        if r.hovered() || r.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
        }
        paint(ui.painter(), rect.shrink(5.0), visuals.fg_stroke.color);
    }
    r
}

/// The panel's menu button: three lines.
fn menu_button(ui: &mut Ui) -> Response {
    icon_button(ui, tr!("PANEL_MENU_HINT"), |p, r, c| {
        let stroke = Stroke::new(1.5, c);
        for k in 0..3 {
            let y = r.top() + 1.0 + k as f32 * (r.height() - 2.0) / 2.0;
            p.hline(r.x_range(), y, stroke);
        }
    })
}

/// The button hiding a floating panel: a cross.
fn close_button(ui: &mut Ui) -> Response {
    icon_button(ui, tr!("HIDE_PANEL"), |p, r, c| {
        let stroke = Stroke::new(1.5, c);
        p.line_segment([r.left_top(), r.right_bottom()], stroke);
        p.line_segment([r.right_top(), r.left_bottom()], stroke);
    })
}

/// The arrow folding a docked panel, as on the sections inside the panels.
fn fold_arrow(ui: &mut Ui, folded: bool) -> Response {
    let size = ui.spacing().icon_width;
    let (_, r) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let openness = if folded { 0.0 } else { 1.0 };
    egui::collapsing_header::paint_default_icon(ui, openness, &r);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_panels_get_what_they_need_and_big_ones_share() {
        // All fit.
        assert_eq!(share(&[50.0, 100.0], 400.0, 10.0), vec![50.0, 100.0]);
        // 600 for three: 50 fits, the two others share 550.
        assert_eq!(
            share(&[500.0, 50.0, 900.0], 600.0, 10.0),
            vec![275.0, 50.0, 275.0]
        );
        // Never below the minimum.
        assert_eq!(share(&[500.0, 500.0], 40.0, 60.0), vec![60.0, 60.0]);
        // Unmeasured panels share equally.
        let inf = f32::INFINITY;
        assert_eq!(share(&[inf, inf], 300.0, 10.0), vec![150.0, 150.0]);
    }

    #[test]
    fn resized_panels_keep_their_height_and_the_others_share_the_rest() {
        let inf = f32::INFINITY;
        // Layers given 200, Properties the rest.
        assert_eq!(
            share_set(&[inf, inf], &[Some(200.0), None], 600.0, 60.0),
            vec![200.0, 400.0]
        );
        // No more than it needs.
        assert_eq!(
            share_set(&[120.0, inf], &[Some(300.0), None], 600.0, 60.0),
            vec![120.0, 480.0]
        );
        // Leaves the minimum to the others.
        assert_eq!(
            share_set(&[inf, inf, inf], &[Some(900.0), None, None], 600.0, 60.0),
            vec![480.0, 60.0, 60.0]
        );
        // None resized: as `share`.
        assert_eq!(
            share_set(&[50.0, inf], &[None, None], 400.0, 10.0),
            vec![50.0, 350.0]
        );
    }

    #[test]
    fn default_layout_has_settings_left_and_layers_right() {
        let l = Layout::default();
        assert_eq!(
            l.docked(Side::Left),
            [
                PanelId::Controller,
                PanelId::Page,
                PanelId::Template,
                PanelId::Variables,
                PanelId::ColorsInUse
            ]
        );
        assert_eq!(
            l.docked(Side::Right),
            [PanelId::Layers, PanelId::Properties]
        );
    }

    #[test]
    fn panels_move_between_sides_in_any_order() {
        let mut l = Layout::default();
        l.move_to(
            PanelId::Variables,
            Target::Dock(Side::Right, Some(PanelId::Layers)),
        );
        assert_eq!(
            l.docked(Side::Right),
            [PanelId::Variables, PanelId::Layers, PanelId::Properties]
        );
        // Below the last one.
        l.move_to(PanelId::Controller, Target::Dock(Side::Right, None));
        assert_eq!(
            l.docked(Side::Right),
            [
                PanelId::Variables,
                PanelId::Layers,
                PanelId::Properties,
                PanelId::Controller
            ]
        );
        // Within a side.
        l.move_to(
            PanelId::Properties,
            Target::Dock(Side::Right, Some(PanelId::Variables)),
        );
        assert_eq!(
            l.docked(Side::Right),
            [
                PanelId::Properties,
                PanelId::Variables,
                PanelId::Layers,
                PanelId::Controller
            ]
        );
        assert_eq!(
            l.docked(Side::Left),
            [PanelId::Page, PanelId::Template, PanelId::ColorsInUse]
        );
        // The left side can be emptied, and docked onto again.
        for id in [PanelId::Page, PanelId::Template, PanelId::ColorsInUse] {
            l.move_to(id, Target::Window);
        }
        assert!(l.docked(Side::Left).is_empty());
        l.move_to(PanelId::Template, Target::Dock(Side::Left, None));
        assert_eq!(l.docked(Side::Left), [PanelId::Template]);
    }

    #[test]
    fn floating_and_hidden_panels_leave_the_sides() {
        let mut l = Layout::default();
        l.move_to(PanelId::Layers, Target::Float(pos2(100.0, 50.0)));
        assert_eq!(l.place(PanelId::Layers), Place::Floating);
        assert_eq!(l.docked(Side::Right), [PanelId::Properties]);
        let (rect, once) = l.float_rect(PanelId::Layers);
        assert!(once);
        assert_eq!(rect.min, pos2(100.0, 50.0));
        assert_eq!(l.float_rect(PanelId::Layers).0.min, pos2(100.0, 50.0));

        l.set_open(PanelId::Properties, false);
        assert!(l.docked(Side::Right).is_empty());
        // Shown again where it was.
        l.set_open(PanelId::Properties, true);
        assert_eq!(l.docked(Side::Right), [PanelId::Properties]);
        // Moving a hidden panel shows it.
        l.set_open(PanelId::Page, false);
        l.move_to(PanelId::Page, Target::Window);
        assert!(l.is_open(PanelId::Page));
        assert_eq!(l.shown_in(Place::Window), [PanelId::Page]);
    }

    #[test]
    fn saved_layout_comes_back_with_missing_panels_added() {
        let mut l = Layout::default();
        l.move_to(
            PanelId::Properties,
            Target::Dock(Side::Left, Some(PanelId::Controller)),
        );
        let json = serde_json::to_string(&l).unwrap();
        let mut back: Layout = serde_json::from_str(&json).unwrap();
        back.normalize();
        assert_eq!(back.docked(Side::Left)[0], PanelId::Properties);
        // A layout saved before a panel existed gets it in its default place.
        back.entries.retain(|e| e.id != PanelId::ColorsInUse);
        back.normalize();
        assert_eq!(back.entries.len(), PanelId::ALL.len());
        assert_eq!(back.place(PanelId::ColorsInUse), Place::Docked(Side::Left));
    }

    /// Labels standing for each panel's contents, many lines in Properties.
    struct Labels;

    impl Contents for Labels {
        fn contents(&mut self, ui: &mut Ui, id: PanelId) -> Option<f32> {
            let lines = if id == PanelId::Properties { 60 } else { 2 };
            for k in 0..lines {
                ui.label(format!("{id:?} line {k}"));
            }
            // Layers stands for a long list of layers, filling the height it gets.
            if id.scrolls_itself() {
                ui.add_space(ui.available_height());
            }
            id.scrolls_itself().then_some(400.0)
        }
    }

    /// A window with the panels in it, run frame by frame with pointer events.
    struct Harness {
        ctx: egui::Context,
        layout: Layout,
        time: f64,
    }

    impl Harness {
        fn new(layout: Layout) -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                layout,
                time: 0.0,
            };
            for _ in 0..3 {
                h.frame(vec![]);
            }
            h
        }

        /// Runs one frame; returns where each text was drawn.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
            self.time += 0.02;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let layout = &mut self.layout;
            let mut out = self.ctx.run_ui(input, |ui| {
                layout.begin_frame(ui.available_rect_before_wrap());
                layout.show_dock(ui, Side::Left, &mut Labels);
                layout.show_dock(ui, Side::Right, &mut Labels);
                layout.show_floating(ui.ctx(), &mut Labels);
                egui::CentralPanel::no_frame().show(ui, |_| {});
                layout.drag_overlay(ui.ctx());
            });
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        Rect::from_min_size(t.pos, t.galley.size()),
                    )),
                    _ => None,
                })
                .collect()
        }

        fn find(&mut self, text: &str) -> Option<Rect> {
            let texts = self.frame(vec![]);
            texts.into_iter().find(|(t, _)| t == text).map(|(_, r)| r)
        }

        fn title(&mut self, id: PanelId) -> Pos2 {
            self.find(id.title()).expect("title shown").left_center() + vec2(10.0, 0.0)
        }

        fn pointer(&mut self, p: Pos2, pressed: Option<bool>) {
            let mut events = vec![egui::Event::PointerMoved(p)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            self.frame(events);
        }

        /// Drags from `from` to `to` in steps, then releases.
        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.pointer(from, None);
            self.pointer(from, Some(true));
            for k in 1..=10 {
                self.pointer(from + (to - from) * (k as f32 / 10.0), None);
            }
            self.pointer(to, Some(false));
            for _ in 0..3 {
                self.frame(vec![]);
            }
        }
    }

    #[test]
    fn dragging_a_title_docks_reorders_and_floats_panels() {
        let mut h = Harness::new(Layout::default());
        // Variables, from the left onto the top half of Layers: above it, on the right.
        let from = h.title(PanelId::Variables);
        let layers = h.title(PanelId::Layers);
        h.drag(from, layers + vec2(40.0, -2.0));
        assert_eq!(
            h.layout.docked(Side::Right),
            [PanelId::Variables, PanelId::Layers, PanelId::Properties]
        );
        assert!(
            h.find("Variables line 0").unwrap().left() > 500.0,
            "drawn on the right"
        );

        // Controller, to the middle of the window: it floats where it was dropped.
        let from = h.title(PanelId::Controller);
        h.drag(from, pos2(450.0, 300.0));
        assert_eq!(h.layout.place(PanelId::Controller), Place::Floating);
        let title = h.title(PanelId::Controller);
        assert!(
            (title.y - 300.0).abs() < 20.0,
            "floats under the pointer: {title:?}"
        );

        // Dragged by its title, the floating panel follows, then docks on the left.
        let left_dock = h.find("Page line 0").unwrap();
        // On Page's title, above its middle.
        h.drag(title, pos2(left_dock.center().x, left_dock.top() - 8.0));
        assert_eq!(h.layout.docked(Side::Left)[0], PanelId::Controller);
    }

    #[test]
    fn an_emptied_side_takes_panels_dropped_at_its_edge() {
        let mut layout = Layout::default();
        for id in [PanelId::Layers, PanelId::Properties] {
            layout.move_to(id, Target::Dock(Side::Left, None));
        }
        let mut h = Harness::new(layout);
        assert!(h.layout.docked(Side::Right).is_empty());
        let from = h.title(PanelId::Page);
        h.drag(from, pos2(990.0, 350.0));
        assert_eq!(h.layout.docked(Side::Right), [PanelId::Page]);
        assert!(h.find("Page line 0").unwrap().left() > 600.0);
    }

    #[test]
    fn a_crowded_side_shares_its_height_and_folds_from_titles() {
        let mut h = Harness::new(Layout::default());
        // Properties' 60 lines do not fit: it scrolls, its last line out of sight.
        assert!(h.find("Properties line 0").is_some());
        assert!(h.find("Properties line 59").is_none());
        let layers = h.find("Layers line 0").unwrap();
        assert!(layers.top() < 100.0, "Layers keeps its place: {layers:?}");
        // A click on a title folds the panel.
        let title = h.title(PanelId::Layers);
        h.pointer(title, None);
        h.pointer(title, Some(true));
        h.pointer(title, Some(false));
        assert!(h.layout.is_folded(PanelId::Layers));
        assert!(h.find("Layers line 0").is_none());
        assert!(h.find(PanelId::Layers.title()).is_some(), "the title stays");
    }

    #[test]
    fn dragging_the_line_between_docked_panels_resizes_them() {
        let mut h = Harness::new(Layout::default());
        let before = h.title(PanelId::Properties);
        // The line is just above Properties' title.
        let line = h.find(PanelId::Properties.title()).unwrap().center_top() - vec2(0.0, 7.0);
        h.drag(line, line - vec2(0.0, 100.0));
        assert!(h.layout.entry(PanelId::Layers).height.is_some());
        let after = h.title(PanelId::Properties);
        assert!(
            (before.y - after.y - 100.0).abs() < 2.0,
            "Properties moved up: {before:?} -> {after:?}"
        );
        // Kept in the layout.
        let json = serde_json::to_string(&h.layout).unwrap();
        let back: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.entry(PanelId::Layers).height,
            h.layout.entry(PanelId::Layers).height
        );
        // A double-click gives Layers the height it needs again.
        let line = h.find(PanelId::Properties.title()).unwrap().center_top() - vec2(0.0, 7.0);
        for _ in 0..2 {
            h.pointer(line, Some(true));
            h.pointer(line, Some(false));
        }
        assert_eq!(h.layout.entry(PanelId::Layers).height, None);
        // Docked elsewhere, it forgets the height it was given.
        h.layout.entry_mut(PanelId::Layers).height = Some(200.0);
        h.layout
            .move_to(PanelId::Layers, Target::Dock(Side::Left, None));
        assert_eq!(h.layout.entry(PanelId::Layers).height, None);
    }

    /// Draws docked panels, one of them folded, and a floating one, to look at them:
    /// `PANELS_PREVIEW=out.png cargo test panels_preview -- --ignored`.
    #[test]
    #[ignore]
    fn panels_preview() {
        let Ok(path) = std::env::var("PANELS_PREVIEW") else {
            return;
        };
        let mut layout = Layout::default();
        layout.move_to(PanelId::Properties, Target::Float(pos2(300.0, 120.0)));
        layout.entry_mut(PanelId::Template).folded = true;
        // Kept between the passes the preview runs: heights are measured on the first.
        let layout = std::cell::RefCell::new(layout);
        crate::preview::render(&path, vec2(900.0, 520.0), |ui| {
            let layout = &mut *layout.borrow_mut();
            layout.begin_frame(ui.max_rect());
            layout.show_dock(ui, Side::Left, &mut Labels);
            layout.show_dock(ui, Side::Right, &mut Labels);
            layout.show_floating(ui.ctx(), &mut Labels);
        });
    }

    #[test]
    fn drops_land_above_the_panel_under_the_pointer() {
        let mut l = Layout::default();
        let area = Rect::from_min_max(pos2(0.0, 20.0), pos2(1000.0, 700.0));
        l.begin_frame(area);
        l.set_dock_rect(
            Side::Right,
            Rect::from_min_max(pos2(680.0, 20.0), pos2(1000.0, 700.0)),
        );
        let right = |y0: f32, y1: f32| Rect::from_x_y_ranges(680.0..=1000.0, y0..=y1);
        l.set_slot(Side::Right, PanelId::Layers, right(20.0, 300.0));
        l.set_slot(Side::Right, PanelId::Properties, right(300.0, 700.0));
        l.begin_frame(area);
        let t = |x, y| l.drop_target(pos2(x, y)).map(|(s, b, _)| (s, b));
        assert_eq!(t(800.0, 100.0), Some((Side::Right, Some(PanelId::Layers))));
        assert_eq!(
            t(800.0, 200.0),
            Some((Side::Right, Some(PanelId::Properties)))
        );
        assert_eq!(t(800.0, 600.0), Some((Side::Right, None)));
        // The left side has no panel this frame: its zone at the window's edge.
        assert_eq!(t(10.0, 400.0), Some((Side::Left, None)));
        assert_eq!(t(400.0, 400.0), None);
    }
}
