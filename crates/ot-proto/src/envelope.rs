use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use ulid::Ulid;

use crate::b64;

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

    pub fn ulid(&self) -> Ulid {
        Ulid::from_string(&self.0[Self::PREFIX.len()..]).expect("DeviceId toujours valide")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DeviceId {
    type Err = InvalidDeviceId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix(Self::PREFIX).map(|u| (u, Ulid::from_string(u))) {
            Some((u, Ok(ulid))) if ulid.to_string() == u => Ok(Self(s.to_owned())),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub msg_id: Ulid,
    pub from: DeviceId,
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
    None,
    Xchacha20poly1305,
}

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
    pub fn plain(content: &Content) -> Self {
        Self {
            enc: Encoding::None,
            data: serde_json::to_vec(content).expect("Content est toujours sérialisable"),
        }
    }

    pub fn plain_content(&self) -> Option<Content> {
        match self.enc {
            Encoding::None => serde_json::from_slice(&self.data).ok(),
            Encoding::Xchacha20poly1305 => None,
        }
    }
}

impl Envelope {
    pub fn new_plain(from: DeviceId, to: Vec<DeviceId>, content: &Content) -> Self {
        Self::new(from, to, Payload::plain(content))
    }

    pub fn new(from: DeviceId, to: Vec<DeviceId>, payload: Payload) -> Self {
        Self {
            msg_id: Ulid::generate(),
            from,
            to,
            created_at: OffsetDateTime::now_utc().replace_nanosecond(0).expect("0 est valide"),
            payload,
        }
    }

    pub fn encoded_len(&self) -> usize {
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(usize::MAX)
    }
}
