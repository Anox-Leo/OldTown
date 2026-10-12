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
        Self { addr, relay, tls: false, stop: Some(stop), task }
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
        Self { addr, relay, tls: true, stop: Some(stop), task }
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
