//! Document model. All geometry is in page millimetres: origin at the
//! top-left corner of the page, x to the right, y down.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A 2D point or vector, usually in page millimetres.
pub type P2 = [f32; 2];

/// 2D affine transform: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Moves by (x, y).
    pub fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    /// Scales x by `sx` and y by `sy`.
    pub fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }

    /// Rotation by `rad`, clockwise on screen because y points down.
    pub fn rotate(rad: f32) -> Self {
        let (s, c) = rad.sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    /// `self.then(other)` applies `self` first, then `other`.
    pub fn then(self, o: Self) -> Self {
        Self {
            a: o.a * self.a + o.c * self.b,
            b: o.b * self.a + o.d * self.b,
            c: o.a * self.c + o.c * self.d,
            d: o.b * self.c + o.d * self.d,
            e: o.a * self.e + o.c * self.f + o.e,
            f: o.b * self.e + o.d * self.f + o.f,
        }
    }

    /// Maps a point.
    pub fn apply(&self, p: P2) -> P2 {
        [
            self.a * p[0] + self.c * p[1] + self.e,
            self.b * p[0] + self.d * p[1] + self.f,
        ]
    }

    /// The reverse transform; `None` if it flattens the plane.
    pub fn inverse(&self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        Some(Self {
            a: self.d * inv,
            b: -self.b * inv,
            c: -self.c * inv,
            d: self.a * inv,
            e: (self.c * self.f - self.d * self.e) * inv,
            f: (self.b * self.e - self.a * self.f) * inv,
        })
    }
}

/// 2D projective transform (homography), row-major 3x3 with `m[8]` usually 1.
/// Maps `(x, y)` to `((m0 x + m1 y + m2) / w, (m3 x + m4 y + m5) / w)`, `w = m6 x + m7 y +
/// m8`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography {
    /// Row-major matrix. f64 keeps strong perspective precise.
    pub m: [f64; 9],
}

impl From<Affine> for Homography {
    fn from(a: Affine) -> Self {
        let [a, b, c, d, e, f] = [a.a, a.b, a.c, a.d, a.e, a.f].map(f64::from);
        Self {
            m: [a, c, e, b, d, f, 0.0, 0.0, 1.0],
        }
    }
}

impl Homography {
    /// Maps the unit square's corners (0,0), (1,0), (1,1), (0,1) onto `q`, in that order.
    pub fn unit_square_to(q: [P2; 4]) -> Self {
        let [x0, x1, x2, x3] = q.map(|p| p[0] as f64);
        let [y0, y1, y2, y3] = q.map(|p| p[1] as f64);
        let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
        let (g, h) = if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            (0.0, 0.0)
        } else {
            let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
            let det = dx1 * dy2 - dx2 * dy1;
            if det.abs() < 1e-12 {
                (0.0, 0.0)
            } else {
                ((sx * dy2 - dx2 * sy) / det, (dx1 * sy - sx * dy1) / det)
            }
        };
        Self {
            m: [
                x1 - x0 + g * x1,
                x3 - x0 + h * x3,
                x0,
                y1 - y0 + g * y1,
                y3 - y0 + h * y3,
                y0,
                g,
                h,
                1.0,
            ],
        }
    }

    /// `self.then(other)` applies `self` first, then `other`.
    pub fn then(self, o: Self) -> Self {
        let (a, b) = (o.m, self.m);
        let mut m = [0.0; 9];
        for r in 0..3 {
            for c in 0..3 {
                m[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
            }
        }
        Self { m }
    }

    /// Applies `self`, then the affine map `a`.
    pub fn then_affine(self, a: Affine) -> Self {
        self.then(a.into())
    }

    /// Maps a point, dividing by the perspective term.
    pub fn apply(&self, p: P2) -> P2 {
        let m = &self.m;
        let (x, y) = (p[0] as f64, p[1] as f64);
        let w = m[6] * x + m[7] * y + m[8];
        [
            ((m[0] * x + m[1] * y + m[2]) / w) as f32,
            ((m[3] * x + m[4] * y + m[5]) / w) as f32,
        ]
    }

    /// The reverse map; `None` if degenerate.
    pub fn inverse(&self) -> Option<Self> {
        let [a, b, c, d, e, f, g, h, i] = self.m;
        let co = [
            e * i - f * h,
            c * h - b * i,
            b * f - c * e,
            f * g - d * i,
            a * i - c * g,
            c * d - a * f,
            d * h - e * g,
            b * g - a * h,
            a * e - b * d,
        ];
        let det = a * co[0] + b * co[3] + c * co[6];
        if det.abs() < 1e-18 || !det.is_finite() {
            return None;
        }
        Some(Self {
            m: co.map(|v| v / det),
        })
    }

    /// The affine part, if there is no perspective.
    pub fn as_affine(&self) -> Option<Affine> {
        let m = self.m.map(|v| v / self.m[8]);
        (m[6].abs() < 1e-9 && m[7].abs() < 1e-9).then(|| Affine {
            a: m[0] as f32,
            b: m[3] as f32,
            c: m[1] as f32,
            d: m[4] as f32,
            e: m[2] as f32,
            f: m[5] as f32,
        })
    }
}

