//! Variables: named values kept with the project, each of a kind ([`Kind`]): a length,
//! an angle, a text size, a plain number or a color.
//!
//! Number fields are linked to formulas such as `button + 2` or `(panel - 2 * margin) / 3`,
//! which use variables of the field's kind and plain numbers. A linked field (`Field`)
//! keeps its formula in the layer's `bindings`. Each frame, [`apply`] sets every linked
//! field to its formula's value, so a changed variable moves or resizes all the layers
//! using it. A field changed some other way (a drag on the canvas, Align, a nudge,
//! another field's aspect lock) is unlinked: it keeps the value it was given. Typing a
//! plain number into a linked field unlinks it too.
//!
//! Color buttons are linked to color variables by name (see `colors`).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use egui::{DragValue, Id, Response, RichText, Ui};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Layer, Node};
use crate::effects::{color_from_hex, color_to_hex};
use crate::i18n::{tr, tr_n};
use crate::shape::{self, ShapeKind};
use crate::text;

/// What a variable holds, which says where it can be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A length in mm: sizes, positions, stroke widths, corner radii.
    Length,
    /// An angle in degrees: rotations.
    Angle,
    /// A text size in pt.
    Size,
    /// A number without a unit, usable in every number field.
    Number,
    /// A color, which color buttons link to.
    Color,
    /// A text, which text layers link to (see `text_links`).
    Text,
}

impl Kind {
    /// Every kind, in the order the Variables section lists them.
    pub const ALL: [Kind; 6] = [
        Kind::Length,
        Kind::Angle,
        Kind::Size,
        Kind::Number,
        Kind::Color,
        Kind::Text,
    ];

    /// The kind's name in the current language, with its unit.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Length => tr!("VARIABLE_KIND_LENGTH"),
            Kind::Angle => tr!("VARIABLE_KIND_ANGLE"),
            Kind::Size => tr!("VARIABLE_KIND_SIZE"),
            Kind::Number => tr!("VARIABLE_KIND_NUMBER"),
            Kind::Color => tr!("VARIABLE_KIND_COLOR"),
            Kind::Text => tr!("VARIABLE_KIND_TEXT"),
        }
    }

    /// Draws the kind's icon, which stands for it in the Variables section: a dimension
    /// line for lengths, an arc between two rays for angles, "Tt" for text sizes, "#" for
    /// numbers and three color dots for colors. Hovering it shows [`Self::label`].
    pub fn icon(self, ui: &mut Ui) -> Response {
        let side = ui.spacing().interact_size.y;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let color = ui.visuals().weak_text_color();
            let stroke = egui::Stroke::new(1.5, color);
            let r = rect.shrink(side * 0.2);
            match self {
                Kind::Length => {
                    let y = r.center().y;
                    painter
                        .line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], stroke);
                    for x in [r.left(), r.right()] {
                        let h = r.height() * 0.3;
                        painter.line_segment([egui::pos2(x, y - h), egui::pos2(x, y + h)], stroke);
                    }
                }
                Kind::Angle => {
                    let o = r.left_bottom();
                    let len = r.width();
                    let a = 50f32.to_radians();
                    let tip = o + len * egui::vec2(a.cos(), -a.sin());
                    painter.line_segment([o, r.right_bottom()], stroke);
                    painter.line_segment([o, tip], stroke);
                    let arc: Vec<_> = (0..=8)
                        .map(|i| {
                            let t = a * i as f32 / 8.0;
                            o + len * 0.55 * egui::vec2(t.cos(), -t.sin())
                        })
                        .collect();
                    painter.add(egui::Shape::line(arc, stroke));
                }
                Kind::Size | Kind::Number | Kind::Text => {
                    let text = match self {
                        Kind::Size => "Tt",
                        Kind::Number => "#",
                        _ => "abc",
                    };
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(side * 0.7),
                        color,
                    );
                }
                Kind::Color => {
                    let d = r.width() * 0.22;
                    let c = r.center();
                    for (k, rgb) in [[230, 70, 70], [70, 180, 90], [70, 120, 230]]
                        .into_iter()
                        .enumerate()
                    {
                        let t = (90.0 + 120.0 * k as f32).to_radians();
                        let [red, green, blue] = rgb;
                        painter.circle_filled(
                            c + d * egui::vec2(t.cos(), -t.sin()),
                            r.width() * 0.3,
                            egui::Color32::from_rgba_unmultiplied(red, green, blue, 200),
                        );
                    }
                }
            }
        }
        response.on_hover_text(self.label())
    }

    /// The unit of a value of this kind, as shown after it: " mm", "°", " pt" or none.
    pub fn unit(self) -> &'static str {
        match self {
            Kind::Length => " mm",
            Kind::Angle => "°",
            Kind::Size => " pt",
            Kind::Number | Kind::Color | Kind::Text => "",
        }
    }

    /// The start of the names new variables of this kind get: "length1", "length2"...
    fn base_name(self) -> &'static str {
        let name = match self {
            Kind::Length => tr!("NEW_VARIABLE_NAME_LENGTH"),
            Kind::Angle => tr!("NEW_VARIABLE_NAME_ANGLE"),
            Kind::Size => tr!("NEW_VARIABLE_NAME_SIZE"),
            Kind::Number => tr!("NEW_VARIABLE_NAME_NUMBER"),
            Kind::Color => tr!("NEW_VARIABLE_NAME_COLOR"),
            Kind::Text => tr!("NEW_VARIABLE_NAME_TEXT"),
        };
        if is_valid_name(name) { name } else { "var" }
    }

    /// The value a new variable of this kind starts with.
    fn new_value(self) -> Value {
        match self {
            Kind::Length => Value::Length(10.0),
            Kind::Angle => Value::Angle(45.0),
            Kind::Size => Value::Size(12.0),
            Kind::Number => Value::Number(1.0),
            Kind::Color => Value::Color([255, 255, 255, 255]),
            Kind::Text => Value::Text(tr!("NEW_TEXT_VARIABLE_VALUE").to_owned()),
        }
    }
}

/// A variable's value, of its kind. Colors are straight RGBA, saved as `#rrggbbaa`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Value {
    Length(f32),
    Angle(f32),
    Size(f32),
    Number(f32),
    Color(#[serde(serialize_with = "hex")] [u8; 4]),
    Text(String),
}

fn hex<S: serde::Serializer>(c: &[u8; 4], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&color_to_hex(*c))
}

impl Value {
    pub fn kind(&self) -> Kind {
        match self {
            Value::Length(_) => Kind::Length,
            Value::Angle(_) => Kind::Angle,
            Value::Size(_) => Kind::Size,
            Value::Number(_) => Kind::Number,
            Value::Color(_) => Kind::Color,
            Value::Text(_) => Kind::Text,
        }
    }

    /// The number of every kind but colors.
    pub fn number(&self) -> Option<f32> {
        match *self {
            Value::Length(v) | Value::Angle(v) | Value::Size(v) | Value::Number(v) => Some(v),
            Value::Color(_) | Value::Text(_) => None,
        }
    }

    fn number_mut(&mut self) -> Option<&mut f32> {
        match self {
            Value::Length(v) | Value::Angle(v) | Value::Size(v) | Value::Number(v) => Some(v),
            Value::Color(_) | Value::Text(_) => None,
        }
    }

    pub fn color(&self) -> Option<[u8; 4]> {
        match *self {
            Value::Color(c) => Some(c),
            _ => None,
        }
    }

