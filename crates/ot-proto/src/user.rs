//! Personnes et profils d'appareils (étape 10) : identifiants, rôles, objets
//! signés opaques pour le relais, et contenu des trames d'annuaire.
//!
//! Le relais range les objets signés (certificats, entrées du journal de
//! roster) sans les lire ni les vérifier : ce sont les clients qui les
//! vérifient, jusqu'au certificat de l'admin.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{DeviceId, b64};

/// Identifiant de personne : `usr_` suivi d'un ULID.
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
        // Invariant : un `UserId` n'est construit que d'un ULID valide.
        Ulid::from_string(&self.0[Self::PREFIX.len()..]).expect("UserId toujours valide")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for UserId {
    type Err = InvalidUserId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Forme canonique seulement, comme `DeviceId`.
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

/// Rôle d'une personne. Un seul `admin` actif par relais.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
    /// Compte de service (diagnostic, pont de codes…) : sans groupe, il
    /// n'écrit qu'aux personnes vers qui l'admin l'a lié, et ne reçoit rien.
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

/// État d'une personne : `pending` tant qu'elle est invitée sans certificat.
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

/// Catégorie d'un appareil, déclarée par le client ou rangée par sa
/// personne. Les clients devinent au nom quand elle manque (note 0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Laptop,
    Desktop,
    Browser,
    /// Ligne de commande : jamais joignable ni par défaut.
    Cli,
    /// Catégorie ajoutée par une version plus récente : traitée comme
    /// `laptop` par les clients, jamais enregistrée par le relais.
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

/// Capacité annoncée par un client : `[a-z0-9-]{1,32}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Feature(String);

/// Client qui sait parler aux autres personnes (annuaire, schéma `0x05`).
pub const FEATURE_USERS_V1: &str = "users-v1";

/// Nombre maximal de capacités par appareil.
pub const MAX_FEATURES: usize = 16;

#[derive(Debug, Clone, thiserror::Error)]
#[error("capacité invalide : {0:?}")]
pub struct InvalidFeature(pub String);

impl Feature {
    pub fn users_v1() -> Self {
        Self(FEATURE_USERS_V1.to_owned())
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

/// Objet signé, opaque pour le relais : `data` est le JSON exact signé,
/// `sig` la signature Ed25519 de `domaine ‖ data` (format dans `ot-crypto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedBlob {
    #[serde(with = "b64::bytes")]
    pub data: Vec<u8>,
    #[serde(with = "b64::array")]
    pub sig: [u8; 64],
}

/// Taille maximale d'un objet signé (certificat, entrée de journal), `data`
/// et signature compris, avant base64.
pub const MAX_SIGNED_BYTES: usize = 16 * 1024;

impl SignedBlob {
    /// Taille brute, comparée à [`MAX_SIGNED_BYTES`].
    pub fn len(&self) -> usize {
        self.data.len() + self.sig.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Entrée du journal de roster d'une personne, telle que le relais la rend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogEntry {
    pub seq: u64,
    pub entry: SignedBlob,
}

/// Une personne de l'annuaire (`directory`), filtrée par le réseau.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryUser {
    pub id: UserId,
    pub name: String,
    pub role: Role,
    pub state: UserState,
    /// Certificat signé par l'admin ; absent tant que la personne n'est pas
    /// certifiée (ou, pour l'admin, avant `admin_init`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<SignedBlob>,
    /// Entrées du journal après celles que le client connaît (`known`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<LogEntry>,
    /// D'autres entrées suivent : relire avec `known` à jour (une trame ne
    /// dépasse jamais `MAX_FRAME_BYTES`). Les autres personnes ne sont pas
    /// retardées pour autant : elles suivent par `next`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
    /// Entre un service et cette personne : la personne a coupé le service
    /// (vu par elle comme par le service).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Profils non signés : tous mes appareils, seulement les joignables
    /// d'une autre personne.
    #[serde(default)]
    pub devices: Vec<DirectoryDevice>,
}

/// Profil d'appareil tenu par le relais, non signé : les clients ne le
/// gardent que pour un appareil du roster vérifié de sa personne.
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

/// Une personne vue par l'admin (`users`), avec tous ses appareils actifs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminUser {
    pub id: UserId,
    pub name: String,
    pub role: Role,
    pub state: UserState,
    /// Millisecondes Unix.
    pub created_at: i64,
    pub devices: Vec<AdminDevice>,
    /// Personnes vers qui elle peut écrire.
    #[serde(default)]
    pub links: Vec<UserId>,
    /// Services qu'elle a coupés.
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
    /// Dernière fin de session, en millisecondes Unix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
}

/// Dernier `seq` vérifié par le client, par personne (`directory_get`).
pub type Known = BTreeMap<UserId, u64>;

/// Nom de personne accepté par le relais : mêmes règles qu'un nom
/// d'appareil.
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
