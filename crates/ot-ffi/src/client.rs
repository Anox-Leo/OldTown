//! [`OtClient`] : la connexion au relais, tenue par le service de premier
//! plan de l'appli (note 0009).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ot_core::{Client, ClientConfig, Dedup, DeviceId, Events, StoreKeyring};
use tokio::sync::mpsc;

use crate::local::{Local, PendingJoin};
use crate::types::millis;
use crate::{
    DeviceState, Event, Group, Invitation, OtError, RUNTIME, Received, RelayDevice, SecretStore,
    Sponsored, on_runtime,
};

/// Client du relais : se connecte et se reconnecte seul tant qu'il existe.
#[derive(uniffi::Object)]
pub struct OtClient {
    client: Client,
    device: DeviceState,
    events: tokio::sync::Mutex<mpsc::Receiver<Event>>,
}

#[uniffi::export]
impl OtClient {
    /// Démarre le client de l'appareil enregistré dans `store`. Si un
    /// appairage par code est en cours, envoie la demande au parrain.
    #[uniffi::constructor]
    pub fn start(store: Arc<dyn SecretStore>) -> Result<Arc<Self>, OtError> {
        let local = Local::new(store);
        let device = local.device()?.ok_or_else(|| OtError::NotInGroup {
            message: "appareil pas encore appairé".into(),
        })?;
        let device_id: DeviceId =
            device.device_id.parse().map_err(|e| OtError::Internal { message: format!("{e}") })?;
        let keyring = local.keyring();
        let mut config = ClientConfig::new(
            device.server_url.clone(),
            device_id,
            local.identity()?,
            Arc::new(keyring.clone()),
        );
        config.seen = local.seen();
        let seen = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, config.seen.clone());

        let _guard = RUNTIME.enter();
        let (client, core_events) = Client::start(config);
        let (tx, rx) = mpsc::channel(256);
        let names = Arc::new(Mutex::new(names(&keyring)));
        if let Some(pending) = local.pending_join()? {
            RUNTIME.spawn(send_join(client.clone(), pending, tx.clone()));
        }
        let forward = Forwarder { client: client.clone(), local, keyring, seen, names, tx };
        RUNTIME.spawn(forward.run(core_events));
        Ok(Arc::new(Self { client, device, events: tokio::sync::Mutex::new(rx) }))
    }

    pub fn device(&self) -> DeviceState {
        self.device.clone()
    }

    /// Prochain événement ; `None` une fois le client arrêté.
    pub async fn next_event(&self) -> Option<Event> {
        self.events.lock().await.recv().await
    }

    /// Envoie un texte (un lien s'il n'est fait que d'une URL) à `to`, des
    /// identifiants de membres ; vide : tous les autres appareils. Hors
    /// ligne, l'envoi part à la reconnexion. Renvoie le `msg_id`.
    pub async fn send_text(&self, text: String, to: Vec<String>) -> Result<String, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let to = to
                .iter()
                .map(|id| {
                    id.parse::<DeviceId>().map_err(|e| OtError::Invalid { message: e.to_string() })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if text.trim().is_empty() {
                return Err(OtError::Invalid { message: "texte vide".into() });
            }
            let accepted = client.send(&ot_core::classify(&text), to).await?;
            Ok(accepted.msg_id.to_string())
        })
        .await
    }

    /// Groupe vu par ce téléphone ; `None` avant la fin de l'appairage.
    pub async fn group(&self) -> Result<Option<Group>, OtError> {
        let (client, me) = (self.client.clone(), self.device.device_id.clone());
        on_runtime(async move { Ok(client.group().await?.map(|g| Group::from_info(g, &me))) }).await
    }

    /// Appareils connus du relais, avec leur présence (connexion requise).
    pub async fn devices(&self) -> Result<Vec<RelayDevice>, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let group = client.group().await?;
            let list = client.list_devices().await?;
            Ok(list
                .into_iter()
                .map(|d| RelayDevice {
                    member: group.as_ref().is_some_and(|g| g.member(&d.id).is_some()),
                    device_id: d.id.to_string(),
                    name: d.name,
                    online: d.online,
                })
                .collect())
        })
        .await
    }

    /// Code d'appairage pour un nouvel appareil (valable 10 minutes, tant
    /// que ce client tourne).
    pub async fn pair_new(&self) -> Result<Invitation, OtError> {
        let (client, server_url) = (self.client.clone(), self.device.server_url.clone());
        on_runtime(async move {
            let invitation = client.pair_create().await?;
            let uri = ot_core::PairUri { server_url, code: invitation.code.clone() }.encode();
            Ok(Invitation {
                code: invitation.code,
                uri,
                expires_at_ms: millis(invitation.expires_at),
            })
        })
        .await
    }

    /// Parraine l'appareil dont `qr` est le QR inversé scanné.
    pub async fn sponsor(&self, qr: String) -> Result<Sponsored, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let (id, name) = client.sponsor(&qr).await?;
            Ok(Sponsored { device_id: id.to_string(), name })
        })
        .await
    }

    /// Nouvelle clé de groupe, sans les membres `exclude` (téléphone perdu,
    /// ordinateur revendu…). Renvoie la nouvelle époque.
    pub async fn rotate(&self, exclude: Vec<String>) -> Result<u32, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let exclude = exclude
                .iter()
                .map(|id| {
                    id.parse::<DeviceId>().map_err(|e| OtError::Invalid { message: e.to_string() })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(client.rotate(exclude).await?)
        })
        .await
    }

    /// Le réseau a changé : reconnexion immédiate.
    pub fn reconnect_now(&self) {
        self.client.reconnect_now();
    }

    /// Ferme proprement la connexion ; le client ne sert plus ensuite.
    pub async fn shutdown(&self) {
        let client = self.client.clone();
        let _ = on_runtime(async move {
            client.shutdown().await;
            Ok(())
        })
        .await;
    }
}

