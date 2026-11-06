//! Write image effect plugins for Artwork Creator in Rust.
//!
//! A plugin is a `cdylib` crate. Implement [`Effect`] for each effect and list them with
//! [`export_plugin!`]:
//!
//! ```ignore
//! use artwork_plugin_sdk::{Effect, Image, Param, Values, export_plugin};
//!
//! struct Fade;
//!
//! impl Effect for Fade {
//!     fn id(&self) -> &str { "myplugin.fade" }
//!     fn name(&self) -> &str { "Fade" }
//!     fn params(&self) -> Vec<Param> {
//!         vec![Param::float("amount", "Amount", 0.0, 100.0, 50.0).unit("%")]
//!     }
//!     fn apply(&self, img: &mut Image, v: &Values, _scale: f32) -> Result<(), String> {
//!         let keep = 1.0 - v.f32("amount") / 100.0;
//!         img.for_each_pixel(|px| px[3] = (px[3] as f32 * keep) as u8);
//!         Ok(())
//!     }
//! }
//!
//! export_plugin!(name: "My plugin", version: env!("CARGO_PKG_VERSION"), effects: [Fade]);
//! ```
//!
//! Build it with `cargo build --release` and copy the library (`.dll` on Windows, `.so`
//! on Linux) into the `plugins` folder next to the app.
//!
//! The SDK turns the trait into the C ABI described in [`abi`], catches panics so they
//! become errors instead of crashing the app, and checks the ABI version for you.

pub mod abi;
mod export;
mod params;

pub use export::{Image, build_table};
pub use params::{ChoiceOption, Param, ParamKind, Values, color_from_hex, color_to_hex};

/// An image effect.
pub trait Effect: Send + Sync + 'static {
    /// Unique, stable id saved in projects. Prefix it with something unique to the
    /// plugin, e.g. `"halftone.dots"`; `builtin.` is reserved.
    fn id(&self) -> &str;
    /// Shown in menus and in the effect list.
    fn name(&self) -> &str;
    /// One line shown as a tooltip.
    fn description(&self) -> &str {
        ""
    }
    /// Parameters, in display order. Called once, when the plugin loads.
    fn params(&self) -> Vec<Param>;
    /// Room the effect draws into around the image (left, top, right, bottom), in
    /// source pixels: e.g. a shadow falling outside it. The image passed to `apply` has
    /// transparent margins at least this wide.
    fn padding(&self, _values: &Values) -> [f32; 4] {
        [0.0; 4]
    }
    /// Changes `img` in place. Pixels are straight (not premultiplied) RGBA.
    ///
    /// `scale` is the size of `img` relative to the layer's source image: 1 at export,
    /// less on screen where a smaller copy is used. Multiply distances given in source
    /// pixels (like a radius) by it, so every size looks the same.
    ///
    /// Can be called from several threads at once, for different images.
    fn apply(&self, img: &mut Image, values: &Values, scale: f32) -> Result<(), String>;
}

/// Exports the plugin's entry point. Use it once, at the root of the plugin crate.
///
/// `name` and `version` are shown to the user; `effects` are values implementing
/// [`Effect`].
#[macro_export]
macro_rules! export_plugin {
    (name: $name:expr, version: $version:expr, effects: [$($effect:expr),* $(,)?] $(,)?) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn artwork_plugin_entry() -> *const $crate::abi::PluginTable {
            static TABLE: ::std::sync::OnceLock<
                ::std::option::Option<&'static $crate::abi::PluginTable>,
            > = ::std::sync::OnceLock::new();
            let table = TABLE.get_or_init(|| {
                $crate::build_table($name, $version, || {
                    ::std::vec![$(::std::boxed::Box::new($effect)
                        as ::std::boxed::Box<dyn $crate::Effect>),*]
                })
            });
            match table {
                ::std::option::Option::Some(t) => *t,
                ::std::option::Option::None => ::std::ptr::null(),
            }
        }
    };
}

/// Runs `f` on chunks of whole rows of `data` in parallel. `row_len` is the length of a
/// row in elements; `f` gets the index of the chunk's first row.
pub fn par_rows<T: Send>(data: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = data.len() / row_len.max(1);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per = rows.div_ceil(threads).max(16);
    if rows <= per {
        f(0, data);
        return;
    }
    std::thread::scope(|s| {
        for (i, chunk) in data.chunks_mut(per * row_len).enumerate() {
            let f = &f;
            s.spawn(move || f(i * per, chunk));
        }
    });
}
