//! Download URL validation, credentials, redirects and downgrade detection.

use ::url::Url;
use percent_encoding::percent_decode_str;
use udm_core::limits;

use crate::error::ProtoError;
use crate::headers::header_value_ok;
use crate::text::truncate_bytes;

/// Schemes UDM downloads from.
const ALLOWED_SCHEMES: [&str; 4] = ["http", "https", "ftp", "ftps"];

/// Longest scheme echoed back in an error message.
const MAX_SCHEME_IN_ERROR: usize = 32;

/// Parses and validates a URL the user (or a browser/clipboard/list) asked to download.
///
/// Accepts `http`, `https`, `ftp` and `ftps` only, requires a host, and bounds both the
/// input and the normalised URL to `limits::MAX_URL_BYTES`. Surrounding whitespace is trimmed.
pub fn validate_download_url(s: &str) -> Result<Url, ProtoError> {
    let max = limits::MAX_URL_BYTES;
    if s.len() > max {
        return Err(ProtoError::UrlTooLong { len: s.len(), max });
    }
    let parsed = Url::parse(s.trim()).map_err(|_| ProtoError::UrlInvalid)?;
    check_url(parsed)
}

fn check_url(u: Url) -> Result<Url, ProtoError> {
    let max = limits::MAX_URL_BYTES;
    if !ALLOWED_SCHEMES.contains(&u.scheme()) {
        return Err(ProtoError::UrlScheme {
            scheme: truncate_bytes(u.scheme(), MAX_SCHEME_IN_ERROR).to_owned(),
        });
    }
    if u.host_str().is_none_or(str::is_empty) {
        return Err(ProtoError::UrlNoHost);
    }
    // Normalisation (percent-encoding) can grow the URL; bound the result too.
    let len = u.as_str().len();
    if len > max {
        return Err(ProtoError::UrlTooLong { len, max });
    }
    Ok(u)
}

/// Removes `user:password@` from the URL and returns it percent-decoded separately,
/// so credentials can go to the keychain and never into state files or logs.
pub fn split_credentials(u: &Url) -> (Url, Option<(String, String)>) {
    let user = u.username();
    let pass = u.password();
    if user.is_empty() && pass.is_none() {
        return (u.clone(), None);
    }
    let creds = (
        percent_decode_str(user).decode_utf8_lossy().into_owned(),
        percent_decode_str(pass.unwrap_or(""))
            .decode_utf8_lossy()
            .into_owned(),
    );
    let mut clean = u.clone();
    // Both setters only fail for cannot-be-a-base URLs, which have no userinfo and
    // therefore returned above; the results carry no further information.
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    (clean, Some(creds))
}

/// True when moving from `from` to `to` drops transport security
/// (`https`/`ftps` → `http`/`ftp`).
pub fn is_downgrade(from: &Url, to: &Url) -> bool {
    let secure = |s: &str| s == "https" || s == "ftps";
    let insecure = |s: &str| s == "http" || s == "ftp";
    secure(from.scheme()) && insecure(to.scheme())
}

/// Resolves a redirect `Location` against the request URL and validates the target
/// like any download URL. Downgrade policy is the caller's decision ([`is_downgrade`]).
pub fn resolve_redirect(base: &Url, location: &str) -> Result<Url, ProtoError> {
    let max = limits::MAX_URL_BYTES;
    if location.len() > max {
        return Err(ProtoError::UrlTooLong {
            len: location.len(),
            max,
        });
    }
    if !header_value_ok(location) || location.trim().is_empty() {
        return Err(ProtoError::RedirectInvalid);
    }
    let joined = base
        .join(location.trim())
        .map_err(|_| ProtoError::RedirectInvalid)?;
    check_url(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_supported_schemes() {
        for s in [
            "http://example.com/a.bin",
            "https://example.com/a.bin",
            "ftp://example.com/a.bin",
            "ftps://example.com/a.bin",
            "  https://example.com/a.bin\n",
        ] {
            assert!(validate_download_url(s).is_ok(), "{s}");
        }
    }

    #[test]
    fn hostile_url_too_long() {
        let s = format!("http://example.com/{}", "a".repeat(limits::MAX_URL_BYTES));
        assert!(matches!(
            validate_download_url(&s),
            Err(ProtoError::UrlTooLong { .. })
        ));
        // Normalisation growth is bounded too: each space becomes %20.
        let s = format!(
            "http://example.com/{}",
            " x".repeat(limits::MAX_URL_BYTES / 3)
        );
        assert!(s.len() <= limits::MAX_URL_BYTES);
        assert!(matches!(
            validate_download_url(&s),
            Err(ProtoError::UrlTooLong { .. })
        ));
        let ok = format!("http://example.com/{}", "a".repeat(100));
        assert!(validate_download_url(&ok).is_ok());
    }

    #[test]
    fn hostile_url_bad_scheme() {
        for s in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/plain,hi",
            "gopher://example.com/",
            "chrome://settings",
        ] {
            assert!(
                matches!(validate_download_url(s), Err(ProtoError::UrlScheme { .. })),
                "{s}"
            );
        }
        assert!(matches!(
            validate_download_url("not a url"),
            Err(ProtoError::UrlInvalid)
        ));
        assert!(validate_download_url("http://").is_err());
        assert!(validate_download_url("").is_err());
    }

    #[test]
    fn credentials_are_split_and_decoded() {
        let u = Url::parse("ftp://us%40er:p%3Ass@example.com/f.bin").unwrap();
        let (clean, creds) = split_credentials(&u);
        assert_eq!(clean.as_str(), "ftp://example.com/f.bin");
        assert_eq!(creds, Some(("us@er".to_owned(), "p:ss".to_owned())));
        let u = Url::parse("http://example.com/").unwrap();
        assert_eq!(split_credentials(&u), (u.clone(), None));
    }

    #[test]
    fn downgrade_detection() {
        let https = Url::parse("https://a/").unwrap();
        let http = Url::parse("http://a/").unwrap();
        let ftps = Url::parse("ftps://a/").unwrap();
        let ftp = Url::parse("ftp://a/").unwrap();
        assert!(is_downgrade(&https, &http));
        assert!(is_downgrade(&ftps, &ftp));
        assert!(is_downgrade(&https, &ftp));
        assert!(!is_downgrade(&http, &https));
        assert!(!is_downgrade(&https, &https));
    }

    #[test]
    fn redirects_resolve_relative() {
        let base = Url::parse("https://example.com/dir/a.bin").unwrap();
        assert_eq!(
            resolve_redirect(&base, "../b.bin").unwrap().as_str(),
            "https://example.com/b.bin"
        );
        assert_eq!(
            resolve_redirect(&base, "//cdn.example.com/c")
                .unwrap()
                .as_str(),
            "https://cdn.example.com/c"
        );
    }

    #[test]
    fn hostile_redirect_location() {
        let base = Url::parse("https://example.com/a").unwrap();
        assert!(resolve_redirect(&base, "file:///etc/passwd").is_err());
        assert!(resolve_redirect(&base, "javascript:alert(1)").is_err());
        assert!(resolve_redirect(&base, "http://x/\r\nSet-Cookie: a").is_err());
        assert!(resolve_redirect(&base, "").is_err());
        let long = format!("/{}", "a".repeat(limits::MAX_URL_BYTES));
        assert!(matches!(
            resolve_redirect(&base, &long),
            Err(ProtoError::UrlTooLong { .. })
        ));
    }
}
