//! Cœur client OldTown : identité d'appareil, connexion au relais,
//! authentification, reconnexion, file d'envoi, dédoublonnage, et, depuis
//! l'étape 2, chiffrement de bout en bout, roster et appairage.
//!
//! L'API est volontairement simple (un handle [`Client`] clonable, un flux
//! d'[`Event`], un trait [`Keyring`] fourni par l'appli) pour être exposée
//! plus tard via UniFFI.

mod client;
mod dedup;
mod identity;
mod keyring;
mod pairing;
pub mod roster;
mod secure;
mod tls;
mod uri;

pub use client::{
    Accepted, Backoff, Client, ClientConfig, ClientError, Event, Events, IDLE_TIMEOUT,
    PairingInvitation, Received,
};
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
pub use pairing::{Redeemed, redeem_pairing};
pub use secure::{GroupError, GroupInfo, MemberInfo, ensure_exchange_key, group_info, init_group};
pub use tls::TlsRoots;
pub use uri::{PAIR_URI_PREFIX, PairUri, classify};
