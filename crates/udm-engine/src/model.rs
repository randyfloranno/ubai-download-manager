//! Persisted records and serialisable views of downloads and queues.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use udm_core::{DownloadId, DownloadStatus, Message, MsgArg, PauseReason};

use crate::schedule::Schedule;

/// Serde mirror of [`Message`] (udm-core has no dependencies).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMessage {
    /// i18n key.
    pub key: String,
    /// Arguments.
    #[serde(default)]
    pub args: Vec<StoredArg>,
}

/// One argument of a [`StoredMessage`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredArg {
    /// Argument name.
    pub name: String,
    /// Kind: `str`, `int`, `uint` or `bytes`.
    pub kind: String,
    /// Value as text.
    pub value: String,
}

impl From<&Message> for StoredMessage {
    fn from(m: &Message) -> Self {
        let args = m
            .args
            .iter()
            .map(|(name, v)| {
                let (kind, value) = match v {
                    MsgArg::Str(s) => ("str", s.clone()),
                    MsgArg::Int(i) => ("int", i.to_string()),
                    MsgArg::UInt(u) => ("uint", u.to_string()),
                    MsgArg::Bytes(b) => ("bytes", b.to_string()),
                };
                StoredArg {
                    name: name.clone(),
                    kind: kind.to_owned(),
                    value,
                }
            })
            .collect();
        Self {
            key: m.key.clone(),
            args,
        }
    }
}

impl StoredMessage {
    /// Converts back; unparsable numeric arguments become strings.
    pub fn to_message(&self) -> Message {
        let mut m = Message::new(self.key.clone());
        for a in &self.args {
            m = match a.kind.as_str() {
                "int" => match a.value.parse::<i64>() {
                    Ok(v) => m.with_int(&a.name, v),
                    Err(_) => m.with_str(&a.name, a.value.clone()),
                },
                "uint" => match a.value.parse::<u64>() {
                    Ok(v) => m.with_uint(&a.name, v),
                    Err(_) => m.with_str(&a.name, a.value.clone()),
                },
                "bytes" => match a.value.parse::<u64>() {
                    Ok(v) => m.with_bytes(&a.name, v),
                    Err(_) => m.with_str(&a.name, a.value.clone()),
                },
                _ => m.with_str(&a.name, a.value.clone()),
            };
        }
        m
    }
}

/// Persisted form of [`DownloadStatus`]. Transient states (probing,
/// downloading, finalising) are stored as `queued` so that an interrupted
/// download is resumed on the next start (crash recovery).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StoredStatus {
    /// Waiting in its queue.
    Queued,
    /// Paused, with reason.
    Paused {
        /// Reason name ([`PauseReason::as_str`]).
        reason: String,
    },
    /// Completed.
    Completed,
    /// Failed with message.
    Failed {
        /// Error message.
        error: StoredMessage,
    },
}

impl StoredStatus {
    /// From a live status.
    pub fn from_status(s: &DownloadStatus) -> Self {
        match s {
            DownloadStatus::Queued
            | DownloadStatus::Scheduled
            | DownloadStatus::Probing
            | DownloadStatus::Downloading
            | DownloadStatus::Finalising => StoredStatus::Queued,
            DownloadStatus::Paused(r) => StoredStatus::Paused {
                reason: r.as_str().to_owned(),
            },
            DownloadStatus::Completed => StoredStatus::Completed,
            DownloadStatus::Failed(m) => StoredStatus::Failed {
                error: StoredMessage::from(m),
            },
        }
    }

    /// To a live status.
    pub fn to_status(&self) -> DownloadStatus {
        match self {
            StoredStatus::Queued => DownloadStatus::Queued,
            StoredStatus::Paused { reason } => {
                DownloadStatus::Paused(PauseReason::parse(reason).unwrap_or(PauseReason::User))
            }
            StoredStatus::Completed => DownloadStatus::Completed,
            StoredStatus::Failed { error } => DownloadStatus::Failed(error.to_message()),
        }
    }
}

/// Persistent description of one download (queue file).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DownloadRecord {
    /// Id.
    pub id: DownloadIdRepr,
    /// URL as entered (credentials stripped).
    pub url: String,
    /// Target folder.
    pub dir: PathBuf,
    /// Final sanitised file name, once known.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Name requested by the user (sanitised before use).
    #[serde(default)]
    pub requested_name: Option<String>,
    /// Category id.
    pub category: String,
    /// Queue id.
    pub queue: String,
    /// Connections.
    pub connections: u32,
    /// Per-download speed limit (0 = unlimited).
    #[serde(default)]
    pub speed_limit: u64,
    /// Expected checksum spec (`sha256:<hex>`).
    #[serde(default)]
    pub checksum: Option<String>,
    /// Referer.
    #[serde(default)]
    pub referer: Option<String>,
    /// Keychain reference of the captured cookie header.
    #[serde(default)]
    pub cookie_ref: Option<String>,
    /// User name for credentials taken from the URL (password in the keychain
    /// under [`DownloadRecord::auth_ref`]).
    #[serde(default)]
    pub auth_user: Option<String>,
    /// HTTPS→HTTP downgrade allowed for this download.
    #[serde(default)]
    pub allow_downgrade: bool,
    /// Status.
    pub status: StoredStatus,
    /// Total size, when known.
    #[serde(default)]
    pub size: Option<u64>,
    /// Bytes stored.
    #[serde(default)]
    pub downloaded: u64,
    /// Whether the server supports ranges (resume possible).
    #[serde(default)]
    pub resumable: bool,
    /// The user explicitly started this download (outside queue processing).
    #[serde(default)]
    pub manual_start: bool,
    /// Unix seconds when added.
    pub added_at: i64,
    /// Unix seconds when completed.
    #[serde(default)]
    pub completed_at: Option<i64>,
    /// Last informational notice (e.g. restarted because the remote changed).
    #[serde(default)]
    pub notice: Option<StoredMessage>,
}

