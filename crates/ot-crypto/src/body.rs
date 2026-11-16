//! Corps des messages, sérialisés en JSON dans le clair (après la signature).

use ot_proto::{BlobId, DeviceId, b64};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::file::{FILE_NONCE_LEN, check_layout};
use crate::{CryptoError, FileKey, GroupKey, Result};

/// Corps d'un message chiffré. `text` et `link` gardent le format de
/// l'étape 1 (`{"kind":"text","body":"…"}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    /// Texte libre (schéma `0x01`).
    Text { body: String },
    /// Lien (schéma `0x01`).
    Link { body: String },
    /// Nouvel appareil → parrain (schéma `0x02`) : ses clés publiques.
    PairRequest {
        name: String,
        #[serde(with = "b64::array")]
        sign_key: [u8; 32],
        #[serde(with = "b64::array")]
        exchange_key: [u8; 32],
    },
    /// Parrain → nouvel appareil (schéma `0x02`) : clés de groupe encore
    /// utiles, de la plus ancienne à la courante, et roster complet.
    PairWelcome { keys: Vec<EpochKey>, roster: Roster },
    /// Roster complet, envoyé à tous après un ajout (schéma `0x01`).
    Roster { roster: Roster },
    /// Nouvelle époque après une rotation, pour un seul appareil (schéma
    /// `0x03`) ; le roster ne contient plus les exclus.
    GroupKey { epoch: u32, key: GroupKey, roster: Roster },
    /// Fichier déposé sur le relais (schéma `0x01`, étape 5, note 0026) :
    /// sa référence, ce qu'il faut pour l'ouvrir et le vérifier.
    File {
        blob_id: BlobId,
        /// Nom donné par l'expéditeur ; à nettoyer avant de l'écrire
        /// ([`crate::sanitize_file_name`]).
        name: String,
        mime: String,
        /// Taille en clair, en octets.
        size: u64,
        chunk_size: u32,
        chunks: u32,
        key: FileKey,
        /// Préfixe de nonce STREAM.
        #[serde(with = "b64::array")]
        nonce: [u8; FILE_NONCE_LEN],
        /// SHA-256 du clair.
        #[serde(with = "b64::array")]
        sha256: [u8; 32],
    },
    /// Code à usage unique lu dans un mail (schéma `0x01`, note 0045),
    /// envoyé par le pont `relais-otp`.
    Otp(Otp),
}

/// Code à usage unique et ce qu'il faut pour le placer sur la bonne page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Otp {
    /// Ce qu'il faut taper : 4 à 16 lettres ou chiffres ASCII.
    pub code: String,
    /// Le code tel qu'écrit dans le mail (`G-482913`).
    pub display: String,
    /// Nom affiché de l'expéditeur (`Google`), éventuellement vide.
    pub sender_name: String,
    /// Domaine de l'adresse de l'expéditeur (`google.com`), en minuscules.
    pub sender_domain: String,
    /// Vrai si l'expéditeur est authentifié (DMARC `pass` vu par le
    /// fournisseur de messagerie).
    pub sender_verified: bool,
    /// Fin de validité, en secondes depuis l'époque Unix.
    pub expires_at: i64,
}

impl Otp {
    /// Longueur maximale de `display` et `sender_name`, en caractères.
    pub const MAX_TEXT_CHARS: usize = 128;

    fn check(&self) -> Result<()> {
        let bad = |why: &str| Err(CryptoError::Body(format!("otp : {why}")));
        if !(4..=16).contains(&self.code.len())
            || !self.code.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return bad("code invalide");
        }
        if self.display.chars().count() > Self::MAX_TEXT_CHARS
            || self.sender_name.chars().count() > Self::MAX_TEXT_CHARS
        {
            return bad("texte trop long");
        }
        let domain = &self.sender_domain;
        if domain.is_empty()
            || domain.len() > 253
            || !domain
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        {
            return bad("domaine invalide");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochKey {
    pub epoch: u32,
    pub key: GroupKey,
}

/// Liste des membres du groupe, tenue par les appareils eux-mêmes. Un roster
/// en remplace un autre seulement si `(epoch, version)` est plus grand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    /// Époque de la clé de groupe à laquelle ce roster correspond.
    pub epoch: u32,
    /// Incrémentée à chaque changement de membres.
    pub version: u64,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub device_id: DeviceId,
    pub name: String,
    /// Clé publique Ed25519 (signature des messages).
    #[serde(with = "b64::array")]
    pub sign_key: [u8; 32],
    /// Clé publique X25519 (réception d'une clé de groupe scellée).
    #[serde(with = "b64::array")]
    pub exchange_key: [u8; 32],
    /// Époque à laquelle l'appareil a rejoint le groupe.
    pub added_epoch: u32,
}

impl Roster {
    pub fn member(&self, id: &DeviceId) -> Option<&Member> {
        self.members.iter().find(|m| &m.device_id == id)
    }