    /// The value with its unit, short: `30 mm`, `#ff0000`, or a text's first line, cut
    /// after 24 characters.
    pub fn short_text(&self) -> String {
        match self {
            Value::Color(c) => color_to_hex(*c),
            Value::Text(t) => {
                let line = t.lines().next().unwrap_or_default();
                let mut short: String = line.chars().take(24).collect();
                if short.len() < t.len() {
                    short.push_str("...");
                }
                format!("\"{short}\"")
            }
            _ => format!(
                "{}{}",
                self.number().unwrap_or_default(),
                self.kind().unit()
            ),
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// A named value. Saved as `{"name": "button", "kind": "length", "value": 30}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SavedVariable")]
pub struct Variable {
    pub name: String,
    #[serde(flatten)]
    pub value: Value,
}

/// A variable as read from a project file. Variables of the first projects that had
/// them have no kind: they are numbers.
#[derive(Deserialize)]
struct SavedVariable {
    name: String,
    #[serde(default)]
    kind: Option<Kind>,
    value: serde_json::Value,
}

impl TryFrom<SavedVariable> for Variable {
    type Error = String;

    fn try_from(saved: SavedVariable) -> Result<Self, String> {
        let kind = saved.kind.unwrap_or(Kind::Number);
        let bad = || format!("invalid value for the variable {}", saved.name);
        let number = || saved.value.as_f64().map(|v| v as f32).ok_or_else(bad);
        let value = match kind {
            Kind::Length => Value::Length(number()?),
            Kind::Angle => Value::Angle(number()?),
            Kind::Size => Value::Size(number()?),
            Kind::Number => Value::Number(number()?),
            Kind::Color => Value::Color(
                saved
                    .value
                    .as_str()
                    .and_then(color_from_hex)
                    .ok_or_else(bad)?,
            ),
            Kind::Text => Value::Text(saved.value.as_str().ok_or_else(bad)?.to_owned()),
        };
        Ok(Variable {
            name: saved.name,
            value,
        })
    }
}

/// A name no variable has, for a new variable of this kind: "length1", "length2"...
pub fn new_name(vars: &[Variable], kind: Kind) -> String {
    let base = kind.base_name();
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|n| vars.iter().all(|v| v.name != *n))
        .expect("an unused name")
}

/// A layer field that can be linked to a formula.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Width in mm: of a shape's geometry, or of an image or text layer.
    Width,
    /// Height in mm, as `Width`.
    Height,
    /// Center x on the page, in mm.
    X,
    /// Center y on the page, in mm.
    Y,
    /// A shape's stroke width, in mm.
    StrokeWidth,
    /// A rectangle's corner radius, in mm.
    CornerRadius,
    /// A text's font size, in pt.
    TextSize,
    /// Clockwise rotation, in degrees.
    Rotation,
}

impl Field {
    /// The kind of variables the field takes, besides numbers.
    pub fn kind(self) -> Kind {
        match self {
            Field::Width
            | Field::Height
            | Field::X
            | Field::Y
            | Field::StrokeWidth
            | Field::CornerRadius => Kind::Length,
            Field::TextSize => Kind::Size,
            Field::Rotation => Kind::Angle,
        }
    }
}

/// Formulas of a layer's linked fields.
pub type Bindings = BTreeMap<Field, String>;

/// The value each linked field was last given by [`apply`], by layer id: a field found
/// with another value was changed by hand. Cleared whenever the document is replaced
/// (undo, redo, opening a project).
pub type Applied = HashMap<(u64, Field), f32>;

// ---- Formulas ----

/// Why a formula has no value.
#[derive(Clone, Debug, PartialEq)]
pub enum FormulaError {
    Syntax,
    UnknownVariable(String),
    /// A variable of another kind than the field's (`kind`) or a number.
    WrongKind {
        name: String,
        kind: Kind,
    },
    DivisionByZero,
}

impl FormulaError {
    /// The error in the current language.
    pub fn message(&self) -> String {
        match self {
            FormulaError::Syntax => tr!("FORMULA_SYNTAX_ERROR").to_owned(),
            FormulaError::UnknownVariable(name) => tr!("FORMULA_UNKNOWN_VARIABLE", name = name),
            FormulaError::WrongKind { name, kind } => tr!(
                "FORMULA_WRONG_KIND",
                name = name,
                kind = kind.label(),
                number = Kind::Number.label()
            ),
            FormulaError::DivisionByZero => tr!("FORMULA_DIVISION_BY_ZERO").to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(char),
}

/// True for a character that may start a variable name.
fn name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// True for a character that may follow in a variable name.
fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True if `name` can be a variable's name: a letter or `_`, then letters, digits, `_`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(name_start) && chars.all(name_char)
}

fn tokenize(s: &str) -> Result<Vec<Token>, FormulaError> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c.is_ascii_digit() || c == '.' {
            let mut num = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit() || **d == '.') {
                num.push(d);
                chars.next();
            }
            out.push(Token::Num(num.parse().map_err(|_| FormulaError::Syntax)?));
        } else if name_start(c) {
            let mut name = String::new();
            while let Some(&d) = chars.peek().filter(|d| name_char(**d)) {
                name.push(d);
                chars.next();
            }
            out.push(Token::Name(name));
        } else if "+-*/()".contains(c) || c == '−' {
            out.push(Token::Op(if c == '−' { '-' } else { c }));
            chars.next();
        } else {
            return Err(FormulaError::Syntax);
        }
    }
    Ok(out)
}

/// Recursive descent over the tokens: sums of products of signed factors.
struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    vars: &'a [Variable],
    /// The kind of variables allowed besides numbers.
    kind: Kind,
}

impl Parser<'_> {
    fn peek_op(&self) -> Option<char> {
        match self.tokens.get(self.at) {
            Some(Token::Op(c)) => Some(*c),
            _ => None,
        }
    }

    fn sum(&mut self) -> Result<f64, FormulaError> {
        let mut v = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek_op() {
            self.at += 1;
            let rhs = self.product()?;
            v = if op == '+' { v + rhs } else { v - rhs };
        }
        Ok(v)
    }

    fn product(&mut self) -> Result<f64, FormulaError> {
        let mut v = self.factor()?;
        while let Some(op @ ('*' | '/')) = self.peek_op() {
            self.at += 1;
            let rhs = self.factor()?;
            v = if op == '*' {
                v * rhs
            } else if rhs == 0.0 {
                return Err(FormulaError::DivisionByZero);
            } else {
                v / rhs
            };
        }
        Ok(v)
    }

    fn factor(&mut self) -> Result<f64, FormulaError> {
        let token = self.tokens.get(self.at).ok_or(FormulaError::Syntax)?;
        self.at += 1;
        match token {
            Token::Num(n) => Ok(*n),
            Token::Name(name) => {
                let var = self
                    .vars
                    .iter()
                    .find(|v| v.name == *name)
                    .ok_or_else(|| FormulaError::UnknownVariable(name.clone()))?;
                let kind = var.value.kind();
                match var.value.number() {
                    Some(v) if kind == self.kind || kind == Kind::Number => Ok(v as f64),
                    _ => Err(FormulaError::WrongKind {
                        name: name.clone(),
                        kind: self.kind,
                    }),
                }
            }
            Token::Op('-') => Ok(-self.factor()?),
            Token::Op('+') => self.factor(),
            Token::Op('(') => {
                let v = self.sum()?;
                (self.peek_op() == Some(')'))
                    .then(|| self.at += 1)
                    .ok_or(FormulaError::Syntax)?;
                Ok(v)
            }
            Token::Op(_) => Err(FormulaError::Syntax),
        }
    }
}

/// The value of `formula` with these variables, for a field taking variables of `kind`.
pub fn eval(formula: &str, vars: &[Variable], kind: Kind) -> Result<f64, FormulaError> {
    let tokens = tokenize(formula)?;
    let mut p = Parser {
        tokens: &tokens,
        at: 0,
        vars,
        kind,
    };
    let v = p.sum()?;
    if p.at != tokens.len() || !v.is_finite() {
        return Err(FormulaError::Syntax);
    }
    Ok(v)
}

/// True if `formula` names a variable, so that it links a field rather than just
/// computing a number.
fn uses_variables(formula: &str) -> bool {
    tokenize(formula).is_ok_and(|t| t.iter().any(|t| matches!(t, Token::Name(_))))
}

/// True if `formula` names the variable `name`.
fn uses(formula: &str, name: &str) -> bool {
    tokenize(formula).is_ok_and(|t| t.contains(&Token::Name(name.to_owned())))
}

/// `formula` with the variable `old` renamed `new`, the rest as typed.
fn rename_in(formula: &str, old: &str, new: &str) -> String {
    let mut out = String::with_capacity(formula.len());
    let mut chars = formula.char_indices().peekable();
    let mut prev_is_name = false;
    while let Some((i, c)) = chars.next() {
        // A name, unless it continues a number such as `2e`.
        if name_start(c) && !prev_is_name {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek().filter(|(_, d)| name_char(*d)) {
                end = j + d.len_utf8();
                chars.next();
            }
            let word = &formula[i..end];
            out.push_str(if word == old { new } else { word });
            prev_is_name = false;
        } else {
            out.push(c);
            prev_is_name = name_char(c);
        }
    }
    out
}

// ---- Fields ----

/// The number a field is stored as, compared to tell a change by hand: the scale for
/// the size of an image or text, the size itself for a shape. `None` if the layer has
/// no such field.
fn raw(layer: &Layer, field: Field) -> Option<f32> {
    let shape = layer.shape.as_deref();
    match field {
        Field::Width | Field::Height => {
            let a = (field == Field::Height) as usize;
            Some(match shape {
                Some(s) => s.geometry_size()[a],
                None => layer.transform.scale[a],
            })
        }
        Field::X => Some(layer.transform.center[0]),
        Field::Y => Some(layer.transform.center[1]),
        Field::StrokeWidth => shape.map(|s| s.stroke_width),
        Field::CornerRadius => shape
            .filter(|s| s.kind == ShapeKind::Rectangle)
            .map(|s| s.corner_radius),
        Field::TextSize => layer.text.is_some().then(|| text::size_pt(layer)),
        Field::Rotation => Some(layer.transform.rotation_deg),
    }
}

/// `deg` brought into the rotation field's range, -180 to 180.
pub fn wrap_degrees(deg: f32) -> f32 {
    let d = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if d == -180.0 && deg > 0.0 { 180.0 } else { d }
}

