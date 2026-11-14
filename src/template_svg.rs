//! Template geometry in SVG: one `template.svg` per template, one group per element
//! (an Inkscape layer, its `id` named by the element in `template.json`).
//!
//! Files are read with `usvg`, which works out units, transforms, CSS and the basic
//! shapes (circles and rounded corners come as exact quarter-arc Beziers, read back as
//! arcs). Nothing is loaded from elsewhere: no images, no fonts.
//!
//! The SVG is written in mm (`width`/`height` in mm, one user unit per mm, y down) in the
//! template's frame. Items are in mm, y up, as `template_geom` has them.

use std::fmt::Write as _;
use std::path::Path;

use usvg::tiny_skia_path::PathSegment;

use crate::i18n::tr;
use crate::template_geom::{self, Item, Seg, V};

/// The file holding a template's geometry.
pub const SVG_FILE: &str = "template.svg";

/// mm per CSS pixel (SVG user unit at 96 per inch).
const MM_PER_PX: f64 = 25.4 / 96.0;
/// Largest file read: templates are a few shapes.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// The SVG at `path`, read safely (no images, fonts or anything from elsewhere).
fn parse(path: &Path) -> Result<usvg::Tree, String> {
    let cannot = |e: &dyn std::fmt::Display| tr!("CANNOT_READ_FILE", path = path.display(), e = e);
    let size = std::fs::metadata(path).map_err(|e| cannot(&e))?.len();
    if size > MAX_FILE_BYTES {
        return Err(tr!("SVG_TOO_LARGE", path = path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| cannot(&e))?;
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    usvg::Tree::from_data(&bytes, &options).map_err(|e| cannot(&e))
}

/// The group of id `id` among `g`'s descendants.
fn find<'a>(g: &'a usvg::Group, id: &str) -> Option<&'a usvg::Group> {
    g.children().iter().find_map(|c| match c {
        usvg::Node::Group(c) if c.id() == id => Some(&**c),
        usvg::Node::Group(c) => find(c, id),
        _ => None,
    })
}

/// Every path drawn inside `g`.
fn paths<'a>(g: &'a usvg::Group, out: &mut Vec<&'a usvg::Path>) {
    for c in g.children() {
        match c {
            usvg::Node::Group(g) => paths(g, out),
            usvg::Node::Path(p) => out.push(p),
            _ => {}
        }
    }
}

/// A path's outlines, in mm, y up: lines, arcs (Beziers that are arcs) and Beziers.
fn path_items(p: &usvg::Path) -> Vec<Item> {
    let ts = p.abs_transform();
    let at = |q: usvg::tiny_skia_path::Point| -> V {
        let (x, y) = (f64::from(q.x), f64::from(q.y));
        let px = [
            f64::from(ts.sx) * x + f64::from(ts.kx) * y + f64::from(ts.tx),
            f64::from(ts.ky) * x + f64::from(ts.sy) * y + f64::from(ts.ty),
        ];
        [px[0] * MM_PER_PX, -px[1] * MM_PER_PX]
    };
    let mut out = Vec::new();
    let mut open: Vec<Vec<Seg>> = Vec::new();
    let mut segs: Vec<Seg> = Vec::new();
    let (mut start, mut cur) = ([0.0; 2], [0.0; 2]);
    let mut flush = |segs: &mut Vec<Seg>, closed: bool, out: &mut Vec<Item>| {
        if !segs.is_empty() {
            let segs = std::mem::take(segs);
            if closed {
                out.push(Item::Outline { segs, closed });
            } else {
                open.push(segs);
            }
        }
    };
    for s in p.data().segments() {
        match s {
            PathSegment::MoveTo(q) => {
                flush(&mut segs, false, &mut out);
                start = at(q);
                cur = start;
            }
            PathSegment::LineTo(q) => {
                let q = at(q);
                segs.push(Seg::Line(cur, q));
                cur = q;
            }
            PathSegment::QuadTo(c, q) => {
                let (c, q) = (at(c), at(q));
                let lerp = |a: V, b: V| {
                    [
                        a[0] + (b[0] - a[0]) * 2.0 / 3.0,
                        a[1] + (b[1] - a[1]) * 2.0 / 3.0,
                    ]
                };
                segs.push(template_geom::cubic_seg([cur, lerp(cur, c), lerp(q, c), q]));
                cur = q;
            }
            PathSegment::CubicTo(c1, c2, q) => {
                let q = at(q);
                segs.push(template_geom::cubic_seg([cur, at(c1), at(c2), q]));
                cur = q;
            }
            PathSegment::Close => {
                let far = (cur[0] - start[0]).abs() > 1e-9 || (cur[1] - start[1]).abs() > 1e-9;
                if far {
                    segs.push(Seg::Line(cur, start));
                }
                flush(&mut segs, true, &mut out);
                cur = start;
            }
        }
    }
    flush(&mut segs, false, &mut out);
    out.extend(template_geom::join(open));
    out
}

