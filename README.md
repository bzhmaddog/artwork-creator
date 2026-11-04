# Artwork Creator

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
- **Transform on the canvas**: move, scale, rotate, stretch width or height on its own, and
  drag single corners to fake perspective.
- **Align** layers or groups to the page or to the template.
- **Undo / redo** for every change to the project.
- **Templates** made of several elements, each in its own DXF file, with its own colour and
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
    { "kind": "cutouts", "dxf": "holes.dxf", "name": "Fastener holes", "color": "#ff8c00" }
  ],
  "placement": { "rotation": 90, "flip_x": false, "flip_y": true, "offset_mm": [0, 0] }
}
```

- `kind` is `outline`, `buttons` or `cutouts`. The outline clips the artwork; buttons and
  cutouts are holes, left blank in the export unless turned off in the Export window.
- All DXF files of a template must share the same coordinates, as when they are exported
  from one drawing. Outlines drawn as separate lines and arcs are joined automatically.
- `units` (`mm`, `cm` or `inch`) is used when the DXF files don't declare their units.
- `name`, `color` (`#rrggbb`) and `placement` are optional. The template's default placement
  can also be saved from the app.
- Without `elements`, the folder's single DXF file is used as the outline.

The app reloads a template when its files change on disk.

## Shortcuts

| Action | Shortcut |
|---|---|
| New / Open / Save project | Ctrl+N / Ctrl+O / Ctrl+S |
| Export | Ctrl+E |
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z or Ctrl+Y |
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
- **Edge handles** stretch one side; **Alt** stretches both sides from the centre.
- **Rotation handle** rotates; **Shift** snaps to 15° steps.

## License

Licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE): free to use, copy,
modify and share for any non-commercial purpose.

**Selling artwork made with this app is not allowed.** Using it to make artwork, panels or
controllers for sale (commissions, shops, made-to-order or batch production) is commercial
use and needs the author's permission. Making artwork for your own controller is fine,
and so is selling your own controller secondhand later.

For commercial use, please contact the author.