/// True if the quad (in order) is strictly convex, so it can be the image of a rectangle.
pub fn is_convex(q: [P2; 4]) -> bool {
    let cross = |i: usize| {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
    };
    let s = [cross(0), cross(1), cross(2), cross(3)];
    s.iter().all(|&v| v > 1e-6) || s.iter().all(|&v| v < -1e-6)
}

/// Paper size of the printed page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSize {
    #[default]
    A4,
    A3,
}

impl PageSize {
    pub const ALL: [PageSize; 2] = [PageSize::A4, PageSize::A3];

    /// Width and height in portrait orientation.
    pub fn portrait_mm(self) -> P2 {
        match self {
            PageSize::A4 => [210.0, 297.0],
            PageSize::A3 => [297.0, 420.0],
        }
    }

    /// Name with dimensions, for the page menu.
    pub fn label(self) -> &'static str {
        match self {
            PageSize::A4 => "A4 (210 x 297 mm)",
            PageSize::A3 => "A3 (297 x 420 mm)",
        }
    }
}

/// Page orientation; landscape swaps the paper's width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Landscape,
    Portrait,
}

/// Placement of an image layer on the page.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Centre of the image on the page, in mm.
    pub center: P2,
    /// Millimetres per image pixel, along the image's x and y axes.
    pub scale: P2,
    /// Clockwise rotation in degrees.
    pub rotation_deg: f32,
    /// Mirrored left to right, in the image's own frame.
    pub flip_x: bool,
    /// Mirrored top to bottom, in the image's own frame.
    pub flip_y: bool,
    /// Corner offsets for perspective distortion, as fractions of the image's width and
    /// height in its own frame: top-left, top-right, bottom-right, bottom-left. Zero when
    /// undistorted, so moving, scaling, rotating and flipping keep the distortion.
    #[serde(default, skip_serializing_if = "is_undistorted")]
    pub distort: [P2; 4],
}

/// No corner has been moved.
fn is_undistorted(d: &[P2; 4]) -> bool {
    *d == [[0.0; 2]; 4]
}

/// Corners of the unit square, in the order of `Transform::distort`.
pub const UNIT_CORNERS: [P2; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

impl Default for Transform {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            scale: [1.0, 1.0],
            rotation_deg: 0.0,
            flip_x: false,
            flip_y: false,
            distort: [[0.0; 2]; 4],
        }
    }
}

impl Transform {
    /// At least one corner has been moved.
    pub fn is_distorted(&self) -> bool {
        !is_undistorted(&self.distort)
    }

    /// The distorted corners in unit-square coordinates.
    pub fn unit_quad(&self) -> [P2; 4] {
        std::array::from_fn(|i| {
            [
                UNIT_CORNERS[i][0] + self.distort[i][0],
                UNIT_CORNERS[i][1] + self.distort[i][1],
            ]
        })
    }

    /// Maps image pixel coordinates to page millimetres, including the distortion.
    pub fn to_homography(self, size_px: [u32; 2]) -> Homography {
        let affine = self.to_affine(size_px);
        if !self.is_distorted() {
            return affine.into();
        }
        let (w, h) = (size_px[0].max(1) as f32, size_px[1].max(1) as f32);
        Homography::from(Affine::scale(1.0 / w, 1.0 / h))
            .then(Homography::unit_square_to(self.unit_quad()))
            .then_affine(Affine::scale(w, h).then(affine))
    }

