# UI style rules

The rules the interface follows, taken from the code as it is. Follow them for any new
widget, panel or dialog. When a change brings a new rule, or changes one of these, update
this file in the same change. See also "Consistency" in [README.md](../README.md).

The app is built with [egui](https://github.com/emilk/egui) and follows its look, in both
light and dark themes: every color comes from `ui.visuals()` (or is picked per theme), never
a fixed color that only works in one.

## Tokens

The shared values (fixed colors, sizes, corner radii, spacing, strokes) are constants
in [`src/ui/tokens.rs`](../src/ui/tokens.rs); colors that follow the theme come from egui's
`Visuals`. [`docs/tokens.png`](tokens.png) shows them all in both themes, and
[`docs/widgets.png`](widgets.png) the widgets made with them. Use a token, not a literal
number or color; a new value that other widgets will share becomes a token, added to
the image's test (`ui_tokens` in `src/ui/gallery.rs`). Values used by only one widget
(a dial's notch, a handle's reach) stay next to it.

## Texts

- Every text has a translation key (see CLAUDE.md and README.md). Plain ASCII punctuation:
  `'` and `"..."`, not `’` or `« »`, also in French.
- A helper text (an explanation, an empty state, a tip, a warning) is a callout:
  `callout::callout`, or `callout_small` in tight places (a menu, a table cell, a bar),
  in the tone that fits: `Info`, `Tip` or `Warning`. Data shown for information
  (sizes, counts, paths) stays plain weak text (`ui.weak`).
- Labels can't be selected (`selectable_labels = false` in `app::set_interaction`).
- Don't add notes, hints or tooltips nobody asked for; one generic message rather than
  one per case.

## Icons and buttons

- An action shown as an icon is drawn with the painter, not a text or emoji glyph (the
  fonts lack many, such as ⚠: use `callout::warning_icon` / `warning_mark`).
- An icon button is `layers::flat_button`: nothing behind it until hovered or pressed,
  then a `weak_bg_fill` rectangle with 3 pt corners; the icon drawn in the color it is
  given, strokes about 1.3 to 1.5 pt; faded when disabled. Examples: `layers::tool_icon`,
  `panels::icon_button`, `effects::regenerate_button`.
- Sizes: 24 pt for the layer toolbar (`TOOL`), 20 pt in panel title bars, 18 pt in the
  layer list rows, `interact_size.y` (a field's height) next to fields and menus.
- Every icon button has a tooltip (`on_hover_text`) saying what it does; a command with
  a shortcut shows it there or in its menu entry.
- A toggle that is off is drawn at 35 % (`layers::icon_color`).
- A button that deletes or discards something (Delete guide, Delete template, Don't
  save, Clear guides, Discard a recovery copy) is `app::danger_button`: white text on
  red. Menu entries that do so (Remove effect, Clear recent list) stay plain.
- A square toggle (bold, italic, alignment) is `interact_size.y` high, highlighted with
  `interact_selectable` when on (`properties::style_toggle`).

## Fields

- A unit never goes inside a field or a slider's value box (no `.suffix`/`.prefix`): it
  follows the field with `layers::unit`. A pair of fields on one row (x and y, width and
  height) shares one.
- A field's short name ("x", "y", "W") goes in a shaded segment on its left:
  `layers::prefixed`, or `prefixed_icon` for an icon.
- A position or size in mm is `mm_value` (one decimal, up to 5000 mm).
- A 0 to 100 % value is a slider, unless something else already sets it (a gradient
  stop is dragged on its bar, so its position is a plain field).
- A list of values is a drop-down (`properties::choice`, 130 pt wide).
- A color is a color button (`colors::button`, `linked_button` when it can be linked to
  a color variable): a square swatch, a chain drawn in it when linked.
- Fields in a popup get their frames back with `layers::field_frames`.

## Layout

- Settings sit in a two-column grid of labels and controls: `prop_grid` in Properties
  (labels as wide as the widest of `PROP_LABELS`, rows 6 pt apart), `panel_grid` in the
  Page, Template and Print panels (`PANEL_LABELS`, rows 4 pt apart), so controls line up
  from one section to the next in every language. A new label goes in the list.
- Settings under a row (a fill's) go across the whole row, 8 pt in: `wide_row`.
- A Properties section is `properties::section`: a line above, the fold arrow and title,
  extra buttons at its right, its body not indented. Folding sections inside it (a fill's
  Settings, each effect) indent their body by `spacing.indent`, 8 pt, set app-wide in
  `app::set_interaction`.

## Dialogs, menus and notices

- A modal dialog is `app::dialog`: framed like a window with 14 pt of room
  (`DIALOG_MARGIN`); a window's contents get the same with `padded`.
- Its buttons sit at the right of the bottom row, the main one rightmost and Cancel on
  its left (`dialog_buttons` / `dialog_button_row`); a secondary one ("Close without
  saving", Unlink) goes apart at the left edge (`at_left_edge`).
- A popup with text fields has a fixed width: a field with `desired_width(f32::INFINITY)`
  would stretch it across the screen.
- A button or menu entry that runs a command (`ui::shortcuts::Command`) shows its
  current key: at the right of a menu entry (`menu_entry`), in a button's tooltip
  (`shortcuts::with_key`). A command gets a default key when its siblings have one.
- A menu entry with an icon is `layers::icon_entry`, as wide as the other entries of its
  menu.
- Something that must not stop the user is a status bar notice (`app::notify`, 6 s), not
  a message box.

## Checking the look

- `cargo test widget_gallery -- --ignored` draws one of each widget to
  `docs/widgets.png`; add a new kind of widget to it (`src/ui/gallery.rs`).
  `cargo test ui_tokens -- --ignored` draws the tokens to `docs/tokens.png`.
- `CALLOUT_PREVIEW=out.png cargo test callout_preview -- --ignored` draws the callouts in
  both themes. Any piece of the interface can be drawn from a test with
  `preview::render`, to look at it in both themes without starting the app.
