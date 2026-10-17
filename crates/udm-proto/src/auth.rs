//! HTTP authentication: Basic (RFC 7617) and Digest (RFC 7616).
//!
//! These functions build header values from secrets; callers must never log them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use md5::Md5;
use sha2::{Digest, Sha256};
use udm_core::limits;

use crate::checksum::to_hex;
use crate::error::ProtoError;
use crate::headers::header_value_ok;

/// `Authorization` value for Basic authentication (UTF-8 credentials).
pub fn basic_authorization(user: &str, pass: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
}

/// Digest hash algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestAlgorithm {
    /// `MD5` (default when absent).
    Md5,
    /// `MD5-sess`.
    Md5Sess,
    /// `SHA-256`.
    Sha256,
    /// `SHA-256-sess`.
    Sha256Sess,
}

impl DigestAlgorithm {
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("MD5") {
            Some(Self::Md5)
        } else if s.eq_ignore_ascii_case("MD5-sess") {
            Some(Self::Md5Sess)
        } else if s.eq_ignore_ascii_case("SHA-256") {
            Some(Self::Sha256)
        } else if s.eq_ignore_ascii_case("SHA-256-sess") {
            Some(Self::Sha256Sess)
        } else {
            None
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Md5Sess => "MD5-sess",
            Self::Sha256 => "SHA-256",
            Self::Sha256Sess => "SHA-256-sess",
        }
    }

    fn is_sess(self) -> bool {
        matches!(self, Self::Md5Sess | Self::Sha256Sess)
    }

    fn hash(self, data: &str) -> String {
        match self {
            Self::Md5 | Self::Md5Sess => to_hex(&Md5::digest(data.as_bytes())),
            Self::Sha256 | Self::Sha256Sess => to_hex(&Sha256::digest(data.as_bytes())),
        }
    }
}

/// A Digest challenge from `WWW-Authenticate` / `Proxy-Authenticate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestChallenge {
    /// `realm`.
    pub realm: String,
    /// `nonce`.
    pub nonce: String,
    /// `opaque`, echoed back verbatim.
    pub opaque: Option<String>,
    /// Hash algorithm.
    pub algorithm: DigestAlgorithm,
    /// Server offered `qop=auth`.
    pub qop_auth: bool,
    /// `stale=true`: retry with the new nonce without asking the user again.
    pub stale: bool,
}

/// A supported authentication challenge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Challenge {
    /// Basic.
    Basic {
        /// `realm`, if given.
        realm: Option<String>,
    },
    /// Digest.
    Digest(DigestChallenge),
}

