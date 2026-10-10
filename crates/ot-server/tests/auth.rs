//! T5 : authentification par défi signé.

mod common;

use std::time::Duration;

use common::*;
use ot_core::Identity;
use ot_proto::{DeviceId, ErrorCode, Frame};
use ot_server::store::Store;

async fn expect_auth_failed(raw: &mut Raw) {
    match raw.recv().await {
        Some(Frame::Error { code: ErrorCode::AuthFailed, .. }) => {}
        other => panic!("auth_failed attendu, reçu {other:?}"),
    }
    assert_eq!(raw.recv().await, None, "la connexion doit être fermée");
}

async fn setup() -> (TestServer, TestDevice) {
    let store = Store::in_memory().await.unwrap();
    let dev = add_device(&store, "mac").await;
    (TestServer::start(store, test_config()).await, dev)
}

#[tokio::test]
async fn good_key_is_welcomed() {
    let (server, dev) = setup().await;
    let (_raw, pending) = Raw::login(&server, &dev).await;
    assert_eq!(pending, 0);
}

#[tokio::test]
async fn wrong_key_is_refused() {
    let (server, dev) = setup().await;
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    let sig = Identity::generate().sign_challenge(&nonce, &dev.id);
    raw.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
    expect_auth_failed(&mut raw).await;
}

#[tokio::test]
async fn unknown_device_is_refused() {
    let (server, _) = setup().await;
    let stranger = Identity::generate();
    let id = DeviceId::generate();
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    raw.send(&Frame::Auth { sig: stranger.sign_challenge(&nonce, &id), device_id: id }).await;
    expect_auth_failed(&mut raw).await;
}

#[tokio::test]
async fn signature_bound_to_device_id() {
    let (server, dev) = setup().await;
    let other = add_device(server.relay.store(), "windows").await;
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    // Bonne clé, mais signature calculée pour un autre identifiant.
    let sig = dev.identity.sign_challenge(&nonce, &other.id);
    raw.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
    expect_auth_failed(&mut raw).await;
}

#[tokio::test]
async fn replayed_nonce_is_refused() {
    let (server, dev) = setup().await;
    let mut first = Raw::connect(&server).await;
    let nonce = first.challenge().await;
    let sig = dev.identity.sign_challenge(&nonce, &dev.id);

    // Une signature valide pour un défi ne vaut rien sur une autre connexion.
    let mut second = Raw::connect(&server).await;
    let other_nonce = second.challenge().await;
    assert_ne!(nonce, other_nonce);
    second.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
    expect_auth_failed(&mut second).await;
}

#[tokio::test]
async fn revoked_device_is_refused() {
    let (server, dev) = setup().await;
    server.relay.store().revoke_device("mac", 1).await.unwrap();
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    let sig = dev.identity.sign_challenge(&nonce, &dev.id);
    raw.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
    expect_auth_failed(&mut raw).await;

    // Le client ot-core le signale sans planter.
    let (_client, mut events) = dev.client(&server.url());
    loop {
        if let ot_core::Event::AuthFailed { .. } = next_event(&mut events).await {
            break;
        }
    }
}

#[tokio::test]
async fn frames_before_auth_are_rejected_without_closing() {
    let (server, dev) = setup().await;
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    raw.send(&Frame::Ack { seq: 1 }).await;
    assert!(matches!(
        raw.recv().await,
        Some(Frame::Error { code: ErrorCode::UnexpectedFrame, .. })
    ));
    raw.send_text("pas du json".into()).await;
    assert!(matches!(raw.recv().await, Some(Frame::Error { code: ErrorCode::BadFrame, .. })));

    let sig = dev.identity.sign_challenge(&nonce, &dev.id);
    raw.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
    assert!(matches!(raw.recv().await, Some(Frame::Welcome { .. })));
}

#[tokio::test]
async fn auth_times_out() {
    let store = Store::in_memory().await.unwrap();
    let config = ot_server::Config { auth_timeout: Duration::from_millis(200), ..test_config() };
    let server = TestServer::start(store, config).await;
    let mut raw = Raw::connect(&server).await;
    raw.challenge().await;
    expect_auth_failed(&mut raw).await;
}