    /// Maps image pixel coordinates (0..w, 0..h) to page millimetres.
    pub fn to_affine(self, size_px: [u32; 2]) -> Affine {
        let (w, h) = (size_px[0] as f32, size_px[1] as f32);
        let fx = if self.flip_x { -1.0 } else { 1.0 };
        let fy = if self.flip_y { -1.0 } else { 1.0 };
        Affine::translate(-w / 2.0, -h / 2.0)
            .then(Affine::scale(self.scale[0] * fx, self.scale[1] * fy))
            .then(Affine::rotate(self.rotation_deg.to_radians()))
            .then(Affine::translate(self.center[0], self.center[1]))
    }
}

/// An image placed on the page: a leaf of the layer tree.
#[derive(Clone, Serialize, Deserialize)]
pub struct Layer {
    /// Unique in the project, shared with groups; never reused.
    pub id: u64,
    /// Shown in the layer list.
    pub name: String,
    /// Image file. Saved relative to the project file when it's in the same folder.
    pub source: PathBuf,
    /// Pixel size of the source image, kept so a missing file still has a footprint.
    pub size_px: [u32; 2],
    /// Drawn and exported.
    pub visible: bool,
    /// Can't be picked or moved on the canvas.
    pub locked: bool,
    /// 0 (transparent) to 1 (opaque).
    pub opacity: f32,
    /// Placement on the page.
    pub transform: Transform,

    /// The image file's original bytes, saved into project archives as they are.
    #[serde(skip)]
    pub data: Option<Arc<Vec<u8>>>,
    /// Full-resolution pixels, premultiplied, used for export.
    #[serde(skip)]
    pub pixmap: Option<Arc<tiny_skia::Pixmap>>,
    /// Smaller premultiplied copy, used to preview semi-transparent groups.
    #[serde(skip)]
    pub preview: Option<Arc<tiny_skia::Pixmap>>,
    /// Copy uploaded to the GPU for the canvas, at most 4096 px.
    #[serde(skip)]
    pub texture: Option<egui::TextureHandle>,
    /// The image file couldn't be loaded.
    #[serde(skip)]
    pub broken: bool,
}

impl Layer {
    /// A visible, unlocked layer; its image is loaded separately.
    pub fn new(id: u64, name: String, source: PathBuf) -> Self {
        Self {
            id,
            name,
            source,
            size_px: [1, 1],
            visible: true,
            locked: false,
            opacity: 1.0,
            transform: Transform::default(),
            data: None,
            pixmap: None,
            preview: None,
            texture: None,
            broken: false,
        }
    }

    /// Image pixels to page mm, without the perspective distortion.
    pub fn affine(&self) -> Affine {
        self.transform.to_affine(self.size_px)
    }

    /// Image pixels to page mm, including the perspective distortion.
    pub fn homography(&self) -> Homography {
        self.transform.to_homography(self.size_px)
    }

    /// Corners in page mm: top-left, top-right, bottom-right, bottom-left.
    pub fn corners(&self) -> [P2; 4] {
        let m = self.affine();
        let (w, h) = (self.size_px[0] as f32, self.size_px[1] as f32);
        self.transform
            .unit_quad()
            .map(|q| m.apply([q[0] * w, q[1] * h]))
    }

    /// Page mm to unit-square coordinates of the undistorted image frame.
    pub fn page_to_unit(&self, p: P2) -> Option<P2> {
        let q = self.affine().inverse()?.apply(p);
        let (w, h) = (self.size_px[0].max(1) as f32, self.size_px[1].max(1) as f32);
        Some([q[0] / w, q[1] / h])
    }

    /// True if the page-mm point lies inside the layer's image.
    pub fn contains(&self, p: P2) -> bool {
        let Some(inv) = self.homography().inverse() else {
            return false;
        };
        let q = inv.apply(p);
        q[0] >= 0.0
            && q[1] >= 0.0
            && q[0] <= self.size_px[0] as f32
            && q[1] <= self.size_px[1] as f32
    }

    /// Size on the page in mm (unrotated width and height).
    pub fn size_mm(&self) -> P2 {
        [
            self.size_px[0] as f32 * self.transform.scale[0],
            self.size_px[1] as f32 * self.transform.scale[1],
        ]
    }

