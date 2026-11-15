//! Printing aids: cut shapes grown by the bleed or shrunk to the safe area, and the crop
//! and registration marks around the outline.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};

use crate::document::P2;
use crate::template::Polyline;

/// Largest bleed and safe margin offered, in mm.
pub const MAX_MARGIN_MM: f32 = 25.0;
/// Width of the crop and registration marks, in mm (about 0.3 pt).
pub const MARK_WIDTH_MM: f32 = 0.1;
/// Color of the marks: black, which prints on every plate.
pub const MARK_COLOR: [u8; 3] = [0, 0, 0];
/// Length of a crop mark.
const CROP_MARK_MM: f32 = 5.0;
/// Space between the bleed's edge and the marks, so a cut never shows them.
const MARK_GAP_MM: f32 = 2.0;
/// Radius of a registration mark's circle.
const TARGET_RADIUS_MM: f32 = 2.0;
/// Longest straight piece of an arc, in mm, where offsetting rounds a corner.
const ARC_PIECE_MM: f32 = 0.2;

/// Closed shapes, filled under the non-zero rule, moved outward by `d` mm (inward when
/// negative), with rounded corners. The rings returned are filled under the non-zero
/// rule too; a shape thinner than twice an inward offset disappears.
pub fn offset(rings: &[Vec<P2>], d: f32) -> Vec<Vec<P2>> {
    if d.abs() < 0.001 || rings.is_empty() {
        return rings.to_vec();
    }
    // Outlines in the offset are oriented by what they enclose, whatever order the DXF
    // points came in.
    let shapes = rings.simplify_shape(FillRule::NonZero);
    let style = OutlineStyle::new(d).line_join(LineJoin::Round((ARC_PIECE_MM / d.abs()).min(0.5)));
    shapes.outline(&style).into_iter().flatten().collect()
}

/// Corners of the box around `polylines`, if they have any point.
pub fn bounds<'a>(polylines: impl IntoIterator<Item = &'a Polyline>) -> Option<(P2, P2)> {
    let mut pts = polylines.into_iter().flat_map(|p| &p.points);
    let first = *pts.next()?;
    Some(pts.fold((first, first), |(mn, mx), p| {
        (
            [mn[0].min(p[0]), mn[1].min(p[1])],
            [mx[0].max(p[0]), mx[1].max(p[1])],
        )
    }))
}

fn segment(a: P2, b: P2) -> Polyline {
    Polyline {
        points: vec![a, b],
        closed: false,
    }
}

/// Marks around the trim box `(min, max)` in page mm, outside a bleed of `bleed` mm:
/// crop marks at its corners, in line with its edges, and registration targets at the
/// middle of its sides.
pub fn marks(trim: (P2, P2), bleed: f32, crop: bool, registration: bool) -> Vec<Polyline> {
    let ([x0, y0], [x1, y1]) = trim;
    let near = bleed.max(0.0) + MARK_GAP_MM;
    let far = near + CROP_MARK_MM;
    let mut out = Vec::new();
    if crop {
        for (x, out_x) in [(x0, -1.0), (x1, 1.0)] {
            for (y, out_y) in [(y0, -1.0), (y1, 1.0)] {
                // A horizontal mark in line with the top or bottom edge, and a vertical
                // one in line with the left or right edge.
                out.push(segment([x + out_x * near, y], [x + out_x * far, y]));
                out.push(segment([x, y + out_y * near], [x, y + out_y * far]));
            }
        }
    }
    if registration {
        let mid = (near + far) / 2.0;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        for c in [
            [cx, y0 - mid],
            [cx, y1 + mid],
            [x0 - mid, cy],
            [x1 + mid, cy],
        ] {
            out.extend(target(c));
        }
    }
    out
}

/// A registration target: two circles and a cross through them.
fn target(c: P2) -> Vec<Polyline> {
    let arm = CROP_MARK_MM / 2.0;
    let circle = |r: f32| Polyline {
        points: (0..48)
            .map(|i| {
                let a = i as f32 / 48.0 * std::f32::consts::TAU;
                [c[0] + r * a.cos(), c[1] + r * a.sin()]
            })
            .collect(),
        closed: true,
    };
    vec![
        circle(TARGET_RADIUS_MM),
        circle(TARGET_RADIUS_MM / 2.0),
        segment([c[0] - arm, c[1]], [c[0] + arm, c[1]]),
        segment([c[0], c[1] - arm], [c[0], c[1] + arm]),
    ]
}

