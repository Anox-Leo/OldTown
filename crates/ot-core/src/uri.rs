use ot_crypto::{InviteCode, PairingCode};
use ot_proto::Content;

use crate::ClientError;

pub const PAIR_URI_PREFIX: &str = "oldtown://pair?";

#[derive(Clone, PartialEq, Eq)]
pub struct PairUri {
    pub server_url: String,
    pub code: String,
}

impl std::fmt::Debug for PairUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairUri").field("server_url", &self.server_url).finish_non_exhaustive()
    }
}

impl PairUri {
    pub fn encode(&self) -> String {
        format!("{PAIR_URI_PREFIX}s={}&c={}", escape(&self.server_url), escape(&self.code))
    }

    pub fn parse(text: &str) -> Result<Self, ClientError> {
        let (server_url, code) = parse_query(text, PAIR_URI_PREFIX, "QR « rejoindre »")?;
        PairingCode::parse(&code).map_err(|e| ClientError::Pairing(e.to_string()))?;
        Ok(Self { server_url, code })
    }
}

pub const INVITE_URI_PREFIX: &str = "oldtown://invite?";

#[derive(Clone, PartialEq, Eq)]
pub struct InviteUri {
    pub server_url: String,
    pub code: String,
}

impl std::fmt::Debug for InviteUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InviteUri").field("server_url", &self.server_url).finish_non_exhaustive()
    }
}

impl InviteUri {
    pub fn encode(&self) -> String {
        format!("{INVITE_URI_PREFIX}s={}&c={}", escape(&self.server_url), escape(&self.code))
    }

    pub fn parse(text: &str) -> Result<Self, ClientError> {
        let (server_url, code) = parse_query(text, INVITE_URI_PREFIX, "invitation")?;
        InviteCode::parse(&code).map_err(|e| ClientError::Pairing(e.to_string()))?;
        Ok(Self { server_url, code })
    }
}

fn parse_query(text: &str, prefix: &str, what: &str) -> Result<(String, String), ClientError> {
    let bad = |why: &str| ClientError::Pairing(format!("{what} invalide : {why}"));
    let query = text.trim().strip_prefix(prefix).ok_or_else(|| bad("préfixe"))?;
    let (mut server_url, mut code) = (None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or_else(|| bad("paramètre"))?;
        let value = unescape(value).ok_or_else(|| bad("encodage"))?;
        match key {
            "s" => server_url = Some(value),
            "c" => code = Some(value),
            _ => {}
        }
    }
    let server_url = server_url.filter(|s| !s.is_empty()).ok_or_else(|| bad("relais"))?;
    let code = code.ok_or_else(|| bad("code"))?;
    Ok((server_url, code))
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b':' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unescape(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn classify(text: &str) -> Content {
    let trimmed = text.trim();
    let is_link = (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        && !trimmed.contains(char::is_whitespace);
    if is_link { Content::link(trimmed) } else { Content::text(text) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_crypto::PairingSecret;
    use ot_proto::ContentKind;

    fn code() -> String {
        PairingCode { token: [1; 10], secret: PairingSecret::from_bytes([2; 16]) }
            .encode()
            .to_string()
    }

    #[test]
    fn pair_uri_roundtrip() {
        let uri = PairUri { server_url: "wss://relais.exemple.ts.net:7878".into(), code: code() };
        let text = uri.encode();
        assert_eq!(text, format!("oldtown://pair?s=wss://relais.exemple.ts.net:7878&c={}", code()));
        assert_eq!(PairUri::parse(&text).unwrap(), uri);
        let odd = PairUri { server_url: "wss://r/a?b=c&d#e f".into(), code: code() };
        assert_eq!(PairUri::parse(&odd.encode()).unwrap(), odd);
        assert!(!format!("{uri:?}").contains("ot1-"));
    }

    #[test]
    fn pair_uri_rejects_garbage() {
        for bad in [
            "".to_owned(),
            "https://exemple.org".to_owned(),
            format!("oldtown://pair?c={}", code()),
            "oldtown://pair?s=wss://r&c=ot1-court".to_owned(),
            format!("oldtown://pair?s=%ZZ&c={}", code()),
            format!("oldtown://sponsor?s=wss://r&c={}", code()),
        ] {
            assert!(PairUri::parse(&bad).is_err(), "{bad}");
        }
        let extra = format!("oldtown://pair?s=wss://r&c={}&x=1", code());
        assert!(PairUri::parse(&extra).is_ok(), "paramètre futur toléré");
    }

    #[test]
    fn links_are_detected() {
        assert_eq!(classify("https://example.org/a?b").kind, ContentKind::Link);
        assert_eq!(classify(" http://x.fr "), Content::link("http://x.fr"));
        assert_eq!(classify("voir https://x.fr").kind, ContentKind::Text);
        assert_eq!(classify("bonjour").kind, ContentKind::Text);
        assert_eq!(classify("  garde les espaces  ").body, "  garde les espaces  ");
    }
}
