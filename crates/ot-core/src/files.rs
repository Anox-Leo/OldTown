//! Fichiers (étape 5, note 0026), partie sans réseau : offres reçues
//! (rangées avec leur clé dans le [`crate::Keyring`]), dépôts en cours (pour
//! reprendre après un redémarrage), signature des requêtes HTTP. Le transfert
//! lui-même est dans `transfer.rs` (fonction `net`) ; l'extension refait la
//! boucle en TypeScript à partir d'ici (note 0023).

use ot_crypto::{
    Body, CHUNK_TAG_LEN, FILE_NONCE_LEN, FileCipher, FileKey, chunk_count, sanitize_file_name,
};
use ot_proto::http::{HEADER_DEVICE, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIME, HTTP_NONCE_LEN};
use ot_proto::{BlobId, DeviceId, Ulid, b64};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::Identity;

/// Durée pendant laquelle une offre reçue reste téléchargeable de notre côté
/// (le message vit 7 jours sur le relais, le fichier 48 h).
pub const FILE_OFFER_LIFETIME: Duration = Duration::days(7);

/// Nombre maximal d'offres gardées (les plus anciennes partent d'abord).
pub const MAX_FILE_OFFERS: usize = 500;

/// Fichier annoncé par un membre, tel que l'appli le voit : jamais de clé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub created_at: OffsetDateTime,
    pub blob_id: BlobId,
    /// Nom nettoyé, sûr à écrire sur le disque.
    pub name: String,
    pub mime: String,
    /// Taille en clair, en octets.
    pub size: u64,
}

/// Fichier reçu (message `file`), avec le `seq` de sa livraison.
#[derive(Debug, Clone)]
pub struct ReceivedFile {
    pub seq: i64,
    pub file: FileInfo,
}

/// Offre reçue, rangée dans le keyring avant l'acquittement : tout ce qu'il
/// faut pour retirer, déchiffrer et vérifier le fichier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredFile {
    pub msg_id: Ulid,
    pub from: DeviceId,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
    pub blob_id: BlobId,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    pub key: FileKey,
    #[serde(with = "b64::array")]
    pub nonce: [u8; FILE_NONCE_LEN],
    #[serde(with = "b64::array")]
    pub sha256: [u8; 32],
}

impl StoredFile {
    pub fn info(&self) -> FileInfo {
        FileInfo {
            msg_id: self.msg_id,
            from: self.from.clone(),
            created_at: self.created_at,
            blob_id: self.blob_id.clone(),
            name: self.name.clone(),
            mime: self.mime.clone(),
            size: self.size,
        }
    }

    pub fn cipher(&self) -> FileCipher {
        FileCipher::new(&self.key, &self.nonce)
    }

    /// Taille d'un morceau chiffré, telle que le relais la connaît.
    pub fn sealed_chunk_size(&self) -> u32 {
        self.chunk_size + CHUNK_TAG_LEN
    }
}

/// Ce que l'expéditeur met dans le message `file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileMeta {
    pub blob_id: BlobId,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    pub key: FileKey,
    #[serde(with = "b64::array")]
    pub nonce: [u8; FILE_NONCE_LEN],
    #[serde(with = "b64::array")]
    pub sha256: [u8; 32],
}

impl FileMeta {
    pub fn body(&self) -> Body {
        Body::File {
            blob_id: self.blob_id.clone(),
            name: self.name.clone(),
            mime: self.mime.clone(),
            size: self.size,
            chunk_size: self.chunk_size,
            chunks: self.chunks,
            key: self.key.clone(),
            nonce: self.nonce,
            sha256: self.sha256,
        }
    }
}

/// Dépôt en cours, rangé dans le keyring (il porte la clé du fichier) :
/// relancer l'envoi de la même source reprend là où il s'était arrêté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingUpload {
    /// Identifiant de la source donné par l'appli (chemin, URI).
    pub source_id: String,
    pub to: Vec<DeviceId>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub meta: FileMeta,
}

/// Offre à ranger, d'après le corps `file` déchiffré d'un message.
pub(crate) fn stored_from_body(
    msg_id: Ulid,
    from: DeviceId,
    created_at: OffsetDateTime,
    now: OffsetDateTime,
    body: Body,
) -> Option<StoredFile> {
    let Body::File { blob_id, name, mime, size, chunk_size, chunks, key, nonce, sha256 } = body
    else {
        return None;
    };
    Some(StoredFile {
        msg_id,
        from,
        created_at,
        received_at: now,
        blob_id,
        name: sanitize_file_name(&name),
        mime: clean_mime(&mime),
        size,
        chunk_size,
        chunks,
        key,
        nonce,
        sha256,
    })
}

/// Type MIME de la forme `type/sous-type`, sans paramètre ; sinon
/// `application/octet-stream` (il sert à choisir comment ouvrir le fichier).
pub fn clean_mime(mime: &str) -> String {
    let essence = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let valid = essence.len() <= 127
        && essence.split_once('/').is_some_and(|(t, s)| {
            let ok = |p: &str| {
                !p.is_empty()
                    && p.chars().all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
            };
            ok(t) && ok(s)
        });
    if valid { essence } else { "application/octet-stream".to_owned() }
}

/// Ajoute une offre (en remplaçant une offre du même blob) et retire les
/// offres trop vieilles ou en trop.
pub(crate) fn add_offer(offers: &mut Vec<StoredFile>, offer: StoredFile, now: OffsetDateTime) {
    offers.retain(|o| o.blob_id != offer.blob_id);
    offers.push(offer);
    prune_offers(offers, now);
}