/// Artwork larger than the paper, split over several sheets. Each sheet keeps a blank
/// margin for the printer and the marks; the artwork inside it overlaps the next
/// sheet's by `overlap_mm`, to trim one sheet in the overlap and glue it over the other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiling {
    /// Sheet width and height in mm.
    pub sheet_mm: P2,
    /// Blank border of each sheet, in mm.
    pub margin_mm: f32,
    /// Artwork shared by neighboring sheets, in mm.
    pub overlap_mm: f32,
    /// Crop marks at the corners of each sheet's artwork, and ticks on the line where
    /// sheets are trimmed and lined up, all in the margin.
    pub marks: bool,
    /// Where the sheets sit, in mm from the grid centered on the page. Sheets are added
    /// or left out so they still cover the whole page.
    pub offset_mm: P2,
}

/// One sheet of a tiling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    /// From the top, from 0.
    pub row: usize,
    /// From the left, from 0.
    pub col: usize,
    /// Top-left corner of its artwork, on the page in mm.
    pub origin_mm: P2,
}

impl Tiling {
    /// Width and height of the artwork on each sheet, in mm: the sheet within its
    /// margins, at least 10 mm.
    pub fn content_mm(&self) -> P2 {
        self.sheet_mm
            .map(|s| (s - 2.0 * self.margin_mm.max(0.0)).max(10.0))
    }

    /// Overlap actually used: at most half the artwork on a sheet.
    fn overlap(&self) -> P2 {
        self.content_mm()
            .map(|c| self.overlap_mm.clamp(0.0, c / 2.0))
    }

    /// Distance between the starts of neighboring sheets' artwork.
    fn step(&self) -> P2 {
        let (content, overlap) = (self.content_mm(), self.overlap());
        [0, 1].map(|i| content[i] - overlap[i])
    }

    /// Along x (`i` = 0) or y (1): where the first sheet's artwork starts on the page,
    /// and how many sheets it takes to cover the page's `len` mm.
    fn axis(&self, i: usize, len: f32) -> (f32, usize) {
        let (c, st, o) = (self.content_mm()[i], self.step()[i], self.overlap()[i]);
        // Centered: the fewest sheets that cover the page, as much past each end.
        let fewest = if len - c <= 0.01 {
            1
        } else {
            1 + ((len - c) / st - 0.0001).ceil() as usize
        };
        let start = (len - (c + (fewest - 1) as f32 * st)) / 2.0 + self.offset_mm[i];
        // The sheets covering some of the page that their neighbors don't: before the
        // next sheet starts, or past where the previous one ends.
        let first = ((0.01 - start - st) / st).floor() as i64 + 1;
        let last = ((len - 0.01 - o - start) / st).ceil() as i64 - 1;
        (
            start + first as f32 * st,
            (last - first + 1).max(1) as usize,
        )
    }

    /// Number of sheets across and down for a page of `page_mm`.
    pub fn grid(&self, page_mm: P2) -> [usize; 2] {
        [0, 1].map(|i| self.axis(i, page_mm[i]).1)
    }

    /// The sheets for a page of `page_mm`, row by row from the top left.
    pub fn tiles(&self, page_mm: P2) -> Vec<Tile> {
        let (x, cols) = self.axis(0, page_mm[0]);
        let (y, rows) = self.axis(1, page_mm[1]);
        let step = self.step();
        (0..rows)
            .flat_map(|row| {
                (0..cols).map(move |col| Tile {
                    row,
                    col,
                    origin_mm: [x + col as f32 * step[0], y + row as f32 * step[1]],
                })
            })
            .collect()
    }

    /// The part of the page plane the sheets' artwork covers, for a page of `page_mm`:
    /// corners in page mm. The outer sheets reach past the page.
    pub fn covered(&self, page_mm: P2) -> (P2, P2) {
        let tiles = self.tiles(page_mm);
        let content = self.content_mm();
        let first = tiles.first().map_or([0.0; 2], |t| t.origin_mm);
        let last = tiles.last().map_or([0.0; 2], |t| t.origin_mm);
        (first, [last[0] + content[0], last[1] + content[1]])
    }

    /// Where the artwork sits on a sheet: corners in sheet mm.
    pub fn content_box(&self) -> (P2, P2) {
        let m = self.margin_mm.max(0.0);
        let [w, h] = self.content_mm();
        ([m, m], [m + w, m + h])
    }

    /// Where `tile`'s artwork sits on its sheet, for a page of `page_mm`: the content box,
    /// cut to where the page starts and ends on the outer sheets.
    pub fn tile_box(&self, tile: Tile, page_mm: P2) -> (P2, P2) {
        let (min, max) = self.content_box();
        let o = tile.origin_mm;
        (
            [0, 1].map(|i| min[i] - o[i].min(0.0)),
            [0, 1].map(|i| max[i].min(min[i] + page_mm[i] - o[i])),
        )
    }

