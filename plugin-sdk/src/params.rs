//! Parameters: how a plugin describes them, and how it reads their values.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A parameter of an effect. The app builds its control from it and saves its value
/// with the project.
///
/// In the ABI, an effect's parameters are a JSON array of these, e.g.
/// ```json
/// [
///   { "id": "size", "label": "Size", "type": "float", "min": 1, "max": 50,
///     "default": 8, "unit": "px", "decimals": 1 },
///   { "id": "count", "label": "Count", "type": "int", "min": 1, "max": 10, "default": 3 },
///   { "id": "soft", "label": "Soft edges", "type": "bool", "default": true },
///   { "id": "ink", "label": "Ink", "type": "color", "default": "#000000ff" },
///   { "id": "shape", "label": "Shape", "type": "choice", "default": "dot",
///     "options": [ { "id": "dot", "label": "Dots" }, { "id": "line", "label": "Lines" } ] }
/// ]
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Param {
    /// Key in the saved values.
    pub id: String,
    /// Shown next to the control.
    pub label: String,
    #[serde(flatten)]
    pub kind: ParamKind,
}

/// What a parameter holds, with its range and default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParamKind {
    Float {
        min: f32,
        max: f32,
        default: f32,
        /// Shown after the value, e.g. "px" or "%".
        #[serde(default)]
        unit: String,
        #[serde(default = "one")]
        decimals: usize,
    },
    Int {
        min: i32,
        max: i32,
        default: i32,
        #[serde(default)]
        unit: String,
    },
    Bool {
        default: bool,
    },
    /// Straight RGBA as `#rrggbbaa` (or `#rrggbb`, opaque).
    Color {
        default: String,
    },
    /// One of several options, saved by id.
    Choice {
        options: Vec<ChoiceOption>,
        default: String,
    },
}

/// An option of a choice parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// Saved value.
    pub id: String,
    /// Shown in the list.
    pub label: String,
}

fn one() -> usize {
    1
}

impl Param {
    fn new(id: &str, label: &str, kind: ParamKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
        }
    }

    pub fn float(id: &str, label: &str, min: f32, max: f32, default: f32) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Float {
                min,
                max,
                default,
                unit: String::new(),
                decimals: 1,
            },
        )
    }

    pub fn int(id: &str, label: &str, min: i32, max: i32, default: i32) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Int {
                min,
                max,
                default,
                unit: String::new(),
            },
        )
    }

    pub fn bool(id: &str, label: &str, default: bool) -> Self {
        Self::new(id, label, ParamKind::Bool { default })
    }

    /// `default` is straight RGBA.
    pub fn color(id: &str, label: &str, default: [u8; 4]) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Color {
                default: color_to_hex(default),
            },
        )
    }

    /// `options` are (id, label) pairs.
    pub fn choice(id: &str, label: &str, options: &[(&str, &str)], default: &str) -> Self {
        Self::new(
            id,
            label,
            ParamKind::Choice {
                options: options
                    .iter()
                    .map(|(id, label)| ChoiceOption {
                        id: (*id).into(),
                        label: (*label).into(),
                    })
                    .collect(),
                default: default.into(),
            },
        )
    }

    /// Sets the unit shown after a number.
    pub fn unit(mut self, u: &str) -> Self {
        match &mut self.kind {
            ParamKind::Float { unit, .. } | ParamKind::Int { unit, .. } => *unit = u.into(),
            _ => {}
        }
        self
    }

    /// Sets the decimals shown for a float.
    pub fn decimals(mut self, n: usize) -> Self {
        if let ParamKind::Float { decimals, .. } = &mut self.kind {
            *decimals = n;
        }
        self
    }
}

/// `#rrggbbaa`.
pub fn color_to_hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
}

/// Parses `#rrggbb` or `#rrggbbaa`.
pub fn color_from_hex(s: &str) -> Option<[u8; 4]> {
    let s = s.strip_prefix('#')?;
    if !s.is_ascii() || (s.len() != 6 && s.len() != 8) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    let a = if s.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, a])
}

/// The values of an effect's parameters, as the app passes them: a JSON object with
/// every parameter, already checked against its range (a float or int is a number in
/// range, a color a `#rrggbbaa` string, a choice one of its option ids).
///
/// The getters return a neutral value (0, false, opaque black, "") for an unknown id.
#[derive(Clone, Debug, Default)]
pub struct Values(Map<String, Value>);

impl Values {
    /// Parses the JSON object passed by the app.
    pub fn from_json(json: &str) -> Result<Self, String> {
        match serde_json::from_str(json) {
            Ok(Value::Object(map)) => Ok(Self(map)),
            Ok(_) => Err("parameter values are not a JSON object".into()),
            Err(e) => Err(format!("invalid parameter values: {e}")),
        }
    }

    pub fn f32(&self, id: &str) -> f32 {
        self.0.get(id).and_then(Value::as_f64).unwrap_or(0.0) as f32
    }

    pub fn i32(&self, id: &str) -> i32 {
        self.0
            .get(id)
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .round() as i32
    }

    pub fn bool(&self, id: &str) -> bool {
        self.0.get(id).and_then(Value::as_bool).unwrap_or(false)
    }

    /// Straight RGBA.
    pub fn color(&self, id: &str) -> [u8; 4] {
        self.0
            .get(id)
            .and_then(Value::as_str)
            .and_then(color_from_hex)
            .unwrap_or([0, 0, 0, 255])
    }

    /// Id of the chosen option.
    pub fn choice(&self, id: &str) -> &str {
        self.0.get(id).and_then(Value::as_str).unwrap_or("")
    }

    /// The raw JSON object.
    pub fn json(&self) -> &Map<String, Value> {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_serialize_to_the_documented_json() {
        let p = Param::choice("shape", "Shape", &[("dot", "Dots")], "dot");
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "shape", "label": "Shape", "type": "choice", "default": "dot",
                "options": [{ "id": "dot", "label": "Dots" }]
            })
        );
        let f: Param = serde_json::from_str(
            r#"{"id":"r","label":"R","type":"float","min":0,"max":5,"default":1}"#,
        )
        .unwrap();
        assert_eq!(f, Param::float("r", "R", 0.0, 5.0, 1.0));
    }

    #[test]
    fn values_read_typed() {
        let v =
            Values::from_json(r##"{"r":2.5,"n":3,"b":true,"c":"#ff000080","m":"dot"}"##).unwrap();
        assert_eq!(
            (v.f32("r"), v.i32("n"), v.bool("b"), v.choice("m")),
            (2.5, 3, true, "dot")
        );
        assert_eq!(v.color("c"), [255, 0, 0, 128]);
        assert_eq!(v.f32("missing"), 0.0);
        assert!(Values::from_json("[]").is_err());
    }
}
