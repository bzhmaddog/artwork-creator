/*
 * Artwork Creator effect and generator plugins: the C ABI.
 *
 * A plugin is a shared library (.dll on Windows, .so on Linux) placed in the `plugins`
 * folder next to the app. It exports one function:
 *
 *     ARTWORK_EXPORT const ArtworkPlugin *artwork_plugin_entry(void);
 *
 * returning a table that stays valid while the library is loaded (the app never unloads
 * it), or NULL if the plugin cannot start. The app reads `abi_version` first and skips
 * the plugin unless it is from 1 to ARTWORK_PLUGIN_ABI_VERSION. Version 2 added
 * generators (the last two fields of ArtworkPlugin); a version 1 table ends before them.
 *
 * All strings are UTF-8 and NUL-terminated.
 *
 * Parameters (`params_json`) are a JSON array; each entry has "id", "label", "type" and,
 * depending on the type:
 *   "float":  "min", "max", "default", optional "unit" (e.g. "px") and "decimals" (1)
 *   "int":    "min", "max", "default", optional "unit"
 *   "bool":   "default"
 *   "color":  "default" as "#rrggbbaa" (straight RGBA)
 *   "choice": "options" as [{"id": ..., "label": ...}], "default" (an option id)
 * and, of any type, an optional "role" saying what it is for, which picks its control:
 *   "seed":   an int, a random variation the app picks when the effect is added and
 *             with a button giving it a new one; declare it from 0 to 2147483647
 *   "angle":  a float in degrees, counterclockwise from the right: its value and a dial
 *   "size":   a float length in the image's pixels: a slider with fine steps for small
 *             values, shown in mm on layers whose pixels have a size on the page
 * The app ignores a role that doesn't fit the type, or that it doesn't know, and shows
 * the type's usual control. Without a role it guesses one: an int with the id "seed"
 * is a seed, a float in "°" an angle, a float in "px" a size.
 *
 * Values (the `params_json` argument of padding/apply/generate) are a JSON object with every
 * parameter id, already checked by the app: numbers within range, colors as
 * "#rrggbbaa", choices one of their option ids.
 *
 * Plugins run inside the app with its full rights; a crash takes the app down.
 * `padding`, `apply` and `generate` may be called from several threads at once.
 *
 * Rust plugins should use the `artwork-plugin-sdk` crate instead of this header.
 */
#ifndef ARTWORK_PLUGIN_H
#define ARTWORK_PLUGIN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ARTWORK_PLUGIN_ABI_VERSION 2

#if defined(_WIN32)
#define ARTWORK_EXPORT __declspec(dllexport)
#else
#define ARTWORK_EXPORT __attribute__((visibility("default")))
#endif

/* Writes the room the effect draws into around the image (left, top, right, bottom),
 * in source pixels, to out[0..4]. Returns 0 on success. */
typedef int32_t (*ArtworkPaddingFn)(const void *user_data, const char *params_json,
                                    float out[4]);

/* Changes the image in place: `height` rows of `width` straight (not premultiplied)
 * RGBA pixels, 8 bits per channel, rows `stride` bytes apart, top to bottom. The image
 * already has the transparent margins asked for by `padding`. `scale` is the size of
 * the image relative to the layer's source: multiply distances in source pixels by it.
 * Returns 0 on success; otherwise may write a NUL-terminated message of at most
 * `error_len` bytes (NUL included) to `error`. */
typedef int32_t (*ArtworkApplyFn)(const void *user_data, uint8_t *pixels, uint32_t width,
                                  uint32_t height, uint32_t stride,
                                  const char *params_json, float scale, char *error,
                                  size_t error_len);

typedef struct ArtworkEffect {
    /* Unique, stable id saved in projects, e.g. "halftone.dots". Prefix it with
     * something unique to the plugin; "builtin." is reserved. */
    const char *id;
    const char *name;
    /* One line shown as a tooltip; may be empty. */
    const char *description;
    const char *params_json;
    /* Passed back to padding and apply. */
    const void *user_data;
    /* NULL if the effect never draws outside the image. */
    ArtworkPaddingFn padding;
    ArtworkApplyFn apply;
} ArtworkEffect;

/* Draws into the image, `height` rows of `width` straight RGBA pixels, 8 bits per
 * channel, rows `stride` bytes apart, top to bottom, all transparent at first. It covers
 * the shape's box at `px_per_mm` pixels per mm: multiply lengths in mm by it. The same
 * values must give the same picture at any resolution. Returns as ArtworkApplyFn. */
typedef int32_t (*ArtworkGenerateFn)(const void *user_data, uint8_t *pixels,
                                     uint32_t width, uint32_t height, uint32_t stride,
                                     const char *params_json, float px_per_mm,
                                     char *error, size_t error_len);

/* Draws a shape's fill, such as a pattern or a texture. */
typedef struct ArtworkGenerator {
    /* Unique, stable id saved in projects, as an effect's; no effect may have it. */
    const char *id;
    const char *name;
    /* One line shown as a tooltip; may be empty. */
    const char *description;
    const char *params_json;
    /* Passed back to generate. */
    const void *user_data;
    ArtworkGenerateFn generate;
} ArtworkGenerator;

typedef struct ArtworkPlugin {
    /* ARTWORK_PLUGIN_ABI_VERSION. Must stay the first field. */
    uint32_t abi_version;
    uint32_t effect_count;
    const char *name;
    const char *version;
    const ArtworkEffect *effects;
    /* Since version 2. */
    uint32_t generator_count;
    const ArtworkGenerator *generators;
} ArtworkPlugin;

typedef const ArtworkPlugin *(*ArtworkEntryFn)(void);

#ifdef __cplusplus
}
#endif

#endif /* ARTWORK_PLUGIN_H */
