//! Vérification du défi signé et jetons d'appairage.

use ed25519_dalek::{Signature, VerifyingKey};
use ot_proto::{
    DeviceId, NONCE_LEN, PAIR_TOKEN_LEN, auth_message, pair_redeem_message, pair_register_message,
};
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
    verify_sig(public_key, &auth_message(nonce, device_id), sig)
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
    verify_sig(public_key, &pair_redeem_message(nonce, token_hash), sig)
}

/// Vérifie la signature de `pair_register` par la clé du nouvel appareil.
pub fn verify_register(
    public_key: &[u8; 32],
    device_id: &DeviceId,
    name: &str,
    sig: &[u8; 64],
) -> bool {
    verify_sig(public_key, &pair_register_message(device_id, public_key, name), sig)
}

/// Signature Ed25519 stricte de `msg` par `public_key` ; une clé invalide
/// donne `false`.
fn verify_sig(public_key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(public_key)
        .is_ok_and(|key| key.verify_strict(msg, &Signature::from_bytes(sig)).is_ok())
}
