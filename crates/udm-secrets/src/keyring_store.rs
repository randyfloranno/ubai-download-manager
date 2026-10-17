//! OS keychain store (Windows Credential Manager, macOS Keychain, Secret Service).

use crate::error::SecretError;
use crate::secret::{Secret, SecretRef, SecretStore};

/// Keychain service name under which every UDM secret is stored.
pub const KEYRING_SERVICE: &str = "dev.udm";

/// [`SecretStore`] backed by the OS keychain via the `keyring` crate.
///
/// The service is [`KEYRING_SERVICE`]; the entry user name is the [`SecretRef`] text.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    /// Creates the store. The keychain itself is opened lazily on first use.
    pub fn new() -> Self {
        Self
    }

    fn entry(r: &SecretRef) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(KEYRING_SERVICE, r.as_str()).map_err(map_error)
    }
}

/// Maps a keyring error without ever copying secret bytes (some variants carry the raw blob).
fn map_error(e: keyring::Error) -> SecretError {
    match e {
        keyring::Error::NoDefaultStore => SecretError::Unavailable {
            detail: "no-default-store".to_owned(),
        },
        keyring::Error::NoStorageAccess(p) => SecretError::Unavailable {
            detail: p.to_string(),
        },
        keyring::Error::PlatformFailure(p) => SecretError::Backend {
            detail: p.to_string(),
        },
        keyring::Error::TooLong(attribute, max) => SecretError::TooLong { attribute, max },
        keyring::Error::BadEncoding(_) => SecretError::Backend {
            detail: "bad-encoding".to_owned(),
        },
        keyring::Error::BadDataFormat(_, _) => SecretError::Backend {
            detail: "bad-data-format".to_owned(),
        },
        keyring::Error::BadStoreFormat(_) => SecretError::Backend {
            detail: "bad-store-format".to_owned(),
        },
        keyring::Error::Invalid(param, _) => SecretError::Backend {
            detail: format!("invalid-{param}"),
        },
        keyring::Error::Ambiguous(_) => SecretError::Backend {
            detail: "ambiguous".to_owned(),
        },
        keyring::Error::NotSupportedByStore(_) => SecretError::Backend {
            detail: "not-supported".to_owned(),
        },
        _ => SecretError::Backend {
            detail: "unknown".to_owned(),
        },
    }
}

impl SecretStore for KeyringStore {
    fn put(&self, r: &SecretRef, s: &Secret) -> Result<(), SecretError> {
        Self::entry(r)?.set_password(s.expose()).map_err(map_error)
    }

    fn get(&self, r: &SecretRef) -> Result<Option<Secret>, SecretError> {
        match Self::entry(r)?.get_password() {
            Ok(p) => Ok(Some(Secret::new(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        }
    }

    fn delete(&self, r: &SecretRef) -> Result<(), SecretError> {
        match Self::entry(r)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Never touches the real keychain: only checks types and error mapping.

    #[test]
    fn keyring_store_is_a_secret_store() {
        fn assert_store<S: SecretStore>() {}
        assert_store::<KeyringStore>();
        let _boxed: Box<dyn SecretStore> = Box::new(KeyringStore::new());
    }

    #[test]
    fn hostile_keyring_error_mapping_drops_secret_bytes() {
        let blob = b"LEAKED-SECRET-FIXTURE".to_vec();
        let mapped = map_error(keyring::Error::BadEncoding(blob));
        let text = format!("{mapped:?} {mapped} {:?}", mapped.to_message());
        assert!(!text.contains("LEAKED-SECRET-FIXTURE"), "{text}");
        assert_eq!(mapped.to_message().key, "error-secrets-backend");
        assert_eq!(
            map_error(keyring::Error::NoDefaultStore).to_message().key,
            "error-secrets-unavailable"
        );
        assert_eq!(
            map_error(keyring::Error::TooLong("user".into(), 10))
                .to_message()
                .key,
            "error-secrets-too-long"
        );
    }
}
