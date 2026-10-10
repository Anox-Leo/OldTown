//! Étape 2, T6 à T9 et T11 : chiffrement de bout en bout, appairage et
//! rotation, avec un relais en processus (parfois derrière un relais
//! malveillant simulé par un proxy).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use ot_core::{
    Client, ClientError, Content, ErrorCode, Event, Events, Identity, MemoryKeyring, redeem_pairing,
};
use ot_proto::{Encoding, b64};
use ot_server::store::Store;
use ot_server::{Config, now_ms};

async fn group_server(names: &[&str]) -> (TestServer, Vec<TestDevice>) {
    let store = Store::in_memory().await.unwrap();
    let mut devs = Vec::new();
    for name in names {
        devs.push(add_device(&store, name).await);
    }
    make_group(&devs.iter().collect::<Vec<_>>());
    (TestServer::start(store, test_config()).await, devs)
}

async fn online(dev: &TestDevice, url: &str) -> (Client, Events) {
    let (client, mut events) = dev.client(url);
    wait_connected(&mut events).await;
    (client, events)
}

fn group_changed(ev: &Event) -> bool {
    matches!(ev, Event::GroupChanged { .. })
}

/// B s'enregistre avec le code puis envoie sa demande au parrain.
async fn join(relay_url: &str, code: &str, name: &str) -> (TestDevice, Client, Events) {
    let identity = Identity::generate();
    let r = within(redeem_pairing(relay_url, &identity, name, code)).await.unwrap();
    let dev = TestDevice {
        id: r.device_id,
        name: name.into(),
        identity,
        keyring: Arc::new(MemoryKeyring::new()),
    };
    let (client, events) = online(&dev, relay_url).await;
    within(client.pair_join(code, r.sponsor, name)).await.unwrap();
    (dev, client, events)
}

#[tokio::test]
async fn pairing_by_code() {
    let (server, devs) = group_server(&["mac", "android"]).await;
    let (a, c) = (&devs[0], &devs[1]);
    let (ca, mut ea) = online(a, &server.url()).await;
    let (cc, mut ec) = online(c, &server.url()).await;

    let invitation = within(ca.pair_create()).await.unwrap();
    assert_eq!(invitation.code.len(), 46);
    let (b, cb, mut eb) = join(&server.url(), &invitation.code, "windows").await;

    wait_for(&mut eb, group_changed).await;
    match wait_for(&mut ea, |e| matches!(e, Event::PairingCompleted { .. })).await {
        Event::PairingCompleted { device_id, name } => {
            assert_eq!((device_id, name.as_str()), (b.id.clone(), "windows"));
        }
        _ => unreachable!(),
    }
    // Un troisième appareil apprend B par le message `roster`.
    wait_for(&mut ec, group_changed).await;
    assert!(within(cc.group()).await.unwrap().unwrap().member(&b.id).is_some());

    // B écrit à A et à C, et ils lui répondent.
    within(cb.send(&Content::text("bonjour de B"), vec![])).await.unwrap();
    assert_eq!(next_message(&mut ea).await.content, Content::text("bonjour de B"));
    assert_eq!(next_message(&mut ec).await.content, Content::text("bonjour de B"));
    within(cc.send(&Content::text("bienvenue"), vec![b.id.clone()])).await.unwrap();
    let m = next_message(&mut eb).await;
    assert_eq!((m.from, m.content), (c.id.clone(), Content::text("bienvenue")));
}

#[tokio::test]
async fn relay_tampering_with_pair_request_fails() {
    let (server, devs) = group_server(&["mac"]).await;
    let (ca, mut ea) = online(&devs[0], &server.url()).await;
    let evil = proxy(
        &server,
        Arc::new(|dir, frame| if dir == Dir::ToServer { flip_payload(frame) } else { frame }),
    )
    .await;

    let invitation = within(ca.pair_create()).await.unwrap();
    let (_b, cb, mut eb) = join(&evil, &invitation.code, "windows").await;
    match wait_for(&mut ea, |e| matches!(e, Event::Rejected { .. })).await {
        Event::Rejected { reason, .. } => assert_eq!(reason, "déchiffrement impossible"),
        _ => unreachable!(),
    }
    assert_eq!(within(ca.group()).await.unwrap().unwrap().members.len(), 1, "B pas ajouté");
    no_message(&mut eb, Duration::from_millis(200)).await;
    assert!(within(cb.group()).await.unwrap().is_none());
}

