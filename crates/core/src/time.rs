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

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// strftime-style formatting of a minute timestamp plus optional seconds.
///
/// `%Y` year, `%y` 2-digit year, `%m` month, `%d` day, `%e` day without zero,
/// `%H` 24h hour, `%I` 12h hour, `%l` 12h hour without zero, `%M` minute,
/// `%S` second (`--` when unknown), `%p` AM/PM, `%P` am/pm, `%b`/`%B` month
/// name, `%a`/`%A` weekday name, `%j` day of year, `%F` = `%Y-%m-%d`,
/// `%T` = `%H:%M:%S`, `%R` = `%H:%M`, `%%` a percent sign.
pub fn format(t: u32, secs: Option<u8>, fmt: &str) -> String {
    let p = parts(t);
    let days = (t / 1440) as i64;
    let weekday = ((days + 4).rem_euclid(7)) as usize; // 1970-01-01 was a Thursday
    let h12 = match p.hour % 12 {
        0 => 12,
        h => h,
    };
    let sec = |out: &mut String| match secs {
        Some(s) => out.push_str(&format!("{s:02}")),
        None => out.push_str("--"),
    };
    let mut out = String::with_capacity(fmt.len() + 8);
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{:04}", p.year)),
            Some('y') => out.push_str(&format!("{:02}", p.year.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{:02}", p.month)),
            Some('d') => out.push_str(&format!("{:02}", p.day)),
            Some('e') => out.push_str(&p.day.to_string()),
            Some('H') => out.push_str(&format!("{:02}", p.hour)),
            Some('k') => out.push_str(&p.hour.to_string()),
            Some('I') => out.push_str(&format!("{h12:02}")),
            Some('l') => out.push_str(&h12.to_string()),
            Some('M') => out.push_str(&format!("{:02}", p.minute)),
            Some('S') => sec(&mut out),
            Some('p') => out.push_str(if p.hour < 12 { "AM" } else { "PM" }),
            Some('P') => out.push_str(if p.hour < 12 { "am" } else { "pm" }),
            Some('b') => out.push_str(&MONTHS[(p.month - 1) as usize][..3]),
            Some('B') => out.push_str(MONTHS[(p.month - 1) as usize]),
            Some('a') => out.push_str(&WEEKDAYS[weekday][..3]),
            Some('A') => out.push_str(WEEKDAYS[weekday]),
            Some('j') => {
                let doy = days - days_from_civil(p.year, 1, 1) + 1;
                out.push_str(&format!("{doy:03}"));
            }
            Some('F') => out.push_str(&format!("{:04}-{:02}-{:02}", p.year, p.month, p.day)),
            Some('R') => out.push_str(&format!("{:02}:{:02}", p.hour, p.minute)),
            Some('T') => {
                out.push_str(&format!("{:02}:{:02}:", p.hour, p.minute));
                sec(&mut out);
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
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
        assert_eq!(
            format(t, Some(7), "%Y-%m-%d %H:%M:%S"),
            "2026-09-30 19:37:07"
        );
        assert_eq!(format(t, None, "%T"), "19:37:--");
        assert_eq!(format(t, None, "%a %e %b, %l:%M %p"), "Wed 30 Sep, 7:37 PM");
        assert_eq!(
            format(t, None, "%A %B %j %y %% %q"),
            "Wednesday September 273 26 % %q"
        );
        let midnight = minutes(2026, 1, 1, 0, 5).unwrap();
        assert_eq!(format(midnight, None, "%I:%M %P"), "12:05 am");
    }
}
