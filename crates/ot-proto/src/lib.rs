//! Protocole OldTown : trames WebSocket, enveloppe et constantes partagées
//! entre le serveur relais et les clients.
//!
//! Chaque trame est un objet JSON `{"v":1,"t":"<type>", ...}`. Le serveur ne
//! dépend que de cette crate : il ne lit jamais `payload`. L'API HTTP des
//! fichiers (étape 5) est dans [`http`].

pub mod b64;
mod blob;
mod envelope;
mod frame;
pub mod http;

pub use blob::{BlobId, InvalidBlobId};
pub use envelope::{Content, ContentKind, DeviceId, Encoding, Envelope, InvalidDeviceId, Payload};
pub use frame::{DecodeError, DeviceInfo, ErrorCode, Frame};
pub use ulid::Ulid;

/// Version du protocole portée par le champ `v` de chaque trame.
pub const PROTOCOL_VERSION: u8 = 1;

/// Taille maximale d'une enveloppe sérialisée.
pub const MAX_ENVELOPE_BYTES: usize = 64 * 1024;

/// Taille maximale d'une trame WebSocket : au-delà, la connexion est coupée ;
/// entre les deux, l'enveloppe est refusée avec une erreur typée.
pub const MAX_FRAME_BYTES: usize = 2 * MAX_ENVELOPE_BYTES;

/// Préfixe de domaine du message signé à l'authentification.
pub const AUTH_DOMAIN: &[u8] = b"ot-auth-v1";

/// Taille du nonce de défi.
pub const NONCE_LEN: usize = 32;

/// Préfixe de domaine du message signé par `pair_redeem`.
pub const PAIR_DOMAIN: &[u8] = b"ot-pair-v1";

/// Préfixe de domaine du message signé par un nouvel appareil pour être
/// enregistré par son parrain (`pair_register`, QR inversé).
pub const PAIR_REGISTER_DOMAIN: &[u8] = b"ot-pair-reg-v1";

/// Taille du jeton d'appairage.
pub const PAIR_TOKEN_LEN: usize = 10;

/// Taille maximale d'un nom d'appareil, en caractères.
pub const MAX_NAME_CHARS: usize = 64;

/// Chemin de l'endpoint WebSocket.
pub const WS_PATH: &str = "/v1/ws";

/// Nom d'appareil accepté par le relais : 1 à [`MAX_NAME_CHARS`] caractères
/// imprimables, sans espace autour. Les clients le vérifient avant de le
/// signer (QR inversé) ; le relais l'impose à l'enregistrement.
pub fn valid_device_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

/// Message signé par le client : `"ot-auth-v1" ‖ nonce ‖ device_id`.
pub fn auth_message(nonce: &[u8; NONCE_LEN], device_id: &DeviceId) -> Vec<u8> {
    let id = device_id.as_str().as_bytes();
    let mut msg = Vec::with_capacity(AUTH_DOMAIN.len() + NONCE_LEN + id.len());
    msg.extend_from_slice(AUTH_DOMAIN);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(id);
    msg
}

/// Message signé par un nouvel appareil dans `pair_redeem` :
/// `"ot-pair-v1" ‖ nonce ‖ sha256(jeton)`. Le relais n'enregistre ainsi que des
/// clés dont le possesseur est présent.
pub fn pair_redeem_message(nonce: &[u8; NONCE_LEN], token_hash: &[u8; 32]) -> Vec<u8> {
    [PAIR_DOMAIN, nonce, token_hash].concat()
}

/// Message signé par un nouvel appareil dans son QR inversé et vérifié par le
/// relais à `pair_register` : `"ot-pair-reg-v1" ‖ device_id ‖ clé publique ‖
/// nom`. L'identifiant (30 octets) et la clé (32) ont une taille fixe ; le nom
/// vient en dernier. Le relais n'enregistre ainsi que des clés dont le
/// possesseur a demandé l'enregistrement, sous ce nom et cet identifiant.
pub fn pair_register_message(device_id: &DeviceId, public_key: &[u8; 32], name: &str) -> Vec<u8> {
    [PAIR_REGISTER_DOMAIN, device_id.as_str().as_bytes(), public_key, name.as_bytes()].concat()
}
