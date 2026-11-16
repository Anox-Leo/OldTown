//! Étape 2, T4 : appairage côté relais (jetons à usage unique).

mod common;

use std::time::Duration;

use common::*;
use ot_core::Identity;
use ot_proto::{ErrorCode, Frame, PAIR_TOKEN_LEN};
use ot_server::store::{Store, StoreError};
use ot_server::{Config, now_ms, token_hash};

async fn setup(config: Config) -> (TestServer, TestDevice) {
    let store = Store::in_memory().await.unwrap();
    let sponsor = add_device(&store, "mac").await;
    (TestServer::start(store, config).await, sponsor)
}

/// Le parrain demande un jeton.
async fn create_token(server: &TestServer, sponsor: &TestDevice) -> [u8; PAIR_TOKEN_LEN] {
    let (mut raw, _) = Raw::login(server, sponsor).await;
    raw.send(&Frame::PairCreate).await;
    match raw.recv().await {
        Some(Frame::PairCreated { token, expires_at }) => {
            assert!(expires_at > time::OffsetDateTime::now_utc());
            token
        }
        other => panic!("pair_created attendu, reçu {other:?}"),
    }
}

/// Le nouvel appareil présente le jeton ; renvoie la réponse, puis vérifie
/// que la connexion est fermée.
async fn redeem(
    server: &TestServer,
    token: &[u8; PAIR_TOKEN_LEN],
    name: &str,
    id: &Identity,
) -> Frame {
    let mut raw = Raw::connect(server).await;
    let nonce = raw.challenge().await;
    let sig = id.sign_pair_redeem(&nonce, &token_hash(token));
    raw.send(&Frame::PairRedeem {
        token: *token,
        name: name.into(),
        public_key: id.public_key(),
        sig,
    })
    .await;
    let reply = raw.recv().await.expect("réponse attendue");
    assert_eq!(raw.recv().await, None, "la connexion doit être fermée");
    reply
}

fn assert_error(frame: Frame, expected: ErrorCode) {
    match frame {
        Frame::Error { code, .. } => assert_eq!(code, expected),
        other => panic!("erreur {expected:?} attendue, reçu {other:?}"),
    }
}

#[tokio::test]
async fn valid_token_registers_device() {
    let (server, sponsor) = setup(test_config()).await;
    let token = create_token(&server, &sponsor).await;
    let id = Identity::generate();
    let device_id = match redeem(&server, &token, "windows", &id).await {
        Frame::PairRedeemed { device_id, sponsor: s } => {
            assert_eq!(s, sponsor.id);
            device_id
        }
        other => panic!("pair_redeemed attendu, reçu {other:?}"),
    };
    let stored = server.relay.store().device(&device_id).await.unwrap().unwrap();
    assert_eq!((stored.name.as_str(), stored.public_key), ("windows", id.public_key()));

    // L'appareil s'authentifie ensuite comme n'importe quel autre.
    let dev = TestDevice {
        id: device_id,
        name: "windows".into(),
        identity: id,
        keyring: Default::default(),
    };
    Raw::login(&server, &dev).await;

    let (_, body) = http_get(server.admin, "/metrics").await;
    assert!(body.contains("ot_pairings_total{result=\"created\"} 1"), "{body}");
    assert!(body.contains("ot_pairings_total{result=\"redeemed\"} 1"), "{body}");
}

#[tokio::test]
async fn used_token_is_refused() {
    let (server, sponsor) = setup(test_config()).await;
    let token = create_token(&server, &sponsor).await;
    assert!(matches!(
        redeem(&server, &token, "windows", &Identity::generate()).await,
        Frame::PairRedeemed { .. }
    ));
    let again = redeem(&server, &token, "android", &Identity::generate()).await;
    assert_error(again, ErrorCode::PairingInvalid);
}

