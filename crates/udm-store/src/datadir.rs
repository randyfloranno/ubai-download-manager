//! Data directory layout (architecture.md §4).

use std::fs;
use std::path::{Path, PathBuf};

use udm_core::{DownloadId, limits};

use crate::error::StoreError;

/// The UDM data directory: `settings.json`, `queue.json`, `history.jsonl`, `state/<id>.json`,
/// `ipc.json`, `logs/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    /// Wraps a data directory root. Nothing is created until [`DataDir::ensure`].
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Per-user OS data directory, or `.udm` when the OS gives none:
    /// `%APPDATA%\UDM\data` (Windows), `~/Library/Application Support/UDM`
    /// (macOS), `$XDG_DATA_HOME/udm` or `~/.local/share/udm` (other Unix).
    pub fn os_default() -> PathBuf {
        os_data_root().unwrap_or_else(|| PathBuf::from(".udm"))
    }

    /// Creates the root, `state/` and `logs/` directories if missing.
    pub fn ensure(&self) -> Result<(), StoreError> {
        for dir in [self.root.clone(), self.state_dir(), self.logs_dir()] {
            fs::create_dir_all(&dir).map_err(|e| StoreError::io(&dir, &e))?;
        }
        Ok(())
    }

    /// Root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory holding resume state files.
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// Resume state file of one download: `state/<id>.json`.
    pub fn state_file(&self, id: DownloadId) -> PathBuf {
        self.state_dir().join(format!("{id}.json"))
    }

    /// Queue file.
    pub fn queue_file(&self) -> PathBuf {
        self.root.join("queue.json")
    }

    /// Settings file.
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// History log (JSON Lines).
    pub fn history_file(&self) -> PathBuf {
        self.root.join("history.jsonl")
    }

    /// IPC discovery file (control port of the running instance).
    pub fn ipc_file(&self) -> PathBuf {
        self.root.join("ipc.json")
    }

    /// Log directory.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Ids of all `state/<id>.json` files, sorted ascending.
    ///
    /// Files whose name is not a valid id (temp files, foreign files) are ignored. At most
    /// `limits::MAX_QUEUE_SIZE` ids are returned (the lowest ones). A missing state directory
    /// yields an empty list.
    pub fn list_state_ids(&self) -> Result<Vec<DownloadId>, StoreError> {
        let dir = self.state_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StoreError::io(&dir, &e)),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| StoreError::io(&dir, &e))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            if let Ok(id) = stem.parse::<DownloadId>() {
                // Canonical names only: `d-1.json` and `d-000001.json` must not both count.
                if id.to_string() == stem {
                    ids.push(id);
                }
            }
        }
        ids.sort_unstable();
        ids.truncate(limits::MAX_QUEUE_SIZE);
        Ok(ids)
    }
}

/// An absolute path from a non-empty environment variable.
fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn os_data_root() -> Option<PathBuf> {
    if cfg!(windows) {
        env_dir("APPDATA").map(|a| a.join("UDM").join("data"))
    } else if cfg!(target_os = "macos") {
        std::env::home_dir()
            .filter(|h| h.is_absolute())
            .map(|h| h.join("Library/Application Support/UDM"))
    } else {
        env_dir("XDG_DATA_HOME")
            .or_else(|| {
                std::env::home_dir()
                    .filter(|h| h.is_absolute())
                    .map(|h| h.join(".local/share"))
            })
            .map(|d| d.join("udm"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_default_is_absolute_or_fallback() {
        let d = DataDir::os_default();
        assert!(d.is_absolute() || d == Path::new(".udm"), "{}", d.display());
    }

    #[test]
    fn layout_and_listing() {
        let tmp = tempfile::tempdir().unwrap();
        let dd = DataDir::new(tmp.path().join("data"));
        assert!(dd.list_state_ids().unwrap().is_empty());
        dd.ensure().unwrap();
        assert!(dd.state_dir().is_dir());
        assert!(dd.logs_dir().is_dir());
        assert_eq!(
            dd.state_file(DownloadId::new(7)),
            dd.root().join("state").join("d-000007.json")
        );
        assert_eq!(dd.queue_file(), dd.root().join("queue.json"));
        assert_eq!(dd.settings_file(), dd.root().join("settings.json"));
        assert_eq!(dd.history_file(), dd.root().join("history.jsonl"));
        assert_eq!(dd.ipc_file(), dd.root().join("ipc.json"));
        for id in [3u64, 1, 2] {
            fs::write(dd.state_file(DownloadId::new(id)), b"{}").unwrap();
        }
        fs::write(dd.state_dir().join("d-000004.json.tmp"), b"").unwrap();
        fs::write(dd.state_dir().join("d-5.json"), b"").unwrap();
        fs::write(dd.state_dir().join("notes.txt"), b"").unwrap();
        let ids: Vec<u64> = dd
            .list_state_ids()
            .unwrap()
            .into_iter()
            .map(DownloadId::value)
            .collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
