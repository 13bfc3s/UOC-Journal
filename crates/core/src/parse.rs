//! Raw line parsing.
//!
//! ClassicUO writes every journal entry as
//! `[{DateTime.Now:g}]  {name}: {text}` into `Data/Client/JournalLogs/
//! yyyy_MM_dd_HH_mm_ss_journal.txt`. The `g` format is culture dependent, so the
//! timestamp parser accepts `MM/DD/YYYY HH:MM`, `D.M.YYYY H:MM`, `YYYY-MM-DD HH:MM`,
//! 12-hour clocks with AM/PM, and so on. Day/month order is resolved per file using
//! the date embedded in the file name.

use crate::time;

/// A journal line split into its three parts. All slices borrow the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawLine<'a> {
    pub stamp: &'a str,
    pub name: &'a str,
    pub text: &'a str,
}

/// Split `[stamp]  name: text`. Returns `None` for lines without a leading
/// timestamp (continuations of multi-line messages).
pub fn split_line(line: &str) -> Option<RawLine<'_>> {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    let rest = line.strip_prefix('[')?;
    let close = memchr::memchr(b']', rest.as_bytes())?;
    let stamp = &rest[..close];
    // A real timestamp has digits; guards against "[Razor]: ..." style lines.
    if !stamp.bytes().any(|b| b.is_ascii_digit()) || stamp.len() > 40 {
        return None;
    }
    let after = &rest[close + 1..];
    let body = after
        .strip_prefix("  ")
        .or_else(|| after.strip_prefix(' '))
        .unwrap_or(after);
    let (name, text) = split_name(body);
    Some(RawLine { stamp, name, text })
}

/// Split `name: text` on the first `": "`.
pub fn split_name(body: &str) -> (&str, &str) {
    match memchr::memmem::find(body.as_bytes(), b": ") {
        Some(i) => (&body[..i], &body[i + 2..]),
        None => match body.strip_suffix(':') {
            Some(name) => (name, ""),
            None => ("", body),
        },
    }
}

/// Order of the first two numbers of a date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DateOrder {
    /// Not determined yet; defaults to month-first until evidence appears.
    #[default]
    Unknown,
    MonthFirst,
    DayFirst,
    YearFirst,
}

/// Date/time components found in a timestamp, before day/month resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawStamp {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub hour: u32,
    pub minute: u32,
    pub year_first: bool,
}

pub fn scan_stamp(stamp: &str) -> Option<RawStamp> {
    let mut nums = [0u32; 6];
    let mut digits = [0u8; 6];
    let mut n = 0usize;
    let mut cur: Option<(u32, u8)> = None;
    for b in stamp.bytes() {
        if b.is_ascii_digit() {
            let (v, d) = cur.unwrap_or((0, 0));
            cur = Some((
                v.saturating_mul(10).saturating_add((b - b'0') as u32),
                d + 1,
            ));
        } else if let Some((v, d)) = cur.take() {
            if n < 6 {
                nums[n] = v;
                digits[n] = d;
                n += 1;
            }
        }
    }
    if let Some((v, d)) = cur {
        if n < 6 {
            nums[n] = v;
            digits[n] = d;
            n += 1;
        }
    }
    if n < 5 {
        return None;
    }
    let has = |a: u8, b: u8| {
        stamp
            .as_bytes()
            .windows(2)
            .any(|w| w[0].eq_ignore_ascii_case(&a) && w[1].eq_ignore_ascii_case(&b))
            || stamp.as_bytes().windows(3).any(|w| {
                w[0].eq_ignore_ascii_case(&a) && w[1] == b'.' && w[2].eq_ignore_ascii_case(&b)
            })
    };
    let pm = has(b'P', b'M') || stamp.contains("午後");
    let am = has(b'A', b'M') || stamp.contains("午前");
    let mut hour = nums[3];
    if pm && hour < 12 {
        hour += 12;
    } else if am && hour == 12 {
        hour = 0;
    }
    let year_first = digits[0] >= 3;
    let mut c = nums[2];
    if !year_first && digits[2] <= 2 {
        c += 2000;
    }
    Some(RawStamp {
        a: nums[0],
        b: nums[1],
        c,
        hour,
        minute: nums[4],
        year_first,
    })
}

/// Resolves timestamps for one file, learning the date order as it goes.
#[derive(Clone, Debug, Default)]
pub struct StampParser {
    pub order: DateOrder,
    /// (year, month, day) from the file name, used to disambiguate.
    pub file_date: Option<(i64, u32, u32)>,
    /// Consecutive lines usually share the same minute.
    last: Option<(String, u32)>,
}

impl StampParser {
    pub fn new(file_date: Option<(i64, u32, u32)>) -> Self {
        StampParser {
            order: DateOrder::Unknown,
            file_date,
            last: None,
        }
    }

    pub fn parse(&mut self, stamp: &str) -> Option<u32> {
        if let Some((s, t)) = &self.last {
            if s == stamp {
                return Some(*t);
            }
        }
        let t = self.parse_uncached(stamp)?;
        match &mut self.last {
            Some((s, old)) => {
                s.clear();
                s.push_str(stamp);
                *old = t;
            }
            None => self.last = Some((stamp.to_string(), t)),
        }
        Some(t)
    }

    fn parse_uncached(&mut self, stamp: &str) -> Option<u32> {
        let r = scan_stamp(stamp)?;
        if r.year_first {
            self.order = DateOrder::YearFirst;
            return time::minutes(r.a as i64, r.b, r.c, r.hour, r.minute);
        }
        if self.order == DateOrder::Unknown || self.order == DateOrder::YearFirst {
            self.order = self.detect(&r);
        }
        let (mo, d) = match self.order {
            DateOrder::DayFirst => (r.b, r.a),
            _ => (r.a, r.b),
        };
        time::minutes(r.c as i64, mo, d, r.hour, r.minute)
            .or_else(|| time::minutes(r.c as i64, d, mo, r.hour, r.minute))
    }

