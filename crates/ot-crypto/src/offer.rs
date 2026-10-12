//! QR inversé (note 0022) : le nouvel appareil affiche ses clés publiques, le
//! secret `S` et une signature ; un membre du groupe le scanne, le fait
//! enregistrer par le relais, puis lui envoie la clé de groupe (schéma
//! `0x04`).
//!
//! Texte du QR : `oldtown://sponsor?o=` puis base64url sans remplissage de
//! `version (1) ‖ ULID de l'identifiant (16) ‖ clé Ed25519 (32) ‖ clé X25519
//! (32) ‖ S (16) ‖ signature (64) ‖ nom (UTF-8, le reste)`.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use ot_proto::{DeviceId, Ulid, pair_register_message};
use zeroize::Zeroizing;

use crate::{CryptoError, PairingSecret, Result};

/// Début du texte d'un QR inversé.
pub const OFFER_PREFIX: &str = "oldtown://sponsor?o=";
const VERSION: u8 = 1;
const FIXED_LEN: usize = 1 + 16 + 32 + 32 + 16 + 64;
/// Taille maximale d'un nom d'appareil, en caractères (comme le relais).
pub const MAX_NAME_CHARS: usize = 64;

/// Demande d'appairage affichée par le nouvel appareil.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingOffer {
    /// Identifiant choisi par le nouvel appareil, enregistré tel quel.
    pub device_id: DeviceId,
    pub name: String,
    /// Clé publique Ed25519 (authentification et signature des messages).
    pub sign_key: [u8; 32],
    /// Clé publique X25519 (réception de la clé de groupe).
    pub exchange_key: [u8; 32],
    /// Secret qui authentifie la réponse du parrain.
    pub secret: PairingSecret,
    /// Signature de [`pair_register_message`] par `sign_key`.
    pub sig: [u8; 64],
}

/// Nom accepté par le relais : 1 à 64 caractères imprimables, sans espace
/// autour.
pub fn valid_device_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

impl PairingOffer {
    /// Construit et signe l'offre du nouvel appareil.
    pub fn new(
        device_id: DeviceId,
        name: &str,
        signer: &SigningKey,
        exchange_key: [u8; 32],
        secret: PairingSecret,
    ) -> Result<Self> {
        if !valid_device_name(name) {
            return Err(CryptoError::BadCode("nom d'appareil invalide"));
        }
        let sign_key = signer.verifying_key().to_bytes();
        let sig = signer.sign(&pair_register_message(&device_id, &sign_key, name)).to_bytes();
        Ok(Self { device_id, name: name.to_owned(), sign_key, exchange_key, secret, sig })
    }

    /// Vérifie la signature de l'offre (ce que le relais vérifiera aussi).
    pub fn verify(&self) -> Result<()> {
        let key =
            VerifyingKey::from_bytes(&self.sign_key).map_err(|_| CryptoError::BadSignature)?;
        let msg = pair_register_message(&self.device_id, &self.sign_key, &self.name);
        key.verify_strict(&msg, &Signature::from_bytes(&self.sig))
            .map_err(|_| CryptoError::BadSignature)
    }

    /// Texte du QR ; il contient `S`, d'où l'effacement à la destruction.
    pub fn encode(&self) -> Zeroizing<String> {
        let mut raw = Zeroizing::new(Vec::with_capacity(FIXED_LEN + self.name.len()));
        raw.push(VERSION);
        raw.extend_from_slice(&self.device_id.ulid().to_bytes());
        raw.extend_from_slice(&self.sign_key);
        raw.extend_from_slice(&self.exchange_key);
        raw.extend_from_slice(self.secret.as_bytes());
        raw.extend_from_slice(&self.sig);
        raw.extend_from_slice(self.name.as_bytes());
        let mut out = Zeroizing::new(String::from(OFFER_PREFIX));
        URL_SAFE_NO_PAD.encode_string(&*raw, &mut out);
        out
    }

