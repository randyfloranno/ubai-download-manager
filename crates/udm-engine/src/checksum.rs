//! File hashing for checksum verification (blocking; run on the blocking pool).

use std::io::Read;
use std::path::Path;

use udm_core::Message;
use udm_proto::{ChecksumAlgo, Hasher};

use crate::error::{EngineError, EngineResult};

const BUF: usize = 256 * 1024;

/// Hashes the whole file at `path`.
pub fn hash_file(path: &Path, algo: ChecksumAlgo) -> EngineResult<Vec<u8>> {
    let io_err = |e: std::io::Error| {
        EngineError::Msg(
            Message::new("error-engine-read-file")
                .with_str("path", path.display().to_string())
                .with_str("detail", e.kind().to_string()),
        )
    };
    let mut f = std::fs::File::open(udm_sys::long_path(path)).map_err(io_err)?;
    let mut h = Hasher::new(algo);
    let mut buf = vec![0u8; BUF];
    loop {
        let n = f.read(&mut buf).map_err(io_err)?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"abc").unwrap();
        let d = hash_file(&p, ChecksumAlgo::Sha256).unwrap();
        assert_eq!(
            udm_proto::to_hex(&d),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
