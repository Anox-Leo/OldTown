use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ot_crypto::{Otp, PairingCode, PairingOffer};
use ot_proto::{
    BlobId, Content, DeviceId, DeviceInfo, Envelope, ErrorCode, Feature, Frame, MAX_FRAME_BYTES,
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
use crate::types::{Accepted, ClientError, Event, Origin, PairingInvitation, Target};
use crate::{Dedup, Identity};

pub(crate) type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Délai maximal pour la poignée de main et l'authentification.
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Attente maximale de la réponse du relais à la fermeture.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Sans aucun message du serveur (qui pinge toutes les 30 s) pendant ce délai,
/// la connexion est considérée morte.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(75);

#[derive(Clone)]
pub struct ClientConfig {
    /// URL du relais, ex. `ws://relais:7878` (`http://` est accepté).
    pub server_url: String,
    pub device_id: DeviceId,
    pub identity: Identity,
    /// Clés de groupe, roster et clé d'échange de l'appareil.
    pub keyring: Arc<dyn Keyring>,
    /// `msg_id` déjà vus lors d'une exécution précédente.
    pub seen: Vec<Ulid>,
    pub backoff: Backoff,
    /// Autorités de confiance pour `wss://`.
    pub tls: TlsRoots,
    /// Silence du serveur (pings compris) au-delà duquel la connexion est
    /// considérée morte ; [`IDLE_TIMEOUT`] par défaut.
    pub idle_timeout: Duration,
    /// Réglages des transferts de fichiers (étape 5).
    pub transfer: TransferConfig,
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

/// URL de l'endpoint WebSocket à partir de l'URL du relais.
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

/// Reconnexion : délai exponentiel plafonné, avec gigue (entre la moitié et
/// la totalité du délai).
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

/// Travail à faire dans la tâche du client, avec le cœur de groupe. `Sync`
/// parce qu'il peut attendre dans le `Worker` (fin de lecture de l'annuaire).
type Job = Box<dyn FnOnce(&mut Secure) + Send + Sync>;

/// Travail qui rend des effets (événements, envois) à appliquer.
type EffectsJob = Box<dyn FnOnce(&mut Secure) -> Result<Vec<Effect>, ClientError> + Send>;

/// Déroulé mené en tâche de fond par le client (étape 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Certifier un invité dont la demande est arrivée.
    Certify(UserId),
    /// Publier mon roster local, plus récent que mon journal.
    Publish,
    /// Admin : republier le journal d'un service dont la dernière entrée
    /// vient d'un appareil retiré de l'admin (D47).
    Republish(UserId),
}

/// Délai avant de relancer un déroulé en échec (hors reconnexion).
const FLOW_RETRY: Duration = Duration::from_secs(60);

/// Rotation : époque adoptée et accusés d'envoi des `group_key`.
type Rotation = (u32, Vec<oneshot::Receiver<Result<Accepted, ClientError>>>);

enum Command {
    Send(Envelope, Reply<Accepted>),
    SendContent(Content, Vec<DeviceId>, Reply<Accepted>),
    ListDevices(Reply<Vec<DeviceInfo>>),
    PairCreate(Reply<PairingInvitation>),
    PairJoin {
        code: PairingCode,
        sponsor: DeviceId,
        name: String,
        reply: Reply<Accepted>,
    },
    Rotate(Vec<DeviceId>, Reply<Rotation>),
    Group(oneshot::Sender<Option<GroupInfo>>),
    PairOffer(String, Reply<String>),
    Sponsor(Box<PairingOffer>, Reply<Accepted>),
    /// Annonce d'un fichier déposé (corps `file`) ; avec une personne, en
    /// `0x05` aux appareils déjà résolus.
    SendFile(Box<FileMeta>, Vec<DeviceId>, Option<UserId>, Reply<Accepted>),
    /// Accès à l'état des fichiers (offres, dépôts en cours) : toutes les
    /// écritures passent par la tâche du client, une à la fois.
    WithSecure(Box<dyn FnOnce(&mut Secure) + Send>),
    /// Comme `WithSecure`, après une relecture de l'annuaire s'il date de
    /// plus de 60 s (envoi à une personne, § 4.3).
    Fresh(Job),
    /// Personnes vérifiées de l'annuaire.
    Directory(oneshot::Sender<Vec<Person>>),
    /// Relire l'annuaire maintenant.
    RefreshDirectory,
    /// Après la fin de la prochaine lecture complète de l'annuaire (relue
    /// au besoin), ou tout de suite sans ancre.
    AfterRead(Job),
    /// Trame dont la réponse ou le refus revient à l'appelant (étape 10 :
    /// administration, journal, profils).
    Call(Frame, Reply<Frame>),
    /// Effets du cœur à appliquer (événements, envois).
    Effects(EffectsJob, Reply<()>),
    /// Un déroulé de fond est terminé (`true` : réussi).
    FlowDone(Flow, bool),
    /// Admin : publier le certificat racine (`admin_init`).
    AdminInit(Reply<UserId>),
    Reconnect,
    Shutdown(oneshot::Sender<()>),
}

/// Handle vers le client. Le client tourne tant qu'un handle existe.
#[derive(Clone)]
pub struct Client {
    device_id: DeviceId,
    pub(crate) server_url: String,
    cmd: mpsc::UnboundedSender<Command>,
    transfers: Transfers,
    /// Une publication de mon journal à la fois (même `seq`).
    pub(crate) publish_lock: Arc<tokio::sync::Mutex<()>>,
}

/// De quoi refaire un [`Client`] dans la tâche du client, sans la tenir en
/// vie (elle s'arrête quand le dernier handle tombe).
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

/// Flux des événements du client.
pub struct Events(mpsc::Receiver<Event>);

impl Events {
    pub async fn next(&mut self) -> Option<Event> {
        self.0.recv().await
    }
}

impl Client {
    /// Démarre le client en tâche de fond ; il se connecte et se reconnecte seul.
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

    /// Chiffre sous la clé de groupe, signe et envoie un contenu (à tous les
    /// autres appareils si `to` est vide ; sinon à des membres du roster).
    /// Hors ligne, l'envoi attend la prochaine connexion. Sans groupe, refuse.
    pub async fn send(
        &self,
        content: &Content,
        to: Vec<DeviceId>,
    ) -> Result<Accepted, ClientError> {
        self.request(|tx| Command::SendContent(content.clone(), to, tx)).await
    }

    /// Chiffre et signe un contenu comme [`Client::send`], sans l'envoyer :
    /// l'appelant connaît ainsi le `msg_id` avant l'envoi (historique des
    /// envois, note 0033), puis passe l'enveloppe à [`Client::send_envelope`].
    pub async fn seal(
        &self,
        content: &Content,
        to: Vec<DeviceId>,
    ) -> Result<Envelope, ClientError> {
        let content = content.clone();
        self.with_secure(move |s| s.seal_content(&content, to)).await
    }

    /// Chiffre, signe et envoie un code à usage unique (note 0045), à des
    /// membres du roster (vide : tous les autres appareils).
    pub async fn send_otp(&self, otp: &Otp, to: Vec<DeviceId>) -> Result<Accepted, ClientError> {
        let otp = otp.clone();
        let env = self.with_secure(move |s| s.seal_otp(&otp, to)).await?;
        self.send_envelope(env).await
    }

    /// Envoie une enveloppe déjà construite (par [`Client::seal`], ou à la
    /// main dans les tests), telle quelle. Réutiliser le même `msg_id` est
    /// sans effet côté serveur.
    pub async fn send_envelope(&self, env: Envelope) -> Result<Accepted, ClientError> {
        self.request(|tx| Command::Send(env, tx)).await
    }

    /// Chiffre un contenu pour une cible (étape 10) sans l'envoyer : mes
    /// appareils sous la clé de groupe, comme [`Client::seal`], ou une autre
    /// personne en `0x05` (ses appareils par défaut ou un appareil précis,
    /// copie à mes appareils), l'annuaire relu s'il date de plus de 60 s.
    pub async fn seal_to(
        &self,
        target: Target,
        content: &Content,
    ) -> Result<Envelope, ClientError> {
        match target {
            Target::MyDevices(to) => self.seal(content, to).await,
            Target::Person { user, device } => {
                let content = content.clone();
                self.with_fresh(move |s| s.seal_content_to(&content, &user, device.as_ref())).await
            }
        }
    }

    /// Chiffre et envoie un contenu à une cible (voir [`Client::seal_to`]).
    pub async fn send_to(
        &self,
        target: Target,
        content: &Content,
    ) -> Result<Accepted, ClientError> {
        let env = self.seal_to(target, content).await?;
        self.send_envelope(env).await
    }

    /// Envoie un code à usage unique à des personnes, en un seul message
    /// `0x05` (pont d'un service, `ot otp-bridge --to-user`).
    pub async fn send_otp_to(&self, users: &[UserId], otp: &Otp) -> Result<Accepted, ClientError> {
        let (users, otp) = (users.to_vec(), otp.clone());
        let env = self.with_fresh(move |s| s.seal_otp_to(&otp, &users)).await?;
        self.send_envelope(env).await
    }

    /// Exécute `f` après la fin de la prochaine lecture complète de
    /// l'annuaire (tout de suite sans ancre).
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

    /// Exécute `f` dans la tâche du client et applique les effets rendus
    /// (événements émis, messages mis en file).
    pub(crate) async fn with_effects(
        &self,
        f: impl FnOnce(&mut Secure) -> Result<Vec<Effect>, ClientError> + Send + 'static,
    ) -> Result<(), ClientError> {
        self.request(|tx| Command::Effects(Box::new(f), tx)).await
    }

    /// Envoie une trame et attend sa réponse (ou son refus, `Rejected`).
    /// Coupée par une perte de connexion : `Connection`.
    pub(crate) async fn call(&self, frame: Frame) -> Result<Frame, ClientError> {
        self.request(|tx| Command::Call(frame, tx)).await
    }

    /// Exécute `f` dans la tâche du client, avec un annuaire de moins de
    /// 60 s s'il y a une ancre (relu sinon, en ligne).
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

    /// Exécute `f` dans la tâche du client, avec le cœur de groupe.
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

    /// Envoie un fichier (étape 5) : dépôt chiffré par morceaux sur le
    /// relais, puis message `file` aux destinataires. Relancé sur la même
    /// source (`source_id`, même taille), reprend le dépôt interrompu.
    pub async fn send_file(
        &self,
        file: OutgoingFile,
        source: Arc<dyn ChunkSource>,
        progress: Arc<dyn Progress>,
    ) -> Result<Accepted, ClientError> {
        self.send_file_info(file, source, progress).await.map(|(accepted, _)| accepted)
    }

    /// Comme [`Client::send_file`], et décrit en plus le fichier annoncé
    /// (sans clé), pour le ranger dans l'historique des envois (note 0033).
    /// `created_at` : l'heure de l'annonce acceptée.
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
            // Une autre personne : les appareils résolus maintenant, ceux à
            // qui le blob sera ouvert.
            Some(user) => {
                let device = file.to.first().cloned();
                self.with_fresh(move |s| s.recipients(&user, device.as_ref())).await?
            }
            None => {
                // Sans groupe, inutile de déposer quoi que ce soit.
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

    /// Abandonne le dépôt en cours de cette source et libère la place sur
    /// le relais.
    pub async fn cancel_upload(&self, source_id: &str) -> Result<(), ClientError> {
        let sid = source_id.to_owned();
        let uploads = self.with_secure(move |s| s.uploads_of(&sid));
        for p in uploads.await? {
            self.transfers.cancel(&p.meta.blob_id).await?;
        }
        let sid = source_id.to_owned();
        self.with_secure(move |s| s.forget_upload(&sid)).await
    }

    /// Fichiers annoncés encore téléchargeables (sans clé).
    pub async fn files(&self) -> Result<Vec<FileInfo>, ClientError> {
        self.with_secure(|s| s.files()).await
    }

    /// Télécharge un fichier annoncé dans `sink`, vérifie son SHA-256, le
    /// finalise et prévient le relais. Un fichier expiré est oublié.
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
        // Le fichier est là : prévenir le relais n'est qu'un nettoyage (sinon
        // il expire de lui-même).
        if let Err(e) = self.transfers.done(blob_id).await {
            tracing::warn!(blob = %blob_id, error = %e, "fin du retrait non signalée au relais");
        }
        let blob = blob_id.clone();
        self.with_secure(move |s| s.forget_file(&blob)).await?;
        tracing::info!(blob = %blob_id, "fichier reçu");
        Ok(file.info())
    }

    /// Oublie un fichier annoncé sans le télécharger.
    pub async fn forget_file(&self, blob_id: &BlobId) -> Result<(), ClientError> {
        let blob = blob_id.clone();
        self.with_secure(move |s| s.forget_file(&blob)).await
    }

    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>, ClientError> {
        self.request(Command::ListDevices).await
    }

    /// Parrain : demande un jeton au relais et renvoie le code d'appairage.
    /// L'appairage se termine par [`Event::PairingCompleted`] si la demande
    /// arrive avant l'expiration, tant que ce client tourne.
    pub async fn pair_create(&self) -> Result<PairingInvitation, ClientError> {
        self.request(Command::PairCreate).await
    }

    /// Nouvel appareil, après [`crate::redeem_pairing`] : envoie la demande au
    /// parrain. Le groupe est adopté à l'arrivée de sa réponse
    /// ([`Event::GroupChanged`]).
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

    /// Tire une nouvelle clé de groupe et l'envoie à chaque membre sauf
    /// `exclude`, qui ne lira plus les messages suivants. Renvoie la nouvelle
    /// époque une fois tous les envois acceptés par le relais.
    pub async fn rotate(&self, exclude: Vec<DeviceId>) -> Result<u32, ClientError> {
        let (epoch, sends) = self.request(|tx| Command::Rotate(exclude, tx)).await?;
        for rx in sends {
            rx.await.map_err(|_| ClientError::Closed)??;
        }
        Ok(epoch)
    }

    /// Nouvel appareil, QR inversé (note 0022) : texte du QR à afficher au
    /// parrain. Le groupe est adopté à l'arrivée de sa réponse
    /// ([`Event::GroupChanged`]), tant que ce client tourne. En attendant
    /// l'enregistrement, l'authentification échoue ([`Event::AuthFailed`]) :
    /// prévoir un `backoff.max` court.
    pub async fn pair_offer(&self, name: &str) -> Result<String, ClientError> {
        let name = name.to_owned();
        self.request(|tx| Command::PairOffer(name, tx)).await
    }

    /// Parrain, QR inversé : fait enregistrer par le relais l'appareil dont
    /// `qr` est le QR scanné, puis lui envoie clés et roster. Renvoie
    /// l'appareil ajouté une fois la réponse acceptée par le relais ;
    /// [`Event::PairingCompleted`] suit.
    pub async fn sponsor(&self, qr: &str) -> Result<(DeviceId, String), ClientError> {
        let offer = PairingOffer::parse(qr).map_err(|e| ClientError::Pairing(e.to_string()))?;
        let added = (offer.device_id.clone(), offer.name.clone());
        self.request(|tx| Command::Sponsor(Box::new(offer), tx)).await?;
        Ok(added)
    }

    /// Le réseau a changé : ferme la connexion en cours s'il y en a une et se
    /// reconnecte tout de suite, sans attendre le délai de reconnexion.
    pub fn reconnect_now(&self) {
        let _ = self.cmd.send(Command::Reconnect);
    }

    /// Personnes vérifiées de l'annuaire (étape 10), moi compris, telles
    /// qu'au dernier tour complet de lecture. Vide sans ancre (installation
    /// à une personne) ou avant la première lecture.
    pub async fn directory(&self) -> Result<Vec<Person>, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Directory(tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)
    }

    /// Comme [`Client::directory`], après une lecture de l'annuaire s'il
    /// date de plus de 60 s (la règle d'un envoi à une personne) ; hors
    /// ligne, attend la connexion. Pour résoudre un nom avant d'agir.
    pub async fn fresh_directory(&self) -> Result<Vec<Person>, ClientError> {
        self.with_fresh(|s| Ok(s.people())).await
    }

    /// Relit l'annuaire maintenant ; sinon il est relu à chaque connexion et
    /// à chaque `directory_changed` du relais. `Event::DirectoryChanged`
    /// signale un changement.
    pub fn refresh_directory(&self) {
        let _ = self.cmd.send(Command::RefreshDirectory);
    }

    /// Appareil de l'admin : publie le certificat racine, auto-signé par cet
    /// appareil, et l'entrée 1 du journal de l'admin, son roster courant
    /// (note 0052) ; son empreinte devient l'ancre de l'annuaire. S'il est
    /// déjà publié par un appareil de mon groupe, l'adopte. Renvoie
    /// l'identifiant de la personne admin.
    pub async fn admin_init(&self) -> Result<UserId, ClientError> {
        self.request(Command::AdminInit).await
    }

    /// État du groupe ; `None` sans groupe.
    pub async fn group(&self) -> Result<Option<GroupInfo>, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Group(tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)
    }

    /// Ferme la connexion et arrête le client ; les envois en attente échouent.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::Shutdown(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

