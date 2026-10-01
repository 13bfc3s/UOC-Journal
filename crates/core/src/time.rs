//! Minute-resolution local timestamps. Journal lines only carry minutes, and the
//! client writes local time, so we keep naive "minutes since 1970-01-01".

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`]: returns (year, month, day).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Build a minute timestamp. Returns `None` for out-of-range components.
pub fn minutes(y: i64, mo: u32, d: u32, h: u32, mi: u32) -> Option<u32> {
    if !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || !(1970..=9999).contains(&y)
    {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    u32::try_from(days * 1440 + h as i64 * 60 + mi as i64).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

pub fn parts(t: u32) -> Parts {
    let days = (t / 1440) as i64;
    let rem = t % 1440;
    let (year, month, day) = civil_from_days(days);
    Parts {
        year,
        month,
        day,
        hour: rem / 60,
        minute: rem % 60,
    }
}

/// `HH:MM`
pub fn hm(t: u32) -> String {
    let p = parts(t);
    format!("{:02}:{:02}", p.hour, p.minute)
}

/// `YYYY-MM-DD HH:MM`
pub fn ymd_hm(t: u32) -> String {
    let p = parts(t);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        p.year, p.month, p.day, p.hour, p.minute
    )
}

/// `MM-DD HH:MM`
pub fn md_hm(t: u32) -> String {
    let p = parts(t);
    format!("{:02}-{:02} {:02}:{:02}", p.month, p.day, p.hour, p.minute)
}

/// Current local time is not knowable without a tz database; callers that need
/// "now" use the newest journal timestamp instead. This returns UTC minutes and is
/// only used for coarse file-age decisions.
pub fn now_utc_minutes() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (secs / 60) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for days in [-1000i64, 0, 1, 365, 10957, 20361, 20727, 40000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        let t = minutes(2026, 9, 30, 19, 37).unwrap();
        assert_eq!(ymd_hm(t), "2026-09-30 19:37");
        assert_eq!(hm(t), "19:37");
        assert_eq!(md_hm(t), "09-30 19:37");
        assert!(minutes(2026, 13, 1, 0, 0).is_none());
    }
}
