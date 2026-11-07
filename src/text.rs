//! Text layers: what they say and how it looks, the fonts they use, and turning them
//! into pixels.
//!
//! A text layer is an image layer whose pixels are drawn from its text. Its `size_px` is
//! the box around the text in text units, `EM` units per em, so moving, scaling,
//! rotating, distorting, snapping, effects and export work as for images. Its pixmap is
//! drawn at the resolution needed: about print resolution on screen, the export DPI when
//! exporting. The font size is the layer's scale: scaling the layer scales the text.
//!
//! For text layers, the layer's `source` and `data` are the font file and its bytes. That
//! is how project archives carry the fonts along, as they do the images.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use egui::{ColorImage, TextureFilter, TextureOptions};
use serde::{Deserialize, Serialize};
use skrifa::instance::{Location, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider as _;
use skrifa::{FontRef, GlyphId, MetadataProvider as _, Tag};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::document::{Document, Homography, Layer, P2};
use crate::render;

/// Text units per em: a text layer's `size_px` and its effects are measured in these.
/// Fine enough that the effects' default sizes, made for images, suit text too: a 20 px
/// shadow distance is 5% of the font size.
pub const EM: f32 = 400.0;
/// Family name of the font built into the program, used when no other is found.
pub const BUILTIN_FAMILY: &str = "Ubuntu Light (built in)";
/// Family names of the emoji fonts built into the program, the last places missing
/// characters are looked for.
pub const EMOJI_FAMILY: &str = "Noto Emoji (built in)";
const EMOJI_ICONS_FAMILY: &str = "Emoji Icons (built in)";
/// Families tried for symbols and emoji, which the system's fallback lists may not cover
/// (Windows has none for them), before the built-in emoji fonts.
const SYMBOL_FAMILIES: &[&str] = &[
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Sans Math",
    "DejaVu Sans",
];
/// Font size of new text layers, in points.
pub const DEFAULT_SIZE_PT: f32 = 24.0;
/// Resolution of text drawn for the screen, in pixels per page mm (about 300 DPI).
const SCREEN_PX_PER_MM: f32 = 12.0;
/// Longest side of text drawn for the screen. Kept moderate: the effects of the layer
/// are worked out on this image, and export draws the text again at full resolution.
const SCREEN_SIDE: f32 = 2048.0;
/// Longest side of text drawn for the export.
const EXPORT_SIDE: f32 = 20000.0;
/// Longest side of the preview pixmap, as for images.
const PREVIEW_SIDE: u32 = 2048;
/// Extra thickness of letters made bold when the font has no bold face, in ems.
const FAKE_BOLD: f32 = 0.04;
/// Slant of letters made italic when the font has no italic face, in degrees.
const FAKE_ITALIC_DEG: f32 = 12.0;
/// Width of an empty text's box, in ems, so it can still be picked.
const EMPTY_WIDTH: f32 = 0.3;

/// How the lines of a text line up with each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// What a text layer says and how it looks. Sizes are in ems, so they follow the font
/// size, which is the layer's scale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Text {
    /// Lines are separated by `\n`.
    pub content: String,
    /// Font family name, as listed by the system.
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub align: TextAlign,
    /// Extra space after each letter, in thousandths of an em.
    pub letter_spacing: f32,
    /// Distance between baselines, in ems.
    pub line_height: f32,
    /// Straight RGBA.
    pub color: [u8; 4],
    /// Draw a line around the letters.
    pub outline: bool,
    /// Thickness of the outline outside the letters, in ems.
    pub outline_width: f32,
    /// Straight RGBA.
    pub outline_color: [u8; 4],
    /// Face within a font collection file (.ttc).
    #[serde(skip_serializing_if = "is_zero")]
    pub font_index: u32,
    /// Fonts the characters missing from the text's own font are taken from, in order.
    /// Kept up to date with the text by `sync`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<FallbackFont>,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// A font that characters missing from a text's own font are taken from. Saved with the
/// project like the text's own font: `source` is its file, in the project archive or on
/// disk.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FallbackFont {
    pub family: String,
    /// Face within a font collection file (.ttc).
    #[serde(skip_serializing_if = "is_zero")]
    pub index: u32,
    #[serde(skip_serializing_if = "is_empty_path")]
    pub source: PathBuf,
    /// The font file's bytes, once found or read.
    #[serde(skip)]
    pub data: Option<Arc<Vec<u8>>>,
}

fn is_empty_path(p: &std::path::Path) -> bool {
    p.as_os_str().is_empty()
}

// The bytes are left out: a font file is large, and the rest says which one it is.
impl PartialEq for FallbackFont {
    fn eq(&self, o: &Self) -> bool {
        (&self.family, self.index, &self.source) == (&o.family, o.index, &o.source)
            && self.data.is_some() == o.data.is_some()
    }
}

impl std::fmt::Debug for FallbackFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackFont")
            .field("family", &self.family)
            .field("index", &self.index)
            .field("source", &self.source)
            .field("loaded", &self.data.is_some())
            .finish()
    }
}

impl FallbackFont {
    /// True if this font draws `c`.
    fn covers(&self, c: char) -> bool {
        self.data.as_ref().is_some_and(|d| covers(d, self.index, c))
    }

    /// True for a font built into the program: it is not saved with projects.
    pub fn is_builtin(&self) -> bool {
        builtin_named(&self.family).is_some()
    }
}

/// True if the font has an outline for `c`. Fonts of colored bitmaps only (as some emoji
/// fonts are) have none.
fn covers(data: &[u8], index: u32, c: char) -> bool {
    FontRef::from_index(data, index).is_ok_and(|f| {
        f.charmap()
            .map(c)
            .is_some_and(|g| f.outline_glyphs().get(g).is_some())
    })
}

impl Default for Text {
    fn default() -> Self {
        Self {
            content: String::new(),
            family: BUILTIN_FAMILY.into(),
            bold: false,
            italic: false,
            align: TextAlign::Center,
            letter_spacing: 0.0,
            line_height: 1.2,
            color: [0, 0, 0, 255],
            outline: false,
            outline_width: 0.05,
            outline_color: [255, 255, 255, 255],
            font_index: 0,
            fallbacks: Vec::new(),
        }
    }
}

/// Where a text layer's drawing stands. Not saved.
#[derive(Clone, Debug, Default)]
pub struct TextState {
    /// The family or style changed: look for the font again.
    pub font_pending: bool,
    /// Identifies what the current pixmap was drawn from.
    key: u64,
    /// Left, right and top of the lines' box in the layer's frame, from the last layout:
    /// the text stays in place on its alignment side when its box changes size.
    metric: Option<[f32; 3]>,
    /// Where the cursor can stand, from the last layout, for typing on the canvas.
    pub carets: Arc<Vec<CaretLine>>,
    /// Characters that neither the text's font nor any fallback font has.
    pub missing: Vec<char>,
    /// Identifies what the fallback fonts were last worked out for.
    fallback_key: u64,
}

