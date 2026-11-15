//! Pattern fills: an image repeated side by side over a shape's fill, a text's letters or
//! the page background.
//!
//! A pattern lies in the frame of what it fills, like a gradient: it moves and turns
//! with the layer. Its tile's width, turn and offset are in the units of that frame: mm
//! for a shape and the page, the text's own units for a text (shown in mm, see
//! `ui::layers`). Its image is kept in the project like a layer's (see `project`).

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tiny_skia::{Pixmap, Rect, Transform};

use crate::document::gradient::Ink;
use crate::i18n::tr;

/// The tile's width when its image is picked: its own size printed at this resolution.
const DEFAULT_DPI: f32 = 300.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    /// The tile's image file; in a project archive, its name there.
    pub source: PathBuf,
    /// Width of one tile, in the frame's units; its height follows the image's.
    pub size: f32,
    /// Turn of the tiles, in degrees clockwise.
    #[serde(default)]
    pub angle_deg: f32,
    /// Where a tile's top-left corner is, from the frame's, in the frame's units.
    #[serde(default)]
    pub offset: [f32; 2],
    /// The image file's original bytes, saved into project archives as they are.
    #[serde(skip)]
    pub data: Option<Arc<Vec<u8>>>,
    /// The tile, premultiplied; `None` until loaded or when the file is missing.
    #[serde(skip)]
    pub tile: Option<Arc<Pixmap>>,
}

impl Pattern {
    /// A pattern of the image file at `source`, its tile as wide as the image printed at
    /// `DEFAULT_DPI` with `units_per_mm` frame units per mm.
    pub fn open(source: PathBuf, units_per_mm: f32) -> Result<Self, String> {
        let mut p = Pattern {
            source,
            size: 1.0,
            angle_deg: 0.0,
            offset: [0.0; 2],
            data: None,
            tile: None,
        };
        p.load()?;
        let w = p.tile.as_ref().map_or(1, |t| t.width()) as f32;
        p.size = w * 25.4 / DEFAULT_DPI * units_per_mm;
        Ok(p)
    }

    /// Reads the tile from `data`, or else from the file at `source` (keeping its bytes
    /// in `data`).
    pub fn load(&mut self) -> Result<(), String> {
        let data = match &self.data {
            Some(d) => d.clone(),
            None => {
                let bytes = std::fs::read(&self.source)
                    .map_err(|e| tr!("CANNOT_OPEN_FILE", path = self.source.display(), e = e))?;
                let d = Arc::new(bytes);
                self.data = Some(d.clone());
                d
            }
        };
        let img = image::load_from_memory(&data)
            .map_err(|e| tr!("CANNOT_OPEN_FILE", path = self.source.display(), e = e))?
            .to_rgba8();
        let tile = crate::render::premultiplied_pixmap(&img)
            .ok_or_else(|| tr!("IMAGE_HAS_NO_PIXELS", path = self.source.display()))?;
        self.tile = Some(Arc::new(tile));
        Ok(())
    }

    /// The ink filling with it a frame starting at `frame`'s top-left corner; `None`
    /// until its tile is loaded.
    pub fn ink(&self, frame: Rect) -> Option<Ink> {
        let tile = self.tile.clone()?;
        let k = self.size / tile.width().max(1) as f32;
        let transform = Transform::from_scale(k, k)
            .post_rotate(self.angle_deg)
            .post_translate(frame.left() + self.offset[0], frame.top() + self.offset[1]);
        Some(Ink::Pattern { tile, transform })
    }
}

/// The same settings and the same image (the same bytes in memory).
impl PartialEq for Pattern {
    fn eq(&self, o: &Self) -> bool {
        let same_data = match (&self.data, &o.data) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        self.source == o.source
            && self.size == o.size
            && self.angle_deg == o.angle_deg
            && self.offset == o.offset
            && same_data
    }
}

/// What a pattern's image is chosen for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A shape layer's fill, or a text layer's letters.
    Layer(u64),
    /// The page background.
    Background,
}

/// Most tiles a pattern-filled shape is drawn with as vectors (PDF draws each one); past
/// that, the layer is exported as pixels.
pub const MAX_VECTOR_TILES: i64 = 4000;

/// The tiles of a pattern covering `bounds` (in the units of the paths it fills): first
/// and past-the-last column, then row; `transform` takes the `w` x `h` tile's pixels to
/// those units.
pub fn tiles(bounds: Rect, w: u32, h: u32, transform: Transform) -> Option<[i64; 4]> {
    let inv = transform.invert()?;
    let mut pts = [
        tiny_skia::Point::from_xy(bounds.left(), bounds.top()),
        tiny_skia::Point::from_xy(bounds.right(), bounds.top()),
        tiny_skia::Point::from_xy(bounds.right(), bounds.bottom()),
        tiny_skia::Point::from_xy(bounds.left(), bounds.bottom()),
    ];
    inv.map_points(&mut pts);
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let xs = pts.iter().map(|p| p.x / w);
    let ys = pts.iter().map(|p| p.y / h);
    let (x0, x1) = xs.fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
    let (y0, y1) = ys.fold((f32::MAX, f32::MIN), |(a, b), y| (a.min(y), b.max(y)));
    let r = [
        x0.floor() as i64,
        x1.ceil() as i64,
        y0.floor() as i64,
        y1.ceil() as i64,
    ];
    r.iter().all(|v| v.abs() < 1 << 40).then_some(r)
}

/// Every pixel of the tile is opaque.
pub fn opaque(tile: &Pixmap) -> bool {
    tile.data().as_chunks::<4>().0.iter().all(|p| p[3] == 255)
}
