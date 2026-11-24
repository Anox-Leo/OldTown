use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use ot_proto::{DeviceId, Role, SignedBlob, UserId, b64};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CryptoError, Result, Roster};

pub const CERT_DOMAIN: &[u8] = b"ot-user-cert-v1";
pub const LOG_DOMAIN: &[u8] = b"ot-roster-log-v1";

const OBJECT_VERSION: u8 = 1;

pub const GENESIS_PREV: [u8; 32] = [0; 32];

pub fn sign_object(domain: &[u8], data: Vec<u8>, signer: &SigningKey) -> SignedBlob {
    let sig = signer.sign(&[domain, &data].concat()).to_bytes();
    SignedBlob { data, sig }
}

pub fn verify_object(domain: &[u8], blob: &SignedBlob, sign_key: &[u8; 32]) -> Result<()> {
    let key = VerifyingKey::from_bytes(sign_key).map_err(|_| CryptoError::BadSignature)?;
    key.verify_strict(&[domain, &blob.data].concat(), &Signature::from_bytes(&blob.sig))
        .map_err(|_| CryptoError::BadSignature)
}

fn read<T: DeserializeOwned>(blob: &SignedBlob) -> Result<T> {
    serde_json::from_slice(&blob.data).map_err(|e| CryptoError::Body(e.to_string()))
}