/// Parses a `WWW-Authenticate` value into the supported challenges, in order.
///
/// Unknown schemes (Bearer, Negotiate, ...), Digest challenges without `realm`/`nonce`,
/// with an unsupported algorithm, or offering only `qop=auth-int` are skipped.
/// Errors: value longer than `limits::MAX_HEADER_VALUE_BYTES` or containing CR/LF/NUL.
pub fn parse_www_authenticate(v: &str) -> Result<Vec<Challenge>, ProtoError> {
    if !header_value_ok(v) {
        return Err(ProtoError::HeaderValueInvalid {
            len: v.len(),
            max: limits::MAX_HEADER_VALUE_BYTES,
        });
    }
    let mut out = Vec::new();
    for (scheme, params) in split_challenges(v) {
        let get = |name: &str| {
            params
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        if scheme.eq_ignore_ascii_case("Basic") {
            out.push(Challenge::Basic {
                realm: get("realm"),
            });
        } else if scheme.eq_ignore_ascii_case("Digest") {
            let (Some(realm), Some(nonce)) = (get("realm"), get("nonce")) else {
                continue;
            };
            let algorithm = match get("algorithm") {
                None => DigestAlgorithm::Md5,
                Some(a) => match DigestAlgorithm::parse(&a) {
                    Some(a) => a,
                    None => continue,
                },
            };
            let qop = get("qop");
            let qop_auth = qop
                .as_deref()
                .is_some_and(|q| q.split(',').any(|t| t.trim().eq_ignore_ascii_case("auth")));
            if qop.is_some() && !qop_auth {
                continue; // auth-int only: unsupported
            }
            let stale = get("stale").is_some_and(|s| s.trim().eq_ignore_ascii_case("true"));
            out.push(Challenge::Digest(DigestChallenge {
                realm,
                nonce,
                opaque: get("opaque"),
                algorithm,
                qop_auth,
                stale,
            }));
        }
    }
    Ok(out)
}

/// Maximum challenges and parameters inspected (input is already length-bounded).
const MAX_CHALLENGES: usize = 16;
const MAX_AUTH_PARAMS: usize = 32;

type Params = Vec<(String, String)>;

/// Splits `Scheme a=b, c="d", Other x=y` into schemes with their parameters.
fn split_challenges(v: &str) -> Vec<(String, Params)> {
    let mut out: Vec<(String, Params)> = Vec::new();
    let mut p = Cursor {
        chars: v.chars().collect(),
        pos: 0,
    };
    while out.len() < MAX_CHALLENGES {
        p.skip(|c| c.is_whitespace() || c == ',');
        if p.peek().is_none() {
            break;
        }
        let scheme = p.token();
        if scheme.is_empty() {
            p.bump(); // stray character: make progress
            continue;
        }
        let mut params = Params::new();
        loop {
            p.skip(char::is_whitespace);
            let save = p.pos;
            let name = p.token();
            p.skip(char::is_whitespace);
            if name.is_empty() || p.peek() != Some('=') {
                // Either the next scheme or a token68: rewind, let the outer loop decide.
                p.pos = save;
                if !name.is_empty() && p.peek_after_token_is_param_list() {
                    break;
                }
                // token68 (e.g. `Negotiate abc==`): consume up to the next comma.
                if !name.is_empty() {
                    p.skip(|c| c != ',');
                }
                break;
            }
            p.bump(); // '='
            p.skip(char::is_whitespace);
            let value = if p.peek() == Some('"') {
                p.quoted()
            } else {
                p.token()
            };
            if params.len() < MAX_AUTH_PARAMS {
                params.push((name, value));
            }
            p.skip(char::is_whitespace);
            if p.peek() == Some(',') {
                p.bump();
                continue;
            }
            break;
        }
        out.push((scheme, params));
    }
    out
}

struct Cursor {
    chars: Vec<char>,
    pos: usize,
}

impl Cursor {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) {
        if self.pos < self.chars.len() {
            self.pos = self.pos.saturating_add(1);
        }
    }

    fn skip(&mut self, f: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&f) {
            self.bump();
        }
    }

    /// RFC 9110 token characters.
    fn token(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    fn quoted(&mut self) -> String {
        let mut s = String::new();
        self.bump(); // opening quote
        while let Some(c) = self.peek() {
            self.bump();
            match c {
                '"' => break,
                '\\' => {
                    if let Some(e) = self.peek() {
                        s.push(e);
                        self.bump();
                    }
                }
                other => s.push(other),
            }
        }
        s
    }

    /// At a token that is followed by whitespace and then `name=`: a new challenge.
    fn peek_after_token_is_param_list(&self) -> bool {
        let mut i = self.pos;
        let at = |i: usize| self.chars.get(i).copied();
        while at(i).is_some_and(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)) {
            i = i.saturating_add(1);
        }
        // A scheme name is followed by whitespace, a comma or the end.
        at(i).is_none_or(|c| c.is_whitespace() || c == ',')
    }
}

/// `Authorization` value for a Digest challenge (RFC 7616 §3.4).
///
/// `nc` is the nonce count for this nonce (starts at 1); `cnonce` must be a fresh
/// random value supplied by the caller (keeps this function pure).
pub fn digest_authorization(
    c: &DigestChallenge,
    user: &str,
    pass: &str,
    method: &str,
    uri: &str,
    cnonce: &str,
    nc: u32,
) -> String {
    let alg = c.algorithm;
    let mut ha1 = alg.hash(&format!("{user}:{}:{pass}", c.realm));
    if alg.is_sess() {
        ha1 = alg.hash(&format!("{ha1}:{}:{cnonce}", c.nonce));
    }
    let ha2 = alg.hash(&format!("{method}:{uri}"));
    let nc_hex = format!("{nc:08x}");
    let response = if c.qop_auth {
        alg.hash(&format!("{ha1}:{}:{nc_hex}:{cnonce}:auth:{ha2}", c.nonce))
    } else {
        alg.hash(&format!("{ha1}:{}:{ha2}", c.nonce))
    };
    let mut h = format!(
        "Digest username=\"{}\", realm=\"{}\", uri=\"{}\", algorithm={}, nonce=\"{}\"",
        quote(user),
        quote(&c.realm),
        quote(uri),
        alg.name(),
        quote(&c.nonce),
    );
    if c.qop_auth {
        h.push_str(&format!(
            ", nc={nc_hex}, cnonce=\"{}\", qop=auth",
            quote(cnonce)
        ));
    }
    h.push_str(&format!(", response=\"{response}\""));
    if let Some(o) = &c.opaque {
        h.push_str(&format!(", opaque=\"{}\"", quote(o)));
    }
    h
}

