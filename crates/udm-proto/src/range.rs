//! `Range` / `Content-Range` handling (RFC 9110 §14).

use udm_core::ByteRange;

use crate::error::ProtoError;
use crate::headers::header_value_ok;
use crate::text::parse_u64_strict;

/// A parsed `Content-Range: bytes start-end/total` (satisfied form only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentRange {
    /// First byte.
    pub start: u64,
    /// Last byte (inclusive), `>= start`.
    pub end_inclusive: u64,
    /// Complete length; `None` for `*`.
    pub total: Option<u64>,
}

impl ContentRange {
    /// Parses a `Content-Range` value. Unsatisfied ranges (`bytes */N`), inverted
    /// ranges, ranges past the total, overflow and garbage are errors.
    pub fn parse(v: &str) -> Result<Self, ProtoError> {
        if !header_value_ok(v) {
            return Err(ProtoError::HeaderValueInvalid {
                len: v.len(),
                max: udm_core::limits::MAX_HEADER_VALUE_BYTES,
            });
        }
        let bad = || ProtoError::ContentRangeInvalid;
        let v = v.trim();
        let (unit, rest) = v.split_once(' ').ok_or_else(bad)?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return Err(bad());
        }
        let (range, total) = rest.trim_start().split_once('/').ok_or_else(bad)?;
        let (a, b) = range.split_once('-').ok_or_else(bad)?;
        let start = parse_u64_strict(a).ok_or_else(bad)?;
        let end_inclusive = parse_u64_strict(b).ok_or_else(bad)?;
        let total = match total {
            "*" => None,
            t => Some(parse_u64_strict(t).ok_or_else(bad)?),
        };
        if end_inclusive < start || total.is_some_and(|t| end_inclusive >= t) {
            return Err(bad());
        }
        Ok(Self {
            start,
            end_inclusive,
            total,
        })
    }

    /// Half-open byte range covered by this response (`None` only on `u64::MAX` end).
    pub fn byte_range(&self) -> Option<ByteRange> {
        ByteRange::new(self.start, self.end_inclusive.checked_add(1)?)
    }

    fn display(&self) -> String {
        match self.total {
            Some(t) => format!("{}-{}/{}", self.start, self.end_inclusive, t),
            None => format!("{}-{}/*", self.start, self.end_inclusive),
        }
    }
}

/// `Range` header value for a non-empty range: `bytes=start-last`.
pub fn range_header(r: ByteRange) -> Option<String> {
    r.last_inclusive()
        .map(|last| format!("bytes={}-{}", r.start(), last))
}

/// Checks that a `206` response answers exactly the requested range of a file of
/// `expected_total` bytes (ADR 0003 §6). Missing header, a different start or end,
/// a different or unknown total, and garbage are all errors.
pub fn verify_content_range(
    requested: ByteRange,
    expected_total: u64,
    header: Option<&str>,
) -> Result<(), ProtoError> {
    let header = header.ok_or(ProtoError::ContentRangeMissing)?;
    let got = ContentRange::parse(header)?;
    let expected = match requested.last_inclusive() {
        Some(last) => format!("{}-{}/{}", requested.start(), last, expected_total),
        None => format!("{}-?/{}", requested.start(), expected_total),
    };
    let matches = requested.last_inclusive() == Some(got.end_inclusive)
        && got.start == requested.start()
        && got.total == Some(expected_total);
    if matches {
        Ok(())
    } else {
        Err(ProtoError::ContentRangeMismatch {
            expected,
            actual: got.display(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use udm_core::limits;

    fn r(a: u64, b: u64) -> ByteRange {
        ByteRange::new(a, b).unwrap()
    }

    #[test]
    fn parse_forms() {
        let c = ContentRange::parse("bytes 0-499/1234").unwrap();
        assert_eq!(
            c,
            ContentRange {
                start: 0,
                end_inclusive: 499,
                total: Some(1234)
            }
        );
        assert_eq!(c.byte_range(), Some(r(0, 500)));
        let c = ContentRange::parse(" BYTES 10-19/* ").unwrap();
        assert_eq!(c.total, None);
    }

    #[test]
    fn header_for_range() {
        assert_eq!(range_header(r(0, 500)).as_deref(), Some("bytes=0-499"));
        assert_eq!(range_header(r(7, 7)), None);
    }

    #[test]
    fn verify_ok() {
        assert!(verify_content_range(r(100, 200), 1000, Some("bytes 100-199/1000")).is_ok());
    }

    #[test]
    fn hostile_content_range_mismatch() {
        let req = r(100, 200);
        for h in [
            "bytes 101-199/1000", // wrong start
            "bytes 100-198/1000", // short end
            "bytes 100-299/1000", // long end
            "bytes 100-199/999",  // wrong total
            "bytes 100-199/*",    // unknown total
            "bytes 0-999/1000",   // whole file
        ] {
            let e = verify_content_range(req, 1000, Some(h)).unwrap_err();
            assert!(
                matches!(e, ProtoError::ContentRangeMismatch { .. }),
                "{h}: {e:?}"
            );
            assert_eq!(e.to_message().key, "error-proto-content-range");
        }
        assert_eq!(
            verify_content_range(req, 1000, None),
            Err(ProtoError::ContentRangeMissing)
        );
        // Empty requested range can never be satisfied.
        assert!(verify_content_range(r(5, 5), 1000, Some("bytes 5-5/1000")).is_err());
    }

    #[test]
    fn hostile_content_range_garbage() {
        for h in [
            "",
            "bytes",
            "bytes 1-2",
            "bytes */1000",
            "bytes 5-4/1000",
            "bytes 0-1000/1000",
            "items 0-9/10",
            "bytes -1-9/10",
            "bytes +0-9/10",
            "bytes 0 - 9/10",
            "bytes 0-9/10/11",
            "bytes 0x0-9/10",
            "bytes 0-9/ten",
        ] {
            assert!(ContentRange::parse(h).is_err(), "{h:?}");
            assert!(
                verify_content_range(r(0, 10), 10, Some(h)).is_err(),
                "{h:?}"
            );
        }
    }

    #[test]
    fn hostile_content_range_overflow() {
        for h in [
            "bytes 0-18446744073709551616/18446744073709551617",
            "bytes 99999999999999999999999-1/2",
            "bytes 0-18446744073709551615/*",
        ] {
            let parsed = ContentRange::parse(h);
            if let Ok(c) = parsed {
                assert_eq!(c.byte_range(), None);
            }
            assert!(
                verify_content_range(r(0, 10), 10, Some(h)).is_err(),
                "{h:?}"
            );
        }
        let huge = format!("bytes 0-9/{}", "9".repeat(limits::MAX_HEADER_VALUE_BYTES));
        assert!(matches!(
            ContentRange::parse(&huge),
            Err(ProtoError::HeaderValueInvalid { .. })
        ));
    }
}
