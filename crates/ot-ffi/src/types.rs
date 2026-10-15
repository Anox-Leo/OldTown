//! Types vus par Kotlin : enregistrements simples, sans clé ni secret, sauf
//! le code d'appairage (`Invitation`, `QrKind::Join`) que l'appli affiche.

use ot_core::{ClientError, ErrorCode, GroupInfo, KeyringError};
use serde::{Deserialize, Serialize};

/// Erreur d'un appel. Kotlin n'en voit que le type et le message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum OtError {
    /// Relais injoignable (Tailscale coupé ?) ou client arrêté.
    #[error("{message}")]
    Offline { message: String },
    /// Le relais ne reconnaît pas cet appareil (révoqué, ou pas encore
    /// enregistré).
    #[error("{message}")]
    AuthFailed { message: String },
    /// Pas de groupe, ou destinataire hors du groupe.
    #[error("{message}")]
    NotInGroup { message: String },
    /// Appairage refusé (code expiré, nom pris, QR invalide…).
    #[error("{message}")]
    Pairing { message: String },
    /// Donnée saisie invalide (URL, nom…).
    #[error("{message}")]
    Invalid { message: String },
    /// Fichier plus disponible sur le relais (expiré, annulé).
    #[error("{message}")]
    Expired { message: String },
    /// Transfert de fichier en échec (réseau, fichier altéré, stockage) ;
    /// le relancer reprend là où il s'est arrêté.
    #[error("{message}")]
    Transfer { message: String },
    #[error("{message}")]
    Internal { message: String },
}

impl From<ClientError> for OtError {
    fn from(e: ClientError) -> Self {
        let message = e.to_string();
        match e {
            ClientError::Connection(_) | ClientError::Closed => Self::Offline { message },
            ClientError::Rejected { code: ErrorCode::AuthFailed, .. } => {
                Self::AuthFailed { message }
            }
            ClientError::Rejected {
                code: ErrorCode::PairingInvalid | ErrorCode::NameTaken,
                ..
            }
            | ClientError::Pairing(_) => Self::Pairing { message },
            ClientError::NoGroup | ClientError::NotMember(_) => Self::NotInGroup { message },
            ClientError::Invalid(_) => Self::Invalid { message },
            ClientError::Expired => Self::Expired { message },
            ClientError::Transfer(_) => Self::Transfer { message },
            ClientError::Rejected { .. } | ClientError::Crypto(_) => Self::Internal { message },
        }
    }
}

impl From<KeyringError> for OtError {
    fn from(e: KeyringError) -> Self {
        Self::Internal { message: e.to_string() }
    }
}

/// Erreur du stockage de l'appli.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum StoreError {
    #[error("{message}")]
    Failed { message: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for StoreError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed { message: e.reason }
    }
}

/// Appareil enregistré sur ce téléphone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct DeviceState {
    pub device_id: String,
    pub name: String,
    /// URL du relais, ex. `wss://relais:7878`.
    pub server_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Link,
    /// Fichier (étape 5) : détails dans `Received::file`.
    File,
}

impl From<ot_core::ContentKind> for ContentKind {
    fn from(k: ot_core::ContentKind) -> Self {
        match k {
            ot_core::ContentKind::Text => Self::Text,
            ot_core::ContentKind::Link => Self::Link,
        }
    }
}

/// Élément de l'historique : reçu (déchiffré et vérifié) ou envoyé d'ici
/// (`outgoing`, note 0033).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct Received {
    pub msg_id: String,
    pub from_id: String,
    /// Nom de l'expéditeur dans le roster (son identifiant à défaut).
    pub from_name: String,
    /// Date d'envoi, en millisecondes depuis l'époque Unix.
    pub created_at_ms: i64,
    pub kind: ContentKind,
    pub body: String,
    /// Contenu à masquer (aperçu, presse-papiers). Toujours faux : le
    /// marquage prévu à l'étape 6 est abandonné (note 0034).
    #[serde(default)]
    pub sensitive: bool,
    /// Fichier annoncé (`kind` vaut `File`, `body` son nom).
    #[serde(default)]
    pub file: Option<crate::FileItem>,
    /// Destinataires, lus dans l'enveloppe ; vide : tous les appareils.
    #[serde(default)]
    pub to: Vec<String>,
    /// Envoyé depuis ce téléphone (`from_id` est alors le sien).
    #[serde(default)]
    pub outgoing: bool,
    /// Envoi pas encore accepté par le relais.
    #[serde(default)]
    pub pending: bool,
}

