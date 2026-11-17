//! Objets signés de l'annuaire (étape 10, note 0052) : certificat de
//! personne et entrée du journal de roster.
//!
//! Un objet signé est un [`SignedBlob`] : `data` est le JSON exact, `sig` la
//! signature Ed25519 de `domaine ‖ data`. Aucune canonisation : on vérifie
//! les octets reçus, puis on les lit (`deny_unknown_fields`). Les règles de
//! chaîne (qui peut signer quelle entrée, à partir de quel roster) sont dans
//! `ot-core`, qui tient les rosters ; ici, seulement les formats.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use ot_proto::{DeviceId, Role, SignedBlob, UserId, b64};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CryptoError, Result, Roster};

/// Domaine de la signature d'un certificat de personne.
pub const CERT_DOMAIN: &[u8] = b"ot-user-cert-v1";
/// Domaine de la signature d'une entrée du journal de roster.
pub const LOG_DOMAIN: &[u8] = b"ot-roster-log-v1";

/// Version des objets signés de l'annuaire.
const OBJECT_VERSION: u8 = 1;

/// `prev` de la première entrée d'un journal.
pub const GENESIS_PREV: [u8; 32] = [0; 32];

/// Signe `data` (le JSON exact) sous `domain`.
pub fn sign_object(domain: &[u8], data: Vec<u8>, signer: &SigningKey) -> SignedBlob {
    let sig = signer.sign(&[domain, &data].concat()).to_bytes();
    SignedBlob { data, sig }
}

/// Vérifie la signature de `blob` sous `domain` par `sign_key` (forme
/// stricte).
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

/// Premier appareil d'une personne, lié à elle par son certificat.
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

/// Certificat de personne, signé par un appareil de l'admin (domaine
/// `"ot-user-cert-v1"`). Celui de l'admin, la racine, est signé par son
/// propre `genesis`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserCert {
    pub v: u8,
    pub user: UserId,
    pub name: String,
    pub role: Role,
    pub genesis: Genesis,
    /// Secondes Unix.
    pub issued_at: i64,
    /// Appareil de l'admin qui a signé.
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

    /// Signe le certificat ; `signer` doit être la clé de `issuer`.
    pub fn sign(&self, signer: &SigningKey) -> Result<SignedBlob> {
        check_version(self.v)?;
        let data = serde_json::to_vec(self).map_err(|e| CryptoError::Body(e.to_string()))?;
        Ok(sign_object(CERT_DOMAIN, data, signer))
    }

    /// Lit un certificat **sans le vérifier**, pour savoir qui l'a signé.
    /// Rien de ce qu'il dit ne vaut avant [`UserCert::open`].
    pub fn read_unverified(blob: &SignedBlob) -> Result<Self> {
        let cert: Self = read(blob)?;
        check_version(cert.v)?;
        Ok(cert)
    }

    /// Vérifie la signature par `issuer_key` (clé de `issuer`, lue dans le
    /// roster vérifié de l'admin), puis lit le certificat.
    pub fn open(blob: &SignedBlob, issuer_key: &[u8; 32]) -> Result<Self> {
        verify_object(CERT_DOMAIN, blob, issuer_key)?;
        Self::read_unverified(blob)
    }

    /// Certificat racine : signé par son propre premier appareil.
    pub fn is_root(&self) -> bool {
        self.issuer == self.genesis.device_id
    }
}

/// Ancre de l'annuaire : `sha256(data du certificat de l'admin)[..16]`,
/// portée par chaque code d'invitation.
pub fn anchor(admin_cert: &SignedBlob) -> [u8; 16] {
    let mut out = [0; 16];
    out.copy_from_slice(&Sha256::digest(&admin_cert.data)[..16]);
    out
}

/// Vérifie le certificat racine contre l'ancre : empreinte, rôle `admin`,
/// auto-signé par son `genesis`. Il ne dépend d'aucun roster : retirer plus
/// tard l'appareil `genesis` du groupe de l'admin ne le change pas (l'ancre
/// est figée dans les codes déjà donnés).
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

/// Entrée du journal de roster d'une personne (domaine
/// `"ot-roster-log-v1"`), signée par un membre du roster de l'entrée
/// précédente (le `genesis` du certificat pour la première ; un appareil de
/// l'admin pour une personne de service).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RosterEntry {
    pub v: u8,
    pub user: UserId,
    pub seq: u64,
    /// `sha256(data de l'entrée seq-1)`, [`GENESIS_PREV`] pour `seq = 1`.
    #[serde(with = "b64::array")]
    pub prev: [u8; 32],
    pub signer: DeviceId,
    /// Roster de la note 0021 ; époque 0 pour un service (pas de clé de
    /// groupe).
    pub roster: Roster,
}

impl RosterEntry {
    pub fn new(user: UserId, seq: u64, prev: [u8; 32], signer: DeviceId, roster: Roster) -> Self {
        Self { v: OBJECT_VERSION, user, seq, prev, signer, roster }
    }

    pub fn sign(&self, signer: &SigningKey) -> Result<SignedBlob> {
        check_version(self.v)?;
        if self.seq == 0 || (self.seq == 1) != (self.prev == GENESIS_PREV) {
            return Err(CryptoError::Malformed("seq ou prev incohérents"));
        }
        let data = serde_json::to_vec(self).map_err(|e| CryptoError::Body(e.to_string()))?;
        Ok(sign_object(LOG_DOMAIN, data, signer))
    }

    /// Lit une entrée **sans la vérifier**, pour savoir qui l'a signée.
    pub fn read_unverified(blob: &SignedBlob) -> Result<Self> {
        let entry: Self = read(blob)?;
        check_version(entry.v)?;
        if entry.seq == 0 || (entry.seq == 1) != (entry.prev == GENESIS_PREV) {
            return Err(CryptoError::Malformed("seq ou prev incohérents"));
        }
        Ok(entry)
    }

    /// Vérifie la signature par `signer_key`, puis lit l'entrée.
    pub fn open(blob: &SignedBlob, signer_key: &[u8; 32]) -> Result<Self> {
        verify_object(LOG_DOMAIN, blob, signer_key)?;
        Self::read_unverified(blob)
    }
}

/// `sha256(data)` d'une entrée : le `prev` de la suivante.
pub fn entry_hash(entry: &SignedBlob) -> [u8; 32] {
    Sha256::digest(&entry.data).into()
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

        // Un octet de plus dans `data` change l'ancre et casse la signature.
        let mut tampered = blob.clone();
        tampered.data.push(b' ');
        assert!(verify_root(&tampered, &a).is_err());
        assert!(verify_root(&tampered, &anchor(&tampered)).is_err());

        // Un membre n'est pas une racine, même auto-signé.
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
        // La même signature ne vaut pas pour une entrée de journal.
        assert_eq!(
            verify_object(LOG_DOMAIN, &blob, &admin.verifying_key().to_bytes()),
            Err(CryptoError::BadSignature)
        );
        // Champ inconnu ou version inconnue : refusés, même bien signés.
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
        // `prev` nul hors de la première entrée, ou non nul pour elle : refusé.
        let bad = RosterEntry { prev: GENESIS_PREV, ..second };
        assert!(bad.sign(&key).is_err());
        let bad = RosterEntry { prev: [1; 32], ..first };
        assert!(bad.sign(&key).is_err());
    }
}
