//! `Content-Disposition` filename extraction (RFC 6266, RFC 8187 `filename*`).
//!
//! The result is **raw and unsanitised**; always pass it through
//! [`crate::filename::sanitize_filename`] before touching the file system.

use crate::headers::header_value_ok;

/// Maximum parameters inspected in one header (the value itself is already bounded).
const MAX_PARAMS: usize = 64;

/// Returns the file name carried by a `Content-Disposition` value.
///
/// `filename*` (RFC 8187, UTF-8 or ISO-8859-1) is preferred over `filename` when it
/// decodes; otherwise `filename` is used. Values longer than
/// `limits::MAX_HEADER_VALUE_BYTES` or containing CR/LF/NUL are rejected (`None`).
pub fn filename_from_content_disposition(v: &str) -> Option<String> {
    if !header_value_ok(v) {
        return None;
    }
    let params = parse_params(v);
    let mut plain: Option<String> = None;
    let mut ext: Option<String> = None;
    for (name, value) in params {
        if name.eq_ignore_ascii_case("filename*") {
            if ext.is_none() {
                ext = decode_ext_value(&value).filter(|s| !s.is_empty());
            }
        } else if name.eq_ignore_ascii_case("filename") && plain.is_none() {
            plain = Some(value).filter(|s| !s.is_empty());
        }
    }
    ext.or(plain)
}

/// Splits `type; a=b; c="d"` into `(name, value)` pairs (quoted-strings unescaped).
/// Malformed parameters are skipped; parsing never fails.
fn parse_params(v: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = v.chars().peekable();
    // Skip the disposition type.
    for c in chars.by_ref() {
        if c == ';' {
            break;
        }
    }
    while out.len() < MAX_PARAMS {
        // Skip separators.
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ';') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ';' {
                break;
            }
            name.push(c);
            chars.next();
        }
        let name = name.trim().to_owned();
        if chars.peek() != Some(&'=') {
            continue; // parameter without value (or `;`): skip
        }
        chars.next(); // '='
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        let mut valid = true;
        if chars.peek() == Some(&'"') {
            chars.next();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some(e) => value.push(e),
                        None => break,
                    },
                    '"' => {
                        closed = true;
                        break;
                    }
                    other => value.push(other),
                }
            }
            valid = closed;
            // Skip junk up to the next ';'.
            while chars.peek().is_some_and(|c| *c != ';') {
                chars.next();
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ';' {
                    break;
                }
                value.push(c);
                chars.next();
            }
            value = value.trim_end().to_owned();
        }
        if valid && !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

/// Decodes an RFC 8187 `ext-value`: `charset'lang'pct-encoded`.
fn decode_ext_value(v: &str) -> Option<String> {
    let mut parts = v.splitn(3, '\'');
    let charset = parts.next()?.trim();
    let _lang = parts.next()?;
    let encoded = parts.next()?;
    let bytes = pct_decode_strict(encoded)?;
    if charset.eq_ignore_ascii_case("utf-8") {
        String::from_utf8(bytes).ok()
    } else if charset.eq_ignore_ascii_case("iso-8859-1") {
        Some(bytes.into_iter().map(char::from).collect())
    } else {
        None
    }
}