#[tokio::test]
async fn relay_tampering_with_pair_welcome_fails() {
    let (server, devs) = group_server(&["mac"]).await;
    let (ca, _ea) = online(&devs[0], &server.url()).await;
    let evil = proxy(
        &server,
        Arc::new(|dir, frame| if dir == Dir::ToClient { flip_payload(frame) } else { frame }),
    )
    .await;

    let invitation = within(ca.pair_create()).await.unwrap();
    let (_b, cb, mut eb) = join(&evil, &invitation.code, "windows").await;
    match wait_for(&mut eb, |e| matches!(e, Event::Rejected { .. })).await {
        Event::Rejected { reason, .. } => assert_eq!(reason, "déchiffrement impossible"),
        _ => unreachable!(),
    }
    assert!(within(cb.group()).await.unwrap().is_none(), "B n'a pas de groupe");
    assert!(matches!(
        within(cb.send(&Content::text("x"), vec![])).await,
        Err(ClientError::NoGroup)
    ));
}

#[tokio::test]
async fn wrong_code_fails_at_relay() {
    let (server, devs) = group_server(&["mac"]).await;
    let (ca, _ea) = online(&devs[0], &server.url()).await;
    let invitation = within(ca.pair_create()).await.unwrap();

    // Même secret, autre jeton : le relais refuse.
    let mut forged = invitation.code.clone();
    let other = if &forged[5..6] == "a" { "b" } else { "a" };
    forged.replace_range(5..6, other);
    let err = within(redeem_pairing(&server.url(), &Identity::generate(), "windows", &forged))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ClientError::Rejected { code: ErrorCode::PairingInvalid, .. }),
        "{err:?}"
    );
    // Un code mal formé échoue avant même de contacter le relais.
    let err = redeem_pairing(&server.url(), &Identity::generate(), "windows", "ot1-court").await;
    assert!(matches!(err, Err(ClientError::Pairing(_))));
}

#[tokio::test]
async fn relay_database_holds_no_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ot.db");
    let store = open_store(&path).await;
    let a = add_device(&store, "mac").await;
    let b = add_device(&store, "windows").await;
    make_group(&[&a, &b]);
    let server = TestServer::start(store, test_config()).await;
    let (ca, _ea) = online(&a, &server.url()).await;

    let secret = "MARQUEUR-SECRET-7f3a";
    within(ca.send(&Content::text(secret), vec![])).await.unwrap();
    within(ca.send(&Content::link(format!("https://example.org/{secret}")), vec![])).await.unwrap();

    // B est hors ligne : les deux messages attendent en base.
    let pending = server.relay.store().pending(&b.id, 0, 10, now_ms()).await.unwrap();
    assert_eq!(pending.len(), 2);
    for p in &pending {
        let env: ot_proto::Envelope = serde_json::from_slice(&p.envelope).unwrap();
        assert_eq!(env.payload.enc, Encoding::Xchacha20poly1305);
        assert!(!String::from_utf8_lossy(&env.payload.data).contains(secret));
    }
    server.stop().await;

    let mut bytes = std::fs::read(&path).unwrap();
    if let Ok(wal) = std::fs::read(path.with_extension("db-wal")) {
        bytes.extend(wal);
    }
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains(secret), "texte en clair dans la base du relais");
    let content = serde_json::to_vec(&Content::text(secret)).unwrap();
    assert!(!text.contains(&b64::encode(&content)), "contenu en clair (base64) dans la base");
}

#[tokio::test]
async fn device_without_group_refuses_to_send() {
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "mac").await;
    let server = TestServer::start(store, test_config()).await;
    let (ca, _ea) = online(&a, &server.url()).await;
    let err = within(ca.send(&Content::text("x"), vec![])).await.unwrap_err();
    assert_eq!(err, ClientError::NoGroup);
    assert!(err.to_string().contains("ot group init"));
}

