//! HTTP header value helpers: entity tags, dates, `Content-Length`, value bounds.

use udm_core::limits;

use crate::civil::{civil_from_days, month_from_abbr, unix_from_civil};
use crate::text::parse_u64_strict;

/// An HTTP entity tag (`ETag`), without the surrounding quotes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntityTag {
    /// Opaque tag value (between the quotes).
    pub tag: String,
    /// `W/` prefix present. Weak tags are never trusted for resume.
    pub weak: bool,
}

impl EntityTag {
    /// Parses `"abc"` or `W/"abc"`. `None` for malformed or oversized values.
    pub fn parse(v: &str) -> Option<Self> {
        if !header_value_ok(v) {
            return None;
        }
        let v = v.trim();
        let (weak, rest) = match v.strip_prefix("W/") {
            Some(r) => (true, r),
            None => (false, v),
        };
        let inner = rest.strip_prefix('"')?.strip_suffix('"')?;
        // etagc = %x21 / %x23-7E / obs-text
        if inner
            .chars()
            .any(|c| c == '"' || (c.is_ascii() && !('\x21'..='\x7e').contains(&c)))
        {
            return None;
        }
        Some(Self {
            tag: inner.to_owned(),
            weak,
        })
    }

    /// Strong comparison (RFC 9110 §8.8.3.2): both strong and identical.
    pub fn strong_eq(&self, o: &Self) -> bool {
        !self.weak && !o.weak && self.tag == o.tag
    }

    /// Header form, e.g. for `If-Range`.
    pub fn to_header(&self) -> String {
        if self.weak {
            format!("W/\"{}\"", self.tag)
        } else {
            format!("\"{}\"", self.tag)
        }
    }
}

/// True when a header value is within `limits::MAX_HEADER_VALUE_BYTES` and contains no CR, LF or NUL.
pub fn header_value_ok(v: &str) -> bool {
    v.len() <= limits::MAX_HEADER_VALUE_BYTES && !v.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0))
}

/// Strict `Content-Length`: digits only (surrounding whitespace allowed). Lists, signs
/// and overflow are rejected.
pub fn parse_content_length(v: &str) -> Option<u64> {
    if !header_value_ok(v) {
        return None;
    }
    parse_u64_strict(v.trim())
}

/// Parses an HTTP date (IMF-fixdate, RFC 850 or asctime) into Unix seconds (UTC).
pub fn parse_http_date(v: &str) -> Option<i64> {
    if !header_value_ok(v) {
        return None;
    }
    let v = v.trim();
    match v.split_once(',') {
        Some((_weekday, rest)) => {
            let mut it = rest.split_whitespace();
            let first = it.next()?;
            if first.contains('-') {
                // RFC 850: "06-Nov-94 08:49:37 GMT"
                let mut dp = first.split('-');
                let day = dp.next()?;
                let mon = dp.next()?;
                let year = dp.next()?;
                if dp.next().is_some() {
                    return None;
                }
                let time = it.next()?;
                expect_gmt_end(&mut it)?;
                let year = parse_num(year)?;
                let year = if year.len() == 2 {
                    let yy = i64::from(year.value);
                    if yy < 70 {
                        yy.checked_add(2000)?
                    } else {
                        yy.checked_add(1900)?
                    }
                } else {
                    i64::from(year.value)
                };
                build(year, mon, day, time)
            } else {
                // IMF-fixdate: "06 Nov 1994 08:49:37 GMT"
                let mon = it.next()?;
                let year = it.next()?;
                let time = it.next()?;
                expect_gmt_end(&mut it)?;
                build(i64::from(parse_num(year)?.value), mon, first, time)
            }
        }
        None => {
            // asctime: "Sun Nov  6 08:49:37 1994"
            let mut it = v.split_whitespace();
            let _weekday = it.next()?;
            let mon = it.next()?;
            let day = it.next()?;
            let time = it.next()?;
            let year = it.next()?;
            if it.next().is_some() {
                return None;
            }
            build(i64::from(parse_num(year)?.value), mon, day, time)
        }
    }
}

struct Num {
    value: u32,
    digits: usize,
}

impl Num {
    fn len(&self) -> usize {
        self.digits
    }
}

fn parse_num(s: &str) -> Option<Num> {
    if s.len() > 9 {
        return None;
    }
    let v = parse_u64_strict(s)?;
    Some(Num {
        value: u32::try_from(v).ok()?,
        digits: s.len(),
    })
}

fn expect_gmt_end<'a>(it: &mut impl Iterator<Item = &'a str>) -> Option<()> {
    if it.next()? != "GMT" || it.next().is_some() {
        return None;
    }
    Some(())
}