#[tokio::test]
async fn expired_token_is_refused() {
    let config = Config { pairing_ttl: Duration::from_millis(100), ..test_config() };
    let (server, sponsor) = setup(config).await;
    let token = create_token(&server, &sponsor).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_error(
        redeem(&server, &token, "windows", &Identity::generate()).await,
        ErrorCode::PairingInvalid,
    );

    // La purge supprime le jeton expiré.
    let stats = server.relay.purge_once().await.unwrap();
    assert_eq!(stats.pairings, 1);
}

#[tokio::test]
async fn unknown_token_is_refused() {
    let (server, _) = setup(test_config()).await;
    let reply = redeem(&server, &[1; PAIR_TOKEN_LEN], "windows", &Identity::generate()).await;
    assert_error(reply, ErrorCode::PairingInvalid);
    let (_, body) = http_get(server.admin, "/metrics").await;
    assert!(body.contains("ot_pairings_total{result=\"rejected\"} 1"), "{body}");
}

#[tokio::test]
async fn wrong_signature_is_refused() {
    let (server, sponsor) = setup(test_config()).await;
    let token = create_token(&server, &sponsor).await;
    let id = Identity::generate();
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    // Signée par une autre clé que celle présentée.
    let sig = Identity::generate().sign_pair_redeem(&nonce, &token_hash(&token));
    raw.send(&Frame::PairRedeem {
        token,
        name: "windows".into(),
        public_key: id.public_key(),
        sig,
    })
    .await;
    assert_error(raw.recv().await.unwrap(), ErrorCode::PairingInvalid);
    assert_eq!(raw.recv().await, None);

    // Le jeton n'est pas consommé.
    assert!(matches!(redeem(&server, &token, "windows", &id).await, Frame::PairRedeemed { .. }));
}

#[tokio::test]
async fn taken_name_is_refused_and_token_kept() {
    let (server, sponsor) = setup(test_config()).await;
    let token = create_token(&server, &sponsor).await;
    assert_error(redeem(&server, &token, "mac", &Identity::generate()).await, ErrorCode::NameTaken);
    assert!(matches!(
        redeem(&server, &token, "windows", &Identity::generate()).await,
        Frame::PairRedeemed { .. }
    ));
}

#[tokio::test]
async fn invalid_name_is_refused() {
    let (server, sponsor) = setup(test_config()).await;
    let token = create_token(&server, &sponsor).await;
    for name in ["", "   ", "a\nb", &"x".repeat(65)] {
        assert_error(
            redeem(&server, &token, name, &Identity::generate()).await,
            ErrorCode::BadFrame,
        );
    }
}

#[tokio::test]
async fn revoked_sponsor_token_is_refused() {
    let (server, sponsor) = setup(test_config()).await;
    let token = create_token(&server, &sponsor).await;
    server.relay.store().revoke_device("mac", now_ms()).await.unwrap();
    assert_error(
        redeem(&server, &token, "windows", &Identity::generate()).await,
        ErrorCode::PairingInvalid,
    );
}

#[tokio::test]
async fn pair_create_before_auth_is_refused() {
    let (server, _) = setup(test_config()).await;
    let mut raw = Raw::connect(&server).await;
    raw.challenge().await;
    raw.send(&Frame::PairCreate).await;
    assert!(matches!(
        raw.recv().await,
        Some(Frame::Error { code: ErrorCode::UnexpectedFrame, .. })
    ));
}

#[tokio::test]
async fn store_never_keeps_token_in_clear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ot.db");
    let store = Store::open(&path).await.unwrap();
    let sponsor = store.add_device("mac", &[1; 32], 0).await.unwrap();
    let token = [0xA5u8; PAIR_TOKEN_LEN];
    store.create_pairing(&sponsor.id, &token_hash(&token), 0, 1_000).await.unwrap();
    assert!(matches!(
        store.redeem_pairing(&token_hash(&[0; PAIR_TOKEN_LEN]), "x", &[2; 32], 1).await,
        Err(StoreError::PairingInvalid)
    ));
    store.redeem_pairing(&token_hash(&token), "windows", &[2; 32], 1).await.unwrap();
    store.close().await;

    let mut bytes = std::fs::read(&path).unwrap();
    if let Ok(wal) = std::fs::read(path.with_extension("db-wal")) {
        bytes.extend(wal);
    }
    assert!(!bytes.windows(token.len()).any(|w| w == token), "jeton en clair dans la base");
    let hash = token_hash(&token);
    assert!(bytes.windows(hash.len()).any(|w| w == hash), "le hash doit y être");
}

