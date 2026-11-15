//! Printing through CUPS: the page written as a PDF, handed to `lp`.

use std::process::Command;

use super::Paper;
use crate::document::PageSize;
use crate::export::{self, ExportSettings, Progress};
use crate::i18n::tr;
use crate::render::Scene;

/// The printers CUPS knows, and its default one.
#[derive(Clone, Debug, Default)]
pub struct Printers {
    pub names: Vec<String>,
    pub default: Option<String>,
}

/// Asks CUPS for its printers; none when it isn't installed or running.
pub fn printers() -> Printers {
    let run = |args: &[&str]| {
        Command::new("lpstat")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let names = run(&["-e"])
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    // "system default destination: name"
    let default = run(&["-d"])
        .split_once(':')
        .map(|(_, name)| name.trim().to_owned())
        .filter(|n| !n.is_empty());
    Printers { names, default }
}

/// The paper's name for CUPS.
fn media(paper: &Paper) -> String {
    match paper.preset {
        Some(PageSize::Custom) | None => {
            let [w, h] = paper.portrait_mm();
            format!("Custom.{w:.1}x{h:.1}mm")
        }
        Some(p) => format!("{p:?}"),
    }
}

/// Arguments for `lp`: printer, copies, paper, and no scaling, so it prints at true
/// size. The PDF's pages are already the paper's size and orientation.
fn lp_args(printer: &str, copies: u32, paper: &Paper, title: &str, file: &str) -> Vec<String> {
    [
        "-d",
        printer,
        "-n",
        &copies.max(1).to_string(),
        "-t",
        title,
        "-o",
        &format!("media={}", media(paper)),
        "-o",
        "print-scaling=none",
        "--",
        file,
    ]
    .map(str::to_owned)
    .into()
}

/// Writes the print as a PDF, the same as the export's, and hands it to `lp`.
pub fn print(
    printer: &str,
    copies: u32,
    paper: &Paper,
    scene: &Scene,
    settings: &ExportSettings,
    title: &str,
    progress: &Progress,
) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("artwork-print-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let settings = ExportSettings {
        png: false,
        pdf: true,
        ..settings.clone()
    };
    let result = export::run(scene, &settings, &dir.join("print"), progress).and_then(|files| {
        let file = files[0].to_string_lossy().into_owned();
        let out = Command::new("lp")
            .args(lp_args(printer, copies, paper, title, &file))
            .output()
            .map_err(|e| tr!("PRINT_NO_CUPS", e = e))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(tr!(
                "PRINT_REFUSED",
                e = String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    });
    std::fs::remove_dir_all(&dir).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lp_prints_at_true_size_on_the_right_paper() {
        let a4 = Paper {
            size_mm: [297.0, 210.0],
            preset: Some(PageSize::A4),
        };
        let args = lp_args("office", 2, &a4, "stick", "/tmp/print.pdf");
        assert_eq!(
            args,
            [
                "-d",
                "office",
                "-n",
                "2",
                "-t",
                "stick",
                "-o",
                "media=A4",
                "-o",
                "print-scaling=none",
                "--",
                "/tmp/print.pdf"
            ]
        );
        let custom = Paper {
            size_mm: [400.0, 250.0],
            preset: None,
        };
        assert!(
            lp_args("p", 0, &custom, "t", "f").contains(&"media=Custom.250.0x400.0mm".to_owned())
        );
        assert!(lp_args("p", 0, &custom, "t", "f").contains(&"1".to_owned()));
    }
}
