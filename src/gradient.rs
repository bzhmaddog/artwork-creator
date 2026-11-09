//! Color gradients: a shape's fill or stroke, a text's letters or the page background
//! filled with colors that change across it, linearly at an angle or around a center.
//!
//! A gradient spans a box (the layer's, or the page): a linear one runs at its angle from
//! the box's corner furthest back to the one furthest ahead, so the first and last colors
//! reach the corners; a radial one grows from its center in an ellipse the shape of the
//! box, 100% reaching its sides. Colors are given by stops along the way, each with its
//! own opacity, and may be linked to color variables like any color (see `colors`).

use serde::{Deserialize, Serialize};
use tiny_skia::{Color, GradientStop, Point, Rect, Shader, SpreadMode, Transform};

/// How the colors spread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
}

/// A color along a gradient.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// Where along the gradient, from 0 (its start) to 1 (its end).
    pub pos: f32,
    /// Straight RGBA.
    pub color: [u8; 4],
    /// Name of the color variable the color is linked to (see `colors`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl Stop {
    pub fn new(pos: f32, color: [u8; 4]) -> Self {
        Self {
            pos,
            color,
            link: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    /// Linear: the direction the colors run, in degrees: 0 from left to right, 90 from
    /// bottom to top.
    #[serde(default)]
    pub angle: f32,
    /// Radial: the center, as a fraction of the box's width and height.
    #[serde(default = "middle")]
    pub center: [f32; 2],
    /// Radial: how far the last color is, 1 reaching the box's sides.
    #[serde(default = "one")]
    pub radius: f32,
    /// At least two, in any order (see [`Gradient::sorted`]).
    pub stops: Vec<Stop>,
}

fn middle() -> [f32; 2] {
    [0.5, 0.5]
}

fn one() -> f32 {
    1.0
}

/// Most stops a gradient has.
pub const MAX_STOPS: usize = 16;

impl Gradient {
    /// A linear gradient from `color` on the left to white on the right, to start from.
    pub fn from_color(color: [u8; 4]) -> Self {
        Self {
            kind: GradientKind::Linear,
            angle: 0.0,
            center: middle(),
            radius: 1.0,
            stops: vec![Stop::new(0.0, color), Stop::new(1.0, [255, 255, 255, 255])],
        }
    }

    /// The stops in order along the gradient, positions within 0 to 1. Never fewer than
    /// one: an empty gradient is transparent.
    pub fn sorted(&self) -> Vec<Stop> {
        let mut stops: Vec<Stop> = self
            .stops
            .iter()
            .map(|s| Stop {
                pos: if s.pos.is_finite() {
                    s.pos.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                ..s.clone()
            })
            .collect();
        stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        if stops.is_empty() {
            stops.push(Stop::new(0.0, [0; 4]));
        }
        stops
    }

    /// The color at `t` along the gradient (0 to 1), straight RGBA.
    pub fn color_at(&self, t: f32) -> [u8; 4] {
        let stops = self.sorted();
        let t = t.clamp(0.0, 1.0);
        let after = stops.iter().position(|s| s.pos > t);
        let (a, b) = match after {
            None => return stops[stops.len() - 1].color,
            Some(0) => return stops[0].color,
            Some(i) => (&stops[i - 1], &stops[i]),
        };
        let k = (t - a.pos) / (b.pos - a.pos).max(1e-6);
        std::array::from_fn(|c| {
            (a.color[c] as f32 + (b.color[c] as f32 - a.color[c] as f32) * k).round() as u8
        })
    }

    /// Every stop fully opaque: what a PDF gradient can show, being RGB only.
    pub fn is_opaque(&self) -> bool {
        self.stops.iter().all(|s| s.color[3] == 255)
    }

    /// Where it runs over `frame`: from and to points for a linear one; for a radial one,
    /// a map from the unit circle around the origin to its ellipse.
    pub fn geometry(&self, frame: Rect) -> Geometry {
        match self.kind {
            GradientKind::Linear => {
                let a = self.angle.to_radians();
                // y grows downward: 90° runs upward.
                let (dx, dy) = (a.cos(), -a.sin());
                let c = [
                    (frame.left() + frame.right()) / 2.0,
                    (frame.top() + frame.bottom()) / 2.0,
                ];
                // Half the box's extent along the direction: its corners land on 0 and 1.
                let half = (frame.width() * dx.abs() + frame.height() * dy.abs()) / 2.0;
                let half = half.max(1e-3);
                Geometry::Linear {
                    from: [c[0] - dx * half, c[1] - dy * half],
                    to: [c[0] + dx * half, c[1] + dy * half],
                }
            }
            GradientKind::Radial => {
                let cx = frame.left() + frame.width() * self.center[0];
                let cy = frame.top() + frame.height() * self.center[1];
                let r = self.radius.max(1e-3);
                let rx = (frame.width() / 2.0 * r).max(1e-3);
                let ry = (frame.height() / 2.0 * r).max(1e-3);
                Geometry::Radial {
                    ellipse: Transform::from_row(rx, 0.0, 0.0, ry, cx, cy),
                }
            }
        }
    }

    /// A shader painting it over `frame`, given in the units of the paths it fills.
    pub fn shader(&self, frame: Rect) -> Option<Shader<'static>> {
        let stops: Vec<GradientStop> = self
            .sorted()
            .iter()
            .map(|s| {
                let [r, g, b, a] = s.color;
                GradientStop::new(s.pos, Color::from_rgba8(r, g, b, a))
            })
            .collect();
        match self.geometry(frame) {
            Geometry::Linear { from, to } => tiny_skia::LinearGradient::new(
                Point::from_xy(from[0], from[1]),
                Point::from_xy(to[0], to[1]),
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            ),
            Geometry::Radial { ellipse } => tiny_skia::RadialGradient::new(
                Point::zero(),
                0.0,
                Point::zero(),
                1.0,
                stops,
                SpreadMode::Pad,
                ellipse,
            ),
        }
    }
}

/// Where a gradient runs, in the units of the box it spans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geometry {
    /// From its first color at `from` to its last at `to`.
    Linear { from: [f32; 2], to: [f32; 2] },
    /// Its first color at the center, its last on the unit circle, mapped by `ellipse`.
    Radial { ellipse: Transform },
}

