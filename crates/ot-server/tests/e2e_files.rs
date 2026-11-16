//! Étape 5, T3 : transferts de fichiers de bout en bout, clients `ot-core`
//! et relais en processus. Reprise après coupure réseau et après
//! redémarrage, expiration, fichier altéré, session WebSocket préservée.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use common::*;
use ot_core::{
    ChunkSource, Client, ClientConfig, ClientError, Content, Event, Events, FileInfo, OutgoingFile,
    TransferConfig,
};
use ot_server::Config;
use tempfile::TempDir;

/// Morceaux de 64 Kio : un fichier de quelques Mio en compte des dizaines.
const CHUNK: u32 = 64 * 1024;

/// Borne d'un transfert de quelques Mio : chaque morceau est synchronisé sur
/// disque par le relais, et `STEP` ne suffit pas sur un exécuteur de CI lent.
const TRANSFER: Duration = Duration::from_secs(20);

/// Échoue si le transfert `fut` ne se termine pas dans `TRANSFER`.
async fn transfer<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(TRANSFER, fut).await.expect("transfert trop long")
}

fn transfer_config() -> TransferConfig {
    TransferConfig {
        chunk_size: CHUNK,
        stall_timeout: Duration::from_secs(10),
        retry: ot_core::Backoff {
            initial: Duration::from_millis(20),
            max: Duration::from_millis(200),
        },
        progress_interval: Duration::ZERO,
        ..TransferConfig::default()
    }
}

fn config(dev: &TestDevice, url: &str) -> ClientConfig {
    let mut c = dev.config(url);
    c.transfer = transfer_config();
    c
}

async fn online(dev: &TestDevice, url: &str) -> (Client, Events) {
    let (client, mut events) = Client::start(config(dev, url));
    wait_connected(&mut events).await;
    (client, events)
}

struct Setup {
    server: TestServer,
    dir: TempDir,
    a: TestDevice,
    b: TestDevice,
}

async fn setup_with(f: impl FnOnce(Config) -> Config) -> Setup {
    let dir = TempDir::new().unwrap();
    // Base sur disque, comme en production : une requête annulée par une
    // coupure peut faire jeter sa connexion SQLite, et une base en mémoire
    // repartirait vide avec la suivante.
    let store = open_store(&dir.path().join("ot.db")).await;
    let a = add_device(&store, "telephone").await;
    let b = add_device(&store, "mac").await;
    make_group(&[&a, &b]);
    let config = f(Config { blob_dir: dir.path().join("blobs"), ..test_config() });
    Setup { server: TestServer::start(store, config).await, dir, a, b }
}

fn outgoing(to: &TestDevice) -> OutgoingFile {
    OutgoingFile {
        source_id: "content://media/video/42".into(),
        name: "vidéo de vacances.mp4".into(),
        mime: "video/mp4".into(),
        to: vec![to.id.clone()],
    }
}

fn no_progress() -> Arc<dyn ot_core::Progress> {
    Arc::new(|_: u64, _: u64| {})
}

/// Prochain fichier annoncé, en ignorant les autres événements.
async fn next_file(events: &mut Events) -> FileInfo {
    match wait_for(events, |e| matches!(e, Event::File(_))).await {
        Event::File(f) => f.file,
        _ => unreachable!(),
    }
}

fn blob_dirs(dir: &TempDir) -> usize {
    std::fs::read_dir(dir.path().join("blobs")).map_or(0, |d| d.count())
}

