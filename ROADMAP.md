# Roadmap

Planned features, with the core features first. Items may still move or change as the
work goes on.

## Core features

### Template creator

- Create a template in the app instead of writing DXF files and `template.json` by hand.
- Outline: rectangle with rounded corners, or imported from DXF / SVG.
- Buttons: place holes by standard size (24 mm, 30 mm, custom), one by one or from layout
  presets, with numeric position, spacing and alignment.
- Cutouts and fastener holes, with the same tools.
- Anchors generated from the button centers.
- Placement on the page: rotation, flips and offset, saved in `template.json`.
- Saves the DXF files and `template.json` into a new folder under `templates`.
- Edit an existing template.
- Publish template to a repository

## Further features

### Printing

- **CMYK / ICC profile** support for PDF export to print shops.

### Editing

- **Clipping masks**: clip a layer to the shape of the layer below it, or to a group.
- **Blend modes** (multiply, screen, overlay...).
- **Pattern fills** for shapes, text and the background.
- **Background color** layer filling the whole outline.
- **Flip** layers horizontally and vertically.
- **Eyedropper** to pick a color from the canvas.
- **Copy and paste effects** from one layer to others.

### Files and safety

- **Template library**: browse and download templates shared by others.

### Plugins

- **Layer plugins** that generate content (patterns, procedural textures), not only
  effects.
- **Import / export plugins** for other file formats.

### Interface

- **More languages**.
