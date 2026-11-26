# TODO

Temporary changes to undo, and why.

- **Remove the fallback to the recorded plugin version** (`shown_version` in
  `src/ui/plugins.rs`, and its test `shown_version_prefers_a_valid_one`). The Python
  plugins released before the plugin repository checked that the code's version matches
  `plugin.json` (film-grain 1.0.0 and 1.0.1, posterize 1.0.0 and 1.0.1, swap-channels
  1.0.0) give `Plugin(..., "1.0")`, so the Installed tab showed "1.0". Until they are
  released again, a loaded version that isn't X.Y.Z is replaced with the one recorded at
  install. Remove it once those plugins have new releases and the older versions no longer
  matter: then show the loaded version again, as before.
