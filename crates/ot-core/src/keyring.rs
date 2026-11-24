use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ot_crypto::{ExchangeKeyPair, GroupKey, Roster};
use ot_proto::{DeviceId, Envelope, b64};

use crate::directory::DirectoryState;
use crate::files::{PendingUpload, StoredFile};
use crate::invite::{InviteRecord, PendingInvite};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use zeroize::Zeroizing;

pub const RETIRED_KEY_LIFETIME: Duration = Duration::days(7);

pub const MAX_QUARANTINE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("stockage des clés : {0}")]
pub struct KeyringError(pub String);

pub trait Keyring: Send + Sync {
    fn load_exchange_key(&self) -> Result<Option<ExchangeKeyPair>, KeyringError>;
    fn save_exchange_key(&self, key: &ExchangeKeyPair) -> Result<(), KeyringError>;
    fn load_group_keys(&self) -> Result<Option<GroupKeys>, KeyringError>;
    fn save_group_keys(&self, keys: &GroupKeys) -> Result<(), KeyringError>;
    fn load_roster(&self) -> Result<Option<Roster>, KeyringError>;
    fn save_roster(&self, roster: &Roster) -> Result<(), KeyringError>;
    fn load_quarantine(&self) -> Result<Vec<Quarantined>, KeyringError>;
    fn save_quarantine(&self, messages: &[Quarantined]) -> Result<(), KeyringError>;
    fn load_files(&self) -> Result<Vec<StoredFile>, KeyringError>;
    fn save_files(&self, files: &[StoredFile]) -> Result<(), KeyringError>;
    fn load_uploads(&self) -> Result<Vec<PendingUpload>, KeyringError>;
    fn save_uploads(&self, uploads: &[PendingUpload]) -> Result<(), KeyringError>;
    fn load_directory(&self) -> Result<DirectoryState, KeyringError>;
    fn save_directory(&self, state: &DirectoryState) -> Result<(), KeyringError>;
    fn load_invites(&self) -> Result<Vec<InviteRecord>, KeyringError>;
    fn save_invites(&self, invites: &[InviteRecord]) -> Result<(), KeyringError>;
    fn load_pending_invite(&self) -> Result<Option<PendingInvite>, KeyringError>;
    fn save_pending_invite(&self, pending: Option<&PendingInvite>) -> Result<(), KeyringError>;
}

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError>;
    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError>;
    fn delete(&self, name: &str) -> Result<(), KeyringError>;
}

pub mod names {
    pub const EXCHANGE_KEY: &str = "exchange.key";
    pub const GROUP: &str = "group.json";
    pub const ROSTER: &str = "roster.json";
    pub const QUARANTINE: &str = "quarantine.json";
    pub const FILES: &str = "files.json";
    pub const UPLOADS: &str = "uploads.json";
    pub const DIRECTORY: &str = "directory.json";
    pub const INVITES: &str = "invites.json";
    pub const PENDING_INVITE: &str = "pending_invite.json";
}

#[derive(Clone)]
pub struct StoreKeyring {
    store: Arc<dyn SecretStore>,
}

impl StoreKeyring {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    pub fn forget_group(&self) -> Result<(), KeyringError> {
        for name in [
            names::EXCHANGE_KEY,
            names::GROUP,
            names::ROSTER,
            names::QUARANTINE,
            names::FILES,
            names::UPLOADS,
            names::DIRECTORY,
            names::INVITES,
            names::PENDING_INVITE,
        ] {
            self.store.delete(name)?;
        }
        Ok(())
    }

    fn read_json<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, KeyringError> {
        let Some(bytes) = self.store.get(name)? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| KeyringError(format!("{name} est corrompu : {e}")))
    }

    fn write_json<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<(), KeyringError> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|e| KeyringError(format!("sérialisation de {name} : {e}")))?;
        self.store.put(name, &Zeroizing::new(bytes))
    }
}

