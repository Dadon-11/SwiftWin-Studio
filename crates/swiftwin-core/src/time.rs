//! Tiny UTC timestamp helpers (avoids pulling in a date crate).

use std::time::{SystemTime, UNIX_EPOCH};

/// Current time as RFC 3339 UTC with milliseconds, e.g. `2026-11-03T12:00:00.123Z`.
pub fn now_rfc3339() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format_unix_millis(d.as_millis() as i64)
}

pub fn unix_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn format_unix_millis(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_known_dates() {
        assert_eq!(format_unix_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_unix_millis(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(format_unix_millis(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }
}
