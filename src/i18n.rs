//! Translations of the user interface.
//!
//! The code names each text by a key that says what it is for, such as
//! `ADD_TEXT_LAYER`, and looks it up in the current language's table when shown. Tables
//! live in `locales/<code>.json` and are built into the program: `en-US.json` holds the
//! English of every key, and is used for a key missing from another table.
//!
//! - [`tr!`] translates a key, and fills in `{name}` placeholders:
//!   `tr!("CANNOT_OPEN_FILE", path = p.display(), e = e)`.
//! - [`tr_n!`] picks the singular or plural text for a count, given as `{n}`: the
//!   tables hold `KEY_ONE` and `KEY_OTHER`, and the code asks for `tr_n!("KEY", count)`.
//! - [`tr_noop!`] marks a key where it is defined (e.g. a constant), for [`translate`] to
//!   look it up where it is shown.
//!
//! The tests check that every key of the code is in `en-US.json`, that the French table
//! is complete, that no table has leftovers, and that placeholders match. The British
//! English table only holds the texts spelled differently (colour, centre, grey...).

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// Key in eframe's storage.
const STORAGE_KEY: &str = "language";

/// A language of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    /// U.S. English, the language of the code.
    English = 0,
    /// French from France, also used for other French locales (fr-CA...).
    French = 1,
    BritishEnglish = 2,
}

impl Language {
    pub const ALL: [Language; 3] = [
        Language::English,
        Language::BritishEnglish,
        Language::French,
    ];

    /// The language's own name, as shown in the Language menu.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English (US)",
            Language::BritishEnglish => "English (UK)",
            Language::French => "Français (France)",
        }
    }

    /// The supported language of a locale such as `fr-FR`, `en_GB.UTF-8` or `fr_CA`;
    /// U.S. English for any other.
    pub fn from_locale(locale: &str) -> Language {
        let mut parts = locale.split(['-', '_', '.', '@']);
        let code = parts.next().unwrap_or("").to_ascii_lowercase();
        let region = parts.next().unwrap_or("").to_ascii_uppercase();
        match (code.as_str(), region.as_str()) {
            ("fr", _) => Language::French,
            ("en", "GB" | "UK") => Language::BritishEnglish,
            _ => Language::English,
        }
    }

    /// The operating system's language, if supported, otherwise English.
    pub fn system() -> Language {
        sys_locale::get_locale()
            .map(|l| Language::from_locale(&l))
            .unwrap_or(Language::English)
    }

    /// The language's texts, by key.
    fn table(self) -> &'static HashMap<&'static str, &'static str> {
        static US: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
        static FR: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
        static GB: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
        match self {
            Language::English => US.get_or_init(|| parse_table(US_ENGLISH)),
            Language::French => FR.get_or_init(|| parse_table(FRENCH)),
            Language::BritishEnglish => GB.get_or_init(|| parse_table(BRITISH_ENGLISH)),
        }
    }

    /// Whether `n` takes the singular: only 1 in English, 0 and 1 in French.
    fn is_singular(self, n: u64) -> bool {
        match self {
            Language::English | Language::BritishEnglish => n == 1,
            Language::French => n <= 1,
        }
    }
}

/// The language setting: follow the system, or a chosen language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preference {
    #[default]
    System,
    English,
    French,
    BritishEnglish,
}

impl Preference {
    pub fn language(self) -> Language {
        match self {
            Preference::System => Language::system(),
            Preference::English => Language::English,
            Preference::French => Language::French,
            Preference::BritishEnglish => Language::BritishEnglish,
        }
    }

    /// The setting saved by a previous session, or System.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default()
    }

    pub fn save(self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self);
    }
}

impl From<Language> for Preference {
    fn from(l: Language) -> Self {
        match l {
            Language::English => Preference::English,
            Language::French => Preference::French,
            Language::BritishEnglish => Preference::BritishEnglish,
        }
    }
}

const US_ENGLISH: &str = include_str!("../locales/en-US.json");
const FRENCH: &str = include_str!("../locales/fr-FR.json");
const BRITISH_ENGLISH: &str = include_str!("../locales/en-GB.json");

