//! Outils communs aux tests d'intégration : serveur lancé dans le processus,
//! appareils de test et client WebSocket brut.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ot_core::crypto::{GroupKey, Member};
use ot_core::{
    Backoff, Client, ClientConfig, Event, Events, GroupKeys, Identity, Keyring, MemoryKeyring,
    Roster, ensure_exchange_key,
};
use ot_proto::{DeviceId, Frame};
use ot_server::store::Store;
use ot_server::tls::Tls;
use ot_server::{Config, Relay};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub const STEP: Duration = Duration::from_secs(5);

pub fn test_config() -> Config {
    Config { ping_interval: Duration::from_secs(1), ..Config::default() }
}

pub struct TestServer {
    pub addr: SocketAddr,
    /// Routes d'administration (`/metrics`).
    pub admin: SocketAddr,
    pub relay: Relay,
    /// En TLS : l'URL est en `wss://localhost`.
    tls: bool,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl TestServer {
    pub async fn start(store: Store, config: Config) -> Self {
        Self::start_on("127.0.0.1:0".parse().unwrap(), store, config).await
    }

    pub async fn start_on(addr: SocketAddr, store: Store, config: Config) -> Self {
        let listener = TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let relay = Relay::new(store, config);
        let (stop, stopped) = oneshot::channel();
        let serve = ot_server::serve(relay.clone(), listener, async {
            let _ = stopped.await;
        });
        let task = tokio::spawn(async move { serve.await.unwrap() });
        let admin = start_admin(&relay).await;
        Self { addr, admin, relay, tls: false, stop: Some(stop), task }
    }

    /// Relais en TLS (`wss://`), avec le certificat donné.
    pub async fn start_tls(store: Store, config: Config, tls: Tls) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let relay = Relay::new(store, config);
        let (stop, stopped) = oneshot::channel();
        let serve = ot_server::serve_tls(relay.clone(), listener, tls, async {
            let _ = stopped.await;
        });
        let task = tokio::spawn(async move { serve.await.unwrap() });
        let admin = start_admin(&relay).await;
        Self { addr, admin, relay, tls: true, stop: Some(stop), task }
    }

    pub fn url(&self) -> String {
        if self.tls {
            format!("wss://localhost:{}", self.addr.port())
        } else {
            format!("ws://{}", self.addr)
        }
    }

    pub async fn stop(self) {
        let store = self.relay.store().clone();
        self.stop_keep_store().await;
        store.close().await;
    }

    /// Arrête le serveur sans fermer la base (utile pour une base en mémoire).
    pub async fn stop_keep_store(mut self) {
        let _ = self.stop.take().unwrap().send(());
        tokio::time::timeout(STEP, &mut self.task).await.expect("arrêt du serveur").unwrap();
    }
}

/// Routes d'administration sur un port local à part, arrêtées avec le test.
async fn start_admin(relay: &Relay) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(ot_server::serve_admin(relay.clone(), listener, std::future::pending()));
    addr
}

pub struct TestDevice {
    pub id: DeviceId,
    pub name: String,
    pub identity: Identity,
    pub keyring: Arc<MemoryKeyring>,
}

/// Appareil enregistré sur le relais, sans groupe.
pub async fn add_device(store: &Store, name: &str) -> TestDevice {
    let identity = Identity::generate();
    let id = store.add_device(name, &identity.public_key(), 0).await.unwrap().id;
    TestDevice { id, name: name.into(), identity, keyring: Arc::new(MemoryKeyring::new()) }
}

/// Met les appareils dans un même groupe (époque 1), comme après appairage.
pub fn make_group(devices: &[&TestDevice]) {
    let members = devices
        .iter()
        .map(|d| Member {
            device_id: d.id.clone(),
            name: d.name.clone(),
            sign_key: d.identity.public_key(),
            exchange_key: ensure_exchange_key(&*d.keyring).unwrap().public_key(),
            added_epoch: 1,
        })
        .collect();
    let roster = Roster { epoch: 1, version: 1, members };
    let keys = GroupKeys::new(1, GroupKey::generate());
    for d in devices {
        d.keyring.save_group_keys(&keys).unwrap();
        d.keyring.save_roster(&roster).unwrap();
    }
}

