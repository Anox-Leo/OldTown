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
    let relay = Relay::new(store.clone(), Config::default());
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
    assert_eq!(classify_text("https://x.fr".into()), ContentKind::Link);
    assert_eq!(classify_text("bonjour".into()), ContentKind::Text);
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
        };
        local.push_history(item).unwrap();
    }
    let items = local.history().unwrap();
    assert_eq!(items.len(), crate::local::HISTORY_LEN);
    assert_eq!((items[0].body.as_str(), items[49].body.as_str()), ("59", "10"));
}
