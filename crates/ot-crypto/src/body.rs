use ot_proto::{BlobId, DeviceId, SignedBlob, UserId, b64};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::directory::LogHead;
use crate::file::{FILE_NONCE_LEN, check_layout};
use crate::{CryptoError, FileKey, GroupKey, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    Text {
        body: String,
    },
    Link {
        body: String,
    },
    PairRequest {
        name: String,
        #[serde(with = "b64::array")]
        sign_key: [u8; 32],
        #[serde(with = "b64::array")]
        exchange_key: [u8; 32],
    },
    PairWelcome {
        keys: Vec<EpochKey>,
        roster: Roster,
        #[serde(default, skip_serializing_if = "Option::is_none", with = "b64::opt_array")]
        anchor: Option<[u8; 16]>,
    },
    Roster {
        roster: Roster,
    },
    GroupKey {
        epoch: u32,
        key: GroupKey,
        roster: Roster,
    },
    File {
        blob_id: BlobId,
        name: String,
        mime: String,
        size: u64,
        chunk_size: u32,
        chunks: u32,
        key: FileKey,
        #[serde(with = "b64::array")]
        nonce: [u8; FILE_NONCE_LEN],
        #[serde(with = "b64::array")]
        sha256: [u8; 32],
    },
    Otp(Otp),
    InviteRequest {
        user: UserId,
        device_id: DeviceId,
        name: String,
        #[serde(with = "b64::array")]
        sign_key: [u8; 32],
        #[serde(with = "b64::array")]
        exchange_key: [u8; 32],
    },
    InviteWelcome {
        cert: SignedBlob,
        admin_head: LogHead,
    },
    ServiceWelcome {
        user: UserId,
        cert: SignedBlob,
        #[serde(with = "b64::array")]
        anchor: [u8; 16],
        admin_head: LogHead,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Otp {
    pub code: String,
    pub display: String,
    pub sender_name: String,
    pub sender_domain: String,
    pub sender_verified: bool,
    pub expires_at: i64,
}

impl Otp {
    pub const MAX_TEXT_CHARS: usize = 128;

    fn check(&self) -> Result<()> {
        let bad = |why: &str| Err(CryptoError::Body(format!("otp : {why}")));
        if !(4..=16).contains(&self.code.len())
            || !self.code.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return bad("code invalide");
        }
        if self.display.chars().count() > Self::MAX_TEXT_CHARS
            || self.sender_name.chars().count() > Self::MAX_TEXT_CHARS
        {
            return bad("texte trop long");
        }
        let domain = &self.sender_domain;
        if domain.is_empty()
            || domain.len() > 253
            || !domain
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        {
            return bad("domaine invalide");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpochKey {
    pub epoch: u32,
    pub key: GroupKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    pub epoch: u32,
    pub version: u64,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub device_id: DeviceId,
    pub name: String,
    #[serde(with = "b64::array")]
    pub sign_key: [u8; 32],
    #[serde(with = "b64::array")]
    pub exchange_key: [u8; 32],
    pub added_epoch: u32,
}

impl Roster {
    pub fn member(&self, id: &DeviceId) -> Option<&Member> {
        self.members.iter().find(|m| &m.device_id == id)
    }

    pub fn contains(&self, id: &DeviceId) -> bool {
        self.member(id).is_some()
    }
}

impl Body {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Link { .. } => "link",
            Self::PairRequest { .. } => "pair_request",
            Self::PairWelcome { .. } => "pair_welcome",
            Self::Roster { .. } => "roster",
            Self::GroupKey { .. } => "group_key",
            Self::File { .. } => "file",
            Self::Otp(_) => "otp",
            Self::InviteRequest { .. } => "invite_request",
            Self::InviteWelcome { .. } => "invite_welcome",
            Self::ServiceWelcome { .. } => "service_welcome",
        }
    }

    pub fn to_json(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(serde_json::to_vec(self).expect("Body est toujours sérialisable"))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let body = serde_json::from_slice(bytes).map_err(|e| CryptoError::Body(e.to_string()))?;
        match &body {
            Self::File { size, chunk_size, chunks, .. } => {
                check_layout(*size, *chunk_size, *chunks)?
            }
            Self::Otp(otp) => otp.check()?,
            Self::InviteRequest { name, .. } if !ot_proto::valid_device_name(name) => {
                return Err(CryptoError::Body("invite_request : nom invalide".into()));
            }
            _ => {}
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_stage_1_format() {
        let body = Body::Text { body: "bonjour".into() };
        assert_eq!(&**body.to_json(), br#"{"kind":"text","body":"bonjour"}"#);
        let content = ot_proto::Content::text("bonjour");
        assert_eq!(Body::from_json(&serde_json::to_vec(&content).unwrap()).unwrap(), body);
    }

    fn file(chunks: u32) -> Body {
        Body::File {
            blob_id: "blob_01J9Z3Q5RT8YB6ZK4C2D7E9F1G".parse().unwrap(),
            name: "photo.jpg".into(),
            mime: "image/jpeg".into(),
            size: 2_500_000,
            chunk_size: crate::FILE_CHUNK_SIZE,
            chunks,
            key: FileKey::from_bytes([1; 32]),
            nonce: [2; FILE_NONCE_LEN],
            sha256: [3; 32],
        }
    }

    #[test]
    fn file_roundtrip_and_layout_check() {
        let body = file(3);
        assert_eq!(Body::from_json(&body.to_json()).unwrap(), body);
        assert_eq!(body.kind(), "file");
        assert!(!format!("{body:?}").contains("[1, 1"));
        assert!(Body::from_json(&file(4).to_json()).is_err());
    }

    #[test]
    fn pair_welcome_anchor_is_optional() {
        let roster = Roster { epoch: 1, version: 1, members: Vec::new() };
        let keys = vec![EpochKey { epoch: 1, key: GroupKey::from_bytes([4; 32]) }];
        let old = Body::PairWelcome { keys: keys.clone(), roster: roster.clone(), anchor: None };
        assert!(!String::from_utf8_lossy(&old.to_json()).contains("anchor"));
        assert_eq!(Body::from_json(&old.to_json()).unwrap(), old);

        let new = Body::PairWelcome { keys, roster, anchor: Some([5; 16]) };
        assert!(
            String::from_utf8_lossy(&new.to_json())
                .contains(r#""anchor":"BQUFBQUFBQUFBQUFBQUFBQ==""#)
        );
        assert_eq!(Body::from_json(&new.to_json()).unwrap(), new);
    }

    fn otp() -> Otp {
        Otp {
            code: "482913".into(),
            display: "G-482913".into(),
            sender_name: "Google".into(),
            sender_domain: "google.com".into(),
            sender_verified: true,
            expires_at: 1_793_628_791,
        }
    }

    #[test]
    fn otp_format() {
        let body = Body::Otp(otp());
        assert_eq!(
            &**body.to_json(),
            br#"{"kind":"otp","code":"482913","display":"G-482913","sender_name":"Google","sender_domain":"google.com","sender_verified":true,"expires_at":1793628791}"#
        );
        assert_eq!(Body::from_json(&body.to_json()).unwrap(), body);
        assert_eq!(body.kind(), "otp");
    }

    #[test]
    fn otp_rejects_bad_fields() {
        let bad = |f: fn(&mut Otp)| {
            let mut o = otp();
            f(&mut o);
            Body::from_json(&Body::Otp(o).to_json()).is_err()
        };
        assert!(bad(|o| o.code = "123".into()));
        assert!(bad(|o| o.code = "12345678901234567".into()));
        assert!(bad(|o| o.code = "48 913".into()));
        assert!(bad(|o| o.code = "４８２９１３".into()));
        assert!(bad(|o| o.sender_domain = String::new()));
        assert!(bad(|o| o.sender_domain = "Google.com".into()));
        assert!(bad(|o| o.sender_domain = "evil.com/x".into()));
        assert!(bad(|o| o.display = "x".repeat(129)));
        assert!(!bad(|o| o.code = "AB12cd".into()));
        assert!(
            Body::from_json(br#"{"kind":"otp","code":"482913","display":"","sender_name":"","sender_domain":"a.fr","sender_verified":false,"expires_at":0,"extra":1}"#)
                .is_err()
        );
    }

    #[test]
    fn rejects_unknown_kinds_and_fields() {
        assert!(Body::from_json(br#"{"kind":"teleport"}"#).is_err());
        assert!(Body::from_json(br#"{"kind":"text","body":"x","extra":1}"#).is_err());
    }
}
