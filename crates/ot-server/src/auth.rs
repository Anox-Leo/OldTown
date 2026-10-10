//! Vérification du défi signé.

use ed25519_dalek::{Signature, VerifyingKey};
use ot_proto::{DeviceId, NONCE_LEN, auth_message};

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
