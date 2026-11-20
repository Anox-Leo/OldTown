//! Types communs au client réseau et au cœur sans réseau (`Secure`) :
//! événements, erreurs, messages reçus. Sans dépendance réseau, pour que le
//! cœur se compile aussi en WebAssembly (extension, note 0023).

use ot_crypto::Otp;
use ot_proto::{Content, DeviceId, ErrorCode, Ulid, UserId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::files::ReceivedFile;

/// Qui a écrit un message reçu, et à qui (étape 10). Tout est vide pour un
/// message de mon groupe.
/// Rangé avec les offres de fichiers (`files.json`) : champs absents des
/// fichiers d'avant l'étape 10 lus à vide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Origin {
    /// Personne de l'expéditeur ; `None` : ma personne.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_user: Option<UserId>,
    /// L'expéditeur est une personne de service (codes, alertes).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_service: bool,
    /// Copie d'un de mes envois (D43) : la personne à qui il était adressé.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_user: Option<UserId>,
}

impl Origin {
    /// Message de mon groupe : rien à dire.
    pub fn is_mine(&self) -> bool {
        self == &Self::default()
    }
}

/// Cible d'un envoi (étape 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Mes appareils, sous la clé de groupe ; vide : tous les autres.
    MyDevices(Vec<DeviceId>),
    /// Une autre personne, en `0x05` : un appareil précis, ou ses appareils
    /// par défaut (D42) ; une copie part vers mes appareils (D43).
    Person { user: UserId, device: Option<DeviceId> },
}

/// Message reçu, déchiffré, d'un membre du roster dont la signature est
/// vérifiée, et pas encore vu.
#[derive(Debug, Clone)]
pub struct Received {
    pub seq: i64,
    pub msg_id: Ulid,
    pub from: DeviceId,
    /// Destinataires lus dans l'enveloppe ; vide : tous les appareils.
    pub to: Vec<DeviceId>,
    pub created_at: OffsetDateTime,
    pub content: Content,
    pub origin: Origin,
}

/// Code à usage unique reçu (note 0045), d'un membre du roster dont la
/// signature est vérifiée. Il peut être déjà expiré : à l'appli d'en juger.
#[derive(Debug, Clone)]
pub struct ReceivedOtp {
    pub seq: i64,
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub to: Vec<DeviceId>,
    pub created_at: OffsetDateTime,
    pub otp: Otp,
    pub origin: Origin,
}

impl ReceivedOtp {
    /// Règle O2 (note 0052) : un code n'est copié ou rempli d'office que
    /// s'il vient de ma personne ou d'un service. D'une autre personne, il
    /// est montré, jamais utilisé seul.
    pub fn auto_fill(&self) -> bool {
        self.origin.from_user.is_none() || self.origin.from_service
    }
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
    /// Fichier annoncé (étape 5) : sa clé est rangée dans le keyring, le
    /// téléchargement se demande par son `blob_id`.
    File(ReceivedFile),
    /// Code à usage unique (note 0045).
    Otp(ReceivedOtp),
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
    /// Annuaire relu (étape 10) : des personnes, appareils ou profils
    /// vérifiés ont changé ; à relire par `Client::directory`.
    DirectoryChanged,
    /// Un objet de l'annuaire (certificat, entrée de journal) a été refusé :
    /// la personne reste à sa dernière entrée vérifiée, ou disparaît.
    DirectoryRejected {
        user: UserId,
        reason: String,
    },
    /// Le journal publié de ma personne porte, à version égale, d'autres
    /// appareils que mon roster : alerte de sécurité.
    DirectoryFork,
    /// Une personne a de nouvelles clés (réinvitation) : à dire, comme le
    /// font les messageries chiffrées.
    KeysChanged {
        user: UserId,
    },
    /// Admin : une personne invitée par cet appareil est certifiée et a reçu
    /// sa bienvenue. `fingerprint` : empreinte courte de la clé de son
    /// appareil, à comparer de vive voix.
    PersonJoined {
        user: UserId,
        name: String,
        device_name: String,
        fingerprint: String,
    },
    /// Invité : ma personne est certifiée, mon groupe est créé.
    Joined {
        user: UserId,
    },
    /// Invitation en échec (bienvenue invalide, certification refusée…).
    InviteFailed {
        reason: String,
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
    /// Envoi à une personne : personne inconnue de l'annuaire vérifié, ou
    /// aucun appareil joignable et à jour (D42).
    #[error("aucun destinataire : {0}")]
    NoRecipient(String),
    #[error("chiffrement : {0}")]
    Crypto(String),
    #[error("appairage : {0}")]
    Pairing(String),
    #[error("connexion : {0}")]
    Connection(String),
    #[error("{0}")]
    Invalid(String),
    /// Fichier supprimé du relais (expiré, annulé ou déjà retiré par tous).
    #[error("fichier expiré : il n'est plus sur le relais")]
    Expired,
    #[error("transfert : {0}")]
    Transfer(String),
}