fn check_version(v: u8) -> Result<()> {
    if v == OBJECT_VERSION { Ok(()) } else { Err(CryptoError::Malformed("version d'objet signé")) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Genesis {
    pub device_id: DeviceId,
    pub name: String,
    #[serde(with = "b64::array")]
    pub sign_key: [u8; 32],
    #[serde(with = "b64::array")]
    pub exchange_key: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserCert {
    pub v: u8,
    pub user: UserId,
    pub name: String,
    pub role: Role,
    pub genesis: Genesis,
    pub issued_at: i64,
    pub issuer: DeviceId,
}

impl UserCert {
    pub fn new(
        user: UserId,
        name: impl Into<String>,
        role: Role,
        genesis: Genesis,
        issued_at: i64,
        issuer: DeviceId,
    ) -> Self {
        Self { v: OBJECT_VERSION, user, name: name.into(), role, genesis, issued_at, issuer }
    }

    pub fn sign(&self, signer: &SigningKey) -> Result<SignedBlob> {
        check_version(self.v)?;
        let data = serde_json::to_vec(self).map_err(|e| CryptoError::Body(e.to_string()))?;
        Ok(sign_object(CERT_DOMAIN, data, signer))
    }

    pub fn read_unverified(blob: &SignedBlob) -> Result<Self> {
        let cert: Self = read(blob)?;
        check_version(cert.v)?;
        Ok(cert)
    }

    pub fn open(blob: &SignedBlob, issuer_key: &[u8; 32]) -> Result<Self> {
        verify_object(CERT_DOMAIN, blob, issuer_key)?;
        Self::read_unverified(blob)
    }

    pub fn is_root(&self) -> bool {
        self.issuer == self.genesis.device_id
    }
}

pub fn cert_hash(cert: &SignedBlob) -> [u8; 32] {
    Sha256::digest(&cert.data).into()
}

pub fn anchor(admin_cert: &SignedBlob) -> [u8; 16] {
    let mut out = [0; 16];
    out.copy_from_slice(&Sha256::digest(&admin_cert.data)[..16]);
    out
}

pub fn verify_root(admin_cert: &SignedBlob, expected: &[u8; 16]) -> Result<UserCert> {
    if &anchor(admin_cert) != expected {
        return Err(CryptoError::Malformed("ancre de l'annuaire différente"));
    }
    let unverified = UserCert::read_unverified(admin_cert)?;
    if !unverified.is_root() || unverified.role != Role::Admin {
        return Err(CryptoError::Malformed("certificat racine invalide"));
    }
    UserCert::open(admin_cert, &unverified.genesis.sign_key)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertRef {
    pub user: UserId,
    #[serde(with = "b64::array")]
    pub cert: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogHead {
    pub seq: u64,
    #[serde(with = "b64::array")]
    pub hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterEntry {
    pub v: u8,
    pub user: UserId,
    pub seq: u64,
    #[serde(with = "b64::array")]
    pub prev: [u8; 32],
    pub signer: DeviceId,
    pub roster: Roster,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub certs: Vec<CertRef>,
}

impl RosterEntry {
    pub fn new(user: UserId, seq: u64, prev: [u8; 32], signer: DeviceId, roster: Roster) -> Self {
        Self { v: OBJECT_VERSION, user, seq, prev, signer, roster, certs: Vec::new() }
    }

    pub fn with_certs(mut self, mut certs: Vec<CertRef>) -> Self {
        certs.sort_by(|a, b| a.user.cmp(&b.user));
        self.certs = certs;
        self
    }

    pub fn cert_of(&self, user: &UserId) -> Option<[u8; 32]> {
        self.certs.iter().find(|c| &c.user == user).map(|c| c.cert)
    }

    fn check(&self) -> Result<()> {
        check_version(self.v)?;
        if self.seq == 0 || (self.seq == 1) != (self.prev == GENESIS_PREV) {
            return Err(CryptoError::Malformed("seq ou prev incohérents"));
        }
        if self.certs.windows(2).any(|w| w[0].user >= w[1].user) {
            return Err(CryptoError::Malformed("certs non triés ou en double"));
        }
        Ok(())
    }

    pub fn sign(&self, signer: &SigningKey) -> Result<SignedBlob> {
        self.check()?;
        let data = serde_json::to_vec(self).map_err(|e| CryptoError::Body(e.to_string()))?;
        Ok(sign_object(LOG_DOMAIN, data, signer))
    }

    pub fn read_unverified(blob: &SignedBlob) -> Result<Self> {
        let entry: Self = read(blob)?;
        entry.check()?;
        Ok(entry)
    }

    pub fn open(blob: &SignedBlob, signer_key: &[u8; 32]) -> Result<Self> {
        verify_object(LOG_DOMAIN, blob, signer_key)?;
        Self::read_unverified(blob)
    }
}

pub fn entry_hash(entry: &SignedBlob) -> [u8; 32] {
    Sha256::digest(&entry.data).into()
}

pub fn log_head(entry: &SignedBlob) -> Result<LogHead> {
    Ok(LogHead { seq: RosterEntry::read_unverified(entry)?.seq, hash: entry_hash(entry) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExchangeKeyPair, Member};

    fn genesis(key: &SigningKey) -> Genesis {
        Genesis {
            device_id: DeviceId::generate(),
            name: "alice-phone".into(),
            sign_key: key.verifying_key().to_bytes(),
            exchange_key: ExchangeKeyPair::generate().public_key(),
        }
    }

    #[test]
    fn root_certificate_is_checked_against_its_anchor() {
        let key = SigningKey::from_bytes(&[1; 32]);
        let g = genesis(&key);
        let cert = UserCert::new(
            UserId::generate(),
            "Alice",
            Role::Admin,
            g.clone(),
            1,
            g.device_id.clone(),
        );
        let blob = cert.sign(&key).unwrap();
        let a = anchor(&blob);
        assert_eq!(verify_root(&blob, &a).unwrap(), cert);
        assert!(verify_root(&blob, &[0; 16]).is_err(), "autre ancre");

        let mut tampered = blob.clone();
        tampered.data.push(b' ');
        assert!(verify_root(&tampered, &a).is_err());
        assert!(verify_root(&tampered, &anchor(&tampered)).is_err());

        let member =
            UserCert::new(UserId::generate(), "Bob", Role::Member, g.clone(), 1, g.device_id);
        let blob = member.sign(&key).unwrap();
        assert!(verify_root(&blob, &anchor(&blob)).is_err());
    }

    #[test]
    fn certificates_need_the_issuer_key_and_the_domain() {
        let (admin, other) = (SigningKey::from_bytes(&[1; 32]), SigningKey::from_bytes(&[2; 32]));
        let cert = UserCert::new(
            UserId::generate(),
            "Bob",
            Role::Member,
            genesis(&other),
            1,
            DeviceId::generate(),
        );
        let blob = cert.sign(&admin).unwrap();
        assert_eq!(UserCert::open(&blob, &admin.verifying_key().to_bytes()).unwrap(), cert);
        assert_eq!(
            UserCert::open(&blob, &other.verifying_key().to_bytes()),
            Err(CryptoError::BadSignature)
        );
        assert_eq!(
            verify_object(LOG_DOMAIN, &blob, &admin.verifying_key().to_bytes()),
            Err(CryptoError::BadSignature)
        );
        let mut extra: serde_json::Value = serde_json::from_slice(&blob.data).unwrap();
        extra["x"] = 1.into();
        let extra = sign_object(CERT_DOMAIN, serde_json::to_vec(&extra).unwrap(), &admin);
        assert!(UserCert::open(&extra, &admin.verifying_key().to_bytes()).is_err());
        let v2 = UserCert { v: 2, ..cert };
        assert!(v2.sign(&admin).is_err());
    }

    #[test]
    fn roster_entries_chain_by_hash() {
        let key = SigningKey::from_bytes(&[3; 32]);
        let member = Member {
            device_id: DeviceId::generate(),
            name: "bob-phone".into(),
            sign_key: key.verifying_key().to_bytes(),
            exchange_key: [9; 32],
            added_epoch: 1,
        };
        let roster = Roster { epoch: 1, version: 1, members: vec![member.clone()] };
        let user = UserId::generate();
        let first =
            RosterEntry::new(user.clone(), 1, GENESIS_PREV, member.device_id.clone(), roster);
        let b1 = first.sign(&key).unwrap();
        assert_eq!(RosterEntry::open(&b1, &key.verifying_key().to_bytes()).unwrap(), first);
        let roster2 = Roster { epoch: 1, version: 2, members: vec![member.clone()] };
        let second =
            RosterEntry::new(user.clone(), 2, entry_hash(&b1), member.device_id.clone(), roster2);
        let b2 = second.sign(&key).unwrap();
        assert_eq!(RosterEntry::read_unverified(&b2).unwrap().prev, entry_hash(&b1));
        let bad = RosterEntry { prev: GENESIS_PREV, ..second };
        assert!(bad.sign(&key).is_err());
        let bad = RosterEntry { prev: [1; 32], ..first.clone() };
        assert!(bad.sign(&key).is_err());
    }

    #[test]
    fn admin_entries_commit_to_certificates() {
        let key = SigningKey::from_bytes(&[4; 32]);
        let roster = Roster { epoch: 1, version: 2, members: vec![] };
        let (a, b) = (UserId::generate(), UserId::generate());
        let refs = vec![
            CertRef { user: b.clone(), cert: [2; 32] },
            CertRef { user: a.clone(), cert: [1; 32] },
        ];
        let entry = RosterEntry::new(UserId::generate(), 2, [9; 32], DeviceId::generate(), roster)
            .with_certs(refs);
        let blob = entry.sign(&key).unwrap();
        let read = RosterEntry::open(&blob, &key.verifying_key().to_bytes()).unwrap();
        assert_eq!(read.cert_of(&a), Some([1; 32]));
        assert_eq!(read.cert_of(&b), Some([2; 32]));
        assert_eq!(log_head(&blob).unwrap(), LogHead { seq: 2, hash: entry_hash(&blob) });

        let mut json: serde_json::Value = serde_json::from_slice(&blob.data).unwrap();
        let certs = json["certs"].as_array().unwrap().clone();
        json["certs"] = serde_json::Value::Array(vec![certs[0].clone(), certs[0].clone()]);
        let dup = sign_object(LOG_DOMAIN, serde_json::to_vec(&json).unwrap(), &key);
        assert!(RosterEntry::open(&dup, &key.verifying_key().to_bytes()).is_err());
        json["certs"] = serde_json::Value::Array(vec![certs[1].clone(), certs[0].clone()]);
        let swapped = sign_object(LOG_DOMAIN, serde_json::to_vec(&json).unwrap(), &key);
        assert!(RosterEntry::open(&swapped, &key.verifying_key().to_bytes()).is_err());

        let plain = RosterEntry { certs: Vec::new(), ..entry };
        assert!(!String::from_utf8(plain.sign(&key).unwrap().data).unwrap().contains("certs"));
    }
}