/// Reads a `locales/*.json` table: an object of keys to texts.
fn parse_table(json: &'static str) -> HashMap<&'static str, &'static str> {
    // Leaked once per language, so lookups hand out `&'static str`.
    let map: HashMap<String, String> = serde_json::from_str(json).unwrap_or_else(|e| {
        log::error!("invalid translation table: {e}");
        HashMap::new()
    });
    map.into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (&*k.leak(), &*v.leak()))
        .collect()
}

static CURRENT: AtomicU8 = AtomicU8::new(Language::English as u8);

/// The language texts are shown in.
pub fn current() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Language::French,
        2 => Language::BritishEnglish,
        _ => Language::English,
    }
}

/// Shows texts in `lang` from now on.
pub fn set(lang: Language) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// The text of `key` in the current language, else in U.S. English. Anything that is
/// not a key, such as a name from a template, is returned as it is.
pub fn translate(key: &str) -> &str {
    lookup(current(), key).unwrap_or(key)
}

/// The singular or plural text of `key` for `n` in the current language: `KEY_ONE` or
/// `KEY_OTHER`, else in U.S. English, else `key` itself.
pub fn translate_n(key: &'static str, n: u64) -> &'static str {
    let lang = current();
    let suffix = if lang.is_singular(n) {
        "_ONE"
    } else {
        "_OTHER"
    };
    lookup(lang, &format!("{key}{suffix}")).unwrap_or(key)
}

/// The text of `key` in `lang`, else in U.S. English.
fn lookup(lang: Language, key: &str) -> Option<&'static str> {
    lang.table()
        .get(key)
        .or_else(|| Language::English.table().get(key))
        .copied()
}

