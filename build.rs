//! Sets `BUILD_DATE` (YYYY-MM-DD, UTC) for Help > About: the day of the build, or that of
//! `SOURCE_DATE_EPOCH` when set, for reproducible builds. Cargo runs this again whenever
//! a file of the package changes, so the date follows the last build that changed it.
//!
//! For Windows, also builds the app's icon into the executable (see `windows_icon`).

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The icon Windows shows for the executable: in Explorer, the taskbar and shortcuts.
const ICON: &str = "assets/icon/artwork-creator.ico";

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
        });
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    println!("cargo:rustc-env=BUILD_DATE={y:04}-{m:02}-{d:02}");
}

/// Compiles a resource script naming the icon with `windres` (MinGW's: the Windows
/// builds use the GNU toolchain) and links the result into the executable. Without
/// `windres`, the build goes on with a warning and the executable has no icon. `WINDRES`
/// names another one.
fn windows_icon() {
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-env-changed=WINDRES");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join(ICON);
    // Forward slashes: the resource compiler reads backslashes as escapes.
    let icon = icon.to_string_lossy().replace('\\', "/");
    let rc = Path::new(&out).join("icon.rc");
    let obj = Path::new(&out).join("icon.o");
    std::fs::write(&rc, format!("1 ICON \"{icon}\"\n")).expect("writing icon.rc");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            "x86_64-w64-mingw32-windres".into()
        }
    });
    let built = Command::new(&windres)
        .arg(&rc)
        .args(["-O", "coff", "-o"])
        .arg(&obj)
        .status();
    match built {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", obj.display()),
        Ok(s) => println!("cargo:warning={windres} failed ({s}): the executable has no icon"),
        Err(e) => println!("cargo:warning={windres} not found ({e}): the executable has no icon"),
    }
}

/// Year, month and day of a count of days since 1970-01-01, in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}