/// Escapes a quoted-string body; CR/LF/NUL are dropped so a header can never be split.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\r' | '\n' | '\0' => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_header() {
        // RFC 7617 §2 example.
        assert_eq!(
            basic_authorization("Aladdin", "open sesame"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    /// RFC 7616 §3.9.1 example, both algorithms.
    #[test]
    fn digest_rfc7616_vectors() {
        let header = r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS", Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let ch = parse_www_authenticate(header).unwrap();
        assert_eq!(ch.len(), 2);
        let (Challenge::Digest(sha), Challenge::Digest(md5)) = (&ch[0], &ch[1]) else {
            panic!("expected two digest challenges: {ch:?}");
        };
        assert_eq!(sha.algorithm, DigestAlgorithm::Sha256);
        assert_eq!(md5.algorithm, DigestAlgorithm::Md5);
        assert!(sha.qop_auth && !sha.stale);
        assert_eq!(sha.realm, "http-auth@example.org");
        assert_eq!(
            sha.opaque.as_deref(),
            Some("FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS")
        );
        let cnonce = "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ";
        let a = digest_authorization(
            sha,
            "Mufasa",
            "Circle of Life",
            "GET",
            "/dir/index.html",
            cnonce,
            1,
        );
        assert!(
            a.contains(
                "response=\"753927fa0e85d155564e2e272a28d1802ca10daf4496794697cf8db5856cb6c1\""
            ),
            "{a}"
        );
        assert!(
            a.contains("nc=00000001") && a.contains("qop=auth") && a.contains("algorithm=SHA-256")
        );
        let a = digest_authorization(
            md5,
            "Mufasa",
            "Circle of Life",
            "GET",
            "/dir/index.html",
            cnonce,
            1,
        );
        assert!(
            a.contains("response=\"8ca523f5e9506fed4657c9700eebdbec\""),
            "{a}"
        );
        assert!(a.contains("opaque=\"FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS\""));
    }

    #[test]
    fn rfc2069_compat_and_sess() {
        let c = DigestChallenge {
            realm: "r".into(),
            nonce: "n".into(),
            opaque: None,
            algorithm: DigestAlgorithm::Md5,
            qop_auth: false,
            stale: false,
        };
        let a = digest_authorization(&c, "u", "p", "GET", "/", "c", 1);
        let ha1 = DigestAlgorithm::Md5.hash("u:r:p");
        let ha2 = DigestAlgorithm::Md5.hash("GET:/");
        let want = DigestAlgorithm::Md5.hash(&format!("{ha1}:n:{ha2}"));
        assert!(a.contains(&want) && !a.contains("qop"));
        let s = DigestChallenge {
            algorithm: DigestAlgorithm::Sha256Sess,
            qop_auth: true,
            ..c
        };
        let a = digest_authorization(&s, "u", "p", "GET", "/", "c", 2);
        assert!(a.contains("algorithm=SHA-256-sess") && a.contains("nc=00000002"));
    }

    #[test]
    fn mixed_schemes() {
        let ch = parse_www_authenticate("Negotiate abc==, Bearer realm=\"x\", Basic realm=\"site\", Digest realm=\"d\", nonce=\"n\", stale=TRUE").unwrap();
        assert_eq!(ch.len(), 2, "{ch:?}");
        assert_eq!(
            ch[0],
            Challenge::Basic {
                realm: Some("site".into())
            }
        );
        let Challenge::Digest(d) = &ch[1] else {
            panic!()
        };
        assert!(d.stale && !d.qop_auth && d.algorithm == DigestAlgorithm::Md5);
        assert_eq!(
            parse_www_authenticate("Basic").unwrap(),
            vec![Challenge::Basic { realm: None }]
        );
    }

    #[test]
    fn hostile_www_authenticate() {
        let huge = format!(
            "Basic realm=\"{}\"",
            "a".repeat(limits::MAX_HEADER_VALUE_BYTES)
        );
        assert!(matches!(
            parse_www_authenticate(&huge),
            Err(ProtoError::HeaderValueInvalid { .. })
        ));
        assert!(parse_www_authenticate("Basic realm=\"a\r\nX: y\"").is_err());
        // Digest without nonce, unsupported algorithm, auth-int only: skipped.
        for v in [
            "Digest realm=\"r\"",
            "Digest realm=\"r\", nonce=\"n\", algorithm=SHA-512-256",
            "Digest realm=\"r\", nonce=\"n\", qop=\"auth-int\"",
        ] {
            assert!(parse_www_authenticate(v).unwrap().is_empty(), "{v}");
        }
        for v in [
            "",
            ",,,,",
            "=====",
            "\"\"\"",
            "Digest realm=\"unterminated",
            "Basic ,,, realm",
            "  ",
            "Digest ==== , ,",
        ] {
            let _ = parse_www_authenticate(v).unwrap();
        }
        let many = "Basic realm=x, ".repeat(500);
        assert!(parse_www_authenticate(&many).unwrap().len() <= MAX_CHALLENGES);
        // Quoting never lets a hostile value break out of the header.
        let c = DigestChallenge {
            realm: "r\"\r\nX: y".into(),
            nonce: "n".into(),
            opaque: None,
            algorithm: DigestAlgorithm::Md5,
            qop_auth: true,
            stale: false,
        };
        let a = digest_authorization(&c, "u\"", "p", "GET", "/", "c", 1);
        assert!(header_value_ok(&a));
        assert!(a.contains("username=\"u\\\"\"") && a.contains("realm=\"r\\\"X: y\""));
    }
}
