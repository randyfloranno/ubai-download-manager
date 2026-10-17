//! Checksummed JSON envelope used by every state file except history.
//!
//! Format: `{"schema":N,"kind":"...","checksum":"<sha256 hex of payload>","payload":"<json string>"}`.
//! The payload is stored as a string so the checksum covers its exact bytes.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use udm_core::limits;

use crate::atomic::{atomic_write, read_bounded};
use crate::error::StoreError;
use crate::hash::sha256_full;

/// Longest `kind` string copied into an error (the file is hostile).
const MAX_KIND_ECHO_CHARS: usize = 64;

#[derive(Serialize)]
struct EnvelopeOut<'a> {
    schema: u32,
    kind: &'a str,
    checksum: String,
    payload: String,
}

#[derive(Deserialize)]
struct EnvelopeIn {
    schema: u32,
    kind: String,
    checksum: String,
    payload: String,
}

/// Serialises `payload` into a checksummed envelope and writes it atomically to `path`.
///
/// Refuses (with [`StoreError::TooLarge`]) to write a file larger than
/// `limits::MAX_STATE_FILE_BYTES`, since it could never be read back.
pub fn save_envelope<T: Serialize>(
    path: &Path,
    kind: &str,
    schema: u32,
    payload: &T,
) -> Result<(), StoreError> {
    let payload = serde_json::to_string(payload).map_err(|e| StoreError::parse(path, &e))?;
    let env = EnvelopeOut {
        schema,
        kind,
        checksum: sha256_full(payload.as_bytes()),
        payload,
    };
    let bytes = serde_json::to_vec(&env).map_err(|e| StoreError::parse(path, &e))?;
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if len > limits::MAX_STATE_FILE_BYTES {
        return Err(StoreError::TooLarge {
            path: path.to_path_buf(),
            size: len,
            max: limits::MAX_STATE_FILE_BYTES,
        });
    }
    atomic_write(path, &bytes)
}

