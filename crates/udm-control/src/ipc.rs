//! Capture IPC discovery file (`ipc.json` in the data dir): the port of the
//! restricted capture endpoint and a random token. Written atomically in a
//! checksummed envelope; the token is regenerated on every app start.

use std::path::Path;

use serde::{Deserialize, Serialize};
use udm_core::Message;
use udm_store::{StoreError, load_envelope, save_envelope};

/// Envelope kind of `ipc.json`.
pub const IPC_KIND: &str = "ipc";
/// Envelope schema of `ipc.json`.
pub const IPC_SCHEMA: u32 = 1;
/// Token length in random bytes (hex-encoded twice as long).
const TOKEN_BYTES: usize = 32;

/// Contents of `ipc.json`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcInfo {
    /// Capture endpoint port on 127.0.0.1.
    pub port: u16,
    /// Shared secret the native host sends with every request.
    pub token: String,
}

impl std::fmt::Debug for IpcInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcInfo")
            .field("port", &self.port)
            .field("token", &"***")
            .finish()
    }
}

impl IpcInfo {
    /// New info with a fresh random token.
    pub fn generate(port: u16) -> Result<Self, Message> {
        let mut raw = [0u8; TOKEN_BYTES];
        getrandom::fill(&mut raw).map_err(|_| Message::new("error-control-random"))?;
        let token = raw.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self { port, token })
    }

    /// Writes `path` atomically.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        save_envelope(path, IPC_KIND, IPC_SCHEMA, self)
    }

    /// Reads `path`; `Ok(None)` when the app has never run.
    pub fn load(path: &Path) -> Result<Option<Self>, StoreError> {
        Ok(load_envelope(path, IPC_KIND, IPC_SCHEMA)?.map(|(_, v)| v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ipc.json");
        let info = IpcInfo::generate(4242).unwrap();
        assert_eq!(info.token.len(), TOKEN_BYTES * 2);
        info.save(&p).unwrap();
        assert_eq!(IpcInfo::load(&p).unwrap(), Some(info.clone()));
        assert!(!format!("{info:?}").contains(&info.token));
        assert_ne!(IpcInfo::generate(1).unwrap().token, info.token);
    }

    #[test]
    fn hostile_ipc_file_tampered() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ipc.json");
        IpcInfo::generate(4242).unwrap().save(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap().replace("4242", "4243");
        std::fs::write(&p, text).unwrap();
        assert!(IpcInfo::load(&p).is_err());
    }
}