impl Keyring for StoreKeyring {
    fn load_exchange_key(&self) -> Result<Option<ExchangeKeyPair>, KeyringError> {
        let Some(bytes) = self.store.get(names::EXCHANGE_KEY)? else {
            return Ok(None);
        };
        let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
        let secret = Zeroizing::new(
            b64::decode_array::<32>(text.trim())
                .ok_or_else(|| KeyringError(format!("{} est corrompu", names::EXCHANGE_KEY)))?,
        );
        Ok(Some(ExchangeKeyPair::from_bytes(*secret)))
    }

    fn save_exchange_key(&self, key: &ExchangeKeyPair) -> Result<(), KeyringError> {
        let secret = Zeroizing::new(key.to_bytes());
        let text = Zeroizing::new(format!("{}\n", b64::encode(&*secret)));
        self.store.put(names::EXCHANGE_KEY, text.as_bytes())
    }

    fn load_group_keys(&self) -> Result<Option<GroupKeys>, KeyringError> {
        self.read_json(names::GROUP)
    }

    fn save_group_keys(&self, keys: &GroupKeys) -> Result<(), KeyringError> {
        self.write_json(names::GROUP, keys)
    }

    fn load_roster(&self) -> Result<Option<Roster>, KeyringError> {
        self.read_json(names::ROSTER)
    }

    fn save_roster(&self, roster: &Roster) -> Result<(), KeyringError> {
        self.write_json(names::ROSTER, roster)
    }

    fn load_quarantine(&self) -> Result<Vec<Quarantined>, KeyringError> {
        Ok(self.read_json(names::QUARANTINE)?.unwrap_or_default())
    }

    fn save_quarantine(&self, messages: &[Quarantined]) -> Result<(), KeyringError> {
        self.write_json(names::QUARANTINE, messages)
    }

    fn load_files(&self) -> Result<Vec<StoredFile>, KeyringError> {
        Ok(self.read_json(names::FILES)?.unwrap_or_default())
    }

    fn save_files(&self, files: &[StoredFile]) -> Result<(), KeyringError> {
        self.write_json(names::FILES, files)
    }

    fn load_uploads(&self) -> Result<Vec<PendingUpload>, KeyringError> {
        Ok(self.read_json(names::UPLOADS)?.unwrap_or_default())
    }

    fn save_uploads(&self, uploads: &[PendingUpload]) -> Result<(), KeyringError> {
        self.write_json(names::UPLOADS, uploads)
    }

    fn load_directory(&self) -> Result<DirectoryState, KeyringError> {
        Ok(self.read_json(names::DIRECTORY)?.unwrap_or_default())
    }

    fn save_directory(&self, state: &DirectoryState) -> Result<(), KeyringError> {
        self.write_json(names::DIRECTORY, state)
    }

    fn load_invites(&self) -> Result<Vec<InviteRecord>, KeyringError> {
        Ok(self.read_json(names::INVITES)?.unwrap_or_default())
    }

    fn save_invites(&self, invites: &[InviteRecord]) -> Result<(), KeyringError> {
        if invites.is_empty() {
            return self.store.delete(names::INVITES);
        }
        self.write_json(names::INVITES, invites)
    }

    fn load_pending_invite(&self) -> Result<Option<PendingInvite>, KeyringError> {
        self.read_json(names::PENDING_INVITE)
    }