/// Sets `field` to the value `v` (in the field's unit). Returns its new raw number.
fn set(layer: &mut Layer, field: Field, v: f32) -> Option<f32> {
    raw(layer, field)?;
    match field {
        Field::Width | Field::Height => {
            let a = (field == Field::Height) as usize;
            let v = v.max(0.01);
            match layer.shape.as_deref() {
                Some(s) => {
                    let mut size = s.geometry_size();
                    size[a] = v;
                    shape::set_geometry_size(layer, size);
                }
                None => layer.transform.scale[a] = v / layer.size_px[a].max(1) as f32,
            }
        }
        Field::X => layer.transform.center[0] = v,
        Field::Y => layer.transform.center[1] = v,
        Field::StrokeWidth => layer.shape.as_deref_mut()?.stroke_width = v.clamp(0.01, 100.0),
        Field::CornerRadius => {
            let s = layer.shape.as_deref_mut()?;
            s.corner_radius = v.clamp(0.0, s.size[0].min(s.size[1]) / 2.0);
        }
        Field::TextSize => text::set_size_pt(layer, v.clamp(1.0, 2000.0)),
        Field::Rotation => layer.transform.rotation_deg = wrap_degrees(v),
    }
    raw(layer, field)
}

/// True if two raw numbers are the same but for rounding.
fn same(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
}

/// Sets every linked field to its formula's value, and unlinks those changed by hand
/// since the last call. A formula with an error leaves its field as it is. Returns true
/// if the document changed.
pub fn apply(doc: &mut Document, applied: &mut Applied) -> bool {
    let vars = doc.variables.clone();
    let mut changed = false;
    doc.for_each_layer_mut(|layer| {
        if layer.bindings.is_empty() {
            return;
        }
        let id = layer.id;
        let fields: Vec<Field> = layer.bindings.keys().copied().collect();
        for field in fields {
            let Some(now) = raw(layer, field) else {
                continue;
            };
            if applied
                .get(&(id, field))
                .is_some_and(|&last| !same(last, now))
            {
                layer.bindings.remove(&field);
                applied.remove(&(id, field));
                changed = true;
                continue;
            }
            let Ok(v) = eval(&layer.bindings[&field], &vars, field.kind()) else {
                continue;
            };
            let mut probe = layer.clone();
            let Some(target) = set(&mut probe, field, v as f32) else {
                continue;
            };
            if !same(now, target) {
                set(layer, field, v as f32);
                changed = true;
            }
            applied.insert((id, field), raw(layer, field).unwrap_or(target));
        }
    });
    changed
}

// ---- Interface ----

fn vars_id() -> Id {
    Id::unique("project_variables")
}

/// Makes the project's variables available to the fields drawn this frame.
pub fn publish(ctx: &egui::Context, doc: &Document) {
    let vars = Arc::new(doc.variables.clone());
    ctx.data_mut(|d| d.insert_temp(vars_id(), vars));
}

pub fn published(ctx: &egui::Context) -> Arc<Vec<Variable>> {
    ctx.data(|d| d.get_temp(vars_id())).unwrap_or_default()
}

/// A number field that can be linked to a formula, with an "fx" button inside it, at
/// its right end, which opens the formula editor ([`formula_editor`]). It is the
/// `DragValue` made by `make`, which also takes a formula: one naming a variable links
/// the field. Linked, it shows the formula's result (the formula is in its tooltip and
/// the editor), its fx button filled with the link color; a number typed in or dragged
/// unlinks it. Linked to a variable that no longer exists (removed), it keeps its last value,
/// its fx button in the warning color: a value typed or dragged unlinks it, a variable
/// of that name links it again. The unit is left to the caller, after the field (see
/// `layers_ui::unit`). The response is the value's, changed when `value` was changed
/// here; [`apply`] keeps linked fields up to date.
pub fn field<'a>(
    ui: &mut Ui,
    bindings: &mut Bindings,
    field: Field,
    value: &'a mut f32,
    make: impl FnOnce(DragValue<'a>) -> DragValue<'a>,
) -> Response {
    let vars = published(ui.ctx());
    let kind = field.kind();
    let formula_id = ui.auto_id_with(("formula", field));
    let link = match bindings.get(&field) {
        None => Link::None,
        Some(formula) => match eval(formula, &vars, kind) {
            Ok(v) => Link::Linked(tr!(
                "LINKED_FIELD_HINT",
                formula = formula,
                value = format!("{v:.2}{}", kind.unit())
            )),
            Err(FormulaError::UnknownVariable(name)) => Link::Broken(tr!(
                "BROKEN_FIELD_LINK_HINT",
                formula = formula,
                name = name
            )),
            Err(e) => Link::Broken(format!("{formula}: {}", e.message())),
        },
    };
    let fx_width = ui.spacing().interact_size.y;
    let width = ui.spacing().interact_size.x * 1.6 + fx_width;
    // A number set in the formula editor last frame (the value is borrowed by the field
    // once drawn, so it is set now).
    let pending_id = formula_id.with("set");
    let pending = ui.data_mut(|d| d.remove_temp::<f32>(pending_id));
    if let Some(v) = pending {
        *value = v;
    }
    let current = *value;
    // The value, the formula's result when linked. A number typed in or dragged unlinks
    // it, a formula links it again.
    let (mut response, typed) = plain_field(ui, value, make, &vars, kind, width);
    match typed {
        Some(formula) => {
            bindings.insert(field, formula);
        }
        None if response.changed() => {
            bindings.remove(&field);
        }
        None => {}
    }
    match &link {
        Link::Linked(hint) | Link::Broken(hint) => response = response.on_hover_text(hint),
        Link::None => {}
    }
    // The fx button, over the field's right end: added last, it takes the clicks there.
    let rect = response.rect;
    let fx_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - fx_width, rect.top()),
        rect.right_bottom(),
    )
    .shrink(2.0);
    let fx = ui.interact(fx_rect, formula_id.with("fx"), egui::Sense::click());
    let hint = match &link {
        Link::None => tr!("LINK_VARIABLE_HINT").to_owned(),
        Link::Linked(_) => tr!("CHANGE_VARIABLE_LINK_HINT").to_owned(),
        Link::Broken(hint) => hint.clone(),
    };
    let fx = fx.on_hover_text(hint);
    if ui.is_rect_visible(fx_rect) {
        let painter = ui.painter();
        // Linked, a filled badge in the link color (the warning color when broken), so
        // the fields using variables stand out; unlinked, a plain gray fx.
        let badge = match link {
            Link::None => None,
            Link::Linked(_) => Some(ui.visuals().hyperlink_color),
            Link::Broken(_) => Some(ui.visuals().warn_fg_color),
        };
        let color = match badge {
            Some(fill) => {
                let fill = if fx.hovered() {
                    fill.gamma_multiply(0.8)
                } else {
                    fill
                };
                painter.rect_filled(fx_rect, 3.0, fill);
                ui.visuals().extreme_bg_color
            }
            None => {
                if fx.hovered() {
                    painter.rect_filled(fx_rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                ui.visuals().weak_text_color()
            }
        };
        let mut font = egui::TextStyle::Small.resolve(ui.style());
        font.size *= 1.1;
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "fx",
            0.0,
            egui::TextFormat {
                font_id: font,
                color,
                italics: true,
                ..Default::default()
            },
        );
        let galley = painter.layout_job(job);
        painter.galley(fx_rect.center() - galley.size() / 2.0, galley, color);
    }
    if let Some(v) = formula_editor(ui, &fx, bindings, field, current, &vars) {
        ui.data_mut(|d| d.insert_temp(pending_id, v));
        ui.ctx().request_repaint();
    }
    if pending.is_some() {
        response.mark_changed();
    }
    response
}

/// Width of the formula editor's contents, in points.
const FORMULA_EDITOR_WIDTH: f32 = 260.0;

