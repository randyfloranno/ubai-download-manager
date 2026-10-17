//! Deterministic, code-generated download fixtures.
//!
//! Content is produced by splitmix64 per 8-byte block, indexed by offset, so any
//! byte range can be generated without generating the prefix. The seed depends on
//! the fixture name and its version, so `set_version` changes both content and
//! ETag.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Names of all built-in fixtures.
pub const FIXTURE_NAMES: [&str; 5] = ["tiny.txt", "small.bin", "medium.bin", "big.bin", "huge.bin"];

/// Content of `tiny.txt` at version 1.
pub const TINY_CONTENT: &[u8] = b"hello, world\n";

/// Version every fixture starts at.
pub const DEFAULT_VERSION: u32 = 1;

/// A deterministic fixture file (one name at one version).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixture {
    /// File name, e.g. `big.bin`.
    pub name: String,
    /// Length in bytes.
    pub len: u64,
    /// Content version (starts at [`DEFAULT_VERSION`]).
    pub version: u32,
    seed: u64,
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub(crate) fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fixture_len(name: &str) -> Option<u64> {
    Some(match name {
        "tiny.txt" => TINY_CONTENT.len() as u64,
        "small.bin" => 64 * 1024,
        "medium.bin" => 4 * 1024 * 1024,
        "big.bin" => 32 * 1024 * 1024,
        "huge.bin" => 256 * 1024 * 1024,
        _ => return None,
    })
}

/// The fixture `name` at the default version, or `None` for unknown names.
pub fn fixture(name: &str) -> Option<Fixture> {
    fixture_version(name, DEFAULT_VERSION)
}

/// The fixture `name` at a given content version.
pub fn fixture_version(name: &str, version: u32) -> Option<Fixture> {
    let len = fixture_len(name)?;
    let seed =
        splitmix64(fnv1a(name.as_bytes()) ^ u64::from(version).wrapping_mul(0xA24B_AED4_963E_E407));
    Some(Fixture {
        name: name.to_string(),
        len,
        version,
        seed,
    })
}

fn sha_cache() -> &'static Mutex<HashMap<(String, u32), String>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u32), String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl Fixture {
    fn is_plain_text(&self) -> bool {
        self.name == "tiny.txt" && self.version == DEFAULT_VERSION
    }

    /// Fills `buf` with the bytes starting at `offset`, clipped to the fixture
    /// length. Returns the number of bytes written.
    pub fn fill(&self, offset: u64, buf: &mut [u8]) -> usize {
        if offset >= self.len {
            return 0;
        }
        let n = (self.len - offset).min(buf.len() as u64) as usize;
        self.fill_unbounded(offset, &mut buf[..n]);
        n
    }

    /// Fills `buf` with the deterministic stream at `offset`, ignoring the
    /// fixture length (used for "extra bytes" faults).
    pub(crate) fn fill_unbounded(&self, offset: u64, buf: &mut [u8]) {
        if self.is_plain_text() {
            for (i, b) in buf.iter_mut().enumerate() {
                let o = offset + i as u64;
                *b = TINY_CONTENT.get(o as usize).copied().unwrap_or(b'\n');
            }
            return;
        }
        let mut pos = offset;
        let mut i = 0usize;
        while i < buf.len() {
            let block = pos / 8;
            let within = (pos % 8) as usize;
            let word =
                splitmix64(self.seed ^ block.wrapping_mul(0xD1B5_4A32_D192_ED03)).to_le_bytes();
            let take = (8 - within).min(buf.len() - i);
            buf[i..i + take].copy_from_slice(&word[within..within + take]);
            i += take;
            pos += take as u64;
        }
    }

    /// The byte at `offset`, or `None` past the end.
    pub fn byte_at(&self, offset: u64) -> Option<u8> {
        let mut b = [0u8; 1];
        (self.fill(offset, &mut b) == 1).then_some(b[0])
    }

    /// The bytes of `[start, start + len)`, clipped to the fixture.
    pub fn range(&self, start: u64, len: u64) -> Vec<u8> {
        let end = start.saturating_add(len).min(self.len);
        let n = end.saturating_sub(start) as usize;
        let mut v = vec![0u8; n];
        self.fill(start, &mut v);
        v
    }

    /// The whole content (avoid for the big fixtures).
    pub fn to_vec(&self) -> Vec<u8> {
        self.range(0, self.len)
    }

    /// Lower-case hex SHA-256 of the content. Computed once per (name, version)
    /// per process and cached.
    pub fn sha256_hex(&self) -> String {
        let key = (self.name.clone(), self.version);
        if let Some(h) = sha_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
            return h;
        }
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut off = 0u64;
        while off < self.len {
            let n = self.fill(off, &mut buf);
            hasher.update(&buf[..n]);
            off += n as u64;
        }
        let hex = to_hex(&hasher.finalize());
        if let Ok(mut cache) = sha_cache().lock() {
            cache.insert(key, hex.clone());
        }
        hex
    }

    /// Opaque tag part of the ETag (without quotes): 16 hex digits derived from
    /// the content seed, plus `-v<version>`. Cheap to compute (no hashing).
    pub fn etag_opaque(&self) -> String {
        format!("{:016x}-v{}", splitmix64(self.seed), self.version)
    }

    /// Strong ETag header value, e.g. `"0123456789abcdef-v1"`.
    pub fn etag(&self) -> String {
        format!("\"{}\"", self.etag_opaque())
    }

    /// Weak ETag header value, e.g. `W/"0123456789abcdef-v1"`.
    pub fn weak_etag(&self) -> String {
        format!("W/{}", self.etag())
    }

    /// `Last-Modified` header value (IMF-fixdate); changes with the version.
    pub fn last_modified(&self) -> String {
        crate::util::http_date(self.mtime_unix())
    }

    /// Modification time in Unix seconds (2015-10-21T07:28:00Z + version days).
    pub fn mtime_unix(&self) -> u64 {
        1_445_412_480 + u64::from(self.version.saturating_sub(1)) * 86_400
    }
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_is_hello_world() {
        let f = fixture("tiny.txt").unwrap();
        assert_eq!(f.to_vec(), TINY_CONTENT);
        assert_eq!(f.len, 13);
    }

    #[test]
    fn sha_stable_across_calls() {
        let a = fixture("small.bin").unwrap().sha256_hex();
        let b = fixture("small.bin").unwrap().sha256_hex();
        assert_eq!(a, b);
        let direct = to_hex(&Sha256::digest(fixture("small.bin").unwrap().to_vec()));
        assert_eq!(a, direct);
        assert_eq!(
            fixture("tiny.txt").unwrap().sha256_hex(),
            to_hex(&Sha256::digest(TINY_CONTENT))
        );
    }

    #[test]
    fn ranges_match_prefix_generation() {
        let f = fixture("small.bin").unwrap();
        let all = f.to_vec();
        for (s, l) in [
            (0u64, 1u64),
            (3, 17),
            (8, 8),
            (65_530, 100),
            (12_345, 4_097),
        ] {
            let r = f.range(s, l);
            let end = (s + l).min(f.len) as usize;
            assert_eq!(r, &all[s as usize..end]);
        }
        assert_eq!(f.byte_at(f.len), None);
    }

    #[test]
    fn version_changes_content_and_etag() {
        let a = fixture_version("small.bin", 1).unwrap();
        let b = fixture_version("small.bin", 2).unwrap();
        assert_ne!(a.range(0, 64), b.range(0, 64));
        assert_ne!(a.etag(), b.etag());
        assert_ne!(a.last_modified(), b.last_modified());
        assert!(fixture("nope.bin").is_none());
    }
}
