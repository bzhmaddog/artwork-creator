//! On-screen results of effect stacks.
//!
//! The canvas needs each layer's image with its effects applied, as a GPU texture and as
//! a smaller pixmap (for semi-transparent groups and the export preview). Those are
//! computed here from a screen-sized copy of the source image and stored in the layer's
//! `fx` field, which is not saved and not kept in undo snapshots. Recent results are kept
//! here too, so undo and redo don't recompute them.
//!
//! While the user drags a slider, a smaller copy is processed so the canvas keeps up; the
//! full screen-size result follows once the mouse button is released.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{ColorImage, TextureFilter, TextureHandle, TextureOptions};
use image::RgbaImage;
use tiny_skia::Pixmap;

use crate::document::{Document, Layer};
use crate::render;

/// Longest side of the processed screen copy, as for unprocessed layer textures.
const SCREEN_SIDE: u32 = 4096;
/// Longest side of the processed preview pixmap.
const PREVIEW_SIDE: u32 = 2048;
/// Longest side processed while a value is being dragged.
const QUICK_SIDE: u32 = 1024;
/// Results kept for layers that don't currently use them (e.g. for undo).
const SPARE: usize = 4;

/// A layer's image with its effects applied, for the screen.
#[derive(Clone)]
pub struct Processed {
    /// `stack_key` of the source and effects this was made from.
    pub key: u64,
    /// Made at full screen size, not the quick size used while dragging.
    pub full: bool,
    pub texture: TextureHandle,
    /// Premultiplied, at most `PREVIEW_SIDE`.
    pub preview: Arc<Pixmap>,
    /// Margins the effects added around the image (left, top, right, bottom), in source
    /// pixels: `texture` and `preview` cover the image and these margins.
    pub pad: [f32; 4],
}

struct Entry {
    layer: u64,
    processed: Processed,
    last_used: u64,
}

/// Screen-size source images and recent results, by layer.
#[derive(Default)]
pub struct EffectCache {
    /// Unprocessed screen copy of layer images, by the address of the layer's pixmap:
    /// shared by duplicated layers, and new when an image is reloaded.
    sources: HashMap<usize, Arc<RgbaImage>>,
    entries: Vec<Entry>,
    frame: u64,
}

/// Identifies a layer's source image and effect stack.
fn stack_key(layer: &Layer) -> Option<u64> {
    let pm = layer.pixmap.as_ref()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (Arc::as_ptr(pm) as usize).hash(&mut h);
    super::key(&layer.effects).hash(&mut h);
    Some(h.finish())
}

impl EffectCache {
    /// Brings every layer's `fx` up to date with its effects. With `quick`, new results
    /// are made at a reduced size. Returns true if something was recomputed.
    pub fn sync(&mut self, ctx: &egui::Context, doc: &mut Document, quick: bool) -> bool {
        self.frame += 1;
        let mut changed = false;
        doc.for_each_layer_mut(|layer| changed |= self.sync_layer(ctx, layer, quick));
        self.trim(doc);
        changed
    }

