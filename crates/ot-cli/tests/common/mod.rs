//! Utilitaires des tests de la CLI : appareils réels (`ot init`) contre un
//! relais en processus, mis dans un même groupe ; depuis l'étape 10,
//! personnes invitées et appareils de service, par les commandes `ot`.
// Chaque fichier de test n'en utilise qu'une partie.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ot_core::crypto::{GroupKey, Member};
use ot_core::{GroupKeys, Keyring, KeyringError, Roster, SecretStore, StoreKeyring};
use ot_server::store::Store;
use ot_server::{Config, Relay};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
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

/// `ot --home <home> <args>`, qui doit échouer ; renvoie sa sortie d'erreur.
pub async fn ot_fails(home: &Path, args: &[&str]) -> String {
    let out = run(home, args).await;
    assert!(!out.status.success(), "ot {args:?} aurait dû échouer");
    String::from_utf8(out.stderr).unwrap()
}

/// Sortie standard de `ot --home <home> <args>`, qui doit réussir.
pub async fn ot_stdout(home: &Path, args: &[&str]) -> String {
    String::from_utf8(ot(home, args).await.stdout).unwrap()
}

async fn run(home: &Path, args: &[&str]) -> Output {
    tokio::time::timeout(
        Duration::from_secs(30),
        Command::new(env!("CARGO_BIN_EXE_ot")).arg("--home").arg(home).args(args).output(),
    )
    .await
    .expect("délai dépassé")
    .unwrap()
}

pub async fn ot(home: &Path, args: &[&str]) -> Output {
    let out = run(home, args).await;
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

/// `ot` lancé en tâche de fond, sortie standard lue ligne à ligne.
pub fn spawn(home: &Path, args: &[&str]) -> (Child, Lines<BufReader<ChildStdout>>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(home)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stdout.take().unwrap()).lines();
    (child, lines)
}

/// Première ligne (sans ses espaces) qui commence par `prefix`.
pub async fn line_starting(lines: &mut Lines<BufReader<ChildStdout>>, prefix: &str) -> String {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let line = lines.next_line().await.unwrap().expect("sortie fermée trop tôt");
            if line.trim().starts_with(prefix) {
                return line.trim().to_owned();
            }
        }
    })
    .await
    .expect("ligne attendue")
}

/// Attend la fin d'un `ot` lancé en tâche de fond, qui doit réussir.
pub async fn finished(mut child: Child) {
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait())
        .await
        .expect("délai dépassé")
        .unwrap();
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err).await;
        }
        panic!("ot en tâche de fond : {status} {err}");
    }
}

/// Premier appareil de l'admin (Alice), seul dans son groupe, annuaire ouvert
/// (`ot group init`, `ot admin init`).
pub async fn admin(store: &Store, root: &Path, url: &str) -> PathBuf {
    let (home, _) = device(store, root, url, "alice-laptop").await;
    // Le nom de l'admin est signé dans son certificat : le donner avant
    // (`ot-server user rename owner Alice`).
    let owner = store.admin().await.unwrap().id;
    store.rename_user(&owner, "Alice").await.unwrap();
    ot(&home, &["group", "init"]).await;
    ot(&home, &["admin", "init"]).await;
    home
}

/// Invite `name` (lié à `links`) depuis l'appareil de l'admin et le fait
/// rejoindre avec l'appareil `device` ; renvoie le dossier de l'invité.
pub async fn invite(
    admin: &Path,
    root: &Path,
    name: &str,
    links: &[&str],
    device: &str,
) -> PathBuf {
    let mut args = vec!["admin", "invite", name];
    for l in links {
        args.extend(["--link", l]);
    }
    let (child, mut lines) = spawn(admin, &args);
    let uri = line_starting(&mut lines, "oldtown://invite?").await;
    let home = root.join(device);
    ot(&home, &["invite", "join", &uri, "--name", device]).await;
    finished(child).await;
    home
}

/// Appareil de service `device` ajouté par l'admin au service `service`
/// (`ot pair join --qr --service`, puis `ot admin service add`).
pub async fn service(admin: &Path, root: &Path, url: &str, service: &str, device: &str) -> PathBuf {
    let home = root.join(device);
    let (child, mut lines) =
        spawn(&home, &["pair", "join", "--qr", "--service", "--server", url, "--name", device]);
    let qr = line_starting(&mut lines, ot_core::OFFER_PREFIX).await;
    ot(admin, &["admin", "service", "add", service, "--qr", &qr]).await;
    finished(child).await;
    home
}