/// What the group of id `id` of the SVG at `path` draws (the whole drawing for `None`),
/// in mm, y up, from the top-left corner of the SVG's canvas. A group that isn't there
/// draws nothing.
pub fn read(path: &Path, id: Option<&str>) -> Result<Vec<Item>, String> {
    let tree = parse(path)?;
    let group = match id {
        Some(id) => find(tree.root(), id),
        None => Some(tree.root()),
    };
    let mut found = Vec::new();
    if let Some(g) = group {
        paths(g, &mut found);
    }
    Ok(found.into_iter().flat_map(path_items).collect())
}

/// What the group of id `id` of the SVG at `path` draws, as `read` gives it, with the
/// `id` of the SVG shape each item comes from when it has one and draws that item alone
/// (the shapes variables link to, see `TemplateConfig::bindings`).
pub fn read_with_ids(path: &Path, id: Option<&str>) -> Result<Vec<(Option<String>, Item)>, String> {
    let tree = parse(path)?;
    let group = match id {
        Some(id) => find(tree.root(), id),
        None => Some(tree.root()),
    };
    let mut found = Vec::new();
    if let Some(g) = group {
        paths(g, &mut found);
    }
    let mut out = Vec::new();
    for p in found {
        let items = path_items(p);
        let id = (items.len() == 1 && !p.id().is_empty()).then(|| p.id().to_owned());
        out.extend(items.into_iter().map(|i| (id.clone(), i)));
    }
    Ok(out)
}

/// Whether the SVG at `path` has a group of id `id`.
pub fn has_group(path: &Path, id: &str) -> Result<bool, String> {
    Ok(find(parse(path)?.root(), id).is_some())
}

/// Every path drawn inside `g` but outside its groups of an id among `ids`.
fn paths_outside<'a>(g: &'a usvg::Group, ids: &[&str], out: &mut Vec<&'a usvg::Path>) {
    for c in g.children() {
        match c {
            usvg::Node::Group(g) if ids.contains(&g.id()) => {}
            usvg::Node::Group(g) => paths_outside(g, ids, out),
            usvg::Node::Path(p) => out.push(p),
            _ => {}
        }
    }
}

/// What the SVG at `path` draws outside the groups of id `ids` (the elements'): shapes
/// in no element, which a template doesn't use. In mm, y up, as `read` gives them.
pub fn read_orphans(path: &Path, ids: &[&str]) -> Result<Vec<Item>, String> {
    let tree = parse(path)?;
    let mut found = Vec::new();
    paths_outside(tree.root(), ids, &mut found);
    Ok(found.into_iter().flat_map(path_items).collect())
}

/// An element as written: its group's id, its name and line color, and its items (mm,
/// y up).
pub struct Element<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub color: [u8; 3],
    pub items: Vec<Item>,
    /// The `id` given to each item's SVG shape, if any, in the order of `items` (those
    /// past its end have none).
    pub ids: Vec<Option<String>>,
}

/// A number as written: up to 4 decimals, no trailing zeros.
fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Where items go on the SVG's canvas: y down, the top-left of the drawing at 0.
#[derive(Clone, Copy)]
struct Canvas {
    left: f64,
    top: f64,
}

impl Canvas {
    fn x(self, p: V) -> f64 {
        p[0] - self.left
    }

    fn y(self, p: V) -> f64 {
        self.top - p[1]
    }

    fn point(self, p: V) -> String {
        format!("{} {}", num(self.x(p)), num(self.y(p)))
    }
}

