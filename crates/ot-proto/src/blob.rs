use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Identifiant d'un fichier déposé sur le relais (étape 5) : `blob_` suivi
/// d'un ULID, tiré par le relais à la création.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BlobId(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("identifiant de fichier invalide : {0:?}")]
pub struct InvalidBlobId(pub String);

impl BlobId {
    const PREFIX: &'static str = "blob_";

    pub fn generate() -> Self {
        Self(format!("{}{}", Self::PREFIX, Ulid::generate()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BlobId {
    type Err = InvalidBlobId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix(Self::PREFIX).map(Ulid::from_string) {
            Some(Ok(_)) => Ok(Self(s.to_owned())),
            _ => Err(InvalidBlobId(s.to_owned())),
        }
    }
}

impl TryFrom<String> for BlobId {
    type Error = InvalidBlobId;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<BlobId> for String {
    fn from(id: BlobId) -> Self {
        id.0
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        let id = BlobId::generate();
        assert_eq!(id.as_str().parse::<BlobId>().unwrap(), id);
        assert!(id.to_string().starts_with("blob_"));
        // Seul le préfixe `blob_` est accepté : un identifiant d'appareil non.
        assert!("dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5".parse::<BlobId>().is_err());
        assert!("blob_pas-un-ulid".parse::<BlobId>().is_err());
        // Un identifiant sert de nom de dossier sur le relais : jamais de `/`.
        assert!("blob_../../etc".parse::<BlobId>().is_err());
    }
}
