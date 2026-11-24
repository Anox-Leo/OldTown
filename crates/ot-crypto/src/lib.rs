mod body;
mod code;
pub mod directory;
mod file;
mod keys;
mod offer;
mod seal;

pub use body::{Body, EpochKey, Member, Otp, Roster};
pub use code::{InviteCode, PairingCode, TOKEN_LEN, token_hash, token_id};
pub use ed25519_dalek::SigningKey;
pub use file::{
    CHUNK_TAG_LEN, FILE_CHUNK_SIZE, FILE_NONCE_LEN, FileCipher, FileKey, MAX_CHUNK_SIZE,
    check_layout, chunk_count, chunk_len, new_file_nonce, sanitize_file_name, sealed_size,
};
pub use keys::{ExchangeKeyPair, GroupKey, PairingSecret, fingerprint};
pub use offer::{OFFER_PREFIX, PairingOffer};
pub use ot_proto::{MAX_NAME_CHARS, valid_device_name};
pub use seal::{
    COMMIT_LEN, Header, NONCE_LEN, OpenKey, Opened, Randomness, SealKey, WRAP_LEN, associated_data,
    open, seal, seal_with,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("message en clair refusé")]
    Plaintext,
    #[error("format chiffré invalide : {0}")]
    Malformed(&'static str),
    #[error("déchiffrement impossible")]
    Decrypt,
    #[error("signature invalide")]
    BadSignature,
    #[error("clé publique X25519 refusée")]
    WeakKey,
    #[error("code d'appairage invalide : {0}")]
    BadCode(&'static str),
    #[error("corps de message invalide : {0}")]
    Body(String),
}

pub type Result<T, E = CryptoError> = std::result::Result<T, E>;

pub(crate) fn random(buf: &mut [u8]) {
    getrandom::fill(buf).expect("source d'aléa du système indisponible");
}