/// Text in an attribute.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The `d` of an outline.
fn path_data(segs: &[Seg], closed: bool, at: Canvas) -> String {
    let mut d = String::new();
    let Some(first) = segs.first() else {
        return d;
    };
    let _ = write!(d, "M {}", at.point(first.start()));
    let n = segs.len();
    for (i, s) in segs.iter().enumerate() {
        match *s {
            // The closing side is drawn by Z.
            Seg::Line(..) if closed && i + 1 == n => {}
            Seg::Line(_, b) => {
                let _ = write!(d, " L {}", at.point(b));
            }
            Seg::Arc { r, sweep, .. } => {
                // Y flips: counter-clockwise (y up) is sweep flag 0 on the canvas. Turns
                // of a half or more are split, an SVG arc not telling them apart.
                let parts = if sweep.abs() >= std::f64::consts::PI - 1e-9 {
                    2
                } else {
                    1
                };
                let flag = u8::from(sweep < 0.0);
                for k in 1..=parts {
                    let Seg::Arc { c, a0, .. } = *s else {
                        unreachable!()
                    };
                    let a = a0 + sweep * k as f64 / parts as f64;
                    let p = [c[0] + r * a.cos(), c[1] + r * a.sin()];
                    let _ = write!(d, " A {} {} 0 0 {flag} {}", num(r), num(r), at.point(p));
                }
            }
            Seg::Cubic([_, c1, c2, b]) => {
                let _ = write!(d, " C {} {} {}", at.point(c1), at.point(c2), at.point(b));
            }
        }
    }
    if closed {
        d.push_str(" Z");
    }
    d
}

/// The SVG of the elements, in mm, its canvas around all of them (their top-left corner
/// at 0, as `read` gives them back); the first element drawn last (on top), as the first
/// is at the top of the layer list.
pub fn to_svg(elements: &[Element]) -> String {
    let (lo, hi) = template_geom::items_bounds(elements.iter().flat_map(|e| &e.items))
        .unwrap_or(([0.0; 2], [0.0; 2]));
    let (w, h) = ((hi[0] - lo[0]).max(1e-3), (hi[1] - lo[1]).max(1e-3));
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
         xmlns:inkscape=\"http://www.inkscape.org/namespaces/inkscape\" \
         width=\"{w}mm\" height=\"{h}mm\" viewBox=\"0 0 {w} {h}\">",
        w = num(w),
        h = num(h),
    );
    let at = Canvas {
        left: lo[0],
        top: hi[1],
    };
    for e in elements.iter().rev() {
        let [r, g, b] = e.color;
        let _ = writeln!(
            s,
            "  <g id=\"{}\" inkscape:groupmode=\"layer\" inkscape:label=\"{}\" \
             fill=\"none\" stroke=\"#{r:02x}{g:02x}{b:02x}\" stroke-width=\"0.3\">",
            escape(e.id),
            escape(e.name),
        );
        for (k, item) in e.items.iter().enumerate() {
            let id = match e.ids.get(k) {
                Some(Some(id)) => format!(" id=\"{}\"", escape(id)),
                _ => String::new(),
            };
            match item {
                Item::Circle { c, r } => {
                    let _ = writeln!(
                        s,
                        "    <circle{id} cx=\"{}\" cy=\"{}\" r=\"{}\"/>",
                        num(at.x(*c)),
                        num(at.y(*c)),
                        num(*r)
                    );
                }
                Item::Outline { segs, closed } => {
                    let d = path_data(segs, *closed, at);
                    if !d.is_empty() {
                        let _ = writeln!(s, "    <path{id} d=\"{d}\"/>");
                    }
                }
                Item::Point(_) => {}
            }
        }
        s.push_str("  </g>\n");
    }
    s.push_str("</svg>\n");
    s
}

