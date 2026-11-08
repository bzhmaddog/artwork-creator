# Roadmap

Planned features, with the core features first. Items may still move or change as the
work goes on.

## Core features

### Text layers

Done:

- Text layer type, placed and transformed on the canvas like image layers (move, scale,
  rotate, distort, snap).
- Font choice from the system fonts, plus fonts kept inside the `.artwork` archive so a
  shared project looks the same elsewhere.
- Size in pt, bold / italic, alignment, letter spacing, line spacing, multi-line text.
- Fill color and outline (width and color).
- Layer effects work on text too.
- Text edited in the Properties panel, or typed in place on the canvas after a double
  click on the layer (new text layers start that way).
- Drawn at the export resolution, so text prints sharp at any DPI.
- Characters missing from the chosen font taken from the system's fallback fonts (saved
  with the project too), or the built-in emoji font.
- PDF export keeps text as vector outlines (except text with effects, or inside a
  semi-transparent group).
- Text along a path: an arc, a circle, or the outline of a shape layer, with position,
  side and baseline shift.

### Shape layers

Done:

- Shapes: rectangle (with optional rounded corners), ellipse / circle, line, polyline,
  polygon, regular polygon and star; one kind can be turned into another.
- Stroke: width, color, opacity, dashes, line caps and joins. Resizing keeps its width.
- Fill for closed shapes: color and opacity, or no fill.
- Layer effects work on shapes too.
- PDF export keeps shapes as vectors (except shapes with effects, or inside a
  semi-transparent group).
- Lines, polylines and polygons drawn point by point on the canvas, with straight or
  Bezier curve segments; their points and curve handles edited afterwards (move, add,
  remove, smooth or sharp); points snap to template anchors.

### Template creator

- Create a template in the app instead of writing DXF files and `template.json` by hand.
- Outline: rectangle with rounded corners, or imported from DXF / SVG.
- Buttons: place holes by standard size (24 mm, 30 mm, custom), one by one or from layout
  presets, with numeric position, spacing and alignment.
- Cutouts and fastener holes, with the same tools.
- Anchors generated from the button centers.
- Saves the DXF files and `template.json` into a new folder under `templates`.
- Edit an existing template.

## Further features

### Printing

Done:

- **Bleed and safe area**: the artwork extends past the outline (and into the holes) by a
  chosen margin, and a safe-area guide is shown inside it.
- **Crop marks** and registration marks in the export.
- **Page sizes**: A5, A4, A3, A2, Letter, Legal and Tabloid, or a custom size.
- **Tiled printing**: a page larger than the paper split over several sheets, with
  margins, overlap and alignment marks.
- **Mirrored export**, for printing on the back of clear film or plexi.
- **Low resolution warning**: images whose effective DPI at their printed size is below
  a threshold are flagged.
- **Print directly from the app**, at true size with the export options: the system
  print dialog on Windows, CUPS on Linux.

Planned:

- **CMYK / ICC profile** support for PDF export to print shops.

### Editing

- **Copy/Paste layers**: Beeing able to copy/paste from the canvas area
- **Clipping masks**: clip a layer to the shape of the layer below it, or to a group.
- **Blend modes** (multiply, screen, overlay...).
- **Gradient and pattern fills** for shapes, text and a background layer.
- **Color manager** to store and reuse colors already used in the project
- **Background color** layer filling the whole outline.
- **Flip** layers horizontally and vertically.
- **Rulers and guides** dragged from the rulers, with snapping.
- **Eyedropper** to pick a color from the canvas.
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

- **Customizable shortcuts**.
- **More languages**.
- **Panel position customisation** : let user drag and place panel where he want (floating or snapped to borders of the window)
