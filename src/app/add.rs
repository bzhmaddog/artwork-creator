//! Adding layers (texts, shapes, canvases, backgrounds, images, SVG files),
//! pattern images, and cut, copy and paste.

use super::*;

impl ArtworkApp {
    /// Decodes a layer's image file into its full-size pixmap, preview pixmap and screen
    /// texture.
    pub(super) fn load_image(ctx: &egui::Context, layer: &mut Layer) -> Result<(), String> {
        // The original bytes are kept, to be saved into project archives as they are.
        let data = match &layer.data {
            Some(data) => data.clone(),
            None => {
                let bytes = std::fs::read(&layer.source)
                    .map_err(|e| tr!("CANNOT_OPEN_FILE", path = layer.source.display(), e = e))?;
                let data = Arc::new(bytes);
                layer.data = Some(data.clone());
                data
            }
        };
        let img = image::load_from_memory(&data)
            .map_err(|e| tr!("CANNOT_OPEN_FILE", path = layer.source.display(), e = e))?
            .to_rgba8();
        let invalid = || tr!("IMAGE_HAS_NO_PIXELS", path = layer.source.display());
        let (w, h) = img.dimensions();
        layer.size_px = [w, h];
        let display = render::downscale(&img, MAX_TEXTURE_SIDE);
        let color = ColorImage::from_rgba_unmultiplied(
            [display.width() as usize, display.height() as usize],
            display.as_raw(),
        );
        let preview = render::premultiplied_pixmap(&render::downscale(&display, MAX_PREVIEW_SIDE))
            .ok_or_else(invalid)?;
        let options = TextureOptions {
            mipmap_mode: Some(TextureFilter::Linear),
            ..TextureOptions::LINEAR
        };
        layer.texture = Some(ctx.load_texture(format!("layer-{}", layer.id), color, options));
        layer.preview = Some(Arc::new(preview));
        layer.pixmap = Some(Arc::new(
            render::premultiplied_pixmap(&img).ok_or_else(invalid)?,
        ));
        layer.broken = false;
        Ok(())
    }

    /// Where a new text or shape layer goes: the template's center (the page's without
    /// a template), or the middle of the view when that is out of sight.
    pub(super) fn new_layer_center(&self) -> P2 {
        let center = Self::outline_bounds(&self.elements())
            .map(|(c, _)| c)
            .unwrap_or_else(|| self.doc.page_center());
        self.canvas.in_view(center)
    }

    /// Adds a text layer above the selection, centered on the template, and puts the
    /// keyboard in its text.
    pub(super) fn add_text(&mut self, ctx: &egui::Context) {
        let id = self.doc.alloc_id();
        let center = self.new_layer_center();
        // A random word to type over, which names it until it is renamed.
        let word = layers::random_name(&self.doc);
        let layer = text::new_layer(id, &word, self.fonts.default_family(), center);
        let at = layers::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        self.layers_panel.show_text = true;
        text::sync(ctx, &mut self.doc, &self.fonts);
        // Typed in right away on the canvas, its placeholder text selected.
        if let Some(layer) = self.doc.layer(id) {
            self.canvas.type_in(layer);
        }
    }

    /// Adds a shape layer of `kind` above the selection, centered on the template, or on
    /// the view when the template's center is out of sight (see `new_layer_center`). Lines,
    /// polylines and polygons are drawn point by point on the canvas instead; paths come
    /// from an SVG file, asked for.
    pub(super) fn add_shape(&mut self, ctx: &egui::Context, kind: ShapeKind) {
        // In a template, only into the element selected (or the one of the shape selected).
        if self.editing.is_some()
            && self
                .selected
                .and_then(|id| crate::template::edit::element_of(&self.doc, id))
                .is_none()
        {
            return;
        }
        self.layers_panel.shape_kind = kind;
        if kind == ShapeKind::Path {
            self.pick_svg(ctx);
            return;
        }
        if kind.has_points() {
            self.canvas.drawing = Some(canvas::Drawing::new(kind));
            self.canvas.points = None;
            self.layers_panel.edit_points = None;
            return;
        }
        let id = self.doc.alloc_id();
        let center = self.new_layer_center();
        let mut layer = shape::new_layer(id, kind, center);
        layer.name = layers::random_name(&self.doc);
        let at = layers::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        shape::sync(ctx, &mut self.doc, true);
    }

