//! In-memory secret store (tests, and sessions where the keychain is unavailable).

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::SecretError;
use crate::secret::{Secret, SecretRef, SecretStore};

/// A process-local [`SecretStore`]. Nothing is persisted.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<SecretRef, Secret>>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.map.lock().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("MemoryStore").field("entries", &n).finish()
    }
}

impl MemoryStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

fn poisoned() -> SecretError {
    SecretError::Backend {
        detail: "lock-poisoned".to_owned(),
    }
}

impl SecretStore for MemoryStore {
    fn put(&self, r: &SecretRef, s: &Secret) -> Result<(), SecretError> {
        self.map
            .lock()
            .map_err(|_| poisoned())?
            .insert(r.clone(), s.clone());
        Ok(())
    }

    fn get(&self, r: &SecretRef) -> Result<Option<Secret>, SecretError> {
        Ok(self.map.lock().map_err(|_| poisoned())?.get(r).cloned())
    }

    fn delete(&self, r: &SecretRef) -> Result<(), SecretError> {
        self.map.lock().map_err(|_| poisoned())?.remove(r);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_delete() {
        let store = MemoryStore::new();
        let r = SecretRef::new("proxy:main").unwrap();
        assert!(store.get(&r).unwrap().is_none());
        store.put(&r, &Secret::new("pw1")).unwrap();
        store.put(&r, &Secret::new("pw2")).unwrap();
        assert_eq!(store.get(&r).unwrap().unwrap().expose(), "pw2");
        store.delete(&r).unwrap();
        store.delete(&r).unwrap();
        assert!(store.get(&r).unwrap().is_none());
    }

    #[test]
    fn hostile_memory_store_debug_never_leaks() {
        let store = MemoryStore::new();
        let r = SecretRef::new("site:x:u").unwrap();
        store.put(&r, &Secret::new("TOPSECRET-FIXTURE")).unwrap();
        let dbg = format!("{store:?}");
        assert!(!dbg.contains("TOPSECRET-FIXTURE"));
    }

    #[test]
    fn usable_as_trait_object() {
        let store: Box<dyn SecretStore> = Box::new(MemoryStore::new());
        let r = SecretRef::new("a").unwrap();
        store.put(&r, &Secret::new("b")).unwrap();
        assert!(store.get(&r).unwrap().is_some());
    }
}
