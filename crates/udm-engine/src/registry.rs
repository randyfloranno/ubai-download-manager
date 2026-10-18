//! Command registry: every user action is a [`CommandSpec`].
//! Labels and descriptions are i18n keys `cmd-<area>-<verb>[-desc]`.

use udm_core::{ArgKind, ArgSpec, CommandSpec};

use crate::schedule::AfterDone;
use crate::settings::ProxyKind;

macro_rules! cmd {
    ($id:literal, $key:literal, $confirm:expr, [$($arg:expr),* $(,)?]) => {
        CommandSpec {
            id: $id,
            label_key: $key,
            description_key: concat!($key, "-desc"),
            args: &[$($arg),*],
            needs_confirmation: $confirm,
        }
    };
}

const fn req(name: &'static str, kind: ArgKind) -> ArgSpec {
    ArgSpec::required(name, kind)
}

const fn opt(name: &'static str, kind: ArgKind) -> ArgSpec {
    ArgSpec::optional(name, kind)
}

const ID: ArgKind = ArgKind::DownloadId;
const STR: ArgKind = ArgKind::Str;
const UINT: ArgKind = ArgKind::UInt;
const BOOL: ArgKind = ArgKind::Bool;
const LIST: ArgKind = ArgKind::StrList;

/// All commands, in menu order.
pub static COMMANDS: &[CommandSpec] = &[
    cmd!(
        "download.add",
        "cmd-download-add",
        false,
        [
            req("url", STR),
            opt("dir", STR),
            opt("file_name", STR),
            opt("connections", UINT),
            opt("category", STR),
            opt("queue", STR),
            opt("start", BOOL),
            opt("checksum", STR),
            opt("referer", STR),
            opt("speed_limit", UINT),
            opt("allow_downgrade", BOOL),
        ]
    ),
    cmd!(
        "download.pause",
        "cmd-download-pause",
        false,
        [req("download_id", ID)]
    ),
    cmd!(
        "download.resume",
        "cmd-download-resume",
        false,
        [req("download_id", ID)]
    ),
    cmd!(
        "download.remove",
        "cmd-download-remove",
        true,
        [req("download_id", ID), opt("delete_file", BOOL),]
    ),
    cmd!(
        "download.restart",
        "cmd-download-restart",
        true,
        [req("download_id", ID)]
    ),
    cmd!(
        "download.verify",
        "cmd-download-verify",
        false,
        [req("download_id", ID), opt("checksum", STR),]
    ),
    cmd!(
        "download.open",
        "cmd-download-open",
        false,
        [req("download_id", ID)]
    ),
    cmd!(
        "download.open_folder",
        "cmd-download-open-folder",
        false,
        [req("download_id", ID)]
    ),
    cmd!(
        "download.set_connections",
        "cmd-download-set-connections",
        false,
        [req("download_id", ID), req("connections", UINT),]
    ),
    cmd!("download.pause_all", "cmd-download-pause-all", false, []),
    cmd!("download.resume_all", "cmd-download-resume-all", false, []),
    cmd!(
        "download.remove_completed",
        "cmd-download-remove-completed",
        true,
        []
    ),
    cmd!("queue.start", "cmd-queue-start", false, [opt("queue", STR)]),
    cmd!("queue.stop", "cmd-queue-stop", false, [opt("queue", STR)]),
    cmd!(
        "queue.create",
        "cmd-queue-create",
        false,
        [
            req("queue", STR),
            opt("name", STR),
            opt("concurrency", UINT),
        ]
    ),
    cmd!(
        "queue.delete",
        "cmd-queue-delete",
        true,
        [req("queue", STR)]
    ),
    cmd!(
        "queue.set_concurrency",
        "cmd-queue-set-concurrency",
        false,
        [req("queue", STR), req("concurrency", UINT),]
    ),
    cmd!(
        "queue.move_item",
        "cmd-queue-move-item",
        false,
        [
            req("download_id", ID),
            opt("queue", STR),
            opt("position", UINT),
        ]
    ),
    cmd!(
        "schedule.set",
        "cmd-schedule-set",
        false,
        [
            opt("queue", STR),
            opt("start_at", STR),
            opt("stop_at", STR),
            opt("days", LIST),
            opt("after_done", ArgKind::Enum(AfterDone::NAMES)),
        ]
    ),
    cmd!(
        "schedule.clear",
        "cmd-schedule-clear",
        false,
        [opt("queue", STR)]
    ),
    cmd!(
        "limit.set_global",
        "cmd-limit-set-global",
        false,
        [req("bytes_per_sec", UINT)]
    ),
    cmd!(
        "limit.set_download",
        "cmd-limit-set-download",
        false,
        [req("download_id", ID), req("bytes_per_sec", UINT),]
    ),
    cmd!(
        "category.assign",
        "cmd-category-assign",
        false,
        [req("download_id", ID), req("category", STR),]
    ),
    cmd!(
        "category.set_folder",
        "cmd-category-set-folder",
        false,
        [req("category", STR), req("folder", STR),]
    ),
    cmd!(
        "category.set_extensions",
        "cmd-category-set-extensions",
        false,
        [req("category", STR), req("extensions", LIST),]
    ),
    cmd!(
        "capture.clipboard_toggle",
        "cmd-capture-clipboard-toggle",
        false,
        [opt("enabled", BOOL)]
    ),
    cmd!(
        "capture.clipboard_text",
        "cmd-capture-clipboard-text",
        false,
        [req("text", STR)]
    ),
    cmd!(
        "capture.browser",
        "cmd-capture-browser",
        false,
        [
            req("url", STR),
            opt("referer", STR),
            opt("cookie", STR),
            opt("file_name", STR),
            opt("size", UINT),
            opt("start", BOOL),
        ]
    ),
    cmd!(
        "batch.import",
        "cmd-batch-import",
        false,
        [
            req("text", STR),
            opt("dir", STR),
            opt("queue", STR),
            opt("start", BOOL),
        ]
    ),
    cmd!(
        "batch.pattern",
        "cmd-batch-pattern",
        false,
        [
            req("pattern", STR),
            opt("dir", STR),
            opt("queue", STR),
            opt("start", BOOL),
        ]
    ),
    cmd!(
        "grabber.start",
        "cmd-grabber-start",
        false,
        [
            req("url", STR),
            opt("depth", UINT),
            opt("extensions", LIST),
            opt("same_host", BOOL),
        ]
    ),
    cmd!(
        "grabber.add_results",
        "cmd-grabber-add-results",
        false,
        [
            req("job", UINT),
            opt("dir", STR),
            opt("queue", STR),
            opt("start", BOOL),
        ]
    ),
    cmd!(
        "settings.set",
        "cmd-settings-set",
        false,
        [req("name", STR), req("value", STR)]
    ),
    cmd!(
        "tls.allow_host",
        "cmd-tls-allow-host",
        true,
        [req("host", STR), req("fingerprint", STR),]
    ),
    cmd!(
        "tls.revoke_host",
        "cmd-tls-revoke-host",
        false,
        [req("host", STR)]
    ),
    cmd!(
        "proxy.set",
        "cmd-proxy-set",
        false,
        [
            req("kind", ArgKind::Enum(ProxyKind::NAMES)),
            opt("host", STR),
            opt("port", UINT),
            opt("user", STR),
            opt("password", STR),
        ]
    ),
    cmd!(
        "auth.set_site",
        "cmd-auth-set-site",
        false,
        [req("host", STR), req("user", STR), req("password", STR),]
    ),
    cmd!(
        "auth.remove_site",
        "cmd-auth-remove-site",
        false,
        [req("host", STR)]
    ),
    cmd!("history.clear", "cmd-history-clear", true, []),
    cmd!(
        "history.remove",
        "cmd-history-remove",
        false,
        [req("download_id", ID)]
    ),
];

/// Looks a command up by id.
pub fn find(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_well_formed() {
        for (i, c) in COMMANDS.iter().enumerate() {
            assert!(c.id_is_well_formed(), "{}", c.id);
            assert!(COMMANDS[i + 1..].iter().all(|o| o.id != c.id), "{}", c.id);
            let expected = format!("cmd-{}", c.id.replace(['.', '_'], "-"));
            assert_eq!(c.label_key, expected, "{}", c.id);
        }
    }

    #[test]
    fn required_agents_md_commands_exist() {
        for id in [
            "download.add",
            "download.pause",
            "download.resume",
            "download.remove",
            "download.verify",
            "queue.start",
            "queue.stop",
            "schedule.set",
            "limit.set_global",
            "limit.set_download",
            "category.assign",
            "capture.clipboard_toggle",
            "batch.import",
            "grabber.start",
            "settings.set",
        ] {
            assert!(find(id).is_some(), "{id}");
        }
    }
}
