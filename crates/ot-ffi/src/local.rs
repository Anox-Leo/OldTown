//! État local de l'appareil, rangé dans le [`SecretStore`] de l'appli :
//! identité, configuration, appairage en cours, messages vus, historique.

use std::sync::{Arc, Mutex};

use ot_core::{
    DeviceId, Identity, KeyringError, PairUri, StoreKeyring, TlsRoots, Ulid, valid_device_name,
};
use ot_proto::b64;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{DeviceState, Group, OtError, Received, SecretStore, on_runtime};

/// Configuration : URL du relais, nom et identifiant.
pub(crate) const DEVICE: &str = "device.json";
/// Graine Ed25519, en base64 (même format que `device.key` de la CLI).
pub(crate) const DEVICE_KEY: &str = "device.key";
/// Appairage par code en attente de la réponse du parrain (contient le code).
pub(crate) const PENDING_JOIN: &str = "pending_join.json";
/// Derniers `msg_id` vus.
pub(crate) const SEEN: &str = "seen.json";
/// Derniers éléments reçus.
pub(crate) const HISTORY: &str = "history.json";

/// Taille de l'historique gardé (plan de l'étape 3).
pub const HISTORY_LEN: usize = 50;

/// Sérialise les lectures-écritures de l'historique (événements et écran).
static HISTORY_LOCK: Mutex<()> = Mutex::new(());

/// [`ot_core::SecretStore`] au-dessus du stockage de l'appli.
pub(crate) struct Bridge(pub Arc<dyn SecretStore>);

impl ot_core::SecretStore for Bridge {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError> {
        self.0.get(name.to_owned()).map(|v| v.map(Zeroizing::new)).map_err(store_error)
    }
    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError> {
        self.0.put(name.to_owned(), value.to_vec()).map_err(store_error)
    }
    fn delete(&self, name: &str) -> Result<(), KeyringError> {
        self.0.delete(name.to_owned()).map_err(store_error)
    }
}

fn store_error(e: crate::StoreError) -> KeyringError {
    KeyringError(e.to_string())
}

