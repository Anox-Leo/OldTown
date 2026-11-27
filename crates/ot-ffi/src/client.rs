use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ot_core::{
    BlobId, Client, ClientConfig, Dedup, DeviceId, Events, OutgoingFile, StoreKeyring, Target,
    UserId,
};
use tokio::sync::mpsc;

use crate::files::{FileItem, FileSink, FileSource, Listener, Sink, Source, TransferListener};
use crate::local::{Local, PendingJoin};
use crate::types::millis;
use crate::{
    AdminRoot, AdminUserItem, DeviceState, Event, Group, Invitation, OtError, PersonInvite,
    PersonItem, RUNTIME, Received, RelayDevice, SecretStore, ServiceAdded, Sponsored, on_runtime,
};

#[derive(uniffi::Object)]
pub struct OtClient {
    client: Client,
    local: Local,
    device: DeviceState,
    events: tokio::sync::Mutex<mpsc::Receiver<Event>>,
    events_tx: mpsc::WeakSender<Event>,
}

#[uniffi::export]
impl OtClient {
    #[uniffi::constructor]
    pub fn start(store: Arc<dyn SecretStore>, features: Vec<String>) -> Result<Arc<Self>, OtError> {
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
        config.kind = Some(ot_core::DeviceKind::Phone);
        config.features = features
            .iter()
            .map(|f| f.parse().map_err(|e| OtError::Invalid { message: format!("{e}") }))
            .collect::<Result<_, _>>()?;
        let seen = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, config.seen.clone());