    /// Relit le texte d'un QR inversé et vérifie sa signature.
    pub fn parse(text: &str) -> Result<Self> {
        let data = text
            .trim()
            .strip_prefix(OFFER_PREFIX)
            .ok_or(CryptoError::BadCode("QR d'appairage OldTown attendu"))?;
        let raw = Zeroizing::new(
            URL_SAFE_NO_PAD.decode(data).map_err(|_| CryptoError::BadCode("QR illisible"))?,
        );
        if raw.len() <= FIXED_LEN {
            return Err(CryptoError::BadCode("QR tronqué"));
        }
        if raw[0] != VERSION {
            return Err(CryptoError::BadCode("version de QR inconnue"));
        }
        let mut at = 1;
        let mut take = |n: usize| {
            let part = &raw[at..at + n];
            at += n;
            part
        };
        let ulid: [u8; 16] = take(16).try_into().map_err(|_| CryptoError::BadCode("QR"))?;
        let sign_key = take(32).try_into().map_err(|_| CryptoError::BadCode("QR"))?;
        let exchange_key = take(32).try_into().map_err(|_| CryptoError::BadCode("QR"))?;
        let secret: [u8; 16] = take(16).try_into().map_err(|_| CryptoError::BadCode("QR"))?;
        let sig = take(64).try_into().map_err(|_| CryptoError::BadCode("QR"))?;
        let name = std::str::from_utf8(&raw[FIXED_LEN..])
            .map_err(|_| CryptoError::BadCode("nom d'appareil invalide"))?;
        if !valid_device_name(name) {
            return Err(CryptoError::BadCode("nom d'appareil invalide"));
        }
        let offer = Self {
            device_id: DeviceId::from_ulid(Ulid::from_bytes(ulid)),
            name: name.to_owned(),
            sign_key,
            exchange_key,
            secret: PairingSecret::from_bytes(secret),
            sig,
        };
        offer.verify()?;
        Ok(offer)
    }
}

impl fmt::Debug for PairingOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingOffer")
            .field("device_id", &self.device_id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(name: &str) -> PairingOffer {
        let signer = SigningKey::from_bytes(&[2; 32]);
        let secret = PairingSecret::from_bytes([3; 16]);
        PairingOffer::new(DeviceId::generate(), name, &signer, [4; 32], secret).unwrap()
    }

    #[test]
    fn roundtrip() {
        let o = offer("PC du salon");
        let text = o.encode();
        assert!(text.starts_with(OFFER_PREFIX));
        assert_eq!(PairingOffer::parse(&text).unwrap(), o);
        assert!(!format!("{o:?}").contains("secret"));
    }

    #[test]
    fn rejects_bad_offers() {
        let text = offer("windows").encode().to_string();
        let raw = URL_SAFE_NO_PAD.decode(&text[OFFER_PREFIX.len()..]).unwrap();
        let reencode = |raw: &[u8]| format!("{OFFER_PREFIX}{}", URL_SAFE_NO_PAD.encode(raw));

        // Nom changé : la signature ne correspond plus.
        let mut renamed = raw.clone();
        *renamed.last_mut().unwrap() ^= 1;
        assert_eq!(PairingOffer::parse(&reencode(&renamed)), Err(CryptoError::BadSignature));
        // Autre version, tronqué, sans nom, mauvais préfixe.
        let mut v2 = raw.clone();
        v2[0] = 2;
        assert!(PairingOffer::parse(&reencode(&v2)).is_err());
        assert!(PairingOffer::parse(&reencode(&raw[..FIXED_LEN])).is_err());
        assert!(PairingOffer::parse(&text.replace("sponsor", "pair")).is_err());
        assert!(PairingOffer::parse("oldtown://sponsor?o=!!").is_err());
    }

    #[test]
    fn names_follow_relay_rules() {
        for bad in ["", " mac", "mac ", "a\nb", &"x".repeat(65)] {
            assert!(!valid_device_name(bad), "{bad:?}");
        }
        assert!(valid_device_name("Téléphone de Léo"));
        let signer = SigningKey::from_bytes(&[2; 32]);
        let secret = PairingSecret::from_bytes([3; 16]);
        assert!(PairingOffer::new(DeviceId::generate(), "", &signer, [4; 32], secret).is_err());
    }
}