/// Accès typé au stockage de l'appli.
#[derive(Clone)]
pub(crate) struct Local {
    store: Arc<Bridge>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingJoin {
    pub code: String,
    pub sponsor: DeviceId,
    pub name: String,
}

impl Local {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store: Arc::new(Bridge(store)) }
    }

    pub fn keyring(&self) -> StoreKeyring {
        StoreKeyring::new(self.store.clone())
    }

    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, OtError> {
        Ok(ot_core::SecretStore::get(&*self.store, name)?)
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), OtError> {
        Ok(ot_core::SecretStore::put(&*self.store, name, value)?)
    }

    fn delete(&self, name: &str) -> Result<(), OtError> {
        Ok(ot_core::SecretStore::delete(&*self.store, name)?)
    }

    fn read_json<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, OtError> {
        let Some(bytes) = self.get(name)? else { return Ok(None) };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| OtError::Internal { message: format!("{name} est corrompu : {e}") })
    }

    fn write_json<T: Serialize + ?Sized>(&self, name: &str, value: &T) -> Result<(), OtError> {
        let bytes = Zeroizing::new(
            serde_json::to_vec(value)
                .map_err(|e| OtError::Internal { message: format!("{name} : {e}") })?,
        );
        self.put(name, &bytes)
    }

    pub fn device(&self) -> Result<Option<DeviceState>, OtError> {
        self.read_json(DEVICE)
    }

    pub fn save_device(&self, device: &DeviceState) -> Result<(), OtError> {
        self.write_json(DEVICE, device)
    }

    pub fn identity(&self) -> Result<Identity, OtError> {
        let bytes = self.get(DEVICE_KEY)?.ok_or_else(|| OtError::Internal {
            message: "clé de l'appareil absente : réappairer le téléphone".into(),
        })?;
        let text = Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
        let seed =
            Zeroizing::new(b64::decode_array::<32>(text.trim()).ok_or_else(|| {
                OtError::Internal { message: format!("{DEVICE_KEY} est corrompu") }
            })?);
        Ok(Identity::from_bytes(&seed))
    }

    pub fn save_identity(&self, identity: &Identity) -> Result<(), OtError> {
        let seed = Zeroizing::new(identity.to_bytes());
        let text = Zeroizing::new(format!("{}\n", b64::encode(&*seed)));
        self.put(DEVICE_KEY, text.as_bytes())
    }

    pub fn pending_join(&self) -> Result<Option<PendingJoin>, OtError> {
        self.read_json(PENDING_JOIN)
    }

    pub fn save_pending_join(&self, pending: &PendingJoin) -> Result<(), OtError> {
        self.write_json(PENDING_JOIN, pending)
    }

    pub fn clear_pending_join(&self) -> Result<(), OtError> {
        self.delete(PENDING_JOIN)
    }

    /// `msg_id` vus ; un état illisible donne une liste vide (au pire un
    /// doublon affiché).
    pub fn seen(&self) -> Vec<Ulid> {
        let ids: Vec<String> = self.read_json(SEEN).ok().flatten().unwrap_or_default();
        ids.iter().filter_map(|s| Ulid::from_string(s).ok()).collect()
    }

    pub fn save_seen<'a>(&self, seen: impl Iterator<Item = &'a Ulid>) -> Result<(), OtError> {
        let ids: Vec<String> = seen.map(Ulid::to_string).collect();
        self.write_json(SEEN, &ids)
    }

    /// Derniers éléments reçus, du plus récent au plus ancien.
    pub fn history(&self) -> Result<Vec<Received>, OtError> {
        let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        Ok(self.read_json(HISTORY)?.unwrap_or_default())
    }

    /// Ajoute un élément en tête ; seuls les [`HISTORY_LEN`] derniers restent.
    pub fn push_history(&self, item: Received) -> Result<(), OtError> {
        let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut items: Vec<Received> = self.read_json(HISTORY)?.unwrap_or_default();
        items.retain(|i| i.msg_id != item.msg_id);
        items.insert(0, item);
        items.truncate(HISTORY_LEN);
        self.write_json(HISTORY, &items)
    }

    pub fn clear_history(&self) -> Result<(), OtError> {
        let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.delete(HISTORY)
    }

    /// Efface tout : identité, configuration, groupe, état.
    pub fn forget(&self) -> Result<(), OtError> {
        self.keyring().forget_group()?;
        for name in [DEVICE, DEVICE_KEY, PENDING_JOIN, SEEN, HISTORY] {
            self.delete(name)?;
        }
        Ok(())
    }
}

/// URL du relais acceptée : `wss://` (ou `ws://` pour les essais), avec un hôte.
fn check_server_url(url: &str) -> Result<String, OtError> {
    let url = url.trim().trim_end_matches('/');
    let rest =
        url.strip_prefix("wss://").or_else(|| url.strip_prefix("ws://")).ok_or_else(|| {
            OtError::Invalid {
                message: "l'URL du relais commence par wss:// (ex. wss://relais:7878)".into(),
            }
        })?;
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return Err(OtError::Invalid { message: "URL du relais invalide".into() });
    }
    Ok(url.to_owned())
}

/// Appareil enregistré, `None` avant l'appairage.
#[uniffi::export]
pub fn device_state(store: Arc<dyn SecretStore>) -> Result<Option<DeviceState>, OtError> {
    Local::new(store).device()
}

/// Groupe enregistré, sans connexion (feuille de partage, écran de démarrage).
#[uniffi::export]
pub fn group_snapshot(store: Arc<dyn SecretStore>) -> Result<Option<Group>, OtError> {
    let local = Local::new(store);
    let Some(device) = local.device()? else { return Ok(None) };
    Ok(ot_core::group_info(&local.keyring())?.map(|g| Group::from_info(g, &device.device_id)))
}

