# Artwork Creator: functional specification

What the app does, feature by feature, as a user sees it. [README.md](../README.md) covers
building the app and working on it; [ROADMAP.md](../ROADMAP.md) lists what is planned.

When a feature is added or changed, describe it here, in the section it belongs to.

## Contents

- [Overview](#overview)
- [Window and workspace](#window-and-workspace)
- [Projects](#projects)
- [Page](#page)
- [Templates](#templates)
- [Template editor](#template-editor)
- [Template file format](#template-file-format)
- [Layers](#layers)
- [Image layers](#image-layers)
- [Text layers](#text-layers)
- [Shape layers](#shape-layers)
- [SVG import](#svg-import)
- [Background layers](#background-layers)
- [Painting](#painting)
- [Eyedropper](#eyedropper)
- [Fills: colors, gradients and patterns](#fills-colors-gradients-and-patterns)
- [Transforming layers](#transforming-layers)
- [Opacity, blend modes and clipping masks](#opacity-blend-modes-and-clipping-masks)
- [Effects](#effects)
- [Plugins](#plugins)
- [Variables](#variables)
- [Snapping, rulers and guides](#snapping-rulers-and-guides)
- [View settings](#view-settings)
- [Print settings](#print-settings)
- [Export](#export)
- [Printing](#printing)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Languages](#languages)

## Overview

A desktop app (Windows and Linux) for designing printed artwork for arcade controllers and
fightsticks. A controller **template**, drawn in SVG, gives the panel's outline and its
holes (buttons, cutouts, fasteners). The user arranges images, texts and shapes on top,
and exports print-ready PNG, PDF or SVG at true size, or prints straight from the app. The
artwork is clipped to the panel outline and left blank in the holes.

## Window and workspace

### Menus

- **File**: New, Open…, Open Recent (with **Clear list**), Save, Save As…, Autosave,
  Export…, Print…, Quit. While a template is edited, only Save (the template) and Quit
  work.
- **Edit**: Undo, Redo, Cut, Copy, Paste, Duplicate, Delete.
- **View**: Panels (show or hide each panel, reset the layout), Theme (System, Light,
  Dark), Language, Keyboard shortcuts….
- **Tools**: Eyedropper; **Templates** submenu: New template…, Edit template…, Delete
  template…, Open template folder, Check template, Reference…, Compare with source, and
  **Rescan templates** (looks again for templates added, changed or removed on disk);
  **Plugins** submenu: Manage plugins… (see [Plugins](#plugins)).
- **Help**: About Artwork Creator (version and build date).

Menu entries show their current keyboard shortcut.

### Panels

Template, Page, View, Print, Variables, Colors in use, Layers and Properties are each a
panel (the template editor adds Template properties and Template check).

- Drag a panel by its title onto the left or right side of the window to dock it there,
  above or below the other panels of that side (a line shows where it goes), or anywhere
  else to float it over the canvas; drag a floating panel's title to move it, or onto a
  side to dock it again.
- The menu at the right of a panel's title docks it on either side, floats it, opens it in
  a window of its own (to put it on another screen; keyboard shortcuts work there too) or
  hides it.
- A click on a docked panel's title folds it.
- Panels on the same side share its height: small ones get what they need, the others
  share the rest and scroll. Drag the edge of a side to widen it, or the line between two
  docked panels to give the one above more or less height (double-click the line to give
  it the height it needs again).
- **View > Panels** shows hidden panels again and resets the layout. The layout is kept
  between sessions.

### Canvas navigation

- **Pan**: Space+drag, middle or right drag, or drag on empty canvas (or outside the
  page). Panning never changes the selection. A plain click on empty canvas, without
  dragging, selects nothing.
- **Zoom**: mouse wheel, Ctrl+scroll, or Ctrl++ / Ctrl+-. The zoom bar has **-** / **+**, the zoom in
  percent (100% is real size on a 96 DPI screen), **Fit** (fit the page in the view, F)
  and **Real size**.

### Theme

**View > Theme**: System (follows the operating system's light or dark setting), Light or
Dark. On Linux under X11 or WSLg, the system setting is read from the XDG desktop portal,
then GNOME's settings.

### Status bar

Shows messages and warnings (failed loads, unlinked texts…), and at its right end when the
project was last saved ("Last saved 5 minutes ago") and, with unsaved changes, when their
recovery copy was made ("Recovery copy just now").

### File dialogs

Native file dialogs are used where the system provides them. On Linux systems without an
xdg-desktop-portal or zenity (such as WSLg), a built-in file browser is shown instead:
places (Home, Desktop, Documents, Downloads, Pictures, current folder), Up, a file-type
filter, Ctrl+click to select several files. Saving over an existing file asks first.

### Dialogs and messages

A dialog's buttons sit at the right of its bottom row, the main one rightmost and Cancel on
its left. Explanations, tips and warnings are shown as callouts (Info, Tip, Warning).

## Projects

- A project is saved as a single `.artwork` file (a zip archive) holding the project, all
  its images and fonts and its own copy of its template, so it can be moved or shared on
  its own and opens the same where that template isn't installed. Fonts whose license
  forbids embedding are left out.
- It keeps the Export window's choices too (formats and lines printed); changing them
  counts as an unsaved change, but not as an undo step. The Print panel's settings are the
  app's, the same for every project.
- **File > Open Recent** lists the last 10 projects; a missing one says so.
- With unsaved changes, **New**, **Open**, **Open Recent** and closing the window first ask
  whether to save them (Save, Don't save or Cancel).
- Saving a project whose image files are missing warns which layers can't be shown.
- **Undo / redo** for every change to the project.
- Several windows of the app can be open at once; layers can be copied between them.

### Autosave and crash recovery

- While a project has unsaved changes, a copy of it is kept every 2 minutes (**File >
  Autosave**: off, or every 1 to 30 minutes) in the app's data folder, never in the
  project file.
- If the app closes unexpectedly (a crash, a plugin failing), the next start offers the
  copy back: **Recover** opens it as the project it came from, with its changes unsaved;
  **Not now** asks again at the next start; **Discard** deletes it. When several projects
  can be recovered, they are offered one after the other.
- The copy is deleted once the project is saved, its changes let go, or the app closed
  normally. Several windows of the app each keep their own.

## Page

The **Page** panel sets:

- **Size**: A5, A4, A3, A2, Letter, Legal and Tabloid, or a custom width and height in mm.
- **Orientation**: portrait or landscape.
- **Background**: the page's color, gradient or pattern (see [Fills](#fills-colors-gradients-and-patterns)).

Picking a template, or starting a new project with it, sets the page the template asks for
(what it leaves out stays as it was); an opened project keeps its own page. A template that
does not fit on the page is flagged.

## Templates

- A template is made of several **elements**, each a layer of the template's SVG, with its
  own color and visibility: one **outline** (clips the artwork) and any number of **holes**
  elements (buttons, cutouts, fastener holes: left blank in the export unless turned off
  in the Print panel).
- The **Template** panel picks the template (listed by vendor, each vendor's models under
  its name, those without one first; shown as "Cosmox - T16" once picked), shows or hides
  each element's lines and sets their color. A template with problems has a warning mark
  in the list (red for errors, amber for warnings), its tooltip listing them.
- The template is centered on the page, as it is drawn: never turned or mirrored.
- The center of each closed shape of the template is a point layers snap to (see
  [Snapping](#snapping-rulers-and-guides)), drawn as a small cross in the element's color.
  Centers are not printed.

### Where templates are found

- The **provided** templates: a `templates` folder next to the executable, then in the
  current directory; installed, `../share/artwork-creator/templates` from the executable.
  The app never changes them, so an update can replace them.
- **Your** templates: the `templates` folder of the app's data folder
  (`~/.local/share/artwork-creator/templates` on Linux,
  `%APPDATA%\Artwork Creator\data\templates` on Windows). One of yours named like a
  provided one (the same folder name) is used instead of it.
- Editing a provided template edits a copy, saved into your templates under a new name;
  the project uses the copy from then on.

### Template kept in the project

- A project archive keeps its own copy of its template and always opens with it.
- When the installed template of the same folder differs from that copy, now or after its
  files change on disk, a popup says so, with both versions, and offers to **Compare** them
  (the project's copy left of a divider on the canvas, the installed one right of it; drag
  the divider, and choose from the bar at the bottom of the canvas), **Use installed** (the
  project takes it and keeps it; Undo goes back), or **Keep project's** (not asked again
  for that installed version). Esc decides later: the Template panel then has a
  **Review...** button that brings the popup back.
- When the project's template isn't installed at all (deleted, or the project comes from
  another computer), opening the project asks whether to **Install** the copy it keeps into
  your templates, where it is listed and can be edited from then on; **Not now** (or Esc)
  opens the project with its copy. The Template panel's list then shows that template
  among the others, marked with a sheet as the project's own.

### Template check

Every template is checked when the templates are listed. While a template is edited, the
**Template check** panel lists its problems as you work, errors first; click one about a
shape to select it. **Check**, at the bottom of the panel, checks it again at once.
**Tools > Templates > Check template** shows the panel again after it was closed. A
template with errors can still be saved, but you are told it isn't valid (only a missing
version, or one not of the form x.y, stops it). What is checked:

- Files and settings: `template.json` can be read, its colors, page and units are
  understood, it has a version of the form x.y, each element names a group that
  `template.svg` has (or a DXF file that is there), no two elements name the same group,
  and no shape is in no element. A shape linked to a variable the template doesn't have is
  a warning. An element with no shapes is a warning.
- Outline: the template has one outline element (none or more than one is an error), made
  of one closed shape (a warning otherwise), and it fits on the page `page` asks for.
- Holes: holes lie inside the outline without crossing it, and don't overlap each other.
- Every shape, in any element, is closed.

## Template editor

- **Edit...** next to the Template panel's template field, or **Tools > Templates > Edit
  template...**, opens the project's template; **Tools > Templates > New template...** a
  new one. The project is put aside and the template opens in the window, under a colored
  bar saying which one, with **Back to project** (ends editing, asking to save first).
  **File > Save** (Ctrl+S) saves the template.
- A template provided with the app can't be changed: editing it first says so and asks
  (**Edit a copy** or Cancel), then edits a copy, named "Copy of name" unless you change the
  name there, that saving puts into your templates; the project uses the copy from then
  on. A copy is named "Copy of name", or "Copy (2) of name" and so on when that name is
  taken.
- The template is shown centered on its page, as a project shows it.
- A project that uses the template takes the saved version when editing ends (Undo goes
  back).

### Template properties panel

Shown only while a template is edited:

- **Name**, **Vendor** (optional), **Version** and **Page**. A template needs a name and a
  version; the version is two numbers, x.y (a new template starts at 1.0): without one, or
  in another form, it is shown as a warning and can't be saved. The shapes move with a new
  page, kept in its middle (Undo goes back).
- A copy shows its **Source**, the template it was copied from ("Cosmox - T16 - v1.1"), and
  starts at its version. **Compare** (beside Source, or **Tools > Templates > Compare with
  source**) shows that source on the canvas left of a divider, the template being edited
  right of it; it only shows them (**End comparison**, at the bottom of the canvas, Esc, or
  Compare again, ends it). It is available when the source is installed.
- **Open folder**: the template's folder in the file manager (also in **Tools >
  Templates**; a new template has one once saved, the copy of a provided one shows the
  provided folder until then).
- **Delete...**, in red: your templates only, after confirming; editing ends. When the open
  project uses it, the dialog asks which template it uses instead (None by default), and
  Undo goes back.

### Elements panel

- The Layers panel, renamed **Elements** while a template is edited. Each element is a
  group (Outline, Action buttons...), its shapes inside. A group's Properties show its
  element's **Kind** and set its color.
- The Element button (an outline with a plus) adds one (Outline is grayed out once the
  template has one: a template has a single outline); the Shape button adds a shape into
  the selected element (it waits for one).
- There are no canvases, groups or stacking order: Group, Ungroup and the move buttons are
  left out.
- A click on the canvas selects the smallest shape under it, so clicking inside a button
  or a cutout picks that hole rather than the outline around it (the outline is picked
  from its own area or the list).
- Every shape belongs to an element: a shape can be dragged from an element to another,
  never out of them, and pasted only into one. Shapes the SVG draws in no element (drawn
  outside its layers in Inkscape, say) are listed at the top in red, with an error mark
  (also on the canvas, in the middle of one selected): drag them into an element or delete
  them before saving.
- Shapes are drawn, moved, resized and point-edited as usual, in their element's color,
  with no fill, paint or effects; images, texts and canvases can't be added, and the
  Template, Page, View, Print and Colors in use panels are hidden meanwhile.
- Circles come in as circles (with their size), rectangles as rectangles (rounded or not),
  other outlines as paths whose points can be edited.
- The **Variables** panel stays: lengths, angles and numbers that the shapes' sizes,
  positions and rotations link to, as in a project (a changed variable moves or resizes
  every shape using it). They are saved in `template.json`, with each shape's links.

### Reference

A drawing to draw the template over: the Reference button (beside Element) or **Tools >
Templates > Reference...** takes a DXF, SVG, PNG or PDF file (a PDF's first page), shown at
its true size (a PNG from its resolution, 96 DPI when it says none), centered on the
template.

- It is the last row of the Elements panel, below a line, always under every element; it
  can't be dragged among them.
- Selected, it is moved, resized and turned on the canvas or in Properties, which also set
  its opacity; its eye hides it. Choosing another file replaces it, Delete removes it.
- Saving copies it into the template's folder as `reference.<extension>`, with where it
  lies in `template.json`. Only the template editor shows it: projects, export and print
  never do.

### Saving

Saving writes `template.svg` (in mm, a layer per element: circles as circles, arcs as
arcs, curves as Beziers) and `template.json`; an older template's DXF files are replaced by
it. Every shape drawn in one piece gets an `id` in the SVG, kept from save to save.

## Template file format

Each subfolder of a templates folder holding a `template.svg` (or, for an older template,
DXF files) is one template. The SVG draws every element, each in a group of its own (an
Inkscape layer), and a `template.json` file next to it describes the elements, each by its
group's `id` (the app always writes its fields in the order shown below):

```json
{
  "name": "T16",
  "vendor": "Cosmox",
  "version": "1.1",
  "page": { "size": "A4", "orientation": "landscape" },
  "elements": [
    { "kind": "outline", "id": "outline", "name": "Outline", "color": "#ff00ff" },
    { "kind": "holes", "id": "action-buttons", "name": "Action buttons", "color": "#00aaff" },
    { "kind": "holes", "id": "menu-buttons", "name": "Menu buttons", "color": "#00aaff" },
    { "kind": "holes", "id": "fastener-holes", "name": "Fastener holes", "color": "#ff8c00" }
  ]
}
```

- The SVG can be edited in Inkscape: its units are worked out (a file in mm keeps its
  size; one in `px` is read at 96 per inch), and so are transforms, styles and the basic
  shapes. Only outlines count: text, images and fills are ignored. Every shape must be in
  an element's group: shapes outside them (in a group named by no element, or in none) are
  ignored by projects, and shown as errors in the template editor. An element whose group
  is missing draws nothing.
- `kind` is `outline` (one per template) or `holes` (as many elements as needed).
  `buttons` and `cutouts`, from older templates, are read as `holes`; the template is saved
  with `holes`. An `anchors` element from an older version is ignored: the centers of the
  holes replace it.
- `vendor`, `name`, `color` (`#rrggbb`) and `page` are optional. `name` defaults to the
  folder's name. An element's `color` is its line color on the canvas and in the export,
  always written by the template editor; one missing is magenta for the outline, orange for
  holes. A `placement` left by an older version is ignored.
- `page` is the page the template is made for: `size` is `A5`, `A4`, `A3`, `A2`,
  `Letter`, `Legal` or `Tabloid`, or `[width, height]` in mm for a custom page;
  `orientation` is `landscape` or `portrait`.
- `version` (such as `"1.2"`) tells versions of a template apart for people; the app tells
  them apart by their files' contents in any case.
- Without `elements`, the whole `template.svg` (or else the folder's single DXF file) is
  used as the outline.
- An older template draws each element in its own DXF file, named by `"dxf":
  "outline.dxf"` instead of `id`; it is still read, and saving it from the template editor
  turns it into a `template.svg`. Its DXF files must share the same coordinates, as when
  they are exported from one drawing. Outlines drawn as separate lines and arcs are joined
  automatically. `units` (`mm`, `cm` or `inch`) is used when DXF files don't declare their
  units (the SVG says its own).

The template editor adds what a template is made from. A copy names its `source`
(`{ "name": "T16", "vendor": "Cosmox", "version": "1.1" }`; its name and version are
required, its vendor optional: a `source` lacking its name or version is ignored, and a
copy of a template without a version gets none). `variables` lists the template's
variables, as a project saves them, and `bindings` the formulas of each linked shape, by
the `id` the editor gives that shape in `template.svg`; `bindings_origin` is where the
SVG's top-left corner was on the editor's page, X and Y formulas being page positions
(they are moved along when the template opens elsewhere on its page). Projects only use
the shapes as drawn:

```json
{
  "variables": [ { "name": "pitch", "kind": "length", "value": 24 } ],
  "bindings": { "action-buttons-1": { "x": "pitch * 2", "width": "pitch - 6" } },
  "bindings_origin": [20, 20]
}
```

## Layers

The **Layers** panel lists the project's layers and groups, top of the list drawn on top.

### Kinds

- **Image**, **Text**, **Shape** (including SVG paths and background layers) and
  **Canvas** (a shape to paint in), added by the buttons under the list (Image, Text,
  Shape, Canvas), their shortcuts, or by dropping image or SVG files on the window.
- **Groups** hold layers and other groups; a group's opacity is applied to the group as a
  whole. A folded group hides its layers in the list.

### The list

- Each row has a thumbnail, the name, an eye (show / hide) and a padlock (lock / unlock).
- Rows are dragged to reorder or to move into or out of a group; the buttons move the
  selection up, down, to the top or the bottom of its group.
- Marks on a row: a bent arrow for a layer clipped to the one below; ⚠ for an image that
  prints at a low resolution; a broken chain, in the warning color, for links to
  variables that no longer exist (naming them in its tooltip; also on a folded group
  holding one); a missing image file is named in the tooltip.
- **Up** / **Down** select the layer above / below (in the layer list clicked last).
- With no layers yet, the panel says how to add some.

### Operations

- **Cut, Copy, Paste** layers and groups, within a project, into another one, or between
  two windows of the app. A paste goes where a new layer would (below). Copies land 5 mm from the
  original, each one a step further. The first paste of a cut, or of a copy from another
  window, lands in place. Layers pasted into another project bring their color variables
  along, unless that project gives the name to another variable.
- **Duplicate**: a copy whose name is taken is named "name copy", then "name copy (2)",
  and so on (the same for pastes).
- **Group** puts the selection in a new group ("Group n"), or adds an empty group;
  **Ungroup** takes its layers out.
- **Delete** removes the selection.
- **Hide**: a hidden layer is neither shown nor printed.
- **Lock**: a locked layer does not move or change on the canvas, but a click selects it
  when no unlocked layer is under the pointer (so the layer toolbar and Properties work on
  it); a padlock in its middle shows it is locked.

### Naming and placing new layers

- A new layer (added, imported, drawn or pasted) goes into a group only when that group
  is selected, at its top. With a layer selected, it goes at the top level of the list,
  just above that layer, or above the group holding it when it is in one. With nothing selected, it
  goes at the top of the stack. In the template editor, a new shape goes next to the
  selected one, in its element.
- New text and shape layers go in the middle of the template, or of the view when that is
  out of sight.
- A new shape or canvas is named with two random words, an adjective and a noun (Swift
  Otter, Golden Comet...), a name the project doesn't use yet. A new text says such a
  name, selected to be typed over, and is named after its text until renamed.
- An image layer is named after its file.

### Properties panel

Shows the selected layer or group (or says to select one):

- **General**: name, position (x, y), size (w, h) with an aspect-ratio lock, rotation with
  its angle dial, 3D tilt and perspective, opacity, blend mode, clip to layer below, Flip H
  / Flip V, **Fit page** (cover the whole page), **Fit template** (cover the template
  outline), **Reset** (fit inside the page, no rotation), Reset distortion, Reset 3D tilt,
  and **Align** (below).
- Then the sections of its kind: Image (source size in pixels, print resolution), Text,
  Shape, and Effects.
- A number field's unit is shown after the field.

## Image layers

- PNG, JPEG, WebP, BMP and GIF, added by the Image button (several at once) or by dropping
  files on the window. Images that can't be loaded are named in a message.
- The image file is never changed: effects, crops and transforms are applied when drawing.
- Properties show its source size in pixels and the resolution it prints at.
- **Low resolution warning**: image layers that would print below a chosen resolution (150
  DPI by default, set in the Print panel) are flagged with a ⚠ in the layer list, in
  Properties, in the Print panel and in the Export window. The resolution is measured where
  the image is stretched most, distortion included.

## Text layers

- Typed in place on the canvas (double-click a straight text), in any installed font
  (searchable list), bold and italic, multi-line text aligned left, centered or right,
  font size (pt), letter spacing (in thousandths of the font size), line spacing (in font
  sizes), color (or gradient or pattern) and outline thickness.
- Text layers move, scale, rotate, distort and take effects like images, and are drawn at
  the export resolution so they print sharp.
- Characters the font lacks (other scripts, symbols, emoji) are taken from the system's
  fallback fonts, or the built-in emoji font; Properties list the fonts used ("Also
  from"). Characters no installed font has are named.
- Fonts, fallback ones included, are saved inside `.artwork` projects (except those whose
  license forbids embedding them), so a shared project looks the same elsewhere. A font
  that is not installed and not in the project is shown with the built-in font, with a
  warning.
- A text can be linked to a text variable (see [Variables](#variables)).

### Typing on the canvas

While typing in a text layer on the canvas, the keys edit the text as in any text field:
arrows, Home / End, Shift to select, Ctrl+A / C / X / V, Ctrl+Z for the typing itself. A
click on the text moves the cursor, a drag selects, a double click selects a word. Esc or
a click outside stops. Dragging a handle stops typing and transforms the layer as usual.

### Text along a path

The **Path** of a text: Straight, **Arc** (bent by **Bend**: at 100% the first line makes
a whole circle; below 0 they bend down), **Circle** (the layer centered on it, so its
center snaps to a button's center; its **Radius** set), or **Shape layer** (following the
outline of a shape layer, curves included; a text following a shape moves with it).

- **Position** along the path (a share of its length, from its start: the top of a
  circle), **Other side** (the text on the other side of the path, running the other way:
  inside a circle, readable at its bottom) and **Baseline shift** (its distance from the
  path).
- Each letter stays upright on the path.
- Text on a path is typed in Properties, not on the canvas.

## Shape layers

- Kinds: **rectangle** (with rounded corners: corner radius), **ellipse**, **line**,
  **polyline**, **polygon**, **regular polygon** (number of sides), **star** (number of
  points and inner radius, as a share of the points' distance) and **path** (from
  [SVG import](#svg-import)). A shape keeps the kind it was made as.
- The Shape button opens a menu of the kinds (and Path..., Background); its shortcut adds
  the kind added last.
- **Stroke**: width (in mm, centered on the outline; resizing the shape keeps it), color
  (or gradient), opacity, dashes (solid, dashed, dotted, dash-dot), line ends (flat, round,
  square) and joins (sharp, round, beveled).
- **Fill**, for closed shapes: a color, gradient or pattern. A path's fill rule: nonzero
  or even-odd (inner outlines are holes).
- Shapes move, rotate, distort and take effects like images, and are drawn at the export
  resolution. Resizing a shape keeps its stroke width in mm.

### Drawing lines, polylines and polygons

Picked in the Shape menu, they are drawn on the canvas: each click places a sharp point,
and a press dragged places a smooth one, pulling out its curve handles. A click on the
first point closes a polyline into a polygon; a double click, **Enter**, **Esc** or a right
click finishes; a line is done at its second point.

### Editing points

Double-click a line, polyline, polygon or path (or **Edit points** in Properties) to edit
its points; Esc or selecting something else stops.

- Drag a point to move it, or double-click a segment to add a point there; the curve keeps
  its shape.
- The picked point shows its curve handles, and its neighbors the ones facing it: drag them
  to bend the curve. A smooth point's two handles turn together; **Alt** moves one on its
  own.
- Double-click a point to make it smooth or sharp. **Del** removes the point picked last
  and the **arrow keys** nudge it.
- While drawing or editing points, **Shift** keeps the segment at 15° steps, and points
  snap to the template's shape centers unless **Alt** is held.

## SVG import

An SVG file picked from **Path...** in the Shape menu or dropped on the window comes in as
shape layers above the selection.

- A group is made for each of the file's own groups that has an id or an opacity. When
  that makes several layers and groups, they all go in one group named after the file, so
  the layer list gets one entry.
- Fills, strokes (width, color, opacity, caps, joins, dashes as the nearest of the app's
  styles), linear and radial gradients and fill rules are kept.
- It keeps its true size, shrunk to fit the page if larger.
- A path made of several outlines (a letter with a hole, a logo in one piece) becomes a
  **path** shape, its fill rule (nonzero or even-odd) set in Properties and its points
  edited like a polygon's.
- Only shapes come in: texts, images, scripts, links, embedded HTML, filters, masks,
  clipping and patterns are left out (the app says which), and nothing the file points to
  is ever loaded. Too large files, and shapes past a maximum count, are refused with a
  message.

## Background layers

**Background** in the Shape button's menu (Ctrl+B) adds, at the bottom of the stack, a
shape filling the template's outline grown by the bleed, so the print has no white rim
where the cut drifts.

- It starts with the page background's color, gradient or pattern, and its fill, opacity,
  blend mode and effects change as any shape's.
- Unlike the page background, it can go anywhere in the stack, be hidden, or have layers
  clipped to it.
- It follows the outline: another template, a new version or another bleed redraws it,
  and it can't be moved, turned or reshaped by hand (nudging, aligning, flipping or
  turning a group leaves it where it is).
- A template with no closed outline can't have one (the app says so).

## Painting

Only a **canvas** is painted in: a closed shape added with the Canvas button (see
[Canvases](#canvases)). Other shapes, even closed ones, can't take paint. A shape painted
in by an older version of the app, before only canvases could be, is a canvas when the
project opens.

- While a canvas is selected, a tool bar at the canvas' top left has **Select**,
  **Pencil**, **Brush**, **Eraser** and the **Eyedropper**; the tool picked last comes back
  whenever a canvas is selected. With anything else selected, the paint tools and Clear
  the paint are hidden: only Select and the Eyedropper stay. Each tool has its color and width (in mm on the page) on the
  tool bar. Esc goes back to selecting.
- The **pencil** paints an even line.
- The **brush** comes in several kinds, picked in the menu its tool bar button opens (its
  icon shows the kind in use): **round** (thinning out at both ends), **calligraphy** (a
  flat nib at an angle set by turning the wheel on the tool bar, double-click for 45°:
  broad across it, thin along it), **marker** (square ends, at half the color's opacity,
  like a highlighter), **spray** (dots scattered around the line, denser in the middle, as
  thick as the density set on the tool bar) and **bristle** (thin lines side by side,
  swelling and running dry here and there, like a dry brush).
- The **eraser** cuts the selected shape's strokes where it passes; a stroke crossed in the
  middle becomes two, a spray loses the dots it passes over.
- **Clear the paint**, next to the tools, empties the selected shape of all its strokes at
  once (Undo brings them back).
- With a pen on Windows, the width follows its pressure.
- Strokes are vectors, sharp at any size and in the PDF, cut off at the shape's outline,
  over its fill (the background) and under its stroke (the border): painting never changes
  the shape.
- Resizing the shape (by its handles or its Size) stretches its strokes, or, with
  **Stretch paint** off in its Properties (shown for canvases only), leaves them where they are on the page, the
  shape showing more or less of them, as when its points are moved. Ctrl+Alt on a handle
  does the other way for that drag. Turned into a line or polyline, it hides them until it
  is closed again.

### Canvases

The **Canvas** button, next to Image, Text and Shape, opens a menu of the closed kinds
(rectangle, ellipse, polygon, regular polygon, star) and adds one to paint in, without
fill or stroke (its outline shows dashed on screen, not printed), with Select in hand:
half the page's size (an ellipse starts as a circle, its handles stretch it), or, for a
polygon, drawn point by point as from the Shape menu. Its shortcut adds the kind added
last.

## Eyedropper

On the canvas' tool bar (always there, with Select, when no canvas is selected) and
in **Tools > Eyedropper** (I).

- With it in hand, a swatch next to the pointer shows the artwork's color under it and its
  code (the page and its layers, without the template's lines, guides or handles).
- A click takes it: it becomes the paint's color and its hex code is copied to the
  clipboard.
- A Ctrl+click also adds it to the colors in use, first (each one before those added
  earlier, a thin line below them), in the Colors in use panel and in every color picker.
- Esc goes back to selecting.

## Fills: colors, gradients and patterns

A shape's fill and stroke, a text's letters and the page background can each be **Solid**
(one color), a **Linear gradient**, a **Radial gradient**, or (fills, texts and page) a
**Pattern**, or (a shape's fill) **Generated**, chosen with the switch next to its color.
A gradient's, pattern's or generator's settings show below it, under a **Settings** header
that folds them away.

### Color pickers

- Every color picker (text, outline, fill, stroke, effects, page background) shows the
  color variables and the colors already used in the project; a click on one applies it.
- Below its sliders, the color's **Hex** code, to type or copy (`#RRGGBBAA` sets the
  opacity too), and its **Opacity** in %. Hovering a color button or swatch shows its code
  and opacity.
- **Save this color** makes a color variable of the color and links to it (only for a color
  that can be linked: not the paint color, a color variable's own or a template element's
  line color).
- See [Variables](#variables) for linked colors.

### Gradients

- A linear gradient runs at an angle (0° upward, turning clockwise: 90° from left to
  right), from one corner of the layer's box to the opposite one.
- A radial one grows from a center (its position and radius can be set) in an ellipse the
  shape of the box.
- Its colors sit on a bar: click the bar to add one, drag one along it, right-click it to
  remove it (two always stay). The box under the bar, pointing at the selected one, sets
  its color, its place (a slider) and removes it.
- Each color has its own opacity and can be linked to a color variable.

### Pattern fills

- An image (PNG, JPEG, WebP, BMP, GIF) repeated side by side: pick **Pattern** in the
  fill's switch, then the image; a click on its name chooses another. An image that can't
  be loaded shows the color or gradient instead.
- **Tile** sets the width of one copy (its height follows the image), **Angle** turns the
  copies and **Offset** moves where they start. A new pattern's tile is the image's size
  printed at 300 dpi.
- The pattern moves and turns with its layer, and keeps its tile's size when a shape is
  resized; a text's grows with the text.
- The image is saved in the project file.
- In PDF and SVG exports the shape's or text's outline stays vector: SVG gets an SVG
  pattern, PDF the image repeated under the outline; a layer that is distorted, or would
  need over 4000 copies of the image, is exported as pixels.

### Generated fills

- A shape's fill drawn by a generator: pick **Generated** in the fill's switch, then the
  generator in the menu next to it; its settings show below. Its sizes are in mm.
- It is made again at the export resolution, so it stays sharp at any size; resizing the
  shape shows more or less of it. PDF and SVG exports get it as an image under the
  outline.

| Generator | Settings |
|---|---|
| Noise | grain, color 1, color 2 |
| Clouds | size, detail (1 to 8), color 1, color 2 |
| Checkerboard | square size, color 1, color 2, angle |
| Stripes | width, color 1, color 2, angle |

- The circular-arrow button next to the generator menu (Noise, Clouds, and a plugin's
  generator with a seed setting) draws another random picture with the same
  settings. A new generated fill, or one switched to another generator, starts with a
  random picture too. The picture is saved with the project.
- More generators come from [plugins](#plugins), listed under the plugin's name. A project
  using one whose plugin is not installed still opens: it shows as "Missing generator",
  is not drawn, and is saved unchanged.
- Its colors can be linked to color variables, as effects' are.

## Transforming layers

### On the canvas

With a layer selected:

- **Drag** it to move it; the **arrow keys** nudge it by 1 mm (10 mm with Shift). The arrow
  keys don't move a layer along a linked X or Y.
- **Corner handles** scale; **Shift** scales width and height freely.
- **Edge handles** stretch one side; **Alt** stretches both sides from the center.
- **Rotation handle** rotates; **Shift** snaps to 15° steps. A selected group has one too,
  above its dashed box, turning the whole group.
- **Ctrl+drag on a corner handle** moves that corner alone, distorting the layer for
  perspective; **Reset distortion** in Properties puts the corners back.
- **Ctrl+drag on the selected layer** tilts it in 3D: up and down tip its top, left and
  right turn its sides (**Shift** snaps to 15°).
- **Ctrl+Alt** on a corner or edge handle of a canvas: its paint does the other
  way than its **Stretch paint** setting says, for that drag only.

### In Properties

- **Position**, **Size** (with the aspect-ratio lock) and **Rotation**, typed or linked to
  variables.
- **3D tilt** (under Rotation) turns a layer in 3D around its center: **x** takes its top
  away from you, **y** its right side (negative values bring them toward you), up to 85°.
  **Perspective** sets how strong the depth looks, from 0 (parallel lines stay parallel) to
  1. Effects such as a halftone are applied before, so they recede with the layer. The
  tilts can be linked to variables like the rotation; corners moved with Ctrl move on top
  of the tilt, and **Reset** clears both; **Reset 3D tilt** (next to Reset distortion)
  clears only the tilt and its perspective.
- **Angle dials**, next to each angle's field in the panels (a layer's rotation, a group's
  turn, a linear gradient's and a pattern's angle, an effect's angles, the calligraphy
  nib): drag or click on one to turn it; **Shift** snaps to 15° steps; a double click puts
  it back to its default. The angle fields themselves aren't dragged: click one to type a
  value.
- **Fit page**, **Fit template**, **Reset**, as listed under [Properties panel](#properties-panel).

### Align

**Align** layers or groups to the **Page** or to the **Template**: left, center
horizontally, right, top, center vertically, bottom edges.

### Flip

**Flip H**, **Flip V** in Properties. A layer is mirrored in place; a group is mirrored as
a whole around its middle, its layers swapping sides and mirrored too (locked ones stay as
they are; a linked position or rotation keeps its formula's value).

### Rotate a group

As a whole around its middle: with the rotation handle above its dashed box on the canvas
(**Shift** snaps to 15° steps), a quarter turn with **-90°** / **+90°** in its Properties,
or by an exact angle with **Rotate by** and **Rotate**. Its layers go around the middle and
turn with it; as for Flip, locked ones stay put. A group has no angle of its own: each
turn is added to its layers'.

## Opacity, blend modes and clipping masks

### Opacity

Each layer and group has an opacity; a group's is applied to the group as a whole.

### Blend modes

A layer's or group's **Blend mode** in Properties (Normal, Multiply, Screen, Overlay,
Darken, Lighten, Color dodge and burn, Soft and Hard light, Difference, Exclusion, Hue,
Saturation, Color, Luminosity) sets how its colors mix with what lies below it, the page
background included.

- A group in Normal mode lets each of its layers mix with what lies below the group; in
  another mode, its layers are put together first and the group mixes as a whole.
- A layer clipped to another mixes with it.
- PDF and SVG exports keep the modes (PDF blend modes, SVG `mix-blend-mode`), so text and
  shapes stay vectors; a text or shape made of several shapes (an outline, a fill and a
  stroke) is exported as an image when in another mode than normal.

### Clipping masks

**Clip to layer below** (in a layer's right-click menu, its Properties, Ctrl+Alt+G, or
Alt+click on the line between two layers in the list) shows a layer or group only where
the one right under it in the list, its base, shows, and as much as it shows: soft edges,
opacity and effects included.

- Several layers in a row can be clipped to the same base, an image, a text, a shape or a
  group; a bent arrow before their thumbnail marks them.
- Hiding the base hides them too; at the bottom of a list, with nothing below, the setting
  does nothing.
- Grouping a clipped layer clips the new group instead, and ungrouping a clipped group
  clips its layers.
- In PDF and SVG exports, layers clipped to a text or shape layer are clipped by its
  outline as vectors when that is what its pixels show (no effects, opaque); otherwise the
  base and its clipped layers are exported as one image.

## Effects

Each image, text and shape layer has a stack of effects, in the **Effects** section of its
Properties: **Add effect** adds one; each can be toggled, reset to its defaults, removed,
and dragged to reorder (applied top to bottom). They are applied to the export at full
resolution and never change the image file. Sizes of a text's or shape's effects are on the
page (a text's follow the font size).

| Effect | Settings |
|---|---|
| Brightness / Contrast | brightness, contrast (-100 to 100%) |
| Hue / Saturation | hue (-180° to 180°), saturation, lightness |
| Grayscale | amount |
| Sepia | amount |
| Tint | color, mode (colorize, multiply, screen), amount |
| Invert | |
| Posterize | levels (2 to 64) |
| Threshold | level (0 to 255), white on black |
| Gaussian blur | radius |
| Sharpen (unsharp mask) | amount, radius, threshold |
| Drop shadow | color, opacity, light angle, distance, softness |
| Outline | color, width, position (outside, center, inside), dashes (solid, dashed, dotted, dash-dot), opacity |
| Fade (linear) | toward (right, left, top, bottom), fade start, fully transparent at, curve |
| Fade (radial) | center x / y, fade start, fully transparent at, curve, transparent center |

- The fades work along the layer's visible pixels, so margins added by a shadow or outline
  do not count. Their **curve** is an easing curve as in animation tools: Linear, Ease in,
  Ease out, Ease in-out, or your own by dragging the two handles of its graph.
- Angles have an angle dial; colors a color picker; numbers can be linked to variables.
- More effects come from [plugins](#plugins). A plugin's effect with a seed setting (a
  random variation) shows a circular-arrow button in its place, which gives another random result
  with the same settings, as for [generated fills](#generated-fills). Such an effect
  starts with a random result when added; **Reset to defaults** keeps it. It is saved with
  the project.

## Plugins

Plugins are shared libraries (`.dll` on Windows, `.so` on Linux) or Python scripts
(`.py`). They add effects and generators (see [Generated fills](#generated-fills)). The
app loads those of every `plugins` folder it finds, in this order:

- next to the executable (or in `../share/artwork-creator/` from it, where the Linux
  packages install their files);
- **your** plugins: the `plugins` folder of the app's data folder
  (`~/.local/share/artwork-creator/plugins` on Linux,
  `%APPDATA%\Artwork Creator\data\plugins` on Windows), to create.

Never from the current directory, which may be any folder, such as the one a project was
opened from: plugins found there would run without being asked for. Debug builds also
look there, so `make run` finds the `plugins` folder of the repository.

- The app loads them at startup and lists their effects in **Add effect**, and their
  generators in a fill's **Generated** menu (an entry's tooltip gives its plugin's name
  and version), grouped by source:
  "Local plugins" (those not installed from a repository), then each repository's under
  its name (its address in the tooltip; "Unknown repository" until its list was read). A library that
  fails to load, or was made for a plugin interface version this app doesn't know, is
  skipped and reported at the bottom of the Add effect menu. Plugins made for version 1
  of the interface, which had effects only, still load. A Python plugin installed from a
  repository for a plugin SDK older than the oldest this app runs (one the SDK has since
  changed in a way that breaks it) is skipped and reported the same way, asking to
  update it.
- Python plugins need Python 3.8 or later. Every package comes with one, the same
  version everywhere, which it uses first: the Windows ones (zip, installer, Microsoft
  Store) in a `python` folder next to the app, the Linux ones in a `python` folder next
  to it (the `.tar.gz`) or in `../lib/artwork-creator/python` (the `.deb`, the
  AppImage). On Linux that Python is isolated from the system's (no user site-packages,
  no `PYTHON*` variables), which is used only by debug builds (`python3` or `python`).
  On Windows, without it, Python is found as `py -3` or `python`. Without any, Python
  plugins are skipped, and the Add effect menu reports "Python plugins need Python 3,
  which was not found". The Python that comes with the app starts with the standard
  library only: the other packages a plugin needs are installed when it is allowed
  (below).
- **A plugin runs only once you allow it**, a library or a Python plugin, wherever it
  is. At startup, a plugin file that is new, or has changed since you allowed it, is not
  loaded: once the window is open (after the question about recovering unsaved work),
  the app opens the plugin manager, where it is "Waiting to be allowed" with an **Allow**
  button. Allowing loads it at once (its effects and generators appear, and a project
  using them is drawn again); the file is remembered with a SHA-256 of its content, in
  `plugins.json` in the app's data folder, so a plugin that changes waits again. One not
  allowed stays off, and waits again at the next start. A waiting plugin shows its file
  name and "?" for its version: it has not run yet.
- **Tools > Plugins > Manage plugins…** lists every plugin file found, with its name,
  version, type (Python or Library) and what became of it: Loaded, Disabled, Waiting to
  be allowed, or Failed to load (why in its tooltip). Its check box
  enables or disables it; a disabled plugin is not loaded at all, nor asked about.
  **Edit**, on a Python plugin's row, opens its file in your text editor (never runs it:
  on Windows, its "edit" action, or Notepad). Enabling and disabling apply at the next
  start (the window says so; its **Restart** button closes the app and starts it again
  with the same project, asking first to save unsaved changes): until then, the effects
  of a plugin disabled stay available. The choices are kept in `plugins.json`.
  **Uninstall**, on the row of a plugin in **your** plugins folder (not one that comes with
  the app), removes its file (set aside as `.<name>.old` beside it, never loaded, as
  Update does) and what `plugins.json` says of it; the plugin leaves the list, and its
  effects stay available until the next start (the window offers the restart). The
  packages a Python plugin needed stay installed.
  These are on its **Installed** tab, where the plugins are grouped by the repository
  they were installed from (its name above them, its address in the tooltip; by name),
  then those that come from none (shipped with the app or put in a plugins folder by
  hand) under "Not from a repository".
- **Installing plugins from a repository**: the manager's **Available** tab lists the
  plugins of one repository at a time, picked in its drop-down (the official one by
  default, else the first; its address in the tooltip) among those listed on its
  **Repositories** tab (name, address, number of
  plugins or why its list could not be read), a list of plugins on the web (a `plugins.json` file, format
  described in `src/render/effects/repository.rs`). The official one,
  [artwork-creator-plugins](https://github.com/bzhmaddog/artwork-creator-plugins), is
  there to begin with; **Add repository** adds another by the address of its list, which
  is downloaded and checked first (a list that can't be read or is not valid is not
  added, and the window says why). **Remove** takes a repository out, the official one
  too (add its address again to get it back); the plugins installed from it stay
  installed. The repositories are kept in `plugins.json`; their lists are downloaded
  each time the plugin manager opens.
  The search field filters the plugins of that repository by name, description, author
  or the name of one of their effects or generators. Each plugin shows the latest version that works with this version of the
  app on this system (what its `engines` asks of the app and, for a Python plugin, of the
  plugin SDK, a library's plugin interface version, and a file for this system;
  "Not available" otherwise) and, in its **Provides** column, how many effects and
  generators that version adds ("1 effect, 1 generator", their names in the tooltip; empty
  when the list does not say). **Install** downloads its file (the `.dll`, `.so` or `.py`)
  into the repository's folder in **your** plugins folder (named with a hash of the
  repository's address, with a `source.json` giving its name and address, written again
  whenever the repositories' lists are downloaded), checks its size and SHA-256 against the list (a file that
  differs is not kept), installs the packages a Python plugin needs as **Allow** does, and,
  since you chose to install it, allows and enables it: it is loaded at once, and can be
  disabled later on the Installed tab like any other. A plugin installed from that
  repository already shows "Installed". The same plugin from two repositories is two
  plugins: both can be installed, their effects and generators are listed apart (each
  under its repository), and a project using one
  shows the effect as missing or "Missing generator" where only the other is installed.
- **Installed plugins a repository lists** are recognized by the SHA-256 of their file,
  or, for one installed from a repository whose list no longer has that version, by the
  id and version its `source.json` recorded (one edited by hand, or that no repository
  lists, is not; one installed from a repository, by that repository's list only): the
  Installed tab shows the
  repository's description in its **Description** column, what the installed version
  provides in its **Provides** column, and when a later version
  works here (none past the plugin's `latest` in the list), "New: x.y.z" highlighted
  next to the version (what changed in its tooltip) and an **Update** button. When that
  version's `compatible_from` is later than the installed one (it can't read the
  settings the installed version saved), "New: x.y.z" is in the warning color and its
  tooltip and Update's say so.
- **Plugin settings in projects**: an effect or generator of a plugin installed from a
  repository saves that plugin's id and version with its settings. Install and Update
  record the version's `compatible_from` in `source.json`. When a project is opened,
  settings saved by a version older than the installed one's `compatible_from` are
  reported in one message listing their layers, since they may now be read differently;
  every saved version is then set to the installed one, so the project no longer warns
  once saved. Update downloads and checks the new file like Install, in the
  folder of the old one, which is replaced (set aside as `.<name>.old` beside it, never
  loaded: a library in use can't be deleted on Windows); the new file is allowed, keeps
  the old one's enabled or disabled state, and is used from the next start (the row says
  "Updated to x.y.z" and the window offers the restart).
- **At startup**, if plugins are installed, the app downloads the repositories' lists in
  the background (one that can't be read is skipped, without a message). If new versions
  are found, once the window is open and the other startup questions answered it asks
  whether to review them: **Review** opens the plugin manager on its Installed tab, the
  new versions highlighted; **Not now** (or Esc) asks again at the next start. When the
  plugin manager is already open (plugins waiting to be allowed), it just shows them.
- **Packages a Python plugin needs** (numpy...) are listed in a block at the top of its
  file, the standard one of Python's inline script metadata (PEP 723):

  ```python
  # /// script
  # dependencies = ["numpy>=1.24", "pillow"]
  # ///
  ```

  With the Python that comes with the app, which has pip, **Allow** first installs
  those not already there (from PyPI, into `python-packages` in the app's data folder,
  which every Python plugin has on its path; its tooltip lists them), then loads the
  plugin. If the install fails, the plugin stays waiting and the window says why (pip's
  last line). With another Python, nothing is installed: the Allow tooltip lists the
  packages, to install for that Python. To remove a package, delete its folder in
  `python-packages`.
- Each Python plugin runs in a Python process of its own, started with the app and kept
  running: its calls wait their turn. If it crashes, only that call fails, like an effect
  error of a native plugin (the layer is drawn without it, the error goes to the log), and
  the next call starts it again.
- **Install only plugins you trust.** A plugin is native code running inside the app with
  all its rights: it can read and write your files, and a crash in it closes the app (the
  autosave copy is then offered back). A Python plugin has your rights too, though a
  crash in it doesn't close the app.
- A project that uses an effect whose plugin is not installed still opens: the effect is
  shown by its name in red (saved with it; its id for one saved before), unchecked, with
  an error box saying it is missing; it is not applied, and is saved unchanged. An effect saves the repository plugin it came from and
  its version (recorded in the repository folder's `source.json` when the plugin is
  installed or updated): when the same repository, among yours, lists that plugin at the
  same version or a later one that works here, the box has an **Install** button, which
  installs it as the plugin manager does (a spinner meanwhile; the effect shows again once
  loaded, or the box asks for a restart). Effects added before this, or from plugins put
  in a plugins folder by hand, have no such button.
- Effects and generators from plugins show their own names, untranslated.

Writing a plugin: see "Plugins" in [README.md](../README.md#plugins).

## Variables

Named values kept with the project, in the **Variables** panel, each of a kind: a
**length** in mm (`button = 30`), an **angle** in degrees, a **text size** in pt, a
**number** without a unit (`count = 3`), a **color** or a **text**.

- **Add a variable** asks for the kind; the panel lists the variables grouped by kind, each
  behind its kind's icon, with its unit after its value and how many fields use it. A
  search box narrows the list.
- A name starts with a lowercase letter or `_` (not a capital or a digit), then letters,
  digits or `_`; its field turns red otherwise, or when another variable has the name.

### Template variables

- The project's template brings its own variables (set in the template editor): the panel
  then has two sections, **Template** first, in tinted read-only rows, then **Project**,
  the project's own. Formulas use the template's like the others
  (`actionButtonDiameter + 2`). They come from the project's copy of its template, not
  saved with the project's variables; a project variable can't take one of their names
  (one that already does is used instead).
- A project length, angle, text size or number can inherit from the template's variables:
  type a formula such as `templateVar + 2` in its value, or use its **fx** button (it lists
  the template's variables only). It then follows the template's variables, and the layers
  using it follow too. If the template no longer has a variable the formula uses (the
  template replaced), the variable's name turns red, a warning under it names the missing
  variable, and it keeps its last value until the formula is changed or unlinked.

### Linked fields and formulas

- Type a variable's name, or a formula such as `button + 2` or `(panel - 2 * margin) / 3`,
  in a layer's width, height, x, y, stroke width or corner radius field (lengths),
  rotation and tilt fields (angles) or text size field (text sizes) to link it: the field
  then shows the result (the formula is in its tooltip and its fx button is filled with the
  link color), and follows when a variable changes.
- Formulas use numbers, variable names, `+ - * /` and parentheses. A field takes the
  variables of its kind and numbers; a formula using another kind, an unknown name, a
  division by zero or a syntax error shows an error and leaves the field as it is.
- Typing a plain number in a linked field, or dragging its value, unlinks it, and so does
  moving, resizing or turning the layer some other way (on the canvas or with Align).
- The **fx** button inside each of those fields, at its right end, opens the formula
  editor: the formula (the field's value to start with), its result or what is wrong with
  it, and the variables the field takes, a click inserting one at the cursor: the
  template's points come folded by family (`action_buttons (16)`), and a search box above
  them narrows the list. **OK** (or Enter) links the field to the formula, or just sets the
  value when it names no variable; **Unlink, keeping the value** unlinks the field. The fx
  becomes a badge filled with the link color when the field is linked, so the fields using
  variables stand out, and with the warning color when its variable is gone.
- With the size's aspect locked, linking the width also links the height to the same
  formula scaled by their ratio (`button` gives `button * 0.5` for a layer half as tall as
  wide), and the other way round; unlinking one unlinks both, and locking the aspect with
  only one of them linked links the other.

### Linked colors

A color picked from the color variables stays linked to it (a chain link is drawn inside
its square swatch, the variable's name in its tooltip) and follows when the variable
changes; picking another color, or clicking the linked color variable again in the picker
(it shows a chain), unlinks it.

### Linked texts

A text layer's text is linked to a text variable with the link button at the top right of
its text box: it then shows the variable's text and follows when it changes, so one
variable can fill several layers. Typing in the text, in the panel or on the canvas,
unlinks it, with a warning in the status bar (undo links it again); **Unlink, keeping the
value** unlinks it too.

### Colors in use

The **Colors in use** panel, below Variables by default, lists the colors in use one per
row, as `#RRGGBB` followed by its opacity (`100%`). Double-click one to add a color
variable of that color, even if another color variable has it.

### Renaming and removing

- Renaming a variable renames it in the formulas and links.
- Removing one that is in use asks first; the fields and colors using it keep their value
  and show a broken link icon (hover it to see what it was linked to). Typing a value in
  such a field, or picking a color, unlinks it; adding a variable of the same name links it
  again.
- In the layer list, a layer with such broken links shows a broken chain, in the warning
  color, naming the variables in its tooltip; so does a folded group holding one.

## Snapping, rulers and guides

### Snap to centers

The centers, corners and edge middles of images snap to the center of each closed shape of
the template (its holes and outline), drawn as a small cross in the element's color. So do
a shape's center, the corners and side middles of its box, or its points (its stroke left
out). Hold Alt to drag freely. On or off with **Snap to centers** in the View panel.

**Linking a position to a template shape**: when a single layer is dropped on the center of
a template shape that has an id in the template's SVG, the app asks whether to link its
position to it. Its X and Y then become formulas over the shape's position
(`action_buttons_3_x`, `action_buttons_3_y`, plus the offset of the snapped corner or
edge), so the layer follows the shape when the template changes. Edit the formulas to
change the offset. A layer whose position is linked stays linked while it is dragged, a
warning next to the pointer while it is near where it started: released there, it goes
back to its linked position; released farther, it is unlinked and stays where it is
dropped. If the shape is no longer in the template, the layer stays put and its links show
as broken.

### Snap to page and outline

While a layer or group is moved, its edges and middle snap to the page's edges and middle,
and to the template's outline: the edges and middle of its bounds, its straight sides, and
the bleed line when there is a bleed. An edge being stretched and a corner being resized
snap too. A dashed line shows the guide it snapped to. Hold Alt to drag freely, or turn it
off with **Snap to page and outline** in the View panel.

### Rulers and guides

- Rulers along the canvas' top and left edges measure in mm from the page's top-left
  corner.
- Drag from a ruler to add a guide: a horizontal one from the top ruler, a vertical one
  from the left ruler. Drag a guide to move it, off the canvas (onto a ruler) to delete it,
  or double-click it to type its position (or delete it).
- Guides are saved with the project and never printed.
- While moved or resized, layer edges and centers snap to them as they do to the page; a
  dragged guide snaps to the edges and centers of the layers, the page and the outline.
  Hold Alt to drag freely.
- The View panel shows or hides the rulers and the guides (hidden guides are neither
  snapped to nor moved), turns snapping to guides on or off, locks the guides so they
  can't be moved, deleted or edited by accident, and clears them all.

## View settings

The **View** panel's settings are the app's, not the project's: saved as soon as they
change, and back at the next start.

- **Show template lines**: the template's outline and holes on the canvas. Never printed.
- **Preview cuts** covers what is cut out of the panel (outside the outline, inside the
  holes) on the canvas, at the **Cut opacity** set there, and hatches it, whatever the
  export prints: the panel is cut there anyway.
- **Preview bleed** shows the cut preview with the bleed set in the Print panel (the holes
  smaller and the outline larger by it), or, turned off, the cut lines themselves.
- **Preview safe area** shows the safe area as a dashed guide on the canvas, that far
  inside the outline and around the holes (its width set next to it), to keep text and
  details clear of the cuts. Never printed.
- **Snapping**: Snap to centers, Snap to page and outline, Snap to guides.
- **Rulers and guides**: Show rulers, Show guides, Lock guides, Clear guides.

## Print settings

How the artwork is printed is set once for all projects in the **Print** panel, saved as
soon as it changes:

- **Resolution**: 72 to 1200 DPI, for the PNG and the images in the PDF and SVG (a printer
  on Windows prints at its own resolution). The page's pixel size is shown.
- **Leave blank**: the outside of the outline and the inside of the holes, each left blank
  or printed over.
- **Bleed**: the artwork printed a few mm past the outline and into the holes, so a cut
  slightly off the line leaves no blank edge: what is under that band is printed, so a
  shape meant to reach the cut should go past it by the bleed.
- **Marks**: **crop marks** in line with the edges of the outline's box and **registration
  marks** at the middle of its sides, both outside the bleed. Marks falling off the page
  or the sheets are flagged.
- **Mirror**: flips the whole print **Horizontally** (left to right) and / or
  **Vertically** (top to bottom), template lines and marks included, to print on the back
  of clear film or plexi and see it the right way through it.
- **Warn below**: the resolution under which image layers are flagged (150 DPI by
  default; see [Image layers](#image-layers)).
- **Split over several sheets** (tiled printing), below.

### Tiled printing

A page larger than your paper is split over several sheets (A5 to Tabloid, portrait or
landscape): one PDF page, and one PNG and one SVG file (`name-r1c2.png`, `name-r1c2.svg`)
per sheet.

- Each sheet keeps a blank **margin**, the artwork of neighboring sheets **overlaps**, and
  **alignment marks** in the margin show the corners of each sheet's artwork and where to
  trim one sheet to line it up with the next.
- The sheets are centered on the page; drag them on the export preview, or set their
  **position**, to place the joins where you want them (**Center** puts them back).
- The number of sheets (columns x rows) and their pixel size are shown.

## Export

**File > Export…** (Ctrl+E) exports PNG, PDF and/or SVG. The Export window only asks what
to export: the **formats** and which template lines are **printed**, saved with the project,
with a preview. It shows the low-resolution warning and, for the rest, points to the Print
panel.

- The export runs in the background with a progress window (rendering, saving PNG, PDF,
  SVG) and can be canceled. Replacing existing files asks first.
- **PNG**: the page (or each sheet) at the Print panel's resolution.
- **PDF**: template lines, text and shapes stay sharp vectors (text as outlines, so no font
  is needed to print it), gradients included. Text and shape layers with effects, inside a
  semi-transparent group, distorted with a gradient, or with a gradient whose colors are
  not all opaque, are printed as images like the rest.
- **SVG**: the same as the PDF, for a vector editor such as Inkscape or for a cutting
  machine: the artwork, each template element's lines and the marks are in layers of their
  own, and the images are embedded in the file.
- See also blend modes, clipping and patterns above for how they are exported.

## Printing

**File > Print…** (Ctrl+P) prints straight from the app, at true size, with the Print
panel's settings.

- The Print window shows the same preview and choice of lines as the Export window, shared
  between the two.
- On Windows, the printer, copies and paper are chosen in the system print dialog, its
  paper preset to the page's or the sheets'; keep its scale at 100%.
- On Linux, printing goes through CUPS (`lp`): pick the printer and copies in the Print
  window.
- Printing runs with a progress window (rendering, sending to the printer) and can be
  canceled.

## Keyboard shortcuts

The keys of the commands can be changed in **View > Keyboard shortcuts**:

- Click a command's key and press the new one (Esc to keep it), **+** to give it another
  one, and right-click a key to remove it; the trash at the end of the row removes them
  all.
- A key can belong to one command only: one already used by another command is refused:
  the window says which, and briefly highlights that command's row and key (shown whatever
  the search); remove it there first.
- The circular arrow at the end of the row gives a command its default keys back, those
  another command does not use by then, and **Reset all** does it for every command.
- Save As, Quit, Real size, the View panel's options, the template commands (New, Edit,
  Delete, Open folder, Check) and the window itself have no key by default; you can give
  them one. Cut, Copy and Paste, and the arrow keys, keep theirs.
- Search by command name or by key.
- While you type in a text field, only the File commands work, and only with Ctrl or Alt
  held.
- The keys are kept between sessions; menus and tooltips show the current ones.

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
| Add a background layer filling the outline | Ctrl+B |
| Type in a text layer on the canvas | Double-click it (straight text only); Esc or a click outside it to stop |
| Finish drawing a polyline or polygon | Double-click, Enter, Esc or right-click; a click on the first point closes it |
| Edit the points of a line, polyline, polygon or path | Double-click it; Esc or selecting something else to stop |
| End comparing the edited template with its source | Esc, or End comparison |
| Add a point while editing points | Double-click a segment |
| Make a point smooth or sharp | Double-click it while editing points |
| Place a smooth point while drawing | Press and drag |
| Cut / Copy / Paste layer | Ctrl+X / Ctrl+C / Ctrl+V |
| Duplicate | Ctrl+D |
| Group / Ungroup | Ctrl+G / Ctrl+Shift+G |
| Clip to layer below (on or off) | Ctrl+Alt+G, or Alt+click on the line between two layers in the list |
| Delete layer | Del or Backspace |
| Nudge by 1 mm (10 mm with Shift) | Arrow keys |
| Tilt the selected layer in 3D (Shift: 15° steps) | Ctrl+drag on it |
| Distort the selected layer (move one corner) | Ctrl+drag a corner handle |
| Select the layer above / below (the layer list clicked last) | Up / Down |
| Zoom in / out | Ctrl++ (or Ctrl+=) / Ctrl+- |
| Fit page in view | F |
| Select / Pencil / Brush / Eraser tool (with a canvas selected) | V / P / B / E (Esc: back to Select) |
| Eyedropper | I (Esc: back to Select) |
| Clear the paint in the selected canvas | Ctrl+Shift+Delete |
| Add a guide | Drag from a ruler |
| Move / delete a guide | Drag it / drag it off the canvas |
| Type a guide's position | Double-click it |
| Pan | Space+drag, middle or right drag, or drag on empty canvas |
| Zoom | Mouse wheel, or Ctrl+scroll |

Modifier keys while dragging on the canvas: **Shift** (free scaling, 15° steps), **Alt**
(stretch from the center, drag without snapping, move one curve handle), **Ctrl**
(distort a corner, tilt in 3D), **Ctrl+Alt** (invert Stretch paint for that drag); see
[Transforming layers](#transforming-layers) and [Editing points](#editing-points).

## Languages

English (US and UK) and French (France) interface. It follows the system language by
default; choose another in **View > Language**.
