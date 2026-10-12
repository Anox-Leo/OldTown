//! Types communs au client réseau et au cœur sans réseau (`Secure`) :
//! événements, erreurs, messages reçus. Sans dépendance réseau, pour que le
//! cœur se compile aussi en WebAssembly (extension, note 0023).

use ot_proto::{Content, DeviceId, ErrorCode, Ulid};
use time::OffsetDateTime;

/// Message reçu, déchiffré, d'un membre du roster dont la signature est
/// vérifiée, et pas encore vu.
#[derive(Debug, Clone)]
pub struct Received {
    pub seq: i64,
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub created_at: OffsetDateTime,
    pub content: Content,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Session ouverte ; `pending` messages vont suivre.
    Connected {
        pending: u64,
    },
    /// Connexion perdue ou refusée ; le client réessaie seul.
    Disconnected {
        reason: String,
    },
    /// Le serveur a refusé l'authentification (appareil inconnu, révoqué ou
    /// mauvaise clé). Le client réessaie quand même, avec le délai maximal.
    AuthFailed {
        message: String,
    },
    Message(Received),
    /// Message rejeté (en clair, expéditeur hors roster, signature fausse,
    /// déchiffrement impossible…) ; il est quand même acquitté.
    Rejected {
        msg_id: Ulid,
        from: DeviceId,
        reason: String,
    },
    /// Clés de groupe ou roster modifiés ; `epoch` est l'époque courante.
    GroupChanged {
        epoch: u32,
    },
    /// Parrain : l'appareil a rejoint le groupe.
    PairingCompleted {
        device_id: DeviceId,
        name: String,
    },
    /// Le keyring n'a pas pu lire ou enregistrer l'état du groupe.
    KeyringError {
        message: String,
    },
}

/// Code d'appairage à transmettre au nouvel appareil.
#[derive(Debug, Clone)]
pub struct PairingInvitation {
    /// Code `ot1-…` : il contient le secret, à ne transmettre que par un
    /// canal de confiance.
    pub code: String,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub msg_id: Ulid,
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("refusé par le serveur ({code:?}) : {message}")]
    Rejected { code: ErrorCode, message: String },
    #[error("client arrêté")]
    Closed,
    /// Pas de clé de groupe : le clair est refusé.
    #[error(
        "cet appareil n'a pas de groupe : `ot group init` sur le premier appareil, `ot pair join` sur les autres"
    )]
    NoGroup,
    #[error("{0} n'est pas membre du groupe")]
    NotMember(DeviceId),
    #[error("chiffrement : {0}")]
    Crypto(String),
    #[error("appairage : {0}")]
    Pairing(String),
    #[error("connexion : {0}")]
    Connection(String),
    #[error("{0}")]
    Invalid(String),
}
