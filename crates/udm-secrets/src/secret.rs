//! Secret references and the store trait. The value type is [`udm_core::Secret`].

use std::fmt;

pub use udm_core::Secret;
use udm_core::limits;

use crate::error::SecretError;

/// A name under which a secret is stored, e.g. `proxy:main` or `site:example.com:user`.
///
/// References are not secret: they may appear in settings, state files and logs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretRef(String);

impl SecretRef {
    /// Validates a reference: non-empty, at most `limits::MAX_SECRET_REF_BYTES`, no control
    /// characters.
    pub fn new(name: impl Into<String>) -> Result<Self, SecretError> {
        let name = name.into();
        if name.is_empty() {
            return Err(SecretError::InvalidRef { reason: "empty" });
        }
        if name.len() > limits::MAX_SECRET_REF_BYTES {
            return Err(SecretError::InvalidRef { reason: "too-long" });
        }
        if name.chars().any(char::is_control) {
            return Err(SecretError::InvalidRef {
                reason: "control-char",
            });
        }
        Ok(Self(name))
    }

    /// The reference text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for SecretRef {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Storage for secrets, keyed by [`SecretRef`].
pub trait SecretStore: Send + Sync {
    /// Stores (or replaces) a secret.
    fn put(&self, r: &SecretRef, s: &Secret) -> Result<(), SecretError>;
    /// Fetches a secret; `Ok(None)` if there is none.
    fn get(&self, r: &SecretRef) -> Result<Option<Secret>, SecretError>;
    /// Deletes a secret; deleting a missing secret is `Ok`.
    fn delete(&self, r: &SecretRef) -> Result<(), SecretError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "hunter2-SECRET-FIXTURE";

    #[test]
    fn hostile_secret_debug_never_leaks() {
        let s = Secret::new(FIXTURE.to_owned());
        assert_eq!(format!("{s:?}"), "***");
        assert_eq!(format!("{s}"), "***");
        assert_eq!(format!("{s:#?}"), "***");
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            user: String,
            password: Secret,
            maybe: Option<Secret>,
        }
        let h = Holder {
            user: "alice".into(),
            password: s.clone(),
            maybe: Some(s.clone()),
        };
        let dbg = format!("{h:?} {h:#?}");
        assert!(!dbg.contains(FIXTURE), "{dbg}");
        assert!(dbg.contains("alice"));
        assert_eq!(s.expose(), FIXTURE);
    }

    #[test]
    fn secret_ref_roundtrip() {
        let r = SecretRef::new("site:example.com:user").unwrap();
        assert_eq!(r.as_str(), "site:example.com:user");
        assert_eq!(r.to_string(), "site:example.com:user");
        assert_eq!(
            "proxy:main".parse::<SecretRef>().unwrap().as_str(),
            "proxy:main"
        );
    }

    #[test]
    fn hostile_secret_ref_invalid() {
        assert_eq!(
            SecretRef::new(""),
            Err(SecretError::InvalidRef { reason: "empty" })
        );
        assert_eq!(
            SecretRef::new("a\nb"),
            Err(SecretError::InvalidRef {
                reason: "control-char"
            })
        );
        assert_eq!(
            SecretRef::new("a\u{0}"),
            Err(SecretError::InvalidRef {
                reason: "control-char"
            })
        );
    }

    #[test]
    fn hostile_secret_ref_too_long() {
        let ok = "r".repeat(limits::MAX_SECRET_REF_BYTES);
        assert!(SecretRef::new(ok).is_ok());
        let long = "r".repeat(limits::MAX_SECRET_REF_BYTES + 1);
        let err = SecretRef::new(long).unwrap_err();
        assert_eq!(err, SecretError::InvalidRef { reason: "too-long" });
        assert_eq!(err.to_message().key, "error-secrets-invalid-ref");
    }
}
