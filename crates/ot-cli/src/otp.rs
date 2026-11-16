//! Pont otp-relay → groupe (note 0045) : lit les codes publiés sur le socket
//! Unix d'otp-relay (une ligne JSON par message, contrat :
//! `tools/otp-relay/PROTOCOL.md`) et décide lesquels envoyer.
//! L'envoi lui-même passe par `ot_core::Client::send_otp`.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use ot_core::Otp;
use serde::Deserialize;
use time::OffsetDateTime;

/// Version du protocole du socket d'otp-relay comprise ici.
pub const PROTOCOL: u32 = 1;

/// Socket par défaut : `$XDG_RUNTIME_DIR/otp-relay/otp-relay.sock`.
pub fn default_socket() -> Result<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR absent : donner le socket avec --socket")?;
    Ok(PathBuf::from(dir).join("otp-relay").join("otp-relay.sock"))
}

/// Une ligne du socket, réduite à ce que le pont utilise.
#[derive(Debug, PartialEq)]
pub enum Line {
    Hello {
        protocol: u32,
    },
    Status {
        state: String,
    },
    Otp(Box<OtpEvent>),
    /// `heartbeat`, `consumed`, `expired`, réponses… : sans effet ici.
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Raw {
    Hello {
        protocol: u32,
    },
    Status {
        state: String,
    },
    Otp(Box<OtpEvent>),
    #[serde(other)]
    Other,
}

/// Événement `otp` d'otp-relay ; ses autres champs (sujet, adresse…) ne
/// quittent jamais cette machine.
#[derive(Deserialize, PartialEq)]
pub struct OtpEvent {
    pub id: String,
    code: String,
    display: String,
    confidence: f64,
    sender: Sender,
    sender_verified: bool,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

/// Jamais le code dans un `Debug`.
impl std::fmt::Debug for OtpEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtpEvent")
            .field("id", &self.id)
            .field("domain", &self.sender.domain)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Deserialize, PartialEq)]
struct Sender {
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
}

impl Line {
    pub fn parse(line: &str) -> serde_json::Result<Self> {
        Ok(match serde_json::from_str(line)? {
            Raw::Hello { protocol } => Self::Hello { protocol },
            Raw::Status { state } => Self::Status { state },
            Raw::Otp(e) => Self::Otp(e),
            Raw::Other => Self::Other,
        })
    }
}

impl OtpEvent {
    pub fn domain(&self) -> &str {
        &self.sender.domain
    }
}

/// Ce que le pont fait d'un code.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Send(Otp),
    /// Déjà envoyé : rejoué par le socket à chaque connexion, sans bruit.
    AlreadySent,
    Skip(&'static str),
}

/// Codes déjà envoyés (`id` d'otp-relay → fin de validité, secondes Unix),
/// pour ne pas renvoyer ceux que le socket rejoue à chaque connexion.
#[derive(Debug, Default, PartialEq)]
pub struct Sent(HashMap<String, i64>);

impl Sent {
    pub fn from_json(text: &str) -> Self {
        Self(serde_json::from_str(text).unwrap_or_default())
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.0)?)
    }

    pub fn insert(&mut self, id: String, expires_at: i64) {
        self.0.insert(id, expires_at);
    }

    /// Oublie les codes expirés.
    pub fn prune(&mut self, now: OffsetDateTime) {
        let now = now.unix_timestamp();
        self.0.retain(|_, exp| *exp > now);
    }
}

/// Décide de l'envoi d'un code : assez sûr, pas expiré, pas déjà envoyé,
/// au format accepté par les autres appareils.
pub fn judge(ev: &OtpEvent, now: OffsetDateTime, min_confidence: f64, sent: &Sent) -> Verdict {
    if sent.0.contains_key(&ev.id) {
        return Verdict::AlreadySent;
    }
    if ev.expires_at <= now {
        return Verdict::Skip("expiré");
    }
    if ev.confidence < min_confidence {
        return Verdict::Skip("confiance trop faible");
    }
    let code = ev.code.trim();
    if !(4..=16).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Verdict::Skip("code dans un format non pris en charge");
    }
    let domain = ev.sender.domain.trim().to_ascii_lowercase();
    let domain_ok = !domain.is_empty()
        && domain.len() <= 253
        && domain.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !domain_ok {
        return Verdict::Skip("domaine de l'expéditeur illisible");
    }
    Verdict::Send(Otp {
        code: code.to_owned(),
        display: truncate(&ev.display, Otp::MAX_TEXT_CHARS),
        sender_name: truncate(&ev.sender.name, Otp::MAX_TEXT_CHARS),
        sender_domain: domain,
        sender_verified: ev.sender_verified,
        expires_at: ev.expires_at.unix_timestamp(),
    })
}