/// Where the cursor can stand on one line of text, in the layer's frame (text units,
/// y down).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaretLine {
    /// Top and bottom of the line.
    pub top: f32,
    pub bottom: f32,
    /// x of each place between characters, from before the first one to after the last
    /// one: one more than the line has characters.
    pub xs: Vec<f32>,
}

/// The line and the place on it of a cursor before character `index` of the text.
pub fn caret_place(lines: &[CaretLine], index: usize) -> Option<(usize, f32)> {
    let mut start = 0;
    for (i, line) in lines.iter().enumerate() {
        let n = line.xs.len().saturating_sub(1);
        if index <= start + n || i + 1 == lines.len() {
            let x = *line.xs.get(index.saturating_sub(start).min(n))?;
            return Some((i, x));
        }
        start += n + 1;
    }
    None
}

/// Index of the character a cursor on `line` nearest to `x` stands before.
pub fn caret_on_line(lines: &[CaretLine], line: usize, x: f32) -> usize {
    let start: usize = lines[..line.min(lines.len())]
        .iter()
        .map(|l| l.xs.len())
        .sum();
    let Some(l) = lines.get(line) else {
        return start.saturating_sub(1);
    };
    let col =
        l.xs.iter()
            .enumerate()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |(i, _)| i);
    start + col
}

/// Index of the character a cursor at `p` in the layer's frame stands before: on the
/// nearest line, at the nearest place between characters.
pub fn caret_at(lines: &[CaretLine], p: [f32; 2]) -> usize {
    let line = lines
        .iter()
        .position(|l| p[1] < l.bottom)
        .unwrap_or(lines.len().saturating_sub(1));
    caret_on_line(lines, line, p[0])
}

/// Places between the characters of `line` from its shaped glyphs: (cluster, pen x
/// before, pen x after, right to left), and the line's width. `rtl` is the line's
/// direction, for the places before and after everything.
fn line_carets(line: &str, glyphs: &[(u32, f32, f32, bool)], width: f32, rtl: bool) -> Vec<f32> {
    // Each cluster spans from its first glyph's pen position to its last one's.
    let mut clusters: Vec<(usize, f32, f32, bool)> = Vec::new();
    for &(c, a, b, r) in glyphs {
        let (lo, hi) = (a.min(b), a.max(b));
        match clusters.iter_mut().find(|k| k.0 == c as usize) {
            Some(k) => (k.1, k.2) = (k.1.min(lo), k.2.max(hi)),
            None => clusters.push((c as usize, lo, hi, r)),
        }
    }
    clusters.sort_by_key(|k| k.0);
    let (start_x, end_x) = if rtl { (width, 0.0) } else { (0.0, width) };
    let mut xs = Vec::new();
    let bytes: Vec<usize> = line.char_indices().map(|(b, _)| b).collect();
    for &b in &bytes {
        let k = clusters.iter().rposition(|k| k.0 <= b);
        let x = match k {
            None => start_x,
            Some(k) => {
                let (c, lo, hi, rtl) = clusters[k];
                if c == b {
                    if rtl { hi } else { lo }
                } else {
                    // Inside a ligature: shared out evenly between its characters.
                    let end = clusters.get(k + 1).map_or(line.len(), |n| n.0);
                    let n = bytes.iter().filter(|&&x| x >= c && x < end).count().max(1);
                    let j = bytes.iter().filter(|&&x| x >= c && x < b).count();
                    let t = j as f32 / n as f32;
                    if rtl {
                        hi + (lo - hi) * t
                    } else {
                        lo + (hi - lo) * t
                    }
                }
            }
        };
        xs.push(x);
    }
    xs.push(end_x);
    xs
}

/// Default name of a text layer: its first line, shortened.
pub fn auto_name(content: &str) -> String {
    let line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.is_empty() {
        return crate::i18n::tr!("DEFAULT_TEXT_LAYER_NAME").into();
    }
    let mut name: String = line.chars().take(30).collect();
    if line.chars().count() > 30 {
        name.push_str("...");
    }
    name
}

/// A new text layer saying `content` at `DEFAULT_SIZE_PT`, centered on `center`.
pub fn new_layer(id: u64, content: &str, family: String, center: P2) -> Layer {
    let mut layer = Layer::new(id, auto_name(content), PathBuf::new());
    layer.text = Some(Box::new(Text {
        content: content.into(),
        family,
        ..Text::default()
    }));
    let s = pt_to_mm(DEFAULT_SIZE_PT) / EM;
    layer.transform.center = center;
    layer.transform.scale = [s, s];
    layer
}

fn pt_to_mm(pt: f32) -> f32 {
    pt * 25.4 / 72.0
}

/// The layer's font size in points: the height of an em on the page.
pub fn size_pt(layer: &Layer) -> f32 {
    EM * layer.transform.scale[1].abs() * 72.0 / 25.4
}

/// Scales the layer to a font size, keeping the text in place on its alignment side.
pub fn set_size_pt(layer: &mut Layer, pt: f32) {
    let old = size_pt(layer);
    if old <= 0.0 || pt <= 0.0 {
        return;
    }
    let k = pt / old;
    let before = anchor_on_page(layer);
    layer.transform.scale = layer.transform.scale.map(|s| s * k);
    let after = anchor_on_page(layer);
    let c = &mut layer.transform.center;
    *c = [c[0] + before[0] - after[0], c[1] + before[1] - after[1]];
}

/// Thickness of a length in ems on the page, in mm (along the text's height).
pub fn em_to_mm(layer: &Layer, ems: f32) -> f32 {
    ems * EM * layer.transform.scale[1].abs()
}

/// Asks for the layer's font to be looked up again, after its family or style changed.
pub fn font_changed(layer: &mut Layer) {
    layer.text_state.font_pending = true;
    // Looked for again, in the new style.
    if let Some(t) = &mut layer.text {
        t.fallbacks.clear();
    }
}

/// The point of the lines' box that stays in place: the top of the alignment side.
fn anchor(align: TextAlign, m: [f32; 3]) -> P2 {
    let x = match align {
        TextAlign::Left => m[0],
        TextAlign::Center => (m[0] + m[1]) / 2.0,
        TextAlign::Right => m[1],
    };
    [x, m[2]]
}

fn anchor_on_page(layer: &Layer) -> P2 {
    let align = layer.text.as_ref().map(|t| t.align).unwrap_or_default();
    match layer.text_state.metric {
        Some(m) => layer.affine().apply(anchor(align, m)),
        None => layer.transform.center,
    }
}

// ---- Fonts ----

/// A font file found for a family and style.
#[derive(Clone)]
pub struct Found {
    pub data: Arc<Vec<u8>>,
    /// Face within the file.
    pub index: u32,
    /// The file on disk, if it is one.
    pub path: Option<PathBuf>,
}

struct SystemFonts {
    collection: fontique::Collection,
    cache: fontique::SourceCache,
    /// Sorted, with the built-in font first.
    families: Vec<String>,
    default: String,
    found: HashMap<(String, bool, bool), Option<Found>>,
    /// The fallback font found for a character and style.
    fallbacks: HashMap<(char, bool, bool), Option<(String, Found)>>,
}