/// Étape 3, T9 : enregistrement par le parrain (QR inversé, note 0022).
fn register_frame(id: &ot_proto::DeviceId, name: &str, key: &ed25519_dalek::SigningKey) -> Frame {
    use ed25519_dalek::Signer;
    let public_key = key.verifying_key().to_bytes();
    let sig = key.sign(&ot_proto::pair_register_message(id, &public_key, name)).to_bytes();
    Frame::PairRegister { device_id: id.clone(), name: name.into(), public_key, sig }
}

#[tokio::test]
async fn sponsor_registers_a_device_by_reverse_qr() {
    let (server, sponsor) = setup(test_config()).await;
    let (mut raw, _) = Raw::login(&server, &sponsor).await;
    let id = ot_proto::DeviceId::generate();
    let key = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);

    raw.send(&register_frame(&id, "windows", &key)).await;
    assert_eq!(raw.recv().await, Some(Frame::PairRegistered { device_id: id.clone() }));
    let stored = server.relay.store().device(&id).await.unwrap().unwrap();
    assert_eq!(
        (stored.name.as_str(), stored.public_key),
        ("windows", key.verifying_key().to_bytes())
    );

    // Le nouvel appareil s'authentifie sous l'identifiant qu'il a choisi.
    let nonce_dev = TestDevice {
        id: id.clone(),
        name: "windows".into(),
        identity: Identity::from_bytes(&[9; 32]),
        keyring: std::sync::Arc::new(ot_core::MemoryKeyring::new()),
    };
    let (_b, pending) = Raw::login(&server, &nonce_dev).await;
    assert_eq!(pending, 0);

    // Rejeu : identifiant et clé déjà pris ; la session du parrain reste ouverte.
    raw.send(&register_frame(&id, "autre", &key)).await;
    match raw.recv().await {
        Some(Frame::Error { code: ErrorCode::PairingInvalid, reference, .. }) => {
            assert_eq!(reference.as_deref(), Some(id.as_str()));
        }
        other => panic!("pairing_invalid attendu, reçu {other:?}"),
    }
    raw.send(&Frame::ListDevices).await;
    assert!(matches!(raw.recv().await, Some(Frame::Devices { .. })));
}

#[tokio::test]
async fn reverse_qr_registration_is_checked() {
    let (server, sponsor) = setup(test_config()).await;
    let (mut raw, _) = Raw::login(&server, &sponsor).await;
    let key = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
    let id = ot_proto::DeviceId::generate();

    // Nom changé après signature (relais ou proxy qui triche).
    let Frame::PairRegister { device_id, public_key, sig, .. } =
        register_frame(&id, "windows", &key)
    else {
        unreachable!()
    };
    raw.send(&Frame::PairRegister { device_id, name: "intrus".into(), public_key, sig }).await;
    assert_error(raw.recv().await.unwrap(), ErrorCode::PairingInvalid);
    // Nom déjà pris, nom invalide.
    raw.send(&register_frame(&id, "mac", &key)).await;
    assert_error(raw.recv().await.unwrap(), ErrorCode::NameTaken);
    raw.send(&register_frame(&id, " windows", &key)).await;
    assert_error(raw.recv().await.unwrap(), ErrorCode::BadFrame);
    assert!(server.relay.store().device(&id).await.unwrap().is_none());

    // Hors session authentifiée : refusé avant `auth`.
    let mut anon = Raw::connect(&server).await;
    anon.challenge().await;
    anon.send(&register_frame(&id, "windows", &key)).await;
    assert_error(anon.recv().await.unwrap(), ErrorCode::UnexpectedFrame);
}