    /// Adds a closed shape of `kind` to paint in, without fill or stroke, above the
    /// selection, half the page's size (an ellipse, regular polygon or star: in a square
    /// half its shorter side across) and centered like a new text (see `new_layer_center`), with the Select
    /// tool in hand. A polygon is drawn point by point on the canvas instead, as from
    /// the Shape menu.
    /// The outline grown by the bleed, made again only when either changes.
    pub(super) fn background_rings(&mut self, elements: &[PlacedElement]) -> Option<Vec<Vec<P2>>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let bleed = self.cuts().bleed_mm;
        bleed.to_bits().hash(&mut h);
        for e in elements.iter().filter(|e| e.kind == ElementKind::Outline) {
            for p in &e.polylines {
                p.closed.hash(&mut h);
                p.points
                    .iter()
                    .flatten()
                    .for_each(|v| v.to_bits().hash(&mut h));
            }
        }
        let key = h.finish();
        if self.background_rings.0 != key {
            self.background_rings = (key, crate::document::background::rings(elements, bleed));
        }
        self.background_rings.1.clone()
    }

    /// Adds a background layer at the bottom of the stack, filling the outline grown by
    /// the bleed, and selects it.
    pub(super) fn add_background(&mut self) {
        if self.editing.is_some() {
            return;
        }
        let elements = self.elements();
        let Some(rings) = self.background_rings(&elements) else {
            self.messages
                .push(tr!("BACKGROUND_NEEDS_OUTLINE").to_owned());
            return;
        };
        let id = self.doc.alloc_id();
        let name = tr!("BACKGROUND_LAYER_NAME").to_owned();
        let layer = crate::document::background::new_layer(id, name, &rings, &self.doc);
        self.doc.insert(&[0], Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        self.dirty = true;
    }

    pub(super) fn add_canvas(&mut self, ctx: &egui::Context, kind: ShapeKind) {
        self.layers_panel.canvas_kind = kind;
        self.canvas.brushes.tool = None;
        if kind.has_points() {
            let mut drawing = canvas::Drawing::new(kind);
            drawing.canvas = true;
            self.canvas.drawing = Some(drawing);
            self.canvas.points = None;
            self.layers_panel.edit_points = None;
            return;
        }
        let id = self.doc.alloc_id();
        let c = self.new_layer_center();
        let [mut w, mut h] = self.doc.page_mm().map(|v| v * CANVAS_PAGE_FRACTION / 2.0);
        if matches!(
            kind,
            ShapeKind::Ellipse | ShapeKind::RegularPolygon | ShapeKind::Star
        ) {
            // In a square box, as from the Shape menu (a circle, an unsquished star or
            // polygon): its handles stretch it.
            (w, h) = (w.min(h), w.min(h));
        }
        let mut layer =
            shape::new_canvas_layer(id, kind, [c[0] - w, c[1] - h], [c[0] + w, c[1] + h]);
        layer.name = layers::random_name(&self.doc);
        let at = layers::insertion_path(&mut self.doc, self.selected);
        self.doc.insert(&at, Node::Layer(Box::new(layer)));
        self.selected = Some(id);
        self.canvas.drawing = None;
        self.canvas.points = None;
        self.layers_panel.edit_points = None;
        // Selecting and moving first: it is placed before it is painted in.
        self.canvas.brushes.tool = None;
        shape::sync(ctx, &mut self.doc, true);
    }

    /// Adds files dropped on the window: SVG files as shapes (see `add_svg`), others as
    /// images.
    pub(super) fn add_dropped(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        let (svgs, images): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| SVG_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        });
        for path in svgs {
            self.add_svg(ctx, &path);
        }
        // A template is made of shapes only.
        if !images.is_empty() && self.editing.is_none() {
            self.add_images(ctx, images);
        }
    }

    /// Adds one layer per image file, fitted to the page, above the selection.
    pub(super) fn add_images(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        for path in paths {
            let id = self.doc.alloc_id();
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| tr!("DEFAULT_IMAGE_LAYER_NAME").into());
            let mut layer = Layer::new(id, name, path);
            match Self::load_image(ctx, &mut layer) {
                Ok(()) => {
                    layer.fit_to(self.doc.page_center(), self.doc.page_mm(), false);
                    let at = layers::insertion_path(&mut self.doc, self.selected);
                    self.doc.insert(&at, Node::Layer(Box::new(layer)));
                    self.selected = Some(id);
                    self.dirty = true;
                }
                Err(e) => self.messages.push(e),
            }
        }
    }

    /// Adds an SVG file's shapes (and its groups) above the selection, at its true size
    /// unless larger than the page.
    pub(super) fn add_svg(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        // In a template, only into the element selected (or the one of the shape selected).
        if self.editing.is_some()
            && self
                .selected
                .and_then(|id| crate::template::edit::element_of(&self.doc, id))
                .is_none()
        {
            self.messages
                .push(tr!("ADD_SHAPE_SELECT_ELEMENT").to_owned());
            return;
        }
        let (page, center) = (self.doc.page_mm(), self.doc.page_center());
        let doc = &mut self.doc;
        match crate::files::svg_import::import(path, page, center, &mut || doc.alloc_id()) {
            Ok(imported) => {
                // Bottom first, each just above the one before (not inside it, when it
                // is a group); the top one ends selected.
                let mut at = layers::insertion_path(&mut self.doc, self.selected);
                for node in imported.nodes {
                    self.selected = Some(node.id());
                    self.doc.insert(&at, node);
                    if let Some(last) = at.last_mut() {
                        *last += 1;
                    }
                }
                self.dirty = true;
                self.messages.extend(imported.notices);
                shape::sync(ctx, &mut self.doc, true);
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// Opens the dialog choosing the image of a pattern fill.
    pub(super) fn pick_pattern(
        &mut self,
        ctx: &egui::Context,
        target: crate::document::pattern::Target,
    ) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::Pattern(target),
                title: tr!("PATTERN_DIALOG_TITLE"),
                save: false,
                multiple: false,
                filters: IMAGE_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Fills `target` with a pattern of the image at `path`. A pattern it already has
    /// keeps its tile's width, turn and offset; a new one gets the image's own size.
    pub(super) fn set_pattern(&mut self, target: crate::document::pattern::Target, path: PathBuf) {
        use crate::document::pattern::{Pattern, Target};
        let (slot, units_per_mm) = match target {
            Target::Background => (Some(&mut self.doc.background_pattern), 1.0),
            Target::Layer(id) => match self.doc.layer_mut(id) {
                Some(l) => {
                    // A text's units are its scale's mm.
                    let k = 1.0 / l.transform.scale[1].abs().max(1e-6);
                    match (&mut l.shape, &mut l.text) {
                        (Some(s), _) => (Some(&mut s.fill_pattern), 1.0),
                        (None, Some(t)) => (Some(&mut t.pattern), k),
                        (None, None) => (None, 1.0),
                    }
                }
                None => (None, 1.0),
            },
        };
        let Some(slot) = slot else {
            return;
        };
        match Pattern::open(path, units_per_mm) {
            Ok(mut new) => {
                if let Some(old) = slot.take() {
                    (new.size, new.angle_deg, new.offset) = (old.size, old.angle_deg, old.offset);
                }
                *slot = Some(new);
                self.dirty = true;
            }
            Err(e) => self.messages.push(e),
        }
    }

    /// Opens the dialog to add image layers.
    pub(super) fn pick_images(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::AddImages,
                title: tr!("ADD_IMAGES_DIALOG_TITLE"),
                save: false,
                multiple: true,
                filters: IMAGE_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Opens the dialog to add the shapes of SVG files as path layers.
    pub(super) fn pick_svg(&mut self, ctx: &egui::Context) {
        self.request_file(
            ctx,
            Request {
                purpose: Purpose::ImportSvg,
                title: tr!("IMPORT_SVG_DIALOG_TITLE"),
                save: false,
                multiple: true,
                filters: SVG_FILTERS,
                default_name: String::new(),
            },
        );
    }

    /// Cuts or copies the selection onto the clipboard. Paste asks for the system
    /// clipboard's text, which comes back next frame as a paste event, as with Ctrl+V.
    pub(super) fn edit(&mut self, ctx: &egui::Context, edit: Edit) {
        let text = match edit {
            Edit::Cut => {
                let text = self.clipboard.cut(&mut self.doc, &mut self.selected);
                self.dirty |= text.is_some();
                text
            }
            Edit::Copy => self.clipboard.copy(&self.doc, self.selected),
            Edit::Paste => {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                None
            }
        };
        if let Some(text) = text {
            ctx.copy_text(text);
        }
    }

    /// Pastes the layers the system clipboard's `text` holds, if it holds any. Layers
    /// copied in another window come without their images, which are loaded here.
    pub(super) fn paste(&mut self, ctx: &egui::Context, text: &str) {
        let before = self.selected;
        let Some(pasted) = self
            .clipboard
            .paste(&mut self.doc, &mut self.selected, text)
        else {
            return;
        };
        // In a template, shapes go into an element and elements at the top of the list.
        if self.editing.is_some() {
            let parent = self.doc.path_of(pasted.id).map(|mut p| {
                p.pop();
                p
            });
            if !parent.is_some_and(|p| crate::template::edit::fits_in(&self.doc, pasted.id, &p)) {
                self.doc.remove(pasted.id);
                self.selected = before;
                self.messages
                    .push(tr!("TEMPLATE_PASTE_OUTSIDE_ELEMENT").to_owned());
                return;
            }
        }
        self.dirty = true;
        if !pasted.foreign {
            return;
        }
        let mut missing = Vec::new();
        if let Some(node) = self.doc.node_mut(pasted.id) {
            node.for_each_layer_mut(&mut |layer| {
                if !layer.is_drawn() && Self::load_image(ctx, layer).is_err() {
                    layer.broken = true;
                    missing.push(layer.source.display().to_string());
                }
                for p in layer.patterns_mut() {
                    if p.tile.is_none() && p.load().is_err() {
                        missing.push(p.source.display().to_string());
                    }
                }
            });
        }
        if !missing.is_empty() {
            self.messages
                .push(tr!("IMAGES_NOT_LOADED", files = missing.join("\n")));
        }
    }
}