impl SystemFonts {
    fn load(system: bool) -> Self {
        let start = std::time::Instant::now();
        let mut collection = fontique::Collection::new(fontique::CollectionOptions {
            shared: false,
            system_fonts: system,
        });
        let mut families: Vec<String> = collection
            .family_names()
            .filter(|n| !n.starts_with('.'))
            .map(str::to_owned)
            .collect();
        families.sort_by_key(|n| n.to_lowercase());
        families.dedup();
        families.insert(0, BUILTIN_FAMILY.into());
        let sans: Vec<_> = collection
            .generic_families(fontique::GenericFamily::SansSerif)
            .collect();
        let default = sans
            .into_iter()
            .find_map(|id| collection.family_name(id).map(str::to_owned))
            .unwrap_or_else(|| BUILTIN_FAMILY.into());
        log::debug!(
            "found {} font families in {:?}",
            families.len(),
            start.elapsed()
        );
        Self {
            collection,
            cache: fontique::SourceCache::new(fontique::SourceCacheOptions::default()),
            families,
            default,
            found: HashMap::new(),
            fallbacks: HashMap::new(),
        }
    }

    fn fallback(&mut self, c: char, bold: bool, italic: bool) -> Option<(String, Found)> {
        let key = (c, bold, italic);
        if let Some(f) = self.fallbacks.get(&key) {
            return f.clone();
        }
        let found = self.lookup_fallback(c, bold, italic);
        self.fallbacks.insert(key, found.clone());
        found
    }

    /// The first font with `c`: the system's fallback fonts for its script, then fonts of
    /// symbols, then the built-in ones.
    fn lookup_fallback(&mut self, c: char, bold: bool, italic: bool) -> Option<(String, Found)> {
        let script = char_script(c);
        let mut scripts = Vec::new();
        if !matches!(&script, b"Zyyy" | b"Zinh" | b"Zzzz") {
            scripts.push(script);
        }
        scripts.extend([*b"Zsym", *b"Zsye"]);
        let mut names: Vec<String> = Vec::new();
        for s in scripts {
            let ids: Vec<_> = self
                .collection
                .fallback_families(fontique::Script::from_bytes(s))
                .collect();
            names.extend(
                ids.into_iter()
                    .filter_map(|id| self.collection.family_name(id).map(str::to_owned)),
            );
        }
        names.extend(SYMBOL_FAMILIES.iter().map(|s| (*s).to_owned()));
        let latin: Vec<_> = self
            .collection
            .fallback_families(fontique::Script::from_bytes(*b"Latn"))
            .collect();
        names.extend(
            latin
                .into_iter()
                .filter_map(|id| self.collection.family_name(id).map(str::to_owned)),
        );
        let mut tried = std::collections::HashSet::new();
        for name in names {
            if !tried.insert(name.clone()) {
                continue;
            }
            if let Some(found) = self.find(&name, bold, italic)
                && covers(&found.data, found.index, c)
            {
                return Some((name, found));
            }
        }
        [BUILTIN_FAMILY, EMOJI_FAMILY, EMOJI_ICONS_FAMILY]
            .into_iter()
            .filter_map(|name| Some((name.to_owned(), builtin_named(name)?)))
            .find(|(_, f)| covers(&f.data, f.index, c))
    }

    fn find(&mut self, family: &str, bold: bool, italic: bool) -> Option<Found> {
        let key = (family.to_owned(), bold, italic);
        if let Some(f) = self.found.get(&key) {
            return f.clone();
        }
        let found = self.lookup(family, bold, italic);
        self.found.insert(key, found.clone());
        found
    }

    fn lookup(&mut self, family: &str, bold: bool, italic: bool) -> Option<Found> {
        let fam = self.collection.family_by_name(family)?;
        let style = if italic {
            fontique::FontStyle::Italic
        } else {
            fontique::FontStyle::Normal
        };
        let weight = if bold {
            fontique::FontWeight::BOLD
        } else {
            fontique::FontWeight::NORMAL
        };
        let font = fam.match_font(fontique::FontWidth::NORMAL, style, weight, true)?;
        let blob = font.load(Some(&mut self.cache))?;
        let path = match font.source().kind() {
            fontique::SourceKind::Path(p) => Some(p.to_path_buf()),
            _ => None,
        };
        Some(Found {
            data: Arc::new(blob.data().to_vec()),
            index: font.index(),
            path,
        })
    }
}

/// The fonts installed on the system, listed on a background thread at startup since
/// that can take a moment.
pub struct FontLibrary {
    fonts: Arc<OnceLock<Mutex<SystemFonts>>>,
}

impl FontLibrary {
    /// Starts listing the system fonts.
    pub fn new() -> Self {
        let fonts = Arc::new(OnceLock::new());
        let slot = fonts.clone();
        std::thread::Builder::new()
            .name("fonts".into())
            .spawn(move || {
                slot.get_or_init(|| Mutex::new(SystemFonts::load(true)));
            })
            .ok();
        Self { fonts }
    }

    /// Only the built-in font, for tests.
    #[cfg(test)]
    pub fn builtin_only() -> Self {
        let fonts = OnceLock::new();
        fonts.get_or_init(|| Mutex::new(SystemFonts::load(false)));
        Self {
            fonts: Arc::new(fonts),
        }
    }

    /// Waits for the list.
    fn wait(&self) -> std::sync::MutexGuard<'_, SystemFonts> {
        let fonts = self
            .fonts
            .get_or_init(|| Mutex::new(SystemFonts::load(true)));
        fonts.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Calls `f` with the family names, or returns None while they are still being
    /// listed.
    pub fn with_families<R>(&self, f: impl FnOnce(&[String]) -> R) -> Option<R> {
        let fonts = self.fonts.get()?.lock().unwrap_or_else(|e| e.into_inner());
        Some(f(&fonts.families))
    }

    /// The system's sans-serif family, for new text layers.
    pub fn default_family(&self) -> String {
        self.wait().default.clone()
    }

    /// The font file for a family and style; the closest style when the family lacks it.
    pub fn find(&self, family: &str, bold: bool, italic: bool) -> Option<Found> {
        if let Some(found) = builtin_named(family) {
            return Some(found);
        }
        self.wait().find(family, bold, italic)
    }

    /// A font that has `c`, and its family name: one of the system's fallback fonts, or a
    /// built-in one.
    pub fn fallback_for(&self, c: char, bold: bool, italic: bool) -> Option<(String, Found)> {
        self.wait().fallback(c, bold, italic)
    }
}

impl Default for FontLibrary {
    fn default() -> Self {
        Self::new()
    }
}

/// The font built into the program.
fn builtin() -> Found {
    builtin_named(BUILTIN_FAMILY).expect("built-in font")
}

/// A font built into the program, by family name.
fn builtin_named(family: &str) -> Option<Found> {
    static UBUNTU: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    static EMOJI: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    static ICONS: OnceLock<Arc<Vec<u8>>> = OnceLock::new();
    let (slot, bytes) = match family {
        BUILTIN_FAMILY => (&UBUNTU, epaint_default_fonts::UBUNTU_LIGHT),
        EMOJI_FAMILY => (&EMOJI, epaint_default_fonts::NOTO_EMOJI_REGULAR),
        EMOJI_ICONS_FAMILY => (&ICONS, epaint_default_fonts::EMOJI_ICON),
        _ => return None,
    };
    Some(Found {
        data: slot.get_or_init(|| Arc::new(bytes.to_vec())).clone(),
        index: 0,
        path: None,
    })
}