/// Rejoint un groupe avec un code d'appairage (QR « rejoindre » ou saisie) :
/// crée l'identité, s'enregistre auprès du relais, et laisse la demande au
/// parrain à [`crate::OtClient`], qui l'envoie à son démarrage. Le groupe
/// arrive ensuite (`Event::GroupChanged`).
#[uniffi::export]
pub async fn join(
    store: Arc<dyn SecretStore>,
    server_url: String,
    code: String,
    name: String,
) -> Result<DeviceState, OtError> {
    on_runtime(async move {
        let local = Local::new(store);
        if local.device()?.is_some() {
            return Err(OtError::Invalid {
                message: "ce téléphone est déjà enregistré : l'oublier d'abord".into(),
            });
        }
        let name = name.trim().to_owned();
        if !valid_device_name(&name) {
            return Err(OtError::Invalid {
                message: "nom invalide : 1 à 64 caractères imprimables".into(),
            });
        }
        let server_url = check_server_url(&server_url)?;
        // Valide le code avant tout contact avec le relais.
        let uri = PairUri::parse(&PairUri { server_url, code }.encode())?;
        let identity = Identity::generate();
        let redeemed = ot_core::redeem_pairing(
            &uri.server_url,
            &TlsRoots::default(),
            &identity,
            &name,
            &uri.code,
        )
        .await?;
        local.forget()?;
        local.save_identity(&identity)?;
        let device = DeviceState {
            device_id: redeemed.device_id.to_string(),
            name: name.clone(),
            server_url: uri.server_url,
        };
        local.save_device(&device)?;
        local.save_pending_join(&PendingJoin {
            code: uri.code,
            sponsor: redeemed.sponsor,
            name,
        })?;
        Ok(device)
    })
    .await
}

/// Oublie l'appareil : il faudra le réappairer. Le client doit être arrêté.
#[uniffi::export]
pub fn forget_device(store: Arc<dyn SecretStore>) -> Result<(), OtError> {
    Local::new(store).forget()
}

/// Change l'URL du relais (passage de Tailscale à un nom public, note 0015).
/// Prise en compte au prochain démarrage du client.
#[uniffi::export]
pub fn set_server_url(store: Arc<dyn SecretStore>, url: String) -> Result<DeviceState, OtError> {
    let local = Local::new(store);
    let mut device = local
        .device()?
        .ok_or_else(|| OtError::NotInGroup { message: "appareil pas encore appairé".into() })?;
    device.server_url = check_server_url(&url)?;
    local.save_device(&device)?;
    Ok(device)
}

/// Derniers éléments reçus, du plus récent au plus ancien.
#[uniffi::export]
pub fn history(store: Arc<dyn SecretStore>) -> Result<Vec<Received>, OtError> {
    Local::new(store).history()
}

#[uniffi::export]
pub fn clear_history(store: Arc<dyn SecretStore>) -> Result<(), OtError> {
    Local::new(store).clear_history()
}

/// Noms gardés à côté du keyring : pour les tests, la liste complète.
#[cfg(test)]
pub(crate) const ALL_NAMES: &[&str] = &[
    DEVICE,
    DEVICE_KEY,
    PENDING_JOIN,
    SEEN,
    HISTORY,
    ot_core::names::EXCHANGE_KEY,
    ot_core::names::GROUP,
    ot_core::names::ROSTER,
    ot_core::names::QUARANTINE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_urls() {
        assert_eq!(check_server_url(" wss://relais:7878/ ").unwrap(), "wss://relais:7878");
        assert_eq!(check_server_url("ws://127.0.0.1:7878").unwrap(), "ws://127.0.0.1:7878");
        for bad in ["relais:7878", "https://relais", "wss://", "wss://a b"] {
            assert!(check_server_url(bad).is_err(), "{bad}");
        }
    }
}
