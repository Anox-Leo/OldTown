//! Chiffrement et signature de `payload.data`, pour les trois schémas.

use chacha20poly1305::aead::{Aead, KeyInit, Payload as AeadPayload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use ot_proto::{Encoding, Envelope, Payload};
use sha2::Sha256;
use time::format_description::well_known::Rfc3339;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::{Body, CryptoError, ExchangeKeyPair, GroupKey, PairingSecret, Result, random};

/// Taille du nonce XChaCha20-Poly1305.
pub const NONCE_LEN: usize = 24;
const SIG_LEN: usize = 64;

const ENV_DOMAIN: &[u8] = b"ot-env-v1";
const SIG_DOMAIN: &[u8] = b"ot-sig-v1";
const PAIR_INFO: &[u8] = b"ot-pair-v1";
const DEVICE_INFO: &[u8] = b"ot-device-v1";

const SCHEME_GROUP: u8 = 0x01;
const SCHEME_PAIRING: u8 = 0x02;
const SCHEME_DEVICE: u8 = 0x03;

/// En-tête d'un `payload.data` chiffré : il dit quelle clé ouvre le message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// `0x01` : clé de groupe de l'époque.
    Group { epoch: u32 },
    /// `0x02` : clé dérivée du secret d'appairage ; `token_id = sha256(T)[..8]`.
    Pairing { token_id: [u8; 8] },
    /// `0x03` : clé dérivée d'un échange X25519 avec une clé éphémère.
    Device { ephemeral: [u8; 32] },
}

impl Header {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(33);
        match self {
            Self::Group { epoch } => {
                out.push(SCHEME_GROUP);
                out.extend_from_slice(&epoch.to_be_bytes());
            }
            Self::Pairing { token_id } => {
                out.push(SCHEME_PAIRING);
                out.extend_from_slice(token_id);
            }
            Self::Device { ephemeral } => {
                out.push(SCHEME_DEVICE);
                out.extend_from_slice(ephemeral);
            }
        }
        out
    }

    /// Lit l'en-tête d'un payload ; refuse le clair.
    pub fn parse(payload: &Payload) -> Result<Self> {
        if payload.enc != Encoding::Xchacha20poly1305 {
            return Err(CryptoError::Plaintext);
        }
        Self::split(&payload.data).map(|(h, _)| h)
    }

    /// En-tête et reste des octets (`nonce ‖ chiffré`).
    fn split(data: &[u8]) -> Result<(Self, &[u8])> {
        let (&scheme, rest) = data.split_first().ok_or(CryptoError::Malformed("vide"))?;
        let take = |n: usize| {
            (rest.len() >= n).then(|| rest.split_at(n)).ok_or(CryptoError::Malformed("tronqué"))
        };
        let (header, rest) = match scheme {
            SCHEME_GROUP => {
                let (h, rest) = take(4)?;
                let epoch =
                    u32::from_be_bytes(h.try_into().map_err(|_| CryptoError::Malformed("époque"))?);
                (Self::Group { epoch }, rest)
            }
            SCHEME_PAIRING => {
                let (h, rest) = take(8)?;
                let token_id = h.try_into().map_err(|_| CryptoError::Malformed("jeton"))?;
                (Self::Pairing { token_id }, rest)
            }
            SCHEME_DEVICE => {
                let (h, rest) = take(32)?;
                let ephemeral = h.try_into().map_err(|_| CryptoError::Malformed("clé éphémère"))?;
                (Self::Device { ephemeral }, rest)
            }
            _ => return Err(CryptoError::Malformed("schéma inconnu")),
        };
        if rest.len() < NONCE_LEN + SIG_LEN + 16 {
            return Err(CryptoError::Malformed("tronqué"));
        }
        Ok((header, rest))
    }
}

/// Clé de chiffrement d'un message sortant.
#[derive(Debug, Clone, Copy)]
pub enum SealKey<'a> {
    Group {
        epoch: u32,
        key: &'a GroupKey,
    },
    Pairing {
        token_id: [u8; 8],
        secret: &'a PairingSecret,
    },
    /// Clé X25519 publique du destinataire.
    Device {
        recipient: [u8; 32],
    },
}

