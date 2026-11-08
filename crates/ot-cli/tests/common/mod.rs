//! Utilitaires des tests de la CLI : appareils réels (`ot init`) contre un
//! relais en processus, mis dans un même groupe.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use ot_core::crypto::{GroupKey, Member};
use ot_core::{GroupKeys, Keyring, KeyringError, Roster, SecretStore, StoreKeyring};
use ot_server::store::Store;
use ot_server::{Config, Relay};
use tokio::process::Command;
use zeroize::Zeroizing;

/// `SecretStore` sur le dossier de la CLI : un fichier par nom, comme `ot`.
struct Dir(PathBuf);

impl SecretStore for Dir {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError> {
        Ok(std::fs::read(self.0.join(name)).ok().map(Zeroizing::new))
    }
    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError> {
        std::fs::write(self.0.join(name), value).map_err(|e| KeyringError(e.to_string()))
    }
    fn delete(&self, name: &str) -> Result<(), KeyringError> {
        let _ = std::fs::remove_file(self.0.join(name));
        Ok(())
    }
}

pub async fn ot(home: &Path, args: &[&str]) -> Output {
    let out = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(env!("CARGO_BIN_EXE_ot")).arg("--home").arg(home).args(args).output(),
    )
    .await
    .expect("délai dépassé")
    .unwrap();
    assert!(
        out.status.success(),
        "ot {args:?} : {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// Crée l'appareil `name` (`ot init`) et l'enregistre sur le relais.
pub async fn device(store: &Store, root: &Path, url: &str, name: &str) -> (PathBuf, Member) {
    let home = root.join(name);
    let out = ot(&home, &["init", "--server", url, "--name", name]).await;
    let stdout = String::from_utf8(out.stdout).unwrap();
    let key = stdout
        .lines()
        .find_map(|l| l.strip_prefix("Clé publique  : "))
        .expect("clé publique affichée");
    let key = ot_proto::b64::decode_array::<32>(key.trim()).unwrap();
    let id = store.add_device(name, &key, 0).await.unwrap().id;
    ot(&home, &["init", "--device-id", id.as_str()]).await;
    let keyring = StoreKeyring::new(Arc::new(Dir(home.clone())));
    let exchange = keyring.load_exchange_key().unwrap().unwrap();
    let member = Member {
        device_id: id,
        name: name.into(),
        sign_key: key,
        exchange_key: exchange.public_key(),
        added_epoch: 1,
    };
    (home, member)
}

/// Relais en processus dans `root` ; renvoie son magasin et son URL.
pub async fn relay(root: &Path) -> (Store, String) {
    let store = Store::open(&root.join("ot.db")).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let config = Config { blob_dir: root.join("blobs"), ..Config::default() };
    let relay = Relay::new(store.clone(), config);
    tokio::spawn(ot_server::serve(relay, listener, std::future::pending()));
    (store, url)
}

/// Met les appareils dans un même groupe, époque 1.
pub fn group(devices: &[(&PathBuf, Member)]) {
    let roster =
        Roster { epoch: 1, version: 1, members: devices.iter().map(|(_, m)| m.clone()).collect() };
    let keys = GroupKeys::new(1, GroupKey::generate());
    for (home, _) in devices {
        let keyring = StoreKeyring::new(Arc::new(Dir((*home).clone())));
        keyring.save_group_keys(&keys).unwrap();
        keyring.save_roster(&roster).unwrap();
    }
}
