use ot_crypto::{
    Body, CHUNK_TAG_LEN, FILE_NONCE_LEN, FileCipher, FileKey, chunk_count, sanitize_file_name,
};
use ot_proto::http::{HEADER_DEVICE, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIME, HTTP_NONCE_LEN};
use ot_proto::{BlobId, DeviceId, Ulid, UserId, b64};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::Identity;
use crate::types::Origin;

pub const FILE_OFFER_LIFETIME: Duration = Duration::days(7);

pub const MAX_FILE_OFFERS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub msg_id: Ulid,
    pub from: DeviceId,
    pub created_at: OffsetDateTime,
    pub blob_id: BlobId,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub origin: Origin,
}

#[derive(Debug, Clone)]
pub struct ReceivedFile {
    pub seq: i64,
    pub file: FileInfo,
    pub to: Vec<DeviceId>,
}

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
    #[serde(default, skip_serializing_if = "Origin::is_mine")]
    pub origin: Origin,
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
            origin: self.origin.clone(),
        }
    }

    pub fn cipher(&self) -> FileCipher {
        FileCipher::new(&self.key, &self.nonce)
    }

    pub fn sealed_chunk_size(&self) -> u32 {
        self.chunk_size + CHUNK_TAG_LEN
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingUpload {
    pub source_id: String,
    pub to: Vec<DeviceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_user: Option<UserId>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub meta: FileMeta,
}

pub(crate) fn stored_from_body(
    msg_id: Ulid,
    from: DeviceId,
    created_at: OffsetDateTime,
    now: OffsetDateTime,
    body: Body,
    origin: Origin,
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
        origin,
    })
}

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

pub(crate) fn add_offer(offers: &mut Vec<StoredFile>, offer: StoredFile, now: OffsetDateTime) {
    offers.retain(|o| o.blob_id != offer.blob_id);
    offers.push(offer);
    prune_offers(offers, now);
}

pub fn prune_offers(offers: &mut Vec<StoredFile>, now: OffsetDateTime) -> bool {
    let before = offers.len();
    offers.retain(|o| o.received_at + FILE_OFFER_LIFETIME > now);
    if offers.len() > MAX_FILE_OFFERS {
        offers.sort_by_key(|o| o.received_at);
        offers.drain(..offers.len() - MAX_FILE_OFFERS);
    }
    offers.len() != before
}

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

pub fn signed_headers(
    identity: &Identity,
    device_id: &DeviceId,
    method: &str,
    path: &str,
    body: &[u8],
    now_ms: u64,
) -> Vec<(&'static str, String)> {
    let mut nonce = [0; HTTP_NONCE_LEN];
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
            origin: Origin::default(),
        }
    }

    #[test]
    fn offers_expire_and_are_bounded() {
        let now = OffsetDateTime::now_utc();
        let mut offers = vec![offer("blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V", now - Duration::days(8))];
        add_offer(&mut offers, offer("blob_01J9Z3R7XK2M4N6P8Q0R2S4T6W", now), now);
        assert_eq!(offers.len(), 1);
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