    /// Scale uniformly so the image covers (`cover = true`) or fits inside an area.
    pub fn fit_to(&mut self, center: P2, size: P2, cover: bool) {
        let (w, h) = (self.size_px[0].max(1) as f32, self.size_px[1].max(1) as f32);
        let (sx, sy) = (size[0] / w, size[1] / h);
        let s = if cover { sx.max(sy) } else { sx.min(sy) };
        self.transform.center = center;
        self.transform.scale = [s, s];
        self.transform.rotation_deg = 0.0;
    }
}

/// Serde default for flags that are true unless saved otherwise.
fn yes() -> bool {
    true
}

/// A folder of layers. Its opacity applies to the group as a whole: the children are
/// composited together first, then blended onto what lies below.
#[derive(Clone, Serialize, Deserialize)]
pub struct Group {
    /// Unique in the project, shared with layers.
    pub id: u64,
    /// Shown in the layer list.
    pub name: String,
    /// Hides all its children when off.
    pub visible: bool,
    /// Locks all its children.
    #[serde(default)]
    pub locked: bool,
    /// Applied to the group as a whole, 0 to 1.
    pub opacity: f32,
    /// Children shown in the layer list.
    #[serde(default = "yes")]
    pub expanded: bool,
    /// Bottom of the stack first.
    pub children: Vec<Node>,
}

/// An entry in the layer stack. Untagged so that projects written before groups existed
/// (a plain list of layers) still load: only groups have `children`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Group(Group),
    Layer(Layer),
}

impl Node {
    /// Id of the layer or group.
    pub fn id(&self) -> u64 {
        match self {
            Node::Group(g) => g.id,
            Node::Layer(l) => l.id,
        }
    }

    /// Its own visibility flag, ignoring parent groups.
    pub fn visible(&self) -> bool {
        match self {
            Node::Group(g) => g.visible,
            Node::Layer(l) => l.visible,
        }
    }

    /// Its own lock flag, ignoring parent groups.
    pub fn locked(&self) -> bool {
        match self {
            Node::Group(g) => g.locked,
            Node::Layer(l) => l.locked,
        }
    }

    /// The group, if this node is one.
    pub fn as_group(&self) -> Option<&Group> {
        match self {
            Node::Group(g) => Some(g),
            Node::Layer(_) => None,
        }
    }

    /// The group, if this node is one.
    pub fn as_group_mut(&mut self) -> Option<&mut Group> {
        match self {
            Node::Group(g) => Some(g),
            Node::Layer(_) => None,
        }
    }

    /// The image layer, if this node is one.
    pub fn as_layer(&self) -> Option<&Layer> {
        match self {
            Node::Layer(l) => Some(l),
            Node::Group(_) => None,
        }
    }

    /// The image layer, if this node is one.
    pub fn as_layer_mut(&mut self) -> Option<&mut Layer> {
        match self {
            Node::Layer(l) => Some(l),
            Node::Group(_) => None,
        }
    }

    /// Image layers in this node and below, bottom of the stack first.
    pub fn for_each_layer(&self, f: &mut dyn FnMut(&Layer)) {
        match self {
            Node::Layer(l) => f(l),
            Node::Group(g) => g.children.iter().for_each(|c| c.for_each_layer(f)),
        }
    }

    /// Mutable version of `for_each_layer`.
    pub fn for_each_layer_mut(&mut self, f: &mut dyn FnMut(&mut Layer)) {
        match self {
            Node::Layer(l) => f(l),
            Node::Group(g) => g.children.iter_mut().for_each(|c| c.for_each_layer_mut(f)),
        }
    }

    /// Calls `f` on every node in this subtree, this one first.
    pub fn for_each_node_mut(&mut self, f: &mut dyn FnMut(&mut Node)) {
        f(self);
        if let Node::Group(g) = self {
            g.children.iter_mut().for_each(|c| c.for_each_node_mut(f));
        }
    }
}

/// An image layer as seen through its parent groups.
pub struct Leaf<'a> {
    /// The image layer itself.
    pub layer: &'a Layer,
    /// Index path from the root list down to the layer.
    pub path: Vec<usize>,
    /// The layer and all its parent groups are visible.
    pub visible: bool,
    /// The layer or one of its parent groups is locked.
    pub locked: bool,
}

