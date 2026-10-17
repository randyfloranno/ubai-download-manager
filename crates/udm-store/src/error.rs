//! Typed storage errors.

use std::io;
use std::path::{Path, PathBuf};

use udm_core::Message;

/// Errors returned by `udm-store`. Every variant names the file involved.
#[derive(Debug)]
pub enum StoreError {
    /// An I/O operation failed.
    Io {
        /// File or directory involved.
        path: PathBuf,
        /// I/O error kind.
        kind: io::ErrorKind,
        /// OS error text (never contains file contents).
        detail: String,
    },
    /// A file (or a record to be written) exceeds its size limit.
    TooLarge {
        /// File involved.
        path: PathBuf,
        /// Observed size in bytes (a lower bound when the file is still growing).
        size: u64,
        /// Allowed maximum in bytes.
        max: u64,
    },
    /// The file is torn, truncated, or its checksum does not match.
    Corrupt {
        /// File involved.
        path: PathBuf,
        /// Short machine-readable reason (`envelope`, `checksum`, `schema-zero`).
        reason: &'static str,
    },
    /// The file was written by a newer UDM with a schema we do not understand.
    SchemaUnsupported {
        /// File involved.
        path: PathBuf,
        /// Schema found in the file.
        found: u32,
        /// Highest schema this build supports.
        supported: u32,
    },
    /// The file holds a different kind of document than expected.
    KindMismatch {
        /// File involved.
        path: PathBuf,
        /// Kind found in the file (bounded copy).
        found: String,
        /// Kind expected by the caller.
        expected: String,
    },
    /// The envelope is intact but the payload (or a record) does not match the expected type,
    /// or a value could not be serialised.
    Parse {
        /// File involved.
        path: PathBuf,
        /// serde_json error text.
        detail: String,
    },
}

impl StoreError {
    pub(crate) fn io(path: &Path, e: &io::Error) -> Self {
        StoreError::Io {
            path: path.to_path_buf(),
            kind: e.kind(),
            detail: e.to_string(),
        }
    }

    pub(crate) fn parse(path: &Path, e: &serde_json::Error) -> Self {
        StoreError::Parse {
            path: path.to_path_buf(),
            detail: e.to_string(),
        }
    }

    /// Localisable message (`error-store-*`).
    pub fn to_message(&self) -> Message {
        match self {
            StoreError::Io { path, detail, .. } => Message::new("error-store-io")
                .with_str("path", path.display().to_string())
                .with_str("detail", detail.clone()),
            StoreError::TooLarge { path, size, max } => Message::new("error-store-too-large")
                .with_str("path", path.display().to_string())
                .with_bytes("size", *size)
                .with_bytes("max", *max),
            StoreError::Corrupt { path, reason } => Message::new("error-store-corrupt")
                .with_str("path", path.display().to_string())
                .with_str("reason", *reason),
            StoreError::SchemaUnsupported {
                path,
                found,
                supported,
            } => Message::new("error-store-schema-unsupported")
                .with_str("path", path.display().to_string())
                .with_uint("found", u64::from(*found))
                .with_uint("supported", u64::from(*supported)),
            StoreError::KindMismatch {
                path,
                found,
                expected,
            } => Message::new("error-store-kind-mismatch")
                .with_str("path", path.display().to_string())
                .with_str("found", found.clone())
                .with_str("expected", expected.clone()),
            StoreError::Parse { path, detail } => Message::new("error-store-parse")
                .with_str("path", path.display().to_string())
                .with_str("detail", detail.clone()),
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_message())
    }
}

impl std::error::Error for StoreError {}

impl udm_core::message::ToMessage for StoreError {
    fn to_message(&self) -> Message {
        StoreError::to_message(self)
    }
}
