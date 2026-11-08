//! The system print dialog, and pages drawn on the printer with GDI.

use std::path::Path;

use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DEVMODEW, DIB_RGB_COLORS, DM_ORIENTATION, DM_PAPERLENGTH,
    DM_PAPERSIZE, DM_PAPERWIDTH, DMORIENT_LANDSCAPE, DMORIENT_PORTRAIT, DeleteDC, GetDeviceCaps,
    HALFTONE, HDC, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY,
    PHYSICALWIDTH, SRCCOPY, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC, PD_RETURNDEFAULT,
    PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW, PrintDlgW,
};

use super::{MAX_PRINT_DPI, Paper};
use crate::document::PageSize;
use crate::export::{self, ExportSettings, Progress};
use crate::i18n::tr;
use crate::render::{self, Scene};

/// A printer chosen in the print dialog, with its paper and copies set. Deleted
/// when dropped.
pub struct Printer(HDC);

// The device context is used by one thread at a time: the one printing.
unsafe impl Send for Printer {}

impl Drop for Printer {
    fn drop(&mut self) {
        unsafe { DeleteDC(self.0) };
    }
}

/// Windows' code for a standard paper size (DMPAPER_*).
fn paper_code(size: PageSize) -> Option<i16> {
    Some(match size {
        PageSize::Letter => 1,
        PageSize::Tabloid => 3,
        PageSize::Legal => 5,
        PageSize::A3 => 8,
        PageSize::A4 => 9,
        PageSize::A5 => 11,
        PageSize::A2 => 66,
        PageSize::Custom => return None,
    })
}

/// Frees what the dialog allocated.
fn free(handles: [HGLOBAL; 2]) {
    for h in handles.into_iter().filter(|h| !h.is_null()) {
        unsafe { GlobalFree(h) };
    }
}

/// Shows the system print dialog, its paper preset to `paper`, over the window
/// `owner`. `Ok(None)` when canceled.
pub fn choose(owner: Option<isize>, paper: &Paper) -> Result<Option<Printer>, String> {
    let mut pd: PRINTDLGW = unsafe { std::mem::zeroed() };
    pd.lStructSize = size_of::<PRINTDLGW>() as u32;
    pd.hwndOwner = owner.unwrap_or(0) as HWND;
    // The default printer's settings first, to set the paper in them.
    pd.Flags = PD_RETURNDEFAULT;
    if unsafe { PrintDlgW(&mut pd) } != 0 && !pd.hDevMode.is_null() {
        let dm = unsafe { GlobalLock(pd.hDevMode) } as *mut DEVMODEW;
        if !dm.is_null() {
            unsafe { set_paper(&mut *dm, paper) };
            unsafe { GlobalUnlock(pd.hDevMode) };
        }
    }
    pd.Flags = PD_RETURNDC | PD_USEDEVMODECOPIESANDCOLLATE | PD_NOSELECTION | PD_NOPAGENUMS;
    pd.nCopies = 1;
    let ok = unsafe { PrintDlgW(&mut pd) } != 0;
    free([pd.hDevMode, pd.hDevNames]);
    if !ok {
        return match unsafe { CommDlgExtendedError() } {
            0 => Ok(None),
            code => Err(tr!("PRINT_DIALOG_FAILED", code = format!("{code:#x}"))),
        };
    }
    if pd.hDC.is_null() {
        return Err(tr!("PRINT_DIALOG_FAILED", code = "no DC"));
    }
    Ok(Some(Printer(pd.hDC)))
}

/// Sets the paper size and orientation in a printer's settings.
fn set_paper(dm: &mut DEVMODEW, paper: &Paper) {
    let fields = unsafe { &mut dm.Anonymous1.Anonymous1 };
    match paper.preset.and_then(paper_code) {
        Some(code) => {
            fields.dmPaperSize = code;
            dm.dmFields |= DM_PAPERSIZE;
        }
        None => {
            // In tenths of a millimeter, portrait.
            let [w, h] = paper.portrait_mm().map(|v| (v * 10.0).round() as i16);
            fields.dmPaperSize = 256; // DMPAPER_USER
            fields.dmPaperWidth = w;
            fields.dmPaperLength = h;
            dm.dmFields |= DM_PAPERSIZE | DM_PAPERWIDTH | DM_PAPERLENGTH;
        }
    }
    fields.dmOrientation = match paper.orientation() {
        crate::document::Orientation::Landscape => DMORIENT_LANDSCAPE,
        crate::document::Orientation::Portrait => DMORIENT_PORTRAIT,
    } as i16;
    dm.dmFields |= DM_ORIENTATION;
}

