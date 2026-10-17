//! Clipboard and desktop notifications.

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

#[cfg(test)]
mod tests {
    use super::*;

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
