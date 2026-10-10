use ed25519_dalek::{Signer, SigningKey};
use ot_proto::{DeviceId, NONCE_LEN, auth_message, b64, pair_redeem_message};

/// Paire de clés Ed25519 de l'appareil.
pub struct Identity(SigningKey);

impl Identity {
    pub fn generate() -> Self {
        let mut seed = [0; 32];
        getrandom::fill(&mut seed).expect("source d'aléa du système indisponible");
        Self::from_bytes(&seed)
    }

    /// Clé privée (graine de 32 octets).
    pub fn from_bytes(seed: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    /// Clé publique en base64, à passer à `ot-server device add`.
    pub fn public_key_b64(&self) -> String {
        b64::encode(&self.public_key())
    }

    /// Réponse au défi du serveur.
    pub fn sign_challenge(&self, nonce: &[u8; NONCE_LEN], device_id: &DeviceId) -> [u8; 64] {
        self.0.sign(&auth_message(nonce, device_id)).to_bytes()
    }

    /// Signature de `pair_redeem` : preuve de possession de la clé.
    pub fn sign_pair_redeem(&self, nonce: &[u8; NONCE_LEN], token_hash: &[u8; 32]) -> [u8; 64] {
        self.0.sign(&pair_redeem_message(nonce, token_hash)).to_bytes()
    }

    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.0
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Identity").field(&self.public_key_b64()).finish()
    }
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Self::from_bytes(&self.to_bytes())
    }
}
