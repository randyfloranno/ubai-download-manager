//! Small string helpers shared by the parsers (no panics, no indexing).

/// Longest prefix of `s` that is at most `max` bytes and ends on a char boundary.
pub(crate) fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    s.get(..cut).unwrap_or("")
}

/// Strict unsigned decimal: ASCII digits only, at least one digit, no sign.
pub(crate) fn parse_u64_strict(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok()
}

/// Decodes one lower- or upper-case hex digit.
pub(crate) fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => b.checked_sub(b'0'),
        b'a'..=b'f' => b.checked_sub(b'a').and_then(|v| v.checked_add(10)),
        b'A'..=b'F' => b.checked_sub(b'A').and_then(|v| v.checked_add(10)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_on_char_boundary() {
        assert_eq!(truncate_bytes("héllo", 2), "h");
        assert_eq!(truncate_bytes("héllo", 3), "hé");
        assert_eq!(truncate_bytes("abc", 10), "abc");
        assert_eq!(truncate_bytes("é", 1), "");
    }

    #[test]
    fn strict_digits() {
        assert_eq!(parse_u64_strict("42"), Some(42));
        assert_eq!(parse_u64_strict("+42"), None);
        assert_eq!(parse_u64_strict(""), None);
        assert_eq!(parse_u64_strict("99999999999999999999999"), None);
    }
}
