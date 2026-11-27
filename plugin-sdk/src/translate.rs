//! A plugin's texts in other languages, from catalogs: for each language, a JSON object
//! mapping the English texts the code gives (names, descriptions, parameter labels,
//! groups and choice labels) to that language's. Units are not translated.
//!
//! The app asks for the texts in its language (`"fr-FR"`, `"en-GB"`...), and shows the
//! English ones where a plugin has none.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{Contents, Param, ParamKind};

/// English text to translated text.
pub(crate) type Catalog = HashMap<String, String>;

/// Reads the catalog of `language`.
pub(crate) fn parse(language: &str, json: &str) -> Result<Catalog, String> {
    serde_json::from_str(json).map_err(|e| format!("translations for {language}: {e}"))
}

/// The catalog of `language` in `list` (by code): the same code, else one of the same
/// language (`fr` or `fr-FR` for `fr-CA`), ignoring case and `_` for `-`.
pub(crate) fn pick<'a, T>(list: &'a [(String, T)], language: &str) -> Option<&'a T> {
    let norm = |s: &str| s.replace('_', "-").to_ascii_lowercase();
    let want = norm(language);
    let base = |s: &str| s.split('-').next().unwrap_or_default().to_owned();
    (list.iter().find(|(l, _)| norm(l) == want))
        .or_else(|| list.iter().find(|(l, _)| base(&norm(l)) == base(&want)))
        .map(|(_, t)| t)
}

/// `text` in the catalog's language, itself when it has none.
fn tr(catalog: &Catalog, text: &str) -> String {
    match catalog.get(text) {
        Some(t) if !t.is_empty() => t.clone(),
        _ => text.to_owned(),
    }
}

/// The texts of `params` that can be translated.
fn param_texts(p: &Param) -> impl Iterator<Item = &str> {
    let options: Vec<&str> = match &p.kind {
        ParamKind::Choice { options, .. } => options.iter().map(|o| o.label.as_str()).collect(),
        _ => Vec::new(),
    };
    std::iter::once(p.label.as_str())
        .chain(p.group.as_deref())
        .chain(options)
}

/// `params` in the catalog's language.
fn translated(params: &[Param], catalog: &Catalog) -> Vec<Param> {
    let mut params = params.to_vec();
    for p in &mut params {
        p.label = tr(catalog, &p.label);
        p.group = p.group.as_deref().map(|g| tr(catalog, g));
        if let ParamKind::Choice { options, .. } = &mut p.kind {
            for o in options {
                o.label = tr(catalog, &o.label);
            }
        }
    }
    params
}

/// An effect's or a generator's id, name, description and parameters.
pub(crate) struct Texts {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
}

impl Texts {
    fn json(&self, catalog: &Catalog) -> Value {
        json!({
            "id": self.id,
            "name": tr(catalog, &self.name),
            "description": tr(catalog, &self.description),
            "params": translated(&self.params, catalog),
        })
    }

    /// Its texts to translate, the empty description left out.
    fn english(&self) -> impl Iterator<Item = &str> {
        let description = Some(self.description.as_str()).filter(|d| !d.is_empty());
        std::iter::once(self.name.as_str())
            .chain(description)
            .chain(self.params.iter().flat_map(param_texts))
    }
}

/// What the ABI's `texts` function gives: `{"effects": [...], "generators": [...]}`,
/// each with its id, name, description and parameters in the catalog's language.
pub(crate) fn describe(effects: &[Texts], generators: &[Texts], catalog: &Catalog) -> String {
    let list = |l: &[Texts]| l.iter().map(|t| t.json(catalog)).collect::<Vec<_>>();
    json!({"effects": list(effects), "generators": list(generators)}).to_string()
}

/// The effects' and generators' texts.
pub(crate) fn texts(contents: &Contents) -> (Vec<Texts>, Vec<Texts>) {
    let effects = (contents.0.iter())
        .map(|e| Texts {
            id: e.id().into(),
            name: e.name().into(),
            description: e.description().into(),
            params: e.params(),
        })
        .collect();
    let generators = (contents.1.iter())
        .map(|g| Texts {
            id: g.id().into(),
            name: g.name().into(),
            description: g.description().into(),
            params: g.params(),
        })
        .collect();
    (effects, generators)
}

/// The English texts of `contents` that the catalog `json` doesn't translate, each
/// once, for a plugin's tests:
///
/// ```ignore
/// let contents = (vec![Box::new(Fade) as Box<dyn Effect>], vec![]);
/// let missing = untranslated(contents, include_str!("../locales/fr-FR.json")).unwrap();
/// assert!(missing.is_empty(), "{missing:?}");
/// ```
///
/// An error if the catalog is not a JSON object of strings.
pub fn untranslated(contents: Contents, json: &str) -> Result<Vec<String>, String> {
    let catalog = parse("the catalog", json)?;
    let (effects, generators) = texts(&contents);
    let mut missing: Vec<String> = Vec::new();
    for text in effects.iter().chain(&generators).flat_map(Texts::english) {
        if !catalog.get(text).is_some_and(|t| !t.is_empty()) && !missing.iter().any(|m| m == text) {
            missing.push(text.to_owned());
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_closest_language_is_picked() {
        let list = [
            ("fr-FR".to_owned(), 1),
            ("en-GB".to_owned(), 2),
            ("de".to_owned(), 3),
        ];
        assert_eq!(pick(&list, "fr-FR"), Some(&1));
        assert_eq!(pick(&list, "fr_CA"), Some(&1));
        assert_eq!(pick(&list, "EN-gb"), Some(&2));
        assert_eq!(pick(&list, "de-AT"), Some(&3));
        // en-GB is not U.S. English, but the closest there is.
        assert_eq!(pick(&list, "en-US"), Some(&2));
        assert_eq!(pick(&list, "it-IT"), None);
    }

    #[test]
    fn labels_groups_and_choices_are_translated_not_units_or_ids() {
        let catalog: Catalog = [
            ("Size", "Taille"),
            ("Shape", "Forme"),
            ("Dots", "Points"),
            ("Look", "Aspect"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .into();
        let params = vec![
            Param::float("size", "Size", 0.0, 1.0, 0.5)
                .unit("mm")
                .group("Look"),
            Param::choice(
                "shape",
                "Shape",
                &[("dot", "Dots"), ("line", "Lines")],
                "dot",
            ),
        ];
        let t = translated(&params, &catalog);
        assert_eq!(
            (t[0].label.as_str(), t[0].group.as_deref()),
            ("Taille", Some("Aspect"))
        );
        assert!(matches!(&t[0].kind, ParamKind::Float { unit, .. } if unit == "mm"));
        let ParamKind::Choice { options, .. } = &t[1].kind else {
            panic!()
        };
        assert_eq!(
            (options[0].id.as_str(), options[0].label.as_str()),
            ("dot", "Points")
        );
        // None given: the English one.
        assert_eq!(options[1].label, "Lines");
    }
}
