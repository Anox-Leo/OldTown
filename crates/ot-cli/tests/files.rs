//! Étape 5, T4 : envoi et réception de fichiers par la CLI, contre un relais
//! en processus. Deux appareils réels (`ot init`), enregistrés sur le relais
//! et mis dans un même groupe.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use ot_core::crypto::{GroupKey, Member};
use ot_core::{GroupKeys, Keyring, KeyringError, Roster, SecretStore, StoreKeyring};
use ot_server::store::Store;
use ot_server::{Config, Relay};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
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

async fn ot(home: &Path, args: &[&str]) -> Output {
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
async fn device(store: &Store, root: &Path, url: &str, name: &str) -> (PathBuf, Member) {
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

#[tokio::test]
async fn file_between_two_cli_devices() {
    let root = TempDir::new().unwrap();
    let store = Store::open(&root.path().join("ot.db")).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let config = Config { blob_dir: root.path().join("blobs"), ..Config::default() };
    let relay = Relay::new(store.clone(), config);
    tokio::spawn(ot_server::serve(relay, listener, std::future::pending()));

    let (home_a, a) = device(&store, root.path(), &url, "mac").await;
    let (home_b, b) = device(&store, root.path(), &url, "windows").await;
    let roster = Roster { epoch: 1, version: 1, members: vec![a, b] };
    let keys = GroupKeys::new(1, GroupKey::generate());
    for home in [&home_a, &home_b] {
        let keyring = StoreKeyring::new(Arc::new(Dir(home.clone())));
        keyring.save_group_keys(&keys).unwrap();
        keyring.save_roster(&roster).unwrap();
    }

    // 3 Mio de contenu, envoyé de « mac » à « windows ».
    let data: Vec<u8> =
        (0..3u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let src = root.path().join("rapport final.pdf");
    std::fs::write(&src, &data).unwrap();
    ot(&home_a, &["send", "--file", src.to_str().unwrap(), "--to", "windows"]).await;

    // `listen` annonce le fichier et la commande pour le télécharger.
    let mut listen = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&home_b)
        .arg("listen")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(listen.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(20), lines.next_line())
        .await
        .expect("annonce du fichier")
        .unwrap()
        .unwrap();
    assert!(line.contains("rapport final.pdf") && line.contains("3,0 Mo"), "{line}");
    let blob = line.rsplit(' ').next().unwrap().to_owned();
    assert!(blob.starts_with("blob_"), "{line}");
    let _ = listen.kill().await;

    let listed = String::from_utf8(ot(&home_b, &["files"]).await.stdout).unwrap();
    assert!(listed.starts_with(&blob) && listed.contains("rapport final.pdf"), "{listed}");
    let out_dir = root.path().join("reçus");
    ot(&home_b, &["download", &blob, "--to", out_dir.to_str().unwrap()]).await;
    assert!(std::fs::read(out_dir.join("rapport final.pdf")).unwrap() == data);
    // Plus rien en attente : l'offre est oubliée après le téléchargement.
    let listed = String::from_utf8(ot(&home_b, &["files"]).await.stdout).unwrap();
    assert!(listed.trim().is_empty(), "{listed}");

    // `listen --download-dir` télécharge tout seul, sans écraser le premier.
    let mut listen = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&home_b)
        .args(["listen", "--download-dir", out_dir.to_str().unwrap()])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    ot(&home_a, &["send", "--file", src.to_str().unwrap()]).await;
    let second = out_dir.join("rapport final (2).pdf");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !second.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("fichier téléchargé par listen");
    assert!(std::fs::read(&second).unwrap() == data);
    let _ = listen.kill().await;
}
