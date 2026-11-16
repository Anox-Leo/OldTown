//! Étape 3, T8 : relais en `wss` (note 0015). Certificats de test signés par
//! une autorité générée pour l'occasion.

mod common;

use std::time::{Duration, SystemTime};

use common::*;
use ot_core::{Content, Event, Events, TlsRoots};
use ot_server::store::Store;
use ot_server::tls::Tls;

fn trusting(ca: &TestCa) -> TlsRoots {
    TlsRoots::with_extra(&[ca.der()]).unwrap()
}

/// Attend une déconnexion, en échouant si la session s'ouvre.
async fn refused(events: &mut Events) -> String {
    within(async {
        loop {
            match next_event(events).await {
                Event::Disconnected { reason } => return reason,
                Event::Connected { .. } => panic!("connexion acceptée avec un certificat refusé"),
                _ => {}
            }
        }
    })
    .await
}

#[tokio::test]
async fn wss_exchange_with_trusted_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let ca = TestCa::new("ca de test");
    let (cert, key) = ca.write_leaf(dir.path());
    let store = Store::in_memory().await.unwrap();
    let (a, b) = (add_device(&store, "mac").await, add_device(&store, "android").await);
    make_group(&[&a, &b]);
    let server = TestServer::start_tls(store, test_config(), Tls::load(&cert, &key).unwrap()).await;
    assert!(server.url().starts_with("wss://"));

    let start = |dev: &TestDevice| {
        let mut config = dev.config(&server.url());
        config.tls = trusting(&ca);
        ot_core::Client::start(config)
    };
    let (ca_client, mut ea) = start(&a);
    let (_cb, mut eb) = start(&b);
    wait_connected(&mut ea).await;
    wait_connected(&mut eb).await;
    within(ca_client.send(&Content::text("par TLS"), vec![])).await.unwrap();
    assert_eq!(next_message(&mut eb).await.content, Content::text("par TLS"));
}

/// Étape 5 : les fichiers passent par HTTPS sur la même écoute.
#[tokio::test]
async fn file_over_https() {
    let dir = tempfile::tempdir().unwrap();
    let ca = TestCa::new("ca de test");
    let (cert, key) = ca.write_leaf(dir.path());
    let store = Store::in_memory().await.unwrap();
    let (a, b) = (add_device(&store, "mac").await, add_device(&store, "android").await);
    make_group(&[&a, &b]);
    let config = ot_server::Config { blob_dir: dir.path().join("blobs"), ..test_config() };
    let server = TestServer::start_tls(store, config, Tls::load(&cert, &key).unwrap()).await;
    let start = |dev: &TestDevice| {
        let mut config = dev.config(&server.url());
        config.tls = trusting(&ca);
        ot_core::Client::start(config)
    };
    let (ca_client, mut ea) = start(&a);
    let (cb, mut eb) = start(&b);
    wait_connected(&mut ea).await;
    wait_connected(&mut eb).await;

    let data = sample(300_000);
    let file = ot_core::OutgoingFile {
        source_id: "photo".into(),
        name: "photo.jpg".into(),
        mime: "image/jpeg".into(),
        to: vec![],
    };
    let none = std::sync::Arc::new(|_: u64, _: u64| {});
    within(ca_client.send_file(file, MemSource::new(data.clone()), none.clone())).await.unwrap();
    let Event::File(f) = wait_for(&mut eb, |e| matches!(e, Event::File(_))).await else {
        unreachable!()
    };
    let sink = std::sync::Arc::new(MemSink::default());
    within(cb.download(&f.file.blob_id, sink.clone(), none)).await.unwrap();
    assert!(*sink.data.lock().unwrap() == data);
}

#[tokio::test]
async fn untrusted_certificate_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (cert, key) = TestCa::new("autorité inconnue").write_leaf(dir.path());
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "mac").await;
    let server = TestServer::start_tls(store, test_config(), Tls::load(&cert, &key).unwrap()).await;

    // Autorités de Mozilla seulement : le certificat de test n'en vient pas.
    let (_client, mut events) = a.client(&server.url());
    let reason = refused(&mut events).await;
    assert!(reason.contains("certificate"), "{reason}");
}

#[tokio::test]
async fn renewed_certificate_is_reloaded_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (old_ca, new_ca) = (TestCa::new("ancienne"), TestCa::new("nouvelle"));
    let (cert, key) = old_ca.write_leaf(dir.path());
    let tls = Tls::load(&cert, &key).unwrap();
    assert!(!tls.reload_if_changed().unwrap(), "rien n'a changé");

    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "mac").await;
    let server = TestServer::start_tls(store, test_config(), tls.clone()).await;
    let mut config = a.config(&server.url());
    config.tls = trusting(&old_ca);
    let (old_client, mut events) = ot_core::Client::start(config);
    wait_connected(&mut events).await;
    old_client.shutdown().await;

    // Renouvellement : nouveaux fichiers, date de modification plus récente.
    new_ca.write_leaf(dir.path());
    let later = SystemTime::now() + Duration::from_secs(60);
    for path in [&cert, &key] {
        std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
    }
    assert!(tls.reload_if_changed().unwrap());

    let mut config = a.config(&server.url());
    config.tls = trusting(&new_ca);
    let (_new_client, mut events) = ot_core::Client::start(config);
    wait_connected(&mut events).await;

    let mut config = a.config(&server.url());
    config.tls = trusting(&old_ca);
    let (_stale, mut events) = ot_core::Client::start(config);
    refused(&mut events).await;
}

#[tokio::test]
async fn broken_files_keep_the_current_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let (cert, key) = TestCa::new("ca").write_leaf(dir.path());
    assert!(Tls::load(&key, &key).is_err(), "une clé n'est pas un certificat");
    let tls = Tls::load(&cert, &key).unwrap();
    std::fs::write(&cert, "à moitié écrit").unwrap();
    let later = SystemTime::now() + Duration::from_secs(60);
    std::fs::File::options().write(true).open(&cert).unwrap().set_modified(later).unwrap();
    assert!(tls.reload_if_changed().is_err());
}
