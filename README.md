# Artwork Creator

[![Test](https://github.com/bzhmaddog/artwork-creator/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/bzhmaddog/artwork-creator/actions/workflows/test.yml)
[![CodeQL](https://github.com/bzhmaddog/artwork-creator/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/bzhmaddog/artwork-creator/actions/workflows/github-code-scanning/codeql)

A desktop app for designing printed artwork for arcade controllers and fightsticks.

Load a controller template drawn in SVG (outline, buttons, cutouts), arrange your
images on top, and export print-ready PNG, PDF or SVG at true size. The artwork is clipped to the
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
  a whole. A locked layer does not move or change on the canvas, but a click selects it
  when no unlocked layer is under the pointer (so the layer toolbar and Properties work
  on it); a padlock in its middle shows it is locked. Cut, copy and paste layers and groups, within a project, into another one, or
  between two windows of the app. A paste goes above the selection. Copies land 5 mm
  from the original, each one a step further. The first paste of a cut, or of a copy
  from another window, lands in place. A copy whose name is taken is named "name copy",
  then "name copy (2)", and so on, as with Duplicate. New text and shape layers go in the
  middle of the template, or of the view when that is out of sight. A new shape or canvas
  is named with two random words, an adjective and a noun (Swift Otter, Golden Comet...),
  a name the project doesn't use yet; a new text says such a name, selected to be typed over, and is named after its text until
  renamed.
- **Text layers**: typed in place on the canvas, in any installed font, bold and italic,
  multi-line text aligned left, centered or right, letter and line spacing, color and
  outline. Text layers move, scale, rotate, distort and take effects like images, and are
  drawn at the export resolution so they print sharp. Characters the font lacks (other
  scripts, symbols, emoji) are taken from the system's fallback fonts, or the built-in
  emoji font. Fonts, fallback ones included, are saved inside `.artwork` projects (except
  those whose license forbids embedding them), so a shared project looks the same
  elsewhere.
- **Text along a path**: bent into an arc, set around a circle (the layer centered on
  it, so its center snaps to a button's center), or following the outline of a shape
  layer, curves included. The text's position along the path, the side it stands on and
  its distance from the path can be set; each letter stays upright on the path. A text
  following a shape moves with it. Text on a path is typed in Properties.
- **Shape layers**: rectangle (with rounded corners), ellipse, line, polyline, polygon,
  regular polygon and star, with a stroke (width, color, opacity, dashes, line ends and
  joins) and, for closed shapes, a fill. Lines, polylines and polygons are drawn by
  clicking their points on the canvas, their segments straight or Bezier curves, and
  their points and curve handles can be moved, added and removed afterwards; points snap
  to the centers of the template's shapes. Resizing a shape keeps its stroke
  width in mm; a shape keeps the kind it was made as. Shapes move, rotate, distort and take effects like
  images, and are drawn at the export resolution.
- **SVG import**: an SVG file picked from **Path...** in the Shape menu or dropped on the
  window comes in as shape layers above the selection, with a group for each of its
  own that has an id or an opacity. When that makes several layers and groups, they all
  go in one group named after the file, so the layer list gets one entry. Fills, strokes (width, color, opacity, caps, joins,
  dashes as the nearest of the app's styles), linear and radial gradients and fill rules
  are kept; it keeps its true size, shrunk to fit the page if larger. A path made of
  several outlines (a letter with a hole, a logo in one piece) becomes a **path** shape,
  its fill rule (non-zero or even-odd) set in Properties and its points edited like a
  polygon's. Only shapes come in: texts, images, scripts, links, embedded HTML, filters,
  masks, clipping and patterns are left out (the app says which), and nothing the file
  points to is ever loaded.
- **Painting**: any closed shape (rectangle, ellipse, polygon, star, closed path) can be
  painted in. While one is selected, a tool bar at the canvas' top left has Select, Pencil, Brush and Eraser; the tool picked last comes back
  whenever such a shape is selected. The pencil paints an even line; each tool has its
  color and width (in mm) on the tool bar. The brush comes in several kinds, picked in
  the menu its tool bar button opens (its icon shows the kind in use): **round** (thinning out at both ends),
  **calligraphy** (a flat nib at an angle set by turning the wheel on the tool bar:
  broad across it, thin along it), **marker** (square ends, at half the color's
  opacity, like a highlighter), **spray** (dots scattered around the line, denser in the middle, as
  thick as the density set on the tool bar) and **bristle** (thin lines side by side,
  swelling and running dry here and there, like a dry brush). Strokes
  are vectors, sharp at any size and in the PDF, cut off at the shape's outline, over
  its fill (the background) and under its stroke (the border): painting never changes
  the shape. Resizing the shape (by its handles or its Size) stretches its strokes, or,
  with **Stretch paint** off in its Properties, leaves them where they are on the page,
  the shape showing more or less of them, as when its points are moved. Ctrl+Alt on a
  handle does the other way for that drag. Turned into a line or polyline, it hides them
  until it is closed again. The **Canvas** button, next to Image, Text and Shape, opens
  a menu of the closed kinds (rectangle, ellipse, polygon, regular polygon, star) and
  adds one to paint in, without fill or stroke (its outline shows dashed on screen, not
  printed), with Select in hand: half the page's size (an ellipse starts as a circle,
  its handles stretch it), or, for a polygon, drawn point by point as from the Shape
  menu. Its shortcut adds the kind added last. The eraser cuts the selected shape's
  strokes where it passes; a stroke crossed in the middle becomes two, a spray loses
  the dots it passes over. **Clear the paint**, next to the tools on the tool bar, empties the selected shape of all its strokes at once (Undo
  brings them back). With a pen on Windows, the width follows its pressure. Esc
  goes back to selecting.
- **Gradients**: a shape's fill and stroke, a text's letters and the page background can
  each be one color or a linear or radial gradient (Solid / Linear gradient / Radial
  gradient, next to its color). A linear gradient runs at an angle, from one corner of
  the layer's box to the opposite one; a radial one grows from a center (its position and
  radius can be set) in an ellipse the shape of the box. Its colors sit on a bar: click
  the bar to add one, drag one along it, right-click it to remove it (two always stay).
  Each color has its own opacity and can be linked to a color variable.
- **Transform on the canvas**: move, scale, rotate, stretch width or height on its own, and
  drag single corners to fake perspective.
- **Effects** on each image layer: brightness / contrast, hue / saturation, grayscale,
  sepia, tint, invert, posterize, threshold, Gaussian blur, sharpen, drop shadow,
  outline (solid, dashed, dotted or dash-dot), and linear (horizontal or vertical) or radial
  fade to transparent (a side of the object, or its edges or center, along its visible pixels, so margins
  added by a shadow or outline do not count), shaped by an easing curve as in
  animation tools: Linear, Ease in, Ease out, Ease in-out, or your own by dragging the two
  handles of its graph. They are stacked, reordered and toggled in the layer's properties, applied to
  the export at full resolution, and never change the image file. More effects can be
  added with [plugins](#plugins).
- **Variables**: named values kept with the project, in the **Variables** panel, each
  of a kind: a length in mm (`button = 30`), an angle in degrees, a text size in pt, a
  number without a unit (`count = 3`), a color or a text. **Add a
  variable** asks for the kind; the panel lists the variables grouped by kind, each
  behind its kind's icon, with its unit after its value. A name starts with a lowercase
  letter or `_` (not a capital or a digit), then letters, digits or `_`; its field turns
  red otherwise.
  - The project's template brings its own variables (set in the template editor): the
    panel then has two sections, **Template** first, in tinted read-only rows, then
    **Project**, the project's own. Formulas use the template's like the others
    (`actionButtonDiameter + 2`). They come from the project's copy of its
    template, not saved with the project's variables; a project variable can't take one
    of their names (one that already does is used instead).
  - A project length, angle, text size or number can inherit from the template's
    variables: type a formula such as `templateVar + 2` in its value, or use its **fx**
    button (as in a layer's fields, below; it lists the template's variables only). It
    then follows the template's variables, and the layers using it follow too. If the
    template no longer has a variable the formula uses (the template replaced), the
    variable's name turns red, a warning under it names the missing variable, and it
    keeps its last value until the formula is changed or unlinked.
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
- **Snap to centers**: the centers, corners and edge middles of images snap to the
  center of each closed shape of the template (its buttons, cutouts and outline), drawn
  as a small cross in the element's color. So do a shape's center, the corners and side
  middles of its box, or its points (its stroke left out). Hold Alt to drag freely.
- **Snap to page and outline**: while a layer or group is moved, its edges and middle snap
  to the page's edges and middle, and to the template's outline: the edges and middle of
  its bounds, its straight sides, and the bleed line when there is a bleed. An edge being
  stretched and a corner being resized snap too. A dashed line shows the guide it snapped
  to. Hold Alt to drag freely, or turn it off with **Snap to page and outline** in the View panel.
- **Rulers and guides**: rulers along the canvas' top and left edges measure in mm from
  the page's top-left corner. Drag from a ruler to add a guide: a horizontal one from the
  top ruler, a vertical one from the left ruler. Drag a guide to move it, off the canvas
  (onto a ruler) to delete it, or double-click it to type its position. Guides are saved
  with the project and never printed. While moved or resized, layer edges and centers snap
  to them as they do to the page; a dragged guide snaps to the edges and centers of the
  layers, the page and the outline. Hold Alt to drag freely. The View panel shows or
  hides the rulers and the guides, turns snapping to guides on or off, locks the guides
  so they can't be moved by accident, and clears them all.
- **Align** layers or groups to the page or to the template.
- **Undo / redo** for every change to the project.
- **Templates** made of several elements, each a layer of the template's SVG, with its own color and
  visibility. The **Template** panel picks the template and shows or hides each
  element's lines, and sets their color. The template is centered on the page, as it
  is drawn (see [Templates](#templates)).
- **Template editor**: **Edit...** next to the Template panel's template field or **Tools > Templates > Edit
  template...** opens the project's template, **Tools > Templates > New template...** a
  new one; the project is put aside and the template opens in the window, under a
  colored bar saying which one, with **Back to project** (ends editing, asking to save
  first). **File > Save** (Ctrl+S) saves the template. A copy is named "name copy", or
  "name copy (2)" and so on when that name is taken. The **Template properties** panel
  ends with **Open folder** (the template's folder in the file manager, also in **Tools >
  Templates**; a new template has one once saved, the copy of a provided one shows the
  provided folder until then) and, in red, **Delete...** (your templates only, after
  confirming; editing ends; when the open project uses it, the dialog asks which
  template it uses instead, None by default, and Undo goes back). The
  template is shown centered on its page, as a project shows it; the **Template
  properties** panel, shown only meanwhile, sets its name, vendor, version and page,
  and the shapes move with a new page, kept in its middle (Undo goes back). A template
  needs a name and a version (the vendor is optional); the version is two numbers, x.y
  (a new template starts at 1.0): without one, or in another form, it is shown as a
  warning and can't be saved. A copy shows its **Source**, the template it was copied
  from ("Cosmox - T16 - v1.1"), and starts at its version. **Compare** (beside Source, or
  **Tools > Templates > Compare with source**) shows that source on the canvas left of a
  divider, the template being edited right of it, as a project compares an updated
  template; it only shows them (**End comparison**, at the bottom of the canvas, or
  Compare again, ends it). It is available when the source is installed. A template provided with the
  app can't be changed: editing it first says so and asks (**Edit a copy** or Cancel),
  then edits a copy, named "name copy", that saving puts into your templates, and the project uses the copy from then on. Each element is a group in the
  Layers panel (Outline, Action buttons...), its shapes inside; the Element button (an
  outline with a plus) adds one (Outline is grayed out once the template has one: a
  template has a single outline), and the Shape button adds a shape into the element
  selected (it waits for one). A group's Properties show its element's **Kind**. There are no canvases, groups or stacking order: Group,
  Ungroup and the move buttons are left out. A click on the canvas selects the smallest
  shape under it, so clicking inside a button or a cutout picks that hole rather than the
  outline around it (the outline is picked from its own area or the list). Every shape belongs to an element: a shape
  can be dragged from an element to another, never out of them, and pasted only into
  one. Shapes the SVG draws in no element (drawn outside its layers in Inkscape, say)
  are listed at the top in red, with an error mark (also on the canvas, in the middle of
  one selected, as the padlock of a locked layer): drag them into an element or delete
  them before saving.
  Shapes are drawn, moved, resized and point-edited as usual, in their element's color,
  with no fill, paint or effects; images, texts and canvases can't be added, and the
  Template, Page, View, Print and Colors in use panels are hidden meanwhile. A
  group's Properties set its element's color. The **Variables** panel stays: lengths,
  angles and numbers that the shapes' sizes, positions and rotations link to, as in a
  project (a changed variable moves or resizes every shape using it). They are saved in
  `template.json`, with each shape's links (see below). Circles come in as circles (with their
  size), rectangles as rectangles (rounded or not), other outlines as paths whose points
  can be edited. Saving writes `template.svg` (in mm, a layer per element:
  circles as circles, arcs as arcs, curves as Beziers) and `template.json`; an older
  template's DXF files are replaced by it. A project that uses the template takes the saved version when editing
  ends (Undo goes back).
- **Export** PNG, PDF and/or SVG. The Export window only asks what to export: the formats
  and which template lines are printed, saved with the project. How it is printed is
  set once for all projects in the **Print** panel: the resolution (72 to 1200 DPI),
  whether the outside of the outline and the inside of buttons and of cutouts are left
  blank or printed over, the bleed, the marks, mirroring and sheets. In the PDF, template lines, text and shapes stay sharp vectors
  (text as outlines, so no font is needed to print it), gradients included; text and
  shape layers with effects, inside a semi-transparent group, distorted with a gradient,
  or with a gradient whose colors are not all opaque, are printed as images like the
  rest. The SVG holds the same as the PDF, for a vector editor such as Inkscape or for a
  cutting machine: the artwork, each template element's lines and the marks are in
  layers of their own, and the images are embedded in the file.
- **Page sizes**: A5, A4, A3, A2, Letter, Legal and Tabloid, in portrait or landscape, or
  a custom width and height.
- **Print** straight from the app (**File > Print…**), at true size. The Print window
  shows the same preview and choice of lines as the Export window, shared between the
  two, and prints with the Print panel's settings. On Windows, the printer, copies
  and paper are chosen in the system print dialog, its paper preset to the page's or the
  sheets'; keep its scale at 100%. On Linux, printing goes through CUPS (`lp`): pick
  the printer and copies in the Print window.
- **Tiled printing**: a page larger than your paper is split over several sheets (A5 to
  Tabloid, portrait or landscape) in the Print panel: one PDF page, and one PNG and one
  SVG file (`name-r1c2.png`, `name-r1c2.svg`) per sheet. Each sheet keeps a blank margin, the artwork of
  neighboring sheets overlaps, and alignment marks in the margin show the corners of
  each sheet's artwork and where to trim one sheet to line it up with the next. The
  sheets are centered on the page; drag them on the export preview, or set their
  position, to place the joins where you want them.
- **Low resolution warning**: image layers that would print below a chosen resolution
  (150 DPI by default, set in the Print panel) are flagged with a ⚠ in the layer list,
  in Properties, in the Print panel and in the Export window. The resolution is measured where the image is
  stretched most, distortion included.
- **Mirrored export**: the whole print flipped left to right, template lines and marks
  included, to print on the back of clear film or plexi and see it the right way through
  it.
- **Print margins and marks**: a **bleed** (Print panel) prints the artwork a few mm
  past the outline and into the holes, so a cut slightly off the line leaves no blank
  edge: what is under that band is printed, so a shape meant to reach the cut should
  go past it by the bleed. **Preview bleed** (View panel) shows it in the
  cut preview, or turned off, the cut lines themselves. A **safe area** is
  shown as a dashed guide on the canvas, that far inside the outline and around the
  holes, to keep text and details clear of the cuts; never printed. Its width is set
  next to **Preview safe area** (View panel), which hides or shows
  it. The View panel's display settings (Show template lines, Preview cuts and its
  opacity, Preview bleed, Preview safe area and its width, Show rulers and Show guides) and the Print panel's settings are the app's, not
  the project's: saved as soon as they change, and back at the next start. The export can add **crop marks**
  in line with the edges of the outline's box and **registration marks** at the middle
  of its sides, both outside the bleed (Print panel).
- **Projects** are saved as a single `.artwork` file (a zip archive) holding the project,
  all its images and fonts and its own copy of its template, so it can be moved or shared
  on its own and opens the same where that template isn't installed.
  It keeps the Export window's choices too (formats and lines printed); changing them
  counts as an unsaved change, but not as an undo step. The Print panel's settings
  are the app's, the same for every project.
  **File > Open Recent** lists the last 10 projects. With unsaved changes, **New**,
  **Open**, **Open Recent** and closing the window first ask whether to save them
  (Save, Don't save or Cancel).
- **Autosave and crash recovery**: while a project has unsaved changes, a copy of it is
  kept every 2 minutes (**File > Autosave**: off, or every 1 to 30 minutes) in the app's
  data folder, never in your project file. If the app closes unexpectedly (a crash, a
  plugin failing), the next start offers the copy back: **Recover** opens it as the
  project it came from, with its changes unsaved; **Not now** asks again at the next
  start; **Discard** deletes it. The copy is deleted once the project is saved, its
  changes let go, or the app closed normally. Several windows of the app each keep
  their own. The right end of the status bar shows when the project was last saved
  ("Last saved 5 minutes ago") and, with unsaved changes, when their recovery copy was
  made ("Recovery copy just now"): the project file itself is only written when you
  save it.
- **Movable panels**: Template, Page, View, Print, Variables, Colors in use,
  Layers and Properties are each a panel. Drag a panel by its title onto the left or right side of
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

The app's icon is in [`assets/icon`](assets/icon): the SVG it is drawn from, PNGs from 16
to 1024 px and the Windows `.ico`. `build.rs` builds the `.ico` into the Windows
executable (with MinGW's `windres`; `WINDRES` names another one), and the app sets the
256 px PNG as its window icon. The Linux package also holds `artwork-creator.png` and
`artwork-creator.desktop`: to list the app in the desktop's menus, copy the PNG to
`~/.local/share/icons/` and the `.desktop` file to `~/.local/share/applications/`, with
its `Exec` line set to where the executable is.

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

`make version-check` suggests the next version, `make version-bump-patch` (or -minor,
-major, `V=X.Y.Z`) sets it, to merge through a pull request, and pushing a `vX.Y.Z` tag of
the merged `main`, made by hand, makes GitHub Actions build the packages into a draft
release to publish by hand. Don't create the release on GitHub: see
[Versioning and releasing](doc/releasing.md) for the steps and why.

## Templates

The app looks for the provided templates in a `templates` folder next to the executable,
then in the current directory, and for your own in the `templates` folder of its data
folder (`~/.local/share/artworkcreator/templates` on Linux,
`%APPDATA%\Artwork Creator\data\templates` on Windows). The provided ones are never
changed by the app, so an update can replace them: editing one (**Edit...** in the
Template panel) edits a copy, saved into your templates under a new name, and the
project uses the copy from then on. Only your templates are changed, from the template
editor (its **Template properties** panel: name, vendor, version and page, written
into their `template.json`) and deleted (projects that use one keep their own copy of it). One of yours named like a provided one
(the same folder name) is used instead of it. Each subfolder holding a `template.svg`
(or, for an older template, DXF files) is one template. The SVG draws every element, each
in a group of its own (an Inkscape layer), and a `template.json` file next to it
describes the elements, each by its group's `id` (the app always writes its fields in
the order shown below):

```json
{
  "name": "T16",
  "vendor": "Cosmox",
  "version": "1.1",
  "page": { "size": "A4", "orientation": "landscape" },
  "elements": [
    { "kind": "outline", "id": "outline", "name": "Outline" },
    { "kind": "buttons", "id": "action-buttons", "name": "Action buttons" },
    { "kind": "buttons", "id": "menu-buttons", "name": "Menu buttons" },
    { "kind": "cutouts", "id": "fastener-holes", "name": "Fastener holes", "color": "#ff8c00" }
  ]
}
```

The SVG can be edited in Inkscape: its units are worked out (a file in mm keeps its
size; one in `px` is read at 96 per inch), and so are transforms, styles and the basic
shapes. Only outlines count: text, images and fills are ignored. Every shape must be in
an element's group: shapes outside them (in a group named by no element, or in none)
are ignored by projects, and shown as errors in the template editor. An element whose
group is missing draws nothing.

The template editor adds what a template is made from. A copy names its `source`
(`{ "name": "T16", "vendor": "Cosmox", "version": "1.1" }`; its name and version are
required, its vendor optional: a `source` lacking its name or version is ignored, and a
copy of a template without a version gets none). `variables` lists the
template's variables, as a project saves them, and `bindings` the formulas of each
linked shape, by the `id` the editor gives that shape in `template.svg`; `bindings_origin`
is where the SVG's top-left corner was on the editor's page, X and Y formulas being page
positions (they are moved along when the template opens elsewhere on its page). Projects
only use the shapes as drawn:

```json
{
  "variables": [ { "name": "pitch", "kind": "length", "value": 24 } ],
  "bindings": { "action-buttons-1": { "x": "pitch * 2", "width": "pitch - 6" } },
  "bindings_origin": [20, 20]
}
```

Every template is checked when the templates are listed. One with problems has a
warning mark in the Template panel's list (red for errors, amber for warnings), and its
tooltip lists them. While a template is edited, the **Template check** panel lists its
problems as you work, errors first; click one about a shape to select it. **Check**, at
the bottom of the panel, checks it again at once. **Tools > Templates > Check template**
shows the panel again after it was closed. A template with errors can still be saved, but you are told it isn't valid (only a missing version, or one not of the form x.y, stops it).
What is checked:

- Files and settings: `template.json` can be read, its colors, page and units are
  understood, it has a version of the form x.y, each element names a group that
  `template.svg` has (or a DXF file that is there), no two elements name the same group,
  and no shape is in no element. A shape linked to a variable the template doesn't have
  is a warning. An element
  with no shapes is a warning.
- Outline: the template has one outline element (none or more than one is an error), made of one
  closed shape (a warning otherwise), and it fits on the page `page` asks for.
- Holes: buttons and cutouts lie inside the outline without crossing it, and don't
  overlap each other.
- Every shape, in any element, is closed.

- `kind` is `outline`, `buttons` or `cutouts`. The outline clips the artwork;
  buttons and cutouts are holes, left blank in the export unless turned off in the Print
  panel. **Preview cuts** (View panel) covers all of them on the canvas, at the
  **Cut opacity** set there, and hatches them, whatever the export prints: the panel is
  cut there anyway.
- The center of each closed shape (of its box: a circle's center) is a point images snap
  to on the canvas (**Snap to centers**, View panel), drawn as a small cross. Centers are
  not printed. An `anchors` element from an older version is ignored: the centers of the
  buttons replace it.
- An older template draws each element in its own DXF file, named by `"dxf":
  "outline.dxf"` instead of `id`; it is still read, and saving it from the template
  editor turns it into a `template.svg`. Its DXF files must share the same coordinates,
  as when they are exported from one drawing. The template is centered on the page as drawn, never turned or
  mirrored: draw it the way it sits on its page. Outlines drawn as separate lines and arcs are joined automatically.
- `units` (`mm`, `cm` or `inch`) is used when DXF files don't declare their units (the
  SVG says its own).
- `page` is the page the template is made for: `size` is `A5`, `A4`, `A3`, `A2`, `Letter`,
  `Legal` or `Tabloid`, or `[width, height]` in mm for a custom page; `orientation` is
  `landscape` or `portrait`. Picking the template, or starting a new project with it, sets
  them in the Page panel (what `page` leaves out stays as it was); a project that is
  opened keeps its own page. They can still be changed there afterwards.
- `vendor`, `name`, `color` (`#rrggbb`) and `page` are optional. Templates are listed
  by `vendor` in the Template panel, each vendor's models under its name (`name` is then
  the model's, shown as "Cosmox - T16" once picked); those without one come first. `name`
  defaults to the folder's name. A `placement` left by an older version is ignored.
- Without `elements`, the whole `template.svg` (or else the folder's single DXF file) is
  used as the outline.
- `version` (optional, such as `"1.2"`) tells versions of a template apart for people;
  the app tells them apart by their files' contents in any case.

A project archive keeps its own copy of its template (`template.json` and its SVG or DXF
files)
and always opens with it. When the installed template of the same folder differs from that
copy, now or after its files change on disk, a popup says so, with both versions, and
offers to **Compare** them (the installed one left of a divider on the canvas, the
project's right of it; drag the divider, and choose from the bar at the bottom of the
canvas), **Use installed** (the project takes it and keeps it; Undo goes back), or **Keep
project's** (not asked again for that installed version). Esc decides later: the Template
panel then has a **Review...** button that brings the popup back.

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

The keys of the commands can be changed in **View > Keyboard shortcuts**: click a
command's key and press the new one (Esc to keep it), **+** to give it another one, and
right-click a key to remove it; the trash at the end of the row removes them all. A key
can belong to one command only: one already used by another command is refused: the
window says which, and briefly highlights that command's row and key (shown whatever the
search); remove it there first. The circular
arrow at the end of the row gives a command its default keys back, those another command
does not use by then, and
**Reset all** does it for every command. Save As, Quit, Real size, the View panel's options, the template commands (New, Edit, Delete, Open folder, Check) and the window itself have no key by
default; you can give them one. Cut, Copy and Paste, and the arrow keys, keep theirs.
Search by command name or by key. While you type in a text field, only the File commands
work, and only with Ctrl or Alt held. The keys are kept between sessions; menus and
tooltips show the current ones.

The default keys:

| Action | Shortcut |
|---|---|
| New / Open / Save project | Ctrl+N / Ctrl+O / Ctrl+S |
| Export | Ctrl+E |
| Print | Ctrl+P |
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y |
| Add an image layer | Ctrl+I, or drop image files on the window |
| Add a text layer | Ctrl+T |
| Add a shape layer (the kind added last; others from the Shape button, Path... importing SVG files) | Ctrl+R, or drop SVG files on the window |
| Add a canvas to paint in (the kind added last; others from the Canvas button) | Ctrl+K |
| Type in a text layer on the canvas | Double-click it (straight text only); Esc or a click outside it to stop |
| Finish drawing a polyline or polygon | Double-click, Enter, Esc or right-click; a click on the first point closes it |
| Edit the points of a line, polyline, polygon or path | Double-click it; Esc or selecting something else to stop |
| Add a point while editing points | Double-click a segment |
| Make a point smooth or sharp | Double-click it while editing points |
| Place a smooth point while drawing | Press and drag |
| Cut / Copy / Paste layer | Ctrl+X / Ctrl+C / Ctrl+V |
| Duplicate | Ctrl+D |
| Group / Ungroup | Ctrl+G / Ctrl+Shift+G |
| Delete layer | Del or Backspace |
| Nudge by 1 mm (10 mm with Shift) | Arrow keys |
| Fit page in view | F |
| Select / Pencil / Brush / Eraser tool (with a closed shape selected) | V / P / B / E (Esc: back to Select) |
| Clear the paint in the selected shape | Ctrl+Shift+Delete |
| Add a guide | Drag from a ruler |
| Move / delete a guide | Drag it / drag it off the canvas |
| Type a guide's position | Double-click it |
| Pan | Space+drag, middle or right drag, or drag on empty canvas |
| Zoom | Mouse wheel, or Ctrl+scroll |

On the canvas, with a layer selected:

- **Corner handles** scale; **Shift** scales width and height freely.
- **Distort** (in Properties) makes corner handles move one corner at a time, for
  perspective. **Holding Ctrl** swaps between resizing and distorting while you drag.
- **Edge handles** stretch one side; **Alt** stretches both sides from the center.
- **Rotation handle** rotates; **Shift** snaps to 15° steps.
- **Ctrl+Alt** on a corner or edge handle of a shape painted in: its paint does the
  other way than its **Stretch paint** setting says, for that drag only.

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
to the template's shape centers unless **Alt** is held.

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
