//! Sets `BUILD_DATE` (YYYY-MM-DD, UTC) for Help > About: the day of the build, or that of
//! `SOURCE_DATE_EPOCH` when set, for reproducible builds. Cargo runs this again whenever
//! a file of the package changes, so the date follows the last build that changed it.

use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
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