/// Prints the page, or each sheet when split, at true size and centered on the
/// paper, as document `title`. With `output`, a printer that writes files (such as
/// Microsoft Print to PDF) writes there instead of asking.
pub fn print(
    printer: Printer,
    scene: &Scene,
    settings: &ExportSettings,
    title: &str,
    output: Option<&Path>,
    progress: &Progress,
) -> Result<(), String> {
    let dc = printer.0;
    let caps = |i: u32| unsafe { GetDeviceCaps(dc, i as i32) };
    let device_dpi = [caps(LOGPIXELSX), caps(LOGPIXELSY)].map(|v| v.max(1) as f32);
    let physical = [caps(PHYSICALWIDTH), caps(PHYSICALHEIGHT)].map(|v| v as f32);
    let offset = [caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY)].map(|v| v as f32);
    let dpi = device_dpi[0].max(device_dpi[1]).min(MAX_PRINT_DPI);

    progress.set(tr!("PRINT_STEP_RENDERING"), 0.0);
    let parts = render::compose_parts(scene, dpi, false, &mut |f| {
        progress.set(tr!("PRINT_STEP_RENDERING"), f * 0.5);
        !progress.is_canceled()
    })?;
    let sheets = settings
        .tiling()
        .map_or(1, |t| t.tiles(scene.page_mm).len());

    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let name = wide(title);
    let output = output.map(|p| wide(&p.to_string_lossy()));
    let info = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: name.as_ptr(),
        lpszOutput: output.as_ref().map_or(std::ptr::null(), |o| o.as_ptr()),
        lpszDatatype: std::ptr::null(),
        fwType: 0,
    };
    if unsafe { StartDocW(dc, &info) } <= 0 {
        return Err(tr!("PRINT_FAILED_TO_START").into());
    }
    let mut done = 0;
    let result = export::for_each_sheet(&parts, scene, settings, dpi, &mut |_, pixmap| {
        if progress.is_canceled() {
            return Err(render::CANCELED.into());
        }
        progress.set(
            tr!("PRINT_STEP_SENDING"),
            0.5 + 0.5 * done as f32 / sheets as f32,
        );
        done += 1;
        send_page(dc, pixmap, dpi, device_dpi, physical, offset)
    });
    match result {
        Ok(()) => {
            if unsafe { EndDoc(dc) } <= 0 {
                return Err(tr!("PRINT_FAILED_TO_FINISH").into());
            }
            progress.set(tr!("EXPORT_STEP_DONE"), 1.0);
            Ok(())
        }
        Err(e) => {
            unsafe { AbortDoc(dc) };
            Err(e)
        }
    }
}

/// Rows sent to the printer at a time: drivers cope badly with huge bitmaps.
const BAND_ROWS: u32 = 256;

