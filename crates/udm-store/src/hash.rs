//! SHA-256 hex helpers.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Lower-case hex of the SHA-256 of `data`, truncated to `max_bytes` digest bytes.
pub(crate) fn sha256_hex(data: &[u8], max_bytes: usize) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in digest.iter().take(max_bytes) {
        // Writing to a String cannot fail.
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Full SHA-256 digest as 64 hex characters.
pub(crate) fn sha256_full(data: &[u8]) -> String {
    sha256_hex(data, 32)
}

/// First 16 hex characters (8 digest bytes) of the SHA-256.
pub(crate) fn sha256_prefix16(data: &[u8]) -> String {
    sha256_hex(data, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        assert_eq!(
            sha256_full(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_prefix16(b"abc"), "ba7816bf8f01cfea");
    }
}
