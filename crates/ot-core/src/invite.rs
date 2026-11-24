use ot_crypto::directory::LogHead;
use ot_crypto::{InviteCode, PairingSecret};
use ot_proto::{DeviceId, SignedBlob, UserId, b64};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::directory::DirectoryState;
use crate::keyring::{Keyring, KeyringError};

pub const INVITE_EXCHANGE: Duration = Duration::hours(48);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guest {
    pub device_id: DeviceId,
    pub name: String,
    #[serde(with = "b64::array")]
    pub sign_key: [u8; 32],
    #[serde(with = "b64::array")]
    pub exchange_key: [u8; 32],
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteRecord {
    pub user: UserId,
    pub name: String,
    #[serde(with = "b64::array")]
    pub token_id: [u8; 8],
    #[serde(with = "b64::array")]
    pub secret: [u8; 16],
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest: Option<Guest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<SignedBlob>,
    #[serde(default)]
    pub certified: bool,
}

impl InviteRecord {
    pub fn secret(&self) -> PairingSecret {
        PairingSecret::from_bytes(self.secret)
    }

    pub fn alive(&self, now: OffsetDateTime) -> bool {
        now < self.expires_at + INVITE_EXCHANGE
    }
}

impl Drop for InviteRecord {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.secret);
    }
}

impl Drop for PendingInvite {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.secret);
    }
}

impl std::fmt::Debug for InviteRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InviteRecord")
            .field("user", &self.user)
            .field("guest", &self.guest.as_ref().map(|g| &g.device_id))
            .field("certified", &self.certified)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingInvite {
    pub user: UserId,
    pub inviter: DeviceId,
    pub name: String,
    #[serde(with = "b64::array")]
    pub token_id: [u8; 8],
    #[serde(with = "b64::array")]
    pub secret: [u8; 16],
    #[serde(with = "b64::array")]
    pub anchor: [u8; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<Welcome>,
}

impl PendingInvite {
    pub fn secret(&self) -> PairingSecret {
        PairingSecret::from_bytes(self.secret)
    }
}

impl std::fmt::Debug for PendingInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingInvite")
            .field("user", &self.user)
            .field("inviter", &self.inviter)
            .field("welcome", &self.welcome.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    pub cert: SignedBlob,
    pub admin_head: LogHead,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PersonInvitation {
    pub user: UserId,
    pub code: String,
    pub uri: String,
    pub expires_at: OffsetDateTime,
}

impl std::fmt::Debug for PersonInvitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonInvitation")
            .field("user", &self.user)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRedeemed {
    pub user: UserId,
    pub device_id: DeviceId,
    pub inviter: DeviceId,
}

pub fn save_pending_invite(
    keyring: &dyn Keyring,
    code: &InviteCode,
    redeemed: &InviteRedeemed,
    name: &str,
) -> Result<(), KeyringError> {
    let pending = PendingInvite {
        user: redeemed.user.clone(),
        inviter: redeemed.inviter.clone(),
        name: name.to_owned(),
        token_id: code.token_id(),
        secret: *code.secret.as_bytes(),
        anchor: code.anchor,
        welcome: None,
    };
    keyring.save_directory(&DirectoryState::with_anchor(code.anchor))?;
    keyring.save_pending_invite(Some(&pending))
}