/// ISO 15924 tag of the script `c` is written in, as the shaper sees it.
fn char_script(c: char) -> [u8; 4] {
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.add(c, 0);
    buffer.guess_segment_properties();
    buffer.script().tag().to_be_bytes()
}

/// False if the font's license forbids embedding it in documents (OpenType "restricted
/// license embedding"), so it must not go into project archives.
pub fn embeddable(data: &[u8], index: u32) -> bool {
    FontRef::from_index(data, index)
        .ok()
        .and_then(|f| f.os2().ok())
        .is_none_or(|os2| os2.fs_type() & 0x000F != 0x0002)
}

/// Gives the layer its font file: the one it has (from a project archive), the file it
/// links to, or the system's font for its family. With none of them, the built-in font
/// is used and the layer is marked broken.
fn ensure_font(layer: &mut Layer, fonts: &FontLibrary) {
    let Some(text) = &layer.text else { return };
    let pending = std::mem::take(&mut layer.text_state.font_pending);
    if !pending {
        if layer.data.is_some() || layer.broken {
            return;
        }
        // A linked project points to the font file it was made with.
        if layer.source.is_absolute()
            && let Ok(bytes) = std::fs::read(&layer.source)
            && FontRef::from_index(&bytes, text.font_index).is_ok()
        {
            layer.data = Some(Arc::new(bytes));
            return;
        }
    }
    match fonts.find(&text.family, text.bold, text.italic) {
        Some(found) => {
            layer.data = Some(found.data);
            layer.source = found.path.unwrap_or_default();
            if let Some(t) = &mut layer.text {
                t.font_index = found.index;
            }
            layer.broken = false;
        }
        // A missing style keeps the font it had: bold and italic are then faked.
        None if pending && layer.data.is_some() => {}
        None => {
            layer.data = None;
            layer.broken = true;
        }
    }
}

/// Brings the text's fallback fonts up to date with its characters: loads the ones it
/// has, finds one for each character its own font lacks, and drops those no longer
/// used. Characters no font has go to `TextState::missing`.
fn ensure_fallbacks(layer: &mut Layer, fonts: &FontLibrary) {
    let Some(text) = &layer.text else { return };
    let key = fallback_key(layer);
    if layer.text_state.fallback_key == key {
        return;
    }
    let main_data = layer.data.clone().unwrap_or(builtin().data);
    let main_index = if layer.data.is_some() {
        text.font_index
    } else {
        0
    };
    let (bold, italic) = (text.bold, text.italic);
    let mut list = text.fallbacks.clone();
    // Load the ones not loaded: from their file on disk, or else the family installed
    // here.
    for f in list.iter_mut().filter(|f| f.data.is_none()) {
        if f.source.is_absolute()
            && let Ok(bytes) = std::fs::read(&f.source)
            && FontRef::from_index(&bytes, f.index).is_ok()
        {
            f.data = Some(Arc::new(bytes));
        } else if let Some(found) = fonts.find(&f.family, bold, italic) {
            f.data = Some(found.data);
            f.index = found.index;
            f.source = found.path.unwrap_or_default();
        }
    }
    let mut needed: Vec<char> = text
        .content
        .chars()
        .filter(|&c| !c.is_control() && !joins_previous(c))
        .filter(|&c| !covers(&main_data, main_index, c))
        .collect();
    needed.sort_unstable();
    needed.dedup();
    let mut used = vec![false; list.len()];
    let mut missing = Vec::new();
    for c in needed {
        if let Some(i) = list.iter().position(|f| f.covers(c)) {
            used[i] = true;
            continue;
        }
        match fonts.fallback_for(c, bold, italic) {
            Some((family, found)) => {
                list.push(FallbackFont {
                    family,
                    index: found.index,
                    source: found.path.unwrap_or_default(),
                    data: Some(found.data),
                });
                used.push(true);
            }
            None => missing.push(c),
        }
    }
    let mut keep = used.into_iter();
    list.retain(|_| keep.next().unwrap_or(false));
    if let Some(t) = &mut layer.text
        && t.fallbacks != list
    {
        t.fallbacks = list;
    }
    layer.text_state.missing = missing;
    layer.text_state.fallback_key = fallback_key(layer);
}

/// Identifies what a text's fallback fonts depend on: its characters, style and fonts.
fn fallback_key(layer: &Layer) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(t) = &layer.text {
        (&t.content, &t.family, t.bold, t.italic, t.font_index).hash(&mut h);
        for f in &t.fallbacks {
            (
                &f.family,
                f.index,
                f.data.as_ref().map(|d| Arc::as_ptr(d) as usize),
            )
                .hash(&mut h);
        }
    }
    layer
        .data
        .as_ref()
        .map(|d| Arc::as_ptr(d) as usize)
        .hash(&mut h);
    h.finish()
}

/// Calls `f` with the layer's text and its fonts: its own one (or the built-in one),
/// then the fallback fonts it has loaded.
fn with_fonts<R>(layer: &Layer, f: impl FnOnce(&Text, &[FontRef]) -> R) -> Option<R> {
    let text = layer.text.as_ref()?;
    let fallback = builtin();
    let data = layer.data.clone().unwrap_or(fallback.data.clone());
    let main = FontRef::from_index(data.as_slice(), text.font_index)
        .or_else(|_| FontRef::new(fallback.data.as_slice()))
        .ok()?;
    let mut fonts = vec![main];
    fonts.extend(
        text.fallbacks
            .iter()
            .filter_map(|fb| FontRef::from_index(fb.data.as_deref()?.as_slice(), fb.index).ok()),
    );
    Some(f(text, &fonts))
}

// ---- Layout ----

/// The text laid out in the layer's frame, in text units, y down.
pub struct Laid {
    /// The letters' outlines; None if there are none.
    pub path: Option<Path>,
    /// Box around the letters, their outline and the lines' height, in whole units.
    pub size: [u32; 2],
    /// Left, right and top of the lines' box.
    pub metric: [f32; 3],
    /// Extra thickness for a fake bold, in units.
    pub embolden: f32,
    /// Where the cursor can stand on each line.
    pub carets: Vec<CaretLine>,
}

/// Collects a glyph outline into a path, placed at a pen position, slanted, y flipped.
struct Pen<'a> {
    b: &'a mut PathBuilder,
    x: f32,
    y: f32,
    skew: f32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x + y * self.skew, self.y - y)
    }
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.b.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.b.line_to(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.at(cx0, cy0);
        let (x, y) = self.at(x, y);
        self.b.quad_to(cx0, cy0, x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.at(cx0, cy0);
        let (cx1, cy1) = self.at(cx1, cy1);
        let (x, y) = self.at(x, y);
        self.b.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }
    fn close(&mut self) {
        self.b.close();
    }
}

