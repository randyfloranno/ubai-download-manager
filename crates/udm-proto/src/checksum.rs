//! Checksum specifications (`sha256:<hex>`, `md5:<hex>`, bare hex) and hashing.
//! MD5 is weak and only used to compare against a published value.

use md5::Md5;
use sha2::{Digest, Sha256};

use crate::error::ProtoError;
use crate::text::hex_val;

/// Longest checksum specification accepted.
const MAX_SPEC_BYTES: usize = 128;

/// Supported checksum algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChecksumAlgo {
    /// MD5 (16 bytes). Weak: comparison only.
    Md5,
    /// SHA-256 (32 bytes).
    Sha256,
}

impl ChecksumAlgo {
    /// Digest length in bytes.
    pub fn digest_len(self) -> usize {
        match self {
            Self::Md5 => 16,
            Self::Sha256 => 32,
        }
    }

    /// Spec prefix, `md5` or `sha256`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha256 => "sha256",
        }
    }
}

/// An expected checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumSpec {
    /// Algorithm.
    pub algo: ChecksumAlgo,
    /// Expected digest bytes (length matches the algorithm).
    pub expected: Vec<u8>,
}

impl ChecksumSpec {
    /// Parses `sha256:<hex>`, `sha-256:<hex>`, `md5:<hex>` (prefix case-insensitive) or
    /// bare hex (32 digits → MD5, 64 → SHA-256). Hex may be upper or lower case.
    pub fn parse(s: &str) -> Result<Self, ProtoError> {
        if s.len() > MAX_SPEC_BYTES {
            return Err(ProtoError::ChecksumInvalid);
        }
        let s = s.trim();
        let (algo, hex) = match s.split_once(':') {
            Some((p, h)) => {
                let algo = if p.eq_ignore_ascii_case("sha256") || p.eq_ignore_ascii_case("sha-256")
                {
                    ChecksumAlgo::Sha256
                } else if p.eq_ignore_ascii_case("md5") {
                    ChecksumAlgo::Md5
                } else {
                    return Err(ProtoError::ChecksumInvalid);
                };
                (algo, h.trim())
            }
            None => match s.len() {
                32 => (ChecksumAlgo::Md5, s),
                64 => (ChecksumAlgo::Sha256, s),
                _ => return Err(ProtoError::ChecksumInvalid),
            },
        };
        let expected = from_hex(hex).ok_or(ProtoError::ChecksumInvalid)?;
        if expected.len() != algo.digest_len() {
            return Err(ProtoError::ChecksumInvalid);
        }
        Ok(Self { algo, expected })
    }

    /// Canonical form, e.g. `sha256:<lower-hex>`.
    pub fn to_string_spec(&self) -> String {
        format!("{}:{}", self.algo.name(), to_hex(&self.expected))
    }

    /// True if `digest` equals the expected value.
    pub fn matches(&self, digest: &[u8]) -> bool {
        self.expected == digest
    }
}

/// Lower-case hex encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        for nibble in [b >> 4, b & 0x0f] {
            if let Some(d) = DIGITS.get(usize::from(nibble)) {
                out.push(char::from(*d));
            }
        }
    }
    out
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks(2)
        .map(|pair| match pair {
            [hi, lo] => hex_val(*hi)?.checked_mul(16)?.checked_add(hex_val(*lo)?),
            _ => None,
        })
        .collect()
}

/// Incremental hasher for either algorithm.
#[derive(Clone, Debug)]
pub enum Hasher {
    /// MD5 state.
    Md5(Md5),
    /// SHA-256 state.
    Sha256(Sha256),
}

impl Hasher {
    /// New hasher for `algo`.
    pub fn new(algo: ChecksumAlgo) -> Self {
        match algo {
            ChecksumAlgo::Md5 => Self::Md5(Md5::new()),
            ChecksumAlgo::Sha256 => Self::Sha256(Sha256::new()),
        }
    }

    /// Feeds bytes.
    pub fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(h) => h.update(data),
            Self::Sha256(h) => h.update(data),
        }
    }

    /// Final digest.
    pub fn finalize(self) -> Vec<u8> {
        match self {
            Self::Md5(h) => h.finalize().to_vec(),
            Self::Sha256(h) => h.finalize().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";

    #[test]
    fn parse_and_round_trip() {
        let s = ChecksumSpec::parse(&format!("SHA256:{}", EMPTY_SHA256.to_uppercase())).unwrap();
        assert_eq!(s.algo, ChecksumAlgo::Sha256);
        assert_eq!(s.to_string_spec(), format!("sha256:{EMPTY_SHA256}"));
        assert_eq!(
            ChecksumSpec::parse(EMPTY_MD5).unwrap().algo,
            ChecksumAlgo::Md5
        );
        assert_eq!(
            ChecksumSpec::parse(EMPTY_SHA256).unwrap().algo,
            ChecksumAlgo::Sha256
        );
        assert_eq!(
            ChecksumSpec::parse(&format!("sha-256: {EMPTY_SHA256} "))
                .unwrap()
                .algo,
            ChecksumAlgo::Sha256
        );
    }

    #[test]
    fn hashers_match_known_vectors() {
        let h = Hasher::new(ChecksumAlgo::Sha256);
        assert_eq!(to_hex(&h.finalize()), EMPTY_SHA256);
        let mut h = Hasher::new(ChecksumAlgo::Md5);
        h.update(b"");
        let d = h.finalize();
        assert_eq!(to_hex(&d), EMPTY_MD5);
        assert!(ChecksumSpec::parse(EMPTY_MD5).unwrap().matches(&d));
        let mut h = Hasher::new(ChecksumAlgo::Sha256);
        h.update(b"ab");
        h.update(b"c");
        assert_eq!(
            to_hex(&h.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hostile_checksum_spec() {
        for s in [
            "",
            "sha256:",
            "sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709",
            &format!("md5:{EMPTY_SHA256}"),
            &format!("sha256:{EMPTY_MD5}"),
            &format!("sha256:{}", &EMPTY_SHA256[1..]),
            &format!("sha256:{}zz", &EMPTY_SHA256[2..]),
            "abc",
            &"0".repeat(4096),
            "md5:+41d8cd98f00b204e9800998ecf8427e",
            "md5:é1d8cd98f00b204e9800998ecf8427",
        ] {
            assert_eq!(
                ChecksumSpec::parse(s),
                Err(ProtoError::ChecksumInvalid),
                "{s:?}"
            );
        }
    }
}
