//! Variables: named values kept with the project, each of a kind ([`Kind`]): a length,
//! an angle, a text size, a plain number or a color.
//!
//! Number fields are linked to formulas such as `button + 2` or `(panel - 2 * margin) / 3`,
//! which use variables of the field's kind and plain numbers. A linked field (`Field`)
//! keeps its formula in the layer's `bindings`. Each frame, [`apply`] sets every linked
//! field to its formula's value, so a changed variable moves or resizes all the layers
//! using it. A field changed some other way (a drag on the canvas, Align, a nudge,
//! another field's aspect lock) is unlinked: it keeps the value it was given. Typing a
//! plain number into a linked field unlinks it too. The fields, the formula editor and the
//! Variables panel are in `ui::variables`.
//!
//! Color buttons are linked to color variables by name (see `colors`).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::document::shape::{self, ShapeKind};
use crate::document::text;
use crate::document::{Document, Layer, Node};
use crate::i18n::tr;
use crate::render::effects::{color_from_hex, color_to_hex};

/// The kinds of variables a template's shapes can use: their fields are lengths and
/// angles.
pub const TEMPLATE_KINDS: [Kind; 3] = [Kind::Length, Kind::Angle, Kind::Number];

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
    pub fn new_value(self) -> Value {
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

    pub fn number_mut(&mut self) -> Option<&mut f32> {
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

/// A named value. Saved as `{"name": "button", "kind": "length", "value": 30}`, with
/// `"formula": "panel + 2"` when it inherits from the template's variables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SavedVariable")]
pub struct Variable {
    pub name: String,
    #[serde(flatten)]
    pub value: Value,
    /// For a project's length, angle, size or number: a formula using the template's
    /// variables, such as `panel + 2`, which [`apply`] keeps the value set to. Its value
    /// is the formula's last result, kept while the formula has an error (a template
    /// variable it uses is gone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
}

/// A variable as read from a project file. Variables of the first projects that had
/// them have no kind: they are numbers.
#[derive(Deserialize)]
struct SavedVariable {
    name: String,
    #[serde(default)]
    kind: Option<Kind>,
    value: serde_json::Value,
    #[serde(default)]
    formula: Option<String>,
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
            formula: saved.formula,
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
    /// 3D tilt around the horizontal axis, in degrees.
    TiltX,
    /// 3D tilt around the vertical axis, in degrees.
    TiltY,
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
            Field::Rotation | Field::TiltX | Field::TiltY => Kind::Angle,
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

/// What is wrong with `name` as a variable's name, in the current language, or `None`
/// if it is a valid one ([`is_valid_name`]) that no other variable has (`taken`).
pub fn name_problem(name: &str, taken: bool) -> Option<String> {
    if !is_valid_name(name) {
        Some(tr!("VARIABLE_NAME_INVALID").to_owned())
    } else if taken {
        Some(tr!("VARIABLE_NAME_TAKEN").to_owned())
    } else {
        None
    }
}

/// True if `name` can be a new variable's name: a lowercase letter or `_`, then
/// letters, digits, `_`. Formulas still read names starting with a capital, which
/// variables named before this rule may have.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| name_start(c) && !c.is_uppercase())
        && chars.all(name_char)
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

/// The names of the x and y of the template shape of SVG id `id`, as formulas use them:
/// `action-buttons-3` gives `action_buttons_3_x` and `action_buttons_3_y`.
pub fn point_names(id: &str) -> [String; 2] {
    let mut base: String = id
        .chars()
        .map(|c| if name_char(c) { c } else { '_' })
        .collect();
    if !base.starts_with(name_start) {
        base.insert(0, '_');
    }
    [format!("{base}_x"), format!("{base}_y")]
}

/// The family a template point's variable belongs to, for the formula editor to fold
/// them: `action_buttons` for `action_buttons_3_x`; `None` for a name not shaped so.
pub fn point_family(name: &str) -> Option<&str> {
    let base = name
        .strip_suffix("_x")
        .or_else(|| name.strip_suffix("_y"))?;
    let (family, number) = base.rsplit_once('_')?;
    (!family.is_empty() && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
        .then_some(family)
}

/// The formula linking a position to the variable `name`, `offset` mm away.
pub fn offset_formula(name: &str, offset: f32) -> String {
    let d = (offset * 100.0).round() / 100.0;
    if d == 0.0 {
        name.to_owned()
    } else if d > 0.0 {
        format!("{name} + {d}")
    } else {
        format!("{name} - {}", -d)
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
pub fn uses_variables(formula: &str) -> bool {
    tokenize(formula).is_ok_and(|t| t.iter().any(|t| matches!(t, Token::Name(_))))
}

/// True if `formula` names the variable `name`.
fn uses(formula: &str, name: &str) -> bool {
    tokenize(formula).is_ok_and(|t| t.contains(&Token::Name(name.to_owned())))
}

/// `formula` with the variable `old` renamed `new`, the rest as typed.
pub fn rename_in(formula: &str, old: &str, new: &str) -> String {
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
        Field::TiltX => Some(layer.transform.tilt_x_deg),
        Field::TiltY => Some(layer.transform.tilt_y_deg),
    }
}

/// `deg` brought into the rotation field's range, -180 to 180.
pub fn wrap_degrees(deg: f32) -> f32 {
    let d = (deg + 180.0).rem_euclid(360.0) - 180.0;
    if d == -180.0 && deg > 0.0 { 180.0 } else { d }
}

/// `deg` within the 3D tilt's range.
pub fn clamp_tilt(deg: f32) -> f32 {
    let max = crate::document::MAX_TILT_DEG;
    deg.clamp(-max, max)
}

/// Sets `field` to the value `v` (in the field's unit). Returns its new raw number.
pub fn set(layer: &mut Layer, field: Field, v: f32) -> Option<f32> {
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
        Field::TiltX => layer.transform.tilt_x_deg = clamp_tilt(v),
        Field::TiltY => layer.transform.tilt_y_deg = clamp_tilt(v),
    }
    raw(layer, field)
}

/// True if two raw numbers are the same but for rounding.
fn same(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
}

/// The result of the project variable `var`'s formula with the template's variables
/// `template`, or `None` if it has no formula. Only the template's variables can be
/// used: of the variable's kind, or numbers.
pub fn inherited(var: &Variable, template: &[Variable]) -> Option<Result<f64, FormulaError>> {
    let formula = var.formula.as_deref()?;
    Some(eval(formula, template, var.value.kind()))
}

/// Sets every project variable with a formula to its result; one with an error keeps
/// its value. Returns true if one changed.
fn apply_inherited(doc: &mut Document) -> bool {
    let mut changed = false;
    for var in &mut doc.variables {
        let Some(Ok(v)) = inherited(var, &doc.template_variables) else {
            continue;
        };
        if let Some(n) = var.value.number_mut()
            && !same(*n, v as f32)
        {
            *n = v as f32;
            changed = true;
        }
    }
    changed
}

/// Sets the project variables inheriting from the template's to their formula's value,
/// then every linked field to its formula's value, and unlinks those changed by hand
/// since the last call. A formula with an error leaves its field as it is. Returns true
/// if the document changed.
/// The layers `held` are left as they are, neither updated nor unlinked: a drag holds
/// them for a moment (see `view::CanvasState::held_layers`).
pub fn apply(doc: &mut Document, applied: &mut Applied, held: &[u64]) -> bool {
    let mut changed = apply_inherited(doc);
    let vars = doc.all_variables();
    doc.for_each_layer_mut(|layer| {
        if layer.bindings.is_empty() || held.contains(&layer.id) {
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
    let gradients = layer.gradients();
    let stops = gradients
        .iter()
        .flat_map(|g| &g.stops)
        .filter_map(|s| s.link.as_ref());
    for name in colors.chain(effects).chain(stops) {
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
    walk(&doc.layers, &doc.all_variables(), &mut out);
    out
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

/// `formula` plus `d`: a number it ends with (`col + 12`) takes `d` in, or else `+ d` is
/// added, and nothing for a zero.
pub fn shifted(formula: &str, d: f32) -> String {
    let formula = formula.trim();
    if d.abs() < 1e-4 {
        return formula.to_owned();
    }
    let number = |v: f32| {
        let s = format!("{:.4}", v.abs());
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    let with = |base: &str, c: f32| {
        if c.abs() < 1e-4 {
            base.to_owned()
        } else if base.is_empty() {
            format!("{}{}", if c < 0.0 { "-" } else { "" }, number(c))
        } else {
            format!("{base} {} {}", if c < 0.0 { "-" } else { "+" }, number(c))
        }
    };
    if let Ok(c) = formula.parse::<f32>() {
        return with("", c + d);
    }
    // The last term, when it is a number added or taken away outside any parentheses.
    let mut depth = 0;
    let mut last = None;
    for (i, c) in formula.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '+' | '-' if depth == 0 && i > 0 => last = Some(i),
            _ => {}
        }
    }
    if let Some(i) = last {
        let base = formula[..i].trim_end();
        let binary = base
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ')' || c == '.');
        if binary && let Ok(c) = formula[i + 1..].trim().parse::<f32>() {
            let c = if formula[i..].starts_with('-') { -c } else { c };
            return with(base, c + d);
        }
    }
    with(formula, d)
}

/// Moves the X and Y formulas of every layer by `d` mm, as when the layers are all
/// moved by `d`.
pub fn shift_positions(doc: &mut Document, d: [f32; 2]) {
    doc.for_each_layer_mut(|l| {
        for (field, k) in [(Field::X, 0), (Field::Y, 1)] {
            if let Some(f) = l.bindings.get_mut(&field) {
                *f = shifted(f, d[k]);
            }
        }
    });
}

/// How many fields and colors use the variable `name`.
pub fn use_count(doc: &mut Document, name: &str) -> usize {
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
        + doc
            .variables
            .iter()
            .filter(|v| v.formula.as_deref().is_some_and(|f| uses(f, name)))
            .count()
        + crate::document::colors::use_count(doc, name)
        + crate::document::text_links::use_count(doc, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_linked_to_a_template_point_follow_it() {
        assert_eq!(
            point_names("action-buttons-3"),
            [
                "action_buttons_3_x".to_owned(),
                "action_buttons_3_y".to_owned()
            ]
        );
        assert_eq!(point_names("3d")[0], "_3d_x");
        assert_eq!(point_family("action_buttons_12_x"), Some("action_buttons"));
        assert_eq!(point_family("menu_buttons_3_y"), Some("menu_buttons"));
        assert_eq!(point_family("display_x"), None);
        assert_eq!(point_family("ButtonSize"), None);
        assert_eq!(offset_formula("p_x", 0.001), "p_x");
        assert_eq!(offset_formula("p_x", 2.5), "p_x + 2.5");
        assert_eq!(offset_formula("p_y", -1.25), "p_y - 1.25");

        let mut layer = crate::document::shape::new_layer(
            1,
            crate::document::shape::ShapeKind::Ellipse,
            [0.0, 0.0],
        );
        layer.bindings.insert(Field::X, "b_1_x + 5".into());
        layer.bindings.insert(Field::Y, "b_1_y".into());
        let mut doc = Document {
            layers: vec![crate::document::Node::Layer(Box::new(layer))],
            template_points: vec![("b-1".into(), [40.0, 60.0])],
            ..Default::default()
        };
        let mut applied = Applied::new();
        assert!(apply(&mut doc, &mut applied, &[]));
        assert_eq!(doc.layer(1).unwrap().transform.center, [45.0, 60.0]);
        // The template changed: the shape moved, the layer goes with it.
        doc.template_points = vec![("b-1".into(), [50.0, 70.0])];
        assert!(apply(&mut doc, &mut applied, &[]));
        assert_eq!(doc.layer(1).unwrap().transform.center, [55.0, 70.0]);
        // Moved by a drag that holds it: neither unlinked nor put back; released, put back
        // by the drag, it is still linked.
        doc.layer_mut(1).unwrap().transform.center = [80.0, 90.0];
        assert!(!apply(&mut doc, &mut applied, &[1]));
        assert_eq!(doc.layer(1).unwrap().transform.center, [80.0, 90.0]);
        doc.layer_mut(1).unwrap().transform.center = [55.0, 70.0];
        apply(&mut doc, &mut applied, &[]);
        assert_eq!(doc.layer(1).unwrap().bindings.len(), 2);
        // The shape is gone: the layer stays, its links shown broken.
        doc.template_points.clear();
        assert!(!apply(&mut doc, &mut applied, &[]));
        assert_eq!(doc.layer(1).unwrap().transform.center, [55.0, 70.0]);
        assert_eq!(
            broken_links(doc.layer(1).unwrap(), &doc.all_variables()).len(),
            2
        );
    }

    #[test]
    fn project_layers_follow_their_template_variables() {
        let mut doc = Document {
            variables: vec![var("gap", Value::Length(2.0))],
            template_variables: vec![
                var("button", Value::Length(41.0)),
                // Named by a project variable too: the project's is used.
                var("gap", Value::Length(99.0)),
            ],
            ..Default::default()
        };
        let names: Vec<String> = doc.all_variables().into_iter().map(|v| v.name).collect();
        assert_eq!(names, ["gap", "button"]);
        let mut layer = crate::document::shape::new_layer(1, ShapeKind::Ellipse, [100.0, 80.0]);
        layer.bindings.insert(Field::Width, "button + gap".into());
        doc.layers.push(Node::Layer(Box::new(layer)));
        let mut applied = Applied::new();
        assert!(apply(&mut doc, &mut applied, &[]));
        let Node::Layer(l) = &doc.layers[0] else {
            unreachable!()
        };
        assert!((raw(l, Field::Width).unwrap() - 43.0).abs() < 1e-4);
        // Not saved with the project: only its own variable is.
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["variables"].as_array().unwrap().len(), 1);
        assert!(json.get("template_variables").is_none());
    }

    #[test]
    fn project_variables_inherit_from_the_template_ones() {
        let mut wide = var("wide", Value::Length(0.0));
        wide.formula = Some("button + 2".into());
        // Project variables can't be used: only the template's.
        let mut other = var("other", Value::Length(5.0));
        other.formula = Some("wide * 2".into());
        let mut doc = Document {
            variables: vec![wide, other],
            template_variables: vec![var("button", Value::Length(41.0))],
            ..Default::default()
        };
        let mut layer = crate::document::shape::new_layer(1, ShapeKind::Ellipse, [100.0, 80.0]);
        layer.bindings.insert(Field::Width, "wide".into());
        doc.layers.push(Node::Layer(Box::new(layer)));
        let mut applied = Applied::new();
        assert!(apply(&mut doc, &mut applied, &[]));
        assert_eq!(doc.variables[0].value, Value::Length(43.0));
        assert_eq!(doc.variables[1].value, Value::Length(5.0));
        assert_eq!(
            inherited(&doc.variables[1], &doc.template_variables),
            Some(Err(FormulaError::UnknownVariable("wide".into())))
        );
        let Node::Layer(l) = &doc.layers[0] else {
            unreachable!()
        };
        assert!((raw(l, Field::Width).unwrap() - 43.0).abs() < 1e-4);
        // The formula is saved and read back.
        let json = serde_json::to_string(&doc.variables[0]).unwrap();
        assert!(json.contains("\"formula\":\"button + 2\""), "{json}");
        let read: Variable = serde_json::from_str(&json).unwrap();
        assert_eq!(read, doc.variables[0]);
        let plain = serde_json::to_string(&var("gap", Value::Length(2.0))).unwrap();
        assert!(!plain.contains("formula"), "{plain}");
        // Another template without `button`: an error, and the last value is kept.
        doc.template_variables = vec![var("panel", Value::Length(10.0))];
        assert!(!apply(&mut doc, &mut applied, &[]));
        assert_eq!(doc.variables[0].value, Value::Length(43.0));
        assert_eq!(
            inherited(&doc.variables[0], &doc.template_variables),
            Some(Err(FormulaError::UnknownVariable("button".into())))
        );
        // Back with a new value: followed again.
        doc.template_variables = vec![var("button", Value::Length(20.0))];
        assert!(apply(&mut doc, &mut applied, &[]));
        assert_eq!(doc.variables[0].value, Value::Length(22.0));
    }

    #[test]
    fn shifted_formulas_take_the_offset_into_their_last_number() {
        assert_eq!(shifted("col", 0.0), "col");
        assert_eq!(shifted("col", 12.5), "col + 12.5");
        assert_eq!(shifted("col", -12.5), "col - 12.5");
        assert_eq!(shifted("col + 12.5", -12.5), "col");
        assert_eq!(shifted("col - 2", 5.0), "col + 3");
        assert_eq!(shifted("(a + 2) * b", 1.0), "(a + 2) * b + 1");
        assert_eq!(shifted("a + 2 * b", 1.0), "a + 2 * b + 1");
        assert_eq!(shifted("a / -2", 1.0), "a / -2 + 1");
        assert_eq!(shifted("40", -10.0), "30");
        let vars = vars();
        for f in [
            "button",
            "button * 2 - 3",
            "(button + 1) / 2",
            "-button + 4",
        ] {
            let before = eval(f, &vars, Kind::Length).unwrap();
            let after = eval(&shifted(f, 7.25), &vars, Kind::Length).unwrap();
            assert!((after - before - 7.25).abs() < 1e-4, "{f}");
        }
    }

    fn var(name: &str, value: Value) -> Variable {
        Variable {
            formula: None,
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
        assert!(is_valid_name("buttonSize") && !is_valid_name("Button"));
        for name in [
            "",
            "Button",
            "1a",
            "a b",
            "a-b",
            "-a",
            "_a1",
            "épaisseur",
            "Épaisseur",
        ] {
            assert_eq!(
                name_problem(name, false).is_none(),
                is_valid_name(name),
                "{name}"
            );
        }
        assert!(name_problem("a", true).is_some());
        assert!(!is_valid_name("Épaisseur"));
        // Still read in formulas.
        let v = [var("Button", Value::Length(3.0))];
        assert_eq!(eval("Button + 1", &v, Kind::Length), Ok(4.0));
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
        assert!(apply(&mut doc, &mut applied, &[]));
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
        assert!(!apply(&mut doc, &mut applied, &[]), "nothing more to do");

        doc.variables[0].value = Value::Length(40.0);
        assert!(apply(&mut doc, &mut applied, &[]));
        let l = doc.layer(1).unwrap();
        assert!(same(l.size_mm()[0], 42.0));
        assert_eq!(l.transform.center[0], 80.0);

        // Moved by hand: x is unlinked and stays; the width is still linked.
        doc.layer_mut(1).unwrap().transform.center[0] = 5.0;
        doc.variables[0].value = Value::Length(50.0);
        apply(&mut doc, &mut applied, &[]);
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
        apply(&mut doc, &mut Applied::new(), &[]);
        let shape = doc.layer(1).unwrap().shape.as_deref().unwrap();
        assert!(same(shape.geometry_size()[1], 30.0));
        assert_eq!(shape.stroke_width, 2.5);
        let max = shape.size[0].min(shape.size[1]) / 2.0;
        assert_eq!(shape.corner_radius, max, "clamped as in Properties");

        let mut t = text::new_layer(1, "Hi", "Sans".into(), [0.0, 0.0]);
        t.bindings.insert(Field::TextSize, "marge_é * 6".into());
        let mut doc = doc_with(t);
        apply(&mut doc, &mut Applied::new(), &[]);
        assert!(same(text::size_pt(doc.layer(1).unwrap()), 15.0));
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
    fn layers_with_links_to_variables_gone_are_found_and_their_groups_too() {
        let mut fine = Layer::new(1, "fine".into(), "".into());
        fine.bindings.insert(Field::X, "button + 2".into());
        fine.color_links
            .insert(crate::document::colors::ColorField::Fill, "ink".into());
        let mut gone = Layer::new(2, "gone".into(), "".into());
        gone.bindings.insert(Field::X, "margin * 2".into());
        // A color link to a variable that isn't a color, and a text link to none.
        gone.color_links
            .insert(crate::document::colors::ColorField::Fill, "button".into());
        let mut shadow = crate::render::effects::EffectInstance::new(
            crate::render::effects::registry()
                .get("builtin.drop_shadow")
                .unwrap(),
        );
        shadow.links.insert("color".into(), "margin".into());
        gone.effects.push(shadow);
        gone.text = Some(Box::new(crate::document::text::Text {
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
                    clip: false,
                    blend: crate::document::BlendMode::Normal,
                    expanded: false,
                    children: vec![Node::Layer(Box::new(gone))],
                    element: None,
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
}
