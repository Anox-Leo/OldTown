//! Étape 1, T10 : scénarios de bout en bout, serveur dans le processus et
//! clients ot-core (membres d'un même groupe depuis l'étape 2).

mod common;

use std::time::{Duration, Instant};

use common::*;
use ot_core::{Content, Event};
use ot_proto::{Envelope, Frame};
use ot_server::store::Store;
use ot_server::{Config, now_ms};

async fn setup(config: Config) -> (TestServer, TestDevice, TestDevice) {
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "windows").await;
    let b = add_device(&store, "mac").await;
    make_group(&[&a, &b]);
    (TestServer::start(store, config).await, a, b)
}

#[tokio::test]
async fn both_online_under_a_second() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    let (_cb, mut eb) = b.client(&server.url());
    wait_connected(&mut ea).await;
    wait_connected(&mut eb).await;

    let start = Instant::now();
    let accepted = within(ca.send(&Content::text("bonjour"), vec![])).await.unwrap();
    let m = next_message(&mut eb).await;
    assert!(start.elapsed() < Duration::from_secs(1), "{:?}", start.elapsed());
    assert_eq!(m.msg_id, accepted.msg_id);
    assert_eq!(m.from, a.id);
    assert_eq!(m.content, Content::text("bonjour"));
}

#[tokio::test]
async fn offline_recipient_gets_messages_in_order() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    for i in 0..5 {
        within(ca.send(&Content::text(format!("m{i}")), vec![b.id.clone()])).await.unwrap();
    }

    let (_cb, mut eb) = b.client(&server.url());
    assert_eq!(wait_connected(&mut eb).await, 5);
    for i in 0..5 {
        assert_eq!(next_message(&mut eb).await.content, Content::text(format!("m{i}")));
    }
}

#[tokio::test]
async fn sends_queued_while_offline_go_out_on_connect() {
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "windows").await;
    let b = add_device(&store, "mac").await;
    make_group(&[&a, &b]);
    // Réserve un port, puis démarre le client avant le serveur.
    let addr = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let (ca, _ea) = a.client(&format!("ws://{addr}"));
    let send =
        tokio::spawn(async move { within(ca.send(&Content::text("en file"), vec![])).await });
    tokio::time::sleep(Duration::from_millis(150)).await;

    let server = TestServer::start_on(addr, store, test_config()).await;
    tokio::time::timeout(STEP, send).await.unwrap().unwrap().unwrap();
    let (_cb, mut eb) = b.client(&server.url());
    assert_eq!(next_message(&mut eb).await.content, Content::text("en file"));
}

#[tokio::test]
async fn interrupted_delivery_is_shown_once() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    let sent = within(ca.send(&Content::text("une fois"), vec![])).await.unwrap();

    // B reçoit la trame mais coupe avant d'acquitter.
    let (mut raw, pending) = Raw::login(&server, &b).await;
    assert_eq!(pending, 1);
    assert!(matches!(raw.recv().await, Some(Frame::Deliver { .. })));
    raw.close().await;

    // À la reconnexion, le message est relivré ; un client qui l'a déjà vu
    // (état persisté) l'acquitte sans l'afficher.
    let mut config = b.config(&server.url());
    config.seen = vec![sent.msg_id];
    let (_cb, mut eb) = ot_core::Client::start(config);
    assert_eq!(wait_connected(&mut eb).await, 1);
    no_message(&mut eb, Duration::from_millis(300)).await;
    assert_eq!(server.relay.store().pending_count(&b.id, now_ms()).await.unwrap(), 0);
}

#[tokio::test]
async fn redelivered_without_prior_display_is_shown_once() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    within(ca.send(&Content::text("x"), vec![])).await.unwrap();

    let (mut raw, _) = Raw::login(&server, &b).await;
    assert!(matches!(raw.recv().await, Some(Frame::Deliver { .. })));
    raw.close().await;

    let (_cb, mut eb) = b.client(&server.url());
    assert_eq!(next_message(&mut eb).await.content, Content::text("x"));
    no_message(&mut eb, Duration::from_millis(300)).await;
}

#[tokio::test]
async fn replayed_send_is_stored_once() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    let env = Envelope::new_plain(a.id.clone(), vec![], &Content::text("deux fois"));
    let first = within(ca.send_envelope(env.clone())).await.unwrap();
    let second = within(ca.send_envelope(env)).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(server.relay.store().pending_count(&b.id, now_ms()).await.unwrap(), 1);
}

