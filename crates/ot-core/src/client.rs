use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ot_crypto::{Otp, PairingCode, PairingOffer};
use ot_proto::{
    BlobId, Content, DeviceId, DeviceInfo, DeviceKind, Envelope, ErrorCode, Frame, MAX_FRAME_BYTES,
    SignedBlob, Ulid, UserId, WS_PATH,
};
use time::OffsetDateTime;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::directory::{DirectoryRead, Person, ReadStep};
use crate::files::{FileInfo, FileMeta, PendingUpload, new_file_meta};
use crate::keyring::Keyring;
use crate::secure::{Effect, GroupInfo, Publish, Secure};
use crate::tls::{self, TlsRoots};
use crate::transfer::{
    ChunkSink, ChunkSource, OutgoingFile, Progress, TransferConfig, Transfers, blocking,
    hash_source,
};
use crate::types::{Accepted, AdminRoot, ClientError, Event, Origin, PairingInvitation, Target};
use crate::{Dedup, Identity};

pub(crate) type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(75);

#[derive(Clone)]
pub struct ClientConfig {
    pub server_url: String,
    pub device_id: DeviceId,
    pub identity: Identity,
    pub keyring: Arc<dyn Keyring>,
    pub seen: Vec<Ulid>,
    pub backoff: Backoff,
    pub tls: TlsRoots,
    pub idle_timeout: Duration,
    pub transfer: TransferConfig,
    pub kind: Option<DeviceKind>,
    pub features: Vec<ot_proto::Feature>,
}

impl ClientConfig {
    pub fn new(
        server_url: impl Into<String>,
        device_id: DeviceId,
        identity: Identity,
        keyring: Arc<dyn Keyring>,
    ) -> Self {
        Self {
            server_url: server_url.into(),
            device_id,
            identity,
            keyring,
            seen: Vec::new(),
            backoff: Backoff::default(),
            tls: TlsRoots::default(),
            idle_timeout: IDLE_TIMEOUT,
            transfer: TransferConfig::default(),
            kind: None,
            features: Vec::new(),
        }
    }

    fn ws_url(&self) -> String {
        ws_url(&self.server_url)
    }
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("server_url", &self.server_url)
            .field("device_id", &self.device_id)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

pub(crate) fn ws_url(server_url: &str) -> String {
    let base = server_url.trim_end_matches('/');
    let base = match base.split_once("://") {
        Some(("http", rest)) => format!("ws://{rest}"),
        Some(("https", rest)) => format!("wss://{rest}"),
        Some(_) => base.to_owned(),
        None => format!("ws://{base}"),
    };
    format!("{base}{WS_PATH}")
}

#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { initial: Duration::from_millis(500), max: Duration::from_secs(60) }
    }
}

impl Backoff {
    pub fn delay(&self, attempt: u32) -> Duration {
        let base = self.initial.saturating_mul(1 << attempt.min(16)).min(self.max);
        let half = base / 2;
        let mut r = [0; 8];
        let _ = getrandom::fill(&mut r);
        let jitter = u64::from_le_bytes(r) % (half.as_millis() as u64 + 1);
        half + Duration::from_millis(jitter)
    }
}

type Reply<T> = oneshot::Sender<Result<T, ClientError>>;

type Job = Box<dyn FnOnce(&mut Secure) + Send + Sync>;

type EffectsJob = Box<dyn FnOnce(&mut Secure) -> Result<Vec<Effect>, ClientError> + Send>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Flow {
    Certify(UserId),
    Publish,
    Republish(UserId),
}

const FLOW_RETRY: Duration = Duration::from_secs(60);

type Rotation = (u32, Vec<oneshot::Receiver<Result<Accepted, ClientError>>>);

