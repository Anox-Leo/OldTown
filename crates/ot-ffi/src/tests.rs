//! API exportée, contre un relais en processus (étape 3, T2).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ot_core::{Client, ClientConfig, Content, Identity, MemoryKeyring};
use ot_server::store::Store;
use ot_server::{Config, Relay};
use tokio::net::TcpListener;

use super::*;
use crate::local::{ALL_NAMES, Local};

/// Stockage de test, comme le ferait Kotlin.
#[derive(Default)]
struct TestStore(Mutex<HashMap<String, Vec<u8>>>);

impl SecretStore for TestStore {
    fn get(&self, name: String) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.0.lock().unwrap().get(&name).cloned())
    }
    fn put(&self, name: String, value: Vec<u8>) -> Result<(), StoreError> {
        self.0.lock().unwrap().insert(name, value);
        Ok(())
    }
    fn delete(&self, name: String) -> Result<(), StoreError> {
        self.0.lock().unwrap().remove(&name);
        Ok(())
    }
}

impl TestStore {
    fn names(&self) -> Vec<String> {
        let mut n: Vec<_> = self.0.lock().unwrap().keys().cloned().collect();
        n.sort();
        n
    }
}

const STEP: Duration = Duration::from_secs(5);

async fn within<T>(fut: impl Future<Output = T>) -> T {
    tokio::time::timeout(STEP, fut).await.expect("délai dépassé")
}

async fn relay() -> (String, Store) {
    let store = Store::in_memory().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    // Dossier des fichiers propre à chaque relais de test, sous le dossier
    // temporaire du système.
    let blobs = std::env::temp_dir().join(format!("ot-ffi-test-{}", ot_core::Ulid::generate()));
    let relay = Relay::new(store.clone(), Config { blob_dir: blobs, ..Config::default() });
    tokio::spawn(ot_server::serve(relay, listener, std::future::pending()));
    (url, store)
}

/// Téléphone déjà enregistré, premier membre du groupe.
async fn phone(url: &str, relay: &Store) -> Arc<TestStore> {
    let store = Arc::new(TestStore::default());
    let local = Local::new(store.clone());
    let identity = Identity::generate();
    let id = relay.add_device("android", &identity.public_key(), 0).await.unwrap().id;
    local.save_identity(&identity).unwrap();
    local
        .save_device(&DeviceState {
            device_id: id.to_string(),
            name: "android".into(),
            server_url: url.into(),
        })
        .unwrap();
    ot_core::init_group(&local.keyring(), &id, "android", &identity, false).unwrap();
    store
}