/// Clé d'ouverture, choisie par l'appelant d'après le [`Header`].
#[derive(Debug, Clone, Copy)]
pub enum OpenKey<'a> {
    Group(&'a GroupKey),
    Pairing(&'a PairingSecret),
    Device(&'a ExchangeKeyPair),
}

/// Aléa d'un chiffrement ; fixé seulement par les vecteurs de test.
#[derive(Clone)]
pub struct Randomness {
    pub nonce: [u8; NONCE_LEN],
    /// Clé X25519 éphémère (schéma `0x03` seulement).
    pub ephemeral: Zeroizing<[u8; 32]>,
}

impl Randomness {
    fn generate() -> Self {
        let mut r = Self { nonce: [0; NONCE_LEN], ephemeral: Zeroizing::new([0; 32]) };
        random(&mut r.nonce);
        random(&mut r.ephemeral[..]);
        r
    }
}

/// Données associées : `"ot-env-v1" ‖ en-tête ‖ msg_id ‖ from ‖ to (séparés
/// par `,`) ‖ created_at (RFC 3339)`. Le `payload` de l'enveloppe est ignoré.
pub fn associated_data(header: &Header, env: &Envelope) -> Result<Vec<u8>> {
    let created_at =
        env.created_at.format(&Rfc3339).map_err(|_| CryptoError::Malformed("created_at"))?;
    let to: Vec<&str> = env.to.iter().map(|d| d.as_str()).collect();
    let mut ad = Vec::with_capacity(128);
    ad.extend_from_slice(ENV_DOMAIN);
    ad.extend_from_slice(&header.encode());
    ad.extend_from_slice(env.msg_id.to_string().as_bytes());
    ad.extend_from_slice(env.from.as_str().as_bytes());
    ad.extend_from_slice(to.join(",").as_bytes());
    ad.extend_from_slice(created_at.as_bytes());
    Ok(ad)
}

fn signed_message(ad: &[u8], body: &[u8]) -> Vec<u8> {
    [SIG_DOMAIN, ad, body].concat()
}

fn hkdf(ikm: &[u8], info: &[&[u8]]) -> Zeroizing<[u8; 32]> {
    let mut okm = Zeroizing::new([0; 32]);
    // Invariant : 32 octets est bien en deçà de la limite de HKDF-SHA256.
    Hkdf::<Sha256>::new(None, ikm)
        .expand_multi_info(info, &mut okm[..])
        .expect("32 octets sont toujours dérivables");
    okm
}

fn pairing_key(secret: &PairingSecret) -> Zeroizing<[u8; 32]> {
    hkdf(secret.as_bytes(), &[PAIR_INFO])
}

fn device_key(
    secret: &StaticSecret,
    peer: &[u8; 32],
    eph: &[u8; 32],
    recipient: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>> {
    let shared = secret.diffie_hellman(&PublicKey::from(*peer));
    if !shared.was_contributory() {
        return Err(CryptoError::WeakKey);
    }
    Ok(hkdf(shared.as_bytes(), &[DEVICE_INFO, eph, recipient]))
}

/// Chiffre et signe `body` pour l'enveloppe `env` (seuls ses champs de
/// routage comptent) ; le résultat va dans `env.payload`.
pub fn seal(env: &Envelope, key: SealKey<'_>, signer: &SigningKey, body: &Body) -> Result<Payload> {
    seal_with(env, key, signer, body, Randomness::generate())
}

/// Comme [`seal`], avec un aléa fixé : réservé aux vecteurs de test.
pub fn seal_with(
    env: &Envelope,
    key: SealKey<'_>,
    signer: &SigningKey,
    body: &Body,
    rand: Randomness,
) -> Result<Payload> {
    let (header, aead_key) = match key {
        SealKey::Group { epoch, key } => (Header::Group { epoch }, Zeroizing::new(*key.as_bytes())),
        SealKey::Pairing { token_id, secret } => {
            (Header::Pairing { token_id }, pairing_key(secret))
        }
        SealKey::Device { recipient } => {
            let eph = StaticSecret::from(*rand.ephemeral);
            let eph_pub = PublicKey::from(&eph).to_bytes();
            let k = device_key(&eph, &recipient, &eph_pub, &recipient)?;
            (Header::Device { ephemeral: eph_pub }, k)
        }
    };
    let ad = associated_data(&header, env)?;
    let json = body.to_json();
    let sig = signer.sign(&signed_message(&ad, &json));

    let mut plain = Zeroizing::new(Vec::with_capacity(SIG_LEN + json.len()));
    plain.extend_from_slice(&sig.to_bytes());
    plain.extend_from_slice(&json);
    let cipher = XChaCha20Poly1305::new(&(*aead_key).into());
    let nonce = XNonce::from(rand.nonce);
    let sealed = cipher
        .encrypt(&nonce, AeadPayload { msg: &plain, aad: &ad })
        .map_err(|_| CryptoError::Malformed("message trop long"))?;

    let mut data = header.encode();
    data.extend_from_slice(&rand.nonce);
    data.extend_from_slice(&sealed);
    Ok(Payload { enc: Encoding::Xchacha20poly1305, data })
}

/// Message déchiffré, dont la signature reste à vérifier avec la clé de
/// l'expéditeur (lue dans le roster, ou dans le corps pour `pair_request`).
pub struct Opened {
    pub header: Header,
    signature: [u8; SIG_LEN],
    signed: Zeroizing<Vec<u8>>,
    body: Zeroizing<Vec<u8>>,
}

impl Opened {
    /// Corps décodé. Ne pas s'y fier avant [`Opened::verify`].
    pub fn body(&self) -> Result<Body> {
        Body::from_json(&self.body)
    }

    /// Vérifie la signature Ed25519 de l'expéditeur (forme stricte).
    pub fn verify(&self, sign_key: &[u8; 32]) -> Result<()> {
        let key = VerifyingKey::from_bytes(sign_key).map_err(|_| CryptoError::BadSignature)?;
        key.verify_strict(&self.signed, &Signature::from_bytes(&self.signature))
            .map_err(|_| CryptoError::BadSignature)
    }
}

/// Déchiffre `env.payload` avec `key`. Toute altération de l'en-tête, du
/// nonce, du chiffré ou des champs de routage fait échouer l'ouverture.
pub fn open(env: &Envelope, key: OpenKey<'_>) -> Result<Opened> {
    if env.payload.enc != Encoding::Xchacha20poly1305 {
        return Err(CryptoError::Plaintext);
    }
    let (header, rest) = Header::split(&env.payload.data)?;
    let aead_key = match (&header, key) {
        (Header::Group { .. }, OpenKey::Group(k)) => Zeroizing::new(*k.as_bytes()),
        (Header::Pairing { .. }, OpenKey::Pairing(s)) => pairing_key(s),
        (Header::Device { ephemeral }, OpenKey::Device(pair)) => {
            device_key(pair.secret(), ephemeral, ephemeral, &pair.public_key())?
        }
        _ => return Err(CryptoError::Decrypt),
    };
    let ad = associated_data(&header, env)?;
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError::Malformed("nonce"))?;
    let cipher = XChaCha20Poly1305::new(&(*aead_key).into());
    let plain = Zeroizing::new(
        cipher
            .decrypt(&XNonce::from(nonce), AeadPayload { msg: sealed, aad: &ad })
            .map_err(|_| CryptoError::Decrypt)?,
    );
    let (sig, body) = plain.split_at(SIG_LEN);
    let signature: [u8; SIG_LEN] =
        sig.try_into().map_err(|_| CryptoError::Malformed("signature"))?;
    Ok(Opened {
        header,
        signature,
        signed: Zeroizing::new(signed_message(&ad, body)),
        body: Zeroizing::new(body.to_vec()),
    })
}