enum Command {
    Send(Envelope, Reply<Accepted>),
    SendContent(Content, Vec<DeviceId>, Reply<Accepted>),
    ListDevices(Reply<Vec<DeviceInfo>>),
    PairCreate(Reply<PairingInvitation>),
    PairJoin { code: PairingCode, sponsor: DeviceId, name: String, reply: Reply<Accepted> },
    Rotate(Vec<DeviceId>, Reply<Rotation>),
    Group(oneshot::Sender<Option<GroupInfo>>),
    PairOffer(String, Reply<String>),
    Sponsor(Box<PairingOffer>, Reply<Accepted>),
    SendFile(Box<FileMeta>, Vec<DeviceId>, Option<UserId>, Reply<Accepted>),
    WithSecure(Box<dyn FnOnce(&mut Secure) + Send>),
    Fresh(Job),
    Directory(oneshot::Sender<Vec<Person>>),
    RefreshDirectory,
    AfterRead(Job),
    Call(Frame, Reply<Frame>),
    Effects(EffectsJob, Reply<()>),
    FlowDone(Flow, bool),
    AdminInit(bool, Reply<AdminRoot>),
    Reconnect,
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct Client {
    device_id: DeviceId,
    pub(crate) server_url: String,
    cmd: mpsc::UnboundedSender<Command>,
    transfers: Transfers,
    pub(crate) publish_lock: Arc<tokio::sync::Mutex<()>>,
}

struct Handle {
    device_id: DeviceId,
    server_url: String,
    cmd: mpsc::WeakUnboundedSender<Command>,
    transfers: Transfers,
    publish_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Handle {
    fn client(&self) -> Option<Client> {
        Some(Client {
            device_id: self.device_id.clone(),
            server_url: self.server_url.clone(),
            cmd: self.cmd.upgrade()?,
            transfers: self.transfers.clone(),
            publish_lock: self.publish_lock.clone(),
        })
    }
}

pub struct Events(mpsc::Receiver<Event>);

impl Events {
    pub async fn next(&mut self) -> Option<Event> {
        self.0.recv().await
    }
}

impl Client {
    pub fn start(config: ClientConfig) -> (Self, Events) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, ev_rx) = mpsc::channel(256);
        let transfers = Transfers::new(
            &config.server_url,
            &config.tls,
            config.device_id.clone(),
            config.identity.clone(),
            config.transfer.clone(),
        );
        let client = Self {
            device_id: config.device_id.clone(),
            server_url: config.server_url.clone(),
            cmd: cmd_tx,
            transfers,
            publish_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        let handle = Handle {
            device_id: client.device_id.clone(),
            server_url: client.server_url.clone(),
            cmd: client.cmd.downgrade(),
            transfers: client.transfers.clone(),
            publish_lock: client.publish_lock.clone(),
        };
        tokio::spawn(Worker::new(config, cmd_rx, ev_tx, handle).run());
        (client, Events(ev_rx))
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    async fn request<T>(&self, cmd: impl FnOnce(Reply<T>) -> Command) -> Result<T, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(cmd(tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)?
    }

    pub async fn send(
        &self,
        content: &Content,
        to: Vec<DeviceId>,
    ) -> Result<Accepted, ClientError> {
        self.request(|tx| Command::SendContent(content.clone(), to, tx)).await
    }

    pub async fn seal(
        &self,
        content: &Content,
        to: Vec<DeviceId>,
    ) -> Result<Envelope, ClientError> {
        let content = content.clone();
        self.with_secure(move |s| s.seal_content(&content, to)).await
    }

    pub async fn send_otp(&self, otp: &Otp, to: Vec<DeviceId>) -> Result<Accepted, ClientError> {
        let otp = otp.clone();
        let env = self.with_secure(move |s| s.seal_otp(&otp, to)).await?;
        self.send_envelope(env).await
    }

    pub async fn send_read(
        &self,
        user: &UserId,
        until: u64,
    ) -> Result<Option<Accepted>, ClientError> {
        let user = user.clone();
        match self.with_secure(move |s| s.seal_read(&user, until)).await? {
            Some(env) => self.send_envelope(env).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn send_envelope(&self, env: Envelope) -> Result<Accepted, ClientError> {
        self.request(|tx| Command::Send(env, tx)).await
    }

    pub async fn seal_to(
        &self,
        target: Target,
        content: &Content,
    ) -> Result<Envelope, ClientError> {
        match target {
            Target::MyDevices(to) => self.seal(content, to).await,
            Target::Person { user } => {
                let content = content.clone();
                self.with_fresh(move |s| s.seal_content_to(&content, &user)).await
            }
        }
    }

    pub async fn send_to(
        &self,
        target: Target,
        content: &Content,
    ) -> Result<Accepted, ClientError> {
        let env = self.seal_to(target, content).await?;
        self.send_envelope(env).await
    }

    pub async fn send_otp_to(&self, users: &[UserId], otp: &Otp) -> Result<Accepted, ClientError> {
        let (users, otp) = (users.to_vec(), otp.clone());
        let env = self.with_fresh(move |s| s.seal_otp_to(&otp, &users)).await?;
        self.send_envelope(env).await
    }

    pub(crate) async fn after_read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Secure) -> Result<T, ClientError> + Send + Sync + 'static,
    ) -> Result<T, ClientError> {
        let (tx, rx) = oneshot::channel();
        let run = Box::new(move |s: &mut Secure| {
            let _ = tx.send(f(s));
        });
        self.cmd.send(Command::AfterRead(run)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)?
    }

    pub(crate) async fn with_effects(
        &self,
        f: impl FnOnce(&mut Secure) -> Result<Vec<Effect>, ClientError> + Send + 'static,
    ) -> Result<(), ClientError> {
        self.request(|tx| Command::Effects(Box::new(f), tx)).await
    }

    pub(crate) async fn call(&self, frame: Frame) -> Result<Frame, ClientError> {
        self.request(|tx| Command::Call(frame, tx)).await
    }

    pub(crate) async fn with_fresh<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Secure) -> Result<T, ClientError> + Send + Sync + 'static,
    ) -> Result<T, ClientError> {
        let (tx, rx) = oneshot::channel();
        let run = Box::new(move |s: &mut Secure| {
            let _ = tx.send(f(s));
        });
        self.cmd.send(Command::Fresh(run)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)?
    }

