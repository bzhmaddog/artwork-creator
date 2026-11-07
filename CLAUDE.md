# Project notes

- **Consistency**: a new tool or feature gets the same treatment as its siblings
  (keyboard shortcut shown in its menu entry or tooltip and listed in README's Shortcuts
  table, a translation key with its text in `locales/en-US.json` and `locales/fr-FR.json`,
  etc.). Before finishing, compare it with
  the closest existing feature and fill in what it lacks. See "Consistency" in README.md.
- **U.S. English** everywhere in code, comments, docs and UI texts (color, center, gray,
  canceled). British spellings go only in `locales/en-GB.json`, which holds just the
  texts spelled differently; add an entry there when a new text has one.
- **Translation keys** say what the text is for, in upper snake case (`ADD_TEXT_LAYER`,
  `EXPORT_CANCELED`), not a copy of its words. Plurals are `KEY_ONE` / `KEY_OTHER`.