/// How the font is made to look bold or italic: variation axes when it has them,
/// otherwise thicker or slanted outlines when the face itself isn't bold or italic.
struct Style {
    variations: Vec<(Tag, f32)>,
    embolden: bool,
    skew: f32,
}

fn style_for(font: &FontRef, bold: bool, italic: bool) -> Style {
    let attrs = font.attributes();
    let axes = font.axes();
    let has = |tag: &[u8; 4]| axes.get_by_tag(Tag::new(tag)).is_some();
    let mut variations = Vec::new();
    let mut embolden = false;
    if has(b"wght") {
        variations.push((Tag::new(b"wght"), if bold { 700.0 } else { 400.0 }));
    } else if bold && attrs.weight.value() < 600.0 {
        embolden = true;
    }
    let mut skew = 0.0;
    if italic && attrs.style == skrifa::attribute::Style::Normal {
        if has(b"ital") {
            variations.push((Tag::new(b"ital"), 1.0));
        } else if has(b"slnt") {
            variations.push((Tag::new(b"slnt"), -FAKE_ITALIC_DEG));
        } else {
            skew = FAKE_ITALIC_DEG.to_radians().tan();
        }
    }
    Style {
        variations,
        embolden,
        skew,
    }
}

/// A glyph: the font it is from (index into the fonts laid out with), its id, and its
/// pen position on the line (x, rise).
type Placed = (usize, u32, f32, f32);

/// What it takes to shape and draw with one font.
struct Face<'a> {
    font: &'a FontRef<'a>,
    style: Style,
    location: Location,
    /// Font units to text units.
    to_units: f32,
    charmap: skrifa::charmap::Charmap<'a>,
}

impl<'a> Face<'a> {
    fn new(font: &'a FontRef<'a>, text: &Text) -> Self {
        let style = style_for(font, text.bold, text.italic);
        let location: Location = font.axes().location(style.variations.iter().copied());
        let upem = font.head().map(|h| h.units_per_em()).unwrap_or(1000).max(1);
        Self {
            font,
            location,
            to_units: EM / f32::from(upem),
            charmap: font.charmap(),
            style,
        }
    }

    fn has(&self, c: char) -> bool {
        self.charmap.map(c).is_some()
    }
}

/// True for characters that belong with the one before them and must come from the same
/// font: combining marks, joiners, variation selectors, emoji modifiers and tags.
fn joins_previous(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
        | 0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF
        | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

/// Splits a line into runs of characters taken from the same font: the first of `faces`
/// that has the character, or the first one if none has it. Returns (start byte, end
/// byte, face).
fn runs(line: &str, faces: &[Face]) -> Vec<(usize, usize, usize)> {
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    for (i, c) in line.char_indices() {
        let end = i + c.len_utf8();
        let face = match out.last() {
            Some(&(_, _, f)) if joins_previous(c) => f,
            _ => faces.iter().position(|f| f.has(c)).unwrap_or(0),
        };
        match out.last_mut() {
            Some(run) if run.2 == face => run.1 = end,
            _ => out.push((i, end, face)),
        }
    }
    out
}

/// Lays the text out with `font`.
#[cfg(test)]
pub fn layout(text: &Text, font: &FontRef) -> Laid {
    layout_with(text, std::slice::from_ref(font))
}

/// Lays the text out with `fonts`: the first one is the text's own; characters it lacks
/// come from the first of the others that has them. Line heights and the fake bold
/// follow the text's own font.
pub fn layout_with(text: &Text, fonts: &[FontRef]) -> Laid {
    let faces: Vec<Face> = fonts.iter().map(|f| Face::new(f, text)).collect();
    let main = &faces[0];
    let metrics = main.font.metrics(Size::new(EM), &main.location);
    let ascent = metrics.ascent;
    let descent = -metrics.descent;
    let line_step = text.line_height * EM;
    let spacing = text.letter_spacing * EM / 1000.0;

    let datas: Vec<harfrust::ShaperData> = faces
        .iter()
        .map(|f| harfrust::ShaperData::new(f.font))
        .collect();
    let instances: Vec<harfrust::ShaperInstance> = faces
        .iter()
        .map(|f| {
            harfrust::ShaperInstance::from_variations(
                f.font,
                f.style
                    .variations
                    .iter()
                    .map(|&(tag, value)| harfrust::Variation { tag, value }),
            )
        })
        .collect();
    let shapers: Vec<harfrust::Shaper> = faces
        .iter()
        .zip(&datas)
        .zip(&instances)
        .map(|((f, d), i)| d.shaper(f.font).instance(Some(i)).build())
        .collect();
    let outlines: Vec<_> = faces.iter().map(|f| f.font.outline_glyphs()).collect();

    // Shape each line, run by run: glyphs with their pen position, the line's width, and
    // the places between its characters relative to its start.
    let lines: Vec<&str> = text.content.split('\n').collect();
    let mut shaped: Vec<(Vec<Placed>, f32)> = Vec::new();
    let mut line_xs: Vec<Vec<f32>> = Vec::new();
    for line in &lines {
        let trimmed = line.trim_end_matches('\r');
        let mut x = 0.0;
        let mut out = Vec::new();
        let mut pens = Vec::new();
        let mut line_rtl = None;
        for (start, end, fi) in runs(trimmed, &faces) {
            let face = &faces[fi];
            let mut buffer = harfrust::UnicodeBuffer::new();
            buffer.push_str(&trimmed[start..end]);
            buffer.guess_segment_properties();
            let rtl = buffer.direction() == harfrust::Direction::RightToLeft;
            line_rtl.get_or_insert(rtl);
            let glyphs = shapers[fi].shape(buffer, harfrust::ShapeOptions::new());
            let k = face.to_units;
            for (info, pos) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
                out.push((
                    fi,
                    info.glyph_id,
                    x + pos.x_offset as f32 * k,
                    pos.y_offset as f32 * k,
                ));
                let before = x;
                x += pos.x_advance as f32 * k;
                if pos.x_advance != 0 {
                    x += spacing;
                }
                pens.push((start as u32 + info.cluster, before, x, rtl));
            }
        }
        if !out.is_empty() {
            x -= spacing;
        }
        let w = x.max(0.0);
        for p in &mut pens {
            (p.1, p.2) = (p.1.min(w), p.2.min(w));
        }
        let mut xs = line_carets(trimmed, &pens, w, line_rtl.unwrap_or(false));
        // The line's ending \r, if any, stands at its end.
        let end = *xs.last().unwrap_or(&0.0);
        xs.extend(std::iter::repeat_n(end, line.len() - trimmed.len()));
        line_xs.push(xs);
        shaped.push((out, w));
    }
    let width = shaped.iter().map(|(_, w)| *w).fold(0.0f32, f32::max).max(
        if text.content.trim().is_empty() {
            EMPTY_WIDTH * EM
        } else {
            0.0
        },
    );

    let mut b = PathBuilder::new();
    let mut carets = Vec::new();
    for (i, ((glyphs, w), xs)) in shaped.iter().zip(line_xs).enumerate() {
        let x0 = match text.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (width - w) / 2.0,
            TextAlign::Right => width - w,
        };
        let baseline = ascent + i as f32 * line_step;
        carets.push(CaretLine {
            top: baseline - ascent,
            bottom: baseline + descent,
            xs: xs.into_iter().map(|x| x0 + x).collect(),
        });
        for &(fi, gid, x, y) in glyphs {
            let Some(glyph) = outlines[fi].get(GlyphId::new(gid)) else {
                continue;
            };
            let face = &faces[fi];
            let mut pen = Pen {
                b: &mut b,
                x: x0 + x,
                y: baseline - y,
                skew: face.style.skew,
            };
            // Outlines come in font units scaled to an em of EM units: the same for
            // every font, whatever its units per em.
            glyph
                .draw(
                    DrawSettings::unhinted(Size::new(EM), &face.location),
                    &mut pen,
                )
                .ok();
        }
    }
    let path = b.finish();

    let height = ascent + descent + (lines.len() as f32 - 1.0) * line_step;
    let embolden = if main.style.embolden {
        FAKE_BOLD * EM
    } else {
        0.0
    };
    let outline = if text.outline {
        text.outline_width.max(0.0) * EM
    } else {
        0.0
    };
    let (mut lo, mut hi) = ([0.0f32, 0.0f32], [width, height.max(1.0)]);
    if let Some(p) = &path {
        let r = p.bounds();
        let e = outline + embolden / 2.0 + 1.0;
        lo = [lo[0].min(r.left() - e), lo[1].min(r.top() - e)];
        hi = [hi[0].max(r.right() + e), hi[1].max(r.bottom() + e)];
    }
    let lo = lo.map(f32::floor);
    let hi = hi.map(f32::ceil);
    let path = path.and_then(|p| p.transform(Transform::from_translate(-lo[0], -lo[1])));
    for c in &mut carets {
        c.top -= lo[1];
        c.bottom -= lo[1];
        c.xs.iter_mut().for_each(|x| *x -= lo[0]);
    }
    Laid {
        path,
        size: [
            (hi[0] - lo[0]).max(1.0) as u32,
            (hi[1] - lo[1]).max(1.0) as u32,
        ],
        metric: [-lo[0], width - lo[0], -lo[1]],
        embolden,
        carets,
    }
}

