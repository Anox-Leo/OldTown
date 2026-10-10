use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ot_proto::{
    Content, DeviceId, DeviceInfo, Envelope, ErrorCode, Frame, MAX_FRAME_BYTES, Ulid, WS_PATH,
};
use time::OffsetDateTime;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::{Dedup, Identity};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Délai maximal pour la poignée de main et l'authentification.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Sans aucune trame du serveur (qui pinge toutes les 30 s) pendant ce délai,
/// la connexion est considérée morte.
const IDLE_TIMEOUT: Duration = Duration::from_secs(75);

#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// URL du relais, ex. `ws://relais:7878` (`http://` est accepté).
    pub server_url: String,
    pub device_id: DeviceId,
    pub identity: Identity,
    /// `msg_id` déjà vus lors d'une exécution précédente.
    pub seen: Vec<Ulid>,
    pub backoff: Backoff,
}

impl ClientConfig {
    pub fn new(server_url: impl Into<String>, device_id: DeviceId, identity: Identity) -> Self {
        Self {
            server_url: server_url.into(),
            device_id,
            identity,
            seen: Vec::new(),
            backoff: Backoff::default(),
        }
    }

    fn ws_url(&self) -> String {
        let base = self.server_url.trim_end_matches('/');
        let base = match base.split_once("://") {
            Some(("http", rest)) => format!("ws://{rest}"),
            Some(("https", rest)) => format!("wss://{rest}"),
            Some(_) => base.to_owned(),
            None => format!("ws://{base}"),
        };
        format!("{base}{WS_PATH}")
    }
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

/// Message reçu et pas encore vu.
#[derive(Debug, Clone)]
pub struct Received {
    pub seq: i64,
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub created_at: OffsetDateTime,
    /// Contenu en clair ; `None` s'il est chiffré ou illisible.
    pub content: Option<Content>,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Session ouverte ; `pending` messages vont suivre.
    Connected {
        pending: u64,
    },
    /// Connexion perdue ou refusée ; le client réessaie seul.
    Disconnected {
        reason: String,
    },
    /// Le serveur a refusé l'authentification (appareil inconnu, révoqué ou
    /// mauvaise clé). Le client réessaie quand même, avec le délai maximal.
    AuthFailed {
        message: String,
    },
    Message(Received),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub msg_id: Ulid,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("refusé par le serveur ({code:?}) : {message}")]
    Rejected { code: ErrorCode, message: String },
    #[error("client arrêté")]
    Closed,
}

enum Command {
    Send(Envelope, oneshot::Sender<Result<Accepted, ClientError>>),
    ListDevices(oneshot::Sender<Result<Vec<DeviceInfo>, ClientError>>),
    Shutdown(oneshot::Sender<()>),
}

/// Handle vers le client. Le client tourne tant qu'un handle existe.
#[derive(Clone)]
pub struct Client {
    device_id: DeviceId,
    cmd: mpsc::UnboundedSender<Command>,
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
        let client = Self { device_id: config.device_id.clone(), cmd: cmd_tx };
        tokio::spawn(Worker::new(config, cmd_rx, ev_tx).run());
        (client, Events(ev_rx))
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// Envoie un contenu (à tous les autres appareils si `to` est vide). Hors
    /// ligne, l'envoi attend la prochaine connexion.
    pub async fn send(
        &self,
        content: &Content,
        to: Vec<DeviceId>,
    ) -> Result<Accepted, ClientError> {
        self.send_envelope(Envelope::new_plain(self.device_id.clone(), to, content)).await
    }

    /// Envoie une enveloppe déjà construite. Réutiliser le même `msg_id` est
    /// sans effet côté serveur.
    pub async fn send_envelope(&self, env: Envelope) -> Result<Accepted, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::Send(env, tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)?
    }

    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>, ClientError> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(Command::ListDevices(tx)).map_err(|_| ClientError::Closed)?;
        rx.await.map_err(|_| ClientError::Closed)?
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
    AuthFailed(String),
    Shutdown(Option<oneshot::Sender<()>>),
}

struct Worker {
    config: ClientConfig,
    cmd: mpsc::UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    dedup: Dedup,
    /// Envois pas encore acceptés, dans l'ordre ; renvoyés à chaque connexion.
    outbox: Vec<(Envelope, oneshot::Sender<Result<Accepted, ClientError>>)>,
    /// Demandes `list_devices` en attente de réponse, dans l'ordre.
    device_requests: VecDeque<oneshot::Sender<Result<Vec<DeviceInfo>, ClientError>>>,
}