    fn save_pending_invite(&self, pending: Option<&PendingInvite>) -> Result<(), KeyringError> {
        match pending {
            Some(p) => self.write_json(names::PENDING_INVITE, p),
            None => self.store.delete(names::PENDING_INVITE),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Zeroizing<Vec<u8>>>> {
        self.values.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.lock().keys().cloned().collect();
        names.sort();
        names
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError> {
        Ok(self.lock().get(name).cloned())
    }
    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError> {
        self.lock().insert(name.to_owned(), Zeroizing::new(value.to_vec()));
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), KeyringError> {
        self.lock().remove(name);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupKeys {
    pub current: u32,
    pub epochs: Vec<EpochEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochEntry {
    pub epoch: u32,
    pub key: GroupKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<DeviceId>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub retired_at: Option<OffsetDateTime>,
}

impl GroupKeys {
    pub fn new(epoch: u32, key: GroupKey) -> Self {
        Self {
            current: epoch,
            epochs: vec![EpochEntry { epoch, key, author: None, retired_at: None }],
        }
    }

    pub fn get(&self, epoch: u32) -> Option<&EpochEntry> {
        self.epochs.iter().find(|e| e.epoch == epoch)
    }

    pub fn current_key(&self) -> Option<&GroupKey> {
        self.get(self.current).map(|e| &e.key)
    }

    pub fn advance(
        &mut self,
        epoch: u32,
        key: GroupKey,
        author: Option<DeviceId>,
        now: OffsetDateTime,
    ) {
        for e in &mut self.epochs {
            if e.retired_at.is_none() {
                e.retired_at = Some(now);
            }
        }
        self.epochs.retain(|e| e.epoch != epoch);
        self.epochs.push(EpochEntry { epoch, key, author, retired_at: None });
        self.epochs.sort_by_key(|e| e.epoch);
        self.current = epoch;
    }

    pub fn insert_retired(
        &mut self,
        epoch: u32,
        key: GroupKey,
        author: Option<DeviceId>,
        now: OffsetDateTime,
    ) {
        if self.get(epoch).is_none() {
            self.epochs.push(EpochEntry { epoch, key, author, retired_at: Some(now) });
            self.epochs.sort_by_key(|e| e.epoch);
        }
    }

    pub fn prune(&mut self, now: OffsetDateTime) -> bool {
        let before = self.epochs.len();
        self.epochs.retain(|e| e.retired_at.is_none_or(|t| now - t < RETIRED_KEY_LIFETIME));
        self.epochs.len() != before
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quarantined {
    pub seq: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
    pub env: Envelope,
}

#[derive(Default)]
pub struct MemoryKeyring {
    inner: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    exchange: Option<ExchangeKeyPair>,
    keys: Option<GroupKeys>,
    roster: Option<Roster>,
    quarantine: Vec<Quarantined>,
    files: Vec<StoredFile>,
    uploads: Vec<PendingUpload>,
    directory: DirectoryState,
    invites: Vec<InviteRecord>,
    pending_invite: Option<PendingInvite>,
}

impl MemoryKeyring {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Keyring for MemoryKeyring {
    fn load_exchange_key(&self) -> Result<Option<ExchangeKeyPair>, KeyringError> {
        Ok(self.lock().exchange.clone())
    }
    fn save_exchange_key(&self, key: &ExchangeKeyPair) -> Result<(), KeyringError> {
        self.lock().exchange = Some(key.clone());
        Ok(())
    }
    fn load_group_keys(&self) -> Result<Option<GroupKeys>, KeyringError> {
        Ok(self.lock().keys.clone())
    }
    fn save_group_keys(&self, keys: &GroupKeys) -> Result<(), KeyringError> {
        self.lock().keys = Some(keys.clone());
        Ok(())
    }
    fn load_roster(&self) -> Result<Option<Roster>, KeyringError> {
        Ok(self.lock().roster.clone())
    }
    fn save_roster(&self, roster: &Roster) -> Result<(), KeyringError> {
        self.lock().roster = Some(roster.clone());
        Ok(())
    }
    fn load_quarantine(&self) -> Result<Vec<Quarantined>, KeyringError> {
        Ok(self.lock().quarantine.clone())
    }
    fn save_quarantine(&self, messages: &[Quarantined]) -> Result<(), KeyringError> {
        self.lock().quarantine = messages.to_vec();
        Ok(())
    }
    fn load_files(&self) -> Result<Vec<StoredFile>, KeyringError> {
        Ok(self.lock().files.clone())
    }
    fn save_files(&self, files: &[StoredFile]) -> Result<(), KeyringError> {
        self.lock().files = files.to_vec();
        Ok(())
    }
    fn load_uploads(&self) -> Result<Vec<PendingUpload>, KeyringError> {
        Ok(self.lock().uploads.clone())
    }
    fn save_uploads(&self, uploads: &[PendingUpload]) -> Result<(), KeyringError> {
        self.lock().uploads = uploads.to_vec();
        Ok(())
    }
    fn load_directory(&self) -> Result<DirectoryState, KeyringError> {
        Ok(self.lock().directory.clone())
    }
    fn save_directory(&self, state: &DirectoryState) -> Result<(), KeyringError> {
        self.lock().directory = state.clone();
        Ok(())
    }
    fn load_invites(&self) -> Result<Vec<InviteRecord>, KeyringError> {
        Ok(self.lock().invites.clone())
    }
    fn save_invites(&self, invites: &[InviteRecord]) -> Result<(), KeyringError> {
        self.lock().invites = invites.to_vec();
        Ok(())
    }
    fn load_pending_invite(&self) -> Result<Option<PendingInvite>, KeyringError> {
        Ok(self.lock().pending_invite.clone())
    }
    fn save_pending_invite(&self, pending: Option<&PendingInvite>) -> Result<(), KeyringError> {
        self.lock().pending_invite = pending.cloned();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn retired_epochs_are_kept_seven_days() {
        let t0 = datetime!(2026-10-10 08:00 UTC);
        let mut keys = GroupKeys::new(1, GroupKey::generate());
        keys.advance(2, GroupKey::generate(), Some(DeviceId::generate()), t0);
        assert_eq!(keys.current, 2);
        assert_eq!(keys.get(1).unwrap().retired_at, Some(t0));

        assert!(!keys.prune(t0 + Duration::days(6)));
        assert!(keys.get(1).is_some(), "encore utile pour les messages en retard");
        assert!(keys.prune(t0 + Duration::days(7)));
        assert!(keys.get(1).is_none());
        assert!(keys.current_key().is_some(), "l'époque courante n'expire jamais");
    }

    #[test]
    fn store_keyring_roundtrip_and_format() {
        let store = Arc::new(MemoryStore::new());
        let keyring = StoreKeyring::new(store.clone());
        assert!(keyring.load_exchange_key().unwrap().is_none());
        assert!(keyring.load_quarantine().unwrap().is_empty());

        let pair = ExchangeKeyPair::generate();
        keyring.save_exchange_key(&pair).unwrap();
        assert_eq!(keyring.load_exchange_key().unwrap().unwrap().public_key(), pair.public_key());
        let text = store.get(names::EXCHANGE_KEY).unwrap().unwrap();
        assert_eq!(&text[..], format!("{}\n", b64::encode(&pair.to_bytes())).as_bytes());

        let keys = GroupKeys::new(1, GroupKey::generate());
        keyring.save_group_keys(&keys).unwrap();
        assert_eq!(keyring.load_group_keys().unwrap(), Some(keys));

        assert_eq!(keyring.load_directory().unwrap(), DirectoryState::default());
        let directory = DirectoryState::with_anchor([3; 16]);
        keyring.save_directory(&directory).unwrap();
        assert_eq!(keyring.load_directory().unwrap(), directory);

        keyring.forget_group().unwrap();
        assert!(store.names().is_empty());
    }

    #[test]
    fn corrupted_value_is_reported() {
        let store = Arc::new(MemoryStore::new());
        store.put(names::GROUP, b"pas du json").unwrap();
        let err = StoreKeyring::new(store).load_group_keys().unwrap_err();
        assert!(err.0.contains("group.json est corrompu"), "{err}");
    }

    #[test]
    fn late_old_epoch_is_kept_retired() {
        let t0 = datetime!(2026-10-10 08:00 UTC);
        let mut keys = GroupKeys::new(3, GroupKey::generate());
        keys.insert_retired(2, GroupKey::generate(), None, t0);
        assert_eq!(keys.current, 3);
        assert_eq!(keys.epochs.iter().map(|e| e.epoch).collect::<Vec<_>>(), [2, 3]);
    }
}
