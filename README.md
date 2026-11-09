# Artwork Creator

[![Test](https://github.com/bzhmaddog/artwork-creator/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/bzhmaddog/artwork-creator/actions/workflows/test.yml)
[![CodeQL](https://github.com/bzhmaddog/artwork-creator/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/bzhmaddog/artwork-creator/actions/workflows/github-code-scanning/codeql)

A desktop app for designing printed artwork for arcade controllers and fightsticks.

Load a controller template made of DXF files (outline, buttons, cutouts), arrange your
images on top, and export print-ready PNG or PDF at true size. The artwork is clipped to the
panel outline and left blank in the button and cutout holes. Runs on Windows and Linux.
Built in Rust with [egui](https://github.com/emilk/egui).

![Artwork Creator with the Cosmox T16 template: settings on the left, the A4 page with the
template outline, buttons and fastener holes over a grid image in the middle, and the
layers panel on the right](doc/capture.png)

> **Note:** this project is shamelessly vibecoded. It was built by describing features to
> an AI coding assistant ([Claude Code](https://claude.com/claude-code)), not by writing
> the code by hand. It has tests and works for its author, but read the code with that in
> mind.

## Features

- **Layers and groups**: add images (PNG, JPEG, WebP, BMP, GIF) by button or drag and drop;
  reorder, group, duplicate, hide and lock them; group opacity is applied to the group as
  a whole. Cut, copy and paste layers and groups, within a project, into another one, or
  between two windows of the app. A paste goes above the selection. Copies land 5 mm
  from the original, each one a step further. The first paste of a cut, or of a copy
  from another window, lands in place. A copy whose name is taken is named "name copy",
  then "name copy (2)", and so on, as with Duplicate. New text and shape layers go in the
  middle of the template, or of the view when that is out of sight.
- **Text layers**: typed in place on the canvas, in any installed font, bold and italic,
  multi-line text aligned left, centered or right, letter and line spacing, color and
  outline. Text layers move, scale, rotate, distort and take effects like images, and are
  drawn at the export resolution so they print sharp. Characters the font lacks (other
  scripts, symbols, emoji) are taken from the system's fallback fonts, or the built-in
  emoji font. Fonts, fallback ones included, are saved inside `.artwork` projects (except
  those whose license forbids embedding them), so a shared project looks the same
  elsewhere.
- **Text along a path**: bent into an arc, set around a circle (the layer centered on
  it, so its center snaps to a button's anchor), or following the outline of a shape
  layer, curves included. The text's position along the path, the side it stands on and
  its distance from the path can be set; each letter stays upright on the path. A text
  following a shape moves with it. Text on a path is typed in Properties.
- **Shape layers**: rectangle (with rounded corners), ellipse, line, polyline, polygon,
  regular polygon and star, with a stroke (width, color, opacity, dashes, line ends and
  joins) and, for closed shapes, a fill. Lines, polylines and polygons are drawn by
  clicking their points on the canvas, their segments straight or Bezier curves, and
  their points and curve handles can be moved, added and removed afterwards; points snap
  to the template's anchors. Resizing a shape keeps its stroke
  width in mm, and one kind can be turned into another (a rectangle into a polygon of its
  corners, for instance). Shapes move, rotate, distort and take effects like
  images, and are drawn at the export resolution.
- **Transform on the canvas**: move, scale, rotate, stretch width or height on its own, and
  drag single corners to fake perspective.
- **Effects** on each image layer: brightness / contrast, hue / saturation, grayscale,
  sepia, tint, invert, posterize, threshold, Gaussian blur, sharpen, drop shadow and
  outline (solid, dashed, dotted or dash-dot). They are stacked, reordered and toggled in the layer's properties, applied to
  the export at full resolution, and never change the image file. More effects can be
  added with [plugins](#plugins).
- **Variables**: named values kept with the project, in the **Variables** panel, each
  of a kind: a length in mm (`button = 30`), an angle in degrees, a text size in pt, a
  number without a unit (`count = 3`), a color or a text. **Add a
  variable** asks for the kind; the panel lists the variables grouped by kind, each
  behind its kind's icon, with its unit after its value.
  - Type a variable's name, or a formula such as `button + 2` or
    `(panel - 2 * margin) / 3`, in a layer's width, height, x, y, stroke width or corner
    radius field (lengths), rotation field (angles) or text size field (text sizes) to
    link it: the field then shows the result (the formula is in its tooltip and its fx
    button is filled with the link color), and follows when a variable changes. A field takes the variables of its
    kind and numbers; a formula using another kind shows an error and leaves the field
    as it is. Typing a plain number in a linked field, or dragging its value, unlinks
    it, and so does moving, resizing or turning the layer some other way (on the canvas,
    with Align or the arrow keys).
  - The **fx** button inside each of those fields, at its right end, opens the formula
    editor: the formula (the field's value to start with), its result or what is wrong
    with it, and the variables the field takes, a click inserting one at the cursor.
    **OK** (or Enter) links the field to the formula, or just sets the value when it
    names no variable; **Unlink, keeping the value** unlinks the field. The fx becomes a
    badge filled with the link color when the field is linked, so the fields using
    variables stand out, and with the warning color when its variable is gone.
  - With the size's aspect locked, linking the width also links the height to the same
    formula scaled by their ratio (`button` gives `button * 0.5` for a layer half as
    tall as wide), and the other way round; unlinking one unlinks both, and locking the
    aspect with only one of them linked links the other.
  - Every color picker (text, outline, fill, stroke, effects, page background) also shows
    the color variables and the colors already used in the project; a click on one
    applies it. A color picked from the color variables stays linked to it (a chain link is
    drawn inside its square swatch, the variable's name in its tooltip) and follows when the
    variable changes; picking another color, or clicking the linked color
    variable again in the picker (it shows a chain), unlinks it. Below its sliders, a
    picker has the color's **Hex** code, to type or copy (`#RRGGBBAA` sets the opacity
    too), and its **Opacity** in %; hovering a color button or swatch shows its code and
    opacity. **Save this color** in a picker makes a color variable of the color and
    links to it.
  - A text layer's text is linked to a text variable with the link button at the top
    right of its text box: it then shows the variable's text and follows when it
    changes, so one variable can fill several layers. Typing in the text, in the panel
    or on the canvas, unlinks it, with a warning in the status bar (undo links it
    again); **Unlink, keeping the value** unlinks it too.
  - The **Colors in use** panel, below Variables by default, lists the colors in use one per
    row, as `#RRGGBB` followed by its opacity (`100%`). Double-click one to
    add a color variable of that color, even if another color variable has it.
  - Renaming a variable renames it in the formulas and links. Removing one that is in
    use asks first; the fields and colors using it keep their value and show a broken
    link icon (hover it to see what it was linked to). Typing a value in such a field,
    or picking a color, unlinks it; adding a variable of the same name links it again.
    In the layer list, a layer with such broken links shows a broken chain, in the
    warning color, naming the variables in its tooltip; so does a folded group holding
    one.
    Layers pasted into another project bring their color variables along, unless that
    project gives the name to another variable.
- **Snap to anchors**: the centers, corners and edge middles of images snap to points
  defined by the template, such as the button centers. So do a shape's center, the
  corners and side middles of its box, or its points (its stroke left out).
- **Snap to page and outline**: while a layer or group is moved, its edges and middle snap
  to the page's edges and middle, and to the template's outline: the edges and middle of
  its bounds, its straight sides, and the bleed line when there is a bleed. An edge being
  stretched and a corner being resized snap too. A dashed line shows the guide it snapped
  to. Hold Alt to drag freely, or turn it off with **View > Snap to page and outline**.
- **Align** layers or groups to the page or to the template.
- **Undo / redo** for every change to the project.
- **Templates** made of several elements, each in its own DXF file, with its own color and
  visibility.
- **Export** PNG and/or PDF at the resolution you choose (72 to 1200 DPI). Choose which
  template lines are printed, and whether the inside of buttons and of cutouts is left
  blank or printed over. In the PDF, template lines, text and shapes stay sharp vectors
  (text as outlines, so no font is needed to print it); text and shape layers with
  effects, or inside a semi-transparent group, are printed as images like the rest.
- **Page sizes**: A5, A4, A3, A2, Letter, Legal and Tabloid, in portrait or landscape, or
  a custom width and height.
- **Print** straight from the app (**File > Print…**), at true size. The Print window
  shows the same preview and settings as the Export window (lines, marks, sheets,
  mirroring), shared between the two. On Windows, the printer, copies
  and paper are chosen in the system print dialog, its paper preset to the page's or the
  sheets'; keep its scale at 100%. On Linux, printing goes through CUPS (`lp`): pick
  the printer and copies in the Print window.
- **Tiled printing**: a page larger than your paper is split over several sheets (A5 to
  Tabloid, portrait or landscape) in the Export window: one PDF page and one PNG file
  (`name-r1c2.png`) per sheet. Each sheet keeps a blank margin, the artwork of
  neighboring sheets overlaps, and alignment marks in the margin show the corners of
  each sheet's artwork and where to trim one sheet to line it up with the next. The
  sheets are centered on the page; drag them on the export preview, or set their
  position, to place the joins where you want them.
- **Low resolution warning**: image layers that would print below a chosen resolution
  (150 DPI by default, set in the Export window) are flagged with a ⚠ in the layer list,
  in Properties and in the Export window. The resolution is measured where the image is
  stretched most, distortion included.
- **Mirrored export**: the whole print flipped left to right, template lines and marks
  included, to print on the back of clear film or plexi and see it the right way through
  it.
- **Print margins and marks**: a **bleed** (Template panel) prints the artwork a few mm
  past the outline and into the holes, so a cut slightly off the line leaves no blank
  edge; a **safe area** shows a dashed guide on the canvas, that far inside the outline
  and around the holes, to keep text and details clear of the cuts. The export can add
  **crop marks** in line with the edges of the outline's box and **registration
  marks** at the middle of its sides, both outside the bleed.
- **Projects** are saved, by default, as a single `.artwork` file (a zip archive) holding
  the project and all its images, so it can be moved or shared on its own. **Save As**
  can also write a linked `.json` project that points to the image files on disk; images
  that only exist inside an archive are then written to an "*name* images" folder next to
  it.
  Both keep the Export window's settings too (formats, resolution, lines, marks, sheets
  and their position), so a project exports the same way every time. Changing them
  counts as an unsaved change, but not as an undo step.
  **File > Open Recent** lists the last 10 projects.
- **Movable panels**: Controller, Page, Template, Variables, Colors in use, Layers and
  Properties are each a panel. Drag a panel by its title onto the left or right side of
  the window to dock it there, above or below the other panels of that side (a line shows
  where it goes), or anywhere else to float it over the canvas; drag a floating panel's
  title to move it, or onto a side to dock it again. The menu at the right of a panel's
  title docks it on either side, floats it, opens it in a window of its own (to put it on
  another screen; keyboard shortcuts work there too) or hides it. A click on a docked
  panel's title folds it. Panels on the same side share its height: small ones get what
  they need, the others share the rest and scroll. Drag the edge of a side to widen it,
  or the line between two docked panels to give the one above more or less height
  (double-click the line to give it the height it needs again). **View > Panels** shows hidden panels
  again and resets the layout. The layout is kept between sessions.
- **English (US and UK) and French (France)** interface. It follows the system language
  by default; choose another in **View > Language**.

## Building

You need a [Rust toolchain](https://rustup.rs) (edition 2024).

```sh
cargo run --release
```

The included `Makefile` builds for Linux and cross-compiles for Windows from Linux or WSL2
(this needs `mingw-w64` and the `x86_64-pc-windows-gnu` Rust target):

```sh
make            # release builds for Linux and Windows
make linux      # Linux only
make windows    # Windows .exe only
make test       # run the tests
make plugins    # build the example plugins into plugins/ (.so and .dll)
make dist       # package each build with the templates folder into dist/
```

## Development

After cloning, install the git hooks once:

```sh
git clone https://github.com/bzhmaddog/artwork-creator.git
cd artwork-creator

# Install hooks
make hooks
```

Before each commit, the hook runs the CI checks: `cargo fmt --check`, clippy for Linux and
Windows (some lints only fire on one of them), and the tests on Linux. The Windows check
is skipped if the `x86_64-pc-windows-gnu` target is not installed. Run them by hand with
`make pre-commit`, or for one target with `make pre-commit ARGS=--linux` or
`ARGS=--windows`.

`make check-dependencies` scans `Cargo.lock` for crates with a known vulnerability or a
malicious version, using [Trivy](https://trivy.dev) in a container. It uses Docker if
installed, otherwise Podman (`CONTAINER=podman` forces one), and fails if anything is
found. The Trivy image version is pinned in the Makefile (`TRIVY_VERSION`); raise it on
purpose rather than following `latest`. Its vulnerability database is cached in
`~/.cache/trivy`.

### Consistency

When adding a tool or feature, give it the same treatment as its siblings. For example,
every way of adding a layer has a keyboard shortcut, shown in its menu entry or tooltip
and listed under [Shortcuts](#shortcuts); every text has a translation key, with its
text in `locales/en-US.json` and `locales/fr-FR.json` (and `locales/en-GB.json` when the
British spelling differs). Before finishing, compare the new feature with the closest
existing one and fill in what it lacks. A number field's unit goes after the field, not inside it
(`layers_ui::unit`); a pair of fields on one row (x and y, width and height) shares one. A
dialog's buttons sit at the right of its bottom row, the main one rightmost and Cancel on
its left (`app::dialog_button_row`); a secondary one such as "Close without saving" or Unlink goes
apart at the left edge (`app::at_left_edge`). A helper text (an explanation, an empty state,
a tip, a warning) is a callout (`callout::callout`, or `callout_small` in tight places),
of the tone that fits: Info, Tip or Warning; data shown for information (sizes, counts,
paths) stays plain weak text. `CALLOUT_PREVIEW=out.png cargo test callout_preview --
--ignored` draws them in both themes, to look at without starting the app. Any piece of the
interface can be drawn that way from a test, with `preview::render`.

Write code, comments, docs and texts in U.S. English (color, center, gray, canceled);
British spellings belong in `locales/en-GB.json` only. Name translation keys after what
the text is for, in upper snake case: `ADD_TEXT_LAYER`, `EXPORT_CANCELED`.

### Releasing

`make version` lists the commits since the last release tag and suggests the next version.
Only commits that change the app count (`src`, `locales`, `templates`, `plugin-sdk`,
`Cargo.toml` and `Cargo.lock`); tooling, CI, docs and tests never call for a release. It
suggests minor when a commit message adds something ("Added...", "New...") or the plugin interface
changed (major from 1.0.0 on), patch otherwise. `make release` sets that version in `Cargo.toml`, commits and tags
it after asking; `make release V=0.3.0` (or `V=major|minor|patch`) picks the version, and
`SDK=minor` releases the plugin SDK too. Nothing is pushed. See
[`scripts/version.sh`](scripts/version.sh) for the details.

## Templates

The app looks for a `templates` folder next to the executable, then in the current
directory. Each subfolder holding DXF files is one template. A `template.json` file in it
describes the elements:

```json
{
  "name": "Cosmox T16",
  "units": "inch",
  "elements": [
    { "kind": "outline", "dxf": "outline.dxf" },
    { "kind": "buttons", "dxf": "buttons.dxf", "name": "Action buttons" },
    { "kind": "buttons", "dxf": "menu.dxf", "name": "Menu buttons" },
    { "kind": "cutouts", "dxf": "holes.dxf", "name": "Fastener holes", "color": "#ff8c00" },
    { "kind": "anchors", "dxf": "anchors.dxf", "name": "Button centers" }
  ],
  "placement": { "rotation": 90, "flip_x": false, "flip_y": true, "offset_mm": [0, 0] }
}
```

- `kind` is `outline`, `buttons`, `cutouts` or `anchors`. The outline clips the artwork;
  buttons and cutouts are holes, left blank in the export unless turned off in the Export
  window.
- `anchors` are points that images snap to on the canvas: while an image is moved, its
  center, a corner or the middle of an edge lands on an anchor that comes near, and a corner being resized or
  distorted does too. Hold Alt to drag freely, or turn it off with **View > Snap to
  anchors**. Anchors are drawn as small crosses and cut nothing; they are printed only when
  ticked under **Print lines** in the Export window. They are read from the DXF's POINT
  entities, usually all in one `anchors.dxf`. The center of each closed shape counts too,
  so an anchors element can also point at a buttons DXF to get the button centers. They
  don't change the template's size or placement.
- All DXF files of a template must share the same coordinates, as when they are exported
  from one drawing. Outlines drawn as separate lines and arcs are joined automatically.
- `units` (`mm`, `cm` or `inch`) is used when the DXF files don't declare their units.
- `name`, `color` (`#rrggbb`) and `placement` are optional. The template's default placement
  can also be saved from the app.
- Without `elements`, the folder's single DXF file is used as the outline.

The app reloads a template when its files change on disk.

## Plugins

Effect plugins are shared libraries (`.dll` on Windows, `.so` on Linux) placed in a
`plugins` folder next to the executable or, failing that, in the current directory. The
app loads them at startup and lists their effects in **Add effect**, under the plugin's
name. A library that fails to load, or was made for another plugin interface version, is
skipped and reported at the bottom of that menu.

> **Install only plugins you trust.** A plugin is native code running inside the app
> with all its rights: it can read and write your files, and a crash in it closes the
> app, losing unsaved work.

A project that uses an effect whose plugin is not installed still opens: the effect is
shown as "Missing effect", is not applied, and is saved unchanged.

### Writing a plugin

In Rust, use the [`artwork-plugin-sdk`](plugin-sdk) crate: implement its `Effect` trait,
which works on straight RGBA pixels and describes its parameters so the app builds their
controls, and export it with `export_plugin!`. The crate must be a `cdylib`. See
[`plugin-examples/halftone`](plugin-examples/halftone) for a complete example, built by
`make plugins`.

Other languages can implement the C interface described in
[`plugin-sdk/include/artwork_plugin.h`](plugin-sdk/include/artwork_plugin.h): one
exported function, `artwork_plugin_entry`, returning a table of effects whose parameters
and values are passed as JSON.

Effect ids are saved in projects, so keep them stable and prefix them with something
unique to your plugin (`builtin.` is reserved). Effects may run on several threads at
once.

## Translations

The code names each text by a key that says what it is for: `tr!("ADD_TEXT_LAYER")`
(`tr_n!("LAYER_COUNT", n)` for counts, and `tr_noop!` where a key is defined apart from
where it is shown). The texts live in `locales/<code>.json`, built into the program:

```json
"ADD_TEXT_LAYER": "Add a text layer (Ctrl+T)",
"LAYER_COUNT_ONE": "{n} layer",
"LAYER_COUNT_OTHER": "{n} layers",
```

To reword a text, edit it in that language's file only. `locales/en-US.json` holds the
English of every key and is used for keys missing from another table. `locales/fr-FR.json`
must have every key. `locales/en-GB.json` only holds the texts spelled differently in
British English (colour, centre, grey, cancelled...). The tests check that every key of
the code is in en-US and fr-FR, that no table has leftovers, that `{placeholders}` match,
and that texts with a U.S. spelling have their en-GB entry. Effects from plugins show
their own names.

To add a language: add a `Language` variant and its table in `src/i18n.rs`, and a
`locales/<code>.json` file.

## Shortcuts

| Action | Shortcut |
|---|---|
| New / Open / Save project | Ctrl+N / Ctrl+O / Ctrl+S |
| Export | Ctrl+E |
| Print | Ctrl+P |
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y |
| Add an image layer | Ctrl+I, or drop image files on the window |
| Add a text layer | Ctrl+T |
| Add a shape layer (the kind added last; others from the Shape button) | Ctrl+R |
| Type in a text layer on the canvas | Double-click it (straight text only); Esc or a click outside it to stop |
| Finish drawing a polyline or polygon | Double-click, Enter, Esc or right-click; a click on the first point closes it |
| Edit the points of a line, polyline or polygon | Double-click it; Esc or selecting something else to stop |
| Add a point while editing points | Double-click a segment |
| Make a point smooth or sharp | Double-click it while editing points |
| Place a smooth point while drawing | Press and drag |
| Cut / Copy / Paste layer | Ctrl+X / Ctrl+C / Ctrl+V |
| Duplicate | Ctrl+D |
| Group / Ungroup | Ctrl+G / Ctrl+Shift+G |
| Delete layer | Del |
| Nudge by 1 mm (10 mm with Shift) | Arrow keys |
| Fit page in view | F |
| Pan | Space+drag, middle or right drag, or drag on empty canvas |
| Zoom | Mouse wheel, or Ctrl+scroll |

On the canvas, with a layer selected:

- **Corner handles** scale; **Shift** scales width and height freely.
- **Distort** (in Properties) makes corner handles move one corner at a time, for
  perspective. **Holding Ctrl** swaps between resizing and distorting while you drag.
- **Edge handles** stretch one side; **Alt** stretches both sides from the center.
- **Rotation handle** rotates; **Shift** snaps to 15° steps.

A line, polyline or polygon picked in the Shape menu is drawn on the canvas: each click
places a sharp point, and a press dragged places a smooth one, pulling out its curve
handles. A click on the first point closes a polyline into a polygon; a
double click, **Enter**, **Esc** or a right click finishes; a line is done at its second
point.

While editing a shape's points, drag a point to move it, or double-click a segment to add
a point there; the curve keeps its shape. The picked point shows its curve handles, and
its neighbors the ones facing it: drag them to bend the curve. A smooth point's two
handles turn together; **Alt** moves one on its own. Double-click a point to make it
smooth or sharp. **Del** removes the point picked last and the **arrow keys** nudge it.

While drawing or editing points, **Shift** keeps the segment at 15° steps, and points snap
to anchors unless **Alt** is held.

While typing in a text layer on the canvas, the keys edit the text as in any text field:
arrows, Home / End, Shift to select, Ctrl+A / C / X / V, Ctrl+Z for the typing itself.
A click on the text moves the cursor, a drag selects, a double click selects a word.
Dragging a handle stops typing and transforms the layer as usual.

## License

Licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE): free to use, copy,
modify and share for any non-commercial purpose.

**Selling artwork made with this app is not allowed.** Using it to make artwork, panels or
controllers for sale (commissions, shops, made-to-order or batch production) is commercial
use and needs the author's permission. Making artwork for your own controller is fine,
and so is selling your own controller secondhand later.

For commercial use, please contact the author.