/// Loads and verifies an envelope written by [`save_envelope`].
///
/// Returns `Ok(None)` if the file does not exist, otherwise the stored schema and payload.
/// The file is bounded by `limits::MAX_STATE_FILE_BYTES`; a torn file or checksum mismatch is
/// [`StoreError::Corrupt`], another kind is [`StoreError::KindMismatch`], a schema above
/// `max_schema` is [`StoreError::SchemaUnsupported`]. Nothing is ever silently used.
pub fn load_envelope<T: DeserializeOwned>(
    path: &Path,
    kind: &str,
    max_schema: u32,
) -> Result<Option<(u32, T)>, StoreError> {
    let Some(bytes) = read_bounded(path, limits::MAX_STATE_FILE_BYTES)? else {
        return Ok(None);
    };
    let env: EnvelopeIn = serde_json::from_slice(&bytes).map_err(|_| StoreError::Corrupt {
        path: path.to_path_buf(),
        reason: "envelope",
    })?;
    if sha256_full(env.payload.as_bytes()) != env.checksum.to_ascii_lowercase() {
        return Err(StoreError::Corrupt {
            path: path.to_path_buf(),
            reason: "checksum",
        });
    }
    if env.kind != kind {
        return Err(StoreError::KindMismatch {
            path: path.to_path_buf(),
            found: env.kind.chars().take(MAX_KIND_ECHO_CHARS).collect(),
            expected: kind.to_owned(),
        });
    }
    if env.schema == 0 {
        return Err(StoreError::Corrupt {
            path: path.to_path_buf(),
            reason: "schema-zero",
        });
    }
    if env.schema > max_schema {
        return Err(StoreError::SchemaUnsupported {
            path: path.to_path_buf(),
            found: env.schema,
            supported: max_schema,
        });
    }
    let value: T = serde_json::from_str(&env.payload).map_err(|e| StoreError::parse(path, &e))?;
    Ok(Some((env.schema, value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Resume {
        url: String,
        done: Vec<(u64, u64)>,
    }

    fn sample() -> Resume {
        Resume {
            url: "http://127.0.0.1/a.bin".into(),
            done: vec![(0, 100), (200, 300)],
        }
    }

    fn load(p: &Path, kind: &str, max: u32) -> Result<Option<(u32, Resume)>, StoreError> {
        load_envelope(p, kind, max)
    }

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("d-000001.json");
        save_envelope(&p, "resume", 1, &sample()).unwrap();
        let (schema, v) = load(&p, "resume", 1).unwrap().unwrap();
        assert_eq!(schema, 1);
        assert_eq!(v, sample());
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.starts_with(r#"{"schema":1,"kind":"resume","checksum":""#));
    }

    #[test]
    fn missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            load(&dir.path().join("x.json"), "resume", 1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn older_schema_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("q.json");
        save_envelope(&p, "queue", 1, &sample()).unwrap();
        let (schema, _) = load(&p, "queue", 3).unwrap().unwrap();
        assert_eq!(schema, 1);
    }

    #[test]
    fn hostile_envelope_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        save_envelope(&p, "resume", 1, &sample()).unwrap();
        let full = fs::read(&p).unwrap();
        for cut in [0, 1, full.len() / 2, full.len() - 1] {
            fs::write(&p, &full[..cut]).unwrap();
            assert!(
                matches!(
                    load(&p, "resume", 1),
                    Err(StoreError::Corrupt {
                        reason: "envelope",
                        ..
                    })
                ),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn hostile_envelope_checksum_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        save_envelope(&p, "resume", 1, &sample()).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        // Change one payload byte while keeping the JSON valid.
        let tampered = text.replace("a.bin", "b.bin");
        assert_ne!(tampered, text);
        fs::write(&p, tampered).unwrap();
        let err = load(&p, "resume", 1).unwrap_err();
        assert!(matches!(
            err,
            StoreError::Corrupt {
                reason: "checksum",
                ..
            }
        ));
        assert_eq!(err.to_message().key, "error-store-corrupt");
    }

    #[test]
    fn hostile_envelope_kind_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        save_envelope(&p, "settings", 1, &sample()).unwrap();
        let err = load(&p, "resume", 1).unwrap_err();
        assert!(
            matches!(&err, StoreError::KindMismatch { found, expected, .. }
            if found == "settings" && expected == "resume")
        );
        assert_eq!(err.to_message().key, "error-store-kind-mismatch");
    }

    #[test]
    fn hostile_envelope_future_schema() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        save_envelope(&p, "resume", 7, &sample()).unwrap();
        let err = load(&p, "resume", 2).unwrap_err();
        assert!(matches!(
            err,
            StoreError::SchemaUnsupported {
                found: 7,
                supported: 2,
                ..
            }
        ));
        assert_eq!(err.to_message().key, "error-store-schema-unsupported");
    }

    #[test]
    fn hostile_envelope_schema_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        save_envelope(&p, "resume", 0, &sample()).unwrap();
        assert!(matches!(
            load(&p, "resume", 1),
            Err(StoreError::Corrupt {
                reason: "schema-zero",
                ..
            })
        ));
    }

    #[test]
    fn hostile_envelope_payload_wrong_type() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        save_envelope(&p, "resume", 1, &vec![1u32, 2, 3]).unwrap();
        let err = load(&p, "resume", 1).unwrap_err();
        assert!(matches!(err, StoreError::Parse { .. }));
        assert_eq!(err.to_message().key, "error-store-parse");
    }

    #[test]
    fn hostile_envelope_garbage_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        fs::write(&p, [0xffu8, 0xfe, 0x00, 0x7b, 0x22]).unwrap();
        assert!(matches!(
            load(&p, "resume", 1),
            Err(StoreError::Corrupt { .. })
        ));
    }

    #[test]
    fn hostile_state_file_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        let f = fs::File::create(&p).unwrap();
        // Sparse on most file systems: cheap to create.
        f.set_len(limits::MAX_STATE_FILE_BYTES + 1).unwrap();
        drop(f);
        let err = load(&p, "resume", 1).unwrap_err();
        assert!(matches!(
            err,
            StoreError::TooLarge { size, max, .. }
                if size == limits::MAX_STATE_FILE_BYTES + 1 && max == limits::MAX_STATE_FILE_BYTES
        ));
        assert_eq!(err.to_message().key, "error-store-too-large");
    }

    #[test]
    fn hostile_save_refuses_payload_over_state_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        let big = "x".repeat(usize::try_from(limits::MAX_STATE_FILE_BYTES).unwrap());
        let err = save_envelope(&p, "resume", 1, &big).unwrap_err();
        assert!(matches!(err, StoreError::TooLarge { .. }));
        assert!(!p.exists());
    }
}