/// Fin d'une session.
enum End {
    /// Reconnexion à prévoir.
    Lost(String),
    /// Reconnexion immédiate demandée par l'appli.
    Reconnect,
    AuthFailed(String),
    Shutdown(Option<oneshot::Sender<()>>),
}

/// Envoi pas encore accepté par le relais.
struct Outgoing {
    env: Envelope,
    /// `None` pour un message système envoyé par le cœur lui-même.
    reply: Option<Reply<Accepted>>,
}

/// Suite d'une commande.
enum Next {
    /// Trames à envoyer si la session est ouverte (sinon, en file).
    Frames(Vec<Frame>),
    Reconnect,
    Shutdown(Option<oneshot::Sender<()>>),
}

/// Parrainage par QR inversé en attente de `pair_registered`.
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
    /// Événements produits au chargement, émis au démarrage.
    startup: Vec<Event>,
    /// Envois pas encore acceptés, dans l'ordre ; renvoyés à chaque connexion.
    outbox: Vec<Outgoing>,
    /// Demandes `list_devices` en attente de réponse, dans l'ordre.
    device_requests: VecDeque<Reply<Vec<DeviceInfo>>>,
    /// Demandes dont le refus revient sans référence (`pair_create`,
    /// `directory_get`, `admin_init`), dans l'ordre d'envoi : le relais
    /// répond dans le même ordre.
    unref: VecDeque<Pending>,
    /// Lecture de l'annuaire en cours.
    read: Option<Reading>,
    /// `directory_changed` reçu pendant une lecture : en refaire une après.
    read_again: bool,
    /// Enregistrements par QR inversé en attente de réponse.
    registrations: Vec<Registration>,
    /// Demandes qui attendent la fin de la lecture de l'annuaire.
    waiting: Vec<Job>,
    /// Demandes qui attendent la fin de la lecture *suivante* (commencée
    /// après elles).
    waiting_next: Vec<Job>,
    /// `users-v1` déjà déclaré par ce client (`device_update`).
    features_sent: bool,
    handle: Handle,
    /// Déroulés de fond en cours.
    flows: Vec<Flow>,
    /// Déroulés en échec, et quand : relancés après `FLOW_RETRY`, ou à la
    /// prochaine connexion.
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
            features_sent: false,
            handle,
            flows: Vec::new(),
            failed: Vec::new(),
        }
    }

    async fn emit(&self, event: Event) {
        // Personne n'écoute : on continue quand même.
        let _ = self.events.send(event).await;
    }

    fn queue(&mut self, env: Envelope, reply: Option<Reply<Accepted>>) -> Next {
        let frame = Frame::Send { env: env.clone() };
        self.outbox.push(Outgoing { env, reply });
        Next::Frames(vec![frame])
    }

    /// Traite une commande, en ligne ou non.
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
                // Sans ancre, rien à relire ; pendant `admin_init`, la lecture
                // vérifiée n'a pas commencé : l'annuaire connu suffit.
                let now = OffsetDateTime::now_utc();
                if !self.secure.has_anchor()
                    || self.secure.directory_fresh(now)
                    || matches!(self.read, Some(Reading::AdminInit(_)))
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
                if !self.secure.has_anchor() || matches!(self.read, Some(Reading::AdminInit(_))) {
                    f(&mut self.secure);
                    return Next::Frames(Vec::new());
                }
                if self.read.is_some() {
                    // La lecture en cours a pu commencer trop tôt.
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
            Command::AdminInit(reply) => {
                let busy = matches!(self.read, Some(Reading::AdminInit(_)))
                    || self.unref.iter().any(|p| matches!(p, Pending::AdminInit { .. }));
                let refused = if self.secure.info().is_none() {
                    Some(ClientError::NoGroup)
                } else if self.secure.has_anchor() {
                    Some(ClientError::Invalid("certificat de l'admin déjà publié".into()))
                } else if busy {
                    Some(ClientError::Invalid("admin_init déjà en cours".into()))
                } else {
                    None
                };
                if let Some(e) = refused {
                    let _ = reply.send(Err(e));
                    return Next::Frames(Vec::new());
                }
                // Une page de l'annuaire d'abord : qui suis-je, qui est
                // l'admin, son certificat manque-t-il ?
                self.read = Some(Reading::AdminInit(reply));
                self.unref.push_back(Pending::Directory);
                Next::Frames(vec![Frame::DirectoryGet { known: Default::default(), after: None }])
            }
            Command::Reconnect => Next::Reconnect,
        }
    }

    /// Applique les effets du cœur : événements émis, messages mis en file.
    /// Renvoie les trames à envoyer.
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

    /// Lance les déroulés de fond dus (étape 10) : certifier les invités
    /// dont la demande est arrivée, publier mon roster s'il est plus récent
    /// que mon journal ; invité, relire l'annuaire après la bienvenue.
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

    /// Attend avant de se reconnecter en mettant les commandes en file.
    /// Renvoie `Some` si un arrêt est demandé.
    async fn wait(&mut self, delay: Duration) -> Option<oneshot::Sender<()>> {
        let until = Instant::now() + delay;
        loop {
            tokio::select! {
                () = sleep_until(until) => return None,
                cmd = self.cmd.recv() => {
                    // Hors ligne, les trames restent en file jusqu'à la connexion.
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
        if let Some(Reading::AdminInit(reply)) = self.read.take() {
            let _ = reply.send(Err(ClientError::Closed));
        }
        // Leurs réponses tombent avec elles : `Closed`.
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
                // Poignée de main de fermeture : le relais traite toutes les
                // trames déjà envoyées (envois, acquittements) avant de
                // répondre ; quitter plus tôt peut les perdre.
                let _ = ws.close(None).await;
                let drain = async { while let Some(Ok(_)) = ws.next().await {} };
                let _ = timeout(CLOSE_TIMEOUT, drain).await;
                End::Shutdown(done)
            }
            end => end,
        }
    }

    async fn session_inner(&mut self, ws: &mut Ws) -> End {
        // Reprise : ce qui n'a pas été accepté repart, dans l'ordre.
        let mut resume: Vec<Frame> =
            self.outbox.iter().map(|o| Frame::Send { env: o.env.clone() }).collect();
        resume.extend(self.device_requests.iter().map(|_| Frame::ListDevices));
        // Les demandes sans référence repartent dans leur ordre ; les pages
        // de l'annuaire sont redemandées par une lecture neuve.
        self.unref.retain(|p| !matches!(p, Pending::Directory));
        resume.extend(self.unref.iter().map(Pending::frame));
        resume.extend(self.registrations.iter().map(Registration::frame));
        // Une connexion neuve relance aussi les déroulés en échec, et
        // redéclare les capacités si le profil ne les a pas.
        self.failed.clear();
        self.features_sent = false;
        // Invité : la demande repart à chaque connexion jusqu'à la bienvenue.
        match self.secure.invite_request() {
            Some(Ok(env)) => {
                self.outbox.push(Outgoing { env: env.clone(), reply: None });
                resume.push(Frame::Send { env });
            }
            Some(Err(e)) => tracing::warn!(error = %e, "demande d'invitation impossible"),
            None => {}
        }
        match &self.read {
            Some(Reading::AdminInit(_)) => {
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
                    // Tout message compte, pings du serveur compris : sans eux,
                    // une session calme tombait toutes les 75 s.
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
                    // Déchiffré et remis, mis de côté ou rejeté : dans tous les
                    // cas acquitté ensuite.
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
                    // Appareil de service (`service_create`, `service_device`).
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
                        // Réponse au nouvel appareil d'abord, puis le roster à tous.
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
                    let _ = reply.send(Ok(user));
                    frames.extend(self.start_read());
                    for frame in &frames {
                        send_frame(ws, frame).await?;
                    }
                }
            }
            // Réponse à un `ping` : ce client n'en envoie pas (les pings
            // WebSocket du relais lui suffisent, note 0023).
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
                // Capacités déclarées : le relais ne prévient pas la session
                // qui change son propre profil.
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
                    // Sans référence : la plus ancienne demande sans réponse.
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
                            if let Some(Reading::AdminInit(reply)) = self.read.take() {
                                let _ = reply.send(Err(err));
                            }
                            self.read_again = false;
                            // Les envois en attente partent sur l'annuaire connu.
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

/// Demande dont le refus revient sans référence.
enum Pending {
    Pair(Reply<PairingInvitation>),
    /// Une page de l'annuaire (`directory_get`).
    Directory,
    /// `admin_init` envoyé ; l'ancre n'est rangée qu'à la réponse.
    AdminInit {
        cert: SignedBlob,
        entry: SignedBlob,
        reply: Reply<UserId>,
    },
    /// Trame d'administration, de journal ou de profil (étape 10). Le
    /// relais la refuse avec `reference`, ou sans référence (`forbidden`).
    Call {
        frame: Frame,
        reference: Option<String>,
        /// `None` : personne n'attend la réponse.
        reply: Option<Reply<Frame>>,
    },
}

/// Référence d'un refus de `frame` par le relais (`session/people.rs`).
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
    /// Trame à renvoyer à la reconnexion (`directory_get` : jamais appelé,
    /// une lecture neuve la remplace).
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

    /// `reply` est la réponse de cette demande.
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

/// Lecture de l'annuaire en cours.
enum Reading {
    /// Lecture vérifiée, page par page, tour par tour.
    Verify(DirectoryRead),
    /// `admin_init` : une page pour savoir qui est l'admin.
    AdminInit(Reply<UserId>),
}

impl Worker {
    /// Retire la première demande en attente qui répond à `pred`.
    fn take_pending(&mut self, pred: impl Fn(&Pending) -> bool) -> Option<Pending> {
        let i = self.unref.iter().position(pred)?;
        self.unref.remove(i)
    }

    /// Commence une lecture vérifiée de l'annuaire, s'il y a une ancre ;
    /// pendant une lecture, en prévoit une autre après.
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

    /// Range une page de l'annuaire ; renvoie les trames à envoyer.
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
            Some(Reading::AdminInit(reply)) => {
                let Some(raw) = users.into_iter().find(|u| u.id == admin) else {
                    let why = "l'admin manque dans l'annuaire".into();
                    let _ = reply.send(Err(ClientError::Invalid(why)));
                    return Vec::new();
                };
                if me != admin {
                    let why = "cet appareil n'est pas à l'admin".into();
                    let _ = reply.send(Err(ClientError::Invalid(why)));
                    return Vec::new();
                }
                if let Some(cert) = &raw.cert {
                    // Publié par un autre appareil de l'admin : l'adopter.
                    let mut frames = Vec::new();
                    match self.secure.adopt_existing_root(&admin, cert) {
                        Ok(effects) => {
                            frames = self.effects(effects).await;
                            let _ = reply.send(Ok(admin));
                            frames.extend(self.start_read());
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                    }
                    return frames;
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
                    if !self.features_sent && self.secure.needs_features() {
                        // Seul l'appareil déclare ses capacités : il sait
                        // ouvrir `0x05`, les autres peuvent lui écrire.
                        self.features_sent = true;
                        let frame = Frame::DeviceUpdate {
                            device: self.config.device_id.clone(),
                            kind: None,
                            reachable: None,
                            default: None,
                            features: Some(vec![Feature::users_v1()]),
                        };
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

    /// Fin d'une lecture : les demandes en attente passent, puis une autre
    /// lecture si l'annuaire a changé entre-temps.
    fn after_read(&mut self) -> Vec<Frame> {
        self.read = None;
        self.flush_waiting();
        if std::mem::take(&mut self.read_again) { self.start_read() } else { Vec::new() }
    }
}

impl Worker {
    /// Exécute les demandes qui attendaient l'annuaire.
    fn flush_waiting(&mut self) {
        for f in std::mem::take(&mut self.waiting) {
            f(&mut self.secure);
        }
    }

    /// Fin de session : les trames d'administration envoyées n'auront pas de
    /// réponse (elles ne sont pas rejouées, `user_invite` n'est pas
    /// idempotente) ; celles posées hors ligne partiront à la connexion.
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

/// Lit le prochain message : `Some` pour une trame du protocole, `None` pour
/// un ping, un pong ou une trame illisible (les pongs sont envoyés par
/// tungstenite).
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

/// Prochaine trame du protocole, en ignorant le reste.
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
