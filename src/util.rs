use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
}

/// Second-precision RFC3339 formatter. Civil-from-days per Howard Hinnant.
pub fn format_rfc3339(mut secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    secs %= 86_400;
    let hour = (secs / 3600) as u32;
    let minute = ((secs / 60) % 60) as u32;
    let second = (secs % 60) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::format_rfc3339;

    #[test]
    fn epoch_formats() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_date() {
        // 2026-04-20T08:37:19Z → 1776674239
        assert_eq!(format_rfc3339(1_776_674_239), "2026-04-20T08:37:19Z");
    }

    #[test]
    fn leap_day() {
        // 2024-02-29T12:34:56Z → 1709210096
        assert_eq!(format_rfc3339(1_709_210_096), "2024-02-29T12:34:56Z");
    }
}