/// One coat of paint on the letters: their outlines filled, or stroked this wide.
struct Coat<'a> {
    path: &'a Path,
    color: [u8; 4],
    stroke: Option<f32>,
}

/// The coats that draw laid out text, bottom first. The outline goes behind the letters,
/// so they keep their shape; a fake bold thickens them with a stroke.
fn coats<'a>(laid: &'a Laid, text: &Text) -> Vec<Coat<'a>> {
    let Some(path) = &laid.path else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if text.outline && text.outline_width > 0.0 {
        out.push(Coat {
            path,
            color: text.outline_color,
            stroke: Some(2.0 * text.outline_width * EM + laid.embolden),
        });
    }
    if laid.embolden > 0.0 {
        out.push(Coat {
            path,
            color: text.color,
            stroke: Some(laid.embolden),
        });
    }
    out.push(Coat {
        path,
        color: text.color,
        stroke: None,
    });
    out
}

fn round_stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

/// Draws laid out text at `px_per_unit`.
pub fn paint(laid: &Laid, text: &Text, px_per_unit: f32) -> Option<Pixmap> {
    let w = (laid.size[0] as f32 * px_per_unit).round().max(1.0);
    let h = (laid.size[1] as f32 * px_per_unit).round().max(1.0);
    let mut pixmap = Pixmap::new(w as u32, h as u32)?;
    // Exactly the layer's frame, even where the pixel size was rounded.
    let ts = Transform::from_scale(w / laid.size[0] as f32, h / laid.size[1] as f32);
    for coat in coats(laid, text) {
        let c = coat.color;
        let mut paint = Paint::default();
        paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
        paint.anti_alias = true;
        match coat.stroke {
            Some(width) => {
                pixmap.stroke_path(coat.path, &paint, &round_stroke(width), ts, None);
            }
            None => pixmap.fill_path(coat.path, &paint, FillRule::Winding, ts, None),
        }
    }
    Some(pixmap)
}

/// A text layer as filled shapes, for vector output such as PDF.
#[derive(Debug)]
pub struct Outlines {
    /// Shapes in the layer's frame (text units, y down), each filled with its straight
    /// RGBA color under the non-zero rule, bottom first: the outline, then the letters.
    pub fills: Vec<(Path, [u8; 4])>,
    /// Text units to page mm, including any distortion.
    pub map: Homography,
}

/// Fineness of strokes turned into shapes: a few times the text unit, itself about
/// 1/400 em.
const STROKE_RES: f32 = 4.0;

/// The layer's text as filled shapes, laid out as for drawing it. Strokes (outline, fake
/// bold) become shapes of their own.
pub fn outlines(layer: &Layer) -> Option<Outlines> {
    let text = layer.text.as_ref()?;
    let laid = with_fonts(layer, layout_with)?;
    let fills = coats(&laid, text)
        .into_iter()
        .filter_map(|coat| {
            let path = match coat.stroke {
                Some(width) => coat.path.stroke(&round_stroke(width), STROKE_RES)?,
                None => coat.path.clone(),
            };
            Some((path, coat.color))
        })
        .collect();
    // The layer's box is the laid out text's, kept up to date by `sync`.
    Some(Outlines {
        fills,
        map: layer.homography(),
    })
}

/// Lays out and draws the layer's text at `px_per_unit` with its font, or the built-in
/// one.
fn draw(layer: &Layer, px_per_unit: f32) -> Option<(Laid, Pixmap)> {
    let text = layer.text.as_ref()?;
    let laid = with_fonts(layer, layout_with)?;
    let pixmap = paint(&laid, text, px_per_unit)?;
    Some((laid, pixmap))
}

/// Text units to pixels for a page density, so the longest side stays under `max_side`.
fn px_per_unit(layer: &Layer, size: [u32; 2], px_per_mm: f32, max_side: f32) -> f32 {
    let mm_per_unit = layer.transform.scale[0]
        .abs()
        .max(layer.transform.scale[1].abs());
    let side = size[0].max(size[1]).max(1) as f32;
    (px_per_mm * mm_per_unit).min(max_side / side).max(1e-3)
}

/// Resolution of the text drawn for the screen, for a box of `size` units: rounded up
/// to steps of a half power of two, so resizing the layer redraws its text only now and
/// then, but never over `max_side` pixels.
fn screen_ppu(layer: &Layer, size: [u32; 2], max_side: f32) -> f32 {
    let v = px_per_unit(layer, size, SCREEN_PX_PER_MM, max_side);
    let side = size[0].max(size[1]).max(1) as f32;
    2f32.powf((v.log2() * 2.0).ceil() / 2.0)
        .min(max_side / side)
        .max(1e-3)
}

fn key(layer: &Layer, ppu: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&layer.text)
        .unwrap_or_default()
        .hash(&mut h);
    layer
        .data
        .as_ref()
        .map(|d| Arc::as_ptr(d) as usize)
        .hash(&mut h);
    if let Some(t) = &layer.text {
        for f in &t.fallbacks {
            f.data
                .as_ref()
                .map(|d| Arc::as_ptr(d) as usize)
                .hash(&mut h);
        }
    }
    ppu.to_bits().hash(&mut h);
    h.finish()
}