/// How a template outline sits on the page. Rotation is applied first, then the
/// flips, which therefore mirror the outline as seen on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplatePlacement {
    /// Clockwise rotation in degrees: 0, 90, 180 or 270.
    pub rotation: u32,
    /// Mirror left/right.
    pub flip_x: bool,
    /// Mirror top/bottom.
    pub flip_y: bool,
    /// Offset of the template centre from the page centre, in mm.
    pub offset_mm: P2,
}

impl TemplatePlacement {
    /// Rotation that matches the template's long side to the page's long side.
    pub fn auto(tpl_size: P2, page: P2) -> Self {
        let same = (tpl_size[0] >= tpl_size[1]) == (page[0] >= page[1]);
        Self {
            rotation: if same { 0 } else { 90 },
            ..Self::default()
        }
    }
}

/// A project: page setup, template choice and the layer tree. Saved as JSON.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Document {
    /// Paper size.
    pub page: PageSize,
    /// Portrait or landscape.
    pub orientation: Orientation,
    /// Name of the template folder, e.g. "cosmox - T16".
    pub template: Option<String>,
    /// How the template sits on the page.
    pub template_placement: TemplatePlacement,
    /// Page colour (RGB), printed under the layers.
    pub background: [u8; 3],
    /// Layers and groups, bottom of the stack first.
    pub layers: Vec<Node>,
    /// Next id for a new layer or group.
    pub next_id: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            page: PageSize::A4,
            orientation: Orientation::Landscape,
            template: None,
            template_placement: TemplatePlacement::default(),
            background: [255, 255, 255],
            layers: Vec::new(),
            next_id: 1,
        }
    }
}

impl Document {
    /// Page width and height in mm, orientation applied.
    pub fn page_mm(&self) -> P2 {
        let [w, h] = self.page.portrait_mm();
        match self.orientation {
            Orientation::Portrait => [w, h],
            Orientation::Landscape => [h, w],
        }
    }

    /// Centre of the page in mm.
    pub fn page_center(&self) -> P2 {
        let [w, h] = self.page_mm();
        [w / 2.0, h / 2.0]
    }

    /// A fresh id for a new layer or group.
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Index path from the root list down to the node with `id`.
    pub fn path_of(&self, id: u64) -> Option<Vec<usize>> {
        fn find(nodes: &[Node], id: u64, path: &mut Vec<usize>) -> bool {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                if n.id() == id {
                    return true;
                }
                if let Node::Group(g) = n
                    && find(&g.children, id, path)
                {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        find(&self.layers, id, &mut path).then_some(path)
    }

    /// The node at an index path.
    pub fn node_at(&self, path: &[usize]) -> Option<&Node> {
        let (&first, rest) = path.split_first()?;
        rest.iter().try_fold(self.layers.get(first)?, |n, &i| {
            n.as_group()?.children.get(i)
        })
    }

    /// Mutable version of `node_at`.
    pub fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let (&first, rest) = path.split_first()?;
        rest.iter().try_fold(self.layers.get_mut(first)?, |n, &i| {
            n.as_group_mut()?.children.get_mut(i)
        })
    }

    /// The list holding the children of the group at `parent`, or the root list.
    pub fn list_mut(&mut self, parent: &[usize]) -> Option<&mut Vec<Node>> {
        if parent.is_empty() {
            Some(&mut self.layers)
        } else {
            Some(&mut self.node_at_mut(parent)?.as_group_mut()?.children)
        }
    }

    /// Read-only version of `list_mut`.
    pub fn list(&self, parent: &[usize]) -> Option<&Vec<Node>> {
        if parent.is_empty() {
            Some(&self.layers)
        } else {
            Some(&self.node_at(parent)?.as_group()?.children)
        }
    }

    /// The node with `id`, anywhere in the tree.
    pub fn node(&self, id: u64) -> Option<&Node> {
        self.node_at(&self.path_of(id)?)
    }

    /// Mutable version of `node`.
    pub fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        let path = self.path_of(id)?;
        self.node_at_mut(&path)
    }

    /// The image layer with `id`; `None` for groups.
    pub fn layer(&self, id: u64) -> Option<&Layer> {
        self.node(id)?.as_layer()
    }

