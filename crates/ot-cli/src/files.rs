//! Fichiers locaux de la CLI : configuration, clés et état du groupe.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ot_core::{DeviceId, Identity, KeyringError, SecretStore, StoreKeyring, Ulid};
use ot_proto::b64;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const CONFIG: &str = "config.toml";
const KEY: &str = "device.key";
const SEEN: &str = "seen.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server_url: String,
    pub device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<DeviceId>,
}

pub struct Home(PathBuf);

impl Home {
    /// Dossier donné, sinon `OT_HOME`, sinon le dossier de configuration de l'OS.
    pub fn locate(explicit: Option<PathBuf>) -> Result<Self> {
        if let Some(dir) = explicit {
            return Ok(Self(dir));
        }
        let dirs = directories::ProjectDirs::from("", "", "oldtown")
            .context("dossier personnel introuvable ; utiliser --home")?;
        Ok(Self(dirs.config_dir().to_owned()))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn load_config(&self) -> Result<Option<Config>> {
        let path = self.0.join(CONFIG);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(
                toml::from_str(&text).with_context(|| format!("lecture de {}", path.display()))?,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("lecture de {}", path.display())),
        }
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        fs::create_dir_all(&self.0)?;
        write_atomic(&self.0.join(CONFIG), toml::to_string(config)?.as_bytes(), false)
    }

    pub fn has_key(&self) -> bool {
        self.0.join(KEY).exists()
    }

    pub fn load_key(&self) -> Result<Identity> {
        let path = self.0.join(KEY);
        let text = Zeroizing::new(
            fs::read_to_string(&path).with_context(|| format!("lecture de {}", path.display()))?,
        );
        let Some(seed) = b64::decode_array::<32>(text.trim()).map(Zeroizing::new) else {
            bail!("{} est corrompu", path.display());
        };
        Ok(Identity::from_bytes(&seed))
    }

    pub fn save_key(&self, identity: &Identity) -> Result<()> {
        fs::create_dir_all(&self.0)?;
        let seed = Zeroizing::new(identity.to_bytes());
        let text = Zeroizing::new(format!("{}\n", b64::encode(&*seed)));
        write_atomic(&self.0.join(KEY), text.as_bytes(), true)
    }

    /// Derniers `msg_id` vus ; un fichier absent ou illisible donne une liste vide.
    pub fn load_seen(&self) -> Vec<Ulid> {
        let Ok(text) = fs::read_to_string(self.0.join(SEEN)) else {
            return Vec::new();
        };
        let ids: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
        ids.iter().filter_map(|s| Ulid::from_string(s).ok()).collect()
    }

    pub fn save_seen<'a>(&self, seen: impl Iterator<Item = &'a Ulid>) -> Result<()> {
        let ids: Vec<String> = seen.map(Ulid::to_string).collect();
        write_atomic(&self.0.join(SEEN), serde_json::to_string(&ids)?.as_bytes(), false)
    }

    /// Clés et état du groupe, dans le même dossier.
    pub fn keyring(&self) -> StoreKeyring {
        StoreKeyring::new(Arc::new(FileStore(self.0.clone())))
    }

    /// Oublie le groupe et la clé d'échange (nouvelle identité d'appareil).
    pub fn reset_group(&self) -> Result<()> {
        Ok(self.keyring().forget_group()?)
    }
}

/// [`SecretStore`] sur fichiers : un fichier par nom, en `0600` (rien n'y
/// est en clair, sauf les clés, mais rien n'oblige à montrer le reste).
pub struct FileStore(PathBuf);

impl FileStore {
    fn path(&self, name: &str) -> Result<PathBuf, KeyringError> {
        let valid = !name.is_empty()
            && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !valid {
            return Err(KeyringError(format!("nom de secret invalide : {name:?}")));
        }
        Ok(self.0.join(name))
    }
}

impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeyringError> {
        let path = self.path(name)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeyringError(format!("lecture de {} : {e}", path.display()))),
        }
    }

    fn put(&self, name: &str, value: &[u8]) -> Result<(), KeyringError> {
        let path = self.path(name)?;
        fs::create_dir_all(&self.0)
            .map_err(anyhow::Error::from)
            .and_then(|()| write_atomic(&path, value, true))
            .map_err(|e| KeyringError(format!("écriture de {name} : {e:#}")))
    }

    fn delete(&self, name: &str) -> Result<(), KeyringError> {
        let path = self.path(name)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(KeyringError(format!("suppression de {} : {e}", path.display()))),
        }
    }
}

/// Écrit dans un fichier temporaire puis renomme, pour ne jamais laisser de
/// fichier à moitié écrit. `private` : lisible par le seul propriétaire.
fn write_atomic(path: &Path, data: &[u8], private: bool) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = opts.open(&tmp).with_context(|| format!("écriture de {}", tmp.display()))?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path).with_context(|| format!("écriture de {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_core::Keyring;
    use ot_core::names::GROUP;
    #[cfg(unix)]
    use ot_core::names::{EXCHANGE_KEY, QUARANTINE, ROSTER};

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home(dir.path().join("ot"));
        assert!(home.load_config().unwrap().is_none());
        assert!(!home.has_key());

        let id = Identity::generate();
        home.save_key(&id).unwrap();
        assert_eq!(home.load_key().unwrap().public_key(), id.public_key());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(home.path().join(KEY)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let config = Config {
            server_url: "ws://relais:7878".into(),
            device_name: "mac".into(),
            device_id: Some(DeviceId::generate()),
        };
        home.save_config(&config).unwrap();
        assert_eq!(home.load_config().unwrap().unwrap().device_id, config.device_id);

        let seen = [Ulid::from_parts(1, 2), Ulid::from_parts(3, 4)];
        home.save_seen(seen.iter()).unwrap();
        assert_eq!(home.load_seen(), seen);
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn keyring_files_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home(dir.path().join("ot"));
        let keyring = home.keyring();
        assert!(keyring.load_exchange_key().unwrap().is_none());
        assert!(keyring.load_group_keys().unwrap().is_none());
        assert!(keyring.load_quarantine().unwrap().is_empty());

        let id = DeviceId::generate();
        let identity = Identity::generate();
        let info = ot_core::init_group(&keyring, &id, "mac", &identity, false).unwrap();
        assert_eq!(info.members.len(), 1);
        let exchange = keyring.load_exchange_key().unwrap().unwrap();
        assert_eq!(
            exchange.public_key(),
            keyring.load_roster().unwrap().unwrap().members[0].exchange_key
        );
        assert_eq!(keyring.load_group_keys().unwrap().unwrap().current, 1);

        #[cfg(unix)]
        for name in [EXCHANGE_KEY, GROUP, ROSTER, QUARANTINE] {
            assert_eq!(mode(&home.path().join(name)), 0o600, "{name}");
        }

        // `group init` refuse d'écraser un groupe existant, sauf --force.
        assert!(ot_core::init_group(&keyring, &id, "mac", &identity, false).is_err());
        assert!(ot_core::init_group(&keyring, &id, "mac", &identity, true).is_ok());

        home.reset_group().unwrap();
        assert!(keyring.load_group_keys().unwrap().is_none());
        assert!(keyring.load_exchange_key().unwrap().is_none());
    }

    #[test]
    fn corrupted_group_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home(dir.path().to_owned());
        fs::write(dir.path().join(GROUP), "pas du json").unwrap();
        let err = home.keyring().load_group_keys().unwrap_err();
        assert!(err.0.contains("corrompu"), "{err}");
    }

    #[test]
    fn store_refuses_path_like_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore(dir.path().to_owned());
        for bad in ["", "../x", "a/b", ".cache", "é"] {
            assert!(store.put(bad, b"x").is_err(), "{bad:?}");
        }
        store.put("seen.json", b"[]").unwrap();
        assert_eq!(&store.get("seen.json").unwrap().unwrap()[..], b"[]");
        store.delete("seen.json").unwrap();
        store.delete("seen.json").unwrap();
        assert!(store.get("seen.json").unwrap().is_none());
    }
}