/// Noms des membres, pour afficher l'expéditeur.
fn names(keyring: &StoreKeyring) -> HashMap<DeviceId, String> {
    ot_core::group_info(keyring)
        .ok()
        .flatten()
        .map(|g| g.members.into_iter().map(|m| (m.device_id, m.name)).collect())
        .unwrap_or_default()
}

async fn send_join(client: Client, pending: PendingJoin, tx: mpsc::Sender<Event>) {
    if let Err(e) = client.pair_join(&pending.code, pending.sponsor, &pending.name).await {
        let _ = tx.send(Event::JoinFailed { message: e.to_string() }).await;
    }
}

/// Traduit les événements du cœur, range les éléments reçus (vus,
/// historique) et tient les noms des membres à jour.
struct Forwarder {
    client: Client,
    local: Local,
    keyring: StoreKeyring,
    seen: Dedup,
    names: Arc<Mutex<HashMap<DeviceId, String>>>,
    tx: mpsc::Sender<Event>,
}

impl Forwarder {
    fn name_of(&self, id: &DeviceId) -> String {
        let names = self.names.lock().unwrap_or_else(|e| e.into_inner());
        names.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    async fn run(mut self, mut events: Events) {
        while let Some(event) = events.next().await {
            let out = self.translate(event).await;
            for ev in out {
                if self.tx.send(ev).await.is_err() {
                    return;
                }
            }
        }
    }

    async fn translate(&mut self, event: ot_core::Event) -> Vec<Event> {
        use ot_core::Event as E;
        let mut out = Vec::new();
        let ev = match event {
            E::Connected { pending } => Event::Connected { pending },
            E::Disconnected { reason } => Event::Disconnected { reason },
            E::AuthFailed { message } => Event::AuthFailed { message },
            E::Rejected { from, reason, .. } => {
                Event::Rejected { from_name: self.name_of(&from), reason }
            }
            E::GroupChanged { epoch } => {
                *self.names.lock().unwrap_or_else(|e| e.into_inner()) = names(&self.keyring);
                // Le groupe est là : l'appairage par code est terminé.
                if let Ok(Some(_)) = self.client.group().await
                    && let Err(e) = self.local.clear_pending_join()
                {
                    out.push(Event::StorageError { message: e.to_string() });
                }
                Event::GroupChanged { epoch }
            }
            E::PairingCompleted { device_id, name } => {
                Event::PairingCompleted { device_id: device_id.to_string(), name }
            }
            E::KeyringError { message } => Event::StorageError { message },
            E::Message(m) => {
                let received = Received {
                    msg_id: m.msg_id.to_string(),
                    from_name: self.name_of(&m.from),
                    from_id: m.from.to_string(),
                    created_at_ms: millis(m.created_at),
                    kind: m.content.kind.into(),
                    body: m.content.body,
                    sensitive: false,
                };
                self.seen.insert(m.msg_id);
                let saved = self
                    .local
                    .save_seen(self.seen.iter())
                    .and_then(|()| self.local.push_history(received.clone()));
                if let Err(e) = saved {
                    out.push(Event::StorageError { message: e.to_string() });
                }
                Event::Message { received }
            }
        };
        out.insert(0, ev);
        out
    }
}