#[tokio::test]
async fn plaintext_from_a_member_is_rejected_and_acked() {
    let (server, devs) = group_server(&["mac", "windows"]).await;
    let (a, b) = (&devs[0], &devs[1]);
    let (ca, _ea) = online(a, &server.url()).await;
    let (_cb, mut eb) = online(b, &server.url()).await;
    let plain = ot_proto::Envelope::new_plain(a.id.clone(), vec![], &Content::text("en clair"));
    within(ca.send_envelope(plain)).await.unwrap();
    match wait_for(&mut eb, |e| matches!(e, Event::Rejected { .. } | Event::Message(_))).await {
        Event::Rejected { reason, .. } => assert_eq!(reason, "message en clair refusé"),
        other => panic!("rejet attendu, reçu {other:?}"),
    }
    for _ in 0..50 {
        if server.relay.store().pending_count(&b.id, now_ms()).await.unwrap() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("message rejeté jamais acquitté");
}

#[tokio::test]
async fn targeted_send_only_to_roster_members() {
    let (server, devs) = group_server(&["mac", "windows"]).await;
    // Ajouté par `ot-server device add`, sans appairage.
    let intruder = add_device(server.relay.store(), "intrus").await;
    let (ca, _ea) = online(&devs[0], &server.url()).await;

    let err = within(ca.send(&Content::text("x"), vec![intruder.id.clone()])).await.unwrap_err();
    assert_eq!(err, ClientError::NotMember(intruder.id.clone()));

    // Il apparaît comme inconnu du roster.
    let devices = within(ca.list_devices()).await.unwrap();
    let group = within(ca.group()).await.unwrap().unwrap();
    let strangers = group.strangers(&devices);
    assert_eq!(strangers.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["intrus"]);
}

#[tokio::test]
async fn rotation_excludes_a_device() {
    let (server, devs) = group_server(&["mac", "windows", "android"]).await;
    let (a, b, c) = (&devs[0], &devs[1], &devs[2]);
    let (ca, mut ea) = online(a, &server.url()).await;
    let (cc, mut ec) = online(c, &server.url()).await;

    // B est hors ligne : un message de l'ancienne époque l'attend.
    within(ca.send(&Content::text("avant"), vec![])).await.unwrap();
    assert_eq!(next_message(&mut ec).await.content, Content::text("avant"));

    let epoch = within(ca.rotate(vec![c.id.clone()])).await.unwrap();
    assert_eq!(epoch, 2);
    within(ca.send(&Content::text("après"), vec![])).await.unwrap();

    // C ne lit plus les nouveaux messages.
    no_message(&mut ec, Duration::from_millis(300)).await;
    assert_eq!(within(cc.group()).await.unwrap().unwrap().epoch, 1);

    // B lit l'ancien message en retard, adopte la clé, puis lit le nouveau.
    let (cb, mut eb) = online(b, &server.url()).await;
    assert_eq!(next_message(&mut eb).await.content, Content::text("avant"));
    assert_eq!(next_message(&mut eb).await.content, Content::text("après"));
    let group = within(cb.group()).await.unwrap().unwrap();
    assert_eq!(group.epoch, 2);
    assert!(group.member(&c.id).is_none());

    // C, exclu, n'est plus cru par A.
    within(cc.send(&Content::text("toujours là ?"), vec![])).await.unwrap();
    match wait_for(&mut ea, |e| matches!(e, Event::Rejected { .. } | Event::Message(_))).await {
        Event::Rejected { reason, .. } => assert_eq!(reason, "expéditeur hors roster"),
        other => panic!("rejet attendu, reçu {other:?}"),
    }
}

#[tokio::test]
async fn pairing_token_expires() {
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "mac").await;
    make_group(&[&a]);
    let config = Config { pairing_ttl: Duration::from_millis(100), ..test_config() };
    let server = TestServer::start(store, config).await;
    let (ca, _ea) = online(&a, &server.url()).await;
    let invitation = within(ca.pair_create()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let err =
        within(redeem_pairing(&server.url(), &Identity::generate(), "windows", &invitation.code))
            .await
            .unwrap_err();
    assert!(matches!(err, ClientError::Rejected { code: ErrorCode::PairingInvalid, .. }));
}
