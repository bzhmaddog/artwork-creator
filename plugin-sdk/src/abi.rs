//! The C ABI between Artwork Creator and a plugin library. `include/artwork_plugin.h`
//! describes the same layout for C and C++.
//!
//! A plugin exports one function, [`ENTRY_SYMBOL`], returning a [`PluginTable`] that
//! stays valid as long as the library is loaded. The host reads `abi_version` first and
//! rejects the plugin if it differs from [`ABI_VERSION`]: any change to these structs
//! must bump it.
//!
//! Strings are UTF-8 and NUL-terminated. Parameter lists and values are JSON (see
//! [`crate::Param`] and [`crate::Values`]).

use std::ffi::{c_char, c_void};

/// Version of the layout below. A plugin built for another version is not loaded.
pub const ABI_VERSION: u32 = 1;

/// Name of the function a plugin exports, of type [`EntryFn`].
pub const ENTRY_SYMBOL: &str = "artwork_plugin_entry";

/// `const ArtworkPlugin *artwork_plugin_entry(void)`: the plugin's table, or null if
/// it failed to start.
pub type EntryFn = unsafe extern "C" fn() -> *const PluginTable;

/// What a plugin provides.
#[repr(C)]
#[derive(Debug)]
pub struct PluginTable {
    /// Must stay the first field: the host reads it before anything else.
    pub abi_version: u32,
    pub effect_count: u32,
    /// Shown to the user, e.g. "Halftone".
    pub name: *const c_char,
    /// Free-form, e.g. "1.0.2".
    pub version: *const c_char,
    /// `effect_count` effects.
    pub effects: *const EffectTable,
}

/// `int32_t padding(const void *user_data, const char *params_json, float out[4])`.
///
/// Writes the room the effect draws into around the image (left, top, right, bottom),
/// in source pixels. Returns 0 on success.
pub type PaddingFn =
    unsafe extern "C" fn(user_data: *const c_void, params: *const c_char, out: *mut f32) -> i32;

/// ```c
/// int32_t apply(const void *user_data, uint8_t *pixels, uint32_t width,
///               uint32_t height, uint32_t stride, const char *params_json,
///               float scale, char *error, size_t error_len)
/// ```
///
/// Changes the image in place: `height` rows of `width` straight (not premultiplied)
/// RGBA pixels, 8 bits per channel, rows `stride` bytes apart, top to bottom. The image
/// already has the transparent margins asked for by `padding`. `scale` is the size of
/// the image relative to the layer's source: multiply distances in source pixels by it.
///
/// Returns 0 on success. Otherwise it may write a NUL-terminated UTF-8 message of at
/// most `error_len` bytes, NUL included, to `error`.
///
/// May be called from several threads at once, for different images.
pub type ApplyFn = unsafe extern "C" fn(
    user_data: *const c_void,
    pixels: *mut u8,
    width: u32,
    height: u32,
    stride: u32,
    params: *const c_char,
    scale: f32,
    error: *mut c_char,
    error_len: usize,
) -> i32;

/// One effect of a plugin.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EffectTable {
    /// Unique, stable id saved in projects, e.g. `"halftone.dots"`. Prefix it with
    /// something unique to the plugin; `builtin.` is reserved.
    pub id: *const c_char,
    pub name: *const c_char,
    /// One line shown as a tooltip; may be empty.
    pub description: *const c_char,
    /// JSON array of parameters, see [`crate::Param`].
    pub params_json: *const c_char,
    /// Passed back to `padding` and `apply`.
    pub user_data: *const c_void,
    /// Null if the effect never draws outside the image.
    pub padding: Option<PaddingFn>,
    pub apply: ApplyFn,
}

// The tables are read-only once built and point to data that lives as long as the
// library; plugins must make `padding` and `apply` thread-safe.
unsafe impl Send for PluginTable {}
unsafe impl Sync for PluginTable {}
unsafe impl Send for EffectTable {}
unsafe impl Sync for EffectTable {}
