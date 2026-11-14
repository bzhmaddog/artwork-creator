//! SVG import: an SVG file's shapes as shape layers, with a group for each of its own
//! (`<g>` with an id or an opacity).
//!
//! Only outlines and how they are painted come in: fill and stroke colors and opacities,
//! linear and radial gradients, stroke width, caps, joins and dashes, fill rules. The
//! file is read with `usvg`, which works out CSS, transforms, units, `<use>` and the
//! basic shapes as paths; what is not a shape is left out, and nothing is ever loaded
//! from elsewhere: no `<image>` (nor `data:` URLs), no fonts (so no text), no scripts,
//! `<foreignObject>`, links or event handlers, which usvg never keeps. Filters, masks,
//! clipping and patterns are left out too. The user is told what was.
//!
//! The SVG keeps its true size (its units, `px` at 96 per inch), shrunk to fit the page
//! if it is larger, and is centered where asked.

use std::collections::BTreeSet;
use std::path::Path as FsPath;

use usvg::tiny_skia_path::{PathSegment, Rect, Transform};

use crate::document::{Group, Layer, Node, P2};
use crate::gradient::{Gradient, GradientKind, MAX_STOPS, Stop};
use crate::i18n::{tr, translate};
use crate::shape::{self, Cap, Contour, DashStyle, Join, Shape, ShapeKind};

/// CSS pixels (SVG user units) per mm.
const PX_PER_MM: f32 = 96.0 / 25.4;
/// Largest file read: SVGs larger than this are not drawings to edit.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Most XML nodes parsed, against files that swell as they are read.
const MAX_XML_NODES: u32 = 1_000_000;
/// Most shapes imported: more would make the layer list and the canvas crawl.
pub const MAX_SHAPES: usize = 5000;

/// Elements that are not shapes, named in the notice saying they were left out.
const LEFT_OUT: &[&str] = &[
    "text",
    "image",
    "foreignObject",
    "script",
    "filter",
    "mask",
    "clipPath",
    "pattern",
    "video",
    "audio",
    "iframe",
];

/// What an import made.
pub struct Imported {
    /// The shapes and groups, bottom first.
    pub nodes: Vec<Node>,
    /// Notices for the user: what was left out.
    pub notices: Vec<String>,
}