fn build(year: i64, mon: &str, day: &str, time: &str) -> Option<i64> {
    let month = month_from_abbr(mon)?;
    let day = parse_num(day)?.value;
    let mut tp = time.split(':');
    let h = parse_num(tp.next()?)?.value;
    let m = parse_num(tp.next()?)?.value;
    let s = parse_num(tp.next()?)?.value;
    if tp.next().is_some() {
        return None;
    }
    unix_from_civil(year, month, day, h, m, s)
}

/// Formats Unix seconds as an IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`).
/// Values outside years 0..=9999 are clamped to that range.
pub fn format_http_date(unix_secs: i64) -> String {
    // 0000-01-01T00:00:00Z ..= 9999-12-31T23:59:59Z
    const MIN: i64 = -62_167_219_200;
    const MAX: i64 = 253_402_300_799;
    let t = unix_secs.clamp(MIN, MAX);
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let wd = usize::try_from(days.rem_euclid(7))
        .ok()
        .and_then(|i| WEEKDAYS.get(i))
        .unwrap_or(&"Thu");
    let (y, m, d) = civil_from_days(days).unwrap_or((1970, 1, 1));
    let mon = usize::try_from(m)
        .ok()
        .and_then(|m| m.checked_sub(1))
        .and_then(|i| MONTHS.get(i))
        .unwrap_or(&"Jan");
    let hh = secs.div_euclid(3_600);
    let mm = secs.rem_euclid(3_600).div_euclid(60);
    let ss = secs.rem_euclid(60);
    format!("{wd}, {d:02} {mon} {y:04} {hh:02}:{mm:02}:{ss:02} GMT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_parse_and_compare() {
        let a = EntityTag::parse("\"abc\"").unwrap();
        let w = EntityTag::parse("W/\"abc\"").unwrap();
        assert!(!a.weak && w.weak);
        assert!(a.strong_eq(&a.clone()));
        assert!(!a.strong_eq(&w));
        assert!(!w.strong_eq(&w.clone()));
        assert_eq!(w.to_header(), "W/\"abc\"");
        assert_eq!(EntityTag::parse("\"\"").unwrap().tag, "");
    }

    #[test]
    fn hostile_etag_malformed() {
        for v in [
            "abc",
            "\"abc",
            "abc\"",
            "\"a\"b\"",
            "\"a b\"",
            "w/\"x\"",
            "\"a\r\n\"",
        ] {
            assert!(EntityTag::parse(v).is_none(), "{v:?}");
        }
        let huge = format!("\"{}\"", "a".repeat(limits::MAX_HEADER_VALUE_BYTES));
        assert!(EntityTag::parse(&huge).is_none());
    }

    #[test]
    fn http_dates_all_formats() {
        let want = Some(784_111_777);
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), want);
        assert_eq!(parse_http_date("Sunday, 06-Nov-94 08:49:37 GMT"), want);
        assert_eq!(parse_http_date("Sun Nov  6 08:49:37 1994"), want);
        assert_eq!(
            format_http_date(784_111_777),
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
        assert_eq!(format_http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        let t = 1_760_000_000;
        assert_eq!(parse_http_date(&format_http_date(t)), Some(t));
    }

    #[test]
    fn hostile_http_date_garbage() {
        for v in [
            "",
            "yesterday",
            "Sun, 32 Nov 1994 08:49:37 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 PST",
            "Sun, 06 Nov 99999999999 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 GMT extra",
            "Sun, 29 Feb 2023 00:00:00 GMT",
        ] {
            assert_eq!(parse_http_date(v), None, "{v:?}");
        }
        assert_eq!(format_http_date(i64::MAX), "Fri, 31 Dec 9999 23:59:59 GMT");
        assert!(format_http_date(i64::MIN).ends_with("GMT"));
    }

    #[test]
    fn hostile_content_length() {
        assert_eq!(parse_content_length(" 42 "), Some(42));
        assert_eq!(parse_content_length("18446744073709551615"), Some(u64::MAX));
        for v in [
            "",
            "-1",
            "+1",
            "1,1",
            "42, 42",
            "1e3",
            "18446744073709551616",
            "0x10",
        ] {
            assert_eq!(parse_content_length(v), None, "{v:?}");
        }
    }

    #[test]
    fn hostile_header_value_bound() {
        assert!(header_value_ok(&"a".repeat(limits::MAX_HEADER_VALUE_BYTES)));
        assert!(!header_value_ok(
            &"a".repeat(limits::MAX_HEADER_VALUE_BYTES + 1)
        ));
        assert!(!header_value_ok("a\rb"));
        assert!(!header_value_ok("a\nb"));
        assert!(!header_value_ok("a\0b"));
    }
}
