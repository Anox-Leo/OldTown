use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{DeviceId, b64};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UserId(String);

#[derive(Debug, Clone, thiserror::Error)]
#[error("identifiant de personne invalide : {0:?}")]
pub struct InvalidUserId(pub String);

impl UserId {
    const PREFIX: &'static str = "usr_";

    pub fn generate() -> Self {
        Self(format!("{}{}", Self::PREFIX, Ulid::generate()))
    }

    pub fn from_ulid(ulid: Ulid) -> Self {
        Self(format!("{}{ulid}", Self::PREFIX))
    }

    pub fn ulid(&self) -> Ulid {
        Ulid::from_string(&self.0[Self::PREFIX.len()..]).expect("UserId toujours valide")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for UserId {
    type Err = InvalidUserId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.strip_prefix(Self::PREFIX).map(|u| (u, Ulid::from_string(u))) {
            Some((u, Ok(ulid))) if ulid.to_string() == u => Ok(Self(s.to_owned())),
            _ => Err(InvalidUserId(s.to_owned())),
        }
    }
}

impl TryFrom<String> for UserId {
    type Error = InvalidUserId;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<UserId> for String {
    fn from(id: UserId) -> Self {
        id.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
    Service,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Service => "service",
        }
    }
}

impl FromStr for Role {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            "service" => Ok(Self::Service),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserState {
    Pending,
    Active,
    Revoked,
}

impl UserState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }
}

impl FromStr for UserState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Laptop,
    Desktop,
    Browser,
    Cli,
    #[serde(other)]
    Unknown,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Laptop => "laptop",
            Self::Desktop => "desktop",
            Self::Browser => "browser",
            Self::Cli => "cli",
            Self::Unknown => "unknown",
        }
    }
}

impl FromStr for DeviceKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "phone" => Ok(Self::Phone),
            "laptop" => Ok(Self::Laptop),
            "desktop" => Ok(Self::Desktop),
            "browser" => Ok(Self::Browser),
            "cli" => Ok(Self::Cli),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Feature(String);

pub const FEATURE_USERS_V1: &str = "users-v1";

pub const FEATURE_READ_V1: &str = "read-v1";

pub const MAX_FEATURES: usize = 16;

#[derive(Debug, Clone, thiserror::Error)]
#[error("capacité invalide : {0:?}")]
pub struct InvalidFeature(pub String);

impl Feature {
    pub fn users_v1() -> Self {
        Self(FEATURE_USERS_V1.to_owned())
    }

    pub fn read_v1() -> Self {
        Self(FEATURE_READ_V1.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Feature {
    type Err = InvalidFeature;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = (1..=32).contains(&s.len())
            && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok { Ok(Self(s.to_owned())) } else { Err(InvalidFeature(s.to_owned())) }
    }
}

impl TryFrom<String> for Feature {
    type Error = InvalidFeature;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Feature> for String {
    fn from(f: Feature) -> Self {
        f.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedBlob {
    #[serde(with = "b64::bytes")]
    pub data: Vec<u8>,
    #[serde(with = "b64::array")]
    pub sig: [u8; 64],
}

pub const MAX_SIGNED_BYTES: usize = 16 * 1024;

impl SignedBlob {
    pub fn len(&self) -> usize {
        self.data.len() + self.sig.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    pub seq: u64,
    pub entry: SignedBlob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryUser {
    pub id: UserId,
    pub name: String,
    pub role: Role,
    pub state: UserState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<SignedBlob>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<LogEntry>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    #[serde(default)]
    pub devices: Vec<DirectoryDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryDevice {
    pub id: DeviceId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DeviceKind>,
    pub reachable: bool,
    pub default: bool,
    #[serde(default)]
    pub features: Vec<Feature>,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminUser {
    pub id: UserId,
    pub name: String,
    pub role: Role,
    pub state: UserState,
    pub created_at: i64,
    pub devices: Vec<AdminDevice>,
    #[serde(default)]
    pub links: Vec<UserId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub muted: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminDevice {
    pub id: DeviceId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DeviceKind>,
    pub reachable: bool,
    pub default: bool,
    #[serde(default)]
    pub features: Vec<Feature>,
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
}

pub type Known = BTreeMap<UserId, u64>;

pub fn valid_user_name(name: &str) -> bool {
    crate::valid_device_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids() {
        let id = UserId::generate();
        assert_eq!(id.as_str().parse::<UserId>().unwrap(), id);
        assert_eq!(UserId::from_ulid(id.ulid()), id);
        assert!("dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5".parse::<UserId>().is_err());
        assert!("usr_pas-un-ulid".parse::<UserId>().is_err());
        assert!("usr_01j9z4a1b2c3d4e5f6g7h8j9k0".parse::<UserId>().is_err(), "minuscules");
    }

    #[test]
    fn features() {
        assert!("users-v1".parse::<Feature>().is_ok());
        assert_eq!(Feature::read_v1().as_str(), "read-v1");
        assert!("".parse::<Feature>().is_err());
        assert!("Users".parse::<Feature>().is_err());
        assert!("a b".parse::<Feature>().is_err());
        assert!("x".repeat(33).parse::<Feature>().is_err());
    }

    #[test]
    fn unknown_kind_is_read_as_unknown() {
        let kind: DeviceKind = serde_json::from_str("\"tablet\"").unwrap();
        assert_eq!(kind, DeviceKind::Unknown);
        assert_eq!("tablet".parse::<DeviceKind>(), Err(()));
    }
}
