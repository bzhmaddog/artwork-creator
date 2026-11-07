# Roadmap

Planned features, with the core features first. Items may still move or change as the
work goes on.

## Core features

### Text layers

- Text layer type, placed and transformed on the canvas like image layers (move, scale,
  rotate).
- Font choice from the system fonts, plus fonts kept inside the `.artwork` archive so a
  shared project looks the same elsewhere.
- Size in pt or mm, bold / italic, alignment, letter spacing, line height, multi-line text.
- Fill colour and outline (stroke width and colour).
- Layer effects work on text too (drop shadow, outline, plugins...).
- Editing the text in place on the canvas, and in the Properties panel.
- PDF export keeps text as vectors.

### Shape layers

- Shapes: rectangle (with optional rounded corners), ellipse / circle, line, polyline,
  polygon, regular polygon and star.
- Stroke: width, colour, opacity, dashes, line caps and joins.
- Fill: colour and opacity, or no fill.
- Edit points on the canvas for lines, polylines and polygons; snap them to template
  anchors.
- Layer effects work on shapes too.
- PDF export keeps shapes as vectors.

### Template creator

- Create a template in the app instead of writing DXF files and `template.json` by hand.
- Outline: rectangle with rounded corners, or imported from DXF / SVG.
- Buttons: place holes by standard size (24 mm, 30 mm, custom), one by one or from layout
  presets, with numeric position, spacing and alignment.
- Cutouts and fastener holes, with the same tools.
- Anchors generated from the button centres.
- Saves the DXF files and `template.json` into a new folder under `templates`.
- Edit an existing template.

## Further features

### Printing

- **Bleed and safe area**: extend the artwork past the outline by a chosen margin, and show
  a safe-area guide inside it.
- **Crop marks** and registration marks in the export.
- **Tiled printing**: split artwork larger than the page over several sheets, with overlap
  and alignment marks.
- **Custom page sizes** and more presets (A3, Letter, Tabloid...).
- **Mirrored export**, for printing on the back of clear film or plexi.
- **Low resolution warning**: flag images whose effective DPI at their printed size is
  below a threshold.
- **CMYK / ICC profile** support for PDF export to print shops.

### Editing

- **Clipping masks**: clip a layer to the shape of the layer below it, or to a group.
- **Blend modes** (multiply, screen, overlay...).
- **Gradient and pattern fills** for shapes, text and a background layer.
- **Background colour** layer filling the whole outline.
- **Flip** layers horizontally and vertically.
- **Rulers and guides** dragged from the rulers, with snapping.
- **Eyedropper** to pick a colour from the canvas.
- **Copy and paste effects** from one layer to others.
- **Per-button artwork**: fill or ring around a single button hole, and export of round
  artwork for clear button caps.
- **SVG import** as vector layers.

### Files and safety

- **Autosave and crash recovery**, since a plugin crash closes the app.
- **SVG export**.
- **Template library**: browse and download templates shared by others.

### Plugins

- **Layer plugins** that generate content (patterns, procedural textures), not only
  effects.
- **Import / export plugins** for other file formats.

### Interface

- **Customisable shortcuts**.
- **More languages**.
