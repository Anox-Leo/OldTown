//! Cœur de groupe de l'extension : [`ot_core::Secure`] au-dessus du coffre,
//! plus les appairages en attente rangés pour survivre à l'arrêt du service
//! worker (note 0023). Entrées et sorties en JSON ; aucune clé n'en sort.

use std::sync::Arc;

use ot_core::crypto::{PairingCode, PairingSecret};
use ot_core::{
    ClientError, Content, ContentKind, DeviceId, Effect, Event, GroupInfo, SecretStore, Secure,
    StoreKeyring,
};
use ot_proto::{Envelope, b64};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use zeroize::Zeroizing;

use crate::vault::{PENDING_JOIN, PENDING_OFFER, VaultStore};

/// Durée de validité d'un QR inversé affiché (comme la CLI).
pub const OFFER_LIFETIME: Duration = Duration::minutes(10);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOffer {
    /// Texte du QR, pour le réafficher après un redémarrage (il contient
    /// le secret, comme `secret`).
    qr: String,
    secret: String,
    #[serde(with = "time::serde::rfc3339")]
    expires_at: OffsetDateTime,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingJoin {
    code: String,
    sponsor: DeviceId,
}

/// Événement du cœur, pour le service worker.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventOut {
    Message {
        seq: i64,
        msg_id: String,
        from: String,
        #[serde(with = "time::serde::rfc3339")]
        created_at: OffsetDateTime,
        kind: &'static str,
        body: String,
    },
    Rejected {
        msg_id: String,
        from: String,
        reason: String,
    },
    GroupChanged {
        epoch: u32,
    },
    PairingCompleted {
        device_id: String,
        name: String,
    },
    KeyringError {
        message: String,
    },
}

impl EventOut {
    /// `None` pour les événements du client réseau, que le cœur n'émet pas.
    fn from_event(event: Event) -> Option<Self> {
        Some(match event {
            Event::Message(m) => Self::Message {
                seq: m.seq,
                msg_id: m.msg_id.to_string(),
                from: m.from.to_string(),
                created_at: m.created_at,
                kind: match m.content.kind {
                    ContentKind::Text => "text",
                    ContentKind::Link => "link",
                },
                body: m.content.body,
            },
            Event::Rejected { msg_id, from, reason } => {
                Self::Rejected { msg_id: msg_id.to_string(), from: from.to_string(), reason }
            }
            Event::GroupChanged { epoch } => Self::GroupChanged { epoch },
            Event::PairingCompleted { device_id, name } => {
                Self::PairingCompleted { device_id: device_id.to_string(), name }
            }
            Event::KeyringError { message } => Self::KeyringError { message },
            Event::Connected { .. } | Event::Disconnected { .. } | Event::AuthFailed { .. } => {
                return None;
            }
        })
    }
}

/// Résultat d'une opération : événements à traiter, enveloppes à envoyer
/// (dans l'ordre).
#[derive(Debug, Default, Serialize)]
pub struct Output {
    pub events: Vec<EventOut>,
    pub sends: Vec<Envelope>,
}

impl Output {
    fn push(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Emit(event) => self.events.extend(EventOut::from_event(event)),
                Effect::Send(env) => self.sends.push(env),
            }
        }
    }
}

/// État du groupe, pour l'écran « Groupe ».
#[derive(Debug, Serialize)]
pub struct GroupOut {
    pub epoch: u32,
    pub key_fingerprint: String,
    pub roster_version: u64,
    pub members: Vec<MemberOut>,
}

#[derive(Debug, Serialize)]
pub struct MemberOut {
    pub device_id: String,
    pub name: String,
    pub sign_fingerprint: String,
    pub exchange_fingerprint: String,
    pub added_epoch: u32,
}

impl From<GroupInfo> for GroupOut {
    fn from(g: GroupInfo) -> Self {
        Self {
            epoch: g.epoch,
            key_fingerprint: g.key_fingerprint,
            roster_version: g.roster_version,
            members: g
                .members
                .into_iter()
                .map(|m| MemberOut {
                    device_id: m.device_id.to_string(),
                    name: m.name,
                    sign_fingerprint: m.sign_fingerprint,
                    exchange_fingerprint: m.exchange_fingerprint,
                    added_epoch: m.added_epoch,
                })
                .collect(),
        }
    }
}

/// QR inversé à afficher.
#[derive(Debug, Serialize)]
pub struct OfferOut {
    pub qr: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// Appairage en attente de la réponse du parrain.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingOut {
    Offer {
        qr: String,
        #[serde(with = "time::serde::rfc3339")]
        expires_at: OffsetDateTime,
    },
    Join,
}

