pub mod b64;
mod blob;
mod envelope;
mod frame;
pub mod http;
mod user;

pub use blob::{BlobId, InvalidBlobId};
pub use envelope::{Content, ContentKind, DeviceId, Encoding, Envelope, InvalidDeviceId, Payload};
pub use frame::{DecodeError, DeviceInfo, ErrorCode, Frame};
pub use ulid::Ulid;
pub use user::{
    AdminDevice, AdminUser, DeviceKind, DirectoryDevice, DirectoryUser, FEATURE_READ_V1,
    FEATURE_USERS_V1, Feature, InvalidFeature, InvalidUserId, Known, LogEntry, MAX_FEATURES,
    MAX_SIGNED_BYTES, Role, SignedBlob, UserId, UserState, valid_user_name,
};

pub const PROTOCOL_VERSION: u8 = 1;

pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;

pub const MAX_FRAME_BYTES: usize = 2 * MAX_ENVELOPE_BYTES;

pub const AUTH_DOMAIN: &[u8] = b"ot-auth-v1";

pub const NONCE_LEN: usize = 32;

pub const PAIR_DOMAIN: &[u8] = b"ot-pair-v1";

pub const PAIR_REGISTER_DOMAIN: &[u8] = b"ot-pair-reg-v1";

pub const PAIR_TOKEN_LEN: usize = 10;

pub const INVITE_DOMAIN: &[u8] = b"ot-invite-v1";

pub const MAX_RECIPIENTS: usize = 32;

pub const MAX_NAME_CHARS: usize = 64;

pub const WS_PATH: &str = "/v1/ws";

pub fn valid_device_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

pub fn auth_message(nonce: &[u8; NONCE_LEN], device_id: &DeviceId) -> Vec<u8> {
    let id = device_id.as_str().as_bytes();
    let mut msg = Vec::with_capacity(AUTH_DOMAIN.len() + NONCE_LEN + id.len());
    msg.extend_from_slice(AUTH_DOMAIN);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(id);
    msg
}

pub fn pair_redeem_message(nonce: &[u8; NONCE_LEN], token_hash: &[u8; 32]) -> Vec<u8> {
    [PAIR_DOMAIN, nonce, token_hash].concat()
}

pub fn pair_register_message(device_id: &DeviceId, public_key: &[u8; 32], name: &str) -> Vec<u8> {
    [PAIR_REGISTER_DOMAIN, device_id.as_str().as_bytes(), public_key, name.as_bytes()].concat()
}

pub fn invite_redeem_message(nonce: &[u8; NONCE_LEN], token_hash: &[u8; 32]) -> Vec<u8> {
    [INVITE_DOMAIN, nonce, token_hash].concat()
}
