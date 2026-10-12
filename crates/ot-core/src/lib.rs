//! Cœur client OldTown : identité d'appareil, connexion au relais,
//! authentification, reconnexion, file d'envoi, dédoublonnage, et, depuis
//! l'étape 2, chiffrement de bout en bout, roster et appairage.
//!
//! La fonction `net` (par défaut) porte la connexion au relais ([`Client`],
//! TLS, tokio). Sans elle, le cœur se compile en WebAssembly : identité,
//! chiffrement, roster, appairage ([`Secure`]), pour l'extension (note 0023).
//!
//! L'API est volontairement simple (un handle [`Client`] clonable, un flux
//! d'[`Event`], un trait [`Keyring`] fourni par l'appli) pour être exposée
//! plus tard via UniFFI.

#[cfg(feature = "net")]
mod client;
mod dedup;
mod identity;
mod keyring;
#[cfg(feature = "net")]
mod pairing;
pub mod roster;
mod secure;
#[cfg(feature = "net")]
mod tls;
mod types;
mod uri;

#[cfg(feature = "net")]
pub use client::{Backoff, Client, ClientConfig, Events, IDLE_TIMEOUT};
pub use dedup::Dedup;
pub use identity::Identity;
pub use keyring::{
    EpochEntry, GroupKeys, Keyring, KeyringError, MAX_QUARANTINE, MemoryKeyring, MemoryStore,
    Quarantined, RETIRED_KEY_LIFETIME, SecretStore, StoreKeyring, names,
};
pub use ot_crypto::{
    self as crypto, ExchangeKeyPair, OFFER_PREFIX, PairingOffer, Roster, valid_device_name,
};
pub use ot_proto::{self as proto, Content, ContentKind, DeviceId, DeviceInfo, ErrorCode, Ulid};
#[cfg(feature = "net")]
pub use pairing::{Redeemed, redeem_pairing};
pub use secure::{
    Effect, GroupError, GroupInfo, MemberInfo, Secure, ensure_exchange_key, group_info, init_group,
};
#[cfg(feature = "net")]
pub use tls::TlsRoots;
pub use types::{Accepted, ClientError, Event, PairingInvitation, Received};
pub use uri::{PAIR_URI_PREFIX, PairUri, classify};