/// Brings text layers' fonts, size and pixels up to date with their text. Returns true
/// if one was redrawn.
pub fn sync(ctx: &egui::Context, doc: &mut Document, fonts: &FontLibrary) -> bool {
    let mut changed = false;
    doc.for_each_layer_mut(|layer| {
        if layer.text.is_some() {
            changed |= sync_layer(ctx, layer, fonts);
        }
    });
    changed
}

fn sync_layer(ctx: &egui::Context, layer: &mut Layer, fonts: &FontLibrary) -> bool {
    ensure_font(layer, fonts);
    ensure_fallbacks(layer, fonts);
    let max_side = SCREEN_SIDE.min(ctx.input(|i| i.max_texture_side) as f32);
    let ppu = screen_ppu(layer, layer.size_px, max_side);
    if layer.text_state.key == key(layer, ppu) && layer.pixmap.is_some() {
        return false;
    }
    // Lay out at the current resolution; redraw if the box changed enough to need
    // another.
    let Some((mut laid, mut pixmap)) = draw(layer, ppu) else {
        return false;
    };
    let ppu2 = screen_ppu(layer, laid.size, max_side);
    if ppu2 < ppu
        && let Some((l, p)) = draw(layer, ppu2)
    {
        (laid, pixmap) = (l, p);
    }
    place(layer, &laid);
    layer.text_state.key = key(layer, ppu2.min(ppu));
    let full = crate::effects::cache::demultiply(&pixmap);
    let preview = render::downscale(&full, PREVIEW_SIDE);
    let color = ColorImage::from_rgba_unmultiplied(
        [full.width() as usize, full.height() as usize],
        full.as_raw(),
    );
    let options = TextureOptions {
        mipmap_mode: Some(TextureFilter::Linear),
        ..TextureOptions::LINEAR
    };
    layer.texture = Some(ctx.load_texture(format!("layer-{}", layer.id), color, options));
    layer.preview = render::premultiplied_pixmap(&preview).map(Arc::new);
    layer.pixmap = Some(Arc::new(pixmap));
    true
}

/// Gives the layer the laid out text's size, keeping the text in place on its alignment
/// side when the size changes.
fn place(layer: &mut Layer, laid: &Laid) {
    let align = layer.text.as_ref().map(|t| t.align).unwrap_or_default();
    if let Some(old) = layer.text_state.metric
        && laid.size != layer.size_px
    {
        let before = layer.affine().apply(anchor(align, old));
        layer.size_px = laid.size;
        let after = layer.affine().apply(anchor(align, laid.metric));
        let c = &mut layer.transform.center;
        *c = [c[0] + before[0] - after[0], c[1] + before[1] - after[1]];
    } else {
        layer.size_px = laid.size;
    }
    layer.text_state.metric = Some(laid.metric);
    if *layer.text_state.carets != laid.carets {
        layer.text_state.carets = Arc::new(laid.carets.clone());
    }
}