impl TestDevice {
    pub fn config(&self, url: &str) -> ClientConfig {
        let keyring: Arc<dyn Keyring> = self.keyring.clone();
        let mut c = ClientConfig::new(url, self.id.clone(), self.identity.clone(), keyring);
        c.backoff = Backoff { initial: Duration::from_millis(50), max: Duration::from_millis(200) };
        c
    }

    pub fn client(&self, url: &str) -> (Client, Events) {
        Client::start(self.config(url))
    }
}

pub async fn open_store(path: &Path) -> Store {
    Store::open(path).await.unwrap()
}

/// Prochain événement, en échouant au-delà de `STEP`.
pub async fn next_event(events: &mut Events) -> Event {
    tokio::time::timeout(STEP, events.next()).await.expect("événement attendu").expect("flux fermé")
}

/// Attend la connexion. Le délai est global : un client qui échoue en boucle
/// produit sans cesse des événements et ne doit pas bloquer le test.
pub async fn wait_connected(events: &mut Events) -> u64 {
    within(async {
        loop {
            match next_event(events).await {
                Event::Connected { pending } => return pending,
                other => eprintln!("en attente de connexion : {other:?}"),
            }
        }
    })
    .await
}

/// Prochain message reçu, en ignorant les événements de connexion.
pub async fn next_message(events: &mut Events) -> ot_core::Received {
    within(async {
        loop {
            if let Event::Message(m) = next_event(events).await {
                return m;
            }
        }
    })
    .await
}

/// Échoue si `fut` ne se termine pas dans `STEP`.
pub async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(STEP, fut).await.expect("délai dépassé")
}

/// Vérifie qu'aucun message n'arrive pendant `d`.
pub async fn no_message(events: &mut Events, d: Duration) {
    let deadline = tokio::time::Instant::now() + d;
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.next()).await {
        assert!(!matches!(ev, Event::Message(_)), "message inattendu : {ev:?}");
    }
}

/// Client WebSocket brut, pour tester le protocole trame par trame.
pub struct Raw(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl Raw {
    pub fn from_stream(ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        Self(ws)
    }

    pub async fn connect(server: &TestServer) -> Self {
        let url = format!("{}{}", server.url(), ot_proto::WS_PATH);
        Self(tokio_tungstenite::connect_async(url).await.unwrap().0)
    }

    pub async fn send(&mut self, frame: &Frame) {
        self.send_text(frame.to_json()).await;
    }

    pub async fn send_text(&mut self, text: String) {
        self.0.send(Message::text(text)).await.unwrap();
    }

    /// Prochaine trame ; `None` si la connexion est fermée.
    pub async fn recv(&mut self) -> Option<Frame> {
        loop {
            let msg = tokio::time::timeout(STEP, self.0.next()).await.expect("trame attendue");
            match msg {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return None,
                Some(Ok(Message::Text(t))) => return Some(Frame::from_json(&t).unwrap()),
                Some(Ok(_)) => {}
            }
        }
    }

    pub async fn challenge(&mut self) -> [u8; 32] {
        match self.recv().await {
            Some(Frame::Challenge { nonce }) => nonce,
            other => panic!("challenge attendu, reçu {other:?}"),
        }
    }

    /// Connexion authentifiée ; renvoie le nombre de messages en attente.
    pub async fn login(server: &TestServer, dev: &TestDevice) -> (Self, u64) {
        let mut raw = Self::connect(server).await;
        let nonce = raw.challenge().await;
        let sig = dev.identity.sign_challenge(&nonce, &dev.id);
        raw.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
        match raw.recv().await {
            Some(Frame::Welcome { device_id, pending }) => {
                assert_eq!(device_id, dev.id);
                (raw, pending)
            }
            other => panic!("welcome attendu, reçu {other:?}"),
        }
    }

    pub async fn close(mut self) {
        let _ = self.0.close(None).await;
    }
}

/// Requête HTTP/1.0 minimale ; renvoie (code, corps).
pub async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: test\r\n\r\n").as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    let code = buf[9..12].parse().unwrap();
    let body = buf.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default();
    (code, body)
}

