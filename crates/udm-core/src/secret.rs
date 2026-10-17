//! A secret value (password, cookie header). `Debug` and `Display` always
//! print `***`; the value is only reachable through [`Secret::expose`].
//! Lives in L0 so that every layer can carry secrets without depending on
//! the keychain crate.

use std::fmt;

/// A redacted secret string.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    /// Wraps a value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value. Never log or display it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted() {
        let s = Secret::new("udm-secret-pass-123");
        assert_eq!(format!("{s:?} {s}"), "*** ***");
        assert_eq!(s.expose(), "udm-secret-pass-123");
    }
}