    fn sync_layer(&mut self, ctx: &egui::Context, layer: &mut Layer, quick: bool) -> bool {
        if !super::is_active(&layer.effects) {
            layer.fx = None;
            return false;
        }
        let Some(key) = stack_key(layer) else {
            layer.fx = None;
            return false;
        };
        let good = |p: &Processed| p.key == key && (p.full || quick);
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.layer == layer.id && good(&e.processed))
        {
            e.last_used = self.frame;
            if !layer
                .fx
                .as_ref()
                .is_some_and(|fx| fx.key == key && fx.full == e.processed.full)
            {
                layer.fx = Some(e.processed.clone());
            }
            return false;
        }
        // A duplicated layer arrives with its original's result: keep it.
        if let Some(fx) = layer.fx.as_ref().filter(|fx| good(fx)) {
            self.entries.push(Entry {
                layer: layer.id,
                processed: fx.clone(),
                last_used: self.frame,
            });
            return false;
        }
        let Some(processed) = self.process(ctx, layer, key, quick) else {
            layer.fx = None;
            return false;
        };
        // A quick result is replaced by the full one.
        self.entries
            .retain(|e| !(e.layer == layer.id && e.processed.key == key));
        self.entries.push(Entry {
            layer: layer.id,
            processed: processed.clone(),
            last_used: self.frame,
        });
        layer.fx = Some(processed);
        true
    }

    /// The layer's unprocessed screen copy.
    fn source(&mut self, layer: &Layer) -> Option<Arc<RgbaImage>> {
        let pm = layer.pixmap.as_ref()?;
        let ptr = Arc::as_ptr(pm) as usize;
        if let Some(img) = self.sources.get(&ptr) {
            return Some(img.clone());
        }
        // From the original file when we have it: exact even for semi-transparent pixels.
        let full = layer
            .data
            .as_ref()
            .and_then(|d| image::load_from_memory(d).ok())
            .map(|i| i.to_rgba8())
            .filter(|i| i.dimensions() == (pm.width(), pm.height()))
            .unwrap_or_else(|| demultiply(pm));
        let img = Arc::new(render::downscale(&full, SCREEN_SIDE).into_owned());
        self.sources.insert(ptr, img.clone());
        Some(img)
    }

    fn process(
        &mut self,
        ctx: &egui::Context,
        layer: &Layer,
        key: u64,
        quick: bool,
    ) -> Option<Processed> {
        let src = self.source(layer)?;
        let mut img = if quick {
            render::downscale(&src, QUICK_SIDE).into_owned()
        } else {
            (*src).clone()
        };
        let scale = img.width() as f32 / layer.size_px[0].max(1) as f32;
        let scale_y = img.height() as f32 / layer.size_px[1].max(1) as f32;
        let done = super::apply(&mut img, &layer.effects, scale);
        for e in &done.errors {
            log::warn!("layer {}: {e}", layer.name);
        }
        let [l, t, r, b] = done.pad.map(|v| v as f32);
        let pad = [l / scale, t / scale_y, r / scale, b / scale_y];
        let preview = render::premultiplied_pixmap(&render::downscale(&img, PREVIEW_SIDE))?;
        let color = ColorImage::from_rgba_unmultiplied(
            [img.width() as usize, img.height() as usize],
            img.as_raw(),
        );
        let options = TextureOptions {
            mipmap_mode: Some(TextureFilter::Linear),
            ..TextureOptions::LINEAR
        };
        Some(Processed {
            key,
            full: !quick,
            texture: ctx.load_texture(format!("layer-fx-{}", layer.id), color, options),
            preview: Arc::new(preview),
            pad,
        })
    }

    /// Drops sources no layer with effects uses, and old results beyond `SPARE`.
    fn trim(&mut self, doc: &Document) {
        let mut used = std::collections::HashSet::new();
        for leaf in doc.leaves() {
            if super::is_active(&leaf.layer.effects)
                && let Some(pm) = &leaf.layer.pixmap
            {
                used.insert(Arc::as_ptr(pm) as usize);
            }
        }
        self.sources.retain(|ptr, _| used.contains(ptr));
        let frame = self.frame;
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.last_used));
        let mut spare = 0;
        self.entries.retain(|e| {
            if e.last_used == frame {
                return true;
            }
            spare += 1;
            spare <= SPARE
        });
    }
}