/// Percent-decoding that rejects malformed escapes (`%zz`, trailing `%`).
fn pct_decode_strict(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut it = s.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hi = crate::text::hex_val(it.next()?)?;
            let lo = crate::text::hex_val(it.next()?)?;
            out.push(hi.checked_mul(16)?.checked_add(lo)?);
        } else {
            out.push(b);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filename::sanitize_filename;
    use udm_core::limits;

    #[test]
    fn plain_and_quoted() {
        let f = filename_from_content_disposition;
        assert_eq!(f("attachment; filename=a.zip").as_deref(), Some("a.zip"));
        assert_eq!(
            f("attachment; filename=\"my file.zip\"").as_deref(),
            Some("my file.zip")
        );
        assert_eq!(
            f("attachment; filename=\"a\\\"b.zip\"").as_deref(),
            Some("a\"b.zip")
        );
        assert_eq!(f("ATTACHMENT; FILENAME = x.bin ").as_deref(), Some("x.bin"));
        assert_eq!(f("inline"), None);
    }

    #[test]
    fn ext_value_preferred() {
        let v = "attachment; filename=\"EURO rates.txt\"; filename*=UTF-8''%e2%82%ac%20rates.txt";
        assert_eq!(
            filename_from_content_disposition(v).as_deref(),
            Some("€ rates.txt")
        );
        let v = "attachment; filename*=iso-8859-1'en'%A3%20rates.txt";
        assert_eq!(
            filename_from_content_disposition(v).as_deref(),
            Some("£ rates.txt")
        );
        // Order does not matter.
        let v = "attachment; filename*=UTF-8''b.txt; filename=a.txt";
        assert_eq!(
            filename_from_content_disposition(v).as_deref(),
            Some("b.txt")
        );
    }

    #[test]
    fn hostile_content_disposition_path_traversal() {
        let raw =
            filename_from_content_disposition("attachment; filename=\"../../etc/passwd\"").unwrap();
        assert_eq!(raw, "../../etc/passwd"); // raw on purpose
        assert_eq!(sanitize_filename(&raw), "passwd");
        let raw = filename_from_content_disposition(
            "attachment; filename*=UTF-8''..%2F..%2F..%5Cwindows%5Csystem32%5Cevil.dll",
        )
        .unwrap();
        assert_eq!(sanitize_filename(&raw), "evil.dll");
        let raw =
            filename_from_content_disposition("attachment; filename=\"C:\\boot.ini\"").unwrap();
        // In a quoted-string `\b` is an escape for `b`, so the raw name is `C:boot.ini`.
        assert_eq!(raw, "C:boot.ini");
        assert_eq!(sanitize_filename(&raw), "C_boot.ini");
        let raw =
            filename_from_content_disposition(r"attachment; filename=C:\Windows\x.exe").unwrap();
        assert_eq!(sanitize_filename(&raw), "x.exe");
    }

    #[test]
    fn hostile_content_disposition_bad_encoding() {
        let f = filename_from_content_disposition;
        // Invalid UTF-8 in filename* falls back to filename.
        assert_eq!(
            f("attachment; filename*=UTF-8''%ff%fe.txt; filename=ok.txt").as_deref(),
            Some("ok.txt")
        );
        // Malformed escapes, unknown charset, missing quotes in ext-value.
        assert_eq!(f("attachment; filename*=UTF-8''%zz.txt"), None);
        assert_eq!(f("attachment; filename*=UTF-8''abc%"), None);
        assert_eq!(f("attachment; filename*=koi8-r''abc"), None);
        assert_eq!(f("attachment; filename*=abc"), None);
        // Unterminated quoted-string.
        assert_eq!(f("attachment; filename=\"abc"), None);
        // Empty.
        assert_eq!(f("attachment; filename=\"\""), None);
        assert_eq!(f(""), None);
        // Header injection characters.
        assert_eq!(f("attachment; filename=a\r\nX: y"), None);
        // NUL percent-encoded is decoded raw; the sanitiser removes it.
        let raw = f("attachment; filename*=UTF-8''a%00b.txt").unwrap();
        assert_eq!(sanitize_filename(&raw), "ab.txt");
    }

    #[test]
    fn hostile_content_disposition_huge_value() {
        let v = format!(
            "attachment; filename=\"{}.zip\"",
            "a".repeat(limits::MAX_HEADER_VALUE_BYTES)
        );
        assert_eq!(filename_from_content_disposition(&v), None);
        // Many parameters are bounded and never loop forever.
        let v = format!("attachment{}; filename=x", ";a=b".repeat(2_000));
        assert!(v.len() <= limits::MAX_HEADER_VALUE_BYTES);
        assert_eq!(filename_from_content_disposition(&v), None);
        let v = ";;;;====\"\"\\\\;".repeat(100);
        let _ = filename_from_content_disposition(&v);
    }
}
