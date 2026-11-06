//! Turns [`Effect`]s into the C tables of [`crate::abi`].

use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::abi::{ABI_VERSION, EffectTable, PluginTable};
use crate::{Effect, Values};

/// An image handed to [`Effect::apply`]: `height` rows of `width` straight RGBA pixels,
/// rows `stride` bytes apart.
pub struct Image<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
}

impl<'a> Image<'a> {
    /// Wraps a buffer; `None` if it is too small for the size given.
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32, stride: usize) -> Option<Self> {
        let row = width as usize * 4;
        (stride >= row && pixels.len() >= stride * height as usize).then_some(Self {
            pixels,
            width,
            height,
            stride,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes from one row to the next.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// The whole buffer, `stride` bytes per row.
    pub fn as_raw(&self) -> &[u8] {
        &self.pixels[..self.stride * self.height as usize]
    }

    /// The whole buffer, `stride` bytes per row.
    pub fn as_raw_mut(&mut self) -> &mut [u8] {
        &mut self.pixels[..self.stride * self.height as usize]
    }

    /// The pixels of row `y`, `width * 4` bytes.
    pub fn row(&self, y: u32) -> &[u8] {
        &self.pixels[y as usize * self.stride..][..self.width as usize * 4]
    }

    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        &mut self.pixels[y as usize * self.stride..][..self.width as usize * 4]
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = y as usize * self.stride + x as usize * 4;
        self.pixels[i..i + 4].try_into().unwrap()
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, px: [u8; 4]) {
        let i = y as usize * self.stride + x as usize * 4;
        self.pixels[i..i + 4].copy_from_slice(&px);
    }

    /// Runs `f(y, row)` on every row in parallel; `row` is `width * 4` bytes.
    pub fn par_rows(&mut self, f: impl Fn(u32, &mut [u8]) + Sync) {
        let (stride, row) = (self.stride, self.width as usize * 4);
        crate::par_rows(self.as_raw_mut(), stride, |y0, chunk| {
            for (j, r) in chunk.chunks_mut(stride).enumerate() {
                f((y0 + j) as u32, &mut r[..row]);
            }
        });
    }

    /// Runs `f` on every pixel in parallel.
    pub fn for_each_pixel(&mut self, f: impl Fn(&mut [u8; 4]) + Sync) {
        self.par_rows(|_, row| {
            for px in row.as_chunks_mut::<4>().0 {
                f(px);
            }
        });
    }
}

/// A string that lives as long as the library.
fn leak(s: &str) -> *const c_char {
    CString::new(s.replace('\0', ""))
        .expect("NULs removed")
        .into_raw()
}

/// Builds the plugin table returned by the entry point. It lives as long as the library.
/// `None` if building it panicked (e.g. in `Effect::params`).
///
/// Used by [`crate::export_plugin!`]; call it yourself only to test a plugin in-process.
pub fn build_table(
    name: &str,
    version: &str,
    effects: impl FnOnce() -> Vec<Box<dyn Effect>>,
) -> Option<&'static PluginTable> {
    catch_unwind(AssertUnwindSafe(|| {
        let effects: Vec<EffectTable> = effects()
            .into_iter()
            .map(|e| {
                let params = serde_json::to_string(&e.params()).expect("params serialize");
                let table = EffectTable {
                    id: leak(e.id()),
                    name: leak(e.name()),
                    description: leak(e.description()),
                    params_json: leak(&params),
                    user_data: std::ptr::null(),
                    padding: Some(padding),
                    apply,
                };
                let user_data = Box::into_raw(Box::new(e)) as *const c_void;
                EffectTable { user_data, ..table }
            })
            .collect();
        let effects = Box::leak(effects.into_boxed_slice());
        &*Box::leak(Box::new(PluginTable {
            abi_version: ABI_VERSION,
            effect_count: effects.len() as u32,
            name: leak(name),
            version: leak(version),
            effects: effects.as_ptr(),
        }))
    }))
    .ok()
}

/// The effect behind `user_data`.
///
/// # Safety
/// `user_data` comes from `build_table`.
unsafe fn effect<'a>(user_data: *const c_void) -> &'a dyn Effect {
    unsafe { &**(user_data as *const Box<dyn Effect>) }
}

/// # Safety
/// `params` is a NUL-terminated string.
unsafe fn values(params: *const c_char) -> Result<Values, String> {
    if params.is_null() {
        return Ok(Values::default());
    }
    let s = unsafe { CStr::from_ptr(params) }
        .to_str()
        .map_err(|_| "parameter values are not UTF-8".to_owned())?;
    Values::from_json(s)
}

fn panic_message(p: Box<dyn std::any::Any + Send>) -> String {
    let msg = p
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown error".into());
    format!("plugin panicked: {msg}")
}

unsafe extern "C" fn padding(
    user_data: *const c_void,
    params: *const c_char,
    out: *mut f32,
) -> i32 {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let values = unsafe { values(params) }?;
        Ok::<_, String>(unsafe { effect(user_data) }.padding(&values))
    }));
    match r {
        Ok(Ok(p)) if !out.is_null() => {
            unsafe { std::ptr::copy_nonoverlapping(p.as_ptr(), out, 4) };
            0
        }
        _ => 1,
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn apply(
    user_data: *const c_void,
    pixels: *mut u8,
    width: u32,
    height: u32,
    stride: u32,
    params: *const c_char,
    scale: f32,
    error: *mut c_char,
    error_len: usize,
) -> i32 {
    let r = catch_unwind(AssertUnwindSafe(|| {
        if pixels.is_null() {
            return Err("no image".to_owned());
        }
        let len = stride as usize * height as usize;
        let buf = unsafe { std::slice::from_raw_parts_mut(pixels, len) };
        let mut img = Image::new(buf, width, height, stride as usize)
            .ok_or_else(|| "invalid image size".to_owned())?;
        let values = unsafe { values(params) }?;
        unsafe { effect(user_data) }.apply(&mut img, &values, scale)
    }));
    let msg = match r {
        Ok(Ok(())) => return 0,
        Ok(Err(e)) => e,
        Err(p) => panic_message(p),
    };
    if !error.is_null() && error_len > 0 {
        // Cut at a character boundary, leaving room for the NUL.
        let mut n = msg.len().min(error_len - 1);
        while !msg.is_char_boundary(n) {
            n -= 1;
        }
        let msg = msg[..n].replace('\0', " ");
        unsafe {
            std::ptr::copy_nonoverlapping(msg.as_ptr(), error as *mut u8, n);
            *error.add(n) = 0;
        }
    }
    1
}
