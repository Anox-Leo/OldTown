use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ulid::Ulid;

use time::OffsetDateTime;

use crate::{
    AdminUser, DeviceId, DeviceKind, DirectoryUser, Envelope, Feature, Known, NONCE_LEN,
    PAIR_TOKEN_LEN, PROTOCOL_VERSION, Role, SignedBlob, UserId, b64,
};

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
    Welcome {
        device_id: DeviceId,
        pending: u64,
    },
    /// Client → serveur : envoi d'une enveloppe.
    Send {
        env: Envelope,
    },
    /// Serveur → client : l'enveloppe est stockée.
    Accepted {
        msg_id: Ulid,
        seq: i64,
    },
    /// Serveur → client : livraison d'un message.
    Deliver {
        seq: i64,
        env: Envelope,
    },
    /// Client → serveur : message `seq` reçu et traité.
    Ack {
        seq: i64,
    },
    /// Client → serveur : demande la liste des appareils.
    ListDevices,
    /// Serveur → client : réponse à `list_devices`.
    Devices {
        devices: Vec<DeviceInfo>,
    },
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
    PairRedeemed {
        device_id: DeviceId,
        sponsor: DeviceId,
    },
    /// Parrain, session authentifiée → serveur (QR inversé, note 0022) :
    /// enregistre le nouvel appareil dont il a scanné le QR. `sig` signe
    /// [`crate::pair_register_message`] avec `public_key`.
    PairRegister {
        device_id: DeviceId,
        name: String,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    /// Serveur → parrain : appareil enregistré. Une erreur porte en `ref`
    /// l'identifiant demandé et laisse la session ouverte.
    PairRegistered {
        device_id: DeviceId,
    },
    // --- Étape 10 : personnes, annuaire et réseau (note 0052) ---------------
    /// Client → serveur : demande l'annuaire. `known` : dernier `seq` du
    /// journal déjà vérifié, par personne ; absent, tout le journal.
    DirectoryGet {
        #[serde(default, skip_serializing_if = "Known::is_empty")]
        known: Known,
        /// Page suivante : les personnes après celle-ci (le `next` de la
        /// trame `directory` précédente) ; absent, depuis le début.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<UserId>,
    },
    /// Serveur → client : ma personne, l'admin (ancre), et les personnes
    /// liées dans un sens ou dans l'autre, filtrées par le réseau. Une
    /// trame ne dépasse jamais `MAX_FRAME_BYTES` : `next` présent, d'autres
    /// personnes suivent (relire avec `after = next`).
    Directory {
        me: UserId,
        admin: UserId,
        users: Vec<DirectoryUser>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<UserId>,
    },
    /// Serveur → client : l'annuaire a changé, le relire.
    DirectoryChanged,
    /// Client → serveur : publie l'entrée `seq` du journal de roster de
    /// `user` (sa personne ; l'admin aussi pour une personne de service).
    RosterPublish {
        user: UserId,
        seq: u64,
        entry: SignedBlob,
    },
    /// Serveur → client : entrée enregistrée.
    RosterPublished {
        user: UserId,
        seq: u64,
    },
    /// Client → serveur : règle le profil d'un appareil de sa personne
    /// (l'admin : tous). Seul l'appareil lui-même déclare ses `features`.
    DeviceUpdate {
        device: DeviceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<DeviceKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reachable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        features: Option<Vec<Feature>>,
    },
    DeviceUpdated {
        device: DeviceId,
    },
    /// Client → serveur : révoque un appareil de sa personne (l'admin :
    /// tous), jamais soi-même. La rotation de la clé de groupe est faite
    /// avant par le client.
    DeviceRevoke {
        device: DeviceId,
    },
    DeviceRevoked {
        device: DeviceId,
    },
    /// Membre → serveur : coupe (`muted`) ou rouvre un service lié à lui.
    ServiceMute {
        service: UserId,
        muted: bool,
    },
    ServiceMuted {
        service: UserId,
        muted: bool,
    },
    /// Admin → serveur, une seule fois : certificat racine auto-signé et
    /// première entrée de son journal.
    AdminInit {
        cert: SignedBlob,
        entry: SignedBlob,
    },
    AdminInitialized {
        user: UserId,
    },
    /// Admin → serveur : invite une personne (`member`), liée dans les deux
    /// sens à chaque personne de `links`.
    UserInvite {
        name: String,
        role: Role,
        #[serde(default)]
        links: Vec<UserId>,
    },
    /// Admin → serveur : réinvite une personne qui a perdu ses appareils ;
    /// ses appareils sont révoqués, son certificat et son journal oubliés,
    /// ses liens gardés.
    UserReinvite {
        user: UserId,
    },
    /// Serveur → admin : jeton d'invitation à usage unique.
    UserInvited {
        user: UserId,
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        #[serde(with = "time::serde::rfc3339")]
        expires_at: OffsetDateTime,
    },
    /// Admin → serveur : certificat d'une personne invitée (elle devient
    /// active), ou certificat re-signé d'une personne active.
    UserCertify {
        user: UserId,
        cert: SignedBlob,
    },
    UserCertified {
        user: UserId,
    },
    /// Admin → serveur : crée une personne de service et son premier
    /// appareil, dont il a scanné le QR inversé. `sig` signe
    /// [`crate::pair_register_message`] avec `public_key`. Réponse :
    /// `pair_registered`.
    ServiceCreate {
        user: UserId,
        name: String,
        device: DeviceId,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        device_name: String,
        #[serde(with = "b64::array")]
        sig: [u8; 64],
        cert: SignedBlob,
    },
    /// Admin → serveur : ajoute un appareil à une personne de service.
    /// Réponse : `pair_registered`.
    ServiceDevice {
        user: UserId,
        device: DeviceId,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        name: String,
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    /// Admin → serveur : révoque une personne, ses appareils et ses liens.
    UserRevoke {
        user: UserId,
    },
    UserRevoked {
        user: UserId,
    },
    UserRename {
        user: UserId,
        name: String,
    },
    UserRenamed {
        user: UserId,
    },
    /// Admin → serveur : pose ou retire un lien. Entre deux personnes, les
    /// deux sens ; depuis un service, le seul sens service → personne.
    LinkSet {
        from: UserId,
        to: UserId,
        allowed: bool,
    },
    LinkUpdated {
        from: UserId,
        to: UserId,
        allowed: bool,
    },
    /// Admin → serveur : toutes les personnes et leurs appareils.
    UsersGet,
    Users {
        users: Vec<AdminUser>,
    },
    /// Premier appareil d'une personne invitée → serveur, à la place de
    /// `auth`. `sig` signe [`crate::invite_redeem_message`] avec `public_key`.
    InviteRedeem {
        #[serde(with = "b64::array")]
        token: [u8; PAIR_TOKEN_LEN],
        name: String,
        #[serde(with = "b64::array")]
        public_key: [u8; 32],
        #[serde(with = "b64::array")]
        sig: [u8; 64],
    },
    /// Serveur → nouvel appareil : enregistré dans sa personne ; la
    /// connexion est ensuite fermée.
    InviteRedeemed {
        user: UserId,
        device: DeviceId,
        inviter: DeviceId,
    },
    /// Client → serveur, session authentifiée : maintien de la connexion.
    /// L'extension Chrome en envoie toutes les 20 s, son service worker
    /// s'arrêtant après 30 s sans message (note 0023).
    Ping,
    /// Serveur → client : réponse à `ping`.
    Pong,
    /// Les deux sens : erreur typée. `auth_failed`, et `pairing_invalid` ou
    /// `name_taken` pendant `pair_redeem` ou `invite_redeem`, ferment la
    /// connexion.
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
    /// Jeton d'appairage inconnu, expiré ou déjà utilisé, signature fausse,
    /// ou clé ou identifiant déjà enregistrés.
    PairingInvalid,
    /// Nom d'appareil (ou de personne) déjà pris.
    NameTaken,
    Internal,
    /// Trame réservée à l'admin, ou interdite à une personne en attente
    /// (étape 10).
    Forbidden,
    /// Destinataire hors réseau : rien n'est stocké.
    NotAllowed,
    /// Quota de la personne atteint (appareils, envois, invitations).
    QuotaExceeded,
    /// La personne de la session vient d'être révoquée.
    UserRevoked,
    /// Écriture concurrente (journal de roster) : relire puis recommencer.
    Conflict,
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
    "pair_register",
    "pair_registered",
    "directory_get",
    "directory",
    "directory_changed",
    "roster_publish",
    "roster_published",
    "device_update",
    "device_updated",
    "device_revoke",
    "device_revoked",
    "service_mute",
    "service_muted",
    "admin_init",
    "admin_initialized",
    "user_invite",
    "user_reinvite",
    "user_invited",
    "user_certify",
    "user_certified",
    "service_create",
    "service_device",
    "user_revoke",
    "user_revoked",
    "user_rename",
    "user_renamed",
    "link_set",
    "link_updated",
    "users_get",
    "users",
    "invite_redeem",
    "invite_redeemed",
    "ping",
    "pong",
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
            Self::PairRegister { .. } => "pair_register",
            Self::PairRegistered { .. } => "pair_registered",
            Self::DirectoryGet { .. } => "directory_get",
            Self::Directory { .. } => "directory",
            Self::DirectoryChanged => "directory_changed",
            Self::RosterPublish { .. } => "roster_publish",
            Self::RosterPublished { .. } => "roster_published",
            Self::DeviceUpdate { .. } => "device_update",
            Self::DeviceUpdated { .. } => "device_updated",
            Self::DeviceRevoke { .. } => "device_revoke",
            Self::DeviceRevoked { .. } => "device_revoked",
            Self::ServiceMute { .. } => "service_mute",
            Self::ServiceMuted { .. } => "service_muted",
            Self::AdminInit { .. } => "admin_init",
            Self::AdminInitialized { .. } => "admin_initialized",
            Self::UserInvite { .. } => "user_invite",
            Self::UserReinvite { .. } => "user_reinvite",
            Self::UserInvited { .. } => "user_invited",
            Self::UserCertify { .. } => "user_certify",
            Self::UserCertified { .. } => "user_certified",
            Self::ServiceCreate { .. } => "service_create",
            Self::ServiceDevice { .. } => "service_device",
            Self::UserRevoke { .. } => "user_revoke",
            Self::UserRevoked { .. } => "user_revoked",
            Self::UserRename { .. } => "user_rename",
            Self::UserRenamed { .. } => "user_renamed",
            Self::LinkSet { .. } => "link_set",
            Self::LinkUpdated { .. } => "link_updated",
            Self::UsersGet => "users_get",
            Self::Users { .. } => "users",
            Self::InviteRedeem { .. } => "invite_redeem",
            Self::InviteRedeemed { .. } => "invite_redeemed",
            Self::Ping => "ping",
            Self::Pong => "pong",
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
