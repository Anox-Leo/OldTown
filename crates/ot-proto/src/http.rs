//! API HTTP des fichiers (étape 5, note 0026) : dépôt et retrait de blobs
//! chiffrés, par morceaux, à côté du WebSocket.
//!
//! Le relais ne voit qu'une géométrie opaque : taille **chiffrée** totale,
//! taille d'un morceau chiffré, nombre de morceaux. Le format en clair (taille,
//! nom, type) ne voyage que dans le message chiffré `file` (`ot-crypto`).
//!
//! Chaque requête est signée par l'appareil, sans session (note 0009) : en-têtes
//! [`HEADER_DEVICE`], [`HEADER_TIME`], [`HEADER_NONCE`], [`HEADER_SIGNATURE`],
//! signature Ed25519 de [`http_request_message`].

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{BlobId, DeviceId};

/// Chemin de la collection des blobs.
pub const BLOBS_PATH: &str = "/v1/blobs";

/// Identifiant de l'appareil qui signe la requête.
pub const HEADER_DEVICE: &str = "ot-device";
/// Heure de la requête, en millisecondes depuis l'époque Unix (décimal).
pub const HEADER_TIME: &str = "ot-time";
/// Nonce de [`HTTP_NONCE_LEN`] octets, en base64.
pub const HEADER_NONCE: &str = "ot-nonce";
/// Signature Ed25519 de [`http_request_message`], en base64.
pub const HEADER_SIGNATURE: &str = "ot-signature";

/// Préfixe de domaine du message signé d'une requête HTTP.
pub const HTTP_DOMAIN: &[u8] = b"ot-http-v1";

/// Taille du nonce d'une requête HTTP.
pub const HTTP_NONCE_LEN: usize = 16;

/// Écart toléré entre l'heure de la requête et celle du relais, dans les deux
/// sens. Un nonce est refusé s'il a déjà servi pendant cette fenêtre.
pub const HTTP_WINDOW_MS: i64 = 60_000;

/// Taille maximale d'un morceau chiffré (4 Mio de clair + étiquette de 16 o).
pub const MAX_SEALED_CHUNK: u32 = (4 << 20) + 16;

/// Message signé par le client :
/// `"ot-http-v1" ‖ 0 ‖ méthode ‖ 0 ‖ chemin ‖ 0 ‖ heure (u64 BE) ‖ nonce ‖
/// SHA-256 du corps`. Méthode et chemin ont une longueur variable ; l'octet nul,
/// qui ne peut apparaître dans aucun des deux, les sépare sans ambiguïté.
pub fn http_request_message(
    method: &str,
    path: &str,
    time_ms: u64,
    nonce: &[u8; HTTP_NONCE_LEN],
    body_sha256: &[u8; 32],
) -> Vec<u8> {
    [
        HTTP_DOMAIN,
        &[0],
        method.as_bytes(),
        &[0],
        path.as_bytes(),
        &[0],
        &time_ms.to_be_bytes(),
        nonce,
        body_sha256,
    ]
    .concat()
}

/// Nombre de morceaux d'un blob de `size` octets chiffrés : au moins un.
/// `None` si `chunk_size` est nul ou si le compte dépasse `u32`.
pub fn blob_chunk_count(size: u64, chunk_size: u32) -> Option<u32> {
    if chunk_size == 0 {
        return None;
    }
    u32::try_from(size.div_ceil(u64::from(chunk_size)).max(1)).ok()
}

/// Taille attendue du morceau `index` d'un blob, ou `None` s'il n'existe pas.
pub fn blob_chunk_len(size: u64, chunk_size: u32, index: u32) -> Option<u32> {
    let chunks = blob_chunk_count(size, chunk_size)?;
    if index >= chunks {
        return None;
    }
    let start = u64::from(index) * u64::from(chunk_size);
    // Invariant : le reste est au plus `chunk_size`, donc tient en u32.
    Some(size.saturating_sub(start).min(u64::from(chunk_size)) as u32)
}

/// `POST /v1/blobs` : crée un blob. Tailles du **chiffré**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBlob {
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    /// Vide : tous les autres appareils actifs au moment de la création.
    #[serde(default)]
    pub to: Vec<DeviceId>,
}

/// Réponse à `POST /v1/blobs` (`201`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobCreated {
    pub blob_id: BlobId,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// Réponse à `GET /v1/blobs/{id}` : où en est le dépôt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobStatus {
    pub blob_id: BlobId,
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    /// Indices des morceaux pas encore déposés, dans l'ordre.
    pub missing: Vec<u32>,
    /// Tous les morceaux sont là : les destinataires peuvent les retirer.
    pub complete: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// Corps de toute réponse d'erreur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpError {
    pub error: HttpErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpErrorCode {
    /// Requête mal formée : JSON, tailles, index ou destinataires invalides.
    BadRequest,
    /// Signature absente ou fausse, heure hors fenêtre, nonce rejoué,
    /// appareil inconnu ou révoqué.
    Unauthorized,
    /// Ni propriétaire ni destinataire, selon l'opération.
    Forbidden,
    /// Blob inconnu, expiré ou supprimé.
    NotFound,
    /// Retrait demandé avant que tous les morceaux soient déposés.
    Incomplete,
    /// Fichier plus gros que la limite du relais.
    TooLarge,
    /// Quota disque du relais atteint.
    QuotaExceeded,
    /// Relais saturé (trop de requêtes dans la fenêtre) ; réessayer plus tard.
    Busy,
    Internal,
    /// Code ajouté par une version plus récente du protocole.
    #[serde(other)]
    Unknown,
}

impl HttpErrorCode {
    /// Code HTTP de la réponse.
    pub fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Incomplete => 409,
            Self::TooLarge => 413,
            Self::Busy => 503,
            Self::QuotaExceeded => 507,
            Self::Internal | Self::Unknown => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_geometry() {
        // 2 500 000 octets de clair en morceaux de 1 Mio : 3 morceaux chiffrés.
        let (size, chunk) = (2_500_000 + 3 * 16, (1 << 20) + 16);
        assert_eq!(blob_chunk_count(size, chunk), Some(3));
        assert_eq!(blob_chunk_len(size, chunk, 0), Some(chunk));
        assert_eq!(blob_chunk_len(size, chunk, 2), Some(2_500_000 - 2 * (1 << 20) + 16));
        assert_eq!(blob_chunk_len(size, chunk, 3), None);
        // Fichier vide : un morceau réduit à l'étiquette.
        assert_eq!(blob_chunk_count(16, chunk), Some(1));
        assert_eq!(blob_chunk_len(16, chunk, 0), Some(16));
        assert_eq!(blob_chunk_count(0, chunk), Some(1));
        assert_eq!(blob_chunk_count(10, 0), None);
        assert_eq!(blob_chunk_count(u64::MAX, 1), None);
    }

    #[test]
    fn statuses() {
        assert_eq!(HttpErrorCode::QuotaExceeded.status(), 507);
        assert_eq!(HttpErrorCode::TooLarge.status(), 413);
        assert_eq!(HttpErrorCode::Unknown.status(), 500);
    }
}
