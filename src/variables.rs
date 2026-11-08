//! Variables: named numbers kept with the project, which layer fields can be linked to
//! through formulas such as `button + 2` or `(panel - 2 * margin) / 3`.
//!
//! A linked field (`Field`) keeps its formula in the layer's `bindings`. Each frame,
//! [`apply`] sets every linked field to its formula's value, so a changed variable
//! moves or resizes all the layers using it. A field changed some other way (a drag on
//! the canvas, Align, a nudge, another field's aspect lock) is unlinked: it keeps the
//! value it was given. Typing a plain number into a linked field unlinks it too.
//!
//! Variables have no unit: a field reads the value in its own (mm, or pt for text size).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use egui::{DragValue, Id, Response, RichText, Ui};
use serde::{Deserialize, Serialize};

use crate::document::{Document, Layer, Node};
use crate::i18n::{tr, tr_n};
use crate::shape::{self, ShapeKind};
use crate::text;

/// A named number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: f32,
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
    DivisionByZero,
}

impl FormulaError {
    /// The error in the current language.
    pub fn message(&self) -> String {
        match self {
            FormulaError::Syntax => tr!("FORMULA_SYNTAX_ERROR").to_owned(),
            FormulaError::UnknownVariable(name) => tr!("FORMULA_UNKNOWN_VARIABLE", name = name),
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
            Token::Name(name) => self
                .vars
                .iter()
                .find(|v| v.name == *name)
                .map(|v| v.value as f64)
                .ok_or_else(|| FormulaError::UnknownVariable(name.clone())),
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

/// The value of `formula` with these variables.
pub fn eval(formula: &str, vars: &[Variable]) -> Result<f64, FormulaError> {
    let tokens = tokenize(formula)?;
    let mut p = Parser {
        tokens: &tokens,
        at: 0,
        vars,
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
    }
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
            let Ok(v) = eval(&layer.bindings[&field], &vars) else {
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

fn published(ctx: &egui::Context) -> Arc<Vec<Variable>> {
    ctx.data(|d| d.get_temp(vars_id())).unwrap_or_default()
}

/// A number field that can be linked to a formula. Unlinked, it is the `DragValue`
/// made by `make`, which also takes a formula: one naming a variable links the field.
/// Linked, it shows the formula, to edit; a plain number typed in unlinks it. The
/// response is changed when `value` was changed here; [`apply`] keeps linked fields up
/// to date.
pub fn field<'a>(
    ui: &mut Ui,
    bindings: &mut Bindings,
    field: Field,
    value: &'a mut f32,
    make: impl FnOnce(DragValue<'a>) -> DragValue<'a>,
) -> Response {
    let vars = published(ui.ctx());
    if let Some(formula) = bindings.get_mut(&field) {
        let result = eval(formula, &vars);
        let color = match result {
            Ok(_) => ui.visuals().hyperlink_color,
            Err(_) => ui.visuals().error_fg_color,
        };
        let width = ui.spacing().interact_size.x * 1.6;
        let mut response = ui.add(
            egui::TextEdit::singleline(formula)
                .text_color(color)
                .desired_width(width),
        );
        let hint = match &result {
            Ok(v) => tr!("LINKED_FIELD_HINT", value = format!("{v:.2}")),
            Err(e) => e.message(),
        };
        response = response.on_hover_text(hint);
        if response.lost_focus() {
            let text = formula.trim().to_owned();
            if text.is_empty() {
                bindings.remove(&field);
            } else if !uses_variables(&text)
                && let Ok(v) = eval(&text, &vars)
            {
                bindings.remove(&field);
                *value = v as f32;
                response.mark_changed();
            }
        }
        return response;
    }

    let typed: RefCell<Option<String>> = RefCell::new(None);
    let parser_vars = vars.clone();
    let drag = make(DragValue::new(value)).custom_parser(|s| {
        let v = eval(s, &parser_vars).ok()?;
        if uses_variables(s) {
            *typed.borrow_mut() = Some(s.trim().to_owned());
        }
        Some(v)
    });
    let response = ui.add(drag);
    if let Some(formula) = typed.into_inner() {
        bindings.insert(field, formula);
    }
    response
}

/// The Variables section's body: each variable with its name, value and a remove
/// button, then an Add button. Renaming a variable renames it in the formulas;
/// removing one unlinks the fields using it, which keep their value. Returns true if
/// the document changed.
pub fn variables_ui(ui: &mut Ui, doc: &mut Document) -> bool {
    let mut changed = false;
    if doc.variables.is_empty() {
        ui.label(RichText::new(tr!("VARIABLES_EMPTY")).weak());
    }
    let mut remove = None;
    let mut rename = None;
    let names: Vec<String> = doc.variables.iter().map(|v| v.name.clone()).collect();
    let used: Vec<usize> = names.iter().map(|n| use_count(doc, n)).collect();
    for (i, var) in doc.variables.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            // The name is edited in a draft, applied when the field is left, so the
            // formulas follow from one valid name to the next.
            let id = ui.scope_id().with(("variable_name", i));
            let mut draft = ui
                .data(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| var.name.clone());
            let taken = |n: &str| names.iter().enumerate().any(|(j, m)| j != i && m == n);
            let ok = is_valid_name(&draft) && !taken(&draft);
            let mut edit = egui::TextEdit::singleline(&mut draft)
                .desired_width(ui.available_width() * 0.5)
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
            changed |= ui
                .add(DragValue::new(&mut var.value).speed(0.1).max_decimals(3))
                .changed();
            if ui
                .add(egui::Button::new("🗑").frame(false))
                .on_hover_text(tr!("REMOVE_VARIABLE"))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some((old, new)) = rename {
        doc.for_each_layer_mut(|l| {
            for f in l.bindings.values_mut() {
                *f = rename_in(f, &old, &new);
            }
        });
    }
    if let Some(i) = remove {
        let gone = doc.variables.remove(i).name;
        doc.for_each_layer_mut(|l| l.bindings.retain(|_, f| !uses(f, &gone)));
        changed = true;
    }
    if ui.button(tr!("ADD_VARIABLE")).clicked() {
        let name = (1..)
            .map(|n| format!("var{n}"))
            .find(|n| doc.variables.iter().all(|v| v.name != *n))
            .expect("an unused name");
        doc.variables.push(Variable { name, value: 10.0 });
        changed = true;
    }
    changed
}

/// How many fields use the variable `name`.
fn use_count(doc: &Document, name: &str) -> usize {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vec<Variable> {
        vec![
            Variable {
                name: "button".into(),
                value: 30.0,
            },
            Variable {
                name: "marge_é".into(),
                value: 2.5,
            },
        ]
    }

    #[test]
    fn formulas() {
        let v = vars();
        let e = |s: &str| eval(s, &v);
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
        assert!(!apply(&mut doc, &mut applied), "nothing more to do");

        doc.variables[0].value = 40.0;
        assert!(apply(&mut doc, &mut applied));
        let l = doc.layer(1).unwrap();
        assert!(same(l.size_mm()[0], 42.0));
        assert_eq!(l.transform.center[0], 80.0);

        // Moved by hand: x is unlinked and stays; the width is still linked.
        doc.layer_mut(1).unwrap().transform.center[0] = 5.0;
        doc.variables[0].value = 50.0;
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
        t.bindings.insert(Field::TextSize, "button / 2".into());
        let mut doc = doc_with(t);
        apply(&mut doc, &mut Applied::new());
        assert!(same(text::size_pt(doc.layer(1).unwrap()), 15.0));
    }

    /// Clicks the field, selects its text, types `text` and presses Enter, a frame per
    /// step. Returns whether the field reported a change.
    fn type_in(ctx: &egui::Context, bindings: &mut Bindings, value: &mut f32, text: &str) -> bool {
        let doc = Document {
            variables: vars(),
            ..Default::default()
        };
        let mut changed = false;
        let p = egui::pos2(20.0, 10.0);
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
