#[cfg(feature = "net")]
mod client;
mod dedup;
pub mod directory;
pub mod files;
#[cfg(feature = "net")]
mod http;
mod identity;
pub mod invite;
mod keyring;
#[cfg(feature = "net")]
mod pairing;
#[cfg(feature = "net")]
mod people;
pub mod roster;
mod secure;
#[cfg(feature = "net")]
mod services;
#[cfg(feature = "net")]
mod tls;
#[cfg(feature = "net")]
mod transfer;
mod types;
mod uri;

#[cfg(feature = "net")]
pub use client::{Backoff, Client, ClientConfig, Events, IDLE_TIMEOUT};
pub use dedup::Dedup;
pub use directory::{DeviceProfile, Person};
pub use files::{FileInfo, FileMeta, PendingUpload, ReceivedFile, StoredFile};
pub use identity::Identity;
pub use invite::{InviteRedeemed, PersonInvitation, save_pending_invite};
pub use keyring::{
    EpochEntry, GroupKeys, Keyring, KeyringError, MAX_QUARANTINE, MemoryKeyring, MemoryStore,
    Quarantined, RETIRED_KEY_LIFETIME, SecretStore, StoreKeyring, names,
};
pub use ot_crypto::{
    self as crypto, ExchangeKeyPair, InviteCode, OFFER_PREFIX, Otp, PairingOffer, Roster,
    valid_device_name,
};
pub use ot_proto::{
    self as proto, BlobId, Content, ContentKind, DeviceId, DeviceInfo, DeviceKind, ErrorCode, Role,
    Ulid, UserId,
};
#[cfg(feature = "net")]
pub use pairing::{Redeemed, redeem_invite, redeem_pairing};
#[cfg(feature = "net")]
pub use people::DeviceSettings;
pub use secure::{
    DIRECTORY_MAX_AGE, Effect, GroupError, GroupInfo, MemberInfo, NextEntry, OwnEntry, Publish,
    Secure, ensure_exchange_key, group_info, init_group,
};
#[cfg(feature = "net")]
pub use services::ServiceAdded;
#[cfg(feature = "net")]
pub use tls::TlsRoots;
#[cfg(feature = "net")]
pub use transfer::{ChunkSink, ChunkSource, OutgoingFile, Progress, TransferConfig};
pub use types::{
    Accepted, AdminRoot, ClientError, Event, Origin, PairingInvitation, Received, ReceivedOtp,
    Target,
};
pub use uri::{INVITE_URI_PREFIX, InviteUri, PAIR_URI_PREFIX, PairUri, classify};
