//! Opaque download identifiers, rendered as `d-000001`.

use core::fmt;
use core::str::FromStr;

/// Maximum number of digits accepted when parsing an id.
const MAX_ID_DIGITS: usize = 12;

/// Opaque download id. Displayed as `d-` followed by at least six digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DownloadId(u64);

impl DownloadId {
    /// Creates an id from its numeric value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Numeric value.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// The id following this one, or `None` on overflow.
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl fmt::Display for DownloadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "d-{:06}", self.0)
    }
}

/// Error returned for a malformed id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIdError;

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid download id")
    }
}

impl FromStr for DownloadId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits = s.strip_prefix("d-").ok_or(ParseIdError)?;
        if digits.is_empty()
            || digits.len() > MAX_ID_DIGITS
            || !digits.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(ParseIdError);
        }
        digits.parse::<u64>().map(Self).map_err(|_| ParseIdError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_roundtrip() {
        let id = DownloadId::new(42);
        assert_eq!(id.to_string(), "d-000042");
        assert_eq!("d-000042".parse::<DownloadId>().unwrap(), id);
        assert_eq!(
            "d-1234567".parse::<DownloadId>().unwrap().value(),
            1_234_567
        );
    }

    #[test]
    fn hostile_download_id_parse_garbage() {
        for bad in [
            "",
            "d-",
            "x-000001",
            "d-+1",
            "d--1",
            "d-0x10",
            "d-1 ",
            " d-1",
            "d-1234567890123",
            "d-١٢٣",
        ] {
            assert!(bad.parse::<DownloadId>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn next_overflow_is_none() {
        assert!(DownloadId::new(u64::MAX).next().is_none());
    }
}