    /// Mutable version of `layer`.
    pub fn layer_mut(&mut self, id: u64) -> Option<&mut Layer> {
        self.node_mut(id)?.as_layer_mut()
    }

    /// Removes a node and returns it with the path it had.
    pub fn remove(&mut self, id: u64) -> Option<(Vec<usize>, Node)> {
        let path = self.path_of(id)?;
        let (&i, parent) = path.split_last()?;
        let node = self.list_mut(parent)?.remove(i);
        Some((path, node))
    }

    /// Inserts a node so that it ends up at `path` (parent path + index, clamped).
    pub fn insert(&mut self, path: &[usize], node: Node) {
        let (&i, parent) = path.split_last().expect("non-empty path");
        let list = self.list_mut(parent).expect("parent is a group");
        list.insert(i.min(list.len()), node);
    }

    /// True if `id` is `ancestor` or lies inside it.
    pub fn is_within(&self, id: u64, ancestor: u64) -> bool {
        match (self.path_of(id), self.path_of(ancestor)) {
            (Some(p), Some(a)) => p.starts_with(&a),
            _ => false,
        }
    }

    /// The node is visible and so are all its parent groups.
    pub fn effectively_visible(&self, id: u64) -> bool {
        self.ancestry(id)
            .is_some_and(|nodes| nodes.iter().all(|n| n.visible()))
    }

    /// The node or one of its parent groups is locked.
    pub fn effectively_locked(&self, id: u64) -> bool {
        self.ancestry(id)
            .is_none_or(|nodes| nodes.iter().any(|n| n.locked()))
    }

    /// The node and its parent groups, outermost first.
    fn ancestry(&self, id: u64) -> Option<Vec<&Node>> {
        let path = self.path_of(id)?;
        (1..=path.len()).map(|k| self.node_at(&path[..k])).collect()
    }