/// Événement du client, dans l'ordre d'arrivée.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Event {
    /// Session ouverte ; `pending` messages vont suivre.
    Connected { pending: u64 },
    /// Connexion perdue ; le client réessaie seul.
    Disconnected { reason: String },
    /// Le relais refuse cet appareil (révoqué, ou pas encore enregistré).
    AuthFailed { message: String },
    /// Élément reçu, déjà rangé dans l'historique.
    Message { received: Received },
    /// Élément envoyé d'ici, rangé ou mis à jour dans l'historique (en
    /// cours, puis accepté). Jamais à notifier.
    Sent { item: Received },
    /// Envoi en échec : l'élément `msg_id` a quitté l'historique.
    Unsent { msg_id: String },
    /// Message rejeté (expéditeur hors groupe, signature fausse…).
    Rejected { from_name: String, reason: String },
    /// Groupe ou roster modifié (appairage, rotation).
    GroupChanged { epoch: u32 },
    /// Parrain : un appareil a rejoint le groupe.
    PairingCompleted { device_id: String, name: String },
    /// La demande d'appairage par code n'a pas pu partir.
    JoinFailed { message: String },
    /// Le stockage n'a pas pu lire ou écrire l'état.
    StorageError { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Member {
    pub device_id: String,
    pub name: String,
    /// Empreinte courte de la clé de signature, à comparer avec `ot group show`.
    pub sign_fingerprint: String,
    pub exchange_fingerprint: String,
    pub added_epoch: u32,
    /// Ce téléphone.
    pub is_self: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Group {
    pub epoch: u32,
    pub key_fingerprint: String,
    pub roster_version: u64,
    pub members: Vec<Member>,
}

impl Group {
    pub(crate) fn from_info(info: GroupInfo, me: &str) -> Self {
        Self {
            epoch: info.epoch,
            key_fingerprint: info.key_fingerprint,
            roster_version: info.roster_version,
            members: info
                .members
                .into_iter()
                .map(|m| Member {
                    is_self: m.device_id.as_str() == me,
                    device_id: m.device_id.to_string(),
                    name: m.name,
                    sign_fingerprint: m.sign_fingerprint,
                    exchange_fingerprint: m.exchange_fingerprint,
                    added_epoch: m.added_epoch,
                })
                .collect(),
        }
    }
}

/// Appareil connu du relais.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RelayDevice {
    pub device_id: String,
    pub name: String,
    pub online: bool,
    /// Membre du groupe ; sinon appairage inachevé ou relais compromis.
    pub member: bool,
}

/// Code d'appairage créé par ce téléphone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Invitation {
    /// Code `ot1-…` (contient le secret).
    pub code: String,
    /// QR « rejoindre » : `oldtown://pair?s=…&c=…`.
    pub uri: String,
    pub expires_at_ms: i64,
}

/// Appareil ajouté par QR inversé.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Sponsored {
    pub device_id: String,
    pub name: String,
}

/// Ce que contient un QR scanné.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum QrKind {
    /// Rejoindre un groupe (`ot pair new --qr`).
    Join { server_url: String, code: String },
    /// Parrainer un appareil (`ot pair join --qr`), signature vérifiée.
    Sponsor { name: String, device_id: String },
    /// QR OldTown mal formé.
    Invalid { reason: String },
    /// Autre chose.
    Unknown,
}

/// QR code à dessiner : `size × size` modules, ligne par ligne.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QrCode {
    pub size: u32,
    /// `true` : module sombre.
    pub modules: Vec<bool>,
}

pub(crate) fn millis(t: time::OffsetDateTime) -> i64 {
    (t.unix_timestamp_nanos() / 1_000_000) as i64
}
