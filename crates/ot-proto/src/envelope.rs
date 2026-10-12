use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use ulid::Ulid;

use crate::b64;

/// Identifiant d'appareil : `dev_` suivi d'un ULID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceId(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("identifiant d'appareil invalide : {0:?}")]
pub struct InvalidDeviceId(pub String);

impl DeviceId {
    const PREFIX: &'static str = "dev_";

    pub fn generate() -> Self {
        Self(format!("{}{}", Self::PREFIX, Ulid::generate()))
    }

    pub fn from_ulid(ulid: Ulid) -> Self {
        Self(format!("{}{ulid}", Self::PREFIX))
    }

    /// ULID de l'identifiant (forme binaire compacte, 16 octets).
    pub fn ulid(&self) -> Ulid {
        // Invariant : un `DeviceId` n'est construit que d'un ULID valide.
        Ulid::from_string(&self.0[Self::PREFIX.len()..]).expect("DeviceId toujours valide")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DeviceId {
    type Err = InvalidDeviceId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix(Self::PREFIX).map(Ulid::from_string) {
            Some(Ok(_)) => Ok(Self(s.to_owned())),
            _ => Err(InvalidDeviceId(s.to_owned())),
        }
    }
}

impl TryFrom<String> for DeviceId {
    type Error = InvalidDeviceId;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<DeviceId> for String {
    fn from(id: DeviceId) -> Self {
        id.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Enveloppe d'un message. `payload` est chiffré par les clients (format
/// défini par `ot-crypto`) ; le serveur ne lit que les champs de routage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// ULID généré par l'expéditeur, sert à l'idempotence et au dédoublonnage.
    pub msg_id: Ulid,
    pub from: DeviceId,
    /// Vide : tous les autres appareils de l'expéditeur.
    #[serde(default)]
    pub to: Vec<DeviceId>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub payload: Payload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub enc: Encoding,
    #[serde(with = "b64::bytes")]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Contenu en clair (étape 1).
    None,
    /// Contenu chiffré sous la clé de groupe (étape 2).
    Xchacha20poly1305,
}

/// Contenu applicatif, sérialisé dans `payload.data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub kind: ContentKind,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Text,
    Link,
}

impl Content {
    pub fn text(body: impl Into<String>) -> Self {
        Self { kind: ContentKind::Text, body: body.into() }
    }

    pub fn link(body: impl Into<String>) -> Self {
        Self { kind: ContentKind::Link, body: body.into() }
    }
}

impl Payload {
    /// Payload en clair contenant `content` sérialisé en JSON.
    pub fn plain(content: &Content) -> Self {
        Self {
            enc: Encoding::None,
            data: serde_json::to_vec(content).expect("Content est toujours sérialisable"),
        }
    }

    /// Relit le contenu d'un payload en clair ; `None` s'il est chiffré ou invalide.
    pub fn plain_content(&self) -> Option<Content> {
        match self.enc {
            Encoding::None => serde_json::from_slice(&self.data).ok(),
            Encoding::Xchacha20poly1305 => None,
        }
    }
}

impl Envelope {
    /// Nouvelle enveloppe en clair, horodatée maintenant, avec un `msg_id` neuf.
    /// Depuis l'étape 2, les clients refusent le clair : ne sert qu'aux tests.
    pub fn new_plain(from: DeviceId, to: Vec<DeviceId>, content: &Content) -> Self {
        Self::new(from, to, Payload::plain(content))
    }

    /// Nouvelle enveloppe horodatée maintenant (à la seconde), avec un
    /// `msg_id` neuf.
    pub fn new(from: DeviceId, to: Vec<DeviceId>, payload: Payload) -> Self {
        Self {
            msg_id: Ulid::generate(),
            from,
            to,
            created_at: OffsetDateTime::now_utc().replace_nanosecond(0).expect("0 est valide"),
            payload,
        }
    }

    /// Taille de l'enveloppe sérialisée, comparée à `MAX_ENVELOPE_BYTES`.
    pub fn encoded_len(&self) -> usize {
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(usize::MAX)
    }
}
