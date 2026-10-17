//! Download status state machine.

use crate::message::Message;

/// Why a download is paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseReason {
    /// The user paused it.
    User,
    /// The target volume ran out of space.
    DiskFull,
    /// A write error (quota, removed drive, ...).
    DiskWrite,
    /// The application shut down while it was active.
    Shutdown,
    /// A scheduler stop time was reached.
    Schedule,
}

impl PauseReason {
    /// Stable machine name.
    pub const fn as_str(self) -> &'static str {
        match self {
            PauseReason::User => "user",
            PauseReason::DiskFull => "disk_full",
            PauseReason::DiskWrite => "disk_write",
            PauseReason::Shutdown => "shutdown",
            PauseReason::Schedule => "schedule",
        }
    }

    /// Parses a machine name.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user" => PauseReason::User,
            "disk_full" => PauseReason::DiskFull,
            "disk_write" => PauseReason::DiskWrite,
            "shutdown" => PauseReason::Shutdown,
            "schedule" => PauseReason::Schedule,
            _ => return None,
        })
    }
}

/// Status of one download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadStatus {
    /// Waiting in a queue.
    Queued,
    /// Waiting for a scheduled start time.
    Scheduled,
    /// Requesting metadata.
    Probing,
    /// Transferring data.
    Downloading,
    /// Paused with a reason.
    Paused(PauseReason),
    /// Running final checks.
    Finalising,
    /// Done; final file in place.
    Completed,
    /// Failed with a localisable error.
    Failed(Message),
}

impl DownloadStatus {
    /// Stable machine name used in JSON and state files.
    pub const fn as_str(&self) -> &'static str {
        match self {
            DownloadStatus::Queued => "queued",
            DownloadStatus::Scheduled => "scheduled",
            DownloadStatus::Probing => "probing",
            DownloadStatus::Downloading => "downloading",
            DownloadStatus::Paused(_) => "paused",
            DownloadStatus::Finalising => "finalising",
            DownloadStatus::Completed => "completed",
            DownloadStatus::Failed(_) => "failed",
        }
    }

    /// i18n key of the status label.
    pub const fn label_key(&self) -> &'static str {
        match self {
            DownloadStatus::Queued => "status-queued",
            DownloadStatus::Scheduled => "status-scheduled",
            DownloadStatus::Probing => "status-probing",
            DownloadStatus::Downloading => "status-downloading",
            DownloadStatus::Paused(PauseReason::User) => "status-paused",
            DownloadStatus::Paused(PauseReason::DiskFull) => "status-paused-disk-full",
            DownloadStatus::Paused(PauseReason::DiskWrite) => "status-paused-disk-write",
            DownloadStatus::Paused(PauseReason::Shutdown) => "status-paused-shutdown",
            DownloadStatus::Paused(PauseReason::Schedule) => "status-paused-schedule",
            DownloadStatus::Finalising => "status-finalising",
            DownloadStatus::Completed => "status-completed",
            DownloadStatus::Failed(_) => "status-failed",
        }
    }

    /// Active means a task is running for it.
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Probing | DownloadStatus::Downloading | DownloadStatus::Finalising
        )
    }

    /// Whether `download.pause` is meaningful.
    pub const fn can_pause(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Queued
                | DownloadStatus::Scheduled
                | DownloadStatus::Probing
                | DownloadStatus::Downloading
        )
    }

    /// Whether `download.resume` / start is meaningful.
    pub const fn can_start(&self) -> bool {
        matches!(
            self,
            DownloadStatus::Queued
                | DownloadStatus::Scheduled
                | DownloadStatus::Paused(_)
                | DownloadStatus::Failed(_)
        )
    }

    /// Completed downloads are terminal.
    pub const fn is_completed(&self) -> bool {
        matches!(self, DownloadStatus::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_reason_roundtrip() {
        for r in [
            PauseReason::User,
            PauseReason::DiskFull,
            PauseReason::DiskWrite,
            PauseReason::Shutdown,
            PauseReason::Schedule,
        ] {
            assert_eq!(PauseReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(PauseReason::parse("nope"), None);
    }

    #[test]
    fn transitions_flags() {
        assert!(DownloadStatus::Downloading.can_pause());
        assert!(!DownloadStatus::Completed.can_start());
        assert!(DownloadStatus::Paused(PauseReason::DiskFull).can_start());
        assert!(DownloadStatus::Finalising.is_active());
    }
}
