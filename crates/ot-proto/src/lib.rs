//! Protocole OldTown : trames WebSocket, enveloppe et constantes partagées
//! entre le serveur relais et les clients.
//!
//! Chaque trame est un objet JSON `{"v":1,"t":"<type>", ...}`. Le serveur ne
//! dépend que de cette crate : il ne lit jamais `payload`.

pub mod b64;
mod envelope;
mod frame;

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

/// Chemin de l'endpoint WebSocket.
pub const WS_PATH: &str = "/v1/ws";

/// Message signé par le client : `"ot-auth-v1" ‖ nonce ‖ device_id`.
pub fn auth_message(nonce: &[u8; NONCE_LEN], device_id: &DeviceId) -> Vec<u8> {
    let id = device_id.as_str().as_bytes();
    let mut msg = Vec::with_capacity(AUTH_DOMAIN.len() + NONCE_LEN + id.len());
    msg.extend_from_slice(AUTH_DOMAIN);
    msg.extend_from_slice(nonce);
    msg.extend_from_slice(id);
    msg
}
