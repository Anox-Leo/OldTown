//! Fichiers locaux de la CLI : configuration, clé privée et état.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ot_core::{DeviceId, Identity, Ulid};
use ot_proto::b64;
use serde::{Deserialize, Serialize};

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
        let text =
            fs::read_to_string(&path).with_context(|| format!("lecture de {}", path.display()))?;
        let Some(seed) = b64::decode_array::<32>(text.trim()) else {
            bail!("{} est corrompu", path.display());
        };
        Ok(Identity::from_bytes(&seed))
    }

    pub fn save_key(&self, identity: &Identity) -> Result<()> {
        fs::create_dir_all(&self.0)?;
        let text = format!("{}\n", b64::encode(&identity.to_bytes()));
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
}
