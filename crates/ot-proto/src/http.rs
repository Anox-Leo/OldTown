use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{BlobId, DeviceId};

pub const BLOBS_PATH: &str = "/v1/blobs";

pub const HEADER_DEVICE: &str = "ot-device";
pub const HEADER_TIME: &str = "ot-time";
pub const HEADER_NONCE: &str = "ot-nonce";
pub const HEADER_SIGNATURE: &str = "ot-signature";

pub const HTTP_DOMAIN: &[u8] = b"ot-http-v1";

pub const HTTP_NONCE_LEN: usize = 16;

pub const HTTP_WINDOW_MS: i64 = 60_000;

pub const MAX_SEALED_CHUNK: u32 = (4 << 20) + 16;

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

pub fn blob_chunk_count(size: u64, chunk_size: u32) -> Option<u32> {
    if chunk_size == 0 {
        return None;
    }
    u32::try_from(size.div_ceil(u64::from(chunk_size)).max(1)).ok()
}

pub fn blob_chunk_len(size: u64, chunk_size: u32, index: u32) -> Option<u32> {
    let chunks = blob_chunk_count(size, chunk_size)?;
    if index >= chunks {
        return None;
    }
    let start = u64::from(index) * u64::from(chunk_size);
    Some(size.saturating_sub(start).min(u64::from(chunk_size)) as u32)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBlob {
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    #[serde(default)]
    pub to: Vec<DeviceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobCreated {
    pub blob_id: BlobId,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobStatus {
    pub blob_id: BlobId,
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    pub missing: Vec<u32>,
    pub complete: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpError {
    pub error: HttpErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotAllowed,
    NotFound,
    Incomplete,
    TooLarge,
    QuotaExceeded,
    Busy,
    Internal,
    #[serde(other)]
    Unknown,
}

impl HttpErrorCode {
    pub fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden | Self::NotAllowed => 403,
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
        let (size, chunk) = (2_500_000 + 3 * 16, (1 << 20) + 16);
        assert_eq!(blob_chunk_count(size, chunk), Some(3));
        assert_eq!(blob_chunk_len(size, chunk, 0), Some(chunk));
        assert_eq!(blob_chunk_len(size, chunk, 2), Some(2_500_000 - 2 * (1 << 20) + 16));
        assert_eq!(blob_chunk_len(size, chunk, 3), None);
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