/// The formula editor of a [`field`], opened by its fx button `fx`: the formula, its
/// result (or what is wrong with it), the variables the field takes (a click inserts
/// one at the cursor), then Unlink (the field keeps its value), Cancel and OK. OK with a
/// formula naming variables links the field to it; with a plain number, it unlinks the
/// field and returns the number, for the field to take. Enter is OK, a click outside
/// cancels. Unlinked, the formula starts as the field's `value`.
fn formula_editor(
    ui: &mut Ui,
    fx: &Response,
    bindings: &mut Bindings,
    field: Field,
    value: f32,
    vars: &[Variable],
) -> Option<f32> {
    let kind = field.kind();
    let popup_id = fx.id.with("editor");
    let draft_id = popup_id.with("draft");
    let edit_id = popup_id.with("edit");
    if fx.clicked() {
        let draft = bindings
            .get(&field)
            .cloned()
            .unwrap_or_else(|| format!("{}", (value * 1000.0).round() / 1000.0));
        ui.data_mut(|d| {
            d.insert_temp(draft_id, draft);
            // Ready to type once the editor shows.
            d.insert_temp(popup_id.with("focus"), true);
        });
    }
    let mut set = None;
    let mut close = false;
    egui::Popup::menu(fx)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            // A fixed width: the formula box would otherwise take all the room it gets.
            ui.set_width(FORMULA_EDITOR_WIDTH);
            let mut draft: String = ui.data(|d| d.get_temp(draft_id)).unwrap_or_default();
            ui.label(RichText::new(tr!("FORMULA_EDITOR_TITLE")).strong());
            // Sized as a whole: a desired width leaves out the box's margins, which then
            // stick out of the editor.
            let size = [ui.available_width(), ui.spacing().interact_size.y];
            let edit = ui
                .scope(|ui| {
                    crate::layers_ui::field_frames(ui);
                    ui.add_sized(
                        size,
                        egui::TextEdit::singleline(&mut draft)
                            .id(edit_id)
                            .align(egui::Align2::LEFT_CENTER),
                    )
                })
                .inner;
            if ui.data_mut(|d| d.remove_temp::<bool>(popup_id.with("focus"))) == Some(true) {
                edit.request_focus();
            }
            let result = eval(&draft, vars, kind);
            match &result {
                Ok(v) => ui.label(tr!(
                    "FORMULA_RESULT",
                    value = format!("{v:.2}{}", kind.unit())
                )),
                Err(e) => ui.colored_label(ui.visuals().error_fg_color, e.message()),
            };
            let compatible: Vec<&Variable> = vars
                .iter()
                .filter(|v| matches!(v.value.kind(), k if k == kind || k == Kind::Number))
                .collect();
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("FORMULA_VARIABLES")).small().weak());
            if compatible.is_empty() {
                crate::callout::callout(
                    ui,
                    crate::callout::Tone::Info,
                    tr!("NO_COMPATIBLE_VARIABLES"),
                );
            }
            for v in compatible {
                let text = format!("{}   {}", v.name, v.value.short_text());
                let clicked = ui
                    .horizontal(|ui| {
                        v.value.kind().icon(ui);
                        ui.selectable_label(false, text).clicked()
                    })
                    .inner;
                if clicked {
                    insert_at_cursor(ui.ctx(), edit_id, &mut draft, &v.name);
                }
            }
            ui.add_space(4.0);
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            // As in the app's dialogs: OK at the right edge, Cancel on its left; Unlink,
            // when linked, apart at the left edge.
            crate::app::dialog_button_row(ui, |ui| {
                let ok = ui
                    .add_enabled(result.is_ok(), egui::Button::new(tr!("OK")))
                    .clicked();
                let cancel = ui.button(tr!("CANCEL")).clicked();
                if bindings.contains_key(&field) {
                    crate::app::at_left_edge(ui, |ui| {
                        if ui.button(tr!("UNLINK_VARIABLE")).clicked() {
                            bindings.remove(&field);
                            close = true;
                        }
                    });
                }
                close |= cancel;
                if (ok || enter)
                    && let Ok(v) = result
                {
                    let text = draft.trim().to_owned();
                    if uses_variables(&text) {
                        bindings.insert(field, text);
                    } else {
                        bindings.remove(&field);
                        set = Some(v as f32);
                    }
                    close = true;
                }
            });
            ui.data_mut(|d| d.insert_temp(draft_id, draft));
        });
    if close {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    set
}

/// Puts `name` into `text` at the cursor of the text field `id` (at the end when it has
/// none), with the cursor after it, and gives the field the keyboard back. A text that
/// is just a number (the field's value, which the editor starts with) is replaced.
fn insert_at_cursor(ctx: &egui::Context, id: Id, text: &mut String, name: &str) {
    if text.trim().is_empty() || text.trim().parse::<f64>().is_ok() {
        text.clear();
    }
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let chars = text.chars().count();
    let at = state
        .cursor
        .char_range()
        .map_or(chars, |r| usize::from(r.primary.index).min(chars));
    let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
    text.insert_str(byte, name);
    let after = egui::text::CCursor::new(at + name.chars().count());
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(after)));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Where a field stands with its variables.
pub enum Link {
    None,
    /// Linked; the hint says to what.
    Linked(String),
    /// Linked to a variable that no longer exists; the hint says which.
    Broken(String),
}

/// What was done with a [`link_button`].
pub enum LinkAction {
    None,
    /// Link to the variable of this name.
    Pick(String),
    /// Unlink, keeping the value.
    Unlink,
}

/// A link button: a chain link, gray when unlinked, in the link color when linked and
/// broken in the warning color when its variable is gone. Its menu lists `candidates`,
/// `current` selected, then Unlink.
pub fn link_button(
    ui: &mut Ui,
    link: &Link,
    current: Option<&str>,
    candidates: &[&Variable],
) -> LinkAction {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let hint = match link {
        Link::None => tr!("LINK_VARIABLE_HINT").to_owned(),
        Link::Linked(_) => tr!("CHANGE_VARIABLE_LINK_HINT").to_owned(),
        Link::Broken(hint) => hint.clone(),
    };
    let response = response.on_hover_text(hint);
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let color = match link {
            Link::None => ui.visuals().weak_text_color(),
            Link::Linked(_) => ui.visuals().hyperlink_color,
            Link::Broken(_) => ui.visuals().warn_fg_color,
        };
        paint_link(ui.painter(), rect, color, matches!(link, Link::Broken(_)));
    }
    let mut action = LinkAction::None;
    egui::Popup::menu(&response).show(|ui| {
        if candidates.is_empty() {
            crate::callout::callout_small(
                ui,
                crate::callout::Tone::Info,
                tr!("NO_COMPATIBLE_VARIABLES"),
            );
        }
        for v in candidates {
            let selected = current == Some(v.name.as_str());
            let text = format!("{}   {}", v.name, v.value.short_text());
            let clicked = ui
                .horizontal(|ui| {
                    v.value.kind().icon(ui);
                    ui.selectable_label(selected, text).clicked()
                })
                .inner;
            if clicked {
                action = LinkAction::Pick(v.name.clone());
            }
        }
        ui.separator();
        if ui
            .add_enabled(current.is_some(), egui::Button::new(tr!("UNLINK_VARIABLE")))
            .clicked()
        {
            action = LinkAction::Unlink;
        }
    });
    action
}

/// Paints a chain link in `rect`: two interlocked links, or, `broken`, two links apart
/// with a crack between them.
fn paint_link(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, broken: bool) {
    paint_link_stroke(painter, rect, egui::Stroke::new(1.5, color), broken);
}

/// [`paint_link`] with `stroke`.
pub fn paint_link_stroke(
    painter: &egui::Painter,
    rect: egui::Rect,
    stroke: egui::Stroke,
    broken: bool,
) {
    let side = rect.width().min(rect.height());
    let c = rect.center();
    // Along the diagonal, from bottom left to top right, as chain icons usually are.
    let axis = egui::vec2(1.0, -1.0).normalized();
    let across = egui::vec2(axis.y, -axis.x);
    let (len, off) = if broken {
        (side * 0.46, side * 0.29)
    } else {
        (side * 0.6, side * 0.2)
    };
    let radius = side * 0.13;
    for sign in [-1.0, 1.0] {
        capsule(painter, c + axis * sign * off, axis, len, radius, stroke);
    }
    if broken {
        // Sparks in the gap: a short stroke on each side of it, across the chain.
        for sign in [-1.0, 1.0] {
            let from = c + across * sign * side * 0.12;
            let to = c + across * sign * side * 0.3;
            painter.line_segment([from, to], stroke);
        }
    }
}

/// A capsule's outline: a link of a chain, `len` long from end to end along `axis`,
/// `radius` round at both ends, centered on `center`.
fn capsule(
    painter: &egui::Painter,
    center: egui::Pos2,
    axis: egui::Vec2,
    len: f32,
    radius: f32,
    stroke: egui::Stroke,
) {
    let half = (len / 2.0 - radius).max(0.0);
    let angle = axis.y.atan2(axis.x);
    let mut points = Vec::with_capacity(20);
    for (end, from) in [
        (center + axis * half, -90.0_f32),
        (center - axis * half, 90.0),
    ] {
        for k in 0..=8 {
            let a = angle + (from + 180.0 * k as f32 / 8.0).to_radians();
            points.push(end + radius * egui::vec2(a.cos(), a.sin()));
        }
    }
    painter.add(egui::Shape::closed_line(points, stroke));
}

/// The variables `layer` is linked to that `vars` no longer has (or that are no longer of
/// the right kind), once each, in order: from its fields' formulas, its colors, its
/// effects' colors and its text.
pub fn broken_links(layer: &Layer, vars: &[Variable]) -> Vec<String> {
    let mut gone = Vec::new();
    let mut add = |name: &str| {
        if !gone.iter().any(|g| g == name) {
            gone.push(name.to_owned());
        }
    };
    for (field, formula) in &layer.bindings {
        match eval(formula, vars, field.kind()) {
            Err(FormulaError::UnknownVariable(name))
            | Err(FormulaError::WrongKind { name, .. }) => add(&name),
            _ => {}
        }
    }
    let is = |name: &str, kind: Kind| {
        vars.iter()
            .any(|v| v.name == name && v.value.kind() == kind)
    };
    let colors = layer.color_links.values();
    let effects = layer.effects.iter().flat_map(|fx| fx.links.values());
    for name in colors.chain(effects) {
        if !is(name, Kind::Color) {
            add(name);
        }
    }
    if let Some(name) = layer.text.as_ref().and_then(|t| t.link.as_deref())
        && !is(name, Kind::Text)
    {
        add(name);
    }
    gone
}

