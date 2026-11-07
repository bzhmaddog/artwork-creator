//! Translations of the user interface.
//!
//! Texts are written in English in the code and looked up in the current language's
//! table when shown; the English text is the key. A text missing from a table shows in
//! English. Tables live in `locales/<code>.json` and are built into the program.
//!
//! - [`tr!`] translates a text, and fills in `{name}` placeholders:
//!   `tr!("Cannot open {path}: {e}", path = p.display(), e = e)`.
//! - [`tr_n!`] picks the singular or plural text for a count, given as `{n}`:
//!   `tr_n!("{n} shape", "{n} shapes", count)`.
//! - [`tr_noop!`] marks a text for translation where it is defined (e.g. a constant),
//!   for [`translate`] to look it up where it is shown.
//!
//! A test checks that every text marked in the code has a French translation, with the
//! same placeholders, and that the table has no leftovers.

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
    English = 0,
    French = 1,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::French];

    /// The language's own name, as shown in the Language menu.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::French => "Français",
        }
    }

    /// The supported language of a locale such as `fr-FR` or `fr_CA.UTF-8`; English for
    /// any other.
    pub fn from_locale(locale: &str) -> Language {
        let code = locale
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match code.as_str() {
            "fr" => Language::French,
            _ => Language::English,
        }
    }

    /// The operating system's language, if supported, otherwise English.
    pub fn system() -> Language {
        sys_locale::get_locale()
            .map(|l| Language::from_locale(&l))
            .unwrap_or(Language::English)
    }

    /// Translations from English; `None` for English itself.
    fn table(self) -> Option<&'static HashMap<&'static str, &'static str>> {
        static FR: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
        match self {
            Language::English => None,
            Language::French => Some(FR.get_or_init(|| parse_table(FRENCH))),
        }
    }

    /// Whether `n` takes the singular: only 1 in English, 0 and 1 in French.
    fn is_singular(self, n: u64) -> bool {
        match self {
            Language::English => n == 1,
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
}

impl Preference {
    pub fn language(self) -> Language {
        match self {
            Preference::System => Language::system(),
            Preference::English => Language::English,
            Preference::French => Language::French,
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
        }
    }
}

const FRENCH: &str = include_str!("../locales/fr.json");

/// Reads a `locales/*.json` table: an object of English texts to translations.
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
        _ => Language::English,
    }
}

/// Shows texts in `lang` from now on.
pub fn set(lang: Language) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// `text` in the current language, or `text` itself if it has no translation.
pub fn translate(text: &str) -> &str {
    current()
        .table()
        .and_then(|t| t.get(text).copied())
        .unwrap_or(text)
}

/// The English singular or plural text for `n` in the current language, translated.
pub fn translate_n(one: &'static str, other: &'static str, n: u64) -> &'static str {
    let text = if current().is_singular(n) { one } else { other };
    current()
        .table()
        .and_then(|t| t.get(text).copied())
        .unwrap_or(text)
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

/// Translates a text; with `name = value` arguments, fills in its placeholders and
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

/// The singular or plural text for a count, translated, with `{n}` and any other
/// `name = value` placeholders filled in.
macro_rules! tr_n {
    ($one:literal, $other:literal, $n:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let n = $n;
        $crate::i18n::fill(
            $crate::i18n::translate_n($one, $other, n as u64),
            &[("n", &n as &dyn ::std::fmt::Display)
                $(, (stringify!($name), &$value as &dyn ::std::fmt::Display))*],
        )
    }};
}

/// Marks a text for translation without translating it; see [`translate`].
macro_rules! tr_noop {
    ($text:literal) => {
        $text
    };
}

pub(crate) use {tr, tr_n, tr_noop};

/// View > Language entries: System, then each language in its own name.
pub fn menu(ui: &mut egui::Ui, pref: &mut Preference) {
    let before = *pref;
    ui.radio_value(pref, Preference::System, tr!("System"))
        .on_hover_text(tr!("Use the operating system's language"));
    for lang in Language::ALL {
        ui.radio_value(pref, lang.into(), lang.native_name());
    }
    if *pref != before {
        set(pref.language());
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    #[test]
    fn locales_map_to_languages() {
        assert_eq!(Language::from_locale("fr-FR"), Language::French);
        assert_eq!(Language::from_locale("fr_CA.UTF-8"), Language::French);
        assert_eq!(Language::from_locale("FR"), Language::French);
        assert_eq!(Language::from_locale("en-US"), Language::English);
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
                for (mac, count) in [("tr!(", 1), ("tr_n!(", 2), ("tr_noop!(", 1)] {
                    for (at, _) in source.match_indices(mac) {
                        // Skips `str!(`-like names ending in the macro's name.
                        let before = source[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let mut rest = source[at + mac.len()..].trim_start();
                        for _ in 0..count {
                            let (text, after) = string_literal(rest).unwrap_or_else(|| {
                                panic!("{}: {mac} must start with string literals", path.display())
                            });
                            found.insert(text, path.display().to_string());
                            rest = after.trim_start().trim_start_matches(',').trim_start();
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

    #[test]
    fn french_table_is_complete() {
        let table: BTreeMap<String, String> =
            serde_json::from_str(FRENCH).expect("locales/fr.json is valid JSON");
        let marked = marked_texts();
        assert!(marked.len() > 100, "found only {} texts", marked.len());
        let missing: Vec<_> = marked
            .iter()
            .filter(|(k, _)| table.get(*k).is_none_or(|v| v.is_empty()))
            .map(|(k, file)| format!("{k:?} ({file})"))
            .collect();
        assert!(
            missing.is_empty(),
            "missing French translations:\n{}",
            missing.join("\n")
        );
        let unused: Vec<_> = table.keys().filter(|k| !marked.contains_key(*k)).collect();
        assert!(
            unused.is_empty(),
            "translations of texts no longer in the code: {unused:?}"
        );
        for (en, fr) in &table {
            assert_eq!(
                placeholders(en),
                placeholders(fr),
                "placeholders differ in the translation of {en:?}"
            );
        }
    }
}
