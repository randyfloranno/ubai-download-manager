//! Path helpers: Windows long paths and no-overwrite rename.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::SysError;

/// Paths at least this long (in UTF-16 units) get the `\\?\` prefix on Windows. The legacy
/// limit is 260 including the drive and terminator; 240 leaves room for `.udmpart` and
/// `.tmp` suffixes added later.
#[cfg(windows)]
const WINDOWS_LONG_PATH_THRESHOLD: usize = 240;

/// Returns a path the OS can open even when it exceeds the legacy Windows `MAX_PATH`.
///
/// On Windows, an absolute path longer than 240 UTF-16 units is lexically normalised (`.` and
/// `..` resolved, `/` turned into `\`) and prefixed with `\\?\` (`\\?\UNC\` for UNC shares).
/// Relative, short and already verbatim paths are returned unchanged. On other platforms the
/// path is returned unchanged.
pub fn long_path(p: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        windows_long_path(p)
    }
    #[cfg(not(windows))]
    {
        p.to_path_buf()
    }
}

#[cfg(windows)]
fn windows_long_path(p: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::{Component, Prefix};

    if p.as_os_str().encode_wide().count() <= WINDOWS_LONG_PATH_THRESHOLD {
        return p.to_path_buf();
    }
    let mut comps = p.components();
    let Some(Component::Prefix(prefix)) = comps.next() else {
        return p.to_path_buf();
    };
    let mut out = match prefix.kind() {
        Prefix::Disk(_) => {
            let mut s = OsString::from(r"\\?\");
            s.push(prefix.as_os_str());
            s
        }
        Prefix::UNC(server, share) => {
            let mut s = OsString::from(r"\\?\UNC\");
            s.push(server);
            s.push(r"\");
            s.push(share);
            s
        }
        // Verbatim or device paths are passed through untouched.
        _ => return p.to_path_buf(),
    };
    if !matches!(comps.next(), Some(Component::RootDir)) {
        // Drive-relative (`C:foo`): cannot be made verbatim without the per-drive cwd.
        return p.to_path_buf();
    }
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for c in comps {
        match c {
            Component::Normal(n) => parts.push(n),
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    for part in parts {
        out.push(r"\");
        out.push(part);
    }
    if out.encode_wide().last() == Some(u16::from(b':')) {
        // Bare `\\?\C:` must keep its root separator.
        out.push(r"\");
    }
    PathBuf::from(out)
}

/// Renames `from` to `to`, failing with [`SysError::AlreadyExists`] if `to` exists.
///
/// Uses a hard link + unlink so the check and the rename are one atomic step where the file
/// system supports hard links; elsewhere (FAT/exFAT, some network shares) it falls back to an
/// existence check followed by a rename.
pub fn rename_no_overwrite(from: &Path, to: &Path) -> Result<(), SysError> {
    let from_l = long_path(from);
    let to_l = long_path(to);
    match fs::hard_link(&from_l, &to_l) {
        Ok(()) => {
            if let Err(e) = fs::remove_file(&from_l) {
                // Undo the link so we never leave two names; report the original problem.
                let _ = fs::remove_file(&to_l);
                return Err(SysError::io(from, &e));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(SysError::AlreadyExists {
                path: to.display().to_string(),
            });
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(SysError::io(from, &e)),
        Err(_) => {
            // Hard links unsupported here: check, then rename.
            if fs::symlink_metadata(&to_l).is_ok() {
                return Err(SysError::AlreadyExists {
                    path: to.display().to_string(),
                });
            }
            fs::rename(&from_l, &to_l).map_err(|e| SysError::io(to, &e))?;
        }
    }
    sync_parent(to)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<(), SysError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    match fs::File::open(dir).and_then(|d| d.sync_all()) {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(SysError::io(dir, &e)),
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<(), SysError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_no_overwrite_moves_and_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin.udmpart");
        let b = dir.path().join("a.bin");
        fs::write(&a, b"data").unwrap();
        rename_no_overwrite(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read(&b).unwrap(), b"data");

        fs::write(&a, b"new").unwrap();
        let err = rename_no_overwrite(&a, &b).unwrap_err();
        assert!(matches!(err, SysError::AlreadyExists { .. }));
        assert_eq!(err.to_message().key, "error-sys-already-exists");
        assert_eq!(fs::read(&b).unwrap(), b"data");
        assert_eq!(fs::read(&a).unwrap(), b"new");
    }

    #[test]
    fn rename_missing_source_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = rename_no_overwrite(&dir.path().join("x"), &dir.path().join("y")).unwrap_err();
        assert!(matches!(err, SysError::Io { .. }));
    }

    #[test]
    fn short_paths_unchanged() {
        let p = Path::new("relative/dir/file.bin");
        assert_eq!(long_path(p), p);
    }

    #[cfg(windows)]
    #[test]
    fn windows_long_paths_get_verbatim_prefix() {
        let long = "a".repeat(250);
        let p = PathBuf::from(format!(r"C:\dl\{long}\.\x\..\f.bin"));
        assert_eq!(
            long_path(&p),
            PathBuf::from(format!(r"\\?\C:\dl\{long}\f.bin"))
        );
        let unc = PathBuf::from(format!(r"\\server\share\{long}/f.bin"));
        assert_eq!(
            long_path(&unc),
            PathBuf::from(format!(r"\\?\UNC\server\share\{long}\f.bin"))
        );
        let verbatim = PathBuf::from(format!(r"\\?\C:\{long}"));
        assert_eq!(long_path(&verbatim), verbatim);
        let rel = PathBuf::from(long.clone());
        assert_eq!(long_path(&rel), rel);
        assert_eq!(long_path(Path::new(r"C:\short")), Path::new(r"C:\short"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_long_path_is_creatable() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("d".repeat(120)).join("e".repeat(120));
        let target = deep.join("f".repeat(60));
        fs::create_dir_all(long_path(&deep)).unwrap();
        fs::write(long_path(&target), b"ok").unwrap();
        assert_eq!(fs::read(long_path(&target)).unwrap(), b"ok");
    }
}