pub struct CoreState {
    vault: Arc<VaultStore>,
    secure: Secure,
    pending: Option<PendingOut>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl CoreState {
    /// Ouvre le cœur de l'appareil `device_id` : l'identité doit être dans le
    /// coffre. Reprend un appairage en attente encore valable.
    pub fn open(vault: Arc<VaultStore>, device_id: &str) -> Result<(Self, Output), String> {
        let me: DeviceId = device_id.parse().map_err(err)?;
        let identity = vault.identity()?.ok_or("identité de l'appareil absente")?;
        let keyring = StoreKeyring::new(vault.clone());
        let (secure, startup) = Secure::load(me, identity, Arc::new(keyring.clone()));
        let mut core = Self { vault, secure, pending: None };
        let mut out = Output::default();
        out.push(startup.into_iter().map(Effect::Emit).collect());
        core.restore(OffsetDateTime::now_utc(), &mut out);
        Ok((core, out))
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Result<Option<T>, String> {
        let Some(bytes) = self.vault.get(name).map_err(err)? else { return Ok(None) };
        serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{name} est corrompu : {e}"))
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(err)?);
        self.vault.put(name, &bytes).map_err(err)
    }

    fn report(out: &mut Output, message: String) {
        out.events.push(EventOut::KeyringError { message });
    }

    /// Reprend l'appairage en attente, ou l'oublie s'il a expiré ou si le
    /// groupe est déjà là.
    fn restore(&mut self, now: OffsetDateTime, out: &mut Output) {
        if self.secure.info().is_some() {
            self.clear_pending(out);
            return;
        }
        match self.read::<PendingOffer>(PENDING_OFFER) {
            Ok(Some(p)) if p.expires_at > now => {
                match b64::decode_array::<16>(&p.secret) {
                    Some(bytes) => {
                        self.secure.restore_offer(PairingSecret::from_bytes(bytes));
                        self.pending =
                            Some(PendingOut::Offer { qr: p.qr, expires_at: p.expires_at });
                    }
                    None => Self::report(out, format!("{PENDING_OFFER} est corrompu")),
                }
                return;
            }
            Ok(Some(_)) => self.delete(PENDING_OFFER, out),
            Ok(None) => {}
            Err(e) => Self::report(out, e),
        }
        match self.read::<PendingJoin>(PENDING_JOIN) {
            Ok(Some(p)) => match PairingCode::parse(&p.code) {
                Ok(code) => {
                    self.secure.restore_join(&code, p.sponsor);
                    self.pending = Some(PendingOut::Join);
                }
                Err(e) => Self::report(out, format!("{PENDING_JOIN} : {e}")),
            },
            Ok(None) => {}
            Err(e) => Self::report(out, e),
        }
    }

    fn delete(&self, name: &str, out: &mut Output) {
        if let Err(e) = self.vault.delete(name) {
            Self::report(out, e.to_string());
        }
    }

    fn clear_pending(&mut self, out: &mut Output) {
        self.delete(PENDING_OFFER, out);
        self.delete(PENDING_JOIN, out);
        self.pending = None;
    }

    pub fn info(&self) -> Option<GroupOut> {
        self.secure.info().map(GroupOut::from)
    }

    pub fn pending(&self) -> Option<&PendingOut> {
        self.pending.as_ref()
    }

    /// Chiffre un texte (un lien s'il n'est fait que d'une URL) pour `to`,
    /// des membres du roster ; vide : tous les autres appareils.
    pub fn seal_text(&self, text: &str, to: &[String]) -> Result<Envelope, String> {
        if text.trim().is_empty() {
            return Err("texte vide".into());
        }
        let to =
            to.iter().map(|id| id.parse::<DeviceId>().map_err(err)).collect::<Result<_, _>>()?;
        let content: Content = ot_core::classify(text);
        self.secure.seal_content(&content, to).map_err(err)
    }

    /// Traite un message livré. Les changements d'état sont dans le coffre
    /// à la sortie : les ranger avant d'acquitter.
    pub fn process(&mut self, seq: i64, env: Envelope) -> Output {
        let mut out = Output::default();
        out.push(self.secure.process(seq, env, OffsetDateTime::now_utc()));
        if self.pending.is_some() && self.secure.info().is_some() {
            self.clear_pending(&mut out);
        }
        out
    }

    /// Nouvel appareil : QR inversé à afficher, valable [`OFFER_LIFETIME`].
    pub fn offer(&mut self, name: &str) -> Result<OfferOut, String> {
        if self.secure.info().is_some() {
            return Err("cet appareil est déjà dans un groupe".into());
        }
        let offer = self.secure.offer(name).map_err(err)?;
        let expires_at = OffsetDateTime::now_utc() + OFFER_LIFETIME;
        let qr = offer.encode().to_string();
        let pending = PendingOffer {
            qr: qr.clone(),
            secret: b64::encode(offer.secret.as_bytes()),
            expires_at,
        };
        self.write(PENDING_OFFER, &pending)?;
        self.vault.delete(PENDING_JOIN).map_err(err)?;
        self.pending = Some(PendingOut::Offer { qr: qr.clone(), expires_at });
        Ok(OfferOut { qr, expires_at })
    }

    /// Nouvel appareil enregistré par le code : demande au parrain, à
    /// envoyer une fois authentifié.
    pub fn start_join(
        &mut self,
        code: &str,
        sponsor: &str,
        name: &str,
    ) -> Result<Envelope, String> {
        let parsed = PairingCode::parse(code).map_err(err)?;
        let sponsor: DeviceId = sponsor.parse().map_err(err)?;
        let env = self.secure.start_join(&parsed, sponsor.clone(), name).map_err(err)?;
        self.write(PENDING_JOIN, &PendingJoin { code: code.trim().to_owned(), sponsor })?;
        self.vault.delete(PENDING_OFFER).map_err(err)?;
        self.pending = Some(PendingOut::Join);
        Ok(env)
    }

    /// Nouvelle clé de groupe sans les membres `exclude`. Les `group_key` à
    /// envoyer sont dans `sends`.
    pub fn rotate(&mut self, exclude: &[String]) -> Result<(u32, Output), String> {
        let exclude: Vec<DeviceId> =
            exclude.iter().map(|id| id.parse().map_err(err)).collect::<Result<_, _>>()?;
        let (epoch, envs, effects) = self
            .secure
            .rotate(&exclude, OffsetDateTime::now_utc())
            .map_err(|e: ClientError| e.to_string())?;
        let mut out = Output { sends: envs, ..Output::default() };
        out.push(effects);
        Ok((epoch, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_core::crypto::PairingOffer;
    use ot_core::{Identity, MemoryKeyring, init_group};

    /// Parrain en Rust (comme le téléphone) et nouvel appareil « extension ».
    struct Sponsor {
        secure: Secure,
    }

    fn sponsor() -> Sponsor {
        let id = DeviceId::generate();
        let identity = Identity::generate();
        let keyring = Arc::new(MemoryKeyring::new());
        init_group(&*keyring, &id, "téléphone", &identity, false).unwrap();
        Sponsor { secure: Secure::load(id, identity, keyring).0 }
    }

    fn extension() -> (Arc<VaultStore>, String) {
        let vault = Arc::new(VaultStore::default());
        vault.ensure_identity().unwrap();
        (vault, DeviceId::generate().to_string())
    }

    fn welcome(sponsor: &mut Sponsor, qr: &str) -> Envelope {
        let offer = PairingOffer::parse(qr).unwrap();
        sponsor.secure.check_offer(&offer).unwrap();
        sponsor.secure.welcome_offer(&offer).unwrap().0
    }

    #[test]
    fn offer_survives_a_restart_and_is_cleared_after_joining() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault.clone(), &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        assert!(vault.has(PENDING_OFFER));
        let env = welcome(&mut phone, &offer.qr);

        // Le service worker est arrêté puis relancé : l'offre est reprise.
        let (mut core, _) = CoreState::open(vault.clone(), &id).unwrap();
        assert!(matches!(core.pending(), Some(PendingOut::Offer { .. })));
        let out = core.process(1, env);
        assert!(out.events.contains(&EventOut::GroupChanged { epoch: 1 }), "{:?}", out.events);
        assert!(core.info().is_some());
        assert!(core.pending().is_none());
        assert!(!vault.has(PENDING_OFFER), "secret effacé une fois le groupe adopté");
    }

    #[test]
    fn expired_offer_is_forgotten() {
        let (vault, id) = extension();
        let (core, _) = CoreState::open(vault.clone(), &id).unwrap();
        let old = PendingOffer {
            qr: String::new(),
            secret: b64::encode(&[1; 16]),
            expires_at: OffsetDateTime::now_utc() - Duration::SECOND,
        };
        core.write(PENDING_OFFER, &old).unwrap();
        let (core, _) = CoreState::open(vault.clone(), &id).unwrap();
        assert!(core.pending().is_none());
        assert!(!vault.has(PENDING_OFFER));
    }

    #[test]
    fn messages_flow_both_ways() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault, &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        core.process(1, welcome(&mut phone, &offer.qr));

        let env = core.seal_text("https://exemple.org", &[]).unwrap();
        let fx = phone.secure.process(2, env, OffsetDateTime::now_utc());
        assert!(fx.iter().any(|f| matches!(
            f,
            Effect::Emit(Event::Message(m)) if m.content == Content::link("https://exemple.org")
        )));

        let back = phone.secure.seal_content(&Content::text("bonjour"), vec![]).unwrap();
        let out = core.process(3, back);
        assert!(matches!(
            &out.events[..],
            [EventOut::Message { kind: "text", body, seq: 3, .. }] if body == "bonjour"
        ));
        assert!(core.seal_text("  ", &[]).is_err());
    }

    #[test]
    fn group_json_has_no_key() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault, &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        core.process(1, welcome(&mut phone, &offer.qr));
        let json = serde_json::to_string(&core.info().unwrap()).unwrap();
        assert!(json.contains("\"members\""));
        assert!(!json.contains("\"key\""), "{json}");
    }
}
