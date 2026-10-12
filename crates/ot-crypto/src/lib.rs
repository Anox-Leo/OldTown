//! Chiffrement de bout en bout OldTown : types de clés, formats chiffrés de
//! `payload.data`, corps des messages et code d'appairage.
//!
//! Aucune E/S, aucun réseau, aucun async : tout est testable seul et
//! réutilisable par les applis (UniFFI, WebAssembly). Le relais ne dépend
//! jamais de cette crate.
//!
//! Format de `payload.data` (figé par `tests/vectors/`) :
//!
//! | Schéma | Octets | Clé |
//! | --- | --- | --- |
//! | `0x01` groupe | `01 ‖ époque (u32 BE) ‖ nonce ‖ chiffré` | clé de groupe de l'époque |
//! | `0x02` appairage | `02 ‖ id du jeton (8 o) ‖ nonce ‖ chiffré` | `HKDF-SHA256(S, "ot-pair-v1")` |
//! | `0x03` appareil | `03 ‖ X25519 éphémère (32 o) ‖ nonce ‖ chiffré` | `HKDF-SHA256(DH, "ot-device-v1" ‖ éph ‖ dest)` |
//! | `0x04` QR inversé | `04 ‖ X25519 éphémère (32 o) ‖ nonce ‖ chiffré` | `HKDF-SHA256(DH ‖ S, "ot-pair-qr-v1" ‖ éph ‖ dest)` |
//!
//! Le clair est `signature Ed25519 (64 o) ‖ corps JSON` ; la signature porte
//! sur `"ot-sig-v1" ‖ données associées ‖ corps`.

mod body;
mod code;
mod keys;
mod offer;
mod seal;

pub use body::{Body, EpochKey, Member, Roster};
pub use code::{PairingCode, TOKEN_LEN, token_hash, token_id};
pub use ed25519_dalek::SigningKey;
pub use keys::{ExchangeKeyPair, GroupKey, PairingSecret, fingerprint};
pub use offer::{MAX_NAME_CHARS, OFFER_PREFIX, PairingOffer, valid_device_name};
pub use seal::{
    Header, NONCE_LEN, OpenKey, Opened, Randomness, SealKey, associated_data, open, seal, seal_with,
};

/// Erreurs de la crate. Aucune ne contient de clé ni de contenu.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// `payload.enc` vaut `none` : le clair est toujours refusé.
    #[error("message en clair refusé")]
    Plaintext,
    #[error("format chiffré invalide : {0}")]
    Malformed(&'static str),
    /// Clé fausse, ou chiffré, en-tête ou données associées altérés.
    #[error("déchiffrement impossible")]
    Decrypt,
    #[error("signature invalide")]
    BadSignature,
    /// Clé publique X25519 de faible ordre : l'échange ne serait pas secret.
    #[error("clé publique X25519 refusée")]
    WeakKey,
    #[error("code d'appairage invalide : {0}")]
    BadCode(&'static str),
    #[error("corps de message invalide : {0}")]
    Body(String),
}

pub type Result<T, E = CryptoError> = std::result::Result<T, E>;

/// Remplit `buf` d'aléa du système.
pub(crate) fn random(buf: &mut [u8]) {
    // Sans aléa, aucune clé ni aucun nonce sûr n'est possible : on s'arrête.
    getrandom::fill(buf).expect("source d'aléa du système indisponible");
}