/// Writes the elements as the SVG at `path`.
pub fn write(path: &Path, elements: &[Element]) -> Result<(), String> {
    std::fs::write(path, to_svg(elements))
        .map_err(|e| tr!("CANNOT_WRITE_FILE", path = path.display(), e = e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test.
    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("artwork-svg-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// What the elements read back as, and how far the writing moved them (the top-left
    /// of the drawing goes to 0).
    fn round_trip(name: &str, elements: &[Element]) -> (Vec<Vec<Item>>, V) {
        let (lo, hi) = template_geom::items_bounds(elements.iter().flat_map(|e| &e.items)).unwrap();
        let dir = folder(name);
        let path = dir.join(SVG_FILE);
        write(&path, elements).unwrap();
        let read = elements
            .iter()
            .map(|e| read(&path, Some(e.id)).unwrap())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        (read, [-lo[0], -hi[1]])
    }

    fn moved(p: V, by: V) -> V {
        [p[0] + by[0], p[1] + by[1]]
    }

    fn close(a: V, b: V) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn circles_and_arcs_come_back_as_written() {
        let quarter = Seg::Arc {
            c: [50.0, -20.0],
            r: 10.0,
            a0: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        };
        let back = Seg::Arc {
            c: [50.0, -20.0],
            r: 10.0,
            a0: std::f64::consts::FRAC_PI_2,
            sweep: -std::f64::consts::PI,
        };
        let elements = [
            Element {
                id: "buttons",
                name: "Action <buttons>",
                color: [0, 170, 255],
                items: vec![Item::Circle {
                    c: [10.0, -12.0],
                    r: 12.0,
                }],
                ids: vec![Some("buttons-1".into())],
            },
            Element {
                id: "outline",
                name: "Outline",
                color: [255, 0, 255],
                items: vec![Item::Outline {
                    segs: vec![quarter, back],
                    closed: false,
                }],
                ids: Vec::new(),
            },
        ];
        let (read, by) = round_trip("arcs", &elements);
        // The circle: four quarter arcs of its center and radius, around once.
        let [Item::Outline { segs, closed: true }] = &read[0][..] else {
            panic!("{:?}", read[0]);
        };
        assert_eq!(segs.len(), 4);
        for s in segs {
            let Seg::Arc { c, r, .. } = s else {
                panic!("{s:?}")
            };
            assert!(
                close(*c, moved([10.0, -12.0], by)) && (r - 12.0).abs() < 1e-3,
                "{c:?} {r}"
            );
        }
        // The open outline: arcs again, the half turn in two.
        let [
            Item::Outline {
                segs,
                closed: false,
            },
        ] = &read[1][..]
        else {
            panic!("{:?}", read[1]);
        };
        assert!(close(segs[0].start(), moved(quarter.start(), by)));
        assert!(close(segs[segs.len() - 1].end(), moved(back.end(), by)));
        let turn: f64 = segs
            .iter()
            .map(|s| match s {
                Seg::Arc { sweep, .. } => *sweep,
                _ => panic!("{s:?}"),
            })
            .sum();
        assert!((turn + std::f64::consts::FRAC_PI_2).abs() < 1e-3);
    }

    #[test]
    fn shape_ids_come_back() {
        let elements = [Element {
            id: "buttons",
            name: "Buttons",
            color: [0; 3],
            items: vec![
                Item::Circle {
                    c: [0.0, 0.0],
                    r: 5.0,
                },
                Item::Circle {
                    c: [20.0, 0.0],
                    r: 5.0,
                },
            ],
            ids: vec![None, Some("buttons-2".into())],
        }];
        let dir = folder("ids");
        let path = dir.join(SVG_FILE);
        write(&path, &elements).unwrap();
        let read = read_with_ids(&path, Some("buttons")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let ids: Vec<Option<&str>> = read.iter().map(|(id, _)| id.as_deref()).collect();
        assert_eq!(ids, [None, Some("buttons-2")]);
    }

    #[test]
    fn closed_outlines_keep_their_sides() {
        let p = [[0.0, 0.0], [30.0, 0.0], [30.0, -20.0], [0.0, -20.0]];
        let segs: Vec<Seg> = (0..4).map(|i| Seg::Line(p[i], p[(i + 1) % 4])).collect();
        let (read, by) = round_trip(
            "sides",
            &[Element {
                id: "outline",
                name: "Outline",
                color: [0; 3],
                items: vec![Item::Outline {
                    segs: segs.clone(),
                    closed: true,
                }],
                ids: Vec::new(),
            }],
        );
        let [
            Item::Outline {
                segs: back,
                closed: true,
            },
        ] = &read[0][..]
        else {
            panic!("{:?}", read[0]);
        };
        assert_eq!(back.len(), 4);
        for (a, b) in segs.iter().zip(back) {
            assert!(close(moved(a.start(), by), b.start()) && close(moved(a.end(), by), b.end()));
        }
    }

    #[test]
    fn inkscape_units_and_transforms_are_worked_out() {
        let dir = folder("units");
        let path = dir.join(SVG_FILE);
        // 1 user unit is 2 mm; the group is moved by 5 units.
        std::fs::write(
            &path,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm" viewBox="0 0 100 50">
                 <g id="layer1" transform="translate(5 0)"><g><circle cx="10" cy="10" r="4" fill="red"/></g></g>
               </svg>"#,
        )
        .unwrap();
        let items = read(&path, Some("layer1")).unwrap();
        let [Item::Outline { segs, closed: true }] = &items[..] else {
            panic!("{items:?}");
        };
        let Seg::Arc { c, r, .. } = segs[0] else {
            panic!("{:?}", segs[0])
        };
        assert!(
            close(c, [30.0, -20.0]) && (r - 8.0).abs() < 1e-3,
            "{c:?} {r}"
        );
        assert!(read(&path, Some("missing")).unwrap().is_empty());
        assert!(read_orphans(&path, &["layer1"]).unwrap().is_empty());
        assert_eq!(read_orphans(&path, &["other"]).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
