//! Étape 2, T4 : aucun jeton d'appairage dans les logs du relais.
//!
//! Seul test de son binaire : le subscriber `tracing` de test est propre au
//! fil, et le cache d'intérêt des callsites est global au processus ; des
//! tests parallèles dans le même binaire feraient perdre des lignes.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::*;
use ot_core::Identity;
use ot_proto::{Frame, PAIR_TOKEN_LEN, b64};
use ot_server::store::Store;
use ot_server::token_hash;

async fn redeem(server: &TestServer, token: &[u8; PAIR_TOKEN_LEN], name: &str) {
    let id = Identity::generate();
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
    while raw.recv().await.is_some() {}
}

/// Tampon partagé où le subscriber de test écrit les logs.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn token_never_logged() {
    let logs = Capture::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    // Test monothread : le relais tourne sur ce fil, sous ce subscriber.
    let _guard = tracing::subscriber::set_default(subscriber);

    let store = Store::in_memory().await.unwrap();
    let sponsor = add_device(&store, "mac").await;
    let server = TestServer::start(store, test_config()).await;
    let (mut raw, _) = Raw::login(&server, &sponsor).await;
    raw.send(&Frame::PairCreate).await;
    let Some(Frame::PairCreated { token, .. }) = raw.recv().await else {
        panic!("pair_created attendu");
    };
    redeem(&server, &[9; PAIR_TOKEN_LEN], "android").await;
    redeem(&server, &token, "windows").await;

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(text.contains("appareil appairé"), "les logs doivent être capturés : {text}");
    for needle in [b64::encode(&token), format!("{token:?}"), format!("{:?}", token_hash(&token))] {
        assert!(!text.contains(&needle), "jeton dans les logs : {needle}");
    }
}