#[tokio::test]
async fn pending_messages_survive_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ot.db");
    let store = open_store(&path).await;
    let a = add_device(&store, "windows").await;
    let b = add_device(&store, "mac").await;
    make_group(&[&a, &b]);
    let server = TestServer::start(store, test_config()).await;
    let addr = server.addr;

    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    within(ca.send(&Content::text("avant"), vec![])).await.unwrap();
    server.stop().await;
    assert!(matches!(next_event(&mut ea).await, Event::Disconnected { .. }));

    // Même port, même base : A se reconnecte seul.
    let server = TestServer::start_on(addr, open_store(&path).await, test_config()).await;
    wait_connected(&mut ea).await;
    within(ca.send(&Content::text("après"), vec![])).await.unwrap();

    let (_cb, mut eb) = b.client(&server.url());
    assert_eq!(wait_connected(&mut eb).await, 2);
    assert_eq!(next_message(&mut eb).await.content, Content::text("avant"));
    assert_eq!(next_message(&mut eb).await.content, Content::text("après"));
}

#[tokio::test]
async fn listener_reconnects_after_network_cut() {
    // Base fichier : une base en mémoire disparaît si sa connexion se ferme
    // pendant la coupure, ce qui n'a rien à voir avec ce qu'on teste.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ot.db");
    let store = open_store(&path).await;
    let a = add_device(&store, "windows").await;
    let b = add_device(&store, "mac").await;
    make_group(&[&a, &b]);
    let server = TestServer::start(store, test_config()).await;
    let addr = server.addr;
    let (_cb, mut eb) = b.client(&server.url());
    wait_connected(&mut eb).await;

    // Coupure : le serveur disparaît puis revient sur la même base.
    server.stop().await;
    assert!(matches!(next_event(&mut eb).await, Event::Disconnected { .. }));
    let server = TestServer::start_on(addr, open_store(&path).await, test_config()).await;
    wait_connected(&mut eb).await;

    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    within(ca.send(&Content::text("de retour"), vec![])).await.unwrap();
    assert_eq!(next_message(&mut eb).await.content, Content::text("de retour"));
}

#[tokio::test]
async fn expired_messages_are_purged() {
    let config = Config { ttl: Duration::from_millis(200), ..test_config() };
    let (server, a, b) = setup(config).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    within(ca.send(&Content::text("éphémère"), vec![])).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;

    let (_cb, mut eb) = b.client(&server.url());
    assert_eq!(wait_connected(&mut eb).await, 0);
    no_message(&mut eb, Duration::from_millis(200)).await;

    let stats = server.relay.purge_once().await.unwrap();
    assert_eq!(stats.expired, 1);
    let (_, body) = http_get(server.addr, "/metrics").await;
    assert!(body.contains("ot_messages_purged_total{reason=\"expired\"} 1"), "{body}");
}

#[tokio::test]
async fn acked_messages_are_purged() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    let (_cb, mut eb) = b.client(&server.url());
    wait_connected(&mut ea).await;
    wait_connected(&mut eb).await;
    within(ca.send(&Content::text("lu"), vec![])).await.unwrap();
    next_message(&mut eb).await;

    for _ in 0..50 {
        if server.relay.purge_once().await.unwrap().acked == 1 {
            let (_, body) = http_get(server.addr, "/metrics").await;
            assert!(body.contains("ot_messages_purged_total{reason=\"acked\"} 1"), "{body}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("message acquitté jamais purgé");
}

#[tokio::test]
async fn list_devices_from_client() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    let devices = within(ca.list_devices()).await.unwrap();
    let names: Vec<_> = devices.iter().map(|d| (d.name.as_str(), d.online)).collect();
    assert_eq!(names, [("windows", true), ("mac", false)]);
    let _ = b;
}

#[tokio::test]
async fn shutdown_keeps_frames_already_sent() {
    let (server, a, b) = setup(test_config()).await;
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    let (cb, mut eb) = b.client(&server.url());
    wait_connected(&mut eb).await;
    for i in 0..20 {
        // B acquitte puis s'arrête aussitôt : l'acquittement ne doit pas se
        // perdre dans la fermeture.
        within(ca.send(&Content::text(format!("m{i}")), vec![])).await.unwrap();
    }
    for _ in 0..20 {
        next_message(&mut eb).await;
    }
    cb.shutdown().await;
    assert_eq!(server.relay.store().pending_count(&b.id, now_ms()).await.unwrap(), 0);
}