        let _guard = RUNTIME.enter();
        let (client, core_events) = Client::start(config);
        let (tx, rx) = mpsc::channel(256);
        let names = Arc::new(Mutex::new(names(&keyring)));
        if let Some(pending) = local.pending_join()? {
            RUNTIME.spawn(send_join(client.clone(), pending, tx.clone()));
        }
        let events_tx = tx.downgrade();
        let forward =
            Forwarder { client: client.clone(), local: local.clone(), keyring, seen, names, tx };
        RUNTIME.spawn(forward.run(core_events));
        Ok(Arc::new(Self { client, local, device, events: tokio::sync::Mutex::new(rx), events_tx }))
    }

    pub fn device(&self) -> DeviceState {
        self.device.clone()
    }

    pub async fn next_event(&self) -> Option<Event> {
        self.events.lock().await.recv().await
    }

    pub async fn send_text(&self, text: String, to: Vec<String>) -> Result<String, OtError> {
        let target = parse_ids(&to).map(Target::MyDevices);
        self.send_text_target(text, target).await
    }

    pub async fn send_text_to_user(&self, text: String, user: String) -> Result<String, OtError> {
        let target = parse_user(&user).map(|user| Target::Person { user });
        self.send_text_target(text, target).await
    }

    pub async fn send_file_to_user(
        &self,
        source: Arc<dyn FileSource>,
        source_id: String,
        name: String,
        mime: String,
        user: String,
        listener: Arc<dyn TransferListener>,
    ) -> Result<String, OtError> {
        let user = parse_user(&user)?;
        let file = OutgoingFile { source_id, name, mime, to: Vec::new(), to_user: Some(user) };
        self.send_outgoing_file(file, source, listener).await
    }

    pub async fn admin_init(&self) -> Result<String, OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.admin_init().await?.to_string()) }).await
    }

    pub async fn admin_root(&self, create: bool) -> Result<AdminRoot, OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.admin_root(create).await?.into()) }).await
    }

    pub async fn invite(&self, name: String, links: Vec<String>) -> Result<PersonInvite, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let links = parse_users(&links)?;
            Ok(person_invite(client.invite(&name, links).await?))
        })
        .await
    }

    pub async fn reinvite(&self, user: String) -> Result<PersonInvite, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let user = parse_user(&user)?;
            Ok(person_invite(client.reinvite(&user).await?))
        })
        .await
    }

    pub async fn users(&self) -> Result<Vec<AdminUserItem>, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            Ok(client.users().await?.into_iter().map(AdminUserItem::from).collect())
        })
        .await
    }

    pub async fn link_set(&self, from: String, to: String, allowed: bool) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            Ok(client.link_set(&parse_user(&from)?, &parse_user(&to)?, allowed).await?)
        })
        .await
    }

    pub async fn revoke_user(&self, user: String) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.revoke_user(&parse_user(&user)?).await?) }).await
    }

    pub async fn rename_user(&self, user: String, name: String) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.rename_user(&parse_user(&user)?, &name).await?) }).await
    }

    pub async fn update_device(
        &self,
        device: String,
        kind: Option<String>,
        reachable: Option<bool>,
        default: Option<bool>,
    ) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let device = parse_device(&device)?;
            let kind = kind
                .map(|k| {
                    k.parse::<ot_core::proto::DeviceKind>().map_err(|()| OtError::Invalid {
                        message: format!("nature inconnue : {k}"),
                    })
                })
                .transpose()?;
            let settings = ot_core::DeviceSettings { kind, reachable, default };
            Ok(client.update_device(&device, settings).await?)
        })
        .await
    }

    pub async fn mark_read(
        &self,
        user_id: String,
        at_ms: i64,
    ) -> Result<HashMap<String, i64>, OtError> {
        let (client, local) = (self.client.clone(), self.local.clone());
        on_runtime(async move {
            let user = parse_user(&user_id)?;
            let before = local.opened().get(&user_id).copied().unwrap_or(0);
            let opened = local.mark_opened(&user_id, at_ms)?;
            if at_ms > before
                && let Ok(until) = u64::try_from(at_ms)
            {
                client.send_read(&user, until).await?;
            }
            Ok(opened)
        })
        .await
    }

    pub async fn remove_device(&self, device: String) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let device = parse_device(&device)?;
            Ok(client.remove_device(&device).await?)
        })
        .await
    }

    pub async fn mute_service(&self, service: String, muted: bool) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.mute_service(&parse_user(&service)?, muted).await?) })
            .await
    }

    pub async fn add_service_device(
        &self,
        service: String,
        qr: String,
    ) -> Result<ServiceAdded, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let added = client.add_service_device(&service, &qr).await?;
            Ok(ServiceAdded {
                user_id: added.user.to_string(),
                device_id: added.device_id.to_string(),
                device_name: added.device_name,
                created: added.created,
            })
        })
        .await
    }

    pub async fn people(&self) -> Result<Vec<PersonItem>, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            Ok(client.directory().await?.into_iter().map(PersonItem::from).collect())
        })
        .await
    }

    pub async fn send_file(
        &self,
        source: Arc<dyn FileSource>,
        source_id: String,
        name: String,
        mime: String,
        to: Vec<String>,
        listener: Arc<dyn TransferListener>,
    ) -> Result<String, OtError> {
        let file = OutgoingFile { source_id, name, mime, to: parse_ids(&to)?, to_user: None };
        self.send_outgoing_file(file, source, listener).await
    }

    pub async fn cancel_upload(&self, source_id: String) -> Result<(), OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.cancel_upload(&source_id).await?) }).await
    }

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

    pub async fn group(&self) -> Result<Option<Group>, OtError> {
        let (client, me) = (self.client.clone(), self.device.device_id.clone());
        on_runtime(async move { Ok(client.group().await?.map(|g| Group::from_info(g, &me))) }).await
    }

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

    pub async fn sponsor(&self, qr: String) -> Result<Sponsored, OtError> {
        let client = self.client.clone();
        on_runtime(async move {
            let (id, name) = client.sponsor(&qr).await?;
            Ok(Sponsored { device_id: id.to_string(), name })
        })
        .await
    }

    pub async fn rotate(&self, exclude: Vec<String>) -> Result<u32, OtError> {
        let client = self.client.clone();
        on_runtime(async move { Ok(client.rotate(parse_ids(&exclude)?).await?) }).await
    }

    pub fn reconnect_now(&self) {
        self.client.reconnect_now();
    }

    pub fn refresh_directory(&self) {
        self.client.refresh_directory();
    }

    pub async fn shutdown(&self) {
        let client = self.client.clone();
        let _ = on_runtime(async move {
            client.shutdown().await;
            Ok(())
        })
        .await;
    }
}