#[tokio::test]
async fn file_roundtrip_while_messages_keep_flowing() {
    let s = setup_with(|c| c).await;
    let url = s.server.url();
    let (ca, mut ea) = online(&s.a, &url).await;
    let (cb, mut eb) = online(&s.b, &url).await;
    let data = sample((5 << 20) + 123);

    let sender = ca.clone();
    let (src, file) = (MemSource::new(data.clone()), outgoing(&s.b));
    let upload = tokio::spawn(async move { sender.send_file(file, src, no_progress()).await });
    // Pendant le dépôt, la session WebSocket sert toujours dans les deux sens.
    within(cb.send(&Content::text("pendant le transfert"), vec![])).await.unwrap();
    assert_eq!(next_message(&mut ea).await.content.body, "pendant le transfert");
    transfer(upload).await.unwrap().unwrap();

    let info = next_file(&mut eb).await;
    assert_eq!(
        (info.name.as_str(), info.mime.as_str(), info.size),
        ("vidéo de vacances.mp4", "video/mp4", data.len() as u64)
    );
    assert_eq!(cb.files().await.unwrap(), vec![info.clone()]);

    let sink = Arc::new(MemSink::default());
    let seen = Arc::new(AtomicU64::new(0));
    let s2 = seen.clone();
    let progress = Arc::new(move |done: u64, _total: u64| s2.store(done, Ordering::SeqCst));
    transfer(cb.download(&info.blob_id, sink.clone(), progress)).await.unwrap();
    assert!(*sink.data.lock().unwrap() == data);
    assert!(sink.finished.load(Ordering::SeqCst));
    assert_eq!(seen.load(Ordering::SeqCst), data.len() as u64);

    // Le seul destinataire a fini : plus rien sur le relais, ni l'offre ici.
    assert_eq!(blob_dirs(&s.dir), 0);
    assert!(cb.files().await.unwrap().is_empty());
    // Aucune session n'a été coupée par les requêtes HTTP.
    let mut events = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), ea.next()).await {
        events.push(ev);
    }
    assert!(!events.iter().any(|e| matches!(e, Event::Disconnected { .. })), "{events:?}");
    s.server.stop().await;
}

#[tokio::test]
async fn transfers_resume_after_network_cuts() {
    let s = setup_with(|c| c).await;
    let gate = Arc::new(Gate::start(s.server.addr).await);
    let (ca, _ea) = online(&s.a, &gate.url()).await;
    let (cb, mut eb) = online(&s.b, &gate.url()).await;
    let data = sample(3 << 20);

    // Coupure d'une demi-seconde au tiers du dépôt, puis au tiers du retrait.
    let cut_at = |gate: Arc<Gate>| {
        let cut = Arc::new(AtomicBool::new(false));
        Arc::new(move |done: u64, total: u64| {
            if done >= total / 3 && !cut.swap(true, Ordering::SeqCst) {
                gate.cut();
                let gate = gate.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    gate.restore();
                });
            }
        })
    };
    let src = MemSource::new(data.clone());
    let sent = ca.send_file(outgoing(&s.b), src, cut_at(gate.clone()));
    transfer(sent).await.unwrap();

    let info = transfer(next_file(&mut eb)).await;
    let sink = Arc::new(MemSink::default());
    let got = cb.download(&info.blob_id, sink.clone(), cut_at(gate.clone()));
    transfer(got).await.unwrap();
    assert!(*sink.data.lock().unwrap() == data);
    s.server.stop().await;
}

/// Source qui échoue après `fail_after` lectures : l'appli est tuée en
/// plein dépôt.
struct Failing {
    inner: Arc<MemSource>,
    fail_after: u64,
}

impl ChunkSource for Failing {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        if self.inner.reads.load(Ordering::SeqCst) >= self.fail_after {
            return Err("appli arrêtée".into());
        }
        self.inner.read_at(offset, len)
    }
}

#[tokio::test]
async fn upload_resumes_after_app_and_relay_restart() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("ot.db");
    let config = || Config { blob_dir: dir.path().join("blobs"), ..test_config() };
    let store = open_store(&db).await;
    let a = add_device(&store, "telephone").await;
    let b = add_device(&store, "mac").await;
    make_group(&[&a, &b]);
    let server = TestServer::start(store, config()).await;
    let addr = server.addr;
    let (ca, _ea) = online(&a, &server.url()).await;

    let data = sample(2 << 20);
    let chunks = (data.len() as u64).div_ceil(u64::from(CHUNK));
    // Le hachage lit tout une fois, puis le dépôt s'arrête à mi-chemin.
    let failing =
        Arc::new(Failing { inner: MemSource::new(data.clone()), fail_after: chunks + chunks / 2 });
    let err = transfer(ca.send_file(outgoing(&b), failing, no_progress())).await.unwrap_err();
    assert!(matches!(err, ClientError::Transfer(_)), "{err}");

    // Le relais redémarre entre-temps.
    server.stop().await;
    let server = TestServer::start_on(addr, open_store(&db).await, config()).await;
    let (cb, mut eb) = online(&b, &server.url()).await;

    // Relancé sur la même source : seuls les morceaux manquants sont relus.
    let src = MemSource::new(data.clone());
    transfer(ca.send_file(outgoing(&b), src.clone(), no_progress())).await.unwrap();
    let reads = src.reads.load(Ordering::SeqCst);
    assert!(reads > 0 && reads < chunks, "{reads} lectures pour {chunks} morceaux");

    let info = next_file(&mut eb).await;
    let sink = Arc::new(MemSink::default());
    transfer(cb.download(&info.blob_id, sink.clone(), no_progress())).await.unwrap();
    assert!(*sink.data.lock().unwrap() == data);
    server.stop().await;
}

