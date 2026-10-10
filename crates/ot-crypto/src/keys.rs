//! Types de clés. Toutes les clés secrètes s'effacent de la mémoire à la
//! destruction et ne s'affichent jamais en `Debug`.

use std::fmt;

use ot_proto::b64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::random;

/// Clé de groupe d'une époque (32 octets), partagée par tous les membres.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct GroupKey([u8; 32]);

impl GroupKey {
    pub fn generate() -> Self {
        let mut key = [0; 32];
        random(&mut key);
        Self(key)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Empreinte courte, pour comparer deux appareils à l'œil.
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.0)
    }
}

impl fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GroupKey(…)")
    }
}

impl Serialize for GroupKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        b64::array::serialize(&self.0, s)
    }
}

impl<'de> Deserialize<'de> for GroupKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        b64::array::deserialize(d).map(Self)
    }
}

/// Paire X25519 de l'appareil : reçoit une clé de groupe scellée pour lui.
#[derive(Clone)]
pub struct ExchangeKeyPair(StaticSecret);

impl ExchangeKeyPair {
    pub fn generate() -> Self {
        let mut seed = [0; 32];
        random(&mut seed);
        let pair = Self::from_bytes(seed);
        seed.zeroize();
        pair
    }

    pub fn from_bytes(secret: [u8; 32]) -> Self {
        Self(StaticSecret::from(secret))
    }

    /// Clé secrète, pour la stocker.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn public_key(&self) -> [u8; 32] {
        PublicKey::from(&self.0).to_bytes()
    }

    pub(crate) fn secret(&self) -> &StaticSecret {
        &self.0
    }
}

impl fmt::Debug for ExchangeKeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ExchangeKeyPair").field(&fingerprint(&self.public_key())).finish()
    }
}

/// Secret d'appairage `S` (16 octets) : ne quitte jamais le code.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct PairingSecret([u8; 16]);

impl PairingSecret {
    pub fn generate() -> Self {
        let mut s = [0; 16];
        random(&mut s);
        Self(s)
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for PairingSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingSecret(…)")
    }
}

/// Empreinte courte d'une clé : 8 premiers octets de SHA-256, en hexadécimal
/// par groupes de quatre (`a1b2 c3d4 e5f6 0718`).
pub fn fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hex: Vec<String> =
        digest[..8].chunks(2).map(|c| format!("{:02x}{:02x}", c[0], c[1])).collect();
    hex.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_secrets() {
        let key = GroupKey::from_bytes([0xab; 32]);
        assert_eq!(format!("{key:?}"), "GroupKey(…)");
        let secret = PairingSecret::from_bytes([0xab; 16]);
        assert!(!format!("{secret:?}").contains("171"));
        let pair = ExchangeKeyPair::from_bytes([7; 32]);
        assert!(format!("{pair:?}").starts_with("ExchangeKeyPair(\""));
    }

    #[test]
    fn exchange_key_roundtrip() {
        let pair = ExchangeKeyPair::generate();
        let again = ExchangeKeyPair::from_bytes(pair.to_bytes());
        assert_eq!(pair.public_key(), again.public_key());
    }

    #[test]
    fn fingerprint_layout() {
        let f = fingerprint(b"x");
        assert_eq!(f.len(), 19);
        assert_eq!(f.split(' ').count(), 4);
    }
}
