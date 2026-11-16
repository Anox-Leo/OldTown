//! Stockage des clés et de l'état du groupe. Chaque appli fournit un
//! [`SecretStore`] (fichiers `0600` pour la CLI, Keystore pour Android) ;
//! [`StoreKeyring`] en tire le [`Keyring`] du cœur.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ot_crypto::{ExchangeKeyPair, GroupKey, Roster};
use ot_proto::{DeviceId, Envelope, b64};

use crate::files::{PendingUpload, StoredFile};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use zeroize::Zeroizing;

/// Durée pendant laquelle une clé de groupe remplacée reste utilisable pour
/// lire les messages en retard (durée de vie des messages sur le relais).
pub const RETIRED_KEY_LIFETIME: Duration = Duration::days(7);

/// Nombre maximal de messages mis de côté en attendant leur clé.
pub const MAX_QUARANTINE: usize = 100;

/// Erreur d'un [`Keyring`] : message lisible, sans secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("stockage des clés : {0}")]
pub struct KeyringError(pub String);

/// Accès aux secrets et à l'état du groupe de l'appareil. Les méthodes sont
/// synchrones et rapides (fichier local ou coffre de l'OS).
pub trait Keyring: Send + Sync {
    /// Paire X25519 de l'appareil, `None` si elle n'existe pas encore.
    fn load_exchange_key(&self) -> Result<Option<ExchangeKeyPair>, KeyringError>;
    fn save_exchange_key(&self, key: &ExchangeKeyPair) -> Result<(), KeyringError>;
    /// Clés de groupe par époque ; `None` tant que l'appareil n'a pas de groupe.
    fn load_group_keys(&self) -> Result<Option<GroupKeys>, KeyringError>;
    fn save_group_keys(&self, keys: &GroupKeys) -> Result<(), KeyringError>;
    fn load_roster(&self) -> Result<Option<Roster>, KeyringError>;
    fn save_roster(&self, roster: &Roster) -> Result<(), KeyringError>;
    /// Messages d'époque inconnue, mis de côté (au plus [`MAX_QUARANTINE`]).
    fn load_quarantine(&self) -> Result<Vec<Quarantined>, KeyringError>;
    fn save_quarantine(&self, messages: &[Quarantined]) -> Result<(), KeyringError>;
    /// Fichiers annoncés, avec leur clé (étape 5).
    fn load_files(&self) -> Result<Vec<StoredFile>, KeyringError>;
    fn save_files(&self, files: &[StoredFile]) -> Result<(), KeyringError>;
    /// Dépôts de fichiers en cours, avec leur clé (étape 5).
    fn load_uploads(&self) -> Result<Vec<PendingUpload>, KeyringError>;
    fn save_uploads(&self, uploads: &[PendingUpload]) -> Result<(), KeyringError>;
}

/// Octets nommés, rangés à l'abri par l'appli. Les noms sont courts, en ASCII
/// (`group.json`, `exchange.key`…). Les méthodes sont synchrones et rapides.
pub trait SecretStore: Send + Sync {
    /// Valeur enregistrée sous `name`, `None` si elle n'existe pas.
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError>;
    /// Remplace la valeur de `name` (écriture atomique).
    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError>;
    /// Efface `name` ; sans effet s'il n'existe pas.
    fn delete(&self, name: &str) -> Result<(), KeyringError>;
}

/// Noms utilisés par [`StoreKeyring`].
pub mod names {
    /// Clé X25519 secrète, en base64 suivi d'un saut de ligne.
    pub const EXCHANGE_KEY: &str = "exchange.key";
    /// [`super::GroupKeys`] en JSON.
    pub const GROUP: &str = "group.json";
    /// Roster en JSON.
    pub const ROSTER: &str = "roster.json";
    /// Messages mis de côté, en JSON.
    pub const QUARANTINE: &str = "quarantine.json";
    /// Fichiers annoncés, avec leur clé, en JSON (étape 5).
    pub const FILES: &str = "files.json";
    /// Dépôts de fichiers en cours, avec leur clé, en JSON (étape 5).
    pub const UPLOADS: &str = "uploads.json";
}

/// [`Keyring`] rangé dans un [`SecretStore`]. Le format des valeurs est celui
/// des fichiers de la CLI depuis l'étape 2 : une installation existante se
/// relit sans conversion.
#[derive(Clone)]
pub struct StoreKeyring {
    store: Arc<dyn SecretStore>,
}

impl StoreKeyring {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// Oublie le groupe et la clé d'échange (nouvelle identité d'appareil).
    pub fn forget_group(&self) -> Result<(), KeyringError> {
        for name in [
            names::EXCHANGE_KEY,
            names::GROUP,
            names::ROSTER,
            names::QUARANTINE,
            names::FILES,
            names::UPLOADS,
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
}

/// [`SecretStore`] en mémoire, pour les tests.
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

    /// Noms présents, triés.
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

/// Clés de groupe connues, de la plus ancienne à la plus récente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupKeys {
    /// Époque utilisée pour chiffrer.
    pub current: u32,
    pub epochs: Vec<EpochEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochEntry {
    pub epoch: u32,
    pub key: GroupKey,
    /// Appareil qui a tiré la clé par rotation ; `None` pour une clé créée
    /// par `group init` ou reçue à l'appairage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<DeviceId>,
    /// Date de remplacement par une époque plus récente.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub retired_at: Option<OffsetDateTime>,
}

impl GroupKeys {
    /// Premier groupe : une seule époque.
    pub fn new(epoch: u32, key: GroupKey) -> Self {
        Self {
            current: epoch,
            epochs: vec![EpochEntry { epoch, key, author: None, retired_at: None }],
        }
    }

    pub fn get(&self, epoch: u32) -> Option<&EpochEntry> {
        self.epochs.iter().find(|e| e.epoch == epoch)
    }

    /// Clé de l'époque courante.
    pub fn current_key(&self) -> Option<&GroupKey> {
        self.get(self.current).map(|e| &e.key)
    }

    /// Passe à une nouvelle époque courante ; l'ancienne est retirée à `now`.
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

    /// Garde une clé d'époque ancienne, reçue en retard, déjà retirée.
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

    /// Oublie les clés retirées depuis plus de [`RETIRED_KEY_LIFETIME`].
    /// Renvoie `true` si quelque chose a changé.
    pub fn prune(&mut self, now: OffsetDateTime) -> bool {
        let before = self.epochs.len();
        self.epochs.retain(|e| e.retired_at.is_none_or(|t| now - t < RETIRED_KEY_LIFETIME));
        self.epochs.len() != before
    }
}

/// Message mis de côté : son époque n'est pas encore connue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quarantined {
    /// `seq` du relais à la livraison (déjà acquitté).
    pub seq: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
    pub env: Envelope,
}

/// [`Keyring`] en mémoire, pour les tests.
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
        // Même format que les fichiers de la CLI à l'étape 2.
        let text = store.get(names::EXCHANGE_KEY).unwrap().unwrap();
        assert_eq!(&text[..], format!("{}\n", b64::encode(&pair.to_bytes())).as_bytes());

        let keys = GroupKeys::new(1, GroupKey::generate());
        keyring.save_group_keys(&keys).unwrap();
        assert_eq!(keyring.load_group_keys().unwrap(), Some(keys));

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