/// Straight RGBA from premultiplied pixels.
pub(super) fn demultiply(pm: &Pixmap) -> RgbaImage {
    let mut img = RgbaImage::new(pm.width(), pm.height());
    for (out, px) in img.pixels_mut().zip(pm.pixels()) {
        let c = px.demultiply();
        out.0 = [c.red(), c.green(), c.blue(), c.alpha()];
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Node;
    use crate::effects::{EffectInstance, registry};
    use crate::history::History;

    fn doc() -> Document {
        let img = RgbaImage::from_pixel(8, 6, image::Rgba([255, 0, 0, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut l = Layer::new(1, "a".into(), "a.png".into());
        l.size_px = [8, 6];
        l.data = Some(Arc::new(png));
        l.pixmap = Some(Arc::new(render::premultiplied_pixmap(&img).unwrap()));
        Document {
            layers: vec![Node::Layer(Box::new(l))],
            next_id: 2,
            ..Default::default()
        }
    }

    fn fx(doc: &Document) -> Option<Processed> {
        doc.layer(1).unwrap().fx.clone()
    }

    #[test]
    fn applies_effects_for_the_screen_and_survives_undo() {
        let ctx = egui::Context::default();
        let mut cache = EffectCache::default();
        let mut doc = doc();
        let mut history = History::new(&doc);
        assert!(!cache.sync(&ctx, &mut doc, false));
        assert!(fx(&doc).is_none(), "no effects, nothing computed");

        doc.layer_mut(1).unwrap().effects.push(EffectInstance::new(
            registry().get("builtin.invert").unwrap(),
        ));
        // While dragging: a quick result.
        assert!(cache.sync(&ctx, &mut doc, true));
        assert!(!fx(&doc).unwrap().full);
        // Released: the full one, then nothing more to do.
        assert!(cache.sync(&ctx, &mut doc, false));
        let done = fx(&doc).unwrap();
        assert!(done.full);
        let px = done.preview.pixel(0, 0).unwrap();
        assert_eq!((px.red(), px.green(), px.blue()), (0, 255, 255));
        assert!(!cache.sync(&ctx, &mut doc, false));
        history.update(&doc, true);

        // Undo restores a snapshot without results: the plain image comes back as is...
        assert!(history.undo(&mut doc));
        assert!(doc.layer(1).unwrap().effects.is_empty());
        assert!(!cache.sync(&ctx, &mut doc, false));
        // ...and redo takes the inverted one from the cache instead of recomputing it.
        assert!(history.redo(&mut doc));
        assert!(fx(&doc).is_none());
        assert!(!cache.sync(&ctx, &mut doc, false));
        assert!(Arc::ptr_eq(&fx(&doc).unwrap().preview, &done.preview));

        // A duplicate shares the source and keeps its original's result.
        let mut copy = doc.layer(1).unwrap().clone();
        copy.id = 2;
        doc.layers.push(Node::Layer(Box::new(copy)));
        assert!(!cache.sync(&ctx, &mut doc, false));
        assert!(Arc::ptr_eq(
            &doc.layer(2).unwrap().fx.as_ref().unwrap().preview,
            &done.preview
        ));
        doc.layers.pop();

        // Disabling the effect shows the original image.
        doc.layer_mut(1).unwrap().effects[0].enabled = false;
        cache.sync(&ctx, &mut doc, false);
        assert!(fx(&doc).is_none());
    }
}

#[cfg(test)]
mod shadow_tests {
    use super::*;
    use crate::document::Node;
    use crate::effects::{EffectInstance, registry};
    use crate::render::{Cuts, Scene, compose};

    /// Blackness (0 to 255) of a page pixel; the page is white.
    fn dark(pm: &Pixmap, x: u32, y: u32) -> u8 {
        255 - pm.pixel(x, y).unwrap().red()
    }

    #[test]
    fn shadow_lands_in_the_same_place_on_screen_and_in_the_export() {
        // A 10 x 10 mm white square centred at (10, 10) mm on a 30 mm grey page, with a
        // hard black shadow 4 mm right and down.
        let img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        let mut l = Layer::new(1, "a".into(), "a.png".into());
        l.size_px = [10, 10];
        l.transform.center = [10.0, 10.0];
        l.pixmap = Some(Arc::new(render::premultiplied_pixmap(&img).unwrap()));
        let mut shadow = EffectInstance::new(registry().get("builtin.drop_shadow").unwrap());
        for (k, v) in [
            ("angle", 135.0),
            ("distance", 32f32.sqrt()),
            ("softness", 0.0),
            ("opacity", 100.0),
        ] {
            shadow.params.insert(k.into(), v.into());
        }
        l.effects.push(shadow);
        let mut doc = Document {
            layers: vec![Node::Layer(Box::new(l))],
            next_id: 2,
            ..Default::default()
        };
        doc.background = [255, 255, 255];

        let ctx = egui::Context::default();
        EffectCache::default().sync(&ctx, &mut doc, false);
        let layer = doc.layer(1).unwrap();
        assert_eq!(layer.fx.as_ref().unwrap().pad, [0.0, 0.0, 4.0, 4.0]);
        assert_eq!(layer.image_uv(), [0.0, 0.0, 10.0 / 14.0, 10.0 / 14.0]);

        let colors = [[0; 3]; crate::template::KINDS];
        let export = Scene::new(
            &doc,
            None,
            colors,
            [false; crate::template::KINDS],
            Cuts::default(),
        );
        let screen = Scene::preview(
            &doc,
            None,
            colors,
            [false; crate::template::KINDS],
            Cuts::default(),
        );
        for scene in [export, screen] {
            // 1 px per mm.
            let pm = compose(&scene, 25.4, false).unwrap();
            assert_eq!(dark(&pm, 10, 10), 0, "the square itself");
            assert_eq!(dark(&pm, 17, 17), 255, "shadow beyond its corner");
            assert_eq!(dark(&pm, 17, 7), 0, "nothing above the shadow");
            assert_eq!(dark(&pm, 3, 3), 0, "nothing up left");
        }
    }
}
