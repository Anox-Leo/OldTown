//! Étape 3, T9 : appairage par QR inversé de bout en bout (note 0022), avec
//! un relais honnête puis malveillant (proxy).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use ot_core::{Client, Content, Event, Events, Identity, MemoryKeyring};
use ot_proto::{DeviceId, Frame};
use ot_server::store::Store;

async fn group_server(names: &[&str]) -> (TestServer, Vec<TestDevice>) {
    let store = Store::in_memory().await.unwrap();
    let mut devs = Vec::new();
    for name in names {
        devs.push(add_device(&store, name).await);
    }
    make_group(&devs.iter().collect::<Vec<_>>());
    (TestServer::start(store, test_config()).await, devs)
}

/// Nouvel appareil : identité et identifiant tirés localement, pas encore
/// enregistré ; renvoie aussi le texte de son QR.
async fn newcomer(url: &str, name: &str) -> (TestDevice, Client, Events, String) {
    let dev = TestDevice {
        id: DeviceId::generate(),
        name: name.into(),
        identity: Identity::generate(),
        keyring: Arc::new(MemoryKeyring::new()),
    };
    let (client, events) = dev.client(url);
    let qr = within(client.pair_offer(name)).await.unwrap();
    (dev, client, events, qr)
}

fn group_changed(ev: &Event) -> bool {
    matches!(ev, Event::GroupChanged { .. })
}

#[tokio::test]
async fn phone_sponsors_a_pc_by_reverse_qr() {
    let (server, devs) = group_server(&["android", "mac"]).await;
    let (a, c) = (&devs[0], &devs[1]);
    let (ca, mut ea) = a.client(&server.url());
    wait_connected(&mut ea).await;
    let (cc, mut ec) = c.client(&server.url());
    wait_connected(&mut ec).await;

    let (b, cb, mut eb, qr) = newcomer(&server.url(), "windows").await;
    assert!(qr.starts_with("oldtown://sponsor?o="));
    let (id, name) = within(ca.sponsor(&qr)).await.unwrap();
    assert_eq!((id, name.as_str()), (b.id.clone(), "windows"));

    // B finit par s'authentifier, reçoit la réponse et adopte le groupe.
    wait_for(&mut eb, group_changed).await;
    wait_for(&mut ea, |e| matches!(e, Event::PairingCompleted { .. })).await;
    wait_for(&mut ec, group_changed).await;
    let roster = within(cc.group()).await.unwrap().unwrap();
    assert_eq!(roster.member(&b.id).unwrap().name, "windows");

    within(cb.send(&Content::text("bonjour du PC"), vec![])).await.unwrap();
    assert_eq!(next_message(&mut ea).await.content, Content::text("bonjour du PC"));
    assert_eq!(next_message(&mut ec).await.content, Content::text("bonjour du PC"));
    within(ca.send(&Content::link("https://example.org/"), vec![b.id.clone()])).await.unwrap();
    assert_eq!(next_message(&mut eb).await.content, Content::link("https://example.org/"));

    // Le même QR ne sert pas deux fois.
    assert!(within(ca.sponsor(&qr)).await.is_err());
}

#[tokio::test]
async fn relay_tampering_with_reverse_qr_welcome_fails() {
    let (server, devs) = group_server(&["android"]).await;
    // Le relais altère tout ce que le parrain envoie.
    let evil = proxy(
        &server,
        Arc::new(|dir, frame| if dir == Dir::ToServer { flip_payload(frame) } else { frame }),
    )
    .await;
    let (ca, mut ea) = devs[0].client(&evil);
    wait_connected(&mut ea).await;

    let (_b, cb, mut eb, qr) = newcomer(&server.url(), "windows").await;
    within(ca.sponsor(&qr)).await.unwrap();
    match wait_for(&mut eb, |e| matches!(e, Event::Rejected { .. })).await {
        Event::Rejected { reason, .. } => assert_eq!(reason, "déchiffrement impossible"),
        _ => unreachable!(),
    }
    assert!(within(cb.group()).await.unwrap().is_none(), "B n'a pas de groupe");
}

#[tokio::test]
async fn relay_registering_another_key_fails() {
    let (server, devs) = group_server(&["android"]).await;
    // Le relais remplace la clé de B par la sienne (signature refaite).
    let attacker = ed25519_dalek::SigningKey::from_bytes(&[66; 32]);
    let evil_key = attacker.clone();
    let evil = proxy(
        &server,
        Arc::new(move |dir, frame| match (dir, frame) {
            (Dir::ToServer, Frame::PairRegister { device_id, name, .. }) => {
                use ed25519_dalek::Signer;
                let public_key = evil_key.verifying_key().to_bytes();
                let msg = ot_proto::pair_register_message(&device_id, &public_key, &name);
                let sig = evil_key.sign(&msg).to_bytes();
                Frame::PairRegister { device_id, name, public_key, sig }
            }
            (_, frame) => frame,
        }),
    )
    .await;
    let (ca, mut ea) = devs[0].client(&evil);
    wait_connected(&mut ea).await;

    let (b, cb, mut eb, qr) = newcomer(&server.url(), "windows").await;
    within(ca.sponsor(&qr)).await.unwrap();

    // B ne peut pas s'authentifier : le relais a enregistré une autre clé.
    wait_for(&mut eb, |e| matches!(e, Event::AuthFailed { .. })).await;
    no_message(&mut eb, Duration::from_millis(300)).await;
    assert!(within(cb.group()).await.unwrap().is_none());

    // L'imposteur s'authentifie sous l'identifiant de B, reçoit la réponse,
    // mais ne l'ouvre pas : il n'a ni `S` ni la clé X25519 de B.
    let impostor = TestDevice {
        id: b.id.clone(),
        name: "windows".into(),
        identity: Identity::from_bytes(&attacker.to_bytes()),
        keyring: Arc::new(MemoryKeyring::new()),
    };
    let (ci, mut ei) = impostor.client(&server.url());
    wait_connected(&mut ei).await;
    wait_for(&mut ei, |e| matches!(e, Event::Rejected { .. })).await;
    assert!(within(ci.group()).await.unwrap().is_none());

    // Le roster du parrain porte la vraie clé de B : rien de signé par
    // l'imposteur n'y sera accepté.
    let group = within(ca.group()).await.unwrap().unwrap();
    let member = group.member(&b.id).unwrap();
    let real = ot_core::crypto::fingerprint(&b.identity.public_key());
    assert_eq!(member.sign_fingerprint, real);
}
