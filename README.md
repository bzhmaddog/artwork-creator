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
  a whole.
- **Text layers**: any installed font, bold and italic, multi-line text aligned left,
  centered or right, letter and line spacing, color and outline. Text layers move, scale,
  rotate, distort and take effects like images, and are drawn at the export resolution
  so they print sharp. Fonts are saved inside `.artwork` projects (except those whose
  license forbids embedding them), so a shared project looks the same elsewhere.
- **Transform on the canvas**: move, scale, rotate, stretch width or height on its own, and
  drag single corners to fake perspective.
- **Effects** on each image layer: brightness / contrast, hue / saturation, grayscale,
  sepia, tint, invert, posterize, threshold, Gaussian blur, sharpen, drop shadow and
  outline. They are stacked, reordered and toggled in the layer's properties, applied to
  the export at full resolution, and never change the image file. More effects can be
  added with [plugins](#plugins).
- **Snap to anchors**: image centers and corners snap to points defined by the template,
  such as the button centers.
- **Align** layers or groups to the page or to the template.
- **Undo / redo** for every change to the project.
- **Templates** made of several elements, each in its own DXF file, with its own color and
  visibility.
- **Export** PNG and/or PDF at the resolution you choose (72 to 1200 DPI). Choose which
  template lines are printed (in the PDF they stay sharp vector lines), and whether the
  inside of buttons and of cutouts is left blank or printed over.
- **Projects** are saved, by default, as a single `.artwork` file (a zip archive) holding
  the project and all its images, so it can be moved or shared on its own. **Save As**
  can also write a linked `.json` project that points to the image files on disk; images
  that only exist inside an archive are then written to an "*name* images" folder next to
  it.
  **File > Open Recent** lists the last 10 projects.
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

### Consistency

When adding a tool or feature, give it the same treatment as its siblings. For example,
every way of adding a layer has a keyboard shortcut, shown in its menu entry or tooltip
and listed under [Shortcuts](#shortcuts); every text has a translation key, with its
text in `locales/en-US.json` and `locales/fr-FR.json` (and `locales/en-GB.json` when the
British spelling differs). Before finishing, compare the new feature with the closest
existing one and fill in what it lacks.

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
  center or a corner lands on an anchor that comes near, and a corner being resized or
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
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y |
| Add an image layer | Ctrl+I, or drop image files on the window |
| Add a text layer | Ctrl+T |
| Edit a text layer's text | Double-click it on the canvas |
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

## License

Licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE): free to use, copy,
modify and share for any non-commercial purpose.

**Selling artwork made with this app is not allowed.** Using it to make artwork, panels or
controllers for sale (commissions, shops, made-to-order or batch production) is commercial
use and needs the author's permission. Making artwork for your own controller is fine,
and so is selling your own controller secondhand later.

For commercial use, please contact the author.
