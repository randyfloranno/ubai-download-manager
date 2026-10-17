//! Atomic file replacement and bounded reads.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::error::StoreError;

/// Path of the temporary sibling used by [`atomic_write`]: `<name>.tmp` in the same directory.
pub(crate) fn tmp_path(path: &Path) -> Result<PathBuf, StoreError> {
    let name = path.file_name().ok_or_else(|| StoreError::Io {
        path: path.to_path_buf(),
        kind: io::ErrorKind::InvalidInput,
        detail: "path has no file name".to_owned(),
    })?;
    let mut tmp_name = OsString::from(name);
    tmp_name.push(".tmp");
    Ok(path.with_file_name(tmp_name))
}

/// Directory containing `path` (`.` for a bare file name).
pub(crate) fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Atomically replaces `path` with `bytes`.
///
/// The data is written to `<name>.tmp` in the same directory, `fsync`ed, then renamed over the
/// target; on Unix the parent directory is `fsync`ed as well. A crash at any point leaves either
/// the old or the new file in place, never a torn one under the final name.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = tmp_path(path)?;
    let result = write_tmp(&tmp, bytes).and_then(|()| {
        fs::rename(&tmp, path).map_err(|e| StoreError::io(path, &e))?;
        sync_dir(parent_dir(path)).map_err(|e| StoreError::io(parent_dir(path), &e))
    });
    if result.is_err() {
        // Best effort: never leave a stale temp file behind. The error that matters is `result`.
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_tmp(tmp: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(tmp)
        .map_err(|e| StoreError::io(tmp, &e))?;
    f.write_all(bytes).map_err(|e| StoreError::io(tmp, &e))?;
    f.flush().map_err(|e| StoreError::io(tmp, &e))?;
    f.sync_all().map_err(|e| StoreError::io(tmp, &e))
}

/// `fsync`s a directory so a rename inside it is durable (Unix only; a no-op elsewhere).
#[cfg(unix)]
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    match File::open(dir).and_then(|d| d.sync_all()) {
        Ok(()) => Ok(()),
        // Some file systems do not support directory fsync; that is not a failure of ours.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// `fsync`s a directory so a rename inside it is durable (Unix only; a no-op elsewhere).
#[cfg(not(unix))]
pub(crate) fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Reads a whole file, refusing anything larger than `max` bytes. `Ok(None)` if it does not exist.
pub(crate) fn read_bounded(path: &Path, max: u64) -> Result<Option<Vec<u8>>, StoreError> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError::io(path, &e)),
    };
    let size = f.metadata().map_err(|e| StoreError::io(path, &e))?.len();
    if size > max {
        return Err(StoreError::TooLarge {
            path: path.to_path_buf(),
            size,
            max,
        });
    }
    // The file may grow between `metadata` and `read`; read at most `max + 1` bytes to notice.
    let mut buf = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    f.take(max.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| StoreError::io(path, &e))?;
    let read = u64::try_from(buf.len()).unwrap_or(u64::MAX);
    if read > max {
        return Err(StoreError::TooLarge {
            path: path.to_path_buf(),
            size: read,
            max,
        });
    }
    Ok(Some(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert!(!dir.path().join("settings.json.tmp").exists());
    }

    #[test]
    fn atomic_write_overwrites_stale_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("queue.json");
        fs::write(
            dir.path().join("queue.json.tmp"),
            b"stale garbage that is longer",
        )
        .unwrap();
        atomic_write(&p, b"fresh").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"fresh");
    }

    #[test]
    fn atomic_write_missing_dir_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope").join("x.json");
        let err = atomic_write(&p, b"x").unwrap_err();
        assert!(matches!(err, StoreError::Io { .. }));
        assert_eq!(err.to_message().key, "error-store-io");
    }

    #[test]
    fn read_bounded_missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_bounded(&dir.path().join("x"), 10).unwrap().is_none());
    }

    #[test]
    fn hostile_read_bounded_rejects_oversize() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x");
        fs::write(&p, [0u8; 11]).unwrap();
        assert!(matches!(
            read_bounded(&p, 10),
            Err(StoreError::TooLarge {
                size: 11,
                max: 10,
                ..
            })
        ));
        assert_eq!(read_bounded(&p, 11).unwrap().unwrap().len(), 11);
    }
}
