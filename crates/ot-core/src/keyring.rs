//! Stockage des clés et de l'état du groupe. Chaque appli fournit son
//! [`Keyring`] : fichiers `0600` pour la CLI, coffre de l'OS pour les applis.

use std::sync::Mutex;

use ot_crypto::{ExchangeKeyPair, GroupKey, Roster};
use ot_proto::{DeviceId, Envelope};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

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
    fn late_old_epoch_is_kept_retired() {
        let t0 = datetime!(2026-10-10 08:00 UTC);
        let mut keys = GroupKeys::new(3, GroupKey::generate());
        keys.insert_retired(2, GroupKey::generate(), None, t0);
        assert_eq!(keys.current, 3);
        assert_eq!(keys.epochs.iter().map(|e| e.epoch).collect::<Vec<_>>(), [2, 3]);
    }
}