    pub fn contains(&self, id: &DeviceId) -> bool {
        self.member(id).is_some()
    }
}

impl Body {
    /// Nom du type, tel qu'écrit dans `kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Link { .. } => "link",
            Self::PairRequest { .. } => "pair_request",
            Self::PairWelcome { .. } => "pair_welcome",
            Self::Roster { .. } => "roster",
            Self::GroupKey { .. } => "group_key",
            Self::File { .. } => "file",
            Self::Otp(_) => "otp",
        }
    }

    /// JSON du corps ; effacé de la mémoire à la destruction (il peut porter
    /// des clés de groupe).
    pub fn to_json(&self) -> Zeroizing<Vec<u8>> {
        // Invariant : tous les champs sont des chaînes, entiers ou tableaux.
        Zeroizing::new(serde_json::to_vec(self).expect("Body est toujours sérialisable"))
    }

    /// Lit un corps ; un `file` aux tailles incohérentes, un `otp` au code ou
    /// au domaine invalide sont refusés.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let body = serde_json::from_slice(bytes).map_err(|e| CryptoError::Body(e.to_string()))?;
        match &body {
            Self::File { size, chunk_size, chunks, .. } => {
                check_layout(*size, *chunk_size, *chunks)?
            }
            Self::Otp(otp) => otp.check()?,
            _ => {}
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_stage_1_format() {
        let body = Body::Text { body: "bonjour".into() };
        assert_eq!(&**body.to_json(), br#"{"kind":"text","body":"bonjour"}"#);
        let content = ot_proto::Content::text("bonjour");
        assert_eq!(Body::from_json(&serde_json::to_vec(&content).unwrap()).unwrap(), body);
    }

    fn file(chunks: u32) -> Body {
        Body::File {
            blob_id: "blob_01J9Z3Q5RT8YB6ZK4C2D7E9F1G".parse().unwrap(),
            name: "photo.jpg".into(),
            mime: "image/jpeg".into(),
            size: 2_500_000,
            chunk_size: crate::FILE_CHUNK_SIZE,
            chunks,
            key: FileKey::from_bytes([1; 32]),
            nonce: [2; FILE_NONCE_LEN],
            sha256: [3; 32],
        }
    }

    #[test]
    fn file_roundtrip_and_layout_check() {
        let body = file(3);
        assert_eq!(Body::from_json(&body.to_json()).unwrap(), body);
        assert_eq!(body.kind(), "file");
        // Le `Debug` d'un corps `file` ne montre pas la clé.
        assert!(!format!("{body:?}").contains("[1, 1"));
        assert!(Body::from_json(&file(4).to_json()).is_err());
    }

    fn otp() -> Otp {
        Otp {
            code: "482913".into(),
            display: "G-482913".into(),
            sender_name: "Google".into(),
            sender_domain: "google.com".into(),
            sender_verified: true,
            expires_at: 1_793_628_791,
        }
    }

    #[test]
    fn otp_format() {
        let body = Body::Otp(otp());
        assert_eq!(
            &**body.to_json(),
            br#"{"kind":"otp","code":"482913","display":"G-482913","sender_name":"Google","sender_domain":"google.com","sender_verified":true,"expires_at":1793628791}"#
        );
        assert_eq!(Body::from_json(&body.to_json()).unwrap(), body);
        assert_eq!(body.kind(), "otp");
    }

    #[test]
    fn otp_rejects_bad_fields() {
        let bad = |f: fn(&mut Otp)| {
            let mut o = otp();
            f(&mut o);
            Body::from_json(&Body::Otp(o).to_json()).is_err()
        };
        assert!(bad(|o| o.code = "123".into()));
        assert!(bad(|o| o.code = "12345678901234567".into()));
        assert!(bad(|o| o.code = "48 913".into()));
        assert!(bad(|o| o.code = "４８２９１３".into()));
        assert!(bad(|o| o.sender_domain = String::new()));
        assert!(bad(|o| o.sender_domain = "Google.com".into()));
        assert!(bad(|o| o.sender_domain = "evil.com/x".into()));
        assert!(bad(|o| o.display = "x".repeat(129)));
        assert!(!bad(|o| o.code = "AB12cd".into()));
        assert!(
            Body::from_json(br#"{"kind":"otp","code":"482913","display":"","sender_name":"","sender_domain":"a.fr","sender_verified":false,"expires_at":0,"extra":1}"#)
                .is_err()
        );
    }

    #[test]
    fn rejects_unknown_kinds_and_fields() {
        assert!(Body::from_json(br#"{"kind":"teleport"}"#).is_err());
        assert!(Body::from_json(br#"{"kind":"text","body":"x","extra":1}"#).is_err());
    }
}
