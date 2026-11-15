//! Background layers: a shape filling the template's outline grown by the bleed, so the
//! print has no white rim where the cut drifts. Unlike the page background (see
//! `Document::background`), it can go anywhere in the stack and take opacity, blend
//! modes, effects, or be clipped to.
//!
//! It follows the outline (`Shape::follows_outline`): whenever the outline or the bleed
//! changes, its shape is redrawn (`sync`), and it can't be moved or reshaped by hand
//! (`is_fixed`). Its fill, opacity, effects and so on change as any shape's.

use crate::document::{Document, Layer, P2};
use crate::render::{self, Cuts, PlacedElement};
use crate::shape::{self, Contour, Shape, ShapeKind};

/// The outline grown by `bleed_mm`, as closed rings in page mm under the nonzero rule;
/// `None` when the template has no closed outline.
pub fn rings(elements: &[PlacedElement], bleed_mm: f32) -> Option<Vec<Vec<P2>>> {
    let cuts = Cuts {
        clip_to_outline: true,
        bleed_mm,
        ..Cuts::default()
    };
    render::cut_rings(elements, cuts).0
}

/// A layer filling `rings`, with the page's background as its fill.
pub fn new_layer(id: u64, name: String, rings: &[Vec<P2>], doc: &Document) -> Layer {
    let mut shape = geometry(rings);
    let [r, g, b] = doc.background;
    shape.fill = true;
    shape.stroke = false;
    shape.fill_color = [r, g, b, 255];
    shape.fill_gradient = doc.background_gradient.clone();
    // The page's pattern starts at the page's corner; this one at the shape's.
    shape.fill_pattern = doc.background_pattern.clone().map(|mut p| {
        let lo = lowest(rings);
        p.offset = [p.offset[0] - lo[0], p.offset[1] - lo[1]];
        p
    });
    shape.follows_outline = true;
    shape::new_shape_layer(id, name, shape, [0.0; 2])
}

/// The top-left corner of the box around `rings`.
fn lowest(rings: &[Vec<P2>]) -> P2 {
    rings
        .iter()
        .flatten()
        .fold([f32::MAX; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])])
}

/// A path shape of `rings`, in page mm, filled under the nonzero rule.
fn geometry(rings: &[Vec<P2>]) -> Shape {
    let mut shape = Shape::new(ShapeKind::Path);
    shape.points = rings.iter().flatten().copied().collect();
    shape.contours = rings
        .iter()
        .map(|r| Contour {
            len: r.len(),
            closed: true,
        })
        .collect();
    shape
}

/// The layer follows the outline: it is a background.
pub fn is_fixed(layer: &Layer) -> bool {
    layer.shape.as_ref().is_some_and(|s| s.follows_outline)
}

/// Brings the background layers onto `rings`, the outline grown by the bleed, when it
/// is known: their shape and placement are put back if they differ. Returns whether any
/// changed.
pub fn sync(doc: &mut Document, rings: Option<&[Vec<P2>]>) -> bool {
    let Some(rings) = rings else {
        return false;
    };
    let mut want: Option<Layer> = None;
    let mut changed = false;
    doc.for_each_layer_mut(|layer| {
        if !is_fixed(layer) {
            return;
        }
        let want = want.get_or_insert_with(|| {
            shape::new_shape_layer(0, String::new(), geometry(rings), [0.0; 2])
        });
        let (Some(s), Some(w)) = (layer.shape.as_deref_mut(), want.shape.as_deref()) else {
            return;
        };
        if s.points != w.points || s.contours != w.contours || layer.transform != want.transform {
            s.points = w.points.clone();
            s.contours = w.contours.clone();
            s.handles.clear();
            // Its box laid out for the new points, where the fresh layer has it.
            layer.transform = want.transform;
            layer.size_px = want.size_px;
            layer.shape_state = want.shape_state.clone();
            changed = true;
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Node;
    use crate::template::{ElementKind, Polyline};

    /// A template outline: the rectangle from `lo` to `hi` in page mm.
    fn outline(lo: P2, hi: P2) -> Vec<PlacedElement> {
        vec![PlacedElement {
            kind: ElementKind::Outline,
            polylines: vec![Polyline {
                points: vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]],
                closed: true,
            }],
            centers: Vec::new(),
        }]
    }

    /// The box around the layer's points on the page.
    fn page_box(doc: &Document, id: u64) -> (P2, P2) {
        let pts = shape::page_points(doc.layer(id).unwrap());
        pts.iter()
            .fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| {
                (
                    [lo[0].min(p[0]), lo[1].min(p[1])],
                    [hi[0].max(p[0]), hi[1].max(p[1])],
                )
            })
    }

    fn close(a: P2, b: P2) -> bool {
        (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05
    }

    #[test]
    fn a_background_fills_the_outline_and_stays_on_it() {
        let mut doc = Document {
            background: [10, 20, 30],
            ..Default::default()
        };
        let rings = rings(&outline([20.0, 30.0], [120.0, 80.0]), 3.0).unwrap();
        let l = new_layer(1, "bg".into(), &rings, &doc);
        let s = l.shape.as_ref().unwrap();
        assert!(s.follows_outline && s.fill && !s.stroke);
        assert_eq!(s.fill_color, [10, 20, 30, 255]);
        doc.layers.push(Node::Layer(Box::new(l)));
        // The outline grown by the 3 mm bleed.
        let (lo, hi) = page_box(&doc, 1);
        assert!(
            close(lo, [17.0, 27.0]) && close(hi, [123.0, 83.0]),
            "{lo:?} {hi:?}"
        );
        // Moved by hand (or by a nudge, which leaves it), it goes back.
        assert!(!crate::layers_ui::nudge(&mut doc, Some(1), [5.0, 0.0]));
        doc.layer_mut(1).unwrap().transform.center[0] += 5.0;
        assert!(sync(&mut doc, Some(&rings)));
        assert!(!sync(&mut doc, Some(&rings)), "nothing more to do");
        let (lo, _) = page_box(&doc, 1);
        assert!(close(lo, [17.0, 27.0]), "{lo:?}");
        // Another outline: it follows.
        let other = rings_of([0.0, 0.0], [50.0, 40.0]);
        assert!(sync(&mut doc, Some(&other)));
        let (lo, hi) = page_box(&doc, 1);
        assert!(
            close(lo, [-3.0, -3.0]) && close(hi, [53.0, 43.0]),
            "{lo:?} {hi:?}"
        );
        // Saved and read back, it still follows.
        let json = serde_json::to_string(&doc).unwrap();
        let back = Document::from_json(&json).unwrap();
        assert!(is_fixed(back.layer(1).unwrap()));
    }

    fn rings_of(lo: P2, hi: P2) -> Vec<Vec<P2>> {
        rings(&outline(lo, hi), 3.0).unwrap()
    }

    #[test]
    fn no_outline_no_background() {
        assert!(rings(&[], 3.0).is_none());
        let open = vec![PlacedElement {
            kind: ElementKind::Outline,
            polylines: vec![Polyline {
                points: vec![[0.0, 0.0], [10.0, 0.0]],
                closed: false,
            }],
            centers: Vec::new(),
        }];
        assert!(rings(&open, 3.0).is_none());
    }
}
