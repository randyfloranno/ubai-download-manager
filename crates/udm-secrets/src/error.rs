//! Typed secret-store errors. They never carry secret material.

use udm_core::{Message, limits};

/// Errors returned by `udm-secrets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The secret reference is empty, too long or contains control characters.
    InvalidRef {
        /// `empty`, `too-long` or `control-char`.
        reason: &'static str,
    },
    /// The OS keychain is not available or locked.
    Unavailable {
        /// Platform error text (never the secret).
        detail: String,
    },
    /// The keychain refused a value because an attribute is too long.
    TooLong {
        /// Attribute name reported by the keychain.
        attribute: String,
        /// Platform limit.
        max: u32,
    },
    /// Any other keychain failure.
    Backend {
        /// Short machine-readable code, or platform error text.
        detail: String,
    },
}

impl SecretError {
    /// Localisable message (`error-secrets-*`).
    pub fn to_message(&self) -> Message {
        match self {
            SecretError::InvalidRef { reason } => Message::new("error-secrets-invalid-ref")
                .with_str("reason", *reason)
                .with_uint(
                    "max",
                    u64::try_from(limits::MAX_SECRET_REF_BYTES).unwrap_or(u64::MAX),
                ),
            SecretError::Unavailable { detail } => {
                Message::new("error-secrets-unavailable").with_str("detail", detail.clone())
            }
            SecretError::TooLong { attribute, max } => Message::new("error-secrets-too-long")
                .with_str("attribute", attribute.clone())
                .with_uint("max", u64::from(*max)),
            SecretError::Backend { detail } => {
                Message::new("error-secrets-backend").with_str("detail", detail.clone())
            }
        }
    }
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_message())
    }
}

impl std::error::Error for SecretError {}

impl udm_core::message::ToMessage for SecretError {
    fn to_message(&self) -> Message {
        SecretError::to_message(self)
    }
}