    fn detect(&self, r: &RawStamp) -> DateOrder {
        if r.a > 12 {
            return DateOrder::DayFirst;
        }
        if r.b > 12 {
            return DateOrder::MonthFirst;
        }
        if let Some((_, fm, fd)) = self.file_date {
            if r.a == fm && r.b == fd {
                return DateOrder::MonthFirst;
            }
            if r.a == fd && r.b == fm {
                return DateOrder::DayFirst;
            }
        }
        DateOrder::Unknown
    }
}

/// Extract `yyyy_MM_dd_HH_mm_ss` from a journal file name.
pub fn file_name_stamp(name: &str) -> Option<(i64, u32, u32, u32, u32, u32)> {
    let b = name.as_bytes();
    // Pattern: 4d _ 2d _ 2d _ 2d _ 2d _ 2d
    const PAT: &[u8] = b"dddd_dd_dd_dd_dd_dd";
    if b.len() < PAT.len() {
        return None;
    }
    'outer: for start in 0..=b.len() - PAT.len() {
        for (i, p) in PAT.iter().enumerate() {
            let c = b[start + i];
            let ok = if *p == b'd' {
                c.is_ascii_digit()
            } else {
                c == b'_' || c == b'-'
            };
            if !ok {
                continue 'outer;
            }
        }
        let s = &name[start..start + PAT.len()];
        let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().ok();
        let y = num(0..4)? as i64;
        let mo = num(5..7)?;
        let d = num(8..10)?;
        let h = num(11..13)?;
        let mi = num(14..16)?;
        let se = num(17..19)?;
        if (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 {
            return Some((y, mo, d, h, mi, se));
        }
    }
    None
}

/// Minutes timestamp of a journal file's start, from its name.
pub fn file_start_minutes(name: &str) -> Option<u32> {
    let (y, mo, d, h, mi, _) = file_name_stamp(name)?;
    time::minutes(y, mo, d, h, mi)
}

/// Does this file name look like a journal log?
pub fn is_journal_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".txt") && (lower.contains("journal") || file_name_stamp(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    // All names, tags and chat below are invented.

    #[test]
    fn splits_lines() {
        let l = split_line("[03/14/2026 18:02]  System: WorldMap loading...").unwrap();
        assert_eq!(l.stamp, "03/14/2026 18:02");
        assert_eq!(l.name, "System");
        assert_eq!(l.text, "WorldMap loading...");

        let l =
            split_line("[03/14/2026 18:12]  [Alliance][Corwen Ash]: [VEX] crypt level 2, 3x pk ")
                .unwrap();
        assert_eq!(l.name, "[Alliance][Corwen Ash]");
        assert_eq!(l.text, "[VEX] crypt level 2, 3x pk ");

        let l = split_line("[03/14/2026 18:13]  : a pile of logs").unwrap();
        assert_eq!(l.name, "");
        assert_eq!(l.text, "a pile of logs");

        let l = split_line("[03/14/2026 18:13]  You see: (40/50 uses remaining)").unwrap();
        assert_eq!(l.name, "You see");

        let l = split_line("[03/14/2026 18:14]  [Razor]: Warning: Nightshade amount is now 2!")
            .unwrap();
        assert_eq!(l.name, "[Razor]");
        assert_eq!(l.text, "Warning: Nightshade amount is now 2!");

        let l = split_line("[03/14/2026 18:15]  [Guild][Bramblewick]: \\").unwrap();
        assert_eq!(l.text, "\\");

        assert!(split_line("continuation of a message").is_none());
        assert!(split_line("[Razor]: hi").is_none());
    }

    #[test]
    fn stamps() {
        let mut p = StampParser::new(Some((2026, 3, 14)));
        assert_eq!(
            time::ymd_hm(p.parse("03/14/2026 18:05").unwrap()),
            "2026-03-14 18:05"
        );
        assert_eq!(p.order, DateOrder::MonthFirst);

        let mut p = StampParser::new(Some((2026, 4, 2)));
        assert_eq!(
            time::ymd_hm(p.parse("02.04.2026 09:05").unwrap()),
            "2026-04-02 09:05"
        );
        assert_eq!(p.order, DateOrder::DayFirst);

        let mut p = StampParser::new(None);
        assert_eq!(
            time::ymd_hm(p.parse("3/14/2026 6:05 PM").unwrap()),
            "2026-03-14 18:05"
        );
        assert_eq!(
            time::ymd_hm(p.parse("3/14/2026 12:05 AM").unwrap()),
            "2026-03-14 00:05"
        );
        assert_eq!(
            time::ymd_hm(p.parse("2026-03-14 18:05").unwrap()),
            "2026-03-14 18:05"
        );
        assert_eq!(
            time::ymd_hm(p.parse("14/03/26 18:05").unwrap()),
            "2026-03-14 18:05"
        );
    }

    #[test]
    fn file_names() {
        assert_eq!(
            file_name_stamp("2026_03_14_18_02_11_journal.txt"),
            Some((2026, 3, 14, 18, 2, 11))
        );
        assert_eq!(
            file_name_stamp("copy-2026_04_02_09_15_30_journal.txt"),
            Some((2026, 4, 2, 9, 15, 30))
        );
        assert!(is_journal_file_name("2026_03_14_18_02_11_journal.txt"));
        assert!(!is_journal_file_name("settings.json"));
    }
}