/// Redraws the visible text layers of `doc` (a copy made for the export) at the export
/// resolution, so text prints sharp at any DPI.
pub fn prepare_export(doc: &mut Document, dpi: f32) {
    let px_per_mm = dpi / 25.4;
    doc.for_each_layer_mut(|layer| {
        if layer.text.is_none() || !layer.visible {
            return;
        }
        let ppu = px_per_unit(layer, layer.size_px, px_per_mm, EXPORT_SIDE);
        if let Some((_, pixmap)) = draw(layer, ppu) {
            layer.pixmap = Some(Arc::new(pixmap));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin_font() -> FontRef<'static> {
        FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).unwrap()
    }

    fn text(content: &str) -> Text {
        Text {
            content: content.into(),
            ..Text::default()
        }
    }

    #[test]
    fn longer_text_makes_a_wider_box() {
        let font = builtin_font();
        let short = layout(&text("Hi"), &font);
        let long = layout(&text("Hello world"), &font);
        assert!(
            long.size[0] > short.size[0] * 2,
            "{:?} {:?}",
            short.size,
            long.size
        );
        assert_eq!(short.size[1], long.size[1], "same height for one line");
        let two = layout(&text("Hello\nworld"), &font);
        assert!(two.size[1] > long.size[1] + (EM as u32));
        assert!(two.size[0] < long.size[0]);
    }

    #[test]
    fn letter_spacing_and_outline_widen_the_box() {
        let font = builtin_font();
        let plain = layout(&text("Hello"), &font);
        let spaced = layout(
            &Text {
                letter_spacing: 200.0,
                ..text("Hello")
            },
            &font,
        );
        // Four gaps of 0.2 em.
        let extra = spaced.size[0] as f32 - plain.size[0] as f32;
        assert!((extra - 0.8 * EM).abs() < 3.0, "{extra}");
        let outlined = layout(
            &Text {
                outline: true,
                outline_width: 0.1,
                ..text("Hello")
            },
            &font,
        );
        assert!(outlined.size[0] > plain.size[0] + 5, "{:?}", outlined.size);
    }

    #[test]
    fn fake_bold_and_italic_with_a_font_lacking_them() {
        let font = builtin_font();
        let s = style_for(&font, true, true);
        assert!(
            s.embolden && s.skew > 0.0,
            "Ubuntu Light has no bold or italic"
        );
        let plain = layout(&text("Hello"), &font);
        let bold = layout(
            &Text {
                bold: true,
                ..text("Hello")
            },
            &font,
        );
        assert!(bold.embolden > 0.0 && bold.size[0] >= plain.size[0]);
        // Thicker letters: more ink.
        let ink = |l: &Laid, t: &Text| {
            let pm = paint(l, t, 1.0).unwrap();
            pm.pixels().iter().map(|p| p.alpha() as u32).sum::<u32>()
        };
        let bold_text = Text {
            bold: true,
            ..text("Hello")
        };
        assert!(ink(&bold, &bold_text) > ink(&plain, &text("Hello")) * 11 / 10);
    }

    #[test]
    fn paints_the_letters_in_their_color() {
        let font = builtin_font();
        let t = Text {
            color: [255, 0, 0, 255],
            ..text("H")
        };
        let laid = layout(&t, &font);
        let pm = paint(&laid, &t, 1.0).unwrap();
        assert_eq!([pm.width(), pm.height()], laid.size);
        let red = pm
            .pixels()
            .iter()
            .filter(|p| p.alpha() == 255 && p.red() == 255 && p.green() == 0)
            .count();
        assert!(red > 100, "{red} red pixels");
        // Twice the resolution, four times the pixels.
        let big = paint(&laid, &t, 2.0).unwrap();
        assert_eq!(big.width(), laid.size[0] * 2);
    }

    #[test]
    fn empty_text_still_has_a_box() {
        let laid = layout(&text(""), &builtin_font());
        assert!(laid.path.is_none());
        assert!(laid.size[0] as f32 >= EMPTY_WIDTH * EM && laid.size[1] > 50);
    }

    #[test]
    fn growing_text_stays_put_on_its_alignment_side() {
        let font = builtin_font();
        let mut layer = new_layer(1, "Hi", BUILTIN_FAMILY.into(), [100.0, 50.0]);
        layer.text.as_mut().unwrap().align = TextAlign::Left;
        let laid = layout(layer.text.as_ref().unwrap(), &font);
        place(&mut layer, &laid);
        let left = layer.affine().apply(anchor(TextAlign::Left, laid.metric));
        layer.text.as_mut().unwrap().content = "Hi there, a longer text".into();
        let laid = layout(layer.text.as_ref().unwrap(), &font);
        place(&mut layer, &laid);
        let now = layer.affine().apply(anchor(TextAlign::Left, laid.metric));
        assert!((now[0] - left[0]).abs() < 1e-3 && (now[1] - left[1]).abs() < 1e-3);
        assert!(layer.transform.center[0] > 100.0, "grew to the right");
    }

    #[test]
    fn font_size_is_the_layer_scale() {
        let mut layer = new_layer(1, "Hi", BUILTIN_FAMILY.into(), [0.0, 0.0]);
        assert!((size_pt(&layer) - DEFAULT_SIZE_PT).abs() < 1e-3);
        set_size_pt(&mut layer, 48.0);
        assert!((size_pt(&layer) - 48.0).abs() < 1e-3);
        assert!((em_to_mm(&layer, 1.0) - pt_to_mm(48.0)).abs() < 1e-3);
    }

    #[test]
    fn missing_family_falls_back_to_the_built_in_font() {
        let fonts = FontLibrary::builtin_only();
        let mut layer = new_layer(1, "Hi", "No Such Family".into(), [0.0, 0.0]);
        ensure_font(&mut layer, &fonts);
        assert!(layer.broken && layer.data.is_none());
        assert!(draw(&layer, 1.0).is_some(), "drawn with the built-in font");
        layer.text.as_mut().unwrap().family = BUILTIN_FAMILY.into();
        font_changed(&mut layer);
        ensure_font(&mut layer, &fonts);
        assert!(!layer.broken && layer.data.is_some());
        assert!(embeddable(layer.data.as_ref().unwrap(), 0));
    }

    #[test]
    fn screen_text_stays_under_the_size_limit() {
        let font = builtin_font();
        let mut layer = new_layer(1, "Text", BUILTIN_FAMILY.into(), [0.0, 0.0]);
        layer.size_px = layout(layer.text.as_ref().unwrap(), &font).size;
        for pt in [8.0, 24.0, 200.0, 600.0, 2000.0] {
            set_size_pt(&mut layer, pt);
            let ppu = screen_ppu(&layer, layer.size_px, SCREEN_SIDE);
            let side = layer.size_px[0].max(layer.size_px[1]) as f32 * ppu;
            assert!(side <= SCREEN_SIDE + 0.5, "{pt} pt: {side} px");
        }
    }

    #[test]
    fn cursor_places_follow_the_letters() {
        let font = builtin_font();
        let mut t = text("Hi\nthere");
        t.align = TextAlign::Left;
        let laid = layout(&t, &font);
        let c = &laid.carets;
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].xs.len(), 3);
        assert_eq!(c[1].xs.len(), 6);
        for l in c {
            assert!(l.xs.windows(2).all(|w| w[0] < w[1]), "{:?}", l.xs);
        }
        assert!(c[0].bottom <= c[1].top + 1.0 && c[0].top < c[0].bottom);
        // Index 3 is the start of the second line, after the line break.
        assert_eq!(caret_place(c, 3), Some((1, c[1].xs[0])));
        assert_eq!(caret_place(c, 2), Some((0, c[0].xs[2])));
        assert_eq!(caret_place(c, 99), Some((1, c[1].xs[5])));
        let mid = |l: &CaretLine| (l.top + l.bottom) / 2.0;
        assert_eq!(caret_at(c, [c[1].xs[2] + 1.0, mid(&c[1])]), 5);
        assert_eq!(caret_at(c, [-100.0, -100.0]), 0);
        assert_eq!(caret_at(c, [1e6, 1e6]), 8);
    }

    #[test]
    fn cursor_inside_a_ligature_is_shared_out() {
        let pens = [(0, 0.0, 10.0, false), (2, 10.0, 15.0, false)];
        // "ffi": one glyph for "ff", then "i".
        assert_eq!(
            line_carets("ffi", &pens, 15.0, false),
            vec![0.0, 5.0, 10.0, 15.0]
        );
        // Right to left: the first character is on the right.
        let pens = [(1, 0.0, 4.0, true), (0, 4.0, 10.0, true)];
        assert_eq!(line_carets("ab", &pens, 10.0, true), vec![10.0, 4.0, 0.0]);
    }

    #[test]
    fn missing_characters_come_from_fallback_fonts() {
        let fonts = FontLibrary::builtin_only();
        let mut layer = new_layer(1, "Hi \u{1F600}", BUILTIN_FAMILY.into(), [0.0, 0.0]);
        ensure_font(&mut layer, &fonts);
        ensure_fallbacks(&mut layer, &fonts);
        let families = |l: &Layer| -> Vec<String> {
            l.text
                .as_ref()
                .unwrap()
                .fallbacks
                .iter()
                .map(|f| f.family.clone())
                .collect()
        };
        assert_eq!(families(&layer), [EMOJI_FAMILY]);
        assert!(layer.text_state.missing.is_empty());

        // The emoji is drawn from the emoji font: wider than the .notdef box alone.
        let main = builtin_font();
        let emoji = FontRef::new(epaint_default_fonts::NOTO_EMOJI_REGULAR).unwrap();
        let t = layer.text.as_ref().unwrap();
        let alone = layout(t, &main);
        let with = layout_with(t, &[main.clone(), emoji]);
        assert_ne!(alone.size, with.size);
        assert_eq!(with.carets[0].xs.len(), 5, "4 characters");

        // A character no font has is reported; one no longer used drops its font.
        layer.text.as_mut().unwrap().content = "Hi \u{E000}".into();
        ensure_fallbacks(&mut layer, &fonts);
        assert!(families(&layer).is_empty());
        assert_eq!(layer.text_state.missing, ['\u{E000}']);
    }

    #[test]
    fn runs_split_by_font_and_keep_marks_together() {
        let main = builtin_font();
        let emoji = FontRef::new(epaint_default_fonts::NOTO_EMOJI_REGULAR).unwrap();
        let t = text("");
        let faces = [Face::new(&main, &t), Face::new(&emoji, &t)];
        // A thumbs up with a skin tone: the modifier stays with it.
        let line = "ab\u{1F44D}\u{1F3FD}c";
        assert_eq!(runs(line, &faces), [(0, 2, 0), (2, 10, 1), (10, 11, 0)]);
        // Nothing has it: the text's own font.
        assert_eq!(runs("\u{E000}", &faces), [(0, 3, 0)]);
    }

    #[test]
    fn auto_names() {
        assert_eq!(auto_name("  \nHello\nworld"), "Hello");
        assert!(auto_name(&"x".repeat(40)).ends_with("..."));
    }
}