fn truncate(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-02 14:05 UTC);

    fn event(extra: &str) -> String {
        let base = r#"{"type":"otp","id":"3f9c1a7e52b04d18","code":"482913","display":"G-482913",
            "confidence":1.0,"sender":{"name":"Google","address":"noreply@google.com","domain":"google.com"},
            "sender_verified":true,"auth":{"dmarc":"pass"},"to":"moi@example.org","subject":"code",
            "received_at":"2026-11-02T14:03:11.000Z","detected_at":"2026-11-02T14:03:11.042Z",
            "expires_at":"2026-11-02T14:13:11.000Z","source":{"mailbox":"INBOX","uid":48121}"#;
        format!("{base}{extra}}}")
    }

    fn parsed(extra: &str) -> Box<OtpEvent> {
        match Line::parse(&event(extra)).unwrap() {
            Line::Otp(e) => e,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_socket_lines() {
        let hello = r#"{"type":"hello","protocol":1,"daemon":"otp-relay/0.1","code_ttl":600}"#;
        assert_eq!(Line::parse(hello).unwrap(), Line::Hello { protocol: 1 });
        let status = r#"{"type":"status","state":"connected","detail":"","since":"x"}"#;
        assert_eq!(Line::parse(status).unwrap(), Line::Status { state: "connected".into() });
        let beat = r#"{"type":"heartbeat","at":"x","imap":"connected"}"#;
        assert_eq!(Line::parse(beat).unwrap(), Line::Other);
        assert!(Line::parse("pas du json").is_err());
        // `replay` et tout champ inconnu sont acceptés.
        assert_eq!(parsed(r#","replay":true"#).id, "3f9c1a7e52b04d18");
    }

    #[test]
    fn debug_hides_the_code() {
        assert!(!format!("{:?}", parsed("")).contains("482913"));
    }

    #[test]
    fn sends_a_good_code_once() {
        let ev = parsed("");
        let mut sent = Sent::default();
        let Verdict::Send(otp) = judge(&ev, NOW, 0.6, &sent) else { panic!() };
        assert_eq!(
            otp,
            Otp {
                code: "482913".into(),
                display: "G-482913".into(),
                sender_name: "Google".into(),
                sender_domain: "google.com".into(),
                sender_verified: true,
                expires_at: datetime!(2026-11-02 14:13:11 UTC).unix_timestamp(),
            }
        );
        sent.insert(ev.id.clone(), otp.expires_at);
        assert_eq!(judge(&ev, NOW, 0.6, &sent), Verdict::AlreadySent);
    }

    #[test]
    fn skips_doubtful_or_stale_codes() {
        let sent = Sent::default();
        let late = datetime!(2026-11-02 14:20 UTC);
        assert_eq!(judge(&parsed(""), late, 0.6, &sent), Verdict::Skip("expiré"));
        let mut low = parsed("");
        low.confidence = 0.5;
        assert_eq!(judge(&low, NOW, 0.6, &sent), Verdict::Skip("confiance trop faible"));
        let mut odd = parsed("");
        odd.code = "48 29 13".into();
        assert!(matches!(judge(&odd, NOW, 0.6, &sent), Verdict::Skip(_)));
        let mut anon = parsed("");
        anon.sender.domain = String::new();
        assert!(matches!(judge(&anon, NOW, 0.6, &sent), Verdict::Skip(_)));
    }

    #[test]
    fn normalizes_domain_and_texts() {
        let mut ev = parsed("");
        ev.sender.domain = " Google.COM ".into();
        ev.sender.name = "x".repeat(300);
        let Verdict::Send(otp) = judge(&ev, NOW, 0.6, &Sent::default()) else { panic!() };
        assert_eq!(otp.sender_domain, "google.com");
        assert_eq!(otp.sender_name.chars().count(), Otp::MAX_TEXT_CHARS);
    }

    #[test]
    fn sent_survives_a_restart_until_expiry() {
        let mut sent = Sent::default();
        sent.insert("a".into(), NOW.unix_timestamp() + 60);
        sent.insert("b".into(), NOW.unix_timestamp() - 1);
        sent.prune(NOW);
        let back = Sent::from_json(&sent.to_json().unwrap());
        assert_eq!(back, sent);
        assert_eq!(back.0.len(), 1);
        assert_eq!(Sent::from_json("illisible"), Sent::default());
    }
}
