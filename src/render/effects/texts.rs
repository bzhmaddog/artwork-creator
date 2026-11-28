//! What plugins' effects and generators show in each language of the app: the texts
//! a plugin gives when asked for that language (see [`LANGUAGES`]), those of its code
//! where it gives none.

use serde_json::Value;

use super::{ParamKind, ParamSpec};
use crate::i18n::{self, Language};

/// The languages plugins are asked for, with the codes they are asked by: every one of
/// the app's. U.S. English comes first: the others start from its texts.
pub const LANGUAGES: [(Language, &str); 3] = [
    (Language::English, "en-US"),
    (Language::BritishEnglish, "en-GB"),
    (Language::French, "fr-FR"),
];

/// An effect's or a generator's name, description and parameters, in one language.
#[derive(Clone, Debug)]
pub struct Shown {
    pub name: String,
    pub description: String,
    pub params: Vec<ParamSpec>,
}

/// The same, in each language of the app.
pub struct Texts(Vec<Shown>);

impl Texts {
    /// The texts of the plugin's code, `english`, in every language until set.
    pub fn new(english: Shown) -> Self {
        Self(vec![english; Language::ALL.len()])
    }

    /// In the language of the interface.
    pub fn current(&self) -> &Shown {
        self.get(i18n::current())
    }

    pub fn get(&self, language: Language) -> &Shown {
        &self.0[language as usize]
    }

    /// Takes the texts of `other`, the same effect or generator in `language`: its name
    /// and description when not empty, its parameters' labels, groups and choice labels,
    /// matched by id. All the rest (ids, types, ranges, defaults, units) stays that of
    /// the plugin's code. U.S. English set, the others start from it.
    pub fn set(&mut self, language: Language, other: &Shown) {
        let mut shown = self.0[Language::English as usize].clone();
        if !other.name.is_empty() {
            shown.name = other.name.clone();
        }
        if !other.description.is_empty() {
            shown.description = other.description.clone();
        }
        for p in &mut shown.params {
            let Some(o) = other.params.iter().find(|o| o.id == p.id) else {
                continue;
            };
            if !o.label.is_empty() {
                p.label = o.label.clone();
            }
            if p.group.is_some() && o.group.is_some() {
                p.group = o.group.clone();
            }
            if let (
                ParamKind::Choice { options, .. },
                ParamKind::Choice {
                    options: theirs, ..
                },
            ) = (&mut p.kind, &o.kind)
            {
                for (id, label) in options {
                    if let Some((_, l)) = theirs.iter().find(|(i, l)| i == id && !l.is_empty()) {
                        *label = l.clone();
                    }
                }
            }
        }
        self.0[language as usize] = shown;
    }
}

/// Sets the texts in `language` of the plugin's effects or generators (`list`,
/// `"effects"` or `"generators"`), by id, from `texts`, what the plugin gave for it:
/// `{"effects": [{"id", "name", "description", "params"}], "generators": [...]}`. An
/// entry whose parameters don't read is left out (logged).
pub fn read<'a>(
    texts: &Value,
    list: &str,
    language: Language,
    items: impl IntoIterator<Item = (&'a str, &'a mut Texts)>,
) {
    let entries = texts.get(list).and_then(Value::as_array);
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    for (id, t) in items {
        let Some(e) = entries.into_iter().flatten().find(|e| text(e, "id") == id) else {
            continue;
        };
        let params = e.get("params").map_or("[]".to_owned(), Value::to_string);
        match super::plugin::param_specs(&params) {
            Ok(params) => t.set(
                language,
                &Shown {
                    name: text(e, "name"),
                    description: text(e, "description"),
                    params,
                },
            ),
            Err(err) => log::warn!("{id}: texts in {language:?}: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> Shown {
        Shown {
            name: "Dots".into(),
            description: "Even dots".into(),
            params: vec![
                ParamSpec::float("size", "Size", 0.0, 10.0, 2.0).unit("mm"),
                ParamSpec::choice("shape", "Shape", &[("dot", "Dot"), ("line", "Line")], "dot"),
            ],
        }
    }

    #[test]
    fn a_language_takes_only_the_texts() {
        let mut t = Texts::new(english());
        let json = serde_json::json!({"effects": [{
            "id": "x.dots", "name": "Points", "description": "",
            "params": [
                // Another range and unit: not taken.
                {"id": "size", "label": "Taille", "type": "float", "min": 5, "max": 6,
                 "default": 5, "unit": "cm"},
                {"id": "shape", "label": "Forme", "type": "choice", "default": "dot",
                 "options": [{"id": "dot", "label": "Point"}]},
                {"id": "extra", "label": "En trop", "type": "bool", "default": true}
            ]
        }]});
        read(&json, "effects", Language::French, [("x.dots", &mut t)]);
        let fr = &t.0[Language::French as usize];
        assert_eq!(
            (fr.name.as_str(), fr.description.as_str()),
            ("Points", "Even dots")
        );
        assert_eq!(fr.params.len(), 2);
        assert_eq!(fr.params[0].label, "Taille");
        assert!(
            matches!(&fr.params[0].kind, ParamKind::Float { max, unit, .. } if *max == 10.0 && unit == "mm")
        );
        let ParamKind::Choice { options, .. } = &fr.params[1].kind else {
            panic!()
        };
        assert_eq!(
            options,
            &[
                ("dot".into(), "Point".into()),
                ("line".into(), "Line".into())
            ]
        );
        // The others stay English.
        assert_eq!(t.0[Language::BritishEnglish as usize].name, "Dots");
        // The current language's.
        assert_eq!(t.current().name, "Dots");
    }

    #[test]
    fn bad_texts_are_left_out() {
        let mut t = Texts::new(english());
        let json =
            serde_json::json!({"generators": [{"id": "x.dots", "name": "Points", "params": 3}]});
        read(&json, "generators", Language::French, [("x.dots", &mut t)]);
        assert_eq!(t.0[Language::French as usize].name, "Dots");
    }
}
