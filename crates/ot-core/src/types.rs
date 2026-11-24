use ot_crypto::Otp;
use ot_proto::{Content, DeviceId, ErrorCode, Ulid, UserId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::files::ReceivedFile;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Origin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_user: Option<UserId>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_service: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_user: Option<UserId>,
}

impl Origin {
    pub fn is_mine(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    MyDevices(Vec<DeviceId>),
    Person { user: UserId, device: Option<DeviceId> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminRoot {
    Ready(UserId),
    Missing { name: String },
    NotAdmin,
}

#[derive(Debug, Clone)]
pub struct Received {
    pub seq: i64,
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub to: Vec<DeviceId>,
    pub created_at: OffsetDateTime,
    pub content: Content,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub struct ReceivedOtp {
    pub seq: i64,
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub to: Vec<DeviceId>,
    pub created_at: OffsetDateTime,
    pub otp: Otp,
    pub origin: Origin,
}

impl ReceivedOtp {
    pub fn auto_fill(&self) -> bool {
        self.origin.from_user.is_none() || self.origin.from_service
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Connected { pending: u64 },
    Disconnected { reason: String },
    AuthFailed { message: String },
    Message(Received),
    File(ReceivedFile),
    Otp(ReceivedOtp),
    Rejected { msg_id: Ulid, from: DeviceId, reason: String },
    GroupChanged { epoch: u32 },
    PairingCompleted { device_id: DeviceId, name: String },
    KeyringError { message: String },
    DirectoryChanged,
    DirectoryRejected { user: UserId, reason: String },
    DirectoryFork,
    KeysChanged { user: UserId },
    PersonJoined { user: UserId, name: String, device_name: String, fingerprint: String },
    Joined { user: UserId },
    InviteFailed { reason: String },
}

#[derive(Debug, Clone)]
pub struct PairingInvitation {
    pub code: String,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub msg_id: Ulid,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("refusé par le serveur ({code:?}) : {message}")]
    Rejected { code: ErrorCode, message: String },
    #[error("client arrêté")]
    Closed,
    #[error(
        "cet appareil n'a pas de groupe : `ot group init` sur le premier appareil, `ot pair join` sur les autres"
    )]
    NoGroup,
    #[error("{0} n'est pas membre du groupe")]
    NotMember(DeviceId),
    #[error("aucun destinataire : {0}")]
    NoRecipient(String),
    #[error("chiffrement : {0}")]
    Crypto(String),
    #[error("appairage : {0}")]
    Pairing(String),
    #[error("connexion : {0}")]
    Connection(String),
    #[error("{0}")]
    Invalid(String),
    #[error("fichier expiré : il n'est plus sur le relais")]
    Expired,
    #[error("transfert : {0}")]
    Transfer(String),
}