    /// Every image layer in paint order (bottom first), with its inherited state.
    pub fn leaves(&self) -> Vec<Leaf<'_>> {
        fn walk<'a>(
            nodes: &'a [Node],
            path: &mut Vec<usize>,
            visible: bool,
            locked: bool,
            out: &mut Vec<Leaf<'a>>,
        ) {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                match n {
                    Node::Layer(l) => out.push(Leaf {
                        layer: l,
                        path: path.clone(),
                        visible: visible && l.visible,
                        locked: locked || l.locked,
                    }),
                    Node::Group(g) => walk(
                        &g.children,
                        path,
                        visible && g.visible,
                        locked || g.locked,
                        out,
                    ),
                }
                path.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.layers, &mut Vec::new(), true, false, &mut out);
        out
    }

    /// Calls `f` on every image layer in the project.
    pub fn for_each_layer_mut(&mut self, mut f: impl FnMut(&mut Layer)) {
        for n in &mut self.layers {
            n.for_each_layer_mut(&mut f);
        }
    }

    /// Maps template mm (origin at its top-left, size `tpl_size`) to page mm.
    pub fn template_affine(&self, tpl_size: P2) -> Affine {
        let c = self.page_center();
        let t = &self.template_placement;
        let fx = if t.flip_x { -1.0 } else { 1.0 };
        let fy = if t.flip_y { -1.0 } else { 1.0 };
        Affine::translate(-tpl_size[0] / 2.0, -tpl_size[1] / 2.0)
            .then(Affine::rotate((t.rotation as f32).to_radians()))
            .then(Affine::scale(fx, fy))
            .then(Affine::translate(
                c[0] + t.offset_mm[0],
                c[1] + t.offset_mm[1],
            ))
    }

    /// The project as JSON for the linked format. Image paths under `base` (the project
    /// file's folder) are written relative, with forward slashes, so the project and its
    /// images can move together and open on both Windows and Linux.
    pub fn to_json_relative(&self, base: &Path) -> Result<String, String> {
        let mut doc = self.clone();
        doc.for_each_layer_mut(|layer| {
            if let Ok(rel) = layer.source.strip_prefix(base) {
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                layer.source = PathBuf::from(parts.join("/"));
            }
        });
        serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
    }

    /// Reads a linked JSON project. Image paths come back absolute; images are not
    /// loaded yet.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let mut doc = Self::from_json(&text)?;
        let base = path.parent().unwrap_or(Path::new(""));
        doc.for_each_layer_mut(|layer| {
            if layer.source.is_relative() {
                layer.source = base.join(&layer.source);
            }
        });
        Ok(doc)
    }

    /// Parses a project description, making sure new ids won't clash with saved ones.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut doc: Document =
            serde_json::from_str(text).map_err(|e| format!("Invalid project file: {e}"))?;
        let mut max_id = 0;
        for n in &mut doc.layers {
            n.for_each_node_mut(&mut |n| max_id = max_id.max(n.id()));
        }
        doc.next_id = doc.next_id.max(max_id + 1);
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P2, b: P2) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn affine_inverse_roundtrip() {
        let t = Transform {
            center: [50.0, 20.0],
            scale: [0.2, 0.3],
            rotation_deg: 33.0,
            flip_x: true,
            flip_y: false,
            ..Default::default()
        };
        let m = t.to_affine([400, 300]);
        let inv = m.inverse().unwrap();
        let p = [123.0, 45.0];
        assert!(close(inv.apply(m.apply(p)), p));
        // The image centre lands on the transform centre.
        assert!(close(m.apply([200.0, 150.0]), [50.0, 20.0]));
    }

    #[test]
    fn homography_maps_unit_square_to_quad_and_back() {
        let q = [[10.0, 5.0], [90.0, 20.0], [70.0, 60.0], [20.0, 50.0]];
        let h = Homography::unit_square_to(q);
        for (c, want) in UNIT_CORNERS.iter().zip(q) {
            assert!(close(h.apply(*c), want), "{:?}", h.apply(*c));
        }
        let inv = h.inverse().unwrap();
        let p = [0.3, 0.8];
        assert!(close(inv.apply(h.apply(p)), p));
        assert!(h.as_affine().is_none());
        // A parallelogram needs no perspective.
        let par = Homography::unit_square_to([[0.0, 0.0], [2.0, 1.0], [3.0, 4.0], [1.0, 3.0]]);
        assert!(par.as_affine().is_some());
    }

    #[test]
    fn distorted_layer_corners_and_hit_test() {
        let mut l = Layer::new(1, "l".into(), "x.png".into());
        l.size_px = [100, 50];
        l.transform.center = [50.0, 25.0];
        // Pull the top corners inwards: a trapezoid, narrower at the top.
        l.transform.distort = [[0.2, 0.0], [-0.2, 0.0], [0.0, 0.0], [0.0, 0.0]];
        let c = l.corners();
        assert!(close(c[0], [20.0, 0.0]) && close(c[1], [80.0, 0.0]));
        assert!(close(c[2], [100.0, 50.0]) && close(c[3], [0.0, 50.0]));
        assert!(close(l.homography().apply([100.0, 0.0]), [80.0, 0.0]));
        assert!(l.contains([50.0, 2.0]));
        assert!(!l.contains([10.0, 2.0]), "cut off by the slanted edge");
        assert!(l.contains([5.0, 48.0]));
        // Moving the layer keeps the distortion.
        l.transform.center = [60.0, 25.0];
        assert!(close(l.corners()[0], [30.0, 0.0]));
        // Old projects without the field still load; undistorted ones don't write it.
        let json = serde_json::to_string(&Transform::default()).unwrap();
        assert!(!json.contains("distort"));
        let t: Transform = serde_json::from_str(&json).unwrap();
        assert!(!t.is_distorted());
    }

    #[test]
    fn convexity() {
        assert!(is_convex(UNIT_CORNERS));
        assert!(!is_convex([[0.0, 0.0], [1.0, 0.0], [0.2, 0.2], [0.0, 1.0]]));
        assert!(!is_convex([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]));
    }

    #[test]
    fn page_sizes() {
        let mut d = Document::default();
        assert_eq!(d.page_mm(), [297.0, 210.0]);
        d.page = PageSize::A3;
        d.orientation = Orientation::Portrait;
        assert_eq!(d.page_mm(), [297.0, 420.0]);
    }

    #[test]
    fn auto_rotate_portrait_template_on_landscape_page() {
        let mut d = Document::default();
        d.template_placement = TemplatePlacement::auto([196.0, 296.0], d.page_mm());
        assert_eq!(d.template_placement.rotation, 90);
        let m = d.template_affine([196.0, 296.0]);
        // The template centre maps to the page centre.
        assert!(close(m.apply([98.0, 148.0]), d.page_center()));
    }

    #[test]
    fn flip_y_mirrors_on_screen_after_rotation() {
        let mut d = Document::default();
        let size = [196.0, 296.0];
        d.template_placement = TemplatePlacement {
            rotation: 90,
            ..Default::default()
        };
        let plain = d.template_affine(size).apply([0.0, 0.0]);
        d.template_placement.flip_y = true;
        let flipped = d.template_affine(size).apply([0.0, 0.0]);
        let c = d.page_center();
        // Same x, y mirrored around the page centre.
        assert!((plain[0] - flipped[0]).abs() < 1e-3);
        assert!((plain[1] - c[1] + (flipped[1] - c[1])).abs() < 1e-3);
    }

    #[test]
    fn project_roundtrip_with_relative_paths() {
        let dir = std::env::temp_dir().join(format!("artwork-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("p.artwork.json");
        let mut d = Document::default();
        let id = d.alloc_id();
        d.template = Some("cosmox - T16".into());
        let mut layer = Layer::new(id, "bg".into(), dir.join("img").join("bg.png"));
        layer.size_px = [100, 50];
        layer.opacity = 0.5;
        let gid = d.alloc_id();
        let inner = Layer::new(d.alloc_id(), "inner".into(), dir.join("inner.png"));
        d.layers.push(Node::Layer(layer));
        d.layers.push(Node::Group(Group {
            id: gid,
            name: "g".into(),
            visible: false,
            locked: false,
            opacity: 0.25,
            expanded: true,
            children: vec![Node::Layer(inner)],
        }));
        std::fs::write(&project, d.to_json_relative(&dir).unwrap()).unwrap();
        let text = std::fs::read_to_string(&project).unwrap();
        assert!(
            !text.contains(dir.to_str().unwrap()),
            "path should be stored relative"
        );
        let back = Document::load(&project).unwrap();
        let bg = back.layers[0].as_layer().unwrap();
        assert_eq!(bg.source, dir.join("img").join("bg.png"));
        assert_eq!(bg.opacity, 0.5);
        let g = back.layers[1].as_group().unwrap();
        assert_eq!((g.opacity, g.visible), (0.25, false));
        let inner = g.children[0].as_layer().unwrap();
        assert_eq!(inner.source, dir.join("inner.png"));
        assert_eq!(back.template.as_deref(), Some("cosmox - T16"));
        assert!(back.next_id > inner.id);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn loads_projects_from_before_groups() {
        let json = r#"{"layers":[{"id":3,"name":"a","source":"/x/a.png","size_px":[10,10],
            "visible":true,"locked":false,"opacity":1.0,
            "transform":{"center":[0,0],"scale":[1,1],"rotation_deg":0,"flip_x":false,"flip_y":false}}]}"#;
        let d: Document = serde_json::from_str(json).unwrap();
        assert_eq!(d.layers[0].as_layer().unwrap().id, 3);
    }

    #[test]
    fn tree_paths_and_inherited_state() {
        let mut d = Document::default();
        let a = Layer::new(1, "a".into(), "a.png".into());
        let b = Layer::new(2, "b".into(), "b.png".into());
        d.layers.push(Node::Layer(a));
        d.layers.push(Node::Group(Group {
            id: 3,
            name: "g".into(),
            visible: false,
            locked: true,
            opacity: 1.0,
            expanded: true,
            children: vec![Node::Layer(b)],
        }));
        assert_eq!(d.path_of(2), Some(vec![1, 0]));
        assert!(d.is_within(2, 3) && !d.is_within(1, 3));
        assert!(!d.effectively_visible(2) && d.effectively_visible(1));
        assert!(d.effectively_locked(2) && !d.effectively_locked(1));
        let leaves = d.leaves();
        assert_eq!(leaves.len(), 2);
        assert!(!leaves[1].visible && leaves[1].locked);
        let (path, node) = d.remove(2).unwrap();
        assert_eq!(path, vec![1, 0]);
        d.insert(&[0], node);
        assert_eq!(d.path_of(2), Some(vec![0]));
    }
}
