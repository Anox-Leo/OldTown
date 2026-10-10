//! Vérification du défi signé et jetons d'appairage.

use ed25519_dalek::{Signature, VerifyingKey};
use ot_proto::{DeviceId, NONCE_LEN, PAIR_TOKEN_LEN, auth_message, pair_redeem_message};
use sha2::{Digest, Sha256};

pub fn new_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("source d'aléa du système indisponible");
    nonce
}

/// Vérifie la signature de `"ot-auth-v1" ‖ nonce ‖ device_id` par `public_key`.
pub fn verify(
    public_key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    device_id: &DeviceId,
    sig: &[u8; 64],
) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    key.verify_strict(&auth_message(nonce, device_id), &Signature::from_bytes(sig)).is_ok()
}

pub fn new_token() -> [u8; PAIR_TOKEN_LEN] {
    let mut token = [0; PAIR_TOKEN_LEN];
    getrandom::fill(&mut token).expect("source d'aléa du système indisponible");
    token
}

/// SHA-256 du jeton : seule forme stockée en base.
pub fn token_hash(token: &[u8; PAIR_TOKEN_LEN]) -> [u8; 32] {
    Sha256::digest(token).into()
}

/// Vérifie la signature de `"ot-pair-v1" ‖ nonce ‖ sha256(jeton)` par la
/// clé du nouvel appareil.
pub fn verify_redeem(
    public_key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    token_hash: &[u8; 32],
    sig: &[u8; 64],
) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    key.verify_strict(&pair_redeem_message(nonce, token_hash), &Signature::from_bytes(sig)).is_ok()
}