    pub(crate) async fn with_secure<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Secure) -> Result<T, ClientError> + Send + 'static,
    ) -> Result<T, ClientError> {
        let (tx, rx) = oneshot::channel();
        let run = Box::new(move |s: &mut Secure| {
            let _ = tx.send(f(s));
        });
        self.cmd.send(Command::WithSecure(run)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)?
    }

    pub async fn send_file(
        &self,
        file: OutgoingFile,
        source: Arc<dyn ChunkSource>,
        progress: Arc<dyn Progress>,
    ) -> Result<Accepted, ClientError> {
        self.send_file_info(file, source, progress).await.map(|(accepted, _)| accepted)
    }

    pub async fn send_file_info(
        &self,
        file: OutgoingFile,
        source: Arc<dyn ChunkSource>,
        progress: Arc<dyn Progress>,
    ) -> Result<(Accepted, FileInfo), ClientError> {
        let t = &self.transfers;
        let size = source.size();
        let now = OffsetDateTime::now_utc();
        let to = match file.to_user.clone() {
            Some(user) => self.with_fresh(move |s| s.recipients(&user)).await?,
            None => {
                self.with_secure(|s| s.info().map(|_| ()).ok_or(ClientError::NoGroup)).await?;
                file.to.clone()
            }
        };
        let sid = file.source_id.clone();
        let mut pending = self.with_secure(move |s| s.upload(&sid, size, now)).await?;
        let mut missing = None;
        if let Some(p) = pending.take_if(|p| p.to == to && p.to_user == file.to_user) {
            match t.status(&p.meta.blob_id).await {
                Ok(st) => {
                    tracing::info!(blob = %p.meta.blob_id, missing = st.missing.len(), "reprise du dépôt");
                    missing = Some(st.missing);
                    pending = Some(p);
                }
                Err(ClientError::Expired) => {}
                Err(e) => return Err(e),
            }
        }
        let pending = match pending {
            Some(p) => p,
            None => {
                let sha256 = hash_source(source.clone(), t.config.chunk_size).await?;
                let (created, _) = t.create(size, &to).await?;
                let meta = new_file_meta(
                    created.blob_id,
                    &file.name,
                    &file.mime,
                    size,
                    t.config.chunk_size,
                    sha256,
                )
                .ok_or_else(|| ClientError::Invalid("fichier trop gros".into()))?;
                let p = PendingUpload {
                    source_id: file.source_id.clone(),
                    to,
                    to_user: file.to_user.clone(),
                    expires_at: created.expires_at,
                    meta,
                };
                let saved = p.clone();
                self.with_secure(move |s| s.save_upload(saved, now)).await?;
                p
            }
        };
        let missing = missing.unwrap_or_else(|| (0..pending.meta.chunks).collect());
        t.upload(&pending.meta, missing, source, progress).await?;
        let (meta, to) = (Box::new(pending.meta.clone()), pending.to.clone());
        let to_user = pending.to_user.clone();
        let accepted = self.request(|tx| Command::SendFile(meta, to, to_user, tx)).await?;
        let sid = pending.source_id;
        self.with_secure(move |s| s.forget_upload(&sid)).await?;
        tracing::info!(blob = %pending.meta.blob_id, "fichier envoyé");
        let info = FileInfo {
            msg_id: accepted.msg_id,
            from: self.device_id.clone(),
            created_at: OffsetDateTime::now_utc(),
            blob_id: pending.meta.blob_id.clone(),
            name: pending.meta.name.clone(),
            mime: pending.meta.mime.clone(),
            size: pending.meta.size,
            origin: Origin { to_user: pending.to_user, ..Origin::default() },
        };
        Ok((accepted, info))
    }

    pub async fn cancel_upload(&self, source_id: &str) -> Result<(), ClientError> {
        let sid = source_id.to_owned();
        let uploads = self.with_secure(move |s| s.uploads_of(&sid));
        for p in uploads.await? {
            self.transfers.cancel(&p.meta.blob_id).await?;
        }
        let sid = source_id.to_owned();
        self.with_secure(move |s| s.forget_upload(&sid)).await
    }

    pub async fn files(&self) -> Result<Vec<FileInfo>, ClientError> {
        self.with_secure(|s| s.files()).await
    }

    pub async fn download(
        &self,
        blob_id: &BlobId,
        sink: Arc<dyn ChunkSink>,
        progress: Arc<dyn Progress>,
    ) -> Result<FileInfo, ClientError> {
        let blob = blob_id.clone();
        let file = self
            .with_secure(move |s| s.file(&blob))
            .await?
            .ok_or_else(|| ClientError::Invalid("fichier inconnu sur cet appareil".into()))?;
        let res = self.transfers.download(&file, sink.clone(), progress).await;
        let res = match res {
            Ok(()) => {
                let dst = sink.clone();
                blocking(move || dst.finish())
                    .await?
                    .map_err(|e| ClientError::Transfer(format!("fichier non enregistré : {e}")))
            }
            Err(e) => Err(e),
        };
        if let Err(e) = res {
            let dst = sink.clone();
            let _ = blocking(move || dst.abort()).await;
            if e == ClientError::Expired {
                let blob = blob_id.clone();
                self.with_secure(move |s| s.forget_file(&blob)).await?;
            }
            return Err(e);
        }
        if let Err(e) = self.transfers.done(blob_id).await {
            tracing::warn!(blob = %blob_id, error = %e, "fin du retrait non signalée au relais");
        }
        let blob = blob_id.clone();
        self.with_secure(move |s| s.forget_file(&blob)).await?;
        tracing::info!(blob = %blob_id, "fichier reçu");
        Ok(file.info())
    }

    pub async fn forget_file(&self, blob_id: &BlobId) -> Result<(), ClientError> {
        let blob = blob_id.clone();
        self.with_secure(move |s| s.forget_file(&blob)).await
    }

    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>, ClientError> {
        self.request(Command::ListDevices).await
    }

    pub async fn pair_create(&self) -> Result<PairingInvitation, ClientError> {
        self.request(Command::PairCreate).await
    }

    pub async fn pair_join(
        &self,
        code: &str,
        sponsor: DeviceId,
        name: &str,
    ) -> Result<Accepted, ClientError> {
        let code = PairingCode::parse(code).map_err(|e| ClientError::Pairing(e.to_string()))?;
        let name = name.to_owned();
        self.request(|reply| Command::PairJoin { code, sponsor, name, reply }).await
    }

    pub async fn rotate(&self, exclude: Vec<DeviceId>) -> Result<u32, ClientError> {
        let (epoch, sends) = self.request(|tx| Command::Rotate(exclude, tx)).await?;
        for rx in sends {
            rx.await.map_err(|_| ClientError::Closed)??;
        }
        Ok(epoch)
    }

    pub async fn pair_offer(&self, name: &str) -> Result<String, ClientError> {
        let name = name.to_owned();
        self.request(|tx| Command::PairOffer(name, tx)).await
    }

    pub async fn sponsor(&self, qr: &str) -> Result<(DeviceId, String), ClientError> {
        let offer = PairingOffer::parse(qr).map_err(|e| ClientError::Pairing(e.to_string()))?;
        let added = (offer.device_id.clone(), offer.name.clone());
        self.request(|tx| Command::Sponsor(Box::new(offer), tx)).await?;
        Ok(added)
    }

    pub fn reconnect_now(&self) {
        let _ = self.cmd.send(Command::Reconnect);
    }

    pub async fn directory(&self) -> Result<Vec<Person>, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Directory(tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)
    }

    pub async fn fresh_directory(&self) -> Result<Vec<Person>, ClientError> {
        self.with_fresh(|s| Ok(s.people())).await
    }

    pub fn refresh_directory(&self) {
        let _ = self.cmd.send(Command::RefreshDirectory);
    }

    pub async fn admin_init(&self) -> Result<UserId, ClientError> {
        match self.admin_root(true).await? {
            AdminRoot::Ready(admin) => Ok(admin),
            AdminRoot::NotAdmin => {
                Err(ClientError::Invalid("cet appareil n'est pas à l'admin".into()))
            }
            AdminRoot::Missing { .. } => {
                Err(ClientError::Invalid("certificat de l'admin non publié".into()))
            }
        }
    }

    pub async fn admin_root(&self, create: bool) -> Result<AdminRoot, ClientError> {
        self.request(|reply| Command::AdminInit(create, reply)).await
    }

    pub async fn group(&self) -> Result<Option<GroupInfo>, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Group(tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::Shutdown(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

enum End {
    Lost(String),
    Reconnect,
    AuthFailed(String),
    Shutdown(Option<oneshot::Sender<()>>),
}

struct Outgoing {
    env: Envelope,
    reply: Option<Reply<Accepted>>,
}

enum Next {
    Frames(Vec<Frame>),
    Reconnect,
    Shutdown(Option<oneshot::Sender<()>>),
}

struct Registration {
    offer: Box<PairingOffer>,
    reply: Reply<Accepted>,
}

impl Registration {
    fn frame(&self) -> Frame {
        Frame::PairRegister {
            device_id: self.offer.device_id.clone(),
            name: self.offer.name.clone(),
            public_key: self.offer.sign_key,
            sig: self.offer.sig,
        }
    }
}

struct Worker {
    config: ClientConfig,
    cmd: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    dedup: Dedup,
    secure: Secure,
    startup: Vec<Event>,
    outbox: Vec<Outgoing>,
    device_requests: VecDeque<Reply<Vec<DeviceInfo>>>,
    unref: VecDeque<Pending>,
    read: Option<Reading>,
    read_again: bool,
    registrations: Vec<Registration>,
    waiting: Vec<Job>,
    waiting_next: Vec<Job>,
    declared: bool,
    handle: Handle,
    flows: Vec<Flow>,
    failed: Vec<(Flow, Instant)>,
}

impl Worker {
    fn new(
        mut config: ClientConfig,
        cmd: mpsc::UnboundedReceiver<Command>,
        events: mpsc::Sender<Event>,
        handle: Handle,
    ) -> Self {
        let dedup = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, std::mem::take(&mut config.seen));
        let (secure, startup) =
            Secure::load(config.device_id.clone(), config.identity.clone(), config.keyring.clone());
        Self {
            config,
            cmd,
            events,
            dedup,
            secure,
            startup,
            outbox: Vec::new(),
            device_requests: VecDeque::new(),
            unref: VecDeque::new(),
            read: None,
            read_again: false,
            registrations: Vec::new(),
            waiting: Vec::new(),
            waiting_next: Vec::new(),
            declared: false,
            handle,
            flows: Vec::new(),
            failed: Vec::new(),
        }
    }

    async fn emit(&self, event: Event) {
        let _ = self.events.send(event).await;
    }

    fn queue(&mut self, env: Envelope, reply: Option<Reply<Accepted>>) -> Next {
        let frame = Frame::Send { env: env.clone() };
        self.outbox.push(Outgoing { env, reply });
        Next::Frames(vec![frame])
    }

    async fn on_command(&mut self, cmd: Option<Command>) -> Next {
        let Some(cmd) = cmd else {
            return Next::Shutdown(None);
        };
        match cmd {
            Command::Shutdown(done) => Next::Shutdown(Some(done)),
            Command::Send(env, tx) => self.queue(env, Some(tx)),
            Command::SendContent(content, to, tx) => match self.secure.seal_content(&content, to) {
                Ok(env) => self.queue(env, Some(tx)),
                Err(e) => {
                    let _ = tx.send(Err(e));
                    Next::Frames(Vec::new())
                }
            },
            Command::ListDevices(tx) => {
                self.device_requests.push_back(tx);
                Next::Frames(vec![Frame::ListDevices])
            }
            Command::PairCreate(tx) => {
                if self.secure.info().is_none() {
                    let _ = tx.send(Err(ClientError::NoGroup));
                    return Next::Frames(Vec::new());
                }
                self.unref.push_back(Pending::Pair(tx));
                Next::Frames(vec![Frame::PairCreate])
            }
            Command::PairJoin { code, sponsor, name, reply } => {
                match self.secure.start_join(&code, sponsor, &name) {
                    Ok(env) => self.queue(env, Some(reply)),
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        Next::Frames(Vec::new())
                    }
                }
            }
            Command::Rotate(exclude, tx) => {
                match self.secure.rotate(&exclude, OffsetDateTime::now_utc()) {
                    Ok((epoch, envs, effects)) => {
                        let mut frames = Vec::new();
                        let mut sent = Vec::new();
                        for env in envs {
                            let (reply, rx) = oneshot::channel();
                            sent.push(rx);
                            if let Next::Frames(f) = self.queue(env, Some(reply)) {
                                frames.extend(f);
                            }
                        }
                        frames.extend(self.effects(effects).await);
                        let _ = tx.send(Ok((epoch, sent)));
                        Next::Frames(frames)
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        Next::Frames(Vec::new())
                    }
                }
            }
            Command::Group(tx) => {
                let _ = tx.send(self.secure.info());
                Next::Frames(Vec::new())
            }
            Command::PairOffer(name, tx) => {
                let _ = tx.send(self.secure.offer(&name).map(|o| o.encode().to_string()));
                Next::Frames(Vec::new())
            }
            Command::Sponsor(offer, reply) => {
                let pending =
                    self.registrations.iter().any(|r| r.offer.device_id == offer.device_id);
                let res = if pending {
                    Err(ClientError::Pairing("appairage déjà en cours".into()))
                } else {
                    self.secure.check_offer(&offer)
                };
                if let Err(e) = res {
                    let _ = reply.send(Err(e));
                    return Next::Frames(Vec::new());
                }
                let registration = Registration { offer, reply };
                let frame = registration.frame();
                self.registrations.push(registration);
                Next::Frames(vec![frame])
            }
            Command::SendFile(meta, to, to_user, tx) => match match to_user {
                Some(_) => self.secure.seal_file_to(&meta, to),
                None => self.secure.seal_file(&meta, to),
            } {
                Ok(env) => self.queue(env, Some(tx)),
                Err(e) => {
                    let _ = tx.send(Err(e));
                    Next::Frames(Vec::new())
                }
            },
            Command::WithSecure(f) => {
                f(&mut self.secure);
                Next::Frames(Vec::new())
            }
            Command::Fresh(f) => {
                let now = OffsetDateTime::now_utc();
                if !self.secure.has_anchor()
                    || self.secure.directory_fresh(now)
                    || matches!(self.read, Some(Reading::AdminInit { .. }))
                {
                    f(&mut self.secure);
                    return Next::Frames(Vec::new());
                }
                self.waiting.push(f);
                if self.read.is_some() {
                    Next::Frames(Vec::new())
                } else {
                    Next::Frames(self.start_read())
                }
            }
            Command::Directory(tx) => {
                let _ = tx.send(self.secure.people());
                Next::Frames(Vec::new())
            }
            Command::RefreshDirectory => Next::Frames(self.start_read()),
            Command::AfterRead(f) => {
                if !self.secure.has_anchor() || matches!(self.read, Some(Reading::AdminInit { .. }))
                {
                    f(&mut self.secure);
                    return Next::Frames(Vec::new());
                }
                if self.read.is_some() {
                    self.waiting_next.push(f);
                    self.read_again = true;
                    return Next::Frames(Vec::new());
                }
                self.waiting_next.push(f);
                Next::Frames(self.start_read())
            }
            Command::Call(frame, reply) => {
                let reference = call_reference(&frame);
                self.unref.push_back(Pending::Call {
                    frame: frame.clone(),
                    reference,
                    reply: Some(reply),
                });
                Next::Frames(vec![frame])
            }
            Command::Effects(f, reply) => match f(&mut self.secure) {
                Ok(effects) => {
                    let frames = self.effects(effects).await;
                    let _ = reply.send(Ok(()));
                    Next::Frames(frames)
                }
                Err(e) => {
                    let _ = reply.send(Err(e));
                    Next::Frames(Vec::new())
                }
            },
            Command::FlowDone(flow, ok) => {
                self.flows.retain(|f| f != &flow);
                self.failed.retain(|(f, _)| f != &flow);
                if !ok {
                    self.failed.push((flow, Instant::now()));
                }
                Next::Frames(self.drive())
            }
            Command::AdminInit(create, reply) => {
                let busy = matches!(self.read, Some(Reading::AdminInit { .. }))
                    || self.unref.iter().any(|p| matches!(p, Pending::AdminInit { .. }));
                let refused = if self.secure.info().is_none() {
                    Some(ClientError::NoGroup)
                } else if self.secure.has_anchor() && create {
                    Some(ClientError::Invalid("certificat de l'admin déjà publié".into()))
                } else if self.secure.has_anchor() {
                    let job: Job = Box::new(move |s| {
                        let _ = reply.send(Ok(known_root(s)));
                    });
                    if self.secure.directory_state().admin.is_some() {
                        job(&mut self.secure);
                        return Next::Frames(Vec::new());
                    }
                    self.waiting.push(job);
                    return Next::Frames(self.start_read());
                } else if busy {
                    Some(ClientError::Invalid("admin_init déjà en cours".into()))
                } else {
                    None
                };
                if let Some(e) = refused {
                    let _ = reply.send(Err(e));
                    return Next::Frames(Vec::new());
                }
                self.read = Some(Reading::AdminInit { create, reply });
                self.unref.push_back(Pending::Directory);
                Next::Frames(vec![Frame::DirectoryGet { known: Default::default(), after: None }])
            }
            Command::Reconnect => Next::Reconnect,
        }
    }

    async fn effects(&mut self, effects: Vec<Effect>) -> Vec<Frame> {
        let mut frames = Vec::new();
        for effect in effects {
            match effect {
                Effect::Emit(event) => self.emit(event).await,
                Effect::Send(env) => {
                    if let Next::Frames(f) = self.queue(env, None) {
                        frames.extend(f);
                    }
                }
            }
        }
        frames.extend(self.drive());
        frames
    }

    fn drive(&mut self) -> Vec<Frame> {
        let mut due: Vec<Flow> =
            self.secure.invites_to_certify().into_iter().map(Flow::Certify).collect();
        if self.secure.needs_publish() {
            due.push(Flow::Publish);
        }
        due.extend(self.secure.services_to_republish().into_iter().map(Flow::Republish));
        let now = Instant::now();
        self.failed.retain(|(_, at)| now.duration_since(*at) < FLOW_RETRY);
        for flow in due {
            if self.flows.contains(&flow) || self.failed.iter().any(|(f, _)| f == &flow) {
                continue;
            }
            let Some(client) = self.handle.client() else { break };
            self.flows.push(flow.clone());
            tokio::spawn(async move {
                let res = match &flow {
                    Flow::Certify(user) => client.certify_invite(user.clone()).await,
                    Flow::Publish => client.publish(Publish::IfBehind).await.map(|_| ()),
                    Flow::Republish(user) => client.publish_service(user, None).await.map(|_| ()),
                };
                if let Err(e) = &res {
                    tracing::warn!(?flow, error = %e, "déroulé de fond en échec, à reprendre");
                }
                let _ = client.cmd.send(Command::FlowDone(flow, res.is_ok()));
            });
        }
        if self.secure.awaits_directory() && self.read.is_none() {
            return self.start_read();
        }
        Vec::new()
    }

    async fn run(mut self) {
        for event in std::mem::take(&mut self.startup) {
            self.emit(event).await;
        }
        let mut attempt = 0u32;
        loop {
            let end = match self.connect().await {
                Ok((ws, pending)) => {
                    attempt = 0;
                    self.emit(Event::Connected { pending }).await;
                    let end = self.session(ws).await;
                    self.fail_calls();
                    end
                }
                Err(end) => end,
            };
            let delay = match end {
                End::Shutdown(done) => {
                    self.fail_all();
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                    return;
                }
                End::Lost(reason) => {
                    tracing::debug!(reason, "connexion perdue");
                    self.emit(Event::Disconnected { reason }).await;
                    self.config.backoff.delay(attempt)
                }
                End::Reconnect => {
                    tracing::debug!("reconnexion demandée");
                    self.emit(Event::Disconnected { reason: "changement de réseau".into() }).await;
                    attempt = 0;
                    continue;
                }
                End::AuthFailed(message) => {
                    tracing::warn!(message, "authentification refusée");
                    self.emit(Event::AuthFailed { message }).await;
                    self.config.backoff.max
                }
            };
            attempt = attempt.saturating_add(1);
            if let Some(done) = self.wait(delay).await {
                self.fail_all();
                let _ = done.send(());
                return;
            }
        }
    }

    async fn wait(&mut self, delay: Duration) -> Option<oneshot::Sender<()>> {
        let until = Instant::now() + delay;
        loop {
            tokio::select! {
                () = sleep_until(until) => return None,
                cmd = self.cmd.recv() => {
                    match self.on_command(cmd).await {
                        Next::Shutdown(done) => {
                            return Some(done.unwrap_or_else(|| oneshot::channel().0));
                        }
                        Next::Reconnect => return None,
                        Next::Frames(_) => {}
                    }
                }
            }
        }
    }

    fn fail_all(&mut self) {
        for out in self.outbox.drain(..) {
            if let Some(tx) = out.reply {
                let _ = tx.send(Err(ClientError::Closed));
            }
        }
        for tx in self.device_requests.drain(..) {
            let _ = tx.send(Err(ClientError::Closed));
        }
        for pending in self.unref.drain(..) {
            match pending {
                Pending::Pair(tx) => {
                    let _ = tx.send(Err(ClientError::Closed));
                }
                Pending::AdminInit { reply, .. } => {
                    let _ = reply.send(Err(ClientError::Closed));
                }
                Pending::Directory => {}
                Pending::Call { reply, .. } => {
                    if let Some(tx) = reply {
                        let _ = tx.send(Err(ClientError::Closed));
                    }
                }
            }
        }
        if let Some(Reading::AdminInit { reply, .. }) = self.read.take() {
            let _ = reply.send(Err(ClientError::Closed));
        }
        self.waiting.clear();
        self.waiting_next.clear();
        for r in self.registrations.drain(..) {
            let _ = r.reply.send(Err(ClientError::Closed));
        }
    }

    async fn connect(&mut self) -> Result<(Ws, u64), End> {
        let url = self.config.ws_url();
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES));
        let connect = tls::connect(&url, &self.config.tls, Some(ws_config));
        let mut ws = match timeout(HANDSHAKE_TIMEOUT, connect).await {
            Ok(Ok(ok)) => ok,
            Ok(Err(e)) => return Err(End::Lost(format!("connexion à {url} : {e}"))),
            Err(_) => return Err(End::Lost(format!("connexion à {url} : délai dépassé"))),
        };
        match timeout(HANDSHAKE_TIMEOUT, self.authenticate(&mut ws)).await {
            Ok(Ok(pending)) => Ok((ws, pending)),
            Ok(Err(end)) => Err(end),
            Err(_) => Err(End::Lost("authentification : délai dépassé".into())),
        }
    }

    async fn authenticate(&self, ws: &mut Ws) -> Result<u64, End> {
        let device_id = self.config.device_id.clone();
        loop {
            match recv_frame(ws).await? {
                Frame::Challenge { nonce } => {
                    let sig = self.config.identity.sign_challenge(&nonce, &device_id);
                    send_frame(ws, &Frame::Auth { device_id: device_id.clone(), sig }).await?;
                }
                Frame::Welcome { pending, .. } => return Ok(pending),
                Frame::Error { code: ErrorCode::AuthFailed, message, .. } => {
                    return Err(End::AuthFailed(message));
                }
                other => tracing::debug!(t = other.type_name(), "trame ignorée avant welcome"),
            }
        }
    }

    async fn session(&mut self, mut ws: Ws) -> End {
        match self.session_inner(&mut ws).await {
            End::Shutdown(done) => {
                let _ = ws.close(None).await;
                let drain = async { while let Some(Ok(_)) = ws.next().await {} };
                let _ = timeout(CLOSE_TIMEOUT, drain).await;
                End::Shutdown(done)
            }
            end => end,
        }
    }

    async fn session_inner(&mut self, ws: &mut Ws) -> End {
        let mut resume: Vec<Frame> =
            self.outbox.iter().map(|o| Frame::Send { env: o.env.clone() }).collect();
        resume.extend(self.device_requests.iter().map(|_| Frame::ListDevices));
        self.unref.retain(|p| !matches!(p, Pending::Directory));
        resume.extend(self.unref.iter().map(Pending::frame));
        resume.extend(self.registrations.iter().map(Registration::frame));
        self.failed.clear();
        self.declared = false;
        match self.secure.invite_request() {
            Some(Ok(env)) => {
                self.outbox.push(Outgoing { env: env.clone(), reply: None });
                resume.push(Frame::Send { env });
            }
            Some(Err(e)) => tracing::warn!(error = %e, "demande d'invitation impossible"),
            None => {}
        }
        match &self.read {
            Some(Reading::AdminInit { .. }) => {
                self.unref.push_back(Pending::Directory);
                resume.push(Frame::DirectoryGet { known: Default::default(), after: None });
            }
            _ => {
                self.read = None;
                self.read_again = false;
                resume.extend(self.start_read());
            }
        }
        for frame in &resume {
            if let Err(end) = send_frame(ws, frame).await {
                return end;
            }
        }

        let idle_timeout = self.config.idle_timeout;
        let mut idle = Instant::now() + idle_timeout;
        loop {
            tokio::select! {
                () = sleep_until(idle) => return End::Lost("plus de nouvelles du serveur".into()),
                cmd = self.cmd.recv() => match self.on_command(cmd).await {
                    Next::Shutdown(done) => return End::Shutdown(done),
                    Next::Reconnect => return End::Reconnect,
                    Next::Frames(frames) => {
                        for frame in &frames {
                            if let Err(end) = send_frame(ws, frame).await {
                                return end;
                            }
                        }
                    }
                },
                msg = recv_message(ws) => {
                    idle = Instant::now() + idle_timeout;
                    let res = match msg {
                        Ok(Some(frame)) => self.on_frame(ws, frame).await,
                        Ok(None) => Ok(()),
                        Err(end) => Err(end),
                    };
                    if let Err(end) = res {
                        return end;
                    }
                }
            }
        }
    }

    async fn on_frame(&mut self, ws: &mut Ws, frame: Frame) -> Result<(), End> {
        match frame {
            Frame::Deliver { seq, env } => {
                if self.dedup.insert(env.msg_id) {
                    let effects = self.secure.process(seq, env, OffsetDateTime::now_utc());
                    for frame in self.effects(effects).await {
                        send_frame(ws, &frame).await?;
                    }
                } else {
                    tracing::debug!(seq, msg_id = %env.msg_id, "doublon ignoré");
                }
                send_frame(ws, &Frame::Ack { seq }).await?;
            }
            Frame::Accepted { msg_id, seq } => {
                if let Some(i) = self.outbox.iter().position(|o| o.env.msg_id == msg_id)
                    && let Some(tx) = self.outbox.remove(i).reply
                {
                    let _ = tx.send(Ok(Accepted { msg_id, seq }));
                }
            }
            Frame::Devices { devices } => {
                if let Some(tx) = self.device_requests.pop_front() {
                    let _ = tx.send(Ok(devices));
                }
            }
            Frame::PairCreated { token, expires_at } => {
                if let Some(Pending::Pair(tx)) =
                    self.take_pending(|p| matches!(p, Pending::Pair(_)))
                {
                    let res = self.secure.sponsor(token, expires_at).map(|code| {
                        PairingInvitation { code: code.encode().to_string(), expires_at }
                    });
                    let _ = tx.send(res);
                }
            }
            Frame::PairRegistered { device_id } => {
                let Some(i) =
                    self.registrations.iter().position(|r| r.offer.device_id == device_id)
                else {
                    let reply = Frame::PairRegistered { device_id };
                    match self.take_pending(|p| p.answered_by(&reply)) {
                        Some(Pending::Call { reply: Some(tx), .. }) => {
                            let _ = tx.send(Ok(reply));
                        }
                        _ => tracing::warn!("pair_registered inattendu"),
                    }
                    return Ok(());
                };
                let Registration { offer, reply } = self.registrations.remove(i);
                match self.secure.welcome_offer(&offer) {
                    Ok((welcome, effects)) => {
                        let mut frames = match self.queue(welcome, Some(reply)) {
                            Next::Frames(f) => f,
                            _ => Vec::new(),
                        };
                        frames.extend(self.effects(effects).await);
                        for frame in &frames {
                            send_frame(ws, frame).await?;
                        }
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            Frame::Directory { me, admin, users, next } => {
                let _ = self.take_pending(|p| matches!(p, Pending::Directory));
                for frame in self.on_directory(me, admin, users, next).await {
                    send_frame(ws, &frame).await?;
                }
            }
            Frame::DirectoryChanged => {
                for frame in self.start_read() {
                    send_frame(ws, &frame).await?;
                }
            }
            Frame::AdminInitialized { user } => {
                if let Some(Pending::AdminInit { cert, reply, .. }) =
                    self.take_pending(|p| matches!(p, Pending::AdminInit { .. }))
                {
                    tracing::info!(user = %user, "certificat de l'admin publié");
                    let effects = self.secure.adopt_admin_root(&cert);
                    let mut frames = self.effects(effects).await;
                    let _ = reply.send(Ok(AdminRoot::Ready(user)));
                    frames.extend(self.start_read());
                    for frame in &frames {
                        send_frame(ws, frame).await?;
                    }
                }
            }
            Frame::Pong => {}
            Frame::Error { code: ErrorCode::AuthFailed, message, .. } => {
                return Err(End::AuthFailed(message));
            }
            reply @ (Frame::UserInvited { .. }
            | Frame::UserCertified { .. }
            | Frame::RosterPublished { .. }
            | Frame::DeviceUpdated { .. }
            | Frame::DeviceRevoked { .. }
            | Frame::ServiceMuted { .. }
            | Frame::UserRevoked { .. }
            | Frame::UserRenamed { .. }
            | Frame::LinkUpdated { .. }
            | Frame::Users { .. }) => match self.take_pending(|p| p.answered_by(&reply)) {
                Some(Pending::Call { reply: Some(tx), .. }) => {
                    let _ = tx.send(Ok(reply));
                }
                Some(_) => {
                    for frame in self.start_read() {
                        send_frame(ws, &frame).await?;
                    }
                }
                None => tracing::debug!(t = reply.type_name(), "réponse sans demande ignorée"),
            },
            Frame::Error { code, message, reference }
                if reference.as_deref().is_some_and(|r| {
                    self.registrations.iter().any(|reg| reg.offer.device_id.as_str() == r)
                }) =>
            {
                let r = reference.as_deref().unwrap_or_default();
                if let Some(i) =
                    self.registrations.iter().position(|reg| reg.offer.device_id.as_str() == r)
                {
                    let reg = self.registrations.remove(i);
                    let _ = reg.reply.send(Err(ClientError::Rejected { code, message }));
                }
            }
            Frame::Error { code, message, reference } => {
                let pos = reference
                    .as_deref()
                    .and_then(|r| self.outbox.iter().position(|o| o.env.msg_id.to_string() == r));
                let err = ClientError::Rejected { code, message: message.clone() };
                let call = match (pos, reference.as_deref()) {
                    (None, Some(r)) => self.take_pending(|p| p.reference() == Some(r)),
                    _ => None,
                };
                if let Some(Pending::Call { frame, reply, .. }) = call {
                    match reply {
                        Some(tx) => {
                            let _ = tx.send(Err(err));
                        }
                        None => tracing::warn!(?code, message, t = frame.type_name(), "refusé"),
                    }
                    return Ok(());
                }
                if pos.is_none() && reference.is_some() {
                    tracing::warn!(?code, message, "refus d'une demande inconnue");
                    return Ok(());
                }
                match pos.map(|i| self.outbox.remove(i)) {
                    Some(Outgoing { reply: Some(tx), .. }) => {
                        let _ = tx.send(Err(err));
                    }
                    Some(Outgoing { env, reply: None }) => {
                        tracing::warn!(?code, message, msg_id = %env.msg_id, "message système refusé");
                    }
                    None => match self.unref.pop_front() {
                        Some(Pending::Pair(tx)) => {
                            let _ = tx.send(Err(err));
                        }
                        Some(Pending::AdminInit { reply, .. }) => {
                            let _ = reply.send(Err(err));
                        }
                        Some(Pending::Call { frame, reply, .. }) => match reply {
                            Some(tx) => {
                                let _ = tx.send(Err(err));
                            }
                            None => tracing::warn!(?code, message, t = frame.type_name(), "refusé"),
                        },
                        Some(Pending::Directory) => {
                            tracing::warn!(?code, message, "lecture de l'annuaire refusée");
                            if let Some(Reading::AdminInit { reply, .. }) = self.read.take() {
                                let _ = reply.send(Err(err));
                            }
                            self.read_again = false;
                            self.flush_waiting();
                        }
                        None => tracing::warn!(?code, message, "erreur du serveur"),
                    },
                }
            }
            other => tracing::debug!(t = other.type_name(), "trame inattendue ignorée"),
        }
        Ok(())
    }
}

fn known_root(secure: &Secure) -> AdminRoot {
    let mine = secure.info().map(|g| g.members).unwrap_or_default();
    match &secure.directory_state().admin {
        Some((id, known))
            if known.chain.as_ref().is_some_and(|c| {
                c.roster.members.iter().any(|m| mine.iter().any(|x| x.device_id == m.device_id))
            }) =>
        {
            AdminRoot::Ready(id.clone())
        }
        _ => AdminRoot::NotAdmin,
    }
}

enum Pending {
    Pair(Reply<PairingInvitation>),
    Directory,
    AdminInit { cert: SignedBlob, entry: SignedBlob, reply: Reply<AdminRoot> },
    Call { frame: Frame, reference: Option<String>, reply: Option<Reply<Frame>> },
}

fn call_reference(frame: &Frame) -> Option<String> {
    match frame {
        Frame::UserReinvite { user }
        | Frame::UserCertify { user, .. }
        | Frame::UserRevoke { user }
        | Frame::UserRename { user, .. } => Some(user.to_string()),
        Frame::RosterPublish { seq, .. } => Some(seq.to_string()),
        Frame::DeviceUpdate { device, .. }
        | Frame::DeviceRevoke { device }
        | Frame::ServiceCreate { device, .. }
        | Frame::ServiceDevice { device, .. } => Some(device.to_string()),
        Frame::ServiceMute { service, .. } => Some(service.to_string()),
        _ => None,
    }
}

impl Pending {
    fn frame(&self) -> Frame {
        match self {
            Self::Pair(_) => Frame::PairCreate,
            Self::AdminInit { cert, entry, .. } => {
                Frame::AdminInit { cert: cert.clone(), entry: entry.clone() }
            }
            Self::Directory => Frame::DirectoryGet { known: Default::default(), after: None },
            Self::Call { frame, .. } => frame.clone(),
        }
    }

    fn reference(&self) -> Option<&str> {
        match self {
            Self::Call { reference, .. } => reference.as_deref(),
            _ => None,
        }
    }

    fn answered_by(&self, reply: &Frame) -> bool {
        let Self::Call { frame, .. } = self else { return false };
        match (frame, reply) {
            (Frame::UserInvite { .. }, Frame::UserInvited { .. })
            | (Frame::UsersGet, Frame::Users { .. }) => true,
            (Frame::UserReinvite { user: a }, Frame::UserInvited { user: b, .. })
            | (Frame::UserCertify { user: a, .. }, Frame::UserCertified { user: b })
            | (Frame::UserRevoke { user: a }, Frame::UserRevoked { user: b })
            | (Frame::UserRename { user: a, .. }, Frame::UserRenamed { user: b }) => a == b,
            (
                Frame::RosterPublish { user: a, seq: x, .. },
                Frame::RosterPublished { user: b, seq: y },
            ) => a == b && x == y,
            (Frame::DeviceUpdate { device: a, .. }, Frame::DeviceUpdated { device: b })
            | (Frame::DeviceRevoke { device: a }, Frame::DeviceRevoked { device: b }) => a == b,
            (Frame::ServiceMute { service: a, .. }, Frame::ServiceMuted { service: b, .. }) => {
                a == b
            }
            (Frame::ServiceCreate { device: a, .. }, Frame::PairRegistered { device_id: b })
            | (Frame::ServiceDevice { device: a, .. }, Frame::PairRegistered { device_id: b }) => {
                a == b
            }
            (Frame::LinkSet { from: a, to: x, .. }, Frame::LinkUpdated { from: b, to: y, .. }) => {
                a == b && x == y
            }
            _ => false,
        }
    }
}

enum Reading {
    Verify(DirectoryRead),
    AdminInit { create: bool, reply: Reply<AdminRoot> },
}

impl Worker {
    fn take_pending(&mut self, pred: impl Fn(&Pending) -> bool) -> Option<Pending> {
        let i = self.unref.iter().position(pred)?;
        self.unref.remove(i)
    }

    fn start_read(&mut self) -> Vec<Frame> {
        if !self.secure.has_anchor() {
            return Vec::new();
        }
        if self.read.is_some() {
            self.read_again = true;
            return Vec::new();
        }
        let (read, frame) = DirectoryRead::start(self.secure.directory_known());
        self.waiting.append(&mut self.waiting_next);
        self.read = Some(Reading::Verify(read));
        self.unref.push_back(Pending::Directory);
        vec![frame]
    }

    async fn on_directory(
        &mut self,
        me: UserId,
        admin: UserId,
        users: Vec<ot_proto::DirectoryUser>,
        next: Option<UserId>,
    ) -> Vec<Frame> {
        match self.read.take() {
            None => {
                tracing::debug!("page de l'annuaire inattendue");
                Vec::new()
            }
            Some(Reading::AdminInit { create, reply }) => {
                let Some(raw) = users.into_iter().find(|u| u.id == admin) else {
                    let why = "l'admin manque dans l'annuaire".into();
                    let _ = reply.send(Err(ClientError::Invalid(why)));
                    return Vec::new();
                };
                if me != admin {
                    let _ = reply.send(Ok(AdminRoot::NotAdmin));
                    return Vec::new();
                }
                if let Some(cert) = &raw.cert {
                    let mut frames = Vec::new();
                    match self.secure.adopt_existing_root(&admin, cert) {
                        Ok(effects) => {
                            frames = self.effects(effects).await;
                            let _ = reply.send(Ok(AdminRoot::Ready(admin)));
                            frames.extend(self.start_read());
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                    }
                    return frames;
                }
                if !create {
                    let _ = reply.send(Ok(AdminRoot::Missing { name: raw.name }));
                    return Vec::new();
                }
                match self.secure.admin_root(&admin, &raw.name, OffsetDateTime::now_utc()) {
                    Ok((cert, entry)) => {
                        let frame = Frame::AdminInit { cert: cert.clone(), entry: entry.clone() };
                        self.unref.push_back(Pending::AdminInit { cert, entry, reply });
                        vec![frame]
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                        Vec::new()
                    }
                }
            }
            Some(Reading::Verify(mut read)) => match read.page(me, admin, users, next) {
                Err(why) => {
                    tracing::warn!(why, "lecture de l'annuaire abandonnée");
                    self.after_read()
                }
                Ok(ReadStep::Request(frame)) => {
                    self.read = Some(Reading::Verify(read));
                    self.unref.push_back(Pending::Directory);
                    vec![*frame]
                }
                Ok(ReadStep::Round { me, admin, users }) => {
                    let (effects, again) = self.secure.apply_directory(&me, &admin, &users);
                    let mut frames = self.effects(effects).await;
                    let declaration = if self.declared {
                        None
                    } else {
                        self.secure.declaration(self.config.kind, &self.config.features)
                    };
                    if let Some(frame) = declaration {
                        self.declared = true;
                        let reference = call_reference(&frame);
                        let pending =
                            Pending::Call { frame: frame.clone(), reference, reply: None };
                        self.unref.push_back(pending);
                        frames.push(frame);
                    }
                    match again.then(|| read.next_round(self.secure.directory_known())).flatten() {
                        Some(frame) => {
                            self.read = Some(Reading::Verify(read));
                            self.unref.push_back(Pending::Directory);
                            frames.push(frame);
                        }
                        None => frames.extend(self.after_read()),
                    }
                    frames
                }
            },
        }
    }

    fn after_read(&mut self) -> Vec<Frame> {
        self.read = None;
        self.flush_waiting();
        if std::mem::take(&mut self.read_again) { self.start_read() } else { Vec::new() }
    }
}

impl Worker {
    fn flush_waiting(&mut self) {
        for f in std::mem::take(&mut self.waiting) {
            f(&mut self.secure);
        }
    }

    fn fail_calls(&mut self) {
        let mut kept = VecDeque::new();
        for p in self.unref.drain(..) {
            match p {
                Pending::Call { reply, .. } => {
                    if let Some(tx) = reply {
                        let _ = tx.send(Err(ClientError::Connection("connexion perdue".into())));
                    }
                }
                other => kept.push_back(other),
            }
        }
        self.unref = kept;
    }
}

async fn send_frame(ws: &mut Ws, frame: &Frame) -> Result<(), End> {
    ws.send(Message::text(frame.to_json())).await.map_err(|e| End::Lost(format!("envoi : {e}")))
}

async fn recv_message(ws: &mut Ws) -> Result<Option<Frame>, End> {
    match ws.next().await {
        None => Err(End::Lost("connexion fermée".into())),
        Some(Err(e)) => Err(End::Lost(format!("lecture : {e}"))),
        Some(Ok(Message::Close(frame))) => {
            let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
            Err(End::Lost(format!("fermée par le serveur : {reason}")))
        }
        Some(Ok(Message::Text(text))) => match Frame::from_json(&text) {
            Ok(frame) => Ok(Some(frame)),
            Err(e) => {
                tracing::warn!(error = %e, "trame illisible ignorée");
                Ok(None)
            }
        },
        Some(Ok(_)) => Ok(None),
    }
}

async fn recv_frame(ws: &mut Ws) -> Result<Frame, End> {
    loop {
        if let Some(frame) = recv_message(ws).await? {
            return Ok(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let b = Backoff { initial: Duration::from_millis(100), max: Duration::from_secs(1) };
        for attempt in 0..20 {
            let d = b.delay(attempt);
            let base = (100u64 << attempt.min(16)).min(1000);
            assert!(
                d.as_millis() as u64 >= base / 2 && d.as_millis() as u64 <= base,
                "{attempt}: {d:?}"
            );
        }
    }

    #[test]
    fn ws_url_forms() {
        let id = DeviceId::generate();
        let keyring = Arc::new(crate::MemoryKeyring::new());
        let url = |s: &str| {
            ClientConfig::new(s, id.clone(), Identity::generate(), keyring.clone()).ws_url()
        };
        assert_eq!(url("ws://relais:7878"), "ws://relais:7878/v1/ws");
        assert_eq!(url("http://relais:7878/"), "ws://relais:7878/v1/ws");
        assert_eq!(url("relais:7878"), "ws://relais:7878/v1/ws");
    }
}
