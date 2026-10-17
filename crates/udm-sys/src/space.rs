//! Free-space checks (AGENTS.md §4.3).

use std::path::{Path, PathBuf};

use udm_core::limits;

use crate::error::SysError;

/// Nearest ancestor of `path` (including itself) that exists, made absolute.
fn existing_ancestor(path: &Path) -> Result<PathBuf, SysError> {
    let abs = std::path::absolute(path).map_err(|e| SysError::io(path, &e))?;
    for anc in abs.ancestors() {
        if anc.exists() {
            return Ok(anc.to_path_buf());
        }
    }
    Err(SysError::io(
        path,
        &std::io::Error::from(std::io::ErrorKind::NotFound),
    ))
}

/// Human-readable volume name for `existing` (an existing absolute path).
fn volume_name(existing: &Path) -> String {
    #[cfg(windows)]
    {
        match existing.components().next() {
            Some(std::path::Component::Prefix(p)) => {
                let s = p.as_os_str().to_string_lossy();
                s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()
            }
            _ => existing.display().to_string(),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        // Walk up while we stay on the same device: the last such directory is the mount point.
        let Ok(dev) = std::fs::metadata(existing).map(|m| m.dev()) else {
            return existing.display().to_string();
        };
        let mut mount = existing;
        for anc in existing.ancestors().skip(1) {
            match std::fs::metadata(anc) {
                Ok(m) if m.dev() == dev => mount = anc,
                _ => break,
            }
        }
        mount.display().to_string()
    }
    #[cfg(not(any(unix, windows)))]
    {
        existing.display().to_string()
    }
}

/// Bytes available to the current user on the volume holding `path`.
///
/// `path` need not exist yet: the nearest existing ancestor is queried.
pub fn available_space(path: &Path) -> Result<u64, SysError> {
    let anc = existing_ancestor(path)?;
    fs4::available_space(&anc).map_err(|e| SysError::io(&anc, &e))
}

/// Checks that the volume holding `path` has `remaining` bytes plus
/// `limits::DISK_SAFETY_MARGIN_BYTES` available, else [`SysError::NotEnoughSpace`].
pub fn ensure_space(path: &Path, remaining: u64) -> Result<(), SysError> {
    let anc = existing_ancestor(path)?;
    let available = fs4::available_space(&anc).map_err(|e| SysError::io(&anc, &e))?;
    let needed = remaining.saturating_add(limits::DISK_SAFETY_MARGIN_BYTES);
    if available < needed {
        return Err(SysError::NotEnoughSpace {
            volume: volume_name(&anc),
            needed,
            available,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use udm_core::MsgArg;

    #[test]
    fn available_space_of_missing_child_uses_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let a = available_space(dir.path()).unwrap();
        let b = available_space(&dir.path().join("not").join("yet").join("f.bin")).unwrap();
        assert!(a > 0 && b > 0);
    }

    #[test]
    fn ensure_space_small_request_ok() {
        let dir = tempfile::tempdir().unwrap();
        let avail = available_space(dir.path()).unwrap();
        if avail > limits::DISK_SAFETY_MARGIN_BYTES + 1024 * 1024 {
            ensure_space(&dir.path().join("f.bin"), 1024).unwrap();
        }
    }

    #[test]
    fn hostile_ensure_space_requires_safety_margin() {
        let dir = tempfile::tempdir().unwrap();
        let avail = available_space(dir.path()).unwrap();
        // Exactly the available bytes is not enough: the margin must come on top.
        let err = ensure_space(dir.path(), avail).unwrap_err();
        match &err {
            SysError::NotEnoughSpace {
                volume,
                needed,
                available,
            } => {
                assert!(!volume.is_empty());
                assert_eq!(
                    *needed,
                    avail.saturating_add(limits::DISK_SAFETY_MARGIN_BYTES)
                );
                assert!(err.shortfall() > 0);
                assert!(*available < *needed);
            }
            other => panic!("unexpected {other:?}"),
        }
        let m = err.to_message();
        assert_eq!(m.key, "error-sys-not-enough-space");
        for k in ["volume", "needed", "available", "shortfall"] {
            assert!(m.arg(k).is_some(), "{k}");
        }
        assert!(matches!(m.arg("shortfall"), Some(MsgArg::Bytes(_))));
    }

    #[test]
    fn hostile_ensure_space_huge_request_saturates() {
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_space(dir.path(), u64::MAX).unwrap_err();
        assert!(matches!(
            err,
            SysError::NotEnoughSpace {
                needed: u64::MAX,
                ..
            }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn volume_name_is_drive() {
        let dir = tempfile::tempdir().unwrap();
        let v = volume_name(&std::path::absolute(dir.path()).unwrap());
        assert!(v.ends_with(':') || v.starts_with(r"\\"), "{v}");
    }
}