/// Attend un événement qui satisfait `pred`, en ignorant les autres.
pub async fn wait_for(events: &mut Events, pred: impl Fn(&Event) -> bool) -> Event {
    within(async {
        loop {
            let ev = next_event(events).await;
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
}

/// Sens d'une trame qui traverse le [`proxy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    ToServer,
    ToClient,
}

pub type TamperFn = std::sync::Arc<dyn Fn(Dir, Frame) -> Frame + Send + Sync>;

/// Relais malveillant : un proxy WebSocket devant le vrai relais, qui peut
/// modifier chaque trame. Renvoie l'URL à donner aux clients.
pub async fn proxy(upstream: &TestServer, tamper: TamperFn) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = format!("{}{}", upstream.url(), ot_proto::WS_PATH);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (upstream, tamper) = (upstream.clone(), tamper.clone());
            tokio::spawn(async move {
                let Ok(client) = tokio_tungstenite::accept_async(tcp).await else { return };
                let Ok((server, _)) = tokio_tungstenite::connect_async(&upstream).await else {
                    return;
                };
                let (mut c_tx, mut c_rx) = client.split();
                let (mut s_tx, mut s_rx) = server.split();
                let forward = |dir: Dir, msg: Message| match msg {
                    Message::Text(t) => match Frame::from_json(&t) {
                        Ok(frame) => Message::text(tamper(dir, frame).to_json()),
                        Err(_) => Message::Text(t),
                    },
                    other => other,
                };
                loop {
                    tokio::select! {
                        m = c_rx.next() => match m {
                            Some(Ok(m)) => if s_tx.send(forward(Dir::ToServer, m)).await.is_err() { break },
                            _ => break,
                        },
                        m = s_rx.next() => match m {
                            Some(Ok(m)) => if c_tx.send(forward(Dir::ToClient, m)).await.is_err() { break },
                            _ => break,
                        },
                    }
                }
                let _ = c_tx.close().await;
                let _ = s_tx.close().await;
            });
        }
    });
    format!("ws://{addr}")
}

/// Inverse un bit du chiffré d'une enveloppe envoyée ou livrée.
pub fn flip_payload(frame: Frame) -> Frame {
    match frame {
        Frame::Send { mut env } => {
            if let Some(b) = env.payload.data.last_mut() {
                *b ^= 1;
            }
            Frame::Send { env }
        }
        Frame::Deliver { seq, mut env } => {
            if let Some(b) = env.payload.data.last_mut() {
                *b ^= 1;
            }
            Frame::Deliver { seq, env }
        }
        other => other,
    }
}

/// Autorité de certification de test : signe des certificats pour `localhost`.
pub struct TestCa(rcgen::CertifiedIssuer<'static, rcgen::KeyPair>);

impl TestCa {
    pub fn new(name: &str) -> Self {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, name);
        let key = rcgen::KeyPair::generate().unwrap();
        Self(rcgen::CertifiedIssuer::self_signed(params, key).unwrap())
    }

    /// Certificat racine, en DER (à donner aux clients).
    pub fn der(&self) -> Vec<u8> {
        self.0.der().to_vec()
    }

    /// Écrit un certificat `localhost` signé par cette autorité et sa clé
    /// dans `dir` ; renvoie leurs chemins.
    pub fn write_leaf(&self, dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &self.0).unwrap();
        let (cert_path, key_path) = (dir.join("relais.crt"), dir.join("relais.key"));
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key.serialize_pem()).unwrap();
        (cert_path, key_path)
    }
}

/// Réponse HTTP brute : code et corps.
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Corps JSON d'une réponse.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!("JSON attendu ({e}) : {}", String::from_utf8_lossy(&self.body))
        })
    }

    /// Code d'erreur de l'API.
    pub fn error(&self) -> ot_proto::http::HttpErrorCode {
        self.json::<ot_proto::http::HttpError>().error
    }
}

/// Requête HTTP/1.1 minimale (`Connection: close`).
pub async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> HttpResponse {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut buf = Vec::new();
    within(s.read_to_end(&mut buf)).await.unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").expect("réponse HTTP");
    let status = std::str::from_utf8(&buf[9..12]).unwrap().parse().unwrap();
    HttpResponse { status, body: buf[split + 4..].to_vec() }
}

