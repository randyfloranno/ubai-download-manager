//! Positional file writer and the `.udmpart` part file.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fs4::FileExt as _;
use udm_core::limits;

use crate::error::SysError;
use crate::path::long_path;

/// A file that segments write into at absolute offsets, from several threads at once.
pub trait FileWriter: Send + Sync {
    /// Writes all of `buf` at `offset` (does not move any shared cursor).
    fn write_at(&self, offset: u64, buf: &[u8]) -> io::Result<()>;
    /// Flushes written data to the device (`fdatasync`).
    fn sync_data(&self) -> io::Result<()>;
    /// Current file length in bytes.
    fn len(&self) -> io::Result<u64>;
    /// `true` if the file is empty.
    fn is_empty(&self) -> io::Result<bool> {
        self.len().map(|l| l == 0)
    }
}

impl<W: FileWriter + ?Sized> FileWriter for Arc<W> {
    fn write_at(&self, offset: u64, buf: &[u8]) -> io::Result<()> {
        (**self).write_at(offset, buf)
    }
    fn sync_data(&self) -> io::Result<()> {
        (**self).sync_data()
    }
    fn len(&self) -> io::Result<u64> {
        (**self).len()
    }
}

impl<W: FileWriter + ?Sized> FileWriter for Box<W> {
    fn write_at(&self, offset: u64, buf: &[u8]) -> io::Result<()> {
        (**self).write_at(offset, buf)
    }
    fn sync_data(&self) -> io::Result<()> {
        (**self).sync_data()
    }
    fn len(&self) -> io::Result<u64> {
        (**self).len()
    }
}

/// How a write error should be reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFailure {
    /// Disk or quota full (`error-disk-full`): pause, keep state.
    DiskFull,
    /// Anything else (`error-disk-write`).
    Other,
}

/// Raw OS error codes that mean "no space left" on this platform.
#[cfg(windows)]
const DISK_FULL_CODES: &[i32] = &[
    112, // ERROR_DISK_FULL
    39,  // ERROR_HANDLE_DISK_FULL
];
#[cfg(target_os = "linux")]
const DISK_FULL_CODES: &[i32] = &[
    28,  // ENOSPC
    122, // EDQUOT
];
#[cfg(target_os = "macos")]
const DISK_FULL_CODES: &[i32] = &[
    28, // ENOSPC
    69, // EDQUOT
];
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
const DISK_FULL_CODES: &[i32] = &[28]; // ENOSPC
#[cfg(not(any(unix, windows)))]
const DISK_FULL_CODES: &[i32] = &[];

/// Classifies a write error: disk/quota full versus anything else.
pub fn classify_write_error(e: &io::Error) -> WriteFailure {
    if matches!(
        e.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    ) {
        return WriteFailure::DiskFull;
    }
    match e.raw_os_error() {
        Some(code) if DISK_FULL_CODES.contains(&code) => WriteFailure::DiskFull,
        _ => WriteFailure::Other,
    }
}

/// The `.udmpart` data file of a download. Opened read/write, never truncated.
#[derive(Debug)]
pub struct PartFile {
    file: File,
    path: PathBuf,
}

impl PartFile {
    /// Opens (creating if missing) the part file at `path`. Existing data is never truncated.
    ///
    /// With a known `size`, the file is extended to `size`; if `preallocate` is set, disk space
    /// is really allocated first (where the OS supports it), so later writes cannot hit a full
    /// disk. Pre-allocation failure for lack of space is [`SysError::DiskFull`].
    pub fn open(path: &Path, size: Option<u64>, preallocate: bool) -> Result<Self, SysError> {
        if path.as_os_str().len() > limits::MAX_PATH_BYTES {
            return Err(SysError::PathTooLong {
                path: path.display().to_string(),
            });
        }
        if let Some(size) = size
            && size > limits::MAX_FILE_SIZE_BYTES
        {
            return Err(SysError::FileTooLarge {
                size,
                max: limits::MAX_FILE_SIZE_BYTES,
            });
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(long_path(path))
            .map_err(|e| SysError::io(path, &e))?;
        let part = Self {
            file,
            path: path.to_path_buf(),
        };
        if let Some(size) = size {
            part.extend_to(size, preallocate)?;
        }
        Ok(part)
    }

    fn extend_to(&self, size: u64, preallocate: bool) -> Result<(), SysError> {
        let current = self.len().map_err(|e| SysError::io(&self.path, &e))?;
        if current >= size {
            return Ok(());
        }
        if preallocate {
            match self.file.allocate(size) {
                Ok(()) => {}
                Err(e) if classify_write_error(&e) == WriteFailure::DiskFull => {
                    return Err(SysError::from_write_error(&self.path, &e));
                }
                // Not supported by this file system: fall back to a plain (sparse) extension.
                Err(e) if e.kind() == io::ErrorKind::Unsupported => {}
                Err(e) => return Err(SysError::from_write_error(&self.path, &e)),
            }
        }
        // `allocate` may leave the logical size unchanged (Windows); make it exact.
        if self.len().map_err(|e| SysError::io(&self.path, &e))? < size {
            self.file
                .set_len(size)
                .map_err(|e| SysError::from_write_error(&self.path, &e))?;
        }
        Ok(())
    }

    /// Path the part file was opened with.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl FileWriter for PartFile {
    fn write_at(&self, offset: u64, buf: &[u8]) -> io::Result<()> {
        // Never write past the largest supported file (also keeps offsets far from i64::MAX,
        // which the OS would otherwise interpret as negative).
        let len = u64::try_from(buf.len()).map_err(|_| offset_overflow())?;
        match offset.checked_add(len) {
            Some(end) if end <= limits::MAX_FILE_SIZE_BYTES => {}
            _ => return Err(offset_overflow()),
        }
        positional_write_all(&self.file, offset, buf)
    }

    fn sync_data(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    fn len(&self) -> io::Result<u64> {
        self.file.metadata().map(|m| m.len())
    }
}

fn offset_overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "write offset overflow")
}

