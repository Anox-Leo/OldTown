use ot_core::{ClientError, ErrorCode, GroupInfo, KeyringError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum OtError {
    #[error("{message}")]
    Offline { message: String },
    #[error("{message}")]
    AuthFailed { message: String },
    #[error("{message}")]
    NotInGroup { message: String },
    #[error("{message}")]
    Pairing { message: String },
    #[error("{message}")]
    Invalid { message: String },
    #[error("{message}")]
    Expired { message: String },
    #[error("{message}")]
    Transfer { message: String },
    #[error("{message}")]
    Internal { message: String },
}

impl From<ClientError> for OtError {
    fn from(e: ClientError) -> Self {
        let message = e.to_string();
        match e {
            ClientError::Connection(_) | ClientError::Closed => Self::Offline { message },
            ClientError::Rejected { code: ErrorCode::AuthFailed, .. } => {
                Self::AuthFailed { message }
            }
            ClientError::Rejected {
                code: ErrorCode::PairingInvalid | ErrorCode::NameTaken,
                ..
            }
            | ClientError::Pairing(_) => Self::Pairing { message },
            ClientError::NoGroup | ClientError::NotMember(_) => Self::NotInGroup { message },
            ClientError::Invalid(_) | ClientError::NoRecipient(_) => Self::Invalid { message },
            ClientError::Expired => Self::Expired { message },
            ClientError::Transfer(_) => Self::Transfer { message },
            ClientError::Rejected { .. } | ClientError::Crypto(_) => Self::Internal { message },
        }
    }
}

impl From<KeyringError> for OtError {
    fn from(e: KeyringError) -> Self {
        Self::Internal { message: e.to_string() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum StoreError {
    #[error("{message}")]
    Failed { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for StoreError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed { message: e.reason }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct DeviceState {
    pub device_id: String,
    pub name: String,
    pub server_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Link,
    File,
}

impl From<ot_core::ContentKind> for ContentKind {
    fn from(k: ot_core::ContentKind) -> Self {
        match k {
            ot_core::ContentKind::Text => Self::Text,
            ot_core::ContentKind::Link => Self::Link,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct Received {
    pub msg_id: String,
    pub from_id: String,
    pub from_name: String,
    pub created_at_ms: i64,
    pub kind: ContentKind,
    pub body: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub file: Option<crate::FileItem>,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub outgoing: bool,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    #[uniffi(default = None)]
    pub from_user: Option<String>,
    #[serde(default)]
    #[uniffi(default = false)]
    pub from_service: bool,
    #[serde(default)]
    #[uniffi(default = None)]
    pub to_user: Option<String>,
    #[serde(default)]
    #[uniffi(default = None)]
    pub to_device: Option<String>,
    #[serde(default)]
    #[uniffi(default = false)]
    pub auto_fill: bool,
}

impl Received {
    pub(crate) fn with_origin(self, origin: &ot_core::Origin) -> Self {
        Self {
            from_user: origin.from_user.as_ref().map(ToString::to_string),
            from_service: origin.from_service,
            to_user: origin.to_user.as_ref().map(ToString::to_string),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PersonDevice {
    pub device_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub reachable: bool,
    pub default: bool,
    pub up_to_date: bool,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PersonItem {
    pub user_id: String,
    pub name: String,
    pub is_admin: bool,
    pub is_service: bool,
    pub muted: bool,
    pub devices: Vec<PersonDevice>,
}

impl From<ot_core::Person> for PersonItem {
    fn from(p: ot_core::Person) -> Self {
        Self {
            user_id: p.id.to_string(),
            name: p.name,
            is_admin: p.role == ot_core::proto::Role::Admin,
            is_service: p.role == ot_core::proto::Role::Service,
            muted: p.muted,
            devices: p
                .devices
                .into_iter()
                .map(|d| PersonDevice {
                    device_id: d.id.to_string(),
                    name: d.name,
                    kind: d.kind.map(|k| k.as_str().to_owned()),
                    reachable: d.reachable,
                    default: d.default,
                    up_to_date: d.up_to_date,
                    online: d.online,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Event {
    Connected { pending: u64 },
    Disconnected { reason: String },
    AuthFailed { message: String },
    Message { received: Received },
    Sent { item: Received },
    Unsent { msg_id: String },
    Rejected { from_name: String, reason: String },
    GroupChanged { epoch: u32 },
    PairingCompleted { device_id: String, name: String },
    JoinFailed { message: String },
    StorageError { message: String },
    DirectoryChanged,
    DirectoryRejected { user_id: String, reason: String },
    DirectoryFork,
    KeysChanged { user_id: String },
    PersonJoined { user_id: String, name: String, device_name: String, fingerprint: String },
    Joined { user_id: String },
    InviteFailed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AdminRoot {
    Ready { user_id: String },
    Missing { name: String },
    NotAdmin,
}

impl From<ot_core::AdminRoot> for AdminRoot {
    fn from(r: ot_core::AdminRoot) -> Self {
        match r {
            ot_core::AdminRoot::Ready(user) => Self::Ready { user_id: user.to_string() },
            ot_core::AdminRoot::Missing { name } => Self::Missing { name },
            ot_core::AdminRoot::NotAdmin => Self::NotAdmin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PersonInvite {
    pub user_id: String,
    pub code: String,
    pub uri: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServiceAdded {
    pub user_id: String,
    pub device_id: String,
    pub device_name: String,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AdminDeviceItem {
    pub device_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub reachable: bool,
    pub default: bool,
    pub up_to_date: bool,
    pub online: bool,
    pub last_seen_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AdminUserItem {
    pub user_id: String,
    pub name: String,
    pub role: String,
    pub state: String,
    pub devices: Vec<AdminDeviceItem>,
    pub links: Vec<String>,
    pub muted: Vec<String>,
}

impl From<ot_core::proto::AdminUser> for AdminUserItem {
    fn from(u: ot_core::proto::AdminUser) -> Self {
        Self {
            user_id: u.id.to_string(),
            name: u.name,
            role: u.role.as_str().to_owned(),
            state: u.state.as_str().to_owned(),
            devices: u
                .devices
                .into_iter()
                .map(|d| AdminDeviceItem {
                    device_id: d.id.to_string(),
                    name: d.name,
                    kind: d.kind.map(|k| k.as_str().to_owned()),
                    reachable: d.reachable,
                    default: d.default,
                    up_to_date: d
                        .features
                        .iter()
                        .any(|f| f.as_str() == ot_core::proto::FEATURE_USERS_V1),
                    online: d.online,
                    last_seen_at_ms: d.last_seen_at,
                })
                .collect(),
            links: u.links.iter().map(ToString::to_string).collect(),
            muted: u.muted.iter().map(ToString::to_string).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Member {
    pub device_id: String,
    pub name: String,
    pub sign_fingerprint: String,
    pub exchange_fingerprint: String,
    pub added_epoch: u32,
    pub is_self: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Group {
    pub epoch: u32,
    pub key_fingerprint: String,
    pub roster_version: u64,
    pub members: Vec<Member>,
}

impl Group {
    pub(crate) fn from_info(info: GroupInfo, me: &str) -> Self {
        Self {
            epoch: info.epoch,
            key_fingerprint: info.key_fingerprint,
            roster_version: info.roster_version,
            members: info
                .members
                .into_iter()
                .map(|m| Member {
                    is_self: m.device_id.as_str() == me,
                    device_id: m.device_id.to_string(),
                    name: m.name,
                    sign_fingerprint: m.sign_fingerprint,
                    exchange_fingerprint: m.exchange_fingerprint,
                    added_epoch: m.added_epoch,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RelayDevice {
    pub device_id: String,
    pub name: String,
    pub online: bool,
    pub member: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Invitation {
    pub code: String,
    pub uri: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Sponsored {
    pub device_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum QrKind {
    Join { server_url: String, code: String },
    Sponsor { name: String, device_id: String },
    Invite { server_url: String, code: String },
    Invalid { reason: String },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QrCode {
    pub size: u32,
    pub modules: Vec<bool>,
}

pub(crate) fn millis(t: time::OffsetDateTime) -> i64 {
    (t.unix_timestamp_nanos() / 1_000_000) as i64
}
