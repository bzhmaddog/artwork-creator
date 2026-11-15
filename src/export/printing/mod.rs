//! Printing on a printer, at true size, with the export settings (tiling, marks,
//! mirroring...): through the system print dialog and GDI on Windows, through CUPS
//! (`lp`) elsewhere.

use crate::document::{Document, Orientation, P2, PageSize};
use crate::export::ExportSettings;

/// Largest resolution the pages are rendered at for a printer, in DPI: finer adds
/// memory and time, not visible detail.
#[cfg(windows)]
pub const MAX_PRINT_DPI: f32 = 600.0;

/// The paper a print needs: the page, or a sheet when the page is split.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    /// Width and height in mm, as printed (landscape when wider than tall).
    pub size_mm: P2,
    /// The standard size it is, if any (`None` for a custom page).
    pub preset: Option<PageSize>,
}

impl Paper {
    /// The paper for the project's page, or for the sheets when the export splits it.
    pub fn of(doc: &Document, settings: &ExportSettings) -> Self {
        match settings.tiling() {
            Some(t) => Self {
                size_mm: t.sheet_mm,
                preset: Some(settings.sheet),
            },
            None => Self {
                size_mm: doc.page_mm(),
                preset: doc.page.portrait_mm().map(|_| doc.page),
            },
        }
    }

    pub fn orientation(&self) -> Orientation {
        if self.size_mm[0] > self.size_mm[1] {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        }
    }

    /// Width and height in portrait orientation.
    pub fn portrait_mm(&self) -> P2 {
        let [w, h] = self.size_mm;
        [w.min(h), w.max(h)]
    }
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::{choose, print};

#[cfg(not(windows))]
mod cups;
#[cfg(not(windows))]
pub use self::cups::{Printers, print, printers};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_follows_the_page_or_the_sheets() {
        let mut doc = Document::default();
        let mut settings = ExportSettings::default();
        assert_eq!(
            Paper::of(&doc, &settings),
            Paper {
                size_mm: [297.0, 210.0],
                preset: Some(PageSize::A4)
            }
        );
        assert_eq!(
            Paper::of(&doc, &settings).orientation(),
            Orientation::Landscape
        );
        doc.page = PageSize::Custom;
        doc.custom_page_mm = [300.0, 500.0];
        assert_eq!(Paper::of(&doc, &settings).preset, None);
        assert_eq!(Paper::of(&doc, &settings).portrait_mm(), [300.0, 500.0]);
        // Split: the sheets' paper.
        settings.tile = true;
        settings.sheet = PageSize::Letter;
        let p = Paper::of(&doc, &settings);
        assert_eq!(p.preset, Some(PageSize::Letter));
        assert_eq!(p.orientation(), Orientation::Portrait);
    }
}