impl Worker {
    fn new(
        mut config: ClientConfig,
        cmd: mpsc::UnboundedReceiver<Command>,
        events: mpsc::Sender<Event>,
    ) -> Self {
        let dedup = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, std::mem::take(&mut config.seen));
        Self { config, cmd, events, dedup, outbox: Vec::new(), device_requests: VecDeque::new() }
    }

    async fn emit(&self, event: Event) {
        // Personne n'écoute : on continue quand même.
        let _ = self.events.send(event).await;
    }

    async fn run(mut self) {
        let mut attempt = 0u32;
        loop {
            let end = match self.connect().await {
                Ok((ws, pending)) => {
                    attempt = 0;
                    self.emit(Event::Connected { pending }).await;
                    self.session(ws).await
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
                cmd = self.cmd.recv() => match cmd {
                    None => return Some(oneshot::channel().0),
                    Some(Command::Shutdown(done)) => return Some(done),
                    Some(Command::Send(env, tx)) => self.outbox.push((env, tx)),
                    Some(Command::ListDevices(tx)) => self.device_requests.push_back(tx),
                },
            }
        }
    }

    fn fail_all(&mut self) {
        for (_, tx) in self.outbox.drain(..) {
            let _ = tx.send(Err(ClientError::Closed));
        }
        for tx in self.device_requests.drain(..) {
            let _ = tx.send(Err(ClientError::Closed));
        }
    }

    async fn connect(&mut self) -> Result<(Ws, u64), End> {
        let url = self.config.ws_url();
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES));
        let connect = tokio_tungstenite::connect_async_with_config(&url, Some(ws_config), true);
        let (mut ws, _) = match timeout(HANDSHAKE_TIMEOUT, connect).await {
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
                End::Shutdown(done)
            }
            end => end,
        }
    }

    async fn session_inner(&mut self, ws: &mut Ws) -> End {
        // Reprise : ce qui n'a pas été accepté repart, dans l'ordre.
        for (env, _) in &self.outbox {
            if let Err(end) = send_frame(ws, &Frame::Send { env: env.clone() }).await {
                return end;
            }
        }
        for _ in &self.device_requests {
            if let Err(end) = send_frame(ws, &Frame::ListDevices).await {
                return end;
            }
        }

        let mut idle = Instant::now() + IDLE_TIMEOUT;
        loop {
            tokio::select! {
                () = sleep_until(idle) => return End::Lost("plus de nouvelles du serveur".into()),
                cmd = self.cmd.recv() => {
                    let res = match cmd {
                        None => return End::Shutdown(None),
                        Some(Command::Shutdown(done)) => return End::Shutdown(Some(done)),
                        Some(Command::Send(env, tx)) => {
                            let frame = Frame::Send { env: env.clone() };
                            self.outbox.push((env, tx));
                            send_frame(ws, &frame).await
                        }
                        Some(Command::ListDevices(tx)) => {
                            self.device_requests.push_back(tx);
                            send_frame(ws, &Frame::ListDevices).await
                        }
                    };
                    if let Err(end) = res {
                        return end;
                    }
                }
                frame = recv_frame(ws) => {
                    idle = Instant::now() + IDLE_TIMEOUT;
                    let res = match frame {
                        Ok(frame) => self.on_frame(ws, frame).await,
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
                    let received = Received {
                        seq,
                        msg_id: env.msg_id,
                        from: env.from,
                        created_at: env.created_at,
                        content: env.payload.plain_content(),
                    };
                    self.emit(Event::Message(received)).await;
                } else {
                    tracing::debug!(seq, msg_id = %env.msg_id, "doublon ignoré");
                }
                send_frame(ws, &Frame::Ack { seq }).await?;
            }
            Frame::Accepted { msg_id, seq } => {
                if let Some(i) = self.outbox.iter().position(|(e, _)| e.msg_id == msg_id) {
                    let (_, tx) = self.outbox.remove(i);
                    let _ = tx.send(Ok(Accepted { msg_id, seq }));
                }
            }
            Frame::Devices { devices } => {
                if let Some(tx) = self.device_requests.pop_front() {
                    let _ = tx.send(Ok(devices));
                }
            }
            Frame::Error { code: ErrorCode::AuthFailed, message, .. } => {
                return Err(End::AuthFailed(message));
            }
            Frame::Error { code, message, reference } => {
                let pos = reference
                    .as_deref()
                    .and_then(|r| self.outbox.iter().position(|(e, _)| e.msg_id.to_string() == r));
                match pos {
                    Some(i) => {
                        let (_, tx) = self.outbox.remove(i);
                        let _ = tx.send(Err(ClientError::Rejected { code, message }));
                    }
                    None => tracing::warn!(?code, message, "erreur du serveur"),
                }
            }
            other => tracing::debug!(t = other.type_name(), "trame inattendue ignorée"),
        }
        Ok(())
    }
}

async fn send_frame(ws: &mut Ws, frame: &Frame) -> Result<(), End> {
    ws.send(Message::text(frame.to_json())).await.map_err(|e| End::Lost(format!("envoi : {e}")))
}

/// Lit la prochaine trame du protocole ; les pings sont traités par tungstenite.
async fn recv_frame(ws: &mut Ws) -> Result<Frame, End> {
    loop {
        match ws.next().await {
            None => return Err(End::Lost("connexion fermée".into())),
            Some(Err(e)) => return Err(End::Lost(format!("lecture : {e}"))),
            Some(Ok(Message::Close(frame))) => {
                let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                return Err(End::Lost(format!("fermée par le serveur : {reason}")));
            }
            Some(Ok(Message::Text(text))) => match Frame::from_json(&text) {
                Ok(frame) => return Ok(frame),
                Err(e) => tracing::warn!(error = %e, "trame illisible ignorée"),
            },
            Some(Ok(_)) => {}
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
        let url = |s: &str| ClientConfig::new(s, id.clone(), Identity::generate()).ws_url();
        assert_eq!(url("ws://relais:7878"), "ws://relais:7878/v1/ws");
        assert_eq!(url("http://relais:7878/"), "ws://relais:7878/v1/ws");
        assert_eq!(url("relais:7878"), "ws://relais:7878/v1/ws");
    }
}
