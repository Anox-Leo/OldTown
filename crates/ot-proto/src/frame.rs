use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ulid::Ulid;

use time::OffsetDateTime;

use crate::{
    AdminUser, DeviceId, DeviceKind, DirectoryUser, Envelope, Feature, Known, NONCE_LEN,
    PAIR_TOKEN_LEN, PROTOCOL_VERSION, Role, SignedBlob, UserId, b64,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Challenge {
        #[serde(with = "b64::array")]
        nonce: [u8; NONCE_LEN],
    },
    Auth {
        device_id: DeviceId,
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    Welcome {
        device_id: DeviceId,
        pending: u64,
    },
    Send {
        env: Envelope,
    },
    Accepted {
        msg_id: Ulid,
        seq: i64,
    },
    Deliver {
        seq: i64,
        env: Envelope,
    },
    Ack {
        seq: i64,
    },
    ListDevices,
    Devices {
        devices: Vec<DeviceInfo>,
    },
    PairCreate,
    PairCreated {
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        #[serde(with = "time::serde::rfc3339")]
        expires_at: OffsetDateTime,
    },
    PairRedeem {
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        name: String,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    PairRedeemed {
        device_id: DeviceId,
        sponsor: DeviceId,
    },
    PairRegister {
        device_id: DeviceId,
        name: String,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    PairRegistered {
        device_id: DeviceId,
    },
    DirectoryGet {
        #[serde(default, skip_serializing_if = "Known::is_empty")]
        known: Known,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<UserId>,
    },
    Directory {
        me: UserId,
        admin: UserId,
        users: Vec<DirectoryUser>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<UserId>,
    },
    DirectoryChanged,
    RosterPublish {
        user: UserId,
        seq: u64,
        entry: SignedBlob,
    },
    RosterPublished {
        user: UserId,
        seq: u64,
    },
    DeviceUpdate {
        device: DeviceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<DeviceKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reachable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        features: Option<Vec<Feature>>,
    },
    DeviceUpdated {
        device: DeviceId,
    },
    DeviceRevoke {
        device: DeviceId,
    },
    DeviceRevoked {
        device: DeviceId,
    },
    ServiceMute {
        service: UserId,
        muted: bool,
    },
    ServiceMuted {
        service: UserId,
        muted: bool,
    },
    AdminInit {
        cert: SignedBlob,
        entry: SignedBlob,
    },
    AdminInitialized {
        user: UserId,
    },
    UserInvite {
        name: String,
        role: Role,
        #[serde(default)]
        links: Vec<UserId>,
    },
    UserReinvite {
        user: UserId,
    },
    UserInvited {
        user: UserId,
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        #[serde(with = "time::serde::rfc3339")]
        expires_at: OffsetDateTime,
    },
    UserCertify {
        user: UserId,
        cert: SignedBlob,
    },
    UserCertified {
        user: UserId,
    },
    ServiceCreate {
        user: UserId,
        name: String,
        device: DeviceId,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        device_name: String,
        #[serde(with = "b64::array")]
        sig: [u8; 64],
        cert: SignedBlob,
    },
    ServiceDevice {
        user: UserId,
        device: DeviceId,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        name: String,
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    UserRevoke {
        user: UserId,
    },
    UserRevoked {
        user: UserId,
    },
    UserRename {
        user: UserId,
        name: String,
    },
    UserRenamed {
        user: UserId,
    },
    LinkSet {
        from: UserId,
        to: UserId,
        allowed: bool,
    },
    LinkUpdated {
        from: UserId,
        to: UserId,
        allowed: bool,
    },
    UsersGet,
    Users {
        users: Vec<AdminUser>,
    },
    InviteRedeem {
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        name: String,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    InviteRedeemed {
        user: UserId,
        device: DeviceId,
        inviter: DeviceId,
    },
    Ping,
    Pong,
    Error {
        code: ErrorCode,
        message: String,
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadFrame,
    UnsupportedVersion,
    UnknownType,
    UnexpectedFrame,
    AuthFailed,
    EnvelopeTooLarge,
    SenderMismatch,
    UnknownRecipient,
    MsgIdConflict,
    PairingInvalid,
    NameTaken,
    Internal,
    Forbidden,
    NotAllowed,
    QuotaExceeded,
    UserRevoked,
    Conflict,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("trame invalide : {0}")]
    BadFrame(String),
    #[error("version de protocole non prise en charge : {0}")]
    UnsupportedVersion(u64),
    #[error("type de trame inconnu : {0:?}")]
    UnknownType(String),
}

impl DecodeError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadFrame(_) => ErrorCode::BadFrame,
            Self::UnsupportedVersion(_) => ErrorCode::UnsupportedVersion,
            Self::UnknownType(_) => ErrorCode::UnknownType,
        }
    }
}

const KNOWN_TYPES: &[&str] = &[
    "challenge",
    "auth",
    "welcome",
    "send",
    "accepted",
    "deliver",
    "ack",
    "list_devices",
    "devices",
    "pair_create",
    "pair_created",
    "pair_redeem",
    "pair_redeemed",
    "pair_register",
    "pair_registered",
    "directory_get",
    "directory",
    "directory_changed",
    "roster_publish",
    "roster_published",
    "device_update",
    "device_updated",
    "device_revoke",
    "device_revoked",
    "service_mute",
    "service_muted",
    "admin_init",
    "admin_initialized",
    "user_invite",
    "user_reinvite",
    "user_invited",
    "user_certify",
    "user_certified",
    "service_create",
    "service_device",
    "user_revoke",
    "user_revoked",
    "user_rename",
    "user_renamed",
    "link_set",
    "link_updated",
    "users_get",
    "users",
    "invite_redeem",
    "invite_redeemed",
    "ping",
    "pong",
    "error",
];

impl Frame {
    pub fn error(code: ErrorCode, message: impl Into<String>, reference: Option<String>) -> Self {
        Self::Error { code, message: message.into(), reference }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Challenge { .. } => "challenge",
            Self::Auth { .. } => "auth",
            Self::Welcome { .. } => "welcome",
            Self::Send { .. } => "send",
            Self::Accepted { .. } => "accepted",
            Self::Deliver { .. } => "deliver",
            Self::Ack { .. } => "ack",
            Self::ListDevices => "list_devices",
            Self::Devices { .. } => "devices",
            Self::PairCreate => "pair_create",
            Self::PairCreated { .. } => "pair_created",
            Self::PairRedeem { .. } => "pair_redeem",
            Self::PairRedeemed { .. } => "pair_redeemed",
            Self::PairRegister { .. } => "pair_register",
            Self::PairRegistered { .. } => "pair_registered",
            Self::DirectoryGet { .. } => "directory_get",
            Self::Directory { .. } => "directory",
            Self::DirectoryChanged => "directory_changed",
            Self::RosterPublish { .. } => "roster_publish",
            Self::RosterPublished { .. } => "roster_published",
            Self::DeviceUpdate { .. } => "device_update",
            Self::DeviceUpdated { .. } => "device_updated",
            Self::DeviceRevoke { .. } => "device_revoke",
            Self::DeviceRevoked { .. } => "device_revoked",
            Self::ServiceMute { .. } => "service_mute",
            Self::ServiceMuted { .. } => "service_muted",
            Self::AdminInit { .. } => "admin_init",
            Self::AdminInitialized { .. } => "admin_initialized",
            Self::UserInvite { .. } => "user_invite",
            Self::UserReinvite { .. } => "user_reinvite",
            Self::UserInvited { .. } => "user_invited",
            Self::UserCertify { .. } => "user_certify",
            Self::UserCertified { .. } => "user_certified",
            Self::ServiceCreate { .. } => "service_create",
            Self::ServiceDevice { .. } => "service_device",
            Self::UserRevoke { .. } => "user_revoke",
            Self::UserRevoked { .. } => "user_revoked",
            Self::UserRename { .. } => "user_rename",
            Self::UserRenamed { .. } => "user_renamed",
            Self::LinkSet { .. } => "link_set",
            Self::LinkUpdated { .. } => "link_updated",
            Self::UsersGet => "users_get",
            Self::Users { .. } => "users",
            Self::InviteRedeem { .. } => "invite_redeem",
            Self::InviteRedeemed { .. } => "invite_redeemed",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::Error { .. } => "error",
        }
    }

    pub fn to_json(&self) -> String {
        let Value::Object(fields) = serde_json::to_value(self).expect("Frame est sérialisable")
        else {
            unreachable!("une trame est toujours un objet JSON")
        };
        let mut obj = Map::with_capacity(fields.len() + 1);
        obj.insert("v".into(), Value::from(PROTOCOL_VERSION));
        obj.extend(fields);
        Value::Object(obj).to_string()
    }

    pub fn from_json(text: &str) -> Result<Self, DecodeError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| DecodeError::BadFrame(e.to_string()))?;
        let Value::Object(mut obj) = value else {
            return Err(DecodeError::BadFrame("objet JSON attendu".into()));
        };
        match obj.remove("v") {
            Some(Value::Number(n)) if n.as_u64() == Some(PROTOCOL_VERSION.into()) => {}
            Some(Value::Number(n)) => {
                return Err(DecodeError::UnsupportedVersion(n.as_u64().unwrap_or(0)));
            }
            _ => return Err(DecodeError::BadFrame("champ \"v\" manquant ou invalide".into())),
        }
        match obj.get("t") {
            Some(Value::String(t)) if KNOWN_TYPES.contains(&t.as_str()) => {}
            Some(Value::String(t)) => return Err(DecodeError::UnknownType(t.clone())),
            _ => return Err(DecodeError::BadFrame("champ \"t\" manquant ou invalide".into())),
        }
        serde_json::from_value(Value::Object(obj)).map_err(|e| DecodeError::BadFrame(e.to_string()))
    }
}