impl OtClient {
    async fn send_text_target(
        &self,
        text: String,
        target: Result<Target, OtError>,
    ) -> Result<String, OtError> {
        let (client, outbox) = (self.client.clone(), self.outbox());
        on_runtime(async move {
            let target = target?;
            if text.trim().is_empty() {
                return Err(OtError::Invalid { message: "texte vide".into() });
            }
            let to_user = match &target {
                Target::Person { user } => Some(user.to_string()),
                Target::MyDevices(_) => None,
            };
            let content = ot_core::classify(&text);
            let env = client.seal_to(target, &content).await?;
            let msg_id = env.msg_id.to_string();
            let item = outbox.item(
                msg_id.clone(),
                millis(env.created_at),
                content.kind.into(),
                content.body,
                &env.to,
            );
            outbox.add(Received { pending: true, to_user, ..item }).await;
            match client.send_envelope(env).await {
                Ok(_) => {
                    outbox.accepted(&msg_id).await;
                    Ok(msg_id)
                }
                Err(e) => {
                    outbox.failed(&msg_id).await;
                    Err(e.into())
                }
            }
        })
        .await
    }

    async fn send_outgoing_file(
        &self,
        file: OutgoingFile,
        source: Arc<dyn FileSource>,
        listener: Arc<dyn TransferListener>,
    ) -> Result<String, OtError> {
        let (client, outbox) = (self.client.clone(), self.outbox());
        on_runtime(async move {
            let to = file.to.clone();
            let (accepted, info) = client
                .send_file_info(file, Arc::new(Source(source)), Arc::new(Listener(listener)))
                .await?;
            let msg_id = accepted.msg_id.to_string();
            let item = outbox.item(
                msg_id.clone(),
                millis(info.created_at),
                crate::ContentKind::File,
                info.name.clone(),
                &to,
            );
            let item = Received { file: Some(FileItem::from_info(&info)), ..item }
                .with_origin(&info.origin);
            outbox.add(item).await;
            Ok(msg_id)
        })
        .await
    }

    fn outbox(&self) -> Outbox {
        Outbox {
            local: self.local.clone(),
            tx: self.events_tx.clone(),
            me_id: self.device.device_id.clone(),
            me_name: self.device.name.clone(),
        }
    }
}

struct Outbox {
    local: Local,
    tx: mpsc::WeakSender<Event>,
    me_id: String,
    me_name: String,
}

impl Outbox {
    fn item(
        &self,
        msg_id: String,
        created_at_ms: i64,
        kind: crate::ContentKind,
        body: String,
        to: &[DeviceId],
    ) -> Received {
        Received {
            msg_id,
            from_id: self.me_id.clone(),
            from_name: self.me_name.clone(),
            created_at_ms,
            kind,
            body,
            file: None,
            to: to.iter().map(DeviceId::to_string).collect(),
            outgoing: true,
            pending: false,
            from_user: None,
            from_service: false,
            to_user: None,
            to_device: None,
            auto_fill: false,
        }
    }

    async fn emit(&self, event: Event) {
        if let Some(tx) = self.tx.upgrade() {
            let _ = tx.send(event).await;
        }
    }

    async fn add(&self, item: Received) {
        match self.local.push_history(item.clone()) {
            Ok(()) => self.emit(Event::Sent { item }).await,
            Err(e) => self.emit(Event::StorageError { message: e.to_string() }).await,
        }
    }

    async fn accepted(&self, msg_id: &str) {
        match self.local.mark_sent(msg_id) {
            Ok(Some(item)) => self.emit(Event::Sent { item }).await,
            Ok(None) => {}
            Err(e) => self.emit(Event::StorageError { message: e.to_string() }).await,
        }
    }

    async fn failed(&self, msg_id: &str) {
        match self.local.remove_history(msg_id) {
            Ok(()) => self.emit(Event::Unsent { msg_id: msg_id.to_owned() }).await,
            Err(e) => self.emit(Event::StorageError { message: e.to_string() }).await,
        }
    }
}

fn parse_ids(ids: &[String]) -> Result<Vec<DeviceId>, OtError> {
    ids.iter()
        .map(|id| id.parse::<DeviceId>().map_err(|e| OtError::Invalid { message: e.to_string() }))
        .collect()
}