/// Retire les offres de plus de [`FILE_OFFER_LIFETIME`] et garde les
/// [`MAX_FILE_OFFERS`] plus récentes. Renvoie `true` si la liste a changé.
pub fn prune_offers(offers: &mut Vec<StoredFile>, now: OffsetDateTime) -> bool {
    let before = offers.len();
    offers.retain(|o| o.received_at + FILE_OFFER_LIFETIME > now);
    if offers.len() > MAX_FILE_OFFERS {
        offers.sort_by_key(|o| o.received_at);
        offers.drain(..offers.len() - MAX_FILE_OFFERS);
    }
    offers.len() != before
}

/// Géométrie d'un nouveau fichier de `size` octets en morceaux de
/// `chunk_size`, avec une clé et un préfixe de nonce neufs.
pub fn new_file_meta(
    blob_id: BlobId,
    name: &str,
    mime: &str,
    size: u64,
    chunk_size: u32,
    sha256: [u8; 32],
) -> Option<FileMeta> {
    Some(FileMeta {
        blob_id,
        name: name.to_owned(),
        mime: clean_mime(mime),
        size,
        chunk_size,
        chunks: chunk_count(size, chunk_size)?,
        key: FileKey::generate(),
        nonce: ot_crypto::new_file_nonce(),
        sha256,
    })
}

/// En-têtes d'authentification d'une requête HTTP (note 0026) : appareil,
/// heure, nonce neuf et signature de [`ot_proto::http::http_request_message`].
pub fn signed_headers(
    identity: &Identity,
    device_id: &DeviceId,
    method: &str,
    path: &str,
    body: &[u8],
    now_ms: u64,
) -> Vec<(&'static str, String)> {
    let mut nonce = [0; HTTP_NONCE_LEN];
    // Sans aléa, aucune requête sûre n'est possible : on s'arrête.
    getrandom::fill(&mut nonce).expect("source d'aléa du système indisponible");
    let sig = identity.sign_http(method, path, now_ms, &nonce, body);
    vec![
        (HEADER_DEVICE, device_id.to_string()),
        (HEADER_TIME, now_ms.to_string()),
        (HEADER_NONCE, b64::encode(&nonce)),
        (HEADER_SIGNATURE, b64::encode(&sig)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(blob: &str, received_at: OffsetDateTime) -> StoredFile {
        StoredFile {
            msg_id: Ulid::generate(),
            from: DeviceId::generate(),
            created_at: received_at,
            received_at,
            blob_id: blob.parse().unwrap(),
            name: "a.txt".into(),
            mime: "text/plain".into(),
            size: 3,
            chunk_size: 16,
            chunks: 1,
            key: FileKey::from_bytes([1; 32]),
            nonce: [2; FILE_NONCE_LEN],
            sha256: [3; 32],
        }
    }

    #[test]
    fn offers_expire_and_are_bounded() {
        let now = OffsetDateTime::now_utc();
        let mut offers = vec![offer("blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V", now - Duration::days(8))];
        add_offer(&mut offers, offer("blob_01J9Z3R7XK2M4N6P8Q0R2S4T6W", now), now);
        assert_eq!(offers.len(), 1);
        // Une offre du même blob remplace l'ancienne.
        add_offer(&mut offers, offer("blob_01J9Z3R7XK2M4N6P8Q0R2S4T6W", now), now);
        assert_eq!(offers.len(), 1);
        for _ in 0..MAX_FILE_OFFERS + 3 {
            add_offer(&mut offers, offer(BlobId::generate().as_str(), now), now);
        }
        assert_eq!(offers.len(), MAX_FILE_OFFERS);
    }

    #[test]
    fn mime_types_are_cleaned() {
        assert_eq!(clean_mime("image/JPEG"), "image/jpeg");
        assert_eq!(clean_mime("text/plain; charset=utf-8"), "text/plain");
        for bad in ["", "texte", "text/", "a/b c", "../x/y", "a/b/c"] {
            assert_eq!(clean_mime(bad), "application/octet-stream", "{bad:?}");
        }
    }

    #[test]
    fn stored_offer_hides_nothing_but_the_key_in_debug() {
        let o = offer("blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V", OffsetDateTime::now_utc());
        assert!(format!("{o:?}").contains("FileKey(…)"));
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(serde_json::from_str::<StoredFile>(&json).unwrap(), o);
    }

    #[test]
    fn signed_headers_verify() {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        use sha2::Digest;
        let id = Identity::generate();
        let dev = DeviceId::generate();
        let h = signed_headers(&id, &dev, "PUT", "/v1/blobs/x/chunks/0", b"abc", 42);
        assert_eq!(h[0], (HEADER_DEVICE, dev.to_string()));
        let nonce: [u8; 16] = b64::decode_array(&h[2].1).unwrap();
        let sig: [u8; 64] = b64::decode_array(&h[3].1).unwrap();
        let hash: [u8; 32] = sha2::Sha256::digest(b"abc").into();
        let msg =
            ot_proto::http::http_request_message("PUT", "/v1/blobs/x/chunks/0", 42, &nonce, &hash);
        let key = VerifyingKey::from_bytes(&id.public_key()).unwrap();
        assert!(key.verify(&msg, &Signature::from_bytes(&sig)).is_ok());
    }
}