/// Replaces each `{name}` in `text` with its value; other braces are kept.
pub fn fill(text: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let value = after.find('}').and_then(|end| {
            let name = &after[..end];
            args.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (end, v.to_string()))
        });
        match value {
            Some((end, v)) => {
                out.push_str(&v);
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Translates a key; with `name = value` arguments, fills in its placeholders and
/// gives a `String`.
macro_rules! tr {
    ($text:literal) => {
        $crate::i18n::translate($text)
    };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::translate($text),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

/// The singular or plural text of a key for a count, translated, with `{n}` and any
/// other `name = value` placeholders filled in.
macro_rules! tr_n {
    ($key:literal, $n:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let n = $n;
        $crate::i18n::fill(
            $crate::i18n::translate_n($key, n as u64),
            &[("n", &n as &dyn ::std::fmt::Display)
                $(, (stringify!($name), &$value as &dyn ::std::fmt::Display))*],
        )
    }};
}

/// Marks a key for translation without translating it; see [`translate`].
macro_rules! tr_noop {
    ($text:literal) => {
        $text
    };
}

pub(crate) use {tr, tr_n, tr_noop};

/// View > Language entries: System, then each language in its own name.
pub fn menu(ui: &mut egui::Ui, pref: &mut Preference) {
    let before = *pref;
    ui.radio_value(pref, Preference::System, tr!("FOLLOW_SYSTEM"))
        .on_hover_text(tr!("FOLLOW_SYSTEM_LANGUAGE_HINT"));
    for lang in Language::ALL {
        ui.radio_value(pref, lang.into(), lang.native_name());
    }
    if *pref != before {
        set(pref.language());
        ui.ctx().request_repaint();
    }
}

/// How long ago `t` was, e.g. "5 minutes ago", "just now" under a minute.
pub fn time_ago(t: std::time::SystemTime) -> String {
    let secs = t.elapsed().map_or(0, |d| d.as_secs());
    let (minutes, hours, days) = (secs / 60, secs / 3600, secs / 86400);
    if minutes < 1 {
        tr!("TIME_AGO_JUST_NOW").into()
    } else if hours < 1 {
        tr_n!("TIME_AGO_MINUTES", minutes)
    } else if days < 1 {
        tr_n!("TIME_AGO_HOURS", hours)
    } else {
        tr_n!("TIME_AGO_DAYS", days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn times_ago_read_naturally() {
        use std::time::{Duration, SystemTime};
        set(Language::English);
        let ago = |secs: u64| time_ago(SystemTime::now() - Duration::from_secs(secs));
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(5 * 60), "5 minutes ago");
        assert_eq!(ago(3 * 3600), "3 hours ago");
        assert_eq!(ago(2 * 86400), "2 days ago");
    }
    use std::path::Path;

    #[test]
    fn locales_map_to_languages() {
        assert_eq!(Language::from_locale("fr-FR"), Language::French);
        assert_eq!(Language::from_locale("fr_CA.UTF-8"), Language::French);
        assert_eq!(Language::from_locale("FR"), Language::French);
        assert_eq!(Language::from_locale("en-US"), Language::English);
        assert_eq!(Language::from_locale("en"), Language::English);
        assert_eq!(Language::from_locale("en-GB"), Language::BritishEnglish);
        assert_eq!(
            Language::from_locale("en_GB.UTF-8"),
            Language::BritishEnglish
        );
        assert_eq!(Language::from_locale("de-DE"), Language::English);
        assert_eq!(Language::from_locale("C.UTF-8"), Language::English);
        assert_eq!(Language::from_locale(""), Language::English);
    }

    #[test]
    fn fill_replaces_named_placeholders() {
        let n = 3;
        assert_eq!(
            fill("{n} of {total}", &[("n", &n), ("total", &"5")]),
            "3 of 5"
        );
        // Unknown names and lone braces stay as they are.
        assert_eq!(fill("{x} {", &[("n", &n)]), "{x} {");
        assert_eq!(fill("é{n}é", &[("n", &n)]), "é3é");
    }

    #[test]
    fn plural_rules() {
        assert!(Language::English.is_singular(1));
        assert!(!Language::English.is_singular(0));
        assert!(Language::French.is_singular(0));
        assert!(Language::French.is_singular(1));
        assert!(!Language::French.is_singular(2));
    }

    /// The text of the Rust string literal at the start of `s`, and the rest after it.
    fn string_literal(s: &str) -> Option<(String, &str)> {
        let body = s.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, &body[i + 1..])),
                '\\' => match chars.next()? {
                    (_, 'n') => out.push('\n'),
                    (_, 't') => out.push('\t'),
                    (_, '\\') => out.push('\\'),
                    (_, '"') => out.push('"'),
                    (_, '\'') => out.push('\''),
                    (_, '0') => out.push('\0'),
                    // Line continuation: skips the newline and the next line's indent.
                    (j, '\n' | '\r') => {
                        let rest = &body[j + 1..];
                        let skip = rest.len() - rest.trim_start().len();
                        for _ in 0..rest[..skip].chars().count() {
                            chars.next();
                        }
                    }
                    (_, other) => panic!("unsupported escape \\{other} in a translated text"),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// The texts marked with `tr!`, `tr_n!` or `tr_noop!` in the program's sources.
    fn marked_texts() -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                // Comments (such as the examples above) and tests are not texts shown
                // by the program.
                let full = std::fs::read_to_string(&path).unwrap();
                let source: String = full
                    .split_once("#[cfg(test)]")
                    .map_or(full.as_str(), |(code, _)| code)
                    .lines()
                    .map(|l| {
                        if l.trim_start().starts_with("//") {
                            ""
                        } else {
                            l
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                for (mac, plural) in [("tr!(", false), ("tr_n!(", true), ("tr_noop!(", false)] {
                    for (at, _) in source.match_indices(mac) {
                        // Skips `str!(`-like names ending in the macro's name.
                        let before = source[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let rest = source[at + mac.len()..].trim_start();
                        let (key, _) = string_literal(rest).unwrap_or_else(|| {
                            panic!("{}: {mac} must start with a string literal", path.display())
                        });
                        let file = path.display().to_string();
                        if plural {
                            found.insert(format!("{key}_ONE"), file.clone());
                            found.insert(format!("{key}_OTHER"), file);
                        } else {
                            found.insert(key, file);
                        }
                    }
                }
            }
        }
        found
    }

    /// `{name}` placeholders of a text.
    fn placeholders(text: &str) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            rest = &rest[start + 1..];
            if let Some(end) = rest.find('}') {
                let name = &rest[..end];
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    out.insert(name);
                }
            }
        }
        out
    }

    fn load(json: &str, name: &str) -> BTreeMap<String, String> {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("{name} is not valid: {e}"))
    }

    /// Keys of `table` that the code does not use.
    fn unused<'a>(
        table: &'a BTreeMap<String, String>,
        keys: &BTreeMap<String, String>,
    ) -> Vec<&'a String> {
        table.keys().filter(|k| !keys.contains_key(*k)).collect()
    }

    #[test]
    fn keys_are_upper_snake_case() {
        for key in marked_texts().keys() {
            assert!(
                !key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "{key:?} is not a key such as ADD_TEXT_LAYER"
            );
        }
    }

    #[test]
    fn us_english_table_has_every_key() {
        let us = load(US_ENGLISH, "locales/en-US.json");
        let keys = marked_texts();
        assert!(keys.len() > 100, "found only {} keys", keys.len());
        let missing: Vec<_> = keys
            .iter()
            .filter(|(k, _)| us.get(*k).is_none_or(|v| v.is_empty()))
            .map(|(k, file)| format!("{k} ({file})"))
            .collect();
        assert!(
            missing.is_empty(),
            "missing in locales/en-US.json:\n{}",
            missing.join("\n")
        );
        let unused = unused(&us, &keys);
        assert!(
            unused.is_empty(),
            "keys no longer in the code, in locales/en-US.json: {unused:?}"
        );
    }

    #[test]
    fn french_table_is_complete() {
        let us = load(US_ENGLISH, "locales/en-US.json");
        let fr = load(FRENCH, "locales/fr-FR.json");
        let keys = marked_texts();
        let missing: Vec<_> = keys
            .iter()
            .filter(|(k, _)| fr.get(*k).is_none_or(|v| v.is_empty()))
            .map(|(k, file)| format!("{k} ({file})"))
            .collect();
        assert!(
            missing.is_empty(),
            "missing French translations:\n{}",
            missing.join("\n")
        );
        let unused = unused(&fr, &keys);
        assert!(
            unused.is_empty(),
            "keys no longer in the code, in locales/fr-FR.json: {unused:?}"
        );
        for (key, text) in &fr {
            if let Some(en) = us.get(key) {
                assert_eq!(
                    placeholders(en),
                    placeholders(text),
                    "placeholders of {key} in French"
                );
            }
        }
    }

    #[test]
    fn british_table_matches_the_code() {
        let us = load(US_ENGLISH, "locales/en-US.json");
        let gb = load(BRITISH_ENGLISH, "locales/en-GB.json");
        let keys = marked_texts();
        let unused = unused(&gb, &keys);
        assert!(
            unused.is_empty(),
            "keys no longer in the code, in locales/en-GB.json: {unused:?}"
        );
        for (key, text) in &gb {
            let Some(en) = us.get(key) else { continue };
            assert_ne!(en, text, "en-GB: {key} is spelled as in en-US");
            assert_eq!(
                placeholders(en),
                placeholders(text),
                "placeholders of {key} in en-GB"
            );
        }
        // Texts with a U.S. spelling need their British one.
        const US: [&str; 9] = [
            "color",
            "center",
            "gray",
            "canceled",
            "canceling",
            "neighbor",
            "meter ",
            "rize",
            "lize",
        ];
        let missing: Vec<_> = us
            .iter()
            .filter(|(k, v)| {
                let lower = v.to_lowercase();
                US.iter().any(|w| lower.contains(w)) && !gb.contains_key(*k)
            })
            .map(|(k, v)| format!("{k}: {v:?}"))
            .collect();
        assert!(
            missing.is_empty(),
            "missing British spellings in locales/en-GB.json:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn missing_keys_fall_back_to_us_english() {
        let gb = Language::BritishEnglish;
        assert_eq!(lookup(gb, "ADD_TEXT_LAYER"), Some("Add a text layer"));
        assert_eq!(lookup(gb, "PARAM_COLOR"), Some("Colour"));
        assert_eq!(lookup(Language::French, "PARAM_COLOR"), Some("Couleur"));
        // Not a key, such as a template's own element name: shown as it is.
        assert_eq!(lookup(gb, "Button centers"), None);
    }
}