/// [`broken_links`] of every layer of `doc` that has some, by id; a folded group gets
/// those of the layers inside it, which the layer list doesn't show (an expanded one
/// shows them on its layers' rows).
pub fn broken_links_by_node(doc: &Document) -> HashMap<u64, Vec<String>> {
    fn walk(nodes: &[Node], vars: &[Variable], out: &mut HashMap<u64, Vec<String>>) -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        for n in nodes {
            let gone = match n {
                Node::Layer(l) => broken_links(l, vars),
                Node::Group(g) => walk(&g.children, vars, out),
            };
            if !gone.is_empty() {
                for name in &gone {
                    if !all.contains(name) {
                        all.push(name.clone());
                    }
                }
                let expanded = matches!(n, Node::Group(g) if g.expanded);
                if !expanded {
                    out.insert(n.id(), gone);
                }
            }
        }
        all
    }
    let mut out = HashMap::new();
    walk(&doc.layers, &doc.variables, &mut out);
    out
}

/// A broken chain in the warning color, for a layer list row whose layer (or group)
/// is linked to variables that no longer exist, named in its tooltip.
pub fn broken_links_badge(ui: &mut Ui, gone: &[String]) -> Response {
    let side = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_link(ui.painter(), rect, ui.visuals().warn_fg_color, true);
    }
    response.on_hover_text(tr_n!(
        "LAYER_BROKEN_LINKS",
        gone.len(),
        names = gone.join(", ")
    ))
}

/// Keeps a layer's width and height linked together while their aspect is locked:
/// `before` are their formulas before this frame's edits, `size` their sizes then.
/// Linking one links the other to the same formula, scaled by their ratio (`button`
/// for the width gives `button * 0.5` for a height half as big); unlinking one unlinks
/// the other. Locked with only one of them linked, the other is linked to it too. A
/// formula that doesn't work out with `vars` (one being typed) isn't passed on.
pub fn link_aspect(
    bindings: &mut Bindings,
    before: [Option<String>; 2],
    size: [f32; 2],
    vars: &[Variable],
) {
    let fields = [Field::Width, Field::Height];
    let now = fields.map(|f| bindings.get(&f).cloned());
    let works = |f: &str| eval(f, vars, Kind::Length).is_ok();
    for i in 0..2 {
        let j = 1 - i;
        if now[i] == before[i] {
            continue;
        }
        match &now[i] {
            Some(f) if size[i] > 1e-6 && works(f) => {
                bindings.insert(fields[j], scaled(f, size[j] / size[i]));
            }
            Some(_) => {}
            None => {
                bindings.remove(&fields[j]);
            }
        }
        return;
    }
    if let [Some(f), None] | [None, Some(f)] = &now {
        let i = usize::from(now[0].is_none());
        if size[i] > 1e-6 && works(f) {
            bindings.insert(fields[1 - i], scaled(f, size[1 - i] / size[i]));
        }
    }
}

/// `formula` times `k`, as a formula: `button * 0.5`, `(a + 2) * 1.25`.
fn scaled(formula: &str, k: f32) -> String {
    let formula = formula.trim();
    if (k - 1.0).abs() < 1e-4 {
        return formula.to_owned();
    }
    let k = format!("{k:.4}");
    let k = k.trim_end_matches('0').trim_end_matches('.');
    let simple = formula
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if simple {
        format!("{formula} * {k}")
    } else {
        format!("({formula}) * {k}")
    }
}

/// An unlinked [`field`]: the `DragValue` made by `make`, which also takes a formula.
/// Returns its response and the formula typed in, when it names a variable.
fn plain_field<'a>(
    ui: &mut Ui,
    value: &'a mut f32,
    make: impl FnOnce(DragValue<'a>) -> DragValue<'a>,
    vars: &[Variable],
    kind: Kind,
    width: f32,
) -> (Response, Option<String>) {
    let typed: RefCell<Option<String>> = RefCell::new(None);
    let drag = make(DragValue::new(value)).custom_parser(|s| {
        let v = eval(s, vars, kind).ok()?;
        if uses_variables(s) {
            *typed.borrow_mut() = Some(s.trim().to_owned());
        }
        Some(v)
    });
    let response = ui.add_sized([width, ui.spacing().interact_size.y], drag);
    (response, typed.into_inner())
}

/// The Variables section's body: the variables grouped by kind, each with its kind's
/// icon, its name, its value, its unit and a remove button, then an Add menu.
/// Renaming a variable renames it in
/// the formulas and color links. Removing one in use asks first; the fields and colors
/// using it keep their link and their value, and show a broken link icon (see
/// [`field`]). Returns true if the document changed.
pub fn variables_ui(ui: &mut Ui, doc: &mut Document) -> bool {
    let mut changed = false;
    if doc.variables.is_empty() {
        crate::callout::callout(ui, crate::callout::Tone::Info, tr!("VARIABLES_EMPTY"));
        ui.add_space(4.0);
    }
    let mut remove = None;
    let mut rename = None;
    let names: Vec<String> = doc.variables.iter().map(|v| v.name.clone()).collect();
    let used: Vec<usize> = names.iter().map(|n| use_count(doc, n)).collect();
    for kind in Kind::ALL {
        let of_kind: Vec<usize> = (0..doc.variables.len())
            .filter(|&i| doc.variables[i].value.kind() == kind)
            .collect();
        if of_kind.is_empty() {
            continue;
        }
        for i in of_kind {
            let var = &mut doc.variables[i];
            ui.horizontal(|ui| {
                kind.icon(ui);
                // From the right edge: the remove button, the value, then the name in the
                // room left, so the values and buttons line up from row to row.
                let value_width = ui.spacing().interact_size.x * 1.6;
                let row_height = ui.spacing().interact_size.y;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if crate::layers_ui::tool_button(
                        ui,
                        crate::layers_ui::Tool::Delete,
                        true,
                        tr!("REMOVE_VARIABLE"),
                    ) {
                        remove = Some(i);
                    }
                    // The unit, in a column of its own so the values line up, empty for
                    // numbers and colors.
                    ui.allocate_ui_with_layout(
                        egui::vec2(row_height * 1.2, row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| crate::layers_ui::unit(ui, kind.unit()),
                    );
                    if let Value::Text(t) = &mut var.value {
                        changed |= text_value_button(ui, t, [value_width, row_height]);
                    } else if let Value::Color(c) = &mut var.value {
                        changed |= ui
                            .scope(|ui| {
                                ui.spacing_mut().interact_size.x = value_width;
                                ui.color_edit_button_srgba_unmultiplied(c)
                            })
                            .inner
                            .on_hover_text(crate::colors::describe(*c))
                            .changed();
                    } else if let Some(v) = var.value.number_mut() {
                        changed |= ui
                            .add_sized(
                                [value_width, row_height],
                                DragValue::new(v).speed(0.1).max_decimals(3),
                            )
                            .on_hover_text(kind.label())
                            .changed();
                    }
                    // The name is edited in a draft, applied when the field is left, so the
                    // formulas and links follow from one valid name to the next.
                    let id = ui.scope_id().with(("variable_name", i));
                    let mut draft = ui
                        .data(|d| d.get_temp::<String>(id))
                        .unwrap_or_else(|| var.name.clone());
                    let taken = |n: &str| names.iter().enumerate().any(|(j, m)| j != i && m == n);
                    let ok = is_valid_name(&draft) && !taken(&draft);
                    let mut edit = egui::TextEdit::singleline(&mut draft)
                        .desired_width(ui.available_width())
                        .hint_text(tr!("VARIABLE_NAME_HINT"));
                    if !ok {
                        edit = edit.text_color(ui.visuals().error_fg_color);
                    }
                    let r = ui.add(edit);
                    let r = if ok {
                        r.on_hover_text(tr_n!("VARIABLE_USE_COUNT", used[i]))
                    } else {
                        r.on_hover_text(tr!("VARIABLE_NAME_INVALID"))
                    };
                    if r.lost_focus() {
                        if ok && draft != var.name {
                            rename = Some((var.name.clone(), draft.clone()));
                            var.name = draft;
                            changed = true;
                        }
                        ui.data_mut(|d| d.remove::<String>(id));
                    } else if r.has_focus() || r.changed() {
                        ui.data_mut(|d| d.insert_temp(id, draft));
                    }
                });
            });
        }
    }
    if let Some((old, new)) = rename {
        doc.for_each_layer_mut(|l| {
            for f in l.bindings.values_mut() {
                *f = rename_in(f, &old, &new);
            }
        });
        crate::colors::rename_links(doc, &old, &new);
        crate::text_links::rename_links(doc, &old, &new);
    }
    // A variable in use is removed once confirmed; its name waits here meanwhile.
    let confirm_id = Id::unique("confirm_remove_variable");
    if let Some(i) = remove {
        if used[i] == 0 {
            doc.variables.remove(i);
            changed = true;
        } else {
            ui.data_mut(|d| d.insert_temp(confirm_id, names[i].clone()));
        }
    }
    if let Some(name) = ui.data(|d| d.get_temp::<String>(confirm_id)) {
        let n = names.iter().position(|m| *m == name).map_or(0, |i| used[i]);
        let mut answer = (false, false);
        let response =
            crate::app::dialog(ui.ctx(), "confirm_remove_variable").show(ui.ctx(), |ui| {
                ui.set_max_width(420.0);
                ui.strong(tr!("CONFIRM_REMOVE_VARIABLE", name = name));
                ui.add_space(4.0);
                crate::callout::callout(
                    ui,
                    crate::callout::Tone::Warning,
                    &tr_n!("REMOVED_VARIABLE_USERS", n),
                );
                ui.add_space(8.0);
                answer = crate::app::dialog_buttons(ui, tr!("REMOVE_VARIABLE_OK"), true);
            });
        let (remove, cancel) = answer;
        if remove || cancel || response.should_close() {
            ui.data_mut(|d| d.remove::<String>(confirm_id));
        }
        if remove {
            // The fields and colors using it keep their link, broken, and their value.
            doc.variables.retain(|v| v.name != name);
            changed = true;
        }
    }
    ui.add_space(6.0);
    ui.menu_button(tr!("ADD_VARIABLE"), |ui| {
        for kind in Kind::ALL {
            let clicked = ui
                .horizontal(|ui| {
                    kind.icon(ui);
                    ui.button(kind.label()).clicked()
                })
                .inner;
            if clicked {
                let name = new_name(&doc.variables, kind);
                doc.variables.push(Variable {
                    name,
                    value: kind.new_value(),
                });
                changed = true;
            }
        }
    });
    ui.add_space(6.0);
    changed
}