/// En-têtes d'authentification d'une requête signée par `dev`, à l'heure
/// `time_ms` avec `nonce`.
pub fn signed_headers(
    dev: &TestDevice,
    method: &str,
    path: &str,
    body: &[u8],
    time_ms: i64,
    nonce: [u8; 16],
) -> Vec<(&'static str, String)> {
    use ed25519_dalek::Signer;
    use ot_proto::http::*;
    use sha2::Digest;
    let hash: [u8; 32] = sha2::Sha256::digest(body).into();
    let msg = http_request_message(method, path, time_ms as u64, &nonce, &hash);
    let key = ed25519_dalek::SigningKey::from_bytes(&dev.identity.to_bytes());
    vec![
        (HEADER_DEVICE, dev.id.to_string()),
        (HEADER_TIME, time_ms.to_string()),
        (HEADER_NONCE, ot_proto::b64::encode(&nonce)),
        (HEADER_SIGNATURE, ot_proto::b64::encode(&key.sign(&msg).to_bytes())),
    ]
}

/// Requête signée par `dev`, à l'heure courante, avec un nonce neuf.
pub async fn signed(
    server: &TestServer,
    dev: &TestDevice,
    method: &str,
    path: &str,
    body: &[u8],
) -> HttpResponse {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce).unwrap();
    let headers = signed_headers(dev, method, path, body, ot_server::now_ms(), nonce);
    http(server.addr, method, path, &headers, body).await
}

/// Interrupteur TCP devant le relais : `cut` coupe les connexions ouvertes
/// et refuse les nouvelles, `restore` les laisse repasser (coupure réseau).
pub struct Gate {
    pub addr: SocketAddr,
    open: Arc<std::sync::atomic::AtomicBool>,
    conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl Gate {
    pub async fn start(target: SocketAddr) -> Self {
        use std::sync::atomic::Ordering;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let open = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let conns = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (o, c) = (open.clone(), conns.clone());
        tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                if !o.load(Ordering::SeqCst) {
                    continue;
                }
                let task = tokio::spawn(async move {
                    if let Ok(mut outbound) = TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
                c.lock().unwrap().push(task.abort_handle());
            }
        });
        Self { addr, open, conns }
    }

    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub fn cut(&self) {
        self.open.store(false, std::sync::atomic::Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            c.abort();
        }
    }

    pub fn restore(&self) {
        self.open.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Fichier en mémoire, comme source d'envoi.
pub struct MemSource {
    pub data: Vec<u8>,
    pub reads: std::sync::atomic::AtomicU64,
}

impl MemSource {
    pub fn new(data: Vec<u8>) -> Arc<Self> {
        Arc::new(Self { data, reads: Default::default() })
    }
}

impl ot_core::ChunkSource for MemSource {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let start = (offset as usize).min(self.data.len());
        let end = (start + len as usize).min(self.data.len());
        Ok(self.data[start..end].to_vec())
    }
}

/// Destination en mémoire : ce qui est écrit, et s'il a été finalisé ou
/// abandonné.
#[derive(Default)]
pub struct MemSink {
    pub data: std::sync::Mutex<Vec<u8>>,
    pub finished: std::sync::atomic::AtomicBool,
    pub aborted: std::sync::atomic::AtomicBool,
}

impl ot_core::ChunkSink for MemSink {
    fn write_at(&self, offset: u64, data: &[u8]) -> Result<(), String> {
        let mut buf = self.data.lock().unwrap();
        let end = offset as usize + data.len();
        if buf.len() < end {
            buf.resize(end, 0);
        }
        buf[offset as usize..end].copy_from_slice(data);
        Ok(())
    }
    fn finish(&self) -> Result<(), String> {
        self.finished.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn abort(&self) {
        self.aborted.store(true, std::sync::atomic::Ordering::SeqCst);
        self.data.lock().unwrap().clear();
    }
}

/// Octets pseudo-aléatoires reproductibles.
pub fn sample(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}