    /// Marks of `tile` for a page of `page_mm`, in sheet mm: crop marks at the corners of
    /// its artwork, and on each side shared with another sheet, two ticks in the margin
    /// in line with the middle of the overlap, where sheets are trimmed and lined up.
    /// Empty without `marks`.
    pub fn marks(&self, tile: Tile, page_mm: P2) -> Vec<Polyline> {
        if !self.marks {
            return Vec::new();
        }
        let grid = self.grid(page_mm);
        let ([x0, y0], [x1, y1]) = self.tile_box(tile, page_mm);
        let mut out = marks(([x0, y0], [x1, y1]), 0.0, true, false);
        let [ox, oy] = self.overlap().map(|o| o / 2.0);
        // As far out as the crop marks.
        let (near, far) = (MARK_GAP_MM, MARK_GAP_MM + CROP_MARK_MM);
        let vertical = |x: f32| {
            [
                segment([x, y0 - near], [x, y0 - far]),
                segment([x, y1 + near], [x, y1 + far]),
            ]
        };
        let horizontal = |y: f32| {
            [
                segment([x0 - near, y], [x0 - far, y]),
                segment([x1 + near, y], [x1 + far, y]),
            ]
        };
        if tile.col > 0 {
            out.extend(vertical(x0 + ox));
        }
        if tile.col + 1 < grid[0] {
            out.extend(vertical(x1 - ox));
        }
        if tile.row > 0 {
            out.extend(horizontal(y0 + oy));
        }
        if tile.row + 1 < grid[1] {
            out.extend(horizontal(y1 - oy));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<P2> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    fn ring_bounds(rings: &[Vec<P2>]) -> (P2, P2) {
        let lines: Vec<Polyline> = rings
            .iter()
            .map(|r| Polyline {
                points: r.clone(),
                closed: true,
            })
            .collect();
        bounds(&lines).unwrap()
    }

    fn close(a: P2, b: P2) -> bool {
        (a[0] - b[0]).abs() < 0.02 && (a[1] - b[1]).abs() < 0.02
    }

    #[test]
    fn offsets_grow_and_shrink_whatever_the_point_order() {
        let cw = square(10.0, 10.0, 50.0, 30.0);
        let mut ccw = cw.clone();
        ccw.reverse();
        for ring in [cw, ccw] {
            let grown = offset(std::slice::from_ref(&ring), 3.0);
            assert_eq!(grown.len(), 1);
            let (mn, mx) = ring_bounds(&grown);
            assert!(
                close(mn, [7.0, 7.0]) && close(mx, [53.0, 33.0]),
                "{mn:?} {mx:?}"
            );
            // Rounded corners: more points than the square had.
            assert!(grown[0].len() > 8);

            let shrunk = offset(std::slice::from_ref(&ring), -3.0);
            let (mn, mx) = ring_bounds(&shrunk);
            assert!(
                close(mn, [13.0, 13.0]) && close(mx, [47.0, 27.0]),
                "{mn:?} {mx:?}"
            );
        }
        // Too thin for the inward offset: nothing is left.
        assert!(offset(&[square(0.0, 0.0, 4.0, 40.0)], -2.5).is_empty());
        // No offset keeps the shapes as they are.
        assert_eq!(offset(&[square(0.0, 0.0, 4.0, 4.0)], 0.0).len(), 1);
    }

    #[test]
    fn offsets_keep_holes_in_shapes() {
        // A frame: a square with a square hole, both moved out by 1 mm.
        let frame = [
            square(0.0, 0.0, 40.0, 40.0),
            square(10.0, 10.0, 30.0, 30.0).into_iter().rev().collect(),
        ];
        let out = offset(&frame, 1.0);
        assert_eq!(out.len(), 2, "the outside and the hole");
        let inner = out
            .iter()
            .map(|r| ring_bounds(std::slice::from_ref(r)))
            .find(|(mn, _)| mn[0] > 5.0)
            .unwrap();
        assert!(close(inner.0, [11.0, 11.0]) && close(inner.1, [29.0, 29.0]));
    }

    #[test]
    fn marks_sit_outside_the_bleed() {
        let trim = ([20.0, 30.0], [120.0, 90.0]);
        let crop = marks(trim, 3.0, true, false);
        assert_eq!(crop.len(), 8, "two at each corner");
        for m in &crop {
            for p in &m.points {
                let inside_x = p[0] > 20.0 - 5.0 + 0.01 && p[0] < 120.0 + 5.0 - 0.01;
                let inside_y = p[1] > 30.0 - 5.0 + 0.01 && p[1] < 90.0 + 5.0 - 0.01;
                assert!(!(inside_x && inside_y), "{p:?} is in the bleed");
            }
        }
        // In line with the trim box's edges.
        assert!(
            crop.iter()
                .any(|m| m.points == [[15.0, 30.0], [10.0, 30.0]])
        );
        assert!(
            crop.iter()
                .any(|m| m.points == [[20.0, 25.0], [20.0, 20.0]])
        );

        let both = marks(trim, 3.0, true, true);
        assert_eq!(both.len(), 8 + 4 * 4);
        let (mn, mx) = bounds(&both).unwrap();
        assert!(
            close(mn, [10.0, 20.0]) && close(mx, [130.0, 100.0]),
            "{mn:?} {mx:?}"
        );
        assert!(marks(trim, 3.0, false, false).is_empty());
    }

    #[test]
    fn tiles_cover_the_page_with_overlap() {
        // A4 portrait sheets with 10 mm margins: 190 x 277 mm of artwork each, 180 and
        // 267 mm apart.
        let t = Tiling {
            sheet_mm: [210.0, 297.0],
            margin_mm: 10.0,
            overlap_mm: 10.0,
            marks: true,
            offset_mm: [0.0, 0.0],
        };
        assert_eq!(t.grid([190.0, 277.0]), [1, 1], "fits on one sheet");
        assert_eq!(t.grid([370.0, 277.0]), [2, 1], "190 + 180");
        let page = [371.0, 300.0];
        assert_eq!(t.grid(page), [3, 2]);
        // Centered: 550 x 544 mm of sheets, as much past each side of the page.
        let tiles = t.tiles(page);
        assert_eq!(tiles.len(), 6);
        assert_eq!(tiles[0].origin_mm, [-89.5, -122.0]);
        assert_eq!(
            tiles[4],
            Tile {
                row: 1,
                col: 1,
                origin_mm: [90.5, 145.0]
            }
        );
        assert_eq!(t.covered(page), ([-89.5, -122.0], [460.5, 422.0]));
        // A small page is centered on one sheet.
        assert_eq!(t.tiles([100.0, 200.0])[0].origin_mm, [-45.0, -38.5]);

        // Moved: the grid starts at the page's corner.
        let moved = Tiling {
            offset_mm: [89.5, 122.0],
            ..t
        };
        assert_eq!(moved.tiles(page)[0].origin_mm, [0.0, 0.0]);
        assert_eq!(moved.grid(page), [3, 2]);
        // Two sheets cover 370 mm exactly; moved by 5 mm, a third one is needed.
        let shifted = Tiling {
            offset_mm: [5.0, 0.0],
            ..t
        };
        assert_eq!(t.grid([370.0, 277.0]), [2, 1]);
        assert_eq!(shifted.grid([370.0, 277.0]), [3, 1]);
        assert_eq!(shifted.tiles([370.0, 277.0])[0].origin_mm[0], 5.0 - 180.0);

        // A corner sheet: 8 crop marks, ticks on its two inner sides; the page starts
        // 89.5 mm into its artwork across and 122 mm down.
        let corner = t.marks(tiles[0], page);
        assert_eq!(corner.len(), 8 + 2 + 2);
        assert!(corner.iter().any(|m| m.points[0] == [99.5 - 2.0, 132.0]));
        // The middle of the overlap with the next sheet: 5 mm in from the right edge.
        assert!(
            corner
                .iter()
                .any(|m| m.points[0][0] == 195.0 && m.points[0][1] < 132.0)
        );
        let middle = t.marks(tiles[1], page);
        assert_eq!(middle.len(), 8 + 2 * 3);
        // All in the margin or on blank paper, never on the artwork.
        for (tile, marks) in [(tiles[0], &corner), (tiles[1], &middle)] {
            let ([x0, y0], [x1, y1]) = t.tile_box(tile, page);
            for p in marks.iter().flat_map(|m| &m.points) {
                let inside = p[0] > x0 && p[0] < x1 && p[1] > y0 && p[1] < y1;
                assert!(!inside, "{p:?}");
            }
        }
        // The last row's crop marks follow the page's bottom edge: 155 mm into it.
        let bottom = t.marks(tiles[4], page);
        assert!(bottom.iter().any(|m| m.points[0] == [8.0, 165.0]));
        assert!(
            Tiling { marks: false, ..t }
                .marks(tiles[0], page)
                .is_empty()
        );
    }
}
