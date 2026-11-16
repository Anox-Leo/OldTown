//! Vérification du défi signé, des requêtes HTTP signées et des jetons
//! d'appairage.

use std::collections::HashMap;
use std::sync::Mutex;

use ed25519_dalek::{Signature, VerifyingKey};
use ot_proto::http::{HTTP_NONCE_LEN, HTTP_WINDOW_MS, http_request_message};
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

/// Vérifie la signature d'une requête HTTP (étape 5) par `public_key`.
pub fn verify_http(
    public_key: &[u8; 32],
    method: &str,
    path: &str,
    time_ms: u64,
    nonce: &[u8; HTTP_NONCE_LEN],
    body: &[u8],
    sig: &[u8; 64],
) -> bool {
    let body_hash: [u8; 32] = Sha256::digest(body).into();
    verify_sig(public_key, &http_request_message(method, path, time_ms, nonce, &body_hash), sig)
}

/// Nonces des requêtes HTTP vus pendant la fenêtre, pour refuser un rejeu.
/// En mémoire seulement : un redémarrage les oublie (note 0026).
#[derive(Debug)]
pub struct NonceCache {
    /// (appareil, nonce) → instant jusqu'auquel le garder.
    seen: Mutex<HashMap<(DeviceId, [u8; HTTP_NONCE_LEN]), i64>>,
    capacity: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceError {
    Replayed,
    /// Plus de `capacity` nonces vivants : le relais refuse plutôt que de
    /// grossir sans fin.
    Full,
}

impl NonceCache {
    pub fn new(capacity: usize) -> Self {
        Self { seen: Mutex::new(HashMap::new()), capacity }
    }

    /// Retient le nonce, ou le refuse s'il a déjà servi. Une requête n'est
    /// acceptée qu'à ±`HTTP_WINDOW_MS` de l'heure du relais : garder un nonce
    /// deux fenêtres couvre tous les rejeux possibles.
    pub fn check(
        &self,
        device: &DeviceId,
        nonce: [u8; HTTP_NONCE_LEN],
        now: i64,
    ) -> Result<(), NonceError> {
        // Invariant : aucun code ne panique en tenant le verrou.
        let mut seen = self.seen.lock().expect("verrou des nonces empoisonné");
        if seen.len() >= self.capacity {
            seen.retain(|_, until| *until > now);
        }
        let key = (device.clone(), nonce);
        if seen.get(&key).is_some_and(|until| *until > now) {
            return Err(NonceError::Replayed);
        }
        if seen.len() >= self.capacity {
            return Err(NonceError::Full);
        }
        seen.insert(key, now + 2 * HTTP_WINDOW_MS);
        Ok(())
    }
}

/// Signature Ed25519 stricte de `msg` par `public_key` ; une clé invalide
/// donne `false`.
fn verify_sig(public_key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(public_key)
        .is_ok_and(|key| key.verify_strict(msg, &Signature::from_bytes(sig)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_cache_refuses_replays_and_stays_bounded() {
        let cache = NonceCache::new(2);
        let (a, b) = (DeviceId::generate(), DeviceId::generate());
        assert_eq!(cache.check(&a, [1; 16], 0), Ok(()));
        assert_eq!(cache.check(&a, [1; 16], 1_000), Err(NonceError::Replayed));
        // Le même nonce d'un autre appareil n'est pas un rejeu.
        assert_eq!(cache.check(&b, [1; 16], 1_000), Ok(()));
        assert_eq!(cache.check(&a, [2; 16], 1_000), Err(NonceError::Full));
        // Deux fenêtres après le dernier, les anciens nonces sont oubliés.
        let later = 1_000 + 2 * HTTP_WINDOW_MS + 1;
        assert_eq!(cache.check(&a, [2; 16], later), Ok(()));
        assert_eq!(cache.check(&a, [1; 16], later), Ok(()));
    }
}
