use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard};

use ot_core::{Identity, KeyringError, SecretStore};
use ot_proto::b64;
use zeroize::Zeroizing;

pub const DEVICE_KEY: &str = "device.key";
pub const PENDING_OFFER: &str = "pending_offer.json";
pub const PENDING_JOIN: &str = "pending_join.json";

#[derive(Default)]
struct State {
    values: HashMap<String, Zeroizing<Vec<u8>>>,
    changed: BTreeSet<String>,
}

pub struct Change {
    pub name: String,
    pub value: Option<Zeroizing<Vec<u8>>>,
}

#[derive(Default)]
pub struct VaultStore {
    state: Mutex<State>,
}

impl VaultStore {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn load(&self, name: &str, value: &[u8]) {
        self.lock().values.insert(name.to_owned(), Zeroizing::new(value.to_vec()));
    }

    pub fn has(&self, name: &str) -> bool {
        self.lock().values.contains_key(name)
    }

    pub fn take_changes(&self) -> Vec<Change> {
        let mut state = self.lock();
        let names = std::mem::take(&mut state.changed);
        names
            .into_iter()
            .map(|name| {
                let value = state.values.get(&name).cloned();
                Change { name, value }
            })
            .collect()
    }

    pub fn clear(&self) {
        let mut state = self.lock();
        let names: Vec<String> = state.values.keys().cloned().collect();
        state.values.clear();
        state.changed.extend(names);
    }

    pub fn ensure_identity(&self) -> Result<Identity, String> {
        if let Some(identity) = self.identity()? {
            return Ok(identity);
        }
        let identity = Identity::generate();
        let seed = Zeroizing::new(identity.to_bytes());
        let text = Zeroizing::new(format!("{}\n", b64::encode(&*seed)));
        self.put(DEVICE_KEY, text.as_bytes()).map_err(|e| e.to_string())?;
        Ok(identity)
    }

    pub fn identity(&self) -> Result<Option<Identity>, String> {
        let Some(bytes) = self.get(DEVICE_KEY).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
        let seed = Zeroizing::new(
            b64::decode_array::<32>(text.trim()).ok_or(format!("{DEVICE_KEY} est corrompu"))?,
        );
        Ok(Some(Identity::from_bytes(&seed)))
    }
}

impl SecretStore for VaultStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError> {
        Ok(self.lock().values.get(name).cloned())
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError> {
        let mut state = self.lock();
        state.values.insert(name.to_owned(), Zeroizing::new(value.to_vec()));
        state.changed.insert(name.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), KeyringError> {
        let mut state = self.lock();
        if state.values.remove(name).is_some() {
            state.changed.insert(name.to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_tracked_once() {
        let vault = VaultStore::default();
        vault.load("a", b"1");
        assert!(vault.take_changes().is_empty(), "un chargement n'est pas un changement");
        vault.put("b", b"2").unwrap();
        vault.delete("a").unwrap();
        vault.delete("absent").unwrap();
        let changes = vault.take_changes();
        let summary: Vec<(&str, Option<&[u8]>)> =
            changes.iter().map(|c| (c.name.as_str(), c.value.as_deref().map(|v| &v[..]))).collect();
        assert_eq!(summary, [("a", None), ("b", Some(&b"2"[..]))]);
        assert!(vault.take_changes().is_empty());
    }

    #[test]
    fn identity_is_created_once_in_cli_format() {
        let vault = VaultStore::default();
        let first = vault.ensure_identity().unwrap();
        let again = vault.ensure_identity().unwrap();
        assert_eq!(first.public_key(), again.public_key());
        let text = vault.get(DEVICE_KEY).unwrap().unwrap();
        assert_eq!(&text[..], format!("{}\n", b64::encode(&first.to_bytes())).as_bytes());
    }

    #[test]
    fn clear_reports_every_name() {
        let vault = VaultStore::default();
        vault.load("a", b"1");
        vault.put("b", b"2").unwrap();
        vault.take_changes();
        vault.clear();
        let names: Vec<String> = vault.take_changes().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(!vault.has("a"));
    }
}