#[cfg(windows)]
fn positional_write_all(file: &File, offset: u64, buf: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt as _;
    let mut done: usize = 0;
    while let Some(rest) = buf.get(done..) {
        if rest.is_empty() {
            break;
        }
        let at = offset
            .checked_add(u64::try_from(done).map_err(|_| offset_overflow())?)
            .ok_or_else(offset_overflow)?;
        match file.seek_write(rest, at) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(n) => done = done.checked_add(n).ok_or_else(offset_overflow)?,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn positional_write_all(file: &File, offset: u64, buf: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt as _;
    file.write_all_at(buf, offset)
}

#[cfg(not(any(unix, windows)))]
fn positional_write_all(_file: &File, _offset: u64, _buf: &[u8]) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn part_file_positional_writes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.bin.udmpart");
        let f = PartFile::open(&p, Some(10), false).unwrap();
        assert_eq!(f.len().unwrap(), 10);
        f.write_at(5, b"world").unwrap();
        f.write_at(0, b"hello").unwrap();
        f.sync_data().unwrap();
        drop(f);
        assert_eq!(fs::read(&p).unwrap(), b"helloworld");
    }

    #[test]
    fn part_file_preallocates_and_never_truncates() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.bin.udmpart");
        let size = 8 * 1024 * 1024;
        let f = PartFile::open(&p, Some(size), true).unwrap();
        assert_eq!(f.len().unwrap(), size);
        f.write_at(size - 3, b"end").unwrap();
        drop(f);
        // Reopen with a smaller size (or unknown size): data stays.
        let f = PartFile::open(&p, Some(1), true).unwrap();
        assert_eq!(f.len().unwrap(), size);
        drop(f);
        let f = PartFile::open(&p, None, false).unwrap();
        assert_eq!(f.len().unwrap(), size);
        drop(f);
        let data = fs::read(&p).unwrap();
        assert_eq!(&data[data.len() - 3..], b"end");
    }

    #[test]
    fn hostile_part_file_size_over_max_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.udmpart");
        let err = PartFile::open(&p, Some(limits::MAX_FILE_SIZE_BYTES + 1), true).unwrap_err();
        assert!(matches!(err, SysError::FileTooLarge { .. }));
        assert_eq!(err.to_message().key, "error-sys-file-too-large");
    }

    #[test]
    fn hostile_part_file_path_over_max_path_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x".repeat(limits::MAX_PATH_BYTES + 1));
        let err = PartFile::open(&p, None, false).unwrap_err();
        assert!(matches!(err, SysError::PathTooLong { .. }));
        assert_eq!(err.to_message().key, "error-sys-path-too-long");
    }

    #[test]
    fn hostile_write_offset_overflow_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let f = PartFile::open(&dir.path().join("o.udmpart"), None, false).unwrap();
        assert!(f.write_at(u64::MAX - 1, b"abc").is_err());
        assert!(f.write_at(limits::MAX_FILE_SIZE_BYTES - 2, b"abc").is_err());
        assert_eq!(f.len().unwrap(), 0);
    }

    #[test]
    fn classify_known_errors() {
        assert_eq!(
            classify_write_error(&io::Error::from(io::ErrorKind::StorageFull)),
            WriteFailure::DiskFull
        );
        assert_eq!(
            classify_write_error(&io::Error::from(io::ErrorKind::PermissionDenied)),
            WriteFailure::Other
        );
        #[cfg(windows)]
        {
            assert_eq!(
                classify_write_error(&io::Error::from_raw_os_error(112)),
                WriteFailure::DiskFull
            );
            assert_eq!(
                classify_write_error(&io::Error::from_raw_os_error(39)),
                WriteFailure::DiskFull
            );
            assert_eq!(
                classify_write_error(&io::Error::from_raw_os_error(5)),
                WriteFailure::Other
            );
        }
        #[cfg(unix)]
        assert_eq!(
            classify_write_error(&io::Error::from_raw_os_error(28)),
            WriteFailure::DiskFull
        );
    }

    #[test]
    fn write_error_messages() {
        let p = Path::new("x.udmpart");
        let full = SysError::from_write_error(p, &io::Error::from(io::ErrorKind::StorageFull));
        assert_eq!(full.to_message().key, "error-disk-full");
        let other = SysError::from_write_error(p, &io::Error::from(io::ErrorKind::Other));
        assert_eq!(other.to_message().key, "error-disk-write");
    }
}