/// Download ids are stored as their display form (`d-000001`).
pub type DownloadIdRepr = String;

impl DownloadRecord {
    /// Parsed id.
    pub fn download_id(&self) -> Option<DownloadId> {
        self.id.parse().ok()
    }

    /// Keychain reference of the cookie captured for this download.
    pub fn cookie_secret_ref(id: &str) -> String {
        format!("cookie:{id}")
    }

    /// Keychain reference of URL credentials for this download.
    pub fn auth_ref(id: &str) -> String {
        format!("auth:{id}")
    }
}

/// One queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueRecord {
    /// Machine id (`main`, …).
    pub id: String,
    /// Display name (user text).
    pub name: String,
    /// Simultaneous downloads.
    pub concurrency: u32,
    /// Whether the queue is processing.
    #[serde(default)]
    pub running: bool,
    /// Schedule.
    #[serde(default)]
    pub schedule: Schedule,
    /// Ordered download ids.
    #[serde(default)]
    pub items: Vec<DownloadIdRepr>,
}

/// Persisted queue file payload.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueFile {
    /// Next id to allocate.
    pub next_id: u64,
    /// Downloads.
    pub downloads: Vec<DownloadRecord>,
    /// Queues.
    pub queues: Vec<QueueRecord>,
}

/// Resume checkpoint (`state/<id>.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeState {
    /// Download id.
    pub id: DownloadIdRepr,
    /// Effective URL after redirects.
    pub final_url: String,
    /// Expected size.
    pub size: Option<u64>,
    /// ETag header value as received.
    pub etag: Option<String>,
    /// Last-Modified header value as received.
    pub last_modified: Option<String>,
    /// Part file path.
    pub part_path: PathBuf,
    /// Final file name.
    pub file_name: String,
    /// Completed, durable byte ranges `[start, end)`.
    pub coverage: Vec<(u64, u64)>,
    /// Server supports ranges.
    pub resumable: bool,
}

/// Segment shown in the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SegmentView {
    /// Start offset.
    pub start: u64,
    /// End offset (exclusive).
    pub end: u64,
    /// Current position.
    pub pos: u64,
}

/// Snapshot of one download for UI, CLI and control port.
#[derive(Clone, Debug, Serialize)]
pub struct DownloadView {
    /// Id (`d-000001`).
    pub id: String,
    /// URL.
    pub url: String,
    /// File name.
    pub file_name: String,
    /// Target folder.
    pub dir: String,
    /// Category id.
    pub category: String,
    /// Queue id.
    pub queue: String,
    /// Machine status (`downloading`, …).
    pub status: String,
    /// i18n key of the status label.
    pub status_key: String,
    /// Error or notice message, if any.
    pub message: Option<StoredMessage>,
    /// Total size.
    pub size: Option<u64>,
    /// Bytes done.
    pub downloaded: u64,
    /// Current speed in bytes per second.
    pub speed: u64,
    /// Estimated seconds remaining.
    pub eta_secs: Option<u64>,
    /// Active connections.
    pub active_connections: u32,
    /// Configured connections.
    pub connections: u32,
    /// Speed limit (0 = unlimited).
    pub speed_limit: u64,
    /// Resume supported.
    pub resumable: bool,
    /// Name has a dangerous double extension (`invoice.pdf.exe`).
    pub dangerous_name: bool,
    /// Expected checksum.
    pub checksum: Option<String>,
    /// Added (unix seconds).
    pub added_at: i64,
    /// Completed (unix seconds).
    pub completed_at: Option<i64>,
    /// Active segments.
    pub segments: Vec<SegmentView>,
}

/// One completed download in history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Download id.
    pub id: DownloadIdRepr,
    /// URL.
    pub url: String,
    /// Full path of the finished file.
    pub path: String,
    /// File name.
    pub file_name: String,
    /// Size.
    pub size: u64,
    /// Category.
    pub category: String,
    /// Completed (unix seconds).
    pub completed_at: i64,
    /// SHA-256 if computed.
    #[serde(default)]
    pub sha256: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roundtrip() {
        let m = Message::new("error-x")
            .with_str("a", "b")
            .with_uint("n", 5)
            .with_int("i", -3)
            .with_bytes("b", 1024);
        let s = StoredMessage::from(&m);
        assert_eq!(s.to_message(), m);
    }

    #[test]
    fn transient_status_persists_as_queued() {
        assert_eq!(
            StoredStatus::from_status(&DownloadStatus::Downloading),
            StoredStatus::Queued
        );
        let p = StoredStatus::from_status(&DownloadStatus::Paused(PauseReason::DiskFull));
        assert_eq!(p.to_status(), DownloadStatus::Paused(PauseReason::DiskFull));
    }

    #[test]
    fn hostile_stored_message_bad_numbers() {
        let s = StoredMessage {
            key: "k".into(),
            args: vec![StoredArg {
                name: "n".into(),
                kind: "uint".into(),
                value: "-1x".into(),
            }],
        };
        assert_eq!(s.to_message().arg("n"), Some(&MsgArg::Str("-1x".into())));
        let p = StoredStatus::Paused {
            reason: "bogus".into(),
        };
        assert_eq!(p.to_status(), DownloadStatus::Paused(PauseReason::User));
    }
}
