//! Seams to the operating system, so tests can inject faults (disk full,
//! write errors, no free space) and never shut the test machine down.

use std::path::Path;
use std::sync::Arc;

use udm_sys::{FileWriter, PartFile, PowerAction, SysError};

/// OS services used by the engine.
pub trait Platform: Send + Sync + 'static {
    /// Opens (creating if needed) the part file without truncating it.
    fn open_part(
        &self,
        path: &Path,
        size: Option<u64>,
        preallocate: bool,
    ) -> Result<Arc<dyn FileWriter>, SysError>;

    /// Fails when the volume of `dir` lacks `remaining` bytes plus the margin.
    fn ensure_space(&self, dir: &Path, remaining: u64) -> Result<(), SysError>;

    /// Renames without overwriting an existing file.
    fn rename_no_overwrite(&self, from: &Path, to: &Path) -> Result<(), SysError>;

    /// Shutdown / sleep / hibernate.
    fn power(&self, action: PowerAction) -> Result<(), SysError>;

    /// Current clipboard text.
    fn clipboard_text(&self) -> Result<Option<String>, SysError>;

    /// Opens a file with its default program or (`reveal`) shows it in the file manager.
    fn open_path(&self, path: &Path, reveal: bool) -> Result<(), SysError>;
}

/// The real platform, backed by `udm-sys`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPlatform;

impl Platform for SystemPlatform {
    fn open_part(
        &self,
        path: &Path,
        size: Option<u64>,
        preallocate: bool,
    ) -> Result<Arc<dyn FileWriter>, SysError> {
        let f = PartFile::open(path, size, preallocate)?;
        Ok(Arc::new(f))
    }

    fn ensure_space(&self, dir: &Path, remaining: u64) -> Result<(), SysError> {
        udm_sys::ensure_space(dir, remaining)
    }

    fn rename_no_overwrite(&self, from: &Path, to: &Path) -> Result<(), SysError> {
        udm_sys::rename_no_overwrite(from, to)
    }

    fn power(&self, action: PowerAction) -> Result<(), SysError> {
        udm_sys::perform_power_action(action)
    }

    fn clipboard_text(&self) -> Result<Option<String>, SysError> {
        udm_sys::clipboard_text()
    }

    fn open_path(&self, path: &Path, reveal: bool) -> Result<(), SysError> {
        udm_sys::open_path(path, reveal)
    }
}