/// A text variable's value in the Variables section: a button of `size` showing the
/// start of the text, which opens a box to edit it, as a color button opens a picker.
/// Returns true if the text changed.
fn text_value_button(ui: &mut Ui, text: &mut String, size: [f32; 2]) -> bool {
    let first_line = text.lines().next().unwrap_or_default();
    let button = ui
        .add_sized(size, egui::Button::new(first_line).truncate())
        .on_hover_text(if text.is_empty() {
            tr!("EDIT_TEXT_VARIABLE_HINT").to_owned()
        } else {
            format!("{text}\n\n{}", tr!("EDIT_TEXT_VARIABLE_HINT"))
        });
    let mut changed = false;
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            crate::layers_ui::field_frames(ui);
            let edit = ui.add(
                egui::TextEdit::multiline(text)
                    .desired_rows(4)
                    .desired_width(260.0),
            );
            // Ready to type as soon as it opens.
            if edit.ctx.memory(|m| m.focused().is_none()) {
                edit.request_focus();
            }
            changed |= edit.changed();
        });
    changed
}

/// How many fields and colors use the variable `name`.
fn use_count(doc: &mut Document, name: &str) -> usize {
    fn walk(nodes: &[Node], name: &str) -> usize {
        nodes
            .iter()
            .map(|n| match n {
                Node::Group(g) => walk(&g.children, name),
                Node::Layer(l) => l.bindings.values().filter(|f| uses(f, name)).count(),
            })
            .sum()
    }
    walk(&doc.layers, name)
        + crate::colors::use_count(doc, name)
        + crate::text_links::use_count(doc, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str, value: Value) -> Variable {
        Variable {
            name: name.into(),
            value,
        }
    }

    fn vars() -> Vec<Variable> {
        vec![
            var("button", Value::Length(30.0)),
            var("marge_é", Value::Number(2.5)),
            var("tilt", Value::Angle(30.0)),
            var("ink", Value::Color([1, 2, 3, 255])),
        ]
    }

    #[test]
    fn formulas() {
        let v = vars();
        let e = |s: &str| eval(s, &v, Kind::Length);
        assert_eq!(e("button + 2"), Ok(32.0));
        assert_eq!(e("(button - 2 * marge_é) / 5"), Ok(5.0));
        assert_eq!(e("-button + +4"), Ok(-26.0));
        assert_eq!(
            e("2 − 0.5"),
            Ok(1.5),
            "the minus sign typed on some keyboards"
        );
        assert_eq!(e(" 12.5 "), Ok(12.5));
        assert_eq!(e("btn"), Err(FormulaError::UnknownVariable("btn".into())));
        assert_eq!(
            e("button / (marge_é - 2.5)"),
            Err(FormulaError::DivisionByZero)
        );
        for bad in ["", "2 +", "(2", "2)", "2 3", "1..2", "2 % 3"] {
            assert_eq!(e(bad), Err(FormulaError::Syntax), "{bad}");
        }
        // A field takes variables of its kind and numbers, not other kinds or colors.
        let wrong = |name: &str, kind| {
            Err(FormulaError::WrongKind {
                name: name.into(),
                kind,
            })
        };
        assert_eq!(e("button + tilt"), wrong("tilt", Kind::Length));
        assert_eq!(e("ink"), wrong("ink", Kind::Length));
        assert_eq!(eval("tilt * marge_é", &v, Kind::Angle), Ok(75.0));
        assert_eq!(eval("button", &v, Kind::Size), wrong("button", Kind::Size));
        assert_eq!(eval("marge_é * 4", &v, Kind::Size), Ok(10.0));
        assert!(uses_variables("button*2") && !uses_variables("2*3"));
        assert!(uses("1 + button", "button") && !uses("buttons", "button"));
        assert_eq!(
            rename_in("button+buttons*(button)", "button", "b"),
            "b+buttons*(b)"
        );
        assert!(is_valid_name("_a1") && is_valid_name("épaisseur"));
        assert!(!is_valid_name("1a") && !is_valid_name("a b") && !is_valid_name(""));
    }

    fn doc_with(layer: Layer) -> Document {
        Document {
            layers: vec![Node::Layer(Box::new(layer))],
            variables: vars(),
            ..Default::default()
        }
    }

    #[test]
    fn linked_fields_follow_their_variables_until_changed_by_hand() {
        let mut l = Layer::new(1, "img".into(), "x.png".into());
        l.size_px = [100, 50];
        l.bindings.insert(Field::Width, "button + 2".into());
        l.bindings.insert(Field::X, "button * 2".into());
        l.bindings.insert(Field::Y, "nope".into());
        l.bindings.insert(Field::StrokeWidth, "1".into());
        l.bindings.insert(Field::Rotation, "tilt * 7".into());
        let mut doc = doc_with(l);
        let mut applied = Applied::new();
        assert!(apply(&mut doc, &mut applied));
        let l = doc.layer(1).unwrap();
        assert!(same(l.size_mm()[0], 32.0) && same(l.size_mm()[1], 50.0));
        assert_eq!(
            l.transform.center,
            [60.0, 0.0],
            "a broken formula changes nothing"
        );
        assert_eq!(
            l.transform.rotation_deg, -150.0,
            "210° turned into the range"
        );
        assert!(!apply(&mut doc, &mut applied), "nothing more to do");

        doc.variables[0].value = Value::Length(40.0);
        assert!(apply(&mut doc, &mut applied));
        let l = doc.layer(1).unwrap();
        assert!(same(l.size_mm()[0], 42.0));
        assert_eq!(l.transform.center[0], 80.0);

        // Moved by hand: x is unlinked and stays; the width is still linked.
        doc.layer_mut(1).unwrap().transform.center[0] = 5.0;
        doc.variables[0].value = Value::Length(50.0);
        apply(&mut doc, &mut applied);
        let l = doc.layer(1).unwrap();
        assert_eq!(l.transform.center[0], 5.0);
        assert!(!l.bindings.contains_key(&Field::X));
        assert!(same(l.size_mm()[0], 52.0));
        assert!(
            l.bindings.contains_key(&Field::StrokeWidth),
            "kept on a layer without the field"
        );
    }

    #[test]
    fn shapes_and_texts_take_their_own_fields() {
        let mut s = shape::new_layer(1, ShapeKind::Rectangle, [100.0, 100.0]);
        s.bindings.insert(Field::Height, "button".into());
        s.bindings.insert(Field::CornerRadius, "100".into());
        s.bindings.insert(Field::StrokeWidth, "marge_é".into());
        let mut doc = doc_with(s);
        apply(&mut doc, &mut Applied::new());
        let shape = doc.layer(1).unwrap().shape.as_deref().unwrap();
        assert!(same(shape.geometry_size()[1], 30.0));
        assert_eq!(shape.stroke_width, 2.5);
        let max = shape.size[0].min(shape.size[1]) / 2.0;
        assert_eq!(shape.corner_radius, max, "clamped as in Properties");

        let mut t = text::new_layer(1, "Hi", "Sans".into(), [0.0, 0.0]);
        t.bindings.insert(Field::TextSize, "marge_é * 6".into());
        let mut doc = doc_with(t);
        apply(&mut doc, &mut Applied::new());
        assert!(same(text::size_pt(doc.layer(1).unwrap()), 15.0));
    }

    /// Clicks the field, selects its text, types `text` and presses Enter, a frame per
    /// step. Returns whether the field reported a change.
    fn type_in(ctx: &egui::Context, bindings: &mut Bindings, value: &mut f32, text: &str) -> bool {
        type_in_at(ctx, bindings, value, text, 20.0)
    }

    /// [`type_in`], clicking the field `x` points from the left.
    fn type_in_at(
        ctx: &egui::Context,
        bindings: &mut Bindings,
        value: &mut f32,
        text: &str,
        x: f32,
    ) -> bool {
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut changed = false;
        let p = egui::pos2(x, 10.0);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let steps = vec![
            vec![egui::Event::PointerMoved(p)],
            vec![button(true)],
            vec![button(false)],
            vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
            vec![egui::Event::Text(text.into())],
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            vec![],
        ];
        let start = ctx.input(|i| i.time);
        for (i, events) in steps.into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                time: Some(start + 0.1 * (i + 1) as f64),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                publish(ui.ctx(), &doc);
                changed |= field(ui, bindings, Field::X, value, |d| d.suffix(" mm")).changed();
            });
            out.textures_delta.clear();
        }
        changed
    }

    /// The texts drawn by a frame of `ui`.
    fn texts_of(ctx: &egui::Context, ui: impl FnMut(&mut Ui)) -> Vec<String> {
        let mut ui = ui;
        let mut out = ctx.run_ui(egui::RawInput::default(), |u| ui(u));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_field_linked_to_a_removed_variable_keeps_its_value_until_replaced() {
        let ctx = egui::Context::default();
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut bindings = Bindings::from([(Field::X, "gone + 2".to_owned())]);
        let mut x = 12.0;
        let texts = texts_of(&ctx, |ui| {
            publish(ui.ctx(), &doc);
            field(ui, &mut bindings, Field::X, &mut x, |d| d.suffix(" mm"));
        });
        assert!(
            !texts.iter().any(|t| t.contains("gone")),
            "the value, not the formula: {texts:?}"
        );
        assert!(texts.iter().any(|t| t.starts_with("12")), "{texts:?}");
        assert_eq!(bindings[&Field::X], "gone + 2", "still linked, broken");

        // Past the broken link icon: a variable links it again, a number unlinks it.
        assert!(type_in_at(&ctx, &mut bindings, &mut x, "button", 40.0));
        assert_eq!((x, bindings[&Field::X].as_str()), (30.0, "button"));
        bindings.insert(Field::X, "gone".into());
        assert!(type_in_at(&ctx, &mut bindings, &mut x, "7", 40.0));
        assert_eq!(x, 7.0);
        assert!(bindings.is_empty());
    }

    #[test]
    fn removing_a_variable_in_use_asks_first_and_leaves_its_links_broken() {
        let mut l = Layer::new(1, "img".into(), "x.png".into());
        l.bindings.insert(Field::X, "button + 2".into());
        let mut doc = doc_with(l);
        let ctx = egui::Context::default();
        let confirm_id = Id::unique("confirm_remove_variable");
        ctx.data_mut(|d| d.insert_temp(confirm_id, "button".to_owned()));
        let frame = |events: Vec<egui::Event>, doc: &mut Document| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                variables_ui(ui, doc);
            });
            out.textures_delta.clear();
            out.shapes
        };
        // The dialog sizes itself on its first frame, and is drawn on the next.
        frame(Vec::new(), &mut doc);
        let shapes = frame(Vec::new(), &mut doc);
        let find = |text: &str| {
            shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.size() / 2.0)
                }
                _ => None,
            })
        };
        assert!(find("Remove the variable button?").is_some());
        assert!(
            find("It is used by 1 field or color, which keeps its current value and shows a broken link icon.")
                .is_some()
        );
        let ok = find("Remove").expect("the Remove button");
        for events in [
            vec![egui::Event::PointerMoved(ok)],
            vec![egui::Event::PointerButton {
                pos: ok,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: ok,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            frame(events, &mut doc);
        }
        assert!(doc.variables.iter().all(|v| v.name != "button"));
        assert_eq!(
            doc.layer(1).unwrap().bindings[&Field::X],
            "button + 2",
            "the field keeps its link, broken"
        );
        assert!(ctx.data(|d| d.get_temp::<String>(confirm_id)).is_none());
    }

    #[test]
    fn the_fx_button_edits_the_formula_with_the_variables_the_field_takes() {
        let ctx = egui::Context::default();
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut bindings = Bindings::new();
        let mut x = 1.0;
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, bindings: &mut Bindings, x: &mut f32| {
            time.set(time.get() + 0.1);
            let input = egui::RawInput {
                time: Some(time.get()),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut fx = egui::Pos2::ZERO;
            let mut out = ctx.run_ui(input, |ui| {
                publish(ui.ctx(), &doc);
                let r = field(ui, bindings, Field::X, x, |d| d);
                // The fx button is inside the field, at its right end.
                fx = egui::pos2(r.rect.right() - r.rect.height() / 2.0, r.rect.center().y);
            });
            out.textures_delta.clear();
            // Settle what the frame changed, as the app does.
            let mut d = doc.clone();
            let mut l = Layer::new(1, "l".into(), "".into());
            l.bindings = bindings.clone();
            l.transform.center[0] = *x;
            d.layers = vec![Node::Layer(Box::new(l))];
            apply(&mut d, &mut Applied::default());
            *x = d.layer(1).unwrap().transform.center[0];
            let texts: Vec<(String, egui::Pos2)> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.pos + t.galley.size() / 2.0))
                    }
                    _ => None,
                })
                .collect();
            (fx, texts)
        };
        let click = |p: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            [
                vec![egui::Event::PointerMoved(p)],
                vec![press(true)],
                vec![press(false)],
            ]
        };
        let mut run = |events: Vec<egui::Event>| frame(events, &mut bindings, &mut x);
        let open =
            |run: &mut dyn FnMut(Vec<egui::Event>) -> (egui::Pos2, Vec<(String, egui::Pos2)>)| {
                let (fx, _) = run(Vec::new());
                for events in click(fx) {
                    run(events);
                }
                for _ in 0..5 {
                    run(Vec::new());
                }
                run(Vec::new()).1
            };
        let find = |texts: &[(String, egui::Pos2)], t: &str| {
            texts
                .iter()
                .find(|(s, _)| s == t)
                .unwrap_or_else(|| panic!("{t} in {texts:?}"))
                .1
        };
        let texts = open(&mut run);
        let names: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            names.contains(&"Result: 1.00 mm"),
            "starts from the value: {names:?}"
        );
        // OK at the right edge, Cancel on its left, the editor as wide as its contents.
        let (ok, cancel) = (find(&texts, "OK"), find(&texts, "Cancel"));
        let title = find(&texts, "Formula");
        assert!(ok.x > cancel.x, "OK right of Cancel");
        assert!(
            ok.x - title.x < FORMULA_EDITOR_WIDTH + 10.0,
            "{ok:?} {title:?}"
        );
        assert!(
            !names.contains(&"Unlink, keeping the value"),
            "not linked yet"
        );
        assert!(names.contains(&"marge_é   2.5"), "numbers too: {names:?}");
        assert!(
            !names.iter().any(|t| t.starts_with("tilt")),
            "not angles: {names:?}"
        );
        // A click on a variable replaces the value with it.
        for events in click(find(&texts, "button   30 mm")) {
            run(events);
        }
        let texts = run(Vec::new()).1;
        assert!(
            texts.iter().any(|(t, _)| t == "Result: 30.00 mm"),
            "{texts:?}"
        );
        for events in click(find(&texts, "OK")) {
            run(events);
        }
        run(Vec::new());
        let texts = run(Vec::new()).1;
        let shown: Vec<&str> = texts.iter().map(|(t, _)| t.as_str()).collect();
        assert!(shown.contains(&"30"), "the result: {shown:?}");
        assert!(!shown.contains(&"button"), "not the formula: {shown:?}");
        assert_eq!(bindings[&Field::X], "button");
        assert_eq!(x, 30.0);

        let mut run = |events: Vec<egui::Event>| frame(events, &mut bindings, &mut x);
        let texts = open(&mut run);
        for events in click(find(&texts, "Unlink, keeping the value")) {
            run(events);
        }
        run(Vec::new());
        assert!(bindings.is_empty());
        assert_eq!(x, 30.0, "the value stays");

        // A plain number, with Enter: the field takes it.
        let mut run = |events: Vec<egui::Event>| frame(events, &mut bindings, &mut x);
        open(&mut run);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        run(vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
        run(vec![egui::Event::Text("4 * 3".into())]);
        run(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        run(Vec::new());
        assert!(bindings.is_empty());
        assert_eq!(x, 12.0);
    }

    #[test]
    fn with_the_aspect_locked_width_and_height_are_linked_together() {
        assert_eq!(scaled("button", 0.5), "button * 0.5");
        assert_eq!(scaled("a + 2", 1.25), "(a + 2) * 1.25");
        assert_eq!(scaled("button", 1.0), "button");
        assert_eq!(scaled("button", 2.0 / 3.0), "button * 0.6667");
        let vars = vars();
        let size = [20.0, 10.0];
        let w = |f: &str| Some(f.to_owned());

        // Linking the width links the height, scaled.
        let mut b = Bindings::from([(Field::Width, "button".to_owned())]);
        link_aspect(&mut b, [None, None], size, &vars);
        assert_eq!(b[&Field::Height], "button * 0.5");
        // And the height the width.
        let mut b = Bindings::from([(Field::Height, "button".to_owned())]);
        link_aspect(&mut b, [None, None], size, &vars);
        assert_eq!(b[&Field::Width], "button * 2");
        // Unlinking one unlinks the other.
        let mut b = Bindings::from([(Field::Height, "button * 0.5".to_owned())]);
        link_aspect(&mut b, [w("button"), w("button * 0.5")], size, &vars);
        assert!(b.is_empty());
        // A formula being typed isn't passed on.
        let mut b = Bindings::from([(Field::Width, "butt".to_owned())]);
        link_aspect(&mut b, [w("b"), None], size, &vars);
        assert_eq!(b.get(&Field::Height), None);
        // Both linked their own way: left as they are.
        let mut b = Bindings::from([
            (Field::Width, "button".to_owned()),
            (Field::Height, "marge_é".to_owned()),
        ]);
        let before = [w("button"), w("marge_é")];
        link_aspect(&mut b, before, size, &vars);
        assert_eq!(b[&Field::Height], "marge_é");
    }

    #[test]
    fn a_text_variable_shows_its_name_and_is_edited_in_a_box() {
        let mut doc = Document {
            variables: vec![var("title", Value::Text("Hello\nWorld".into()))],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let time = std::cell::Cell::new(0.0);
        let frame = |events: Vec<egui::Event>, doc: &mut Document| {
            time.set(time.get() + 0.1);
            let input = egui::RawInput {
                time: Some(time.get()),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                variables_ui(ui, doc);
            });
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => {
                        Some((t.galley.text().to_owned(), t.pos + t.galley.size() / 2.0))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let texts = frame(Vec::new(), &mut doc);
        let name = texts.iter().find(|(t, _)| t == "title").expect("the name");
        let value = texts
            .iter()
            .find(|(t, _)| t == "Hello")
            .expect("the first line");
        assert!(name.1.x < value.1.x, "name, then value: {texts:?}");
        assert!(!texts.iter().any(|(t, _)| t.contains("World")), "{texts:?}");

        let at = value.1;
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![press(true)],
            vec![press(false)],
        ] {
            frame(events, &mut doc);
        }
        for _ in 0..5 {
            frame(Vec::new(), &mut doc);
        }
        let texts = frame(Vec::new(), &mut doc);
        assert!(
            texts.iter().any(|(t, _)| t == "Hello\nWorld"),
            "the box: {texts:?}"
        );
        frame(vec![egui::Event::Text("!".into())], &mut doc);
        let Value::Text(t) = &doc.variables[0].value else {
            unreachable!()
        };
        assert!(t.contains('!') && t.len() == "Hello\nWorld!".len(), "{t:?}");
    }

    #[test]
    fn layers_with_links_to_variables_gone_are_found_and_their_groups_too() {
        let mut fine = Layer::new(1, "fine".into(), "".into());
        fine.bindings.insert(Field::X, "button + 2".into());
        fine.color_links
            .insert(crate::colors::ColorField::Fill, "ink".into());
        let mut gone = Layer::new(2, "gone".into(), "".into());
        gone.bindings.insert(Field::X, "margin * 2".into());
        // A color link to a variable that isn't a color, and a text link to none.
        gone.color_links
            .insert(crate::colors::ColorField::Fill, "button".into());
        let mut shadow = crate::effects::EffectInstance::new(
            crate::effects::registry()
                .get("builtin.drop_shadow")
                .unwrap(),
        );
        shadow.links.insert("color".into(), "margin".into());
        gone.effects.push(shadow);
        gone.text = Some(Box::new(crate::text::Text {
            link: Some("title".into()),
            ..Default::default()
        }));
        let doc = Document {
            layers: vec![
                Node::Layer(Box::new(fine)),
                Node::Group(crate::document::Group {
                    id: 3,
                    name: "g".into(),
                    visible: true,
                    locked: false,
                    opacity: 1.0,
                    expanded: false,
                    children: vec![Node::Layer(Box::new(gone))],
                }),
            ],
            variables: vars(),
            ..Default::default()
        };
        let found = broken_links_by_node(&doc);
        assert!(!found.contains_key(&1), "all its links work");
        assert_eq!(found[&2], ["margin", "button", "title"], "once each");
        assert_eq!(found[&3], found[&2], "its group shows them, folded");
        let mut open = doc.clone();
        if let Node::Group(g) = &mut open.layers[1] {
            g.expanded = true;
        }
        let found = broken_links_by_node(&open);
        assert!(!found.contains_key(&3), "expanded, its layers show them");
        assert_eq!(found[&2], ["margin", "button", "title"]);
    }

    #[test]
    fn typing_a_formula_links_the_field_and_a_number_unlinks_it() {
        let ctx = egui::Context::default();
        let mut bindings = Bindings::new();
        let mut x = 1.0;
        assert!(type_in(&ctx, &mut bindings, &mut x, "button + 2"));
        assert_eq!(x, 32.0);
        assert_eq!(bindings[&Field::X], "button + 2");

        // A formula without variables is just a number.
        let mut other = Bindings::new();
        let mut y = 1.0;
        type_in(&egui::Context::default(), &mut other, &mut y, "3 * 4");
        assert_eq!(y, 12.0);
        assert!(other.is_empty());

        // Linked, the field holds the formula; a number unlinks it.
        assert!(type_in(&ctx, &mut bindings, &mut x, "7.5"));
        assert_eq!(x, 7.5);
        assert!(bindings.is_empty());
    }

    #[test]
    fn angles_wrap_into_the_rotation_range() {
        for (deg, wrapped) in [(0.0, 0.0), (190.0, -170.0), (-190.0, 170.0), (180.0, 180.0)] {
            assert_eq!(wrap_degrees(deg), wrapped, "{deg}");
        }
        assert_eq!(wrap_degrees(-180.0), -180.0);
        assert_eq!(wrap_degrees(540.0), 180.0);
    }

    #[test]
    fn variables_are_saved_with_their_kind_and_older_ones_are_numbers() {
        let json = serde_json::to_string(&vars()).unwrap();
        assert_eq!(
            json,
            r##"[{"name":"button","kind":"length","value":30.0},{"name":"marge_é","kind":"number","value":2.5},{"name":"tilt","kind":"angle","value":30.0},{"name":"ink","kind":"color","value":"#010203ff"}]"##
        );
        let back: Vec<Variable> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, vars());
        let old: Vec<Variable> = serde_json::from_str(r#"[{"name":"a","value":3}]"#).unwrap();
        assert_eq!(old, [var("a", Value::Number(3.0))]);
        for bad in [
            r#"[{"name":"a","kind":"color","value":3}]"#,
            r##"[{"name":"a","kind":"length","value":"#fff"}]"##,
            r#"[{"name":"a","kind":"weight","value":3}]"#,
        ] {
            assert!(serde_json::from_str::<Vec<Variable>>(bad).is_err(), "{bad}");
        }
        assert_eq!(new_name(&vars(), Kind::Color), "color1");
        assert_eq!(
            new_name(&[var("length1", Value::Length(1.0))], Kind::Length),
            "length2"
        );
    }

    #[test]
    fn the_section_lists_variables_by_kind_then_the_add_menu() {
        let mut doc = Document {
            variables: vec![
                var("ink", Value::Color([1, 2, 3, 255])),
                var("button", Value::Length(30.0)),
                var("count", Value::Number(3.0)),
            ],
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            variables_ui(ui, &mut doc);
        });
        out.textures_delta.clear();
        let texts: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        let at = |t: &str| texts.iter().position(|x| x == t);
        let order = ["button", "#", "count", "ink", "Add a variable"]
            .map(|t| at(t).unwrap_or_else(|| panic!("{t} in {texts:?}")));
        assert!(order.is_sorted(), "{texts:?}");
        assert!(
            at("Length (mm)").is_none() && at("Number").is_none() && at("Color").is_none(),
            "icons, not headings: {texts:?}"
        );
        let value = at("30.0").expect("the length's value");
        assert_eq!(
            texts[value - 1],
            "mm",
            "its unit, outside the field: {texts:?}"
        );
    }

    #[test]
    fn variables_and_bindings_are_saved_only_when_used() {
        let mut l = Layer::new(1, "img".into(), "x.png".into());
        l.bindings.insert(Field::TextSize, "button".into());
        let doc = doc_with(l);
        let json = serde_json::to_string(&doc).unwrap();
        assert!(
            json.contains(r#""bindings":{"text_size":"button"}"#),
            "{json}"
        );
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back.variables, doc.variables);
        assert_eq!(
            back.layer(1).unwrap().bindings,
            doc.layer(1).unwrap().bindings
        );
        let empty = serde_json::to_string(&doc_with(Layer::new(1, "a".into(), "".into())))
            .unwrap()
            .replace("\"variables\"", "");
        assert!(!empty.contains("bindings"));
        assert!(
            !serde_json::to_string(&Document::default())
                .unwrap()
                .contains("variables")
        );
    }

    /// Draws the chain icons, whole and broken, at a few sizes and colors, to the PNG
    /// named by `LINK_ICON_PREVIEW`.
    #[test]
    #[ignore]
    fn link_icon_preview() {
        let Ok(path) = std::env::var("LINK_ICON_PREVIEW") else {
            return;
        };
        crate::preview::render(&path, egui::vec2(260.0, 90.0), |ui| {
            for broken in [false, true] {
                ui.horizontal(|ui| {
                    for side in [14.0, 18.0, 24.0, 32.0] {
                        for color in [ui.visuals().text_color(), ui.visuals().warn_fg_color] {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                            paint_link(ui.painter(), rect, color, broken);
                        }
                    }
                });
            }
        });
    }
}
