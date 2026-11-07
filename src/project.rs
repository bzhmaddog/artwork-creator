//! Project files, in two formats:
//! - `.artwork`, the default: a zip archive holding `project.json` and every image and
//!   font the project uses, so it can be moved, backed up or shared on its own;
//! - `.json`, linked: the project alone, pointing to image files on disk.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::document::{Document, Layer};
use crate::i18n::tr;

/// Extension of project archives.
pub const EXTENSION: &str = "artwork";
/// The project description inside an archive.
const JSON_ENTRY: &str = "project.json";
/// Folder of the images inside an archive.
const IMAGE_DIR: &str = "images/";
/// Folder of the text layers' fonts inside an archive.
const FONT_DIR: &str = "fonts/";

/// The file is a project archive (as opposed to a linked JSON project).
pub fn is_archive(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// Loads a project archive or a linked JSON project. Images from an archive come back
/// in each layer's `data`, with `source` set to their name inside the archive; images
/// of JSON projects are read from disk later.
pub fn load(path: &Path) -> Result<Document, String> {
    let bytes =
        std::fs::read(path).map_err(|e| tr!("CANNOT_READ_FILE", path = path.display(), e = e))?;
    if !bytes.starts_with(b"PK\x03\x04") {
        return Document::load(path);
    }
    let invalid =
        |e: &dyn std::fmt::Display| tr!("INVALID_PROJECT_FILE_AT", path = path.display(), e = e);
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| invalid(&e))?;
    let json = read_entry(&mut zip, JSON_ENTRY).map_err(|e| invalid(&e))?;
    let text = String::from_utf8(json).map_err(|e| invalid(&e))?;
    let mut doc = Document::from_json(&text)?;

    // Each image is read once, even when several layers use it.
    let mut images: HashMap<String, Option<Arc<Vec<u8>>>> = HashMap::new();
    doc.for_each_layer_mut(|layer| {
        let name = layer.source.to_string_lossy().replace('\\', "/");
        if !name.starts_with(IMAGE_DIR) && !name.starts_with(FONT_DIR) {
            return;
        }
        layer.data = images
            .entry(name)
            .or_insert_with_key(|name| read_entry(&mut zip, name).ok().map(Arc::new))
            .clone();
    });
    Ok(doc)
}

