//! Étape 8e : limites d'un relais exposé (connexions non authentifiées,
//! échecs par IP, IP réelle derrière un frontal de confiance).

mod common;

use std::time::Duration;

use common::*;
use ot_proto::{DeviceId, ErrorCode, Frame};
use ot_server::Config;
use ot_server::limits::LimitsConfig;
use ot_server::store::Store;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn server(limits: LimitsConfig) -> TestServer {
    server_with(Config { limits, ..test_config() }).await
}

async fn server_with(config: Config) -> TestServer {
    TestServer::start(Store::in_memory().await.unwrap(), config).await
}

/// Ouvre une connexion WebSocket, éventuellement avec un `X-Forwarded-For` ;
/// renvoie le code HTTP du refus.
async fn connect(server: &TestServer, forwarded: Option<&str>) -> Result<Raw, u16> {
    let url = format!("{}{}", server.url(), ot_proto::WS_PATH);
    let mut req = url.into_client_request().unwrap();
    if let Some(ip) = forwarded {
        req.headers_mut().insert("x-forwarded-for", ip.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(Raw::from_stream(ws)),
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => Err(res.status().as_u16()),
        Err(e) => panic!("connexion : {e}"),
    }
}

/// Une authentification refusée (appareil inconnu).
async fn fail_auth(server: &TestServer, forwarded: Option<&str>) {
    let mut raw = connect(server, forwarded).await.expect("connexion admise");
    raw.challenge().await;
    raw.send(&Frame::Auth { device_id: DeviceId::generate(), sig: [0; 64] }).await;
    assert!(matches!(raw.recv().await, Some(Frame::Error { code: ErrorCode::AuthFailed, .. })));
    assert_eq!(raw.recv().await, None);
}

async fn metric(server: &TestServer, line: &str) -> bool {
    http_get(server.admin, "/metrics").await.1.contains(line)
}

#[tokio::test]
async fn unauthenticated_connections_are_bounded_per_ip() {
    let server = server(LimitsConfig { max_unauth_per_ip: 2, ..LimitsConfig::default() }).await;
    let dev = add_device(server.relay.store(), "mac").await;

    let mut first = connect(&server, None).await.unwrap();
    let mut second = connect(&server, None).await.unwrap();
    let nonce = first.challenge().await;
    second.challenge().await;
    assert_eq!(connect(&server, None).await.err(), Some(503), "troisième en attente refusée");
    assert!(metric(&server, "ot_limited_total{reason=\"busy\"} 1").await);

    // Une connexion authentifiée rend sa place.
    let sig = dev.identity.sign_challenge(&nonce, &dev.id);
    first.send(&Frame::Auth { device_id: dev.id.clone(), sig }).await;
    assert!(matches!(first.recv().await, Some(Frame::Welcome { .. })));
    let mut third = connect(&server, None).await.expect("place rendue après l'auth");
    third.challenge().await;

    // Une connexion fermée aussi.
    second.close().await;
    third.close().await;
    within(async {
        loop {
            match connect(&server, None).await {
                Ok(mut raw) => {
                    raw.challenge().await;
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await;
}

#[tokio::test]
async fn an_auth_timeout_gives_the_place_back() {
    let server = server_with(Config {
        auth_timeout: Duration::from_millis(200),
        limits: LimitsConfig { max_unauth_per_ip: 1, ..LimitsConfig::default() },
        ..test_config()
    })
    .await;
    let mut idle = connect(&server, None).await.unwrap();
    idle.challenge().await;
    assert_eq!(connect(&server, None).await.err(), Some(503));
    assert!(matches!(idle.recv().await, Some(Frame::Error { code: ErrorCode::AuthFailed, .. })));
    assert_eq!(idle.recv().await, None);
    let mut next = connect(&server, None).await.expect("place rendue au délai");
    next.challenge().await;
}

#[tokio::test]
async fn repeated_failures_block_the_ip_until_it_recovers() {
    let server = server(LimitsConfig {
        failure_burst: 3,
        failure_refill: Duration::from_millis(300),
        ..LimitsConfig::default()
    })
    .await;
    let dev = add_device(server.relay.store(), "mac").await;
    for _ in 0..3 {
        fail_auth(&server, None).await;
    }
    // Même un appareil connu est refusé depuis cette IP, avant tout défi.
    assert_eq!(connect(&server, None).await.err(), Some(503));
    assert!(metric(&server, "ot_limited_total{reason=\"failures\"} 1").await);

    tokio::time::sleep(Duration::from_millis(400)).await;
    let (session, _) = Raw::login(&server, &dev).await;
    session.close().await;
}

#[tokio::test]
async fn invalid_pairing_tokens_count_as_failures() {
    let server = server(LimitsConfig {
        failure_burst: 2,
        failure_refill: Duration::from_secs(60),
        ..LimitsConfig::default()
    })
    .await;
    for _ in 0..2 {
        let mut raw = connect(&server, None).await.unwrap();
        let nonce = raw.challenge().await;
        let identity = ot_core::Identity::generate();
        let token = [7; ot_proto::PAIR_TOKEN_LEN];
        let sig = identity.sign_pair_redeem(&nonce, &ot_server::token_hash(&token));
        let public_key = identity.public_key();
        raw.send(&Frame::PairRedeem { token, name: "pc".into(), public_key, sig }).await;
        assert!(matches!(
            raw.recv().await,
            Some(Frame::Error { code: ErrorCode::PairingInvalid, .. })
        ));
    }
    assert_eq!(connect(&server, None).await.err(), Some(503));
}

#[tokio::test]
async fn refused_file_requests_count_as_failures() {
    let server = server(LimitsConfig {
        failure_burst: 2,
        failure_refill: Duration::from_secs(60),
        ..LimitsConfig::default()
    })
    .await;
    let path = "/v1/blobs/blob_01J0000000000000000000000";
    for _ in 0..2 {
        assert_eq!(http_get(server.addr, path).await.0, 401, "requête non signée");
    }
    let (code, body) = http_get(server.addr, path).await;
    assert_eq!(code, 503);
    assert!(body.contains("\"busy\""), "{body}");
    assert_eq!(connect(&server, None).await.err(), Some(503), "WebSocket aussi");
}

#[tokio::test]
async fn forwarded_for_identifies_clients_behind_a_trusted_proxy() {
    let server = server(LimitsConfig {
        failure_burst: 1,
        failure_refill: Duration::from_secs(60),
        trusted_proxies: vec!["127.0.0.1".parse().unwrap()],
        ..LimitsConfig::default()
    })
    .await;
    fail_auth(&server, Some("198.51.100.1")).await;
    assert_eq!(connect(&server, Some("198.51.100.1")).await.err(), Some(503));
    let mut other = connect(&server, Some("198.51.100.2")).await.expect("autre client admis");
    other.challenge().await;
    let mut direct = connect(&server, None).await.expect("le frontal lui-même admis");
    direct.challenge().await;
}

#[tokio::test]
async fn forwarded_for_is_ignored_without_a_trusted_proxy() {
    let server = server(LimitsConfig {
        failure_burst: 1,
        failure_refill: Duration::from_secs(60),
        ..LimitsConfig::default()
    })
    .await;
    fail_auth(&server, Some("198.51.100.1")).await;
    assert_eq!(
        connect(&server, Some("198.51.100.2")).await.err(),
        Some(503),
        "l'en-tête d'un client quelconque ne change pas son IP"
    );
}