/// Reads the SVG file at `path` as shape layers and groups, fitted to a page of
/// `page_mm` and centered on `center`; `new_id` gives each layer and group its id.
pub fn import(
    path: &FsPath,
    page_mm: P2,
    center: P2,
    new_id: &mut dyn FnMut() -> u64,
) -> Result<Imported, String> {
    let cannot = |e: &dyn std::fmt::Display| tr!("CANNOT_OPEN_FILE", path = path.display(), e = e);
    let size = std::fs::metadata(path).map_err(|e| cannot(&e))?.len();
    if size > MAX_FILE_BYTES {
        return Err(tr!("SVG_TOO_LARGE", path = path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| cannot(&e))?;
    let mut imported = import_bytes(&bytes, page_mm, center, new_id).map_err(|e| cannot(&e))?;
    if imported.nodes.is_empty() {
        return Err(tr!("SVG_HAS_NO_SHAPES", path = path.display()));
    }
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    imported.nodes = one_group(std::mem::take(&mut imported.nodes), name, new_id);
    for n in &mut imported.notices {
        *n = format!("{}: {n}", path.display());
    }
    Ok(imported)
}

/// Several shapes and groups with a group among them, all in one group named `name` (the
/// file's), so that the project gets one group rather than many; others as they are.
fn one_group(nodes: Vec<Node>, name: String, new_id: &mut dyn FnMut() -> u64) -> Vec<Node> {
    if nodes.len() < 2 || !nodes.iter().any(|n| matches!(n, Node::Group(_))) {
        return nodes;
    }
    vec![Node::Group(Group {
        id: new_id(),
        name,
        visible: true,
        locked: false,
        opacity: 1.0,
        expanded: false,
        children: nodes,
        element: None,
    })]
}

/// [`import`] from the file's contents.
pub fn import_bytes(
    bytes: &[u8],
    page_mm: P2,
    center: P2,
    new_id: &mut dyn FnMut() -> u64,
) -> Result<Imported, String> {
    let unzipped;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        unzipped = usvg::decompress_svgz(bytes).map_err(|e| e.to_string())?;
        &unzipped[..]
    } else {
        bytes
    };
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let xml = usvg::roxmltree::Document::parse_with_options(
        text,
        usvg::roxmltree::ParsingOptions {
            // Entities are expanded by roxmltree within its own limits; external ones
            // are never loaded (no entity resolver).
            allow_dtd: true,
            nodes_limit: MAX_XML_NODES,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let mut left_out: BTreeSet<&'static str> = xml
        .descendants()
        .filter(|n| n.is_element())
        .filter_map(|n| {
            LEFT_OUT
                .iter()
                .find(|&&k| k == n.tag_name().name())
                .copied()
        })
        .collect();
    let options = usvg::Options {
        // Nothing is loaded from files, URLs or data: images are left out.
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_xmltree(&xml, &options).map_err(|e| e.to_string())?;

    // The SVG's canvas, in mm, shrunk to fit the page, centered on `center`.
    let size = [tree.size().width(), tree.size().height()].map(|v| v / PX_PER_MM);
    let k = (page_mm[0] / size[0].max(1e-3))
        .min(page_mm[1] / size[1].max(1e-3))
        .min(1.0);
    let s = k / PX_PER_MM;
    let map = Transform::from_row(
        s,
        0.0,
        0.0,
        s,
        center[0] - size[0] * k / 2.0,
        center[1] - size[1] * k / 2.0,
    );
    let mut walk = Walk {
        new_id,
        map,
        shapes: 0,
        groups: 0,
        left_out: BTreeSet::new(),
    };
    let nodes = walk.group(tree.root());
    left_out.extend(walk.left_out);
    let mut notices = Vec::new();
    if walk.shapes >= MAX_SHAPES {
        notices.push(tr!("SVG_TOO_MANY_SHAPES", count = MAX_SHAPES));
    }
    if !left_out.is_empty() {
        let what = left_out.into_iter().collect::<Vec<_>>().join(", ");
        notices.push(tr!("SVG_LEFT_OUT", what = what));
    }
    Ok(Imported { nodes, notices })
}

/// Going through the SVG's tree.
struct Walk<'a> {
    new_id: &'a mut dyn FnMut() -> u64,
    /// From the SVG's canvas (px) to the page (mm).
    map: Transform,
    /// Shapes made so far.
    shapes: usize,
    /// Groups named so far.
    groups: usize,
    /// Kinds of elements left out, beyond those found in the XML.
    left_out: BTreeSet<&'static str>,
}

impl Walk<'_> {
    /// A group's children as layers and groups, bottom first as in the SVG.
    fn group(&mut self, g: &usvg::Group) -> Vec<Node> {
        let mut out = Vec::new();
        for child in g.children() {
            if self.shapes >= MAX_SHAPES {
                break;
            }
            match child {
                usvg::Node::Group(g) => {
                    if g.clip_path().is_some() {
                        self.left_out.insert("clipPath");
                    }
                    if g.mask().is_some() {
                        self.left_out.insert("mask");
                    }
                    if !g.filters().is_empty() {
                        self.left_out.insert("filter");
                    }
                    let children = self.group(g);
                    if children.is_empty() {
                        continue;
                    }
                    let opacity = g.opacity().get();
                    // Groups usvg makes for its own sake (no id, opaque) are let go.
                    if g.id().is_empty() && opacity >= 1.0 {
                        out.extend(children);
                        continue;
                    }
                    self.groups += 1;
                    out.push(Node::Group(Group {
                        id: (self.new_id)(),
                        name: if g.id().is_empty() {
                            tr!("DEFAULT_GROUP_NAME", n = self.groups)
                        } else {
                            g.id().to_owned()
                        },
                        visible: true,
                        locked: false,
                        opacity,
                        expanded: false,
                        children,
                        element: None,
                    }));
                }
                usvg::Node::Path(p) => {
                    if p.is_visible()
                        && let Some(layer) = self.path(p)
                    {
                        self.shapes += 1;
                        out.push(Node::Layer(Box::new(layer)));
                    }
                }
                usvg::Node::Image(_) => {
                    self.left_out.insert("image");
                }
                usvg::Node::Text(_) => {
                    self.left_out.insert("text");
                }
            }
        }
        out
    }

    /// A path as a shape layer; `None` if it draws nothing.
    fn path(&mut self, p: &usvg::Path) -> Option<Layer> {
        let ts = p.abs_transform().post_concat(self.map);
        let data = p.data().clone().transform(ts)?;
        let b = data.bounds();
        // The shape's frame is centered on its geometry.
        let at = [(b.left() + b.right()) / 2.0, (b.top() + b.bottom()) / 2.0];
        let data = data.transform(Transform::from_translate(-at[0], -at[1]))?;
        let to_frame = ts.post_translate(-at[0], -at[1]);
        let (points, handles, contours) = contours(&data);
        if contours.is_empty() {
            return None;
        }

        let fill = p
            .fill()
            .filter(|f| !matches!(f.paint(), usvg::Paint::Pattern(_)));
        let stroke = p
            .stroke()
            .filter(|s| !matches!(s.paint(), usvg::Paint::Pattern(_)));
        let patterned =
            |paint: Option<&usvg::Paint>| matches!(paint, Some(usvg::Paint::Pattern(_)));
        if patterned(p.fill().map(|f| f.paint())) || patterned(p.stroke().map(|s| s.paint())) {
            self.left_out.insert("pattern");
        }
        if fill.is_none() && stroke.is_none() {
            return None;
        }
        let even_odd = fill.is_some_and(|f| f.rule() == usvg::FillRule::EvenOdd);

        // The simplest kind that draws it the same.
        let straight = handles.iter().flatten().all(|h| *h == [0.0; 2]);
        let kind = match contours[..] {
            [c] if c.closed && c.len >= 3 && (!even_odd || fill.is_none()) => ShapeKind::Polygon,
            [c] if !c.closed && fill.is_none() => {
                if c.len == 2 && straight {
                    ShapeKind::Line
                } else {
                    ShapeKind::Polyline
                }
            }
            _ => ShapeKind::Path,
        };
        let mut shape = Shape::new(kind);
        shape.points = points;
        shape.handles = if straight { Vec::new() } else { handles };
        shape.contours = if kind == ShapeKind::Path {
            contours
        } else {
            Vec::new()
        };
        shape.even_odd = even_odd && kind == ShapeKind::Path;

        shape.fill = fill.is_some();
        if let Some(f) = fill {
            shape.fill_color = color(f.paint(), f.opacity().get());
        }
        shape.stroke = stroke.is_some();
        if let Some(s) = stroke {
            // Lengths across the stroke grow as the transform's area does.
            let scale = (ts.sx * ts.sy - ts.kx * ts.ky).abs().sqrt();
            shape.stroke_width = s.width().get() * scale;
            shape.stroke_color = color(s.paint(), s.opacity().get());
            shape.cap = match s.linecap() {
                usvg::LineCap::Butt => Cap::Butt,
                usvg::LineCap::Round => Cap::Round,
                usvg::LineCap::Square => Cap::Square,
            };
            shape.join = match s.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => Join::Miter,
                usvg::LineJoin::Round => Join::Round,
                usvg::LineJoin::Bevel => Join::Bevel,
            };
            shape.dash = dash(s.dasharray(), s.width().get());
        }
        // Gradients span the shape's box, stroke included: known now.
        if let Some((lo, hi)) = shape.bounds()
            && let Some(frame) = Rect::from_ltrb(lo[0], lo[1], hi[0], hi[1])
        {
            if let Some(f) = fill {
                shape.fill_gradient = gradient(f.paint(), f.opacity().get(), to_frame, frame);
            }
            if let Some(s) = stroke {
                shape.stroke_gradient = gradient(s.paint(), s.opacity().get(), to_frame, frame);
            }
        }

        let name = if p.id().is_empty() {
            translate(kind.name_key()).to_owned()
        } else {
            p.id().to_owned()
        };
        Some(shape::new_shape_layer((self.new_id)(), name, shape, at))
    }
}

/// A path's points, their Bezier handles and its contours, as a path shape keeps them.
/// A closed contour ending where it starts loses its last point, the curve coming in
/// going to its first. Contours of a single point are left out.
fn contours(path: &usvg::tiny_skia_path::Path) -> (Vec<P2>, Vec<[P2; 2]>, Vec<Contour>) {
    let mut points: Vec<P2> = Vec::new();
    let mut handles: Vec<[P2; 2]> = Vec::new();
    let mut contours: Vec<Contour> = Vec::new();
    // Where the contour going on starts in `points`, if one is.
    let mut start: Option<usize> = None;
    // Where the pen is after a contour closes: back at its start.
    let mut pen: Option<P2> = None;
    let finish = |points: &mut Vec<P2>,
                  handles: &mut Vec<[P2; 2]>,
                  contours: &mut Vec<Contour>,
                  start: &mut Option<usize>,
                  closed: bool| {
        let Some(s) = start.take() else { return };
        let n = points.len();
        if closed && n - s >= 3 {
            let (first, last) = (points[s], points[n - 1]);
            if (first[0] - last[0]).abs() < 1e-4 && (first[1] - last[1]).abs() < 1e-4 {
                handles[s][0] = handles[n - 1][0];
                points.pop();
                handles.pop();
            }
        }
        let len = points.len() - s;
        if len < 2 {
            points.truncate(s);
            handles.truncate(s);
            return;
        }
        contours.push(Contour { len, closed });
    };
    let off = |p: P2, from: P2| [p[0] - from[0], p[1] - from[1]];
    for seg in path.segments() {
        // A segment after a closed contour starts another from where it ended.
        if !matches!(seg, PathSegment::MoveTo(_) | PathSegment::Close)
            && start.is_none()
            && let Some(at) = pen
        {
            start = Some(points.len());
            points.push(at);
            handles.push([[0.0; 2]; 2]);
        }
        match seg {
            PathSegment::MoveTo(p) => {
                finish(&mut points, &mut handles, &mut contours, &mut start, false);
                start = Some(points.len());
                pen = Some([p.x, p.y]);
                points.push([p.x, p.y]);
                handles.push([[0.0; 2]; 2]);
            }
            PathSegment::LineTo(p) => {
                points.push([p.x, p.y]);
                handles.push([[0.0; 2]; 2]);
            }
            PathSegment::QuadTo(c, p) => {
                // The same curve as a cubic one: control points two thirds of the way to
                // the quad's.
                let Some(&a) = points.last() else { continue };
                let (c, e) = ([c.x, c.y], [p.x, p.y]);
                let toward = |from: P2| -> P2 {
                    [(c[0] - from[0]) * 2.0 / 3.0, (c[1] - from[1]) * 2.0 / 3.0]
                };
                if let Some(h) = handles.last_mut() {
                    h[1] = toward(a);
                }
                points.push(e);
                handles.push([toward(e), [0.0; 2]]);
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let Some(&a) = points.last() else { continue };
                let e = [p.x, p.y];
                if let Some(h) = handles.last_mut() {
                    h[1] = off([c1.x, c1.y], a);
                }
                points.push(e);
                handles.push([off([c2.x, c2.y], e), [0.0; 2]]);
            }
            PathSegment::Close => {
                finish(&mut points, &mut handles, &mut contours, &mut start, true);
            }
        }
    }
    finish(&mut points, &mut handles, &mut contours, &mut start, false);
    (points, handles, contours)
}

/// A paint's color, straight RGBA, `opacity` included; a gradient's first color.
fn color(paint: &usvg::Paint, opacity: f32) -> [u8; 4] {
    let first = |stops: &[usvg::Stop]| stops.first().map(|s| stop_color(s, opacity));
    match paint {
        usvg::Paint::Color(c) => Some([c.red, c.green, c.blue, alpha(opacity)]),
        usvg::Paint::LinearGradient(g) => first(g.stops()),
        usvg::Paint::RadialGradient(g) => first(g.stops()),
        usvg::Paint::Pattern(_) => None,
    }
    .unwrap_or([0, 0, 0, alpha(opacity)])
}

fn alpha(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn stop_color(s: &usvg::Stop, opacity: f32) -> [u8; 4] {
    let c = s.color();
    [c.red, c.green, c.blue, alpha(s.opacity().get() * opacity)]
}

/// The color an SVG gradient's stops give at `t`, padded past the ends.
fn color_at(stops: &[(f32, [u8; 4])], t: f32) -> [u8; 4] {
    let after = stops.iter().position(|s| s.0 > t);
    let (a, b) = match after {
        None => return stops.last().map_or([0; 4], |s| s.1),
        Some(0) => return stops[0].1,
        Some(i) => (stops[i - 1], stops[i]),
    };
    let k = (t - a.0) / (b.0 - a.0).max(1e-6);
    std::array::from_fn(|c| (a.1[c] as f32 + (b.1[c] as f32 - a.1[c] as f32) * k).round() as u8)
}

/// A gradient paint as a gradient spanning the shape's box `frame`; `to_frame` maps the
/// path's own coordinates into the shape's frame. A linear gradient keeps its direction
/// and where its colors fall; a radial one its center, its size only as near as an
/// ellipse the shape of the box can (its focus is left out). Both pad past their ends.
fn gradient(
    paint: &usvg::Paint,
    opacity: f32,
    to_frame: Transform,
    frame: Rect,
) -> Option<Gradient> {
    let (base, kind) = match paint {
        usvg::Paint::LinearGradient(g) => (&**g as &usvg::BaseGradient, GradientKind::Linear),
        usvg::Paint::RadialGradient(g) => (&**g as &usvg::BaseGradient, GradientKind::Radial),
        _ => return None,
    };
    let svg: Vec<(f32, [u8; 4])> = base
        .stops()
        .iter()
        .map(|s| (s.offset().get(), stop_color(s, opacity)))
        .collect();
    if svg.is_empty() {
        return None;
    }
    let m = base.transform().post_concat(to_frame);
    let at = |x: f32, y: f32| -> P2 { [m.sx * x + m.kx * y + m.tx, m.ky * x + m.sy * y + m.ty] };
    let mut g = Gradient {
        kind,
        ..Gradient::from_color(svg[0].1)
    };
    // Our position along the gradient, to the SVG's.
    let to_svg: Box<dyn Fn(f32) -> f32> = match paint {
        usvg::Paint::LinearGradient(l) => {
            let (p1, p2) = (at(l.x1(), l.y1()), at(l.x2(), l.y2()));
            let d = [p2[0] - p1[0], p2[1] - p1[1]];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if len < 1e-6 {
                // No length: all the last color.
                let last = svg[svg.len() - 1].1;
                g.stops = vec![Stop::new(0.0, last), Stop::new(1.0, last)];
                return Some(g);
            }
            let u = [d[0] / len, d[1] / len];
            // y grows downward: 90° runs upward.
            g.angle = (-u[1]).atan2(u[0]).to_degrees();
            let crate::gradient::Geometry::Linear { from, to } = g.geometry(frame) else {
                return None;
            };
            let dot = |v: P2| v[0] * u[0] + v[1] * u[1];
            let span = dot([to[0] - from[0], to[1] - from[1]]).max(1e-6);
            let start = dot([from[0] - p1[0], from[1] - p1[1]]);
            Box::new(move |t| (start + t * span) / len)
        }
        usvg::Paint::RadialGradient(r) => {
            let c = at(r.cx(), r.cy());
            let scale = (m.sx * m.sy - m.kx * m.ky).abs().sqrt();
            let radius = r.r().get() * scale;
            g.center = [
                (c[0] - frame.left()) / frame.width().max(1e-6),
                (c[1] - frame.top()) / frame.height().max(1e-6),
            ];
            let half = (frame.width() * frame.height()).sqrt() / 2.0;
            g.radius = (radius / half.max(1e-6)).max(1e-3);
            Box::new(|t| t)
        }
        _ => return None,
    };
    let from_svg = |s: f32| -> Option<f32> {
        // `to_svg` is linear: its inverse from two points.
        let (a, b) = (to_svg(0.0), to_svg(1.0));
        ((b - a).abs() > 1e-9).then(|| (s - a) / (b - a))
    };
    // The SVG's stops that fall on ours, and our ends.
    let mut stops: Vec<Stop> = svg
        .iter()
        .filter_map(|&(s, c)| {
            let t = from_svg(s)?;
            (0.0..=1.0).contains(&t).then(|| Stop::new(t, c))
        })
        .collect();
    for end in [0.0, 1.0] {
        if !stops.iter().any(|s| (s.pos - end).abs() < 1e-4) {
            stops.push(Stop::new(end, color_at(&svg, to_svg(end))));
        }
    }
    stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    if stops.len() > MAX_STOPS {
        let n = MAX_STOPS - 1;
        stops = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                Stop::new(t, color_at(&svg, to_svg(t)))
            })
            .collect();
    }
    g.stops = stops;
    Some(g)
}

