//! Code d'appairage : `ot1-` puis base32 (RFC 4648, sans remplissage, en
//! minuscules) de `T (10 o) ‖ S (16 o)`, soit 46 caractères. Le relais ne voit
//! que le jeton `T` ; le secret `S` ne quitte jamais le code.

use std::fmt;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{CryptoError, PairingSecret, Result};

/// Taille du jeton d'appairage tiré par le relais.
pub const TOKEN_LEN: usize = ot_proto::PAIR_TOKEN_LEN;

const PREFIX: &str = "ot1-";
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const RAW_LEN: usize = TOKEN_LEN + 16;
/// 26 octets = 208 bits, soit 42 caractères base32.
const ENCODED_LEN: usize = (RAW_LEN * 8).div_ceil(5);

/// Code d'appairage : jeton du relais et secret du parrain.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub token: [u8; TOKEN_LEN],
    pub secret: PairingSecret,
}

impl PairingCode {
    /// Texte à afficher au parrain et à saisir sur le nouvel appareil.
    pub fn encode(&self) -> Zeroizing<String> {
        let mut raw = Zeroizing::new([0u8; RAW_LEN]);
        raw[..TOKEN_LEN].copy_from_slice(&self.token);
        raw[TOKEN_LEN..].copy_from_slice(self.secret.as_bytes());
        let mut out = Zeroizing::new(String::with_capacity(PREFIX.len() + ENCODED_LEN));
        out.push_str(PREFIX);
        let (mut acc, mut bits) = (0u16, 0u32);
        for &byte in raw.iter() {
            acc = (acc << 8) | u16::from(byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(char::from(ALPHABET[usize::from((acc >> bits) & 31)]));
            }
        }
        if bits > 0 {
            out.push(char::from(ALPHABET[usize::from((acc << (5 - bits)) & 31)]));
        }
        out
    }

    /// Relit un code ; insensible à la casse, espaces et tirets ignorés après
    /// le préfixe (pour un code recopié à la main).
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let rest = text
            .get(..PREFIX.len())
            .filter(|p| p.eq_ignore_ascii_case(PREFIX))
            .map(|_| &text[PREFIX.len()..])
            .ok_or(CryptoError::BadCode("préfixe ot1- attendu"))?;
        let mut raw = Zeroizing::new([0u8; RAW_LEN]);
        let (mut acc, mut bits, mut n, mut chars) = (0u16, 0u32, 0usize, 0usize);
        for c in rest.chars().filter(|c| !matches!(c, ' ' | '-')) {
            let c = c.to_ascii_lowercase();
            let v = ALPHABET
                .iter()
                .position(|&a| char::from(a) == c)
                .ok_or(CryptoError::BadCode("caractère hors de l'alphabet base32"))?;
            chars += 1;
            acc = (acc << 5) | v as u16;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                if n == RAW_LEN {
                    return Err(CryptoError::BadCode("code trop long"));
                }
                raw[n] = (acc >> bits) as u8;
                n += 1;
            }
        }
        // Les bits de remplissage du dernier caractère doivent être nuls :
        // un seul texte possible par code.
        if chars != ENCODED_LEN || n != RAW_LEN || acc & ((1 << bits) - 1) != 0 {
            return Err(CryptoError::BadCode("longueur incorrecte"));
        }
        let mut token = [0; TOKEN_LEN];
        token.copy_from_slice(&raw[..TOKEN_LEN]);
        let mut secret = [0; 16];
        secret.copy_from_slice(&raw[TOKEN_LEN..]);
        Ok(Self { token, secret: PairingSecret::from_bytes(secret) })
    }

    /// Identifiant du jeton porté par l'en-tête `0x02`.
    pub fn token_id(&self) -> [u8; 8] {
        token_id(&self.token)
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(…)")
    }
}

/// `sha256(T)` : ce que le relais stocke et ce que signe `pair_redeem`.
pub fn token_hash(token: &[u8; TOKEN_LEN]) -> [u8; 32] {
    Sha256::digest(token).into()
}

/// `sha256(T)[..8]` : identifiant du jeton dans l'en-tête `0x02`.
pub fn token_id(token: &[u8; TOKEN_LEN]) -> [u8; 8] {
    let mut id = [0; 8];
    id.copy_from_slice(&token_hash(token)[..8]);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> PairingCode {
        PairingCode {
            token: std::array::from_fn(|i| i as u8),
            secret: PairingSecret::from_bytes(std::array::from_fn(|i| 0xf0 | i as u8)),
        }
    }

    #[test]
    fn roundtrip_and_length() {
        let text = code().encode();
        assert_eq!(text.len(), 46);
        assert!(text.starts_with("ot1-"));
        assert_eq!(PairingCode::parse(&text).unwrap(), code());
        assert_eq!(PairingCode::parse(&text.to_uppercase()).unwrap(), code());
        let spaced: String = text
            .chars()
            .enumerate()
            .flat_map(|(i, c)| if i > 4 && i % 6 == 0 { vec![' ', c] } else { vec![c] })
            .collect();
        assert_eq!(PairingCode::parse(&spaced).unwrap(), code());
    }

    #[test]
    fn rejects_bad_codes() {
        let text = code().encode();
        for bad in [
            "",
            "ot2-aaaa",
            &text[..45],
            &format!("{}a", *text),
            &format!("{}1", &text[..45]),
            &text[4..],
        ] {
            assert!(PairingCode::parse(bad).is_err(), "aurait dû être refusé : {bad}");
        }
        // Dernier caractère avec des bits de remplissage non nuls.
        let mut tweaked = text.to_string();
        let last = tweaked.pop().unwrap();
        let idx = ALPHABET.iter().position(|&a| char::from(a) == last).unwrap();
        tweaked.push(char::from(ALPHABET[idx ^ 1]));
        assert!(PairingCode::parse(&tweaked).is_err());
    }

    #[test]
    fn token_id_is_hash_prefix() {
        let c = code();
        assert_eq!(c.token_id(), token_hash(&c.token)[..8]);
    }
}