async fn next(client: &OtClient, pred: impl Fn(&Event) -> bool) -> Event {
    within(async {
        loop {
            let ev = client.next_event().await.expect("client arrêté");
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
}

fn connected(e: &Event) -> bool {
    matches!(e, Event::Connected { .. })
}

#[tokio::test]
async fn phone_joins_by_code_then_exchanges() {
    let (url, relay_store) = relay().await;
    let a_store = phone(&url, &relay_store).await;
    let a = OtClient::start(a_store.clone()).unwrap();
    next(&a, connected).await;
    let invitation = a.pair_new().await.unwrap();
    assert!(invitation.uri.starts_with("oldtown://pair?s=ws://"));

    // Le second téléphone scanne le QR.
    let QrKind::Join { server_url, code } = classify_qr(invitation.uri.clone()) else {
        panic!("QR « rejoindre » attendu");
    };
    let b_store = Arc::new(TestStore::default());
    assert_eq!(device_state(b_store.clone()).unwrap(), None);
    let device =
        join(b_store.clone(), server_url.clone(), code.clone(), "pixel".into()).await.unwrap();
    assert_eq!(device.name, "pixel");
    // Le code ne sert qu'une fois, et un appareil enregistré ne rejoint pas deux fois.
    assert!(matches!(
        join(b_store.clone(), server_url, code, "pixel".into()).await,
        Err(OtError::Invalid { .. })
    ));

    let b = OtClient::start(b_store.clone()).unwrap();
    next(&b, |e| matches!(e, Event::GroupChanged { .. })).await;
    next(&a, |e| matches!(e, Event::PairingCompleted { name, .. } if name == "pixel")).await;
    assert!(!b_store.names().contains(&"pending_join.json".to_owned()), "appairage terminé");
    let group = b.group().await.unwrap().unwrap();
    assert_eq!(group.members.len(), 2);
    assert!(group.members.iter().any(|m| m.is_self && m.name == "pixel"));
    assert_eq!(group_snapshot(b_store.clone()).unwrap(), Some(group));

    // Échange : un lien de B vers A, rangé dans l'historique de A.
    let msg_id = b.send_text(" https://example.org/a ".into(), vec![]).await.unwrap();
    let Event::Message { received } = next(&a, |e| matches!(e, Event::Message { .. })).await else {
        unreachable!()
    };
    assert_eq!((received.msg_id.as_str(), received.from_name.as_str()), (msg_id.as_str(), "pixel"));
    assert_eq!(
        (received.kind, received.body.as_str()),
        (ContentKind::Link, "https://example.org/a")
    );
    assert_eq!(history(a_store.clone()).unwrap(), vec![received.clone()]);
    clear_history(a_store.clone()).unwrap();
    assert!(history(a_store.clone()).unwrap().is_empty());

    // Destinataire hors groupe, texte vide.
    assert!(matches!(
        b.send_text("x".into(), vec![ot_core::DeviceId::generate().to_string()]).await,
        Err(OtError::NotInGroup { .. })
    ));
    assert!(matches!(b.send_text("  ".into(), vec![]).await, Err(OtError::Invalid { .. })));

    let devices = a.devices().await.unwrap();
    assert!(devices.iter().all(|d| d.member), "{devices:?}");
    a.shutdown().await;
    assert_eq!(within(a.next_event()).await, None, "flux fermé après l'arrêt");
}

#[tokio::test]
async fn phone_sponsors_by_reverse_qr() {
    let (url, relay_store) = relay().await;
    let a_store = phone(&url, &relay_store).await;
    let a = OtClient::start(a_store).unwrap();
    next(&a, connected).await;

    // Un PC affiche son QR (comme `ot pair join --qr`).
    let pc = ClientConfig::new(
        url.clone(),
        ot_core::DeviceId::generate(),
        Identity::generate(),
        Arc::new(MemoryKeyring::new()),
    );
    let mut pc = pc;
    pc.backoff =
        ot_core::Backoff { initial: Duration::from_millis(50), max: Duration::from_millis(200) };
    let (pc, mut pc_events) = Client::start(pc);
    let qr = within(pc.pair_offer("windows")).await.unwrap();

    match classify_qr(qr.clone()) {
        QrKind::Sponsor { name, .. } => assert_eq!(name, "windows"),
        other => panic!("QR inversé attendu, reçu {other:?}"),
    }
    let added = a.sponsor(qr).await.unwrap();
    assert_eq!(added.name, "windows");
    within(async {
        while !matches!(pc_events.next().await, Some(ot_core::Event::GroupChanged { .. })) {}
    })
    .await;
    within(pc.send(&Content::text("merci"), vec![])).await.unwrap();
    let Event::Message { received } = next(&a, |e| matches!(e, Event::Message { .. })).await else {
        unreachable!()
    };
    assert_eq!((received.from_name.as_str(), received.body.as_str()), ("windows", "merci"));
}

/// Fichier en mémoire, comme le lirait Kotlin.
struct MemSource(Vec<u8>);

impl FileSource for MemSource {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, StoreError> {
        let start = (offset as usize).min(self.0.len());
        Ok(self.0[start..(start + len as usize).min(self.0.len())].to_vec())
    }
}

#[derive(Default)]
struct MemSink(Mutex<Vec<u8>>);

impl FileSink for MemSink {
    fn write_at(&self, offset: u64, data: Vec<u8>) -> Result<(), StoreError> {
        let mut buf = self.0.lock().unwrap();
        let end = buf.len().max(offset as usize + data.len());
        buf.resize(end, 0);
        buf[offset as usize..offset as usize + data.len()].copy_from_slice(&data);
        Ok(())
    }
    fn finish(&self) -> Result<String, StoreError> {
        Ok("content://media/downloads/7".into())
    }
    fn abort(&self) {
        self.0.lock().unwrap().clear();
    }
}

struct Quiet;

impl TransferListener for Quiet {
    fn on_progress(&self, _done: u64, _total: u64) {}
}

#[tokio::test]
async fn files_both_ways() {
    let (url, relay_store) = relay().await;
    let a_store = phone(&url, &relay_store).await;
    let a = OtClient::start(a_store.clone()).unwrap();
    next(&a, connected).await;
    // Un PC rejoint le groupe par QR inversé.
    let pc_id = Identity::generate();
    let mut pc = ClientConfig::new(
        url.clone(),
        ot_core::DeviceId::generate(),
        pc_id,
        Arc::new(MemoryKeyring::new()),
    );
    pc.backoff =
        ot_core::Backoff { initial: Duration::from_millis(50), max: Duration::from_millis(200) };
    let (pc, mut pc_events) = Client::start(pc);
    a.sponsor(within(pc.pair_offer("windows")).await.unwrap()).await.unwrap();
    within(async {
        while !matches!(pc_events.next().await, Some(ot_core::Event::GroupChanged { .. })) {}
    })
    .await;

    // Téléphone → PC.
    let photo: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    let source = Arc::new(MemSource(photo.clone()));
    let msg = a
        .send_file(
            source,
            "content://media/images/1".into(),
            "IMG_0001.jpg".into(),
            "image/jpeg".into(),
            vec![],
            Arc::new(Quiet),
        )
        .await
        .unwrap();
    let file = within(async {
        loop {
            if let Some(ot_core::Event::File(f)) = pc_events.next().await {
                return f.file;
            }
        }
    })
    .await;
    assert_eq!((file.msg_id.to_string(), file.name.as_str()), (msg, "IMG_0001.jpg"));

    // PC → téléphone : rangé dans l'historique, téléchargé, URI notée.
    let source = Arc::new(MemSource(photo.clone()));
    let pc_file = ot_core::OutgoingFile {
        source_id: "/home/x/scan.pdf".into(),
        name: "scan.pdf".into(),
        mime: "application/pdf".into(),
        to: vec![],
    };
    let none = Arc::new(|_: u64, _: u64| {});
    within(pc.send_file(pc_file, Arc::new(crate::files::Source(source)), none)).await.unwrap();
    let Event::Message { received } = next(&a, |e| matches!(e, Event::Message { .. })).await else {
        unreachable!()
    };
    assert_eq!((received.kind, received.body.as_str()), (ContentKind::File, "scan.pdf"));
    let item = received.file.clone().unwrap();
    assert_eq!(
        (item.mime.as_str(), item.size, item.saved_uri.clone()),
        ("application/pdf", photo.len() as u64, None)
    );
    let sink = Arc::new(MemSink::default());
    let saved = a.download(received.msg_id.clone(), sink.clone(), Arc::new(Quiet)).await.unwrap();
    assert!(*sink.0.lock().unwrap() == photo);
    assert_eq!(saved.saved_uri.as_deref(), Some("content://media/downloads/7"));
    let history = history(a_store.clone()).unwrap();
    assert_eq!(history[0].file.as_ref().unwrap().saved_uri, saved.saved_uri);
    // Aucune clé de fichier dans l'historique rangé par le pont.
    let raw = String::from_utf8(a_store.0.lock().unwrap()["history.json"].clone()).unwrap();
    assert!(!raw.contains("\"key\"") && !raw.contains("nonce"), "{raw}");
}

#[test]
fn old_history_without_files_still_reads() {
    let old = r#"[{"msg_id":"m","from_id":"d","from_name":"mac","created_at_ms":1,"kind":"text","body":"x","sensitive":false}]"#;
    let items: Vec<Received> = serde_json::from_str(old).unwrap();
    assert_eq!(items[0].file, None);
}

#[tokio::test]
async fn bad_join_inputs_are_refused_before_contact() {
    let store = Arc::new(TestStore::default());
    let code = "ot1-court".to_owned();
    for (url, code, name) in [
        ("wss://relais:7878", code.clone(), "pixel"),
        ("relais:7878", code.clone(), "pixel"),
        ("wss://relais:7878", code, " "),
    ] {
        let res = join(store.clone(), url.into(), code, name.into()).await;
        assert!(matches!(res, Err(OtError::Pairing { .. } | OtError::Invalid { .. })), "{res:?}");
    }
    assert!(store.names().is_empty(), "rien d'écrit");
    assert!(matches!(OtClient::start(store), Err(OtError::NotInGroup { .. })));
}

#[test]
fn forget_erases_everything() {
    let store = Arc::new(TestStore::default());
    for name in ALL_NAMES {
        store.put((*name).into(), b"{}".to_vec()).unwrap();
    }
    forget_device(store.clone()).unwrap();
    assert!(store.names().is_empty(), "{:?}", store.names());
}

#[test]
fn qr_helpers() {
    assert_eq!(classify_qr("https://example.org".into()), QrKind::Unknown);
    assert!(matches!(classify_qr("oldtown://sponsor?o=AAAA".into()), QrKind::Invalid { .. }));
    assert!(matches!(classify_qr("oldtown://pair?s=x&c=y".into()), QrKind::Invalid { .. }));
    let qr = qr_code("oldtown://pair?s=wss://relais&c=ot1-…".into()).unwrap();
    assert_eq!(qr.modules.len(), (qr.size * qr.size) as usize);
    assert!(qr.modules.iter().any(|&m| m) && qr.modules.iter().any(|&m| !m));
    assert!(!core_version().is_empty());
}

#[test]
fn history_keeps_the_last_fifty() {
    let local = Local::new(Arc::new(TestStore::default()));
    for i in 0..60 {
        let item = Received {
            msg_id: format!("m{i}"),
            from_id: "dev".into(),
            from_name: "mac".into(),
            created_at_ms: i,
            kind: ContentKind::Text,
            body: format!("{i}"),
            sensitive: false,
            file: None,
        };
        local.push_history(item).unwrap();
    }
    let items = local.history().unwrap();
    assert_eq!(items.len(), crate::local::HISTORY_LEN);
    assert_eq!((items[0].body.as_str(), items[49].body.as_str()), ("59", "10"));
}