/// The dash style nearest an SVG dash array (in the same units as `width`).
fn dash(array: Option<&[f32]>, width: f32) -> DashStyle {
    match array {
        None | Some([]) => DashStyle::Solid,
        Some(a) if a.len() >= 4 => DashStyle::DashDot,
        Some(a) if a[0] <= width * 1.5 => DashStyle::Dotted,
        Some(_) => DashStyle::Dashed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(svg: &str) -> Imported {
        let mut id = 0;
        import_bytes(svg.as_bytes(), [200.0, 200.0], [100.0, 100.0], &mut || {
            id += 1;
            id
        })
        .unwrap()
    }

    fn shape(node: &Node) -> &Shape {
        match node {
            Node::Layer(l) => l.shape.as_deref().unwrap(),
            Node::Group(_) => panic!("a group"),
        }
    }

    #[test]
    fn true_size_centered() {
        // 96 px = 1 inch = 25.4 mm.
        let out = import(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96">
                <rect x="0" y="0" width="96" height="48" fill="red"/></svg>"#,
        );
        let kids = &out.nodes;
        assert_eq!(kids.len(), 1);
        let Node::Layer(l) = &kids[0] else { panic!() };
        let s = l.shape.as_deref().unwrap();
        assert_eq!(s.kind, ShapeKind::Polygon);
        assert_eq!(s.fill_color, [255, 0, 0, 255]);
        assert!(!s.stroke);
        let size = s.geometry_size();
        assert!(
            (size[0] - 25.4).abs() < 0.01 && (size[1] - 12.7).abs() < 0.01,
            "{size:?}"
        );
        // The canvas is centered on (100, 100): the rectangle is its top half.
        let c = l.transform.center;
        assert!(
            (c[0] - 100.0).abs() < 0.1 && (c[1] - (100.0 - 6.35)).abs() < 0.1,
            "{c:?}"
        );
    }

    #[test]
    fn shrunk_to_the_page() {
        let out = import(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="400mm" height="100mm" viewBox="0 0 4 1">
                <rect width="4" height="1"/></svg>"#,
        );
        let size = shape(&out.nodes[0]).geometry_size();
        assert!(
            (size[0] - 200.0).abs() < 0.1 && (size[1] - 50.0).abs() < 0.1,
            "{size:?}"
        );
    }

    #[test]
    fn holes_stay_holes() {
        let out = import(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <path fill-rule="evenodd" d="M0 0H100V100H0Z M25 25H75V75H25Z"/></svg>"#,
        );
        let s = shape(&out.nodes[0]);
        assert_eq!(s.kind, ShapeKind::Path);
        assert!(s.even_odd);
        assert_eq!(s.contours.len(), 2);
        assert_eq!(s.points.len(), 8);
        assert_eq!(s.segment_count(), 8);
    }

    #[test]
    fn harmful_and_foreign_parts_left_out() {
        let out = import(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100" onload="alert(1)">
                <script>alert(1)</script>
                <image href="file:///etc/passwd" width="10" height="10"/>
                <image xlink:href="data:image/png;base64,iVBORw0KGgo=" width="10" height="10"/>
                <foreignObject width="10" height="10"><div xmlns="http://www.w3.org/1999/xhtml">x</div></foreignObject>
                <text x="10" y="10">hello</text>
                <a href="javascript:alert(1)"><circle cx="50" cy="50" r="10" onclick="alert(1)" fill="blue"/></a>
            </svg>"#,
        );
        let kids = &out.nodes;
        assert_eq!(kids.len(), 1, "only the circle");
        assert_eq!(shape(&kids[0]).fill_color, [0, 0, 255, 255]);
        let notice = out.notices.join(" ");
        for what in ["script", "image", "foreignObject", "text"] {
            assert!(notice.contains(what), "{notice}");
        }
    }

    #[test]
    fn groups_styles_and_gradients() {
        let out = import(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
                    <stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/>
                </linearGradient></defs>
                <style>.s { stroke: #00ff00; stroke-width: 4; stroke-dasharray: 12 6 }</style>
                <g id="logo" opacity="0.5">
                    <rect width="50" height="20" fill="url(#g)"/>
                    <polyline class="s" points="0,50 50,60 100,50" fill="none" stroke-linecap="round"/>
                </g>
            </svg>"##,
        );
        let kids = &out.nodes;
        let Node::Group(g) = &kids[0] else {
            panic!("a group")
        };
        assert_eq!(g.name, "logo");
        assert_eq!(g.opacity, 0.5);
        let rect = shape(&g.children[0]);
        let grad = rect.fill_gradient.as_ref().unwrap();
        assert_eq!(grad.kind, GradientKind::Linear);
        assert!(grad.angle.abs() < 1e-3);
        assert_eq!(grad.color_at(0.0)[..3], [0, 0, 0]);
        assert_eq!(grad.color_at(1.0)[..3], [255, 255, 255]);
        let line = shape(&g.children[1]);
        assert_eq!(line.kind, ShapeKind::Polyline);
        assert!(!line.fill && line.stroke);
        assert_eq!(line.stroke_color, [0, 255, 0, 255]);
        assert_eq!(line.cap, Cap::Round);
        assert_eq!(line.dash, DashStyle::Dashed);
        assert!((line.stroke_width - 4.0 / PX_PER_MM).abs() < 1e-3);
    }

    #[test]
    fn several_groups_come_in_one_group_named_after_the_file() {
        let svg = |body: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{body}</svg>"#
            )
        };
        let rect = r#"<rect width="10" height="10"/>"#;
        let mut id = 100;
        let mut new_id = || {
            id += 1;
            id
        };
        let mut nodes = |body: &str| {
            let out = import_bytes(
                svg(body).as_bytes(),
                [297.0, 210.0],
                [148.5, 105.0],
                &mut new_id,
            )
            .unwrap();
            one_group(out.nodes, "logo".into(), &mut new_id)
        };
        // Two groups and a shape: one group around them all.
        let both = nodes(&format!(
            r#"<g id="a">{rect}</g><g id="b">{rect}</g>{rect}"#
        ));
        let [Node::Group(g)] = &both[..] else {
            panic!("one group")
        };
        assert_eq!(g.name, "logo");
        assert_eq!(g.children.len(), 3);
        // One group alone, or shapes without groups: as they are.
        assert!(
            matches!(&nodes(&format!(r#"<g id="a">{rect}{rect}</g>"#))[..], [Node::Group(g)] if g.name == "a")
        );
        assert_eq!(nodes(&format!("{rect}{rect}")).len(), 2);
    }

    /// Imports the SVG file named by `SVG_FILE` into a page and renders it as an export
    /// does, to look at it:
    /// `SVG_FILE=in.svg SVG_PAGE_PREVIEW=out.png cargo test svg_page_preview -- --ignored`.
    #[test]
    #[ignore]
    fn svg_page_preview() {
        use crate::render::{Cuts, Scene, compose};
        let (Ok(svg), Ok(out)) = (std::env::var("SVG_FILE"), std::env::var("SVG_PAGE_PREVIEW"))
        else {
            return;
        };
        let mut doc = crate::document::Document::default();
        let (page, center) = (doc.page_mm(), doc.page_center());
        let mut id = 0;
        let imported = super::import(FsPath::new(&svg), page, center, &mut || {
            id += 1;
            id
        })
        .unwrap();
        for n in &imported.notices {
            eprintln!("{n}");
        }
        doc.layers = imported.nodes;
        crate::shape::prepare_export(&mut doc, 100.0);
        let scene = Scene::new(
            &doc,
            None,
            [[255, 0, 255]; crate::template::KINDS],
            [true; crate::template::KINDS],
            Cuts::default(),
        );
        compose(&scene, 100.0, false)
            .unwrap()
            .save_png(out)
            .unwrap();
    }

    #[test]
    fn joins_kept() {
        let out = import(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <polygon points="60,60 70,80 50,80" fill="#fc0" stroke="#000"
                    stroke-linejoin="round" stroke-width="2"/></svg>"##,
        );
        let s = shape(&out.nodes[0]);
        assert_eq!(s.join, Join::Round, "{:?}", s.kind);
    }

    #[test]
    fn entity_bombs_are_refused() {
        let mut svg = String::from(r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a0 "lol">"#);
        for i in 1..10 {
            let p = format!("&a{};", i - 1).repeat(10);
            svg.push_str(&format!(r#"<!ENTITY a{i} "{p}">"#));
        }
        svg.push_str(r#"]><svg xmlns="http://www.w3.org/2000/svg"><text>&a9;</text></svg>"#);
        let mut id = 0;
        let r = import_bytes(svg.as_bytes(), [100.0; 2], [50.0; 2], &mut || {
            id += 1;
            id
        });
        assert!(r.is_err());
    }
}