#[tokio::test]
async fn expired_file_is_reported_and_forgotten() {
    let s = setup_with(|c| Config { blob_ttl: Duration::from_secs(1), ..c }).await;
    let url = s.server.url();
    let (ca, _ea) = online(&s.a, &url).await;
    let (cb, mut eb) = online(&s.b, &url).await;
    let src = MemSource::new(sample(1000));
    within(ca.send_file(outgoing(&s.b), src, no_progress())).await.unwrap();
    let info = next_file(&mut eb).await;

    tokio::time::sleep(Duration::from_millis(1200)).await;
    let sink = Arc::new(MemSink::default());
    let err = within(cb.download(&info.blob_id, sink.clone(), no_progress())).await.unwrap_err();
    assert_eq!(err, ClientError::Expired);
    assert!(sink.aborted.load(Ordering::SeqCst) && !sink.finished.load(Ordering::SeqCst));
    assert!(cb.files().await.unwrap().is_empty());
    s.server.stop().await;
}

/// Source dont le contenu change après le hachage : le destinataire doit
/// refuser le fichier.
struct Changing(Arc<MemSource>);

impl ChunkSource for Changing {
    fn size(&self) -> u64 {
        self.0.size()
    }
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        let hashed = self.0.reads.load(Ordering::SeqCst) >= self.size().div_ceil(u64::from(CHUNK));
        let mut data = self.0.read_at(offset, len)?;
        if hashed && offset == 0 {
            data[0] ^= 1;
        }
        Ok(data)
    }
}

#[tokio::test]
async fn altered_file_is_rejected_and_never_finalized() {
    let s = setup_with(|c| c).await;
    let url = s.server.url();
    let (ca, _ea) = online(&s.a, &url).await;
    let (cb, mut eb) = online(&s.b, &url).await;
    let src = Arc::new(Changing(MemSource::new(sample(200_000))));
    within(ca.send_file(outgoing(&s.b), src, no_progress())).await.unwrap();
    let info = next_file(&mut eb).await;

    let sink = Arc::new(MemSink::default());
    let err = within(cb.download(&info.blob_id, sink.clone(), no_progress())).await.unwrap_err();
    assert!(err.to_string().contains("SHA-256"), "{err}");
    assert!(sink.aborted.load(Ordering::SeqCst) && !sink.finished.load(Ordering::SeqCst));
    assert!(sink.data.lock().unwrap().is_empty());
    s.server.stop().await;
}

#[tokio::test]
async fn relay_disk_holds_neither_name_nor_plaintext() {
    let s = setup_with(|c| c).await;
    let url = s.server.url();
    let (ca, _ea) = online(&s.a, &url).await;
    let (_cb, mut eb) = online(&s.b, &url).await;
    let marker = b"CONTENU-EN-CLAIR-TRES-RECONNAISSABLE";
    let data: Vec<u8> = marker.iter().copied().cycle().take(300_000).collect();
    within(ca.send_file(outgoing(&s.b), MemSource::new(data), no_progress())).await.unwrap();
    next_file(&mut eb).await;

    // Morceaux chiffrés sur le disque du relais, base comprise : ni le nom,
    // ni le type, ni le contenu.
    let mut found = Vec::new();
    for entry in walk(s.dir.path()) {
        let bytes = std::fs::read(&entry).unwrap();
        for needle in [&marker[..], "vacances".as_bytes(), b"video/mp4"] {
            if bytes.windows(needle.len()).any(|w| w == needle) {
                found.push(entry.display().to_string());
            }
        }
    }
    assert!(found.is_empty(), "clair trouvé dans {found:?}");
    assert!(blob_dirs(&s.dir) > 0, "le fichier doit être sur le disque du relais");
    s.server.stop().await;
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