fn person_invite(i: ot_core::PersonInvitation) -> PersonInvite {
    PersonInvite {
        user_id: i.user.to_string(),
        expires_at_ms: millis(i.expires_at),
        code: i.code,
        uri: i.uri,
    }
}

fn parse_device(device: &str) -> Result<DeviceId, OtError> {
    device.parse::<DeviceId>().map_err(|e| OtError::Invalid { message: e.to_string() })
}

fn parse_user(user: &str) -> Result<UserId, OtError> {
    user.parse::<UserId>().map_err(|e| OtError::Invalid { message: e.to_string() })
}

fn parse_users(users: &[String]) -> Result<Vec<UserId>, OtError> {
    users.iter().map(|u| parse_user(u)).collect()
}

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
            E::DirectoryChanged => {
                if let Ok(people) = self.client.directory().await {
                    let mut names = self.names.lock().unwrap_or_else(|e| e.into_inner());
                    for d in people.iter().flat_map(|p| &p.devices) {
                        names.entry(d.id.clone()).or_insert_with(|| d.name.clone());
                    }
                }
                Event::DirectoryChanged
            }
            E::DirectoryRejected { user, reason } => {
                Event::DirectoryRejected { user_id: user.to_string(), reason }
            }
            E::DirectoryFork => Event::DirectoryFork,
            E::KeysChanged { user } => Event::KeysChanged { user_id: user.to_string() },
            E::PersonJoined { user, name, device_name, fingerprint } => {
                Event::PersonJoined { user_id: user.to_string(), name, device_name, fingerprint }
            }
            E::Joined { user } => Event::Joined { user_id: user.to_string() },
            E::InviteFailed { reason } => Event::InviteFailed { reason },
            E::Read { user, until } => {
                let until_ms = i64::try_from(until).unwrap_or(i64::MAX);
                if let Err(e) = self.local.mark_opened(user.as_str(), until_ms) {
                    out.push(Event::StorageError { message: e.to_string() });
                }
                Event::Read { user_id: user.to_string(), until_ms }
            }
            E::Message(m) => {
                let received = Received {
                    msg_id: m.msg_id.to_string(),
                    from_name: self.name_of(&m.from),
                    from_id: m.from.to_string(),
                    created_at_ms: millis(m.created_at),
                    kind: m.content.kind.into(),
                    body: m.content.body,
                    file: None,
                    to: m.to.iter().map(DeviceId::to_string).collect(),
                    outgoing: false,
                    pending: false,
                    from_user: None,
                    from_service: false,
                    to_user: None,
                    to_device: None,
                    auto_fill: false,
                }
                .with_origin(&m.origin);
                self.remember(m.msg_id, &received, &mut out);
                Event::Message { received }
            }
            E::Otp(o) => {
                let auto_fill = o.auto_fill();
                let received = Received {
                    msg_id: o.msg_id.to_string(),
                    from_name: self.name_of(&o.from),
                    from_id: o.from.to_string(),
                    created_at_ms: millis(o.created_at),
                    kind: crate::ContentKind::Text,
                    body: o.otp.code,
                    file: None,
                    to: o.to.iter().map(DeviceId::to_string).collect(),
                    outgoing: false,
                    pending: false,
                    from_user: None,
                    from_service: false,
                    to_user: None,
                    to_device: None,
                    auto_fill,
                }
                .with_origin(&o.origin);
                self.remember(o.msg_id, &received, &mut out);
                Event::Message { received }
            }
            E::File(f) => {
                let to = f.to.iter().map(DeviceId::to_string).collect();
                let f = f.file;
                let received = Received {
                    msg_id: f.msg_id.to_string(),
                    from_name: self.name_of(&f.from),
                    from_id: f.from.to_string(),
                    created_at_ms: millis(f.created_at),
                    kind: crate::ContentKind::File,
                    body: f.name.clone(),
                    file: Some(FileItem::from_info(&f)),
                    to,
                    outgoing: false,
                    pending: false,
                    from_user: None,
                    from_service: false,
                    to_user: None,
                    to_device: None,
                    auto_fill: false,
                }
                .with_origin(&f.origin);
                self.remember(f.msg_id, &received, &mut out);
                Event::Message { received }
            }
        };
        out.insert(0, ev);
        out
    }

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