/// One page: `pixmap`, rendered at `dpi`, centered on the paper at true size.
fn send_page(
    dc: HDC,
    pixmap: &tiny_skia::Pixmap,
    dpi: f32,
    device_dpi: [f32; 2],
    physical: [f32; 2],
    offset: [f32; 2],
) -> Result<(), String> {
    let (w, h) = (pixmap.width(), pixmap.height());
    // Device pixels per rendered pixel, and where the picture starts: centered on
    // the sheet of paper, whose printable area starts at `offset`.
    let scale = device_dpi.map(|d| d / dpi);
    let size = [w as f32 * scale[0], h as f32 * scale[1]];
    let x0 = ((physical[0] - size[0]) / 2.0 - offset[0]).round() as i32;
    let y0 = (physical[1] - size[1]) / 2.0 - offset[1];
    if unsafe { StartPage(dc) } <= 0 {
        return Err(tr!("PRINT_FAILED_TO_START").into());
    }
    unsafe {
        SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
    }
    // 24-bit BGR rows, each padded to 4 bytes.
    let stride = (w as usize * 3).div_ceil(4) * 4;
    let mut bits = vec![0u8; stride * BAND_ROWS as usize];
    let data = pixmap.data();
    let mut top = 0;
    while top < h {
        let rows = BAND_ROWS.min(h - top);
        for r in 0..rows as usize {
            let src = &data[((top as usize + r) * w as usize * 4)..][..w as usize * 4];
            let dst = &mut bits[r * stride..][..w as usize * 3];
            for (d, s) in dst
                .as_chunks_mut::<3>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                // The page is opaque: premultiplied is straight.
                d.copy_from_slice(&[s[2], s[1], s[0]]);
            }
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(rows as i32), // top-down
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        // Band edges rounded on the device, so bands meet without a gap.
        let y = (y0 + top as f32 * scale[1]).round() as i32;
        let y_end = (y0 + (top + rows) as f32 * scale[1]).round() as i32;
        let drawn = unsafe {
            StretchDIBits(
                dc,
                x0,
                y,
                size[0].round() as i32,
                y_end - y,
                0,
                0,
                w as i32,
                rows as i32,
                bits.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            )
        };
        if drawn == 0 {
            unsafe { EndPage(dc) };
            return Err(tr!("PRINT_FAILED_TO_SEND").into());
        }
        top += rows;
    }
    if unsafe { EndPage(dc) } <= 0 {
        return Err(tr!("PRINT_FAILED_TO_SEND").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Printer {
        /// A printer by name, without the dialog, with its default settings: for tests,
        /// with a printer that writes to a file.
        pub fn named(name: &str) -> Option<Self> {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let driver: Vec<u16> = "WINSPOOL".encode_utf16().chain([0]).collect();
            use windows_sys::Win32::Graphics::Gdi::CreateDCW;
            let dc = unsafe {
                CreateDCW(
                    driver.as_ptr(),
                    wide.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            (!dc.is_null()).then_some(Self(dc))
        }
    }

    /// Prints a page through Microsoft Print to PDF into a file, when that printer
    /// is installed.
    #[test]
    fn prints_to_a_file_through_gdi() {
        let Some(printer) = Printer::named("Microsoft Print to PDF") else {
            eprintln!("no Microsoft Print to PDF printer: skipped");
            return;
        };
        // A4 portrait, the printer's default paper here, with a black square.
        let mut doc = crate::document::Document {
            orientation: crate::document::Orientation::Portrait,
            ..Default::default()
        };
        let img = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255]));
        let mut l = crate::document::Layer::new(1, "square".into(), "x.png".into());
        l.size_px = [100, 100];
        l.transform.center = [105.0, 148.5];
        l.pixmap = Some(std::sync::Arc::new(
            render::premultiplied_pixmap(&img).unwrap(),
        ));
        doc.layers = vec![crate::document::Node::Layer(Box::new(l))];
        let scene = Scene::new(
            &doc,
            None,
            [[0, 0, 0]; crate::template::KINDS],
            [false; crate::template::KINDS],
            render::Cuts::default(),
        );
        let out = std::env::temp_dir().join(format!("artwork-print-{}.pdf", std::process::id()));
        std::fs::remove_file(&out).ok();
        print(
            printer,
            &scene,
            &ExportSettings::default(),
            "Artwork Creator test",
            Some(&out),
            &Progress::default(),
        )
        .unwrap();
        // The spooler writes the file once the job is through.
        for _ in 0..100 {
            if std::fs::metadata(&out).is_ok_and(|m| m.len() > 0) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let bytes = std::fs::read(&out).expect("the printer wrote the file");
        assert!(bytes.starts_with(b"%PDF"));
        if let Some(dir) = std::env::var_os("ARTWORK_PDF_SAMPLE") {
            std::fs::copy(&out, std::path::Path::new(&dir).join("printed.pdf")).unwrap();
        }
        std::fs::remove_file(&out).ok();
    }
}
