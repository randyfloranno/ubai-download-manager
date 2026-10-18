//! Clipboard, desktop notifications and opening files in the desktop shell.

use std::path::Path;
use std::process::Command;

use udm_core::limits;

use crate::error::SysError;

/// Truncates `s` to at most `max` bytes, on a UTF-8 character boundary.
pub(crate) fn truncate_on_char_boundary(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    s.truncate(cut);
    s
}

/// Current clipboard text, truncated to `limits::MAX_CLIPBOARD_BYTES`.
///
/// `Ok(None)` when the clipboard holds no text.
pub fn clipboard_text() -> Result<Option<String>, SysError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| SysError::Clipboard {
        detail: e.to_string(),
    })?;
    match cb.get_text() {
        Ok(text) => Ok(Some(truncate_on_char_boundary(
            text,
            limits::MAX_CLIPBOARD_BYTES,
        ))),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(SysError::Clipboard {
            detail: e.to_string(),
        }),
    }
}

/// Shows a desktop notification.
pub fn notify(title: &str, body: &str) -> Result<(), SysError> {
    notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .appname("UDM")
        .show()
        .map(|_| ())
        .map_err(|e| SysError::Notify {
            detail: e.to_string(),
        })
}

/// Opens `path` with its default program, or (`reveal`) shows it selected in
/// the file manager. Programs get the path as one argument; no shell runs.
pub fn open_path(path: &Path, reveal: bool) -> Result<(), SysError> {
    let fail = |detail: String| SysError::Open {
        path: path.display().to_string(),
        detail,
    };
    let mut cmd = open_command(path, reveal).ok_or_else(|| fail("unsupported".into()))?;
    let mut child = cmd.spawn().map_err(|e| fail(e.to_string()))?;
    // The opener hands over to the desktop and exits; reap it off-thread.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(windows)]
fn open_command(path: &Path, reveal: bool) -> Option<Command> {
    use std::os::windows::process::CommandExt as _;
    let mut cmd = Command::new("explorer.exe");
    if reveal {
        // Explorer parses `/select,"<path>"` itself; Windows paths cannot contain `"`.
        let shown = path.display().to_string();
        if shown.contains('"') {
            return None;
        }
        cmd.raw_arg(format!("/select,\"{shown}\""));
    } else {
        cmd.arg(path);
    }
    Some(cmd)
}

#[cfg(target_os = "macos")]
fn open_command(path: &Path, reveal: bool) -> Option<Command> {
    let mut cmd = Command::new("open");
    if reveal {
        cmd.arg("-R");
    }
    cmd.arg(path);
    Some(cmd)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_command(path: &Path, reveal: bool) -> Option<Command> {
    // xdg-open cannot select a file: reveal opens the containing folder.
    let target = if reveal { path.parent()? } else { path };
    let mut cmd = Command::new("xdg-open");
    cmd.arg(target);
    Some(cmd)
}

#[cfg(not(any(unix, windows)))]
fn open_command(_path: &Path, _reveal: bool) -> Option<Command> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_command_passes_path_as_one_argument() {
        let p = Path::new("dir with space").join("a & b.txt");
        let cmd = open_command(&p, false).unwrap();
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, vec![p.as_os_str()]);
        assert!(open_command(&p, true).is_some());
    }

    #[test]
    fn hostile_clipboard_truncated_on_char_boundary() {
        // 3-byte characters: the limit falls inside a character.
        let s = "€".repeat(limits::MAX_CLIPBOARD_BYTES);
        let t = truncate_on_char_boundary(s, limits::MAX_CLIPBOARD_BYTES);
        assert!(t.len() <= limits::MAX_CLIPBOARD_BYTES);
        assert!(t.len() > limits::MAX_CLIPBOARD_BYTES - 3);
        assert!(t.chars().all(|c| c == '€'));

        let short = "http://x/a".to_owned();
        assert_eq!(
            truncate_on_char_boundary(short.clone(), limits::MAX_CLIPBOARD_BYTES),
            short
        );
        let ascii = "a".repeat(limits::MAX_CLIPBOARD_BYTES + 10);
        assert_eq!(
            truncate_on_char_boundary(ascii, limits::MAX_CLIPBOARD_BYTES).len(),
            limits::MAX_CLIPBOARD_BYTES
        );
    }
}
