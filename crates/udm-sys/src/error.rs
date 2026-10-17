//! Typed OS-service errors.

use std::io;
use std::path::Path;

use udm_core::Message;

/// Errors returned by `udm-sys`.
#[derive(Debug)]
pub enum SysError {
    /// Not enough free space on the target volume (remaining bytes + safety margin).
    NotEnoughSpace {
        /// Volume (drive or mount point) that is short of space.
        volume: String,
        /// Bytes required, including `limits::DISK_SAFETY_MARGIN_BYTES`.
        needed: u64,
        /// Bytes available to the user.
        available: u64,
    },
    /// The disk (or quota) is full while writing.
    DiskFull {
        /// File being written.
        path: String,
    },
    /// Another write error (removed drive, permissions, ...).
    DiskWrite {
        /// File being written.
        path: String,
        /// OS error text.
        detail: String,
    },
    /// Any other I/O failure.
    Io {
        /// File or directory involved.
        path: String,
        /// I/O error kind.
        kind: io::ErrorKind,
        /// OS error text.
        detail: String,
    },
    /// The target already exists and must not be overwritten.
    AlreadyExists {
        /// Existing path.
        path: String,
    },
    /// A path longer than `limits::MAX_PATH_BYTES`.
    PathTooLong {
        /// Path (as text).
        path: String,
    },
    /// A file size above `limits::MAX_FILE_SIZE_BYTES`.
    FileTooLarge {
        /// Requested size.
        size: u64,
        /// Allowed maximum.
        max: u64,
    },
    /// The operation is not supported on this platform.
    Unsupported {
        /// Short operation name (`hibernate`, ...).
        what: &'static str,
    },
    /// Clipboard access failed.
    Clipboard {
        /// Backend error text.
        detail: String,
    },
    /// Showing a notification failed.
    Notify {
        /// Backend error text.
        detail: String,
    },
    /// A power action (shutdown, sleep, hibernate) failed.
    Power {
        /// Action name (`shutdown`, `sleep`, `hibernate`).
        action: &'static str,
        /// Error text or exit status.
        detail: String,
    },
}

impl SysError {
    pub(crate) fn io(path: &Path, e: &io::Error) -> Self {
        SysError::Io {
            path: path.display().to_string(),
            kind: e.kind(),
            detail: e.to_string(),
        }
    }

    /// Maps a write error on `path` to [`SysError::DiskFull`] or [`SysError::DiskWrite`]
    /// using [`crate::classify_write_error`].
    pub fn from_write_error(path: &Path, e: &io::Error) -> Self {
        match crate::classify_write_error(e) {
            crate::WriteFailure::DiskFull => SysError::DiskFull {
                path: path.display().to_string(),
            },
            crate::WriteFailure::Other => SysError::DiskWrite {
                path: path.display().to_string(),
                detail: e.to_string(),
            },
        }
    }

    /// Bytes missing for [`SysError::NotEnoughSpace`] (0 for other variants).
    pub fn shortfall(&self) -> u64 {
        match self {
            SysError::NotEnoughSpace {
                needed, available, ..
            } => needed.saturating_sub(*available),
            _ => 0,
        }
    }

    /// Localisable message (`error-sys-*`, `error-disk-full`, `error-disk-write`).
    pub fn to_message(&self) -> Message {
        match self {
            SysError::NotEnoughSpace {
                volume,
                needed,
                available,
            } => Message::new("error-sys-not-enough-space")
                .with_str("volume", volume.clone())
                .with_bytes("needed", *needed)
                .with_bytes("available", *available)
                .with_bytes("shortfall", self.shortfall()),
            SysError::DiskFull { path } => {
                Message::new("error-disk-full").with_str("path", path.clone())
            }
            SysError::DiskWrite { path, detail } => Message::new("error-disk-write")
                .with_str("path", path.clone())
                .with_str("detail", detail.clone()),
            SysError::Io { path, detail, .. } => Message::new("error-sys-io")
                .with_str("path", path.clone())
                .with_str("detail", detail.clone()),
            SysError::AlreadyExists { path } => {
                Message::new("error-sys-already-exists").with_str("path", path.clone())
            }
            SysError::PathTooLong { path } => Message::new("error-sys-path-too-long")
                .with_str("path", path.clone())
                .with_uint(
                    "max",
                    u64::try_from(udm_core::limits::MAX_PATH_BYTES).unwrap_or(u64::MAX),
                ),
            SysError::FileTooLarge { size, max } => Message::new("error-sys-file-too-large")
                .with_bytes("size", *size)
                .with_bytes("max", *max),
            SysError::Unsupported { what } => {
                Message::new("error-sys-unsupported").with_str("what", *what)
            }
            SysError::Clipboard { detail } => {
                Message::new("error-sys-clipboard").with_str("detail", detail.clone())
            }
            SysError::Notify { detail } => {
                Message::new("error-sys-notify").with_str("detail", detail.clone())
            }
            SysError::Power { action, detail } => Message::new("error-sys-power")
                .with_str("action", *action)
                .with_str("detail", detail.clone()),
        }
    }
}

impl std::fmt::Display for SysError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_message())
    }
}

impl std::error::Error for SysError {}

impl udm_core::message::ToMessage for SysError {
    fn to_message(&self) -> Message {
        SysError::to_message(self)
    }
}
