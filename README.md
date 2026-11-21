# Artwork Creator

[![Test](https://github.com/bzhmaddog/artwork-creator/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/bzhmaddog/artwork-creator/actions/workflows/test.yml)
[![CodeQL](https://github.com/bzhmaddog/artwork-creator/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/bzhmaddog/artwork-creator/actions/workflows/github-code-scanning/codeql)
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=bzhmaddog_artwork-creator&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=bzhmaddog_artwork-creator)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=bzhmaddog_artwork-creator&metric=coverage)](https://sonarcloud.io/component_measures?id=bzhmaddog_artwork-creator&metric=coverage)

A desktop app for designing printed artwork for arcade controllers and fightsticks.

Load a controller template drawn in SVG (an outline and holes), arrange your
images on top, and export print-ready PNG, PDF or SVG at true size. The artwork is clipped to the
panel outline and left blank in the button and cutout holes. Runs on Windows and Linux.
Built in Rust with [egui](https://github.com/emilk/egui).

![Artwork Creator with the Cosmox T16 template: settings on the left, the A4 page with the
template outline, buttons and fastener holes over a grid image in the middle, and the
layers panel on the right](docs/capture.png)

> **Note:** this project is shamelessly vibecoded. It was built by describing features to
> an AI coding assistant ([Claude Code](https://claude.com/claude-code)), not by writing
> the code by hand. It has tests and works for its author, but read the code with that in
> mind.

## Features

- Layers and groups of images, texts (straight or along a path), vector shapes, imported
  SVG paths and canvases to paint in, with clipping masks, blend modes and opacity.
- Fills of one color, linear or radial gradients, image patterns, or generated (noise,
  clouds, checkerboard, stripes); a stack of effects per layer; both extended by plugins.
- Move, scale, rotate, distort and tilt in 3D on the canvas, with snapping to the
  template's holes, the page, the outline and guides.
- Variables and formulas to link sizes, positions, colors and texts.
- A template editor, with checks, versions and a reference drawing to trace.
- Export to PNG, PDF and SVG (vectors kept), and printing, with bleed, marks, mirroring
  and tiling over several sheets.
- Autosave and crash recovery, configurable shortcuts, movable panels, English and French.

Every feature is described in detail in [docs/specs.md](docs/specs.md).

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
make linux-portable  # Linux build that also runs on older distributions (see below)
make dist       # package each build with the templates folder into dist/
make deb        # Linux .deb package into dist/ (needs cargo-deb)
make appimage   # Linux AppImage into dist/
make windows-setup   # Windows installer into dist/ (Inno Setup in a Docker/Podman container)
```

The Linux packages (`make dist`, `make deb`, `make appimage`) are made from
`make linux-portable`, which links against an old glibc (2.28, `GLIBC` in the `Makefile`)
so they run on older distributions too. It needs [zig](https://ziglang.org/download/) in
the `PATH` and [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild):

```sh
cargo install cargo-zigbuild --locked
cargo install cargo-deb --locked    # for make deb
```

Each release also has installers, built by the Release workflow: a setup `.exe` for
Windows ([Inno Setup](https://jrsoftware.org/isinfo.php), from
[`installer/artwork-creator.iss`](installer/artwork-creator.iss)), with a Start menu entry
and `.artwork` files opened with the app; a `.deb` (settings in `Cargo.toml`, made with
[cargo-deb](https://github.com/kornelski/cargo-deb)) and an AppImage
([`scripts/appimage.sh`](scripts/appimage.sh)) for Linux. Built for glibc 2.28, the `.deb`
installs on any Debian or Ubuntu from Debian 10 / Ubuntu 20.04 on. Installed, the app finds
its templates in `../share/artwork-creator/` from the executable when there are none next
to it.

The app's icon is in [`assets/icon`](assets/icon): the SVG it is drawn from, PNGs from 16
to 1024 px and the Windows `.ico`. `build.rs` builds the `.ico` into the Windows
executable (with MinGW's `windres`; `WINDRES` names another one), and the app sets the
256 px PNG as its window icon. The Linux package also holds `artwork-creator.png` and
`artwork-creator.desktop`: to list the app in the desktop's menus, copy the PNG to
`~/.local/share/icons/` and the `.desktop` file to `~/.local/share/applications/`, with
its `Exec` line set to where the executable is.

## Development

After cloning, install the tools the quality checks need and the git hooks once:

```sh
git clone https://github.com/bzhmaddog/artwork-creator.git
cd artwork-creator

# Install rustfmt, clippy, the Windows target, cargo-llvm-cov, rust-analyzer, a .venv with
# Python's coverage and the hooks, then check that every tool is there
make dev-setup
```

rust-analyzer is there for code completion and checks in an editor (VS Code's
rust-analyzer extension and others). `make dev-setup` first checks the tools it can't
install itself (git, rustup, mingw-w64 for the Windows checks, python3 and python3-venv
for the .venv) and stops, saying how to install them, while one is missing.
`make check-tools` lists every tool, what it is for and whether it is installed.

Before each commit, the hook runs the CI checks: `cargo fmt --check`, clippy for Linux and
Windows (some lints only fire on one of them), and the tests on Linux under coverage. It
fails when the lines changed since `main` are covered below 80%, as the SonarQube Cloud
quality gate does, and lists the files with uncovered lines; `make coverage` runs that
check alone (`MIN=...` sets another threshold). The Windows check
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
and listed in the Shortcuts table of [docs/specs.md](docs/specs.md#keyboard-shortcuts); every text
has a translation key, with its text in `locales/en-US.json` and `locales/fr-FR.json` (and
`locales/en-GB.json` when the British spelling differs); every feature is described in
[docs/specs.md](docs/specs.md), in the section it belongs to. Before finishing, compare the new feature with the closest
existing one and fill in what it lacks. A number field's unit goes after the field, not inside it
(`ui::layers::unit`); a pair of fields on one row (x and y, width and height) shares one. A
dialog's buttons sit at the right of its bottom row, the main one rightmost and Cancel on
its left (`app::dialog_button_row`); a secondary one such as "Close without saving" or Unlink goes
apart at the left edge (`app::at_left_edge`). A helper text (an explanation, an empty state,
a tip, a warning) is a callout (`callout::callout`, or `callout_small` in tight places),
of the tone that fits: Info, Tip or Warning; data shown for information (sizes, counts,
paths) stays plain weak text. `CALLOUT_PREVIEW=out.png cargo test callout_preview --
--ignored` draws them in both themes, to look at without starting the app. Any piece of the
interface can be drawn that way from a test, with `preview::render`. These and the other
look-and-layout rules are gathered in [docs/ui-style.md](docs/ui-style.md); a change that
adds or changes one updates it.

Write code, comments, docs and texts in U.S. English (color, center, gray, canceled);
British spellings belong in `locales/en-GB.json` only. Name translation keys after what
the text is for, in upper snake case: `ADD_TEXT_LAYER`, `EXPORT_CANCELED`.

### Releasing

`make version-check` suggests the next version, `make version-bump-patch` (or -minor,
-major, `APP=X.Y.Z`) sets it, to merge through a pull request, and pushing a `vX.Y.Z` tag of
the merged `main`, made by hand, makes GitHub Actions build the packages into a draft
release to publish by hand. Don't create the release on GitHub: see
[Versioning and releasing](doc/releasing.md) for the steps and why.

## Plugins

Plugins are shared libraries (`.dll` on Windows, `.so` on Linux) adding effects and
generators (a shape's generated fill), loaded at startup from a `plugins` folder; see "Plugins" in [docs/specs.md](docs/specs.md#plugins) for how the app finds
and uses them.

### Writing a plugin

In Rust, use the [`artwork-plugin-sdk`](plugin-sdk) crate: implement its `Effect` trait,
which works on straight RGBA pixels, or its `Generator` trait, which draws a shape's
fill given its pixels per mm; both describe their parameters so the app builds their
controls. Export them with `export_plugin!`. The crate must be a `cdylib`. See
[`plugin-examples/halftone`](plugin-examples/halftone) for a complete example, built by
`make plugins`.

Other languages can implement the C interface described in
[`plugin-sdk/include/artwork_plugin.h`](plugin-sdk/include/artwork_plugin.h): one
exported function, `artwork_plugin_entry`, returning a table of effects and generators
whose parameters and values are passed as JSON. The app still loads plugins made for
version 1 of this interface, which had effects only.

Effect and generator ids are saved in projects, so keep them stable and prefix them with
something unique to your plugin (`builtin.` is reserved). Both may run on several threads
at once. A generator must draw the same picture at any resolution: it is drawn smaller on
screen than in the export.

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
and that texts with a U.S. spelling have their en-GB entry. Effects and generators from
plugins show their own names.

To add a language: add a `Language` variant and its table in `src/i18n.rs`, and a
`locales/<code>.json` file.

## License

Licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE): free to use, copy,
modify and share for any non-commercial purpose.

**Selling artwork made with this app is not allowed.** Using it to make artwork, panels or
controllers for sale (commissions, shops, made-to-order or batch production) is commercial
use and needs the author's permission. Making artwork for your own controller is fine,
and so is selling your own controller secondhand later.

For commercial use, please contact the author.
