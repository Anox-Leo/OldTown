//! [`OtClient`] : la connexion au relais, tenue par le service de premier
//! plan de l'appli (note 0009).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ot_core::{BlobId, Client, ClientConfig, Dedup, DeviceId, Events, OutgoingFile, StoreKeyring};
use tokio::sync::mpsc;

use crate::files::{FileItem, FileSink, FileSource, Listener, Sink, Source, TransferListener};
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
    local: Local,
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
        let forward =
            Forwarder { client: client.clone(), local: local.clone(), keyring, seen, names, tx };
        RUNTIME.spawn(forward.run(core_events));
        Ok(Arc::new(Self { client, local, device, events: tokio::sync::Mutex::new(rx) }))
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
            let to = parse_ids(&to)?;
            if text.trim().is_empty() {
                return Err(OtError::Invalid { message: "texte vide".into() });
            }
            let accepted = client.send(&ot_core::classify(&text), to).await?;
            Ok(accepted.msg_id.to_string())
        })
        .await
    }

    /// Envoie un fichier (étape 5) : dépôt chiffré par morceaux, puis
    /// annonce aux destinataires (vide : tous les autres). `source_id`
    /// identifie la source (son URI) : relancer l'envoi de la même source
    /// reprend là où il s'est arrêté. Renvoie le `msg_id` de l'annonce.
    pub async fn send_file(
        &self,
        source: Arc<dyn FileSource>,
        source_id: String,
        name: String,
        mime: String,
        to: Vec<String>,
        listener: Arc<dyn TransferListener>,
    ) -> Result<String, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let to = parse_ids(&to)?;
            let file = OutgoingFile { source_id, name, mime, to };
            let accepted = client
                .send_file(file, Arc::new(Source(source)), Arc::new(Listener(listener)))
                .await?;
            Ok(accepted.msg_id.to_string())
        })
        .await
    }

    /// Abandonne l'envoi en cours de cette source (libère la place sur le
    /// relais).
    pub async fn cancel_upload(&self, source_id: String) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.cancel_upload(&source_id).await?) }).await
    }

    /// Télécharge le fichier annoncé par l'élément `msg_id` dans `sink`,
    /// vérifie son SHA-256 et note son URI dans l'historique. Renvoie
    /// l'élément mis à jour.
    pub async fn download(
        &self,
        msg_id: String,
        sink: Arc<dyn FileSink>,
        listener: Arc<dyn TransferListener>,
    ) -> Result<FileItem, OtError> {
        let (client, local) = (self.client.clone(), self.local.clone());
        on_runtime(async move {
            let item = local
                .history()?
                .into_iter()
                .find(|i| i.msg_id == msg_id)
                .and_then(|i| i.file)
                .ok_or_else(|| OtError::Invalid { message: "fichier inconnu".into() })?;
            let blob: BlobId =
                item.blob_id.parse().map_err(|e| OtError::Internal { message: format!("{e}") })?;
            let sink = Arc::new(Sink { inner: sink, uri: std::sync::Mutex::new(None) });
            client.download(&blob, sink.clone(), Arc::new(Listener(listener))).await?;
            let uri =
                sink.uri.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
            local.set_file_uri(&msg_id, &uri)?;
            Ok(FileItem { saved_uri: Some(uri), ..item })
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
        on_runtime(async move { Ok(client.rotate(parse_ids(&exclude)?).await?) }).await
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

/// Identifiants d'appareils donnés par Kotlin.
fn parse_ids(ids: &[String]) -> Result<Vec<DeviceId>, OtError> {
    ids.iter()
        .map(|id| id.parse::<DeviceId>().map_err(|e| OtError::Invalid { message: e.to_string() }))
        .collect()
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
                    file: None,
                };
                self.remember(m.msg_id, &received, &mut out);
                Event::Message { received }
            }
            E::File(f) => {
                let f = f.file;
                let received = Received {
                    msg_id: f.msg_id.to_string(),
                    from_name: self.name_of(&f.from),
                    from_id: f.from.to_string(),
                    created_at_ms: millis(f.created_at),
                    kind: crate::ContentKind::File,
                    body: f.name.clone(),
                    sensitive: false,
                    file: Some(FileItem::from_info(&f)),
                };
                self.remember(f.msg_id, &received, &mut out);
                Event::Message { received }
            }
        };
        out.insert(0, ev);
        out
    }

    /// Range un élément reçu : vu, et dans l'historique.
    fn remember(&mut self, msg_id: ot_core::Ulid, received: &Received, out: &mut Vec<Event>) {
        self.seen.insert(msg_id);
        let saved = self
            .local
            .save_seen(self.seen.iter())
            .and_then(|()| self.local.push_history(received.clone()));
        if let Err(e) = saved {
            out.push(Event::StorageError { message: e.to_string() });
        }
    }
}
