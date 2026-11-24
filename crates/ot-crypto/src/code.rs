use std::fmt;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{CryptoError, PairingSecret, Result};

pub const TOKEN_LEN: usize = ot_proto::PAIR_TOKEN_LEN;

const PREFIX: &str = "ot1-";
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const RAW_LEN: usize = TOKEN_LEN + 16;

#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub token: [u8; TOKEN_LEN],
    pub secret: PairingSecret,
}

impl PairingCode {
    pub fn encode(&self) -> Zeroizing<String> {
        let mut raw = Zeroizing::new([0u8; RAW_LEN]);
        raw[..TOKEN_LEN].copy_from_slice(&self.token);
        raw[TOKEN_LEN..].copy_from_slice(self.secret.as_bytes());
        base32_encode(PREFIX, &raw[..])
    }

    pub fn parse(text: &str) -> Result<Self> {
        let raw = base32_decode(text, PREFIX, RAW_LEN, "préfixe ot1- attendu")?;
        let mut token = [0; TOKEN_LEN];
        token.copy_from_slice(&raw[..TOKEN_LEN]);
        let mut secret = [0; 16];
        secret.copy_from_slice(&raw[TOKEN_LEN..]);
        Ok(Self { token, secret: PairingSecret::from_bytes(secret) })
    }

    pub fn token_id(&self) -> [u8; 8] {
        token_id(&self.token)
    }
}

fn base32_encode(prefix: &str, raw: &[u8]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(prefix.len() + (raw.len() * 8).div_ceil(5)));
    out.push_str(prefix);
    let (mut acc, mut bits) = (0u16, 0u32);
    for &byte in raw {
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

fn base32_decode(
    text: &str,
    prefix: &str,
    len: usize,
    bad_prefix: &'static str,
) -> Result<Zeroizing<Vec<u8>>> {
    let text = text.trim();
    let rest = text
        .get(..prefix.len())
        .filter(|p| p.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
        .ok_or(CryptoError::BadCode(bad_prefix))?;
    let mut raw = Zeroizing::new(vec![0u8; len]);
    let (mut acc, mut bits, mut n, mut chars) = (0u16, 0u32, 0usize, 0usize);
    for c in rest.chars().filter(|c| !matches!(c, ' ' | '-')) {
        let c = c.to_ascii_lowercase();
        let v = ALPHABET
            .iter()
            .position(|&a| char::from(a) == c)
            .ok_or(CryptoError::BadCode("caractère hors de l'alphabet base32"))?;
        chars += 1;
        acc = ((acc << 5) | v as u16) & 0x0fff;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if n == len {
                return Err(CryptoError::BadCode("code trop long"));
            }
            raw[n] = (acc >> bits) as u8;
            n += 1;
        }
    }
    if chars != (len * 8).div_ceil(5) || n != len || acc & ((1 << bits) - 1) != 0 {
        return Err(CryptoError::BadCode("longueur incorrecte"));
    }
    Ok(raw)
}

#[derive(Clone, PartialEq, Eq)]
pub struct InviteCode {
    pub token: [u8; TOKEN_LEN],
    pub secret: PairingSecret,
    pub anchor: [u8; 16],
}

const INVITE_PREFIX: &str = "ot2-";
const INVITE_VERSION: u8 = 1;
const INVITE_RAW_LEN: usize = 1 + TOKEN_LEN + 16 + 16;

impl InviteCode {
    pub fn encode(&self) -> Zeroizing<String> {
        let mut raw = Zeroizing::new([0u8; INVITE_RAW_LEN]);
        raw[0] = INVITE_VERSION;
        raw[1..1 + TOKEN_LEN].copy_from_slice(&self.token);
        raw[1 + TOKEN_LEN..1 + TOKEN_LEN + 16].copy_from_slice(self.secret.as_bytes());
        raw[1 + TOKEN_LEN + 16..].copy_from_slice(&self.anchor);
        base32_encode(INVITE_PREFIX, &raw[..])
    }

    pub fn parse(text: &str) -> Result<Self> {
        let raw = base32_decode(text, INVITE_PREFIX, INVITE_RAW_LEN, "préfixe ot2- attendu")?;
        if raw[0] != INVITE_VERSION {
            return Err(CryptoError::BadCode("version d'invitation inconnue"));
        }
        let mut token = [0; TOKEN_LEN];
        token.copy_from_slice(&raw[1..1 + TOKEN_LEN]);
        let mut secret = [0; 16];
        secret.copy_from_slice(&raw[1 + TOKEN_LEN..1 + TOKEN_LEN + 16]);
        let mut anchor = [0; 16];
        anchor.copy_from_slice(&raw[1 + TOKEN_LEN + 16..]);
        Ok(Self { token, secret: PairingSecret::from_bytes(secret), anchor })
    }

    pub fn token_id(&self) -> [u8; 8] {
        token_id(&self.token)
    }
}

impl fmt::Debug for InviteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InviteCode(…)")
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(…)")
    }
}

pub fn token_hash(token: &[u8; TOKEN_LEN]) -> [u8; 32] {
    Sha256::digest(token).into()
}

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
        let mut tweaked = text.to_string();
        let last = tweaked.pop().unwrap();
        let idx = ALPHABET.iter().position(|&a| char::from(a) == last).unwrap();
        tweaked.push(char::from(ALPHABET[idx ^ 1]));
        assert!(PairingCode::parse(&tweaked).is_err());
    }

    fn invite() -> InviteCode {
        InviteCode {
            token: std::array::from_fn(|i| i as u8),
            secret: PairingSecret::from_bytes(std::array::from_fn(|i| 0xf0 | i as u8)),
            anchor: std::array::from_fn(|i| 0x80 | i as u8),
        }
    }

    #[test]
    fn invite_roundtrip_and_rejects() {
        let text = invite().encode();
        assert_eq!(text.len(), 73);
        assert!(text.starts_with("ot2-"));
        assert_eq!(InviteCode::parse(&text).unwrap(), invite());
        assert_eq!(InviteCode::parse(&text.to_uppercase()).unwrap(), invite());
        assert!(InviteCode::parse(&text[..72]).is_err());
        assert!(InviteCode::parse(&format!("ot1-{}", &text[4..])).is_err());
        assert!(PairingCode::parse(&text).is_err());
        assert!(InviteCode::parse(&code().encode()).is_err());
        let mut v2 = text.to_string();
        v2.replace_range(4..5, "a");
        v2.replace_range(5..6, "i");
        assert!(InviteCode::parse(&v2).is_err());
        assert!(!format!("{:?}", invite()).contains("240"));
    }

    #[test]
    fn token_id_is_hash_prefix() {
        let c = code();
        assert_eq!(c.token_id(), token_hash(&c.token)[..8]);
    }
}