/// What fills a shape: one color, or a gradient over a box.
#[derive(Clone, Debug, PartialEq)]
pub enum Ink {
    /// Straight RGBA.
    Solid([u8; 4]),
    /// Spanning `frame`, in the units of the paths it fills.
    Gradient { gradient: Gradient, frame: Rect },
}

impl Ink {
    /// A gradient when there is one, else the color.
    pub fn of(color: [u8; 4], gradient: Option<&Gradient>, frame: Option<Rect>) -> Self {
        match (gradient, frame) {
            (Some(g), Some(frame)) => Ink::Gradient {
                gradient: g.clone(),
                frame,
            },
            _ => Ink::Solid(color),
        }
    }

    /// A paint for tiny-skia, antialiased.
    pub fn paint(&self) -> tiny_skia::Paint<'static> {
        let mut paint = tiny_skia::Paint {
            anti_alias: true,
            ..Default::default()
        };
        match self {
            Ink::Solid([r, g, b, a]) => paint.set_color_rgba8(*r, *g, *b, *a),
            Ink::Gradient { gradient, frame } => match gradient.shader(*frame) {
                Some(shader) => paint.shader = shader,
                None => {
                    let [r, g, b, a] = gradient.color_at(0.0);
                    paint.set_color_rgba8(r, g, b, a);
                }
            },
        }
        paint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two(a: [u8; 4], b: [u8; 4]) -> Gradient {
        Gradient {
            stops: vec![Stop::new(1.0, b), Stop::new(0.0, a)],
            ..Gradient::from_color(a)
        }
    }

    #[test]
    fn colors_between_stops_in_any_order() {
        let g = two([0, 0, 0, 255], [200, 100, 0, 255]);
        assert_eq!(g.color_at(0.0), [0, 0, 0, 255]);
        assert_eq!(g.color_at(0.5), [100, 50, 0, 255]);
        assert_eq!(g.color_at(1.0), [200, 100, 0, 255]);
        // Outside the stops: the nearest one's color.
        let mut g = g;
        g.stops[0].pos = 0.8;
        g.stops[1].pos = 0.2;
        assert_eq!(g.color_at(0.1), [0, 0, 0, 255]);
        assert_eq!(g.color_at(0.9), [200, 100, 0, 255]);
    }

    #[test]
    fn linear_runs_corner_to_corner_at_its_angle() {
        let frame = Rect::from_xywh(0.0, 0.0, 100.0, 50.0).unwrap();
        let mut g = Gradient::from_color([0, 0, 0, 255]);
        assert_eq!(
            g.geometry(frame),
            Geometry::Linear {
                from: [0.0, 25.0],
                to: [100.0, 25.0]
            }
        );
        // 90°: from the bottom up.
        g.angle = 90.0;
        let Geometry::Linear { from, to } = g.geometry(frame) else {
            panic!()
        };
        assert!(
            (from[1] - 50.0).abs() < 1e-3 && to[1].abs() < 1e-3,
            "{from:?} {to:?}"
        );
    }

    /// Draws a gradient over a box; returns the pixmap.
    fn draw(g: &Gradient, w: u32, h: u32) -> tiny_skia::Pixmap {
        let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
        let frame = Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
        let ink = Ink::Gradient {
            gradient: g.clone(),
            frame,
        };
        pm.fill_rect(frame, &ink.paint(), Transform::identity(), None);
        pm
    }

    #[test]
    fn shaders_draw_the_stops_where_they_are() {
        let g = two([255, 0, 0, 255], [0, 0, 255, 255]);
        let pm = draw(&g, 100, 10);
        let px = |x, y| pm.pixel(x, y).unwrap();
        assert!(px(0, 5).red() > 245 && px(0, 5).blue() < 10);
        assert!(px(99, 5).blue() > 245 && px(99, 5).red() < 10);
        let mid = px(50, 5);
        assert!((115..140).contains(&mid.red()), "{mid:?}");

        // Radial: red in the middle, blue at the sides and beyond.
        let mut g = g;
        g.kind = GradientKind::Radial;
        let pm = draw(&g, 100, 50);
        let px = |x, y| pm.pixel(x, y).unwrap();
        assert!(px(50, 25).red() > 245);
        assert!(px(0, 25).blue() > 240 && px(50, 0).blue() > 240);
        assert!(px(0, 0).blue() > 250);
    }

    #[test]
    fn opacity_per_stop() {
        let g = two([255, 0, 0, 255], [255, 0, 0, 0]);
        assert!(!g.is_opaque());
        let pm = draw(&g, 100, 4);
        assert!(pm.pixel(0, 2).unwrap().alpha() > 250);
        assert!(pm.pixel(99, 2).unwrap().alpha() < 8);
    }

    #[test]
    fn saved_with_defaults_for_what_is_missing() {
        let g: Gradient = serde_json::from_str(
            r##"{"kind":"radial","stops":[{"pos":0,"color":[1,2,3,255]},{"pos":1,"color":[4,5,6,255]}]}"##,
        )
        .unwrap();
        assert_eq!(g.center, [0.5, 0.5]);
        assert_eq!(g.radius, 1.0);
        let back: Gradient = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
    }
}