fn read_entry(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Result<Vec<u8>, String> {
    let mut entry = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut out = Vec::new();
    entry
        .read_to_end(&mut out)
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(out)
}

/// Saves the project as an archive with all its images and fonts. Returns the names of
/// layers whose image could not be included because it never loaded (missing file);
/// they keep their link to the file on disk. Text layers whose font isn't loaded, or
/// whose license forbids embedding it, keep their link to the font file and will look
/// for their font family where the project is opened. Writes to a temporary file first, so a failed save
/// leaves an existing project untouched.
pub fn save(doc: &Document, path: &Path) -> Result<Vec<String>, String> {
    let mut doc = doc.clone();
    let mut images: Vec<(String, Arc<Vec<u8>>)> = Vec::new();
    let mut missing = Vec::new();
    doc.for_each_layer_mut(|layer| {
        if !font_to_save(layer) {
            return;
        }
        match &layer.data {
            Some(data) => {
                let name = entry_name(data, &layer.source, layer.text.is_some());
                if !images.iter().any(|(n, _)| *n == name) {
                    images.push((name.clone(), data.clone()));
                }
                layer.source = PathBuf::from(name);
            }
            None => missing.push(layer.name.clone()),
        }
    });
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    write_safely(path, |tmp| write_archive(tmp, &json, &images))?;
    Ok(missing)
}

/// Saves the project as linked JSON. Images it only has in memory (from an archive)
/// are written to an "<name> images" folder next to the project and linked from there.
/// Returns the names of layers whose image is neither on disk nor in memory.
pub fn save_json(doc: &Document, path: &Path) -> Result<Vec<String>, String> {
    let base = path.parent().unwrap_or(Path::new(""));
    let folder = base.join(format!("{} images", crate::recent::label(path)));
    let mut doc = doc.clone();
    let mut missing = Vec::new();
    let mut error = None;
    doc.for_each_layer_mut(|layer| {
        if !font_to_save(layer) || (layer.source.is_absolute() && layer.source.is_file()) {
            return;
        }
        let Some(data) = &layer.data else {
            missing.push(layer.name.clone());
            return;
        };
        let name = entry_name(data, &layer.source, layer.text.is_some());
        let file = folder.join(
            name.trim_start_matches(IMAGE_DIR)
                .trim_start_matches(FONT_DIR),
        );
        // Identical images share a file, so an existing one is already right.
        let written = file.is_file()
            || std::fs::create_dir_all(&folder)
                .and_then(|()| std::fs::write(&file, data.as_slice()))
                .map_err(|e| {
                    error.get_or_insert(tr!("CANNOT_WRITE_FILE", path = file.display(), e = e))
                })
                .is_ok();
        if written {
            layer.source = file;
        }
    });
    if let Some(e) = error {
        return Err(e);
    }
    let json = doc.to_json_relative(base)?;
    write_safely(path, |tmp| {
        std::fs::write(tmp, &json).map_err(|e| e.to_string())
    })?;
    Ok(missing)
}

/// Writes through a temporary file renamed over `path`, so a failed save leaves an
/// existing file untouched.
fn write_safely(
    path: &Path,
    write: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = write(&tmp).and_then(|()| std::fs::rename(&tmp, path).map_err(|e| e.to_string()));
    if let Err(e) = result {
        std::fs::remove_file(&tmp).ok();
        return Err(tr!("CANNOT_WRITE_FILE", path = path.display(), e = e));
    }
    Ok(())
}

fn write_archive(path: &Path, json: &str, images: &[(String, Arc<Vec<u8>>)]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(std::io::BufWriter::new(file));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // Images are already compressed: storing them as they are is faster and as small.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(images.iter().any(|(_, d)| d.len() >= u32::MAX as usize));
    zip.start_file(JSON_ENTRY, deflated)
        .map_err(|e| e.to_string())?;
    zip.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    for (name, data) in images {
        zip.start_file(name, stored).map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())?;
    }
    let mut out = zip.finish().map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    let file = out.into_inner().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

/// Name of an image inside the archive: identical images share one entry. The extension
/// comes from the image data, or else from the original file name.
/// False for a text layer whose font file is not saved with the project: not loaded, the
/// built-in font, or one whose license forbids embedding it. Its `source` is cleared
/// unless it is a file on disk; the font is looked up by family where the project opens.
fn font_to_save(layer: &mut Layer) -> bool {
    let Some(text) = &layer.text else {
        return true;
    };
    let save = layer.data.as_ref().is_some_and(|d| {
        text.family != crate::text::BUILTIN_FAMILY && crate::text::embeddable(d, text.font_index)
    });
    if !save && !layer.source.is_absolute() {
        layer.source = PathBuf::new();
    }
    save
}

fn entry_name(data: &[u8], source: &Path, font: bool) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut h);
    if font {
        let ext = match data.get(..4) {
            Some(b"OTTO") => "otf",
            Some(b"ttcf") => "ttc",
            _ => "ttf",
        };
        return format!("{FONT_DIR}{:016x}.{ext}", h.finish());
    }
    let ext = image::guess_format(data)
        .ok()
        .and_then(|f| f.extensions_str().first().copied())
        .map(str::to_owned)
        .or_else(|| {
            source
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
        })
        .unwrap_or_else(|| "bin".into());
    format!("{IMAGE_DIR}{:016x}.{ext}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Node;

    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(4, 3, image::Rgb(rgb));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn layer(id: u64, name: &str, data: Option<&Arc<Vec<u8>>>) -> Node {
        let mut l = Layer::new(id, name.into(), format!("/elsewhere/{name}.png").into());
        l.data = data.cloned();
        Node::Layer(Box::new(l))
    }

    #[test]
    fn archive_holds_text_fonts_but_not_the_built_in_one() {
        use crate::text::{self, BUILTIN_FAMILY};
        let dir = std::env::temp_dir().join(format!("artwork-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hack = Arc::new(epaint_default_fonts::HACK_REGULAR.to_vec());
        let mut doc = Document::default();
        let mut a = text::new_layer(1, "Hello", "Hack".into(), [50.0, 50.0]);
        a.data = Some(hack.clone());
        a.source = "/usr/share/fonts/Hack-Regular.ttf".into();
        let mut b = text::new_layer(2, "Built in", BUILTIN_FAMILY.into(), [50.0, 80.0]);
        b.data = Some(Arc::new(epaint_default_fonts::UBUNTU_LIGHT.to_vec()));
        doc.layers.push(Node::Layer(Box::new(a)));
        doc.layers.push(Node::Layer(Box::new(b)));

        let path = dir.join("t.artwork");
        assert!(
            save(&doc, &path).unwrap().is_empty(),
            "nothing reported missing"
        );
        let back = load(&path).unwrap();
        let a = back.layer(1).unwrap();
        assert_eq!(a.data.as_deref(), Some(hack.as_ref()), "font read back");
        assert!(a.source.to_string_lossy().starts_with(FONT_DIR));
        assert_eq!(a.text.as_ref().unwrap().content, "Hello");
        let b = back.layer(2).unwrap();
        assert!(
            b.data.is_none() && b.source.as_os_str().is_empty(),
            "not stored"
        );

        // A linked project writes the archive's font next to it.
        let json = dir.join("t.json");
        save_json(&back, &json).unwrap();
        let linked = Document::load(&json).unwrap();
        let src = &linked.layer(1).unwrap().source;
        assert!(src.is_file() && src.extension().is_some_and(|e| e == "ttf"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn archive_holds_the_images_and_opens_on_its_own() {
        let dir = std::env::temp_dir().join(format!("artwork-archive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = Arc::new(png([255, 0, 0]));
        let blue = Arc::new(png([0, 0, 255]));
        let doc = Document {
            // Two layers share the red image; one image is missing.
            layers: vec![
                layer(1, "red", Some(&red)),
                layer(2, "red copy", Some(&red)),
                layer(3, "blue", Some(&blue)),
                layer(4, "lost", None),
            ],
            next_id: 5,
            ..Default::default()
        };
        let path = dir.join("p.artwork");
        let missing = save(&doc, &path).unwrap();
        assert_eq!(missing, ["lost"]);
        assert!(!dir.join("p.artwork.tmp").exists());

        let mut zip = ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        assert_eq!(names.len(), 3, "project.json and two images: {names:?}");
        assert!(
            names
                .iter()
                .all(|n| n == JSON_ENTRY || (n.starts_with(IMAGE_DIR) && n.ends_with(".png")))
        );
        assert!(zip.by_name(JSON_ENTRY).is_ok());

        let back = load(&path).unwrap();
        let data = |id| back.layer(id).unwrap().data.clone();
        assert_eq!(data(1).as_deref(), Some(&*red));
        assert!(
            Arc::ptr_eq(&data(1).unwrap(), &data(2).unwrap()),
            "read once"
        );
        assert_eq!(data(3).as_deref(), Some(&*blue));
        assert!(data(4).is_none());
        assert_eq!(
            back.layer(4).unwrap().source,
            Path::new("/elsewhere/lost.png")
        );
        assert_eq!(back.next_id, 5);

        // Saving again over the same file works too.
        save(&back, &path).unwrap();
        assert_eq!(
            load(&path).unwrap().layer(3).unwrap().data.as_deref(),
            Some(&*blue)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn archive_project_saved_as_linked_json_writes_its_images_out() {
        let dir = std::env::temp_dir().join(format!("artwork-tojson-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let red = Arc::new(png([255, 0, 0]));
        let on_disk = dir.join("disk.png");
        std::fs::write(&on_disk, png([0, 255, 0])).unwrap();
        let mut linked = Layer::new(3, "disk".into(), on_disk.clone());
        linked.data = Some(Arc::new(std::fs::read(&on_disk).unwrap()));
        let doc = Document {
            layers: vec![
                // As loaded from an archive: the source is a name inside it.
                {
                    let mut l = Layer::new(1, "red".into(), "images/abc.png".into());
                    l.data = Some(red.clone());
                    Node::Layer(Box::new(l))
                },
                Node::Layer(Box::new(linked)),
                layer(4, "lost", None),
            ],
            next_id: 5,
            ..Default::default()
        };
        let path = dir.join("My stick.json");
        let missing = save_json(&doc, &path).unwrap();
        assert_eq!(missing, ["lost"]);

        let back = load(&path).unwrap();
        let red_file = &back.layer(1).unwrap().source;
        assert!(
            red_file.starts_with(dir.join("My stick images")),
            "{red_file:?}"
        );
        assert_eq!(std::fs::read(red_file).unwrap(), *red);
        assert_eq!(
            back.layer(3).unwrap().source,
            on_disk,
            "files on disk stay linked"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(dir.to_str().unwrap()), "links are relative");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn older_json_projects_still_open() {
        let dir = std::env::temp_dir().join(format!("artwork-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.artwork.json");
        std::fs::write(
            &path,
            r#"{"layers":[{"id":1,"name":"a","source":"img/a.png","size_px":[4,3],
                "visible":true,"locked":false,"opacity":1.0,
                "transform":{"center":[0,0],"scale":[1,1],"rotation_deg":0,
                "flip_x":false,"flip_y":false}}]}"#,
        )
        .unwrap();
        assert!(!is_archive(&path));
        let doc = load(&path).unwrap();
        let l = doc.layer(1).unwrap();
        assert_eq!(l.source, dir.join("img/a.png"), "relative to the project");
        assert!(l.data.is_none(), "read from disk later");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn damaged_archive_is_reported() {
        let dir = std::env::temp_dir().join(format!("artwork-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.artwork");
        std::fs::write(&path, b"PK\x03\x04 not really a zip").unwrap();
        let Err(err) = load(&path) else {
            panic!("a damaged archive should not load");
        };
        assert!(err.contains("Invalid project file"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entry_names_follow_the_image_format() {
        let p = png([1, 2, 3]);
        let name = entry_name(&p, Path::new("photo.JPG"), false);
        assert!(
            name.starts_with(IMAGE_DIR) && name.ends_with(".png"),
            "{name}"
        );
        assert_eq!(
            name,
            entry_name(&p, Path::new("other.png"), false),
            "same data, same entry"
        );
        assert!(entry_name(b"???", Path::new("x.WEBP"), false).ends_with(".webp"));
    }
}
