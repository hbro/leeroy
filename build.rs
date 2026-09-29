//! Build time for the help popup: `LEEROY_BUILT` (`2026-09-29 13:40 UTC`).
//!
//! `SOURCE_DATE_EPOCH` wins for reproducible builds, except for Nix's
//! placeholder (1980-01-01, also set in the devShell), which says nothing.

use std::time::{SystemTime, UNIX_EPOCH};

/// 1980-01-01T00:00:00Z: what Nix sets when there's no real date.
const NIX_PLACEHOLDER: u64 = 315_532_800;

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&secs| secs > NIX_PLACEHOLDER)
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        });
    println!("cargo:rustc-env=LEEROY_BUILT={}", format_utc(epoch));
}

/// `YYYY-MM-DD HH:MM UTC` (civil-from-days, Howard Hinnant's algorithm).
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}
