use chacha20poly1305::aead::{Aead, KeyInit, Payload as AeadPayload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use ot_proto::{DeviceId, Encoding, Envelope, MAX_RECIPIENTS, Payload, Ulid};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::{Body, CryptoError, ExchangeKeyPair, GroupKey, PairingSecret, Result, random};

pub const NONCE_LEN: usize = 24;
const SIG_LEN: usize = 64;

const ENV_DOMAIN: &[u8] = b"ot-env-v1";
const SIG_DOMAIN: &[u8] = b"ot-sig-v1";
const PAIR_INFO: &[u8] = b"ot-pair-v1";
const DEVICE_INFO: &[u8] = b"ot-device-v1";
const PAIR_QR_INFO: &[u8] = b"ot-pair-qr-v1";
const INVITE_INFO: &[u8] = b"ot-invite-v1";
const MULTI_INFO: &[u8] = b"ot-multi-v1";
const MULTI_COMMIT: &[u8] = b"ot-multi-commit-v1";

const SCHEME_GROUP: u8 = 0x01;
const SCHEME_PAIRING: u8 = 0x02;
const SCHEME_DEVICE: u8 = 0x03;
const SCHEME_PAIR_QR: u8 = 0x04;
const SCHEME_MULTI: u8 = 0x05;

pub const WRAP_LEN: usize = 32 + 16;

const WRAP_NONCE: [u8; NONCE_LEN] = [0; NONCE_LEN];

pub const COMMIT_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Header {
    Group {
        epoch: u32,
    },
    Pairing {
        token_id: [u8; 8],
    },
    Device {
        ephemeral: [u8; 32],
    },
    PairQr {
        ephemeral: [u8; 32],
    },
    Multi {
        ephemeral: [u8; 32],
        recipients: Vec<([u8; 16], [u8; WRAP_LEN])>,
        commitment: [u8; COMMIT_LEN],
    },
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
            Self::PairQr { ephemeral } => {
                out.push(SCHEME_PAIR_QR);
                out.extend_from_slice(ephemeral);
            }
            Self::Multi { ephemeral, recipients, commitment } => {
                out.reserve(1 + recipients.len() * (16 + WRAP_LEN) + COMMIT_LEN);
                out.push(SCHEME_MULTI);
                out.extend_from_slice(ephemeral);
                out.push(recipients.len() as u8);
                for (ulid, wrap) in recipients {
                    out.extend_from_slice(ulid);
                    out.extend_from_slice(wrap);
                }
                out.extend_from_slice(commitment);
            }
        }
        out
    }

    pub fn recipients(&self) -> Vec<DeviceId> {
        match self {
            Self::Multi { recipients, .. } => {
                recipients.iter().map(|(u, _)| DeviceId::from_ulid(Ulid::from_bytes(*u))).collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn parse(payload: &Payload) -> Result<Self> {
        if payload.enc != Encoding::Xchacha20poly1305 {
            return Err(CryptoError::Plaintext);
        }
        Self::split(&payload.data).map(|(h, _)| h)
    }

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
            SCHEME_PAIR_QR => {
                let (h, rest) = take(32)?;
                let ephemeral = h.try_into().map_err(|_| CryptoError::Malformed("clé éphémère"))?;
                (Self::PairQr { ephemeral }, rest)
            }
            SCHEME_MULTI => {
                let (h, rest) = take(33)?;
                let ephemeral =
                    h[..32].try_into().map_err(|_| CryptoError::Malformed("clé éphémère"))?;
                let n = usize::from(h[32]);
                if n == 0 || n > MAX_RECIPIENTS {
                    return Err(CryptoError::Malformed("nombre de destinataires"));
                }
                let entry = 16 + WRAP_LEN;
                if rest.len() < n * entry {
                    return Err(CryptoError::Malformed("tronqué"));
                }
                let (table, rest) = rest.split_at(n * entry);
                let mut recipients: Vec<([u8; 16], [u8; WRAP_LEN])> = Vec::with_capacity(n);
                for e in table.chunks_exact(entry) {
                    let ulid: [u8; 16] =
                        e[..16].try_into().map_err(|_| CryptoError::Malformed("destinataire"))?;
                    if recipients.iter().any(|(u, _)| *u == ulid) {
                        return Err(CryptoError::Malformed("destinataire en double"));
                    }
                    let wrap = e[16..].try_into().map_err(|_| CryptoError::Malformed("clé"))?;
                    recipients.push((ulid, wrap));
                }
                if rest.len() < COMMIT_LEN {
                    return Err(CryptoError::Malformed("tronqué"));
                }
                let (c, rest) = rest.split_at(COMMIT_LEN);
                let commitment = c.try_into().map_err(|_| CryptoError::Malformed("engagement"))?;
                (Self::Multi { ephemeral, recipients, commitment }, rest)
            }
            _ => return Err(CryptoError::Malformed("schéma inconnu")),
        };
        if rest.len() < NONCE_LEN + SIG_LEN + 16 {
            return Err(CryptoError::Malformed("tronqué"));
        }
        Ok((header, rest))
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SealKey<'a> {
    Group { epoch: u32, key: &'a GroupKey },
    Pairing { token_id: [u8; 8], secret: &'a PairingSecret },
    Device { recipient: [u8; 32] },
    PairQr { recipient: [u8; 32], secret: &'a PairingSecret },
    Invite { token_id: [u8; 8], secret: &'a PairingSecret },
    Multi { recipients: &'a [(DeviceId, [u8; 32])] },
}

#[derive(Debug, Clone, Copy)]
pub enum OpenKey<'a> {
    Group(&'a GroupKey),
    Pairing(&'a PairingSecret),
    Device(&'a ExchangeKeyPair),
    PairQr(&'a ExchangeKeyPair, &'a PairingSecret),
    Invite(&'a PairingSecret),
    Multi { me: &'a DeviceId, key: &'a ExchangeKeyPair },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Group,
    Pairing,
    Device,
    PairQr,
    Invite,
    Multi,
}

impl KeyKind {
    fn admits(self, body: &Body) -> bool {
        match body {
            Body::InviteRequest { .. } | Body::InviteWelcome { .. } => self == Self::Invite,
            Body::ServiceWelcome { .. } => self == Self::PairQr,
            Body::Text { .. } | Body::Link { .. } | Body::File { .. } | Body::Otp(_) => {
                self != Self::Invite
            }
            Body::PairRequest { .. }
            | Body::PairWelcome { .. }
            | Body::Roster { .. }
            | Body::GroupKey { .. } => !matches!(self, Self::Invite | Self::Multi),
        }
    }
}

#[derive(Clone)]
pub struct Randomness {
    pub nonce: [u8; NONCE_LEN],
    pub ephemeral: Zeroizing<[u8; 32]>,
    pub message_key: Zeroizing<[u8; 32]>,
}

impl Randomness {
    fn generate() -> Self {
        let mut r = Self {
            nonce: [0; NONCE_LEN],
            ephemeral: Zeroizing::new([0; 32]),
            message_key: Zeroizing::new([0; 32]),
        };
        random(&mut r.nonce);
        random(&mut r.ephemeral[..]);
        random(&mut r.message_key[..]);
        r
    }
}

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
    Hkdf::<Sha256>::new(None, ikm)
        .expand_multi_info(info, &mut okm[..])
        .expect("32 octets sont toujours dérivables");
    okm
}

fn pairing_key(secret: &PairingSecret) -> Zeroizing<[u8; 32]> {
    hkdf(secret.as_bytes(), &[PAIR_INFO])
}

fn invite_key(secret: &PairingSecret) -> Zeroizing<[u8; 32]> {
    hkdf(secret.as_bytes(), &[INVITE_INFO])
}

fn multi_key(
    secret: &StaticSecret,
    peer: &[u8; 32],
    eph: &[u8; 32],
    recipient: &[u8; 32],
    ulid: &[u8; 16],
    msg_id: &[u8; 16],
) -> Result<Zeroizing<[u8; 32]>> {
    let shared = secret.diffie_hellman(&PublicKey::from(*peer));
    if !shared.was_contributory() {
        return Err(CryptoError::WeakKey);
    }
    Ok(hkdf(shared.as_bytes(), &[MULTI_INFO, eph, recipient, ulid, msg_id]))
}

fn multi_commitment(message_key: &[u8; 32]) -> [u8; COMMIT_LEN] {
    Sha256::new().chain_update(MULTI_COMMIT).chain_update(message_key).finalize().into()
}

fn check_multi_recipients(header: &Header, env: &Envelope) -> Result<()> {
    if env.to.is_empty() || header.recipients() != env.to {
        return Err(CryptoError::Malformed("destinataires de 0x05 différents de env.to"));
    }
    Ok(())
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

fn pair_qr_key(
    secret: &StaticSecret,
    peer: &[u8; 32],
    eph: &[u8; 32],
    recipient: &[u8; 32],
    pairing: &PairingSecret,
) -> Result<Zeroizing<[u8; 32]>> {
    let shared = secret.diffie_hellman(&PublicKey::from(*peer));
    if !shared.was_contributory() {
        return Err(CryptoError::WeakKey);
    }
    let mut ikm = Zeroizing::new([0u8; 48]);
    ikm[..32].copy_from_slice(shared.as_bytes());
    ikm[32..].copy_from_slice(pairing.as_bytes());
    Ok(hkdf(&ikm[..], &[PAIR_QR_INFO, eph, recipient]))
}

pub fn seal(env: &Envelope, key: SealKey<'_>, signer: &SigningKey, body: &Body) -> Result<Payload> {
    seal_with(env, key, signer, body, Randomness::generate())
}

pub fn seal_with(
    env: &Envelope,
    key: SealKey<'_>,
    signer: &SigningKey,
    body: &Body,
    rand: Randomness,
) -> Result<Payload> {
    let kind = match key {
        SealKey::Group { .. } => KeyKind::Group,
        SealKey::Pairing { .. } => KeyKind::Pairing,
        SealKey::Device { .. } => KeyKind::Device,
        SealKey::PairQr { .. } => KeyKind::PairQr,
        SealKey::Invite { .. } => KeyKind::Invite,
        SealKey::Multi { .. } => KeyKind::Multi,
    };
    if !kind.admits(body) {
        return Err(CryptoError::Malformed("corps interdit sous cette clé"));
    }
    let (header, aead_key) = match key {
        SealKey::Group { epoch, key } => (Header::Group { epoch }, Zeroizing::new(*key.as_bytes())),
        SealKey::Pairing { token_id, secret } => {
            (Header::Pairing { token_id }, pairing_key(secret))
        }
        SealKey::Invite { token_id, secret } => (Header::Pairing { token_id }, invite_key(secret)),
        SealKey::Multi { recipients } => {
            if recipients.is_empty() || recipients.len() > MAX_RECIPIENTS {
                return Err(CryptoError::Malformed("nombre de destinataires"));
            }
            let eph = StaticSecret::from(*rand.ephemeral);
            let eph_pub = PublicKey::from(&eph).to_bytes();
            let message_key = Zeroizing::new(*rand.message_key);
            let msg_id = env.msg_id.to_bytes();
            let mut table = Vec::with_capacity(recipients.len());
            for (device, x) in recipients {
                let ulid = device.ulid().to_bytes();
                if table.iter().any(|(u, _)| *u == ulid) {
                    return Err(CryptoError::Malformed("destinataire en double"));
                }
                let k = multi_key(&eph, x, &eph_pub, x, &ulid, &msg_id)?;
                let wrapped = XChaCha20Poly1305::new(&(*k).into())
                    .encrypt(&XNonce::from(WRAP_NONCE), &message_key[..])
                    .map_err(|_| CryptoError::Malformed("clé de message"))?;
                let wrap: [u8; WRAP_LEN] =
                    wrapped.try_into().map_err(|_| CryptoError::Malformed("clé de message"))?;
                table.push((ulid, wrap));
            }
            let commitment = multi_commitment(&message_key);
            let header = Header::Multi { ephemeral: eph_pub, recipients: table, commitment };
            check_multi_recipients(&header, env)?;
            (header, message_key)
        }
        SealKey::Device { recipient } => {
            let eph = StaticSecret::from(*rand.ephemeral);
            let eph_pub = PublicKey::from(&eph).to_bytes();
            let k = device_key(&eph, &recipient, &eph_pub, &recipient)?;
            (Header::Device { ephemeral: eph_pub }, k)
        }
        SealKey::PairQr { recipient, secret } => {
            let eph = StaticSecret::from(*rand.ephemeral);
            let eph_pub = PublicKey::from(&eph).to_bytes();
            let k = pair_qr_key(&eph, &recipient, &eph_pub, &recipient, secret)?;
            (Header::PairQr { ephemeral: eph_pub }, k)
        }
    };
    let ad = associated_data(&header, env)?;
    let json = body.to_json();
    let signed = Zeroizing::new(signed_message(&ad, &json));
    let sig = signer.sign(&signed);

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

pub struct Opened {
    pub header: Header,
    kind: KeyKind,
    signature: [u8; SIG_LEN],
    signed: Zeroizing<Vec<u8>>,
    body: Zeroizing<Vec<u8>>,
}

impl Opened {
    pub fn body(&self) -> Result<Body> {
        let body = Body::from_json(&self.body)?;
        if !self.kind.admits(&body) {
            return Err(CryptoError::Body("corps interdit sous cette clé".into()));
        }
        Ok(body)
    }

    pub fn verify(&self, sign_key: &[u8; 32]) -> Result<()> {
        let key = VerifyingKey::from_bytes(sign_key).map_err(|_| CryptoError::BadSignature)?;
        key.verify_strict(&self.signed, &Signature::from_bytes(&self.signature))
            .map_err(|_| CryptoError::BadSignature)
    }
}

pub fn open(env: &Envelope, key: OpenKey<'_>) -> Result<Opened> {
    if env.payload.enc != Encoding::Xchacha20poly1305 {
        return Err(CryptoError::Plaintext);
    }
    let (header, rest) = Header::split(&env.payload.data)?;
    let (kind, aead_key) = match (&header, key) {
        (Header::Group { .. }, OpenKey::Group(k)) => {
            (KeyKind::Group, Zeroizing::new(*k.as_bytes()))
        }
        (Header::Pairing { .. }, OpenKey::Pairing(s)) => (KeyKind::Pairing, pairing_key(s)),
        (Header::Pairing { .. }, OpenKey::Invite(s)) => (KeyKind::Invite, invite_key(s)),
        (Header::Device { ephemeral }, OpenKey::Device(pair)) => {
            (KeyKind::Device, device_key(pair.secret(), ephemeral, ephemeral, &pair.public_key())?)
        }
        (Header::PairQr { ephemeral }, OpenKey::PairQr(pair, secret)) => (
            KeyKind::PairQr,
            pair_qr_key(pair.secret(), ephemeral, ephemeral, &pair.public_key(), secret)?,
        ),
        (Header::Multi { ephemeral, recipients, .. }, OpenKey::Multi { me, key }) => {
            check_multi_recipients(&header, env)?;
            let ulid = me.ulid().to_bytes();
            let (_, wrap) =
                recipients.iter().find(|(u, _)| *u == ulid).ok_or(CryptoError::Decrypt)?;
            let mine = key.public_key();
            let msg_id = env.msg_id.to_bytes();
            let k = multi_key(key.secret(), ephemeral, ephemeral, &mine, &ulid, &msg_id)?;
            let unwrapped = Zeroizing::new(
                XChaCha20Poly1305::new(&(*k).into())
                    .decrypt(&XNonce::from(WRAP_NONCE), &wrap[..])
                    .map_err(|_| CryptoError::Decrypt)?,
            );
            let message_key: [u8; 32] =
                unwrapped[..].try_into().map_err(|_| CryptoError::Decrypt)?;
            (KeyKind::Multi, Zeroizing::new(message_key))
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
    if let Header::Multi { commitment, .. } = &header
        && multi_commitment(&aead_key) != *commitment
    {
        return Err(CryptoError::Decrypt);
    }
    let (sig, body) = plain.split_at(SIG_LEN);
    let signature: [u8; SIG_LEN] =
        sig.try_into().map_err(|_| CryptoError::Malformed("signature"))?;
    Ok(Opened {
        header,
        kind,
        signature,
        signed: Zeroizing::new(signed_message(&ad, body)),
        body: Zeroizing::new(body.to_vec()),
    })
}
