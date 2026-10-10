use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ulid::Ulid;

use time::OffsetDateTime;

use crate::{DeviceId, Envelope, NONCE_LEN, PAIR_TOKEN_LEN, PROTOCOL_VERSION, b64};

/// Trame échangée sur la connexion WebSocket. Le type est porté par `t`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    /// Serveur → client, à l'ouverture de la connexion.
    Challenge {
        #[serde(with = "b64::array")]
        nonce: [u8; NONCE_LEN],
    },
    /// Client → serveur : signature Ed25519 de [`crate::auth_message`].
    Auth {
        device_id: DeviceId,
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    /// Serveur → client : session ouverte, `pending` messages en attente.
    Welcome { device_id: DeviceId, pending: u64 },
    /// Client → serveur : envoi d'une enveloppe.
    Send { env: Envelope },
    /// Serveur → client : l'enveloppe est stockée.
    Accepted { msg_id: Ulid, seq: i64 },
    /// Serveur → client : livraison d'un message.
    Deliver { seq: i64, env: Envelope },
    /// Client → serveur : message `seq` reçu et traité.
    Ack { seq: i64 },
    /// Client → serveur : demande la liste des appareils.
    ListDevices,
    /// Serveur → client : réponse à `list_devices`.
    Devices { devices: Vec<DeviceInfo> },
    /// Client → serveur, session authentifiée : demande un jeton d'appairage.
    PairCreate,
    /// Serveur → client : jeton à usage unique, valable jusqu'à `expires_at`.
    PairCreated {
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        #[serde(with = "time::serde::rfc3339")]
        expires_at: OffsetDateTime,
    },
    /// Nouvel appareil → serveur, à la place de `auth` : s'enregistre avec un
    /// jeton. `sig` signe [`crate::pair_redeem_message`] avec `public_key`.
    PairRedeem {
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        name: String,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    /// Serveur → nouvel appareil : enregistré ; la connexion est ensuite fermée.
    PairRedeemed { device_id: DeviceId, sponsor: DeviceId },
    /// Les deux sens : erreur typée. `auth_failed`, `pairing_invalid` et
    /// `name_taken` ferment la connexion.
    Error {
        code: ErrorCode,
        message: String,
        /// Référence de la trame fautive (`msg_id` d'un `send`, par exemple).
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// JSON invalide ou champs incorrects.
    BadFrame,
    UnsupportedVersion,
    UnknownType,
    /// Trame valide mais inattendue à ce stade (ex. `send` avant `auth`).
    UnexpectedFrame,
    AuthFailed,
    EnvelopeTooLarge,
    /// `from` ne correspond pas à l'appareil authentifié.
    SenderMismatch,
    UnknownRecipient,
    /// `msg_id` déjà utilisé par un autre expéditeur.
    MsgIdConflict,
    /// Jeton d'appairage inconnu, expiré ou déjà utilisé, ou signature fausse.
    PairingInvalid,
    /// Nom d'appareil déjà pris par un appareil actif.
    NameTaken,
    Internal,
    /// Code ajouté par une version plus récente du protocole.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("trame invalide : {0}")]
    BadFrame(String),
    #[error("version de protocole non prise en charge : {0}")]
    UnsupportedVersion(u64),
    #[error("type de trame inconnu : {0:?}")]
    UnknownType(String),
}

impl DecodeError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadFrame(_) => ErrorCode::BadFrame,
            Self::UnsupportedVersion(_) => ErrorCode::UnsupportedVersion,
            Self::UnknownType(_) => ErrorCode::UnknownType,
        }
    }
}

const KNOWN_TYPES: &[&str] = &[
    "challenge",
    "auth",
    "welcome",
    "send",
    "accepted",
    "deliver",
    "ack",
    "list_devices",
    "devices",
    "pair_create",
    "pair_created",
    "pair_redeem",
    "pair_redeemed",
    "error",
];

impl Frame {
    pub fn error(code: ErrorCode, message: impl Into<String>, reference: Option<String>) -> Self {
        Self::Error { code, message: message.into(), reference }
    }

    /// Nom du type, tel qu'écrit dans `t`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Challenge { .. } => "challenge",
            Self::Auth { .. } => "auth",
            Self::Welcome { .. } => "welcome",
            Self::Send { .. } => "send",
            Self::Accepted { .. } => "accepted",
            Self::Deliver { .. } => "deliver",
            Self::Ack { .. } => "ack",
            Self::ListDevices => "list_devices",
            Self::Devices { .. } => "devices",
            Self::PairCreate => "pair_create",
            Self::PairCreated { .. } => "pair_created",
            Self::PairRedeem { .. } => "pair_redeem",
            Self::PairRedeemed { .. } => "pair_redeemed",
            Self::Error { .. } => "error",
        }
    }

    /// Sérialise la trame avec son champ de version `v`.
    pub fn to_json(&self) -> String {
        let Value::Object(fields) = serde_json::to_value(self).expect("Frame est sérialisable")
        else {
            unreachable!("une trame est toujours un objet JSON")
        };
        let mut obj = Map::with_capacity(fields.len() + 1);
        obj.insert("v".into(), Value::from(PROTOCOL_VERSION));
        obj.extend(fields);
        Value::Object(obj).to_string()
    }

    /// Décode une trame en distinguant version, type inconnu et contenu invalide.
    pub fn from_json(text: &str) -> Result<Self, DecodeError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| DecodeError::BadFrame(e.to_string()))?;
        let Value::Object(mut obj) = value else {
            return Err(DecodeError::BadFrame("objet JSON attendu".into()));
        };
        match obj.remove("v") {
            Some(Value::Number(n)) if n.as_u64() == Some(PROTOCOL_VERSION.into()) => {}
            Some(Value::Number(n)) => {
                return Err(DecodeError::UnsupportedVersion(n.as_u64().unwrap_or(0)));
            }
            _ => return Err(DecodeError::BadFrame("champ \"v\" manquant ou invalide".into())),
        }
        match obj.get("t") {
            Some(Value::String(t)) if KNOWN_TYPES.contains(&t.as_str()) => {}
            Some(Value::String(t)) => return Err(DecodeError::UnknownType(t.clone())),
            _ => return Err(DecodeError::BadFrame("champ \"t\" manquant ou invalide".into())),
        }
        serde_json::from_value(Value::Object(obj)).map_err(|e| DecodeError::BadFrame(e.to_string()))
    }
}
