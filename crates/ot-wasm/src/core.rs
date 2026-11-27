use std::sync::Arc;

use ot_core::crypto::{CHUNK_TAG_LEN, chunk_count, chunk_len, sealed_size};
use ot_core::crypto::{PairingCode, PairingSecret};
use ot_core::directory::{DirectoryRead, ReadStep};
use ot_core::files::new_file_meta;
use ot_core::{
    BlobId, ClientError, Content, ContentKind, DeviceId, Effect, Event, FileInfo, GroupInfo,
    NextEntry, Origin, Otp, OwnEntry, PendingUpload, Person, Publish, SecretStore, Secure,
    StoreKeyring, UserId,
};
use ot_proto::{Envelope, Frame, SignedBlob, b64};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use zeroize::Zeroizing;

use crate::vault::{PENDING_JOIN, PENDING_OFFER, VaultStore};

pub const OFFER_LIFETIME: Duration = Duration::minutes(10);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOffer {
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
        #[serde(flatten)]
        origin: OriginOut,
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
    File {
        seq: i64,
        #[serde(flatten)]
        file: FileOut,
    },
    DirectoryChanged,
    DirectoryRejected {
        user: String,
        reason: String,
    },
    DirectoryFork,
    KeysChanged {
        user: String,
    },
    PersonJoined {
        user: String,
        name: String,
        device_name: String,
        fingerprint: String,
    },
    Joined {
        user: String,
    },
    InviteFailed {
        reason: String,
    },
    Read {
        user: String,
        until: u64,
    },
    Otp {
        seq: i64,
        msg_id: String,
        from: String,
        #[serde(with = "time::serde::rfc3339")]
        created_at: OffsetDateTime,
        #[serde(flatten)]
        otp: Otp,
        #[serde(flatten)]
        origin: OriginOut,
        auto_fill: bool,
    },
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct OriginOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_user: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_service: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_user: Option<String>,
}

impl From<&Origin> for OriginOut {
    fn from(o: &Origin) -> Self {
        Self {
            from_user: o.from_user.as_ref().map(ToString::to_string),
            from_service: o.from_service,
            to_user: o.to_user.as_ref().map(ToString::to_string),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FileOut {
    pub msg_id: String,
    pub from: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub blob_id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(flatten)]
    pub origin: OriginOut,
}

impl From<FileInfo> for FileOut {
    fn from(f: FileInfo) -> Self {
        Self {
            msg_id: f.msg_id.to_string(),
            from: f.from.to_string(),
            created_at: f.created_at,
            blob_id: f.blob_id.to_string(),
            name: f.name,
            mime: f.mime,
            size: f.size,
            origin: (&f.origin).into(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Geometry {
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    pub sealed_size: u64,
    pub sealed_chunk_size: u32,
}

impl Geometry {
    pub fn new(size: u64, chunk_size: u32) -> Result<Self, String> {
        let too_big = || "fichier trop gros".to_owned();
        Ok(Self {
            size,
            chunk_size,
            chunks: chunk_count(size, chunk_size).ok_or_else(too_big)?,
            sealed_size: sealed_size(size, chunk_size).ok_or_else(too_big)?,
            sealed_chunk_size: chunk_size + CHUNK_TAG_LEN,
        })
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UploadOut {
    pub blob_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub to: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_user: Option<String>,
    #[serde(flatten)]
    pub geometry: Geometry,
}

impl UploadOut {
    fn new(p: &PendingUpload) -> Result<Self, String> {
        Ok(Self {
            blob_id: p.meta.blob_id.to_string(),
            expires_at: p.expires_at,
            to: p.to.iter().map(ToString::to_string).collect(),
            to_user: p.to_user.as_ref().map(ToString::to_string),
            geometry: Geometry::new(p.meta.size, p.meta.chunk_size)?,
        })
    }
}

impl EventOut {
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
                origin: (&m.origin).into(),
            },
            Event::Rejected { msg_id, from, reason } => {
                Self::Rejected { msg_id: msg_id.to_string(), from: from.to_string(), reason }
            }
            Event::GroupChanged { epoch } => Self::GroupChanged { epoch },
            Event::PairingCompleted { device_id, name } => {
                Self::PairingCompleted { device_id: device_id.to_string(), name }
            }
            Event::KeyringError { message } => Self::KeyringError { message },
            Event::File(f) => Self::File { seq: f.seq, file: f.file.into() },
            Event::Otp(o) => Self::Otp {
                auto_fill: o.auto_fill(),
                origin: (&o.origin).into(),
                seq: o.seq,
                msg_id: o.msg_id.to_string(),
                from: o.from.to_string(),
                created_at: o.created_at,
                otp: o.otp,
            },
            Event::Connected { .. } | Event::Disconnected { .. } | Event::AuthFailed { .. } => {
                return None;
            }
            Event::DirectoryChanged => Self::DirectoryChanged,
            Event::DirectoryRejected { user, reason } => {
                Self::DirectoryRejected { user: user.to_string(), reason }
            }
            Event::DirectoryFork => Self::DirectoryFork,
            Event::KeysChanged { user } => Self::KeysChanged { user: user.to_string() },
            Event::PersonJoined { user, name, device_name, fingerprint } => {
                Self::PersonJoined { user: user.to_string(), name, device_name, fingerprint }
            }
            Event::Joined { user } => Self::Joined { user: user.to_string() },
            Event::InviteFailed { reason } => Self::InviteFailed { reason },
            Event::Read { user, until } => Self::Read { user: user.to_string(), until },
        })
    }
}

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

#[derive(Debug, Serialize)]
pub struct OfferOut {
    pub qr: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

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

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PersonOut {
    pub user: String,
    pub name: String,
    pub role: &'static str,
    pub muted: bool,
    pub devices: Vec<PersonDeviceOut>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PersonDeviceOut {
    pub device_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub reachable: bool,
    pub default: bool,
    pub up_to_date: bool,
    pub online: bool,
}

impl From<Person> for PersonOut {
    fn from(p: Person) -> Self {
        Self {
            user: p.id.to_string(),
            name: p.name,
            role: p.role.as_str(),
            muted: p.muted,
            devices: p
                .devices
                .into_iter()
                .map(|d| PersonDeviceOut {
                    device_id: d.id.to_string(),
                    name: d.name,
                    kind: d.kind.map(|k| k.as_str()),
                    reachable: d.reachable,
                    default: d.default,
                    up_to_date: d.up_to_date,
                    online: d.online,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct DirectoryStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    pub done: bool,
    #[serde(flatten)]
    pub out: Output,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PublishIn {
    IfBehind,
    Certify { user: UserId, cert: SignedBlob },
    Revoke { user: UserId },
}

#[derive(Debug, Serialize)]
pub struct PublishOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<ot_core::crypto::directory::LogHead>,
}

#[derive(Debug, Default, Serialize)]
pub struct AdminRootOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
    pub adopted: bool,
    #[serde(flatten)]
    pub out: Output,
}

pub struct CoreState {
    vault: Arc<VaultStore>,
    secure: Secure,
    pending: Option<PendingOut>,
    read: Option<DirectoryRead>,
    publishing: Option<OwnEntry>,
    publishing_services: Vec<OwnEntry>,
    admin_root: Option<SignedBlob>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl CoreState {
    pub fn open(vault: Arc<VaultStore>, device_id: &str) -> Result<(Self, Output), String> {
        let me: DeviceId = device_id.parse().map_err(err)?;
        let identity = vault.identity()?.ok_or("identité de l'appareil absente")?;
        let keyring = StoreKeyring::new(vault.clone());
        let (secure, startup) = Secure::load(me, identity, Arc::new(keyring.clone()));
        let mut core = Self {
            vault,
            secure,
            pending: None,
            read: None,
            publishing: None,
            publishing_services: Vec::new(),
            admin_root: None,
        };
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

    pub fn seal_text(&self, text: &str, to: &[String]) -> Result<Envelope, String> {
        if text.trim().is_empty() {
            return Err("texte vide".into());
        }
        let to =
            to.iter().map(|id| id.parse::<DeviceId>().map_err(err)).collect::<Result<_, _>>()?;
        let content: Content = ot_core::classify(text);
        self.secure.seal_content(&content, to).map_err(err)
    }

    pub fn process(&mut self, seq: i64, env: Envelope) -> Output {
        let mut out = Output::default();
        out.push(self.secure.process(seq, env, OffsetDateTime::now_utc()));
        if self.pending.is_some() && self.secure.info().is_some() {
            self.clear_pending(&mut out);
        }
        out
    }

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

    pub fn upload(&self, source_id: &str, size: u64) -> Result<Option<UploadOut>, String> {
        let now = OffsetDateTime::now_utc();
        self.secure
            .upload(source_id, size, now)
            .map_err(err)?
            .as_ref()
            .map(UploadOut::new)
            .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_upload(
        &self,
        source_id: &str,
        name: &str,
        mime: &str,
        size: u64,
        chunk_size: u32,
        sha256: &[u8],
        to: &[String],
        to_user: Option<&str>,
        blob_id: &str,
        expires_at: &str,
    ) -> Result<UploadOut, String> {
        let sha256: [u8; 32] = sha256.try_into().map_err(|_| "SHA-256 de 32 octets attendu")?;
        let blob: BlobId = blob_id.parse().map_err(err)?;
        let to: Vec<DeviceId> =
            to.iter().map(|id| id.parse::<DeviceId>().map_err(err)).collect::<Result<_, _>>()?;
        let to_user = to_user.map(str::parse::<UserId>).transpose().map_err(err)?;
        if to_user.is_some() && to.is_empty() {
            return Err("envoi à une personne sans appareil destinataire".into());
        }
        let expires_at =
            OffsetDateTime::parse(expires_at, &time::format_description::well_known::Rfc3339)
                .map_err(err)?;
        let meta =
            new_file_meta(blob, name, mime, size, chunk_size, sha256).ok_or("fichier trop gros")?;
        let pending =
            PendingUpload { source_id: source_id.to_owned(), to, to_user, expires_at, meta };
        let out = UploadOut::new(&pending)?;
        self.secure.save_upload(pending, OffsetDateTime::now_utc()).map_err(err)?;
        Ok(out)
    }

    fn pending_upload(&self, source_id: &str) -> Result<PendingUpload, String> {
        self.secure
            .uploads_of(source_id)
            .map_err(err)?
            .into_iter()
            .next()
            .ok_or_else(|| "aucun envoi en cours pour ce fichier".to_owned())
    }

    pub fn seal_chunk(&self, source_id: &str, index: u32, plain: &[u8]) -> Result<Vec<u8>, String> {
        let p = self.pending_upload(source_id)?;
        let m = &p.meta;
        if chunk_len(m.size, m.chunk_size, index) != Some(plain.len() as u32) {
            return Err(format!("morceau {index} de taille inattendue"));
        }
        let cipher = ot_core::crypto::FileCipher::new(&m.key, &m.nonce);
        cipher.seal_chunk(index, index + 1 == m.chunks, plain).map_err(err)
    }

    pub fn seal_file(&self, source_id: &str) -> Result<Envelope, String> {
        let p = self.pending_upload(source_id)?;
        match p.to_user {
            Some(_) => self.secure.seal_file_to(&p.meta, p.to),
            None => self.secure.seal_file(&p.meta, p.to),
        }
        .map_err(err)
    }

    pub fn forget_upload(&self, source_id: &str) -> Result<(), String> {
        self.secure.forget_upload(source_id).map_err(err)
    }

    pub fn files(&self) -> Result<Vec<FileOut>, String> {
        Ok(self.secure.files().map_err(err)?.into_iter().map(FileOut::from).collect())
    }

    fn stored(&self, blob_id: &str) -> Result<ot_core::StoredFile, String> {
        let blob: BlobId = blob_id.parse().map_err(err)?;
        self.secure.file(&blob).map_err(err)?.ok_or_else(|| "fichier inconnu".to_owned())
    }

    pub fn file_geometry(&self, blob_id: &str) -> Result<Geometry, String> {
        let f = self.stored(blob_id)?;
        Geometry::new(f.size, f.chunk_size)
    }

    pub fn open_chunk(&self, blob_id: &str, index: u32, sealed: &[u8]) -> Result<Vec<u8>, String> {
        let f = self.stored(blob_id)?;
        let plain = f
            .cipher()
            .open_chunk(index, index + 1 == f.chunks, sealed)
            .map_err(|_| format!("morceau {index} altéré"))?;
        if chunk_len(f.size, f.chunk_size, index) != Some(plain.len() as u32) {
            return Err(format!("morceau {index} incohérent"));
        }
        Ok(plain.to_vec())
    }

    pub fn check_file(&self, blob_id: &str, sha256: &[u8]) -> Result<bool, String> {
        Ok(self.stored(blob_id)?.sha256 == sha256)
    }

    pub fn forget_file(&self, blob_id: &str) -> Result<(), String> {
        let blob: BlobId = blob_id.parse().map_err(err)?;
        self.secure.forget_file(&blob).map_err(err)
    }

    pub fn directory_start(&mut self) -> Option<Frame> {
        if !self.secure.has_anchor() {
            return None;
        }
        let (read, frame) = DirectoryRead::start(self.secure.directory_known());
        self.read = Some(read);
        Some(frame)
    }

    pub fn directory_page(&mut self, frame: Frame) -> Result<DirectoryStep, String> {
        let Frame::Directory { me, admin, users, next } = frame else {
            return Err("trame directory attendue".into());
        };
        let mut read = self.read.take().ok_or("aucune lecture de l'annuaire en cours")?;
        let mut step = DirectoryStep::default();
        match read.page(me, admin, users, next).map_err(err)? {
            ReadStep::Request(frame) => {
                step.frame = Some(*frame);
                self.read = Some(read);
            }
            ReadStep::Round { me, admin, users } => {
                let (effects, again) = self.secure.apply_directory(&me, &admin, &users);
                step.out.push(effects);
                match again.then(|| read.next_round(self.secure.directory_known())).flatten() {
                    Some(frame) => {
                        step.frame = Some(frame);
                        self.read = Some(read);
                    }
                    None => step.done = true,
                }
            }
        }
        Ok(step)
    }

    pub fn people(&self) -> Vec<PersonOut> {
        self.secure.people().into_iter().map(PersonOut::from).collect()
    }

    pub fn declaration_frame(
        &self,
        kind: Option<String>,
        features: &[String],
    ) -> Result<Option<Frame>, String> {
        let kind = kind
            .map(|k| serde_json::from_value::<ot_proto::DeviceKind>(serde_json::Value::String(k)))
            .transpose()
            .map_err(err)?;
        let features: Vec<ot_proto::Feature> =
            features.iter().map(|f| f.parse().map_err(err)).collect::<Result<_, _>>()?;
        Ok(self.secure.declaration(kind, &features))
    }

    pub fn seal_read(&self, user: &str, until: u64) -> Result<Option<Envelope>, String> {
        let user: UserId = user.parse().map_err(err)?;
        self.secure.seal_read(&user, until).map_err(err)
    }

    pub fn recipients(&self, user: &str) -> Result<Vec<String>, String> {
        let user: UserId = user.parse().map_err(err)?;
        let to = self.secure.recipients(&user).map_err(err)?;
        Ok(to.iter().map(ToString::to_string).collect())
    }

    pub fn seal_text_to(&self, text: &str, user: &str) -> Result<Envelope, String> {
        if text.trim().is_empty() {
            return Err("texte vide".into());
        }
        let user: UserId = user.parse().map_err(err)?;
        let content: Content = ot_core::classify(text);
        self.secure.seal_content_to(&content, &user).map_err(err)
    }

    pub fn next_entry(&mut self, what: PublishIn) -> Result<PublishOut, String> {
        let what = match what {
            PublishIn::IfBehind => Publish::IfBehind,
            PublishIn::Certify { user, cert } => Publish::Certify(user, cert),
            PublishIn::Revoke { user } => Publish::Revoke(user),
        };
        match self.secure.next_entry(&what).map_err(err)? {
            NextEntry::Done(head) => Ok(PublishOut { frame: None, head }),
            NextEntry::Entry(own) => {
                let frame = Frame::RosterPublish {
                    user: own.user.clone(),
                    seq: own.entry.seq,
                    entry: own.entry.entry.clone(),
                };
                let head = Some(own.head());
                self.publishing = Some(*own);
                Ok(PublishOut { frame: Some(frame), head })
            }
        }
    }

    pub fn entry_published(&mut self) -> Result<Output, String> {
        let own = self.publishing.take().ok_or("aucune publication en cours")?;
        let mut out = Output::default();
        out.push(self.secure.entry_published(&own));
        Ok(out)
    }

    pub fn create_invite(
        &mut self,
        user: &str,
        name: &str,
        token: &str,
        expires_at: &str,
    ) -> Result<String, String> {
        let user: UserId = user.parse().map_err(err)?;
        let token =
            b64::decode_array::<{ ot_core::crypto::TOKEN_LEN }>(token).ok_or("jeton invalide")?;
        let expires_at =
            OffsetDateTime::parse(expires_at, &time::format_description::well_known::Rfc3339)
                .map_err(err)?;
        let now = OffsetDateTime::now_utc();
        let code = self.secure.create_invite(user, name, token, expires_at, now).map_err(err)?;
        Ok(code.encode().to_string())
    }

    pub fn invites_to_certify(&self) -> Vec<String> {
        self.secure.invites_to_certify().iter().map(ToString::to_string).collect()
    }

    pub fn invite_cert(&mut self, user: &str) -> Result<(SignedBlob, bool), String> {
        let user: UserId = user.parse().map_err(err)?;
        self.secure.invite_cert(&user, OffsetDateTime::now_utc()).map_err(err)
    }

    pub fn invite_certified(&mut self, user: &str) -> Result<(), String> {
        let user: UserId = user.parse().map_err(err)?;
        self.secure.invite_certified(&user).map_err(err)
    }

    pub fn invite_welcome(
        &self,
        user: &str,
        head: ot_core::crypto::directory::LogHead,
    ) -> Result<Envelope, String> {
        let user: UserId = user.parse().map_err(err)?;
        self.secure.invite_welcome(&user, head).map_err(err)
    }

    pub fn invite_done(&mut self, user: &str) -> Result<Output, String> {
        let user: UserId = user.parse().map_err(err)?;
        let mut out = Output::default();
        out.push(self.secure.invite_done(&user).map_err(err)?);
        Ok(out)
    }

    pub fn invite_request(&self) -> Option<Result<Envelope, String>> {
        self.secure.invite_request().map(|r| r.map_err(err))
    }

    pub fn awaits_directory(&self) -> bool {
        self.secure.awaits_directory()
    }

    pub fn admin_init_start(&self) -> Result<Frame, String> {
        if self.secure.info().is_none() {
            return Err(ClientError::NoGroup.to_string());
        }
        if self.secure.has_anchor() {
            return Err("certificat de l'admin déjà publié".into());
        }
        Ok(Frame::DirectoryGet { known: Default::default(), after: None })
    }

    pub fn admin_root(&mut self, frame: Frame) -> Result<AdminRootOut, String> {
        let Frame::Directory { me, admin, users, .. } = frame else {
            return Err("trame directory attendue".into());
        };
        let raw =
            users.into_iter().find(|u| u.id == admin).ok_or("l'admin manque dans l'annuaire")?;
        if me != admin {
            return Err("cet appareil n'est pas à l'admin".into());
        }
        let mut res = AdminRootOut::default();
        if let Some(cert) = &raw.cert {
            res.out.push(self.secure.adopt_existing_root(&admin, cert).map_err(err)?);
            res.adopted = true;
            self.admin_root = None;
            return Ok(res);
        }
        let (cert, entry) =
            self.secure.admin_root(&admin, &raw.name, OffsetDateTime::now_utc()).map_err(err)?;
        res.frame = Some(Frame::AdminInit { cert: cert.clone(), entry });
        self.admin_root = Some(cert);
        Ok(res)
    }

    pub fn adopt_admin_root(&mut self, user: &str) -> Result<Output, String> {
        let user: UserId = user.parse().map_err(err)?;
        let cert = self.admin_root.as_ref().ok_or("aucun admin_init en cours")?;
        let ours = ot_core::crypto::directory::UserCert::read_unverified(cert)
            .is_ok_and(|c| c.user == user);
        if !ours {
            return Err("admin_initialized pour une autre personne".into());
        }
        let mut out = Output::default();
        out.push(self.secure.adopt_admin_root(cert));
        self.admin_root = None;
        Ok(out)
    }

    pub fn certified_by(&self, device: &str) -> Result<Vec<String>, String> {
        let device: DeviceId = device.parse().map_err(err)?;
        Ok(self.secure.certified_by(&device).iter().map(ToString::to_string).collect())
    }

    pub fn recertify(&self, user: &str, name: Option<&str>) -> Result<SignedBlob, String> {
        let user: UserId = user.parse().map_err(err)?;
        self.secure.recertify(&user, name, OffsetDateTime::now_utc()).map_err(err)
    }

    pub fn services_to_republish(&self) -> Vec<String> {
        self.secure.services_to_republish().iter().map(ToString::to_string).collect()
    }

    pub fn next_service_entry(&mut self, user: &str) -> Result<PublishOut, String> {
        let user: UserId = user.parse().map_err(err)?;
        match self.secure.next_service_entry(&user, None).map_err(err)? {
            NextEntry::Done(head) => Ok(PublishOut { frame: None, head }),
            NextEntry::Entry(own) => {
                let frame = Frame::RosterPublish {
                    user: own.user.clone(),
                    seq: own.entry.seq,
                    entry: own.entry.entry.clone(),
                };
                let head = Some(own.head());
                self.publishing_services.retain(|o| o.user != own.user);
                self.publishing_services.push(*own);
                Ok(PublishOut { frame: Some(frame), head })
            }
        }
    }

    pub fn service_entry_published(&mut self, user: &str, seq: u64) -> Result<Output, String> {
        let user: UserId = user.parse().map_err(err)?;
        let i = self
            .publishing_services
            .iter()
            .position(|o| o.user == user && o.entry.seq == seq)
            .ok_or("aucune publication en cours pour ce service")?;
        let own = self.publishing_services.swap_remove(i);
        let mut out = Output::default();
        out.push(self.secure.service_entry_published(&own));
        Ok(out)
    }

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

    struct Sponsor {
        id: DeviceId,
        secure: Secure,
    }

    fn sponsor() -> Sponsor {
        let id = DeviceId::generate();
        let identity = Identity::generate();
        let keyring = Arc::new(MemoryKeyring::new());
        init_group(&*keyring, &id, "téléphone", &identity, false).unwrap();
        Sponsor { id: id.clone(), secure: Secure::load(id, identity, keyring).0 }
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
    fn a_read_event_reaches_the_service_worker() {
        let user: UserId = "usr_01J9Z4B1C2D3E4F5G6H7J8K9M0".parse().unwrap();
        let out = EventOut::from_event(Event::Read { user, until: 1_795_819_680_123 }).unwrap();
        assert_eq!(
            serde_json::to_string(&out).unwrap(),
            r#"{"type":"read","user":"usr_01J9Z4B1C2D3E4F5G6H7J8K9M0","until":1795819680123}"#
        );
    }

    #[test]
    fn offer_survives_a_restart_and_is_cleared_after_joining() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault.clone(), &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        assert!(vault.has(PENDING_OFFER));
        let env = welcome(&mut phone, &offer.qr);

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
    fn otp_is_handed_to_the_service_worker() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault, &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        core.process(1, welcome(&mut phone, &offer.qr));

        let otp = Otp {
            code: "482913".into(),
            display: "G-482913".into(),
            sender_name: "Google".into(),
            sender_domain: "google.com".into(),
            sender_verified: true,
            expires_at: 1_793_628_791,
        };
        let env = phone.secure.seal_otp(&otp, vec![]).unwrap();
        let out = core.process(2, env);
        let json = serde_json::to_value(&out.events).unwrap();
        assert_eq!(json[0]["type"], "otp");
        assert_eq!(json[0]["code"], "482913");
        assert_eq!(json[0]["sender_domain"], "google.com");
        assert_eq!(json[0]["expires_at"], 1_793_628_791);
    }

    #[test]
    fn file_upload_and_download_keep_keys_inside() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault.clone(), &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        core.process(1, welcome(&mut phone, &offer.qr));

        let data: Vec<u8> = (0..40).collect();
        let sha: [u8; 32] = {
            use sha2::Digest;
            sha2::Sha256::digest(&data).into()
        };
        let blob = BlobId::generate().to_string();
        let up = core
            .start_upload(
                "f",
                "a.bin",
                "application/x",
                40,
                16,
                &sha,
                &[],
                None,
                &blob,
                "2099-01-01T00:00:00Z",
            )
            .unwrap();
        assert_eq!((up.geometry.chunks, up.geometry.sealed_size), (3, 40 + 48));
        assert_eq!(core.upload("f", 40).unwrap(), Some(up));
        let sealed: Vec<Vec<u8>> = (0..3u32)
            .map(|i| {
                core.seal_chunk("f", i, &data[i as usize * 16..(i as usize * 16 + 16).min(40)])
                    .unwrap()
            })
            .collect();
        assert!(core.seal_chunk("f", 0, &data[..3]).is_err());
        let env = core.seal_file("f").unwrap();
        core.forget_upload("f").unwrap();
        assert_eq!(core.upload("f", 40).unwrap(), None);

        let fx = phone.secure.process(2, env, OffsetDateTime::now_utc());
        let Some(Effect::Emit(Event::File(f))) = fx.into_iter().next() else {
            panic!("fichier attendu")
        };
        let stored = phone.secure.file(&f.file.blob_id).unwrap().unwrap();
        let meta = ot_core::FileMeta {
            blob_id: stored.blob_id.clone(),
            name: stored.name.clone(),
            mime: stored.mime.clone(),
            size: stored.size,
            chunk_size: stored.chunk_size,
            chunks: stored.chunks,
            key: stored.key.clone(),
            nonce: stored.nonce,
            sha256: stored.sha256,
        };
        let back = phone.secure.seal_file(&meta, vec![]).unwrap();
        let out = core.process(3, back);
        assert!(matches!(&out.events[..], [EventOut::File { file, .. }] if file.name == "a.bin"));
        assert_eq!(core.files().unwrap().len(), 1);
        let mut plain = Vec::new();
        for (i, c) in sealed.iter().enumerate() {
            plain.extend(core.open_chunk(&blob, i as u32, c).unwrap());
        }
        assert_eq!(plain, data);
        assert!(core.open_chunk(&blob, 1, &sealed[0]).is_err());
        assert!(core.check_file(&blob, &sha).unwrap());
        assert!(!core.check_file(&blob, &[0; 32]).unwrap());
        core.forget_file(&blob).unwrap();
        assert!(core.files().unwrap().is_empty());
        let json = serde_json::to_string(&out).unwrap();
        assert!(!json.contains("\"key\"") && !json.contains("nonce"), "{json}");
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

    #[test]
    fn without_anchor_there_is_no_directory_to_read() {
        let (vault, id) = extension();
        let mut core = CoreState::open(vault, &id).unwrap().0;
        assert!(core.directory_start().is_none());
        assert!(core.people().is_empty());
        assert!(core.invite_request().is_none());
        assert!(core.declaration_frame(Some("browser".into()), &[]).unwrap().is_none());
        assert!(core.entry_published().is_err());
    }

    #[test]
    fn new_events_have_a_stable_json_shape() {
        let user = UserId::generate();
        let ev = EventOut::from_event(Event::PersonJoined {
            user: user.clone(),
            name: "Bob".into(),
            device_name: "bob-phone".into(),
            fingerprint: "ab:cd".into(),
        })
        .unwrap();
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "person_joined");
        assert_eq!(json["user"], user.to_string());
        let ev = EventOut::from_event(Event::DirectoryFork).unwrap();
        assert_eq!(serde_json::to_value(&ev).unwrap()["type"], "directory_fork");
    }

    #[derive(Default)]
    struct Relay {
        admin: Option<UserId>,
        users: Vec<ot_proto::DirectoryUser>,
    }

    impl Relay {
        fn user(&mut self, id: &UserId) -> &mut ot_proto::DirectoryUser {
            self.users.iter_mut().find(|u| &u.id == id).unwrap()
        }

        fn append(&mut self, frame: &Frame) {
            let Frame::RosterPublish { user, seq, entry } = frame else { panic!("{frame:?}") };
            let log = &mut self.user(user).log;
            assert_eq!(log.len() as u64 + 1, *seq, "journal continu");
            log.push(ot_proto::LogEntry { seq: *seq, entry: entry.clone() });
        }

        fn page(&self, request: &Frame) -> Frame {
            let Frame::DirectoryGet { known, .. } = request else { panic!("{request:?}") };
            let admin = self.admin.clone().unwrap();
            let users = self
                .users
                .iter()
                .map(|u| {
                    let after = known.get(&u.id).copied().unwrap_or(0);
                    let log = u.log.iter().filter(|e| e.seq > after).cloned().collect();
                    ot_proto::DirectoryUser { log, ..u.clone() }
                })
                .collect();
            Frame::Directory { me: admin.clone(), admin, users, next: None }
        }
    }

    fn device(id: &DeviceId, name: &str) -> ot_proto::DirectoryDevice {
        ot_proto::DirectoryDevice {
            id: id.clone(),
            name: name.into(),
            kind: None,
            reachable: true,
            default: true,
            features: vec![ot_proto::Feature::users_v1()],
            online: true,
        }
    }

    fn person(id: &UserId, name: &str, role: ot_proto::Role) -> ot_proto::DirectoryUser {
        ot_proto::DirectoryUser {
            id: id.clone(),
            name: name.into(),
            role,
            state: ot_proto::UserState::Active,
            cert: None,
            log: Vec::new(),
            more: false,
            muted: false,
            devices: Vec::new(),
        }
    }

    fn read(core: &mut CoreState, relay: &Relay) -> Vec<EventOut> {
        let mut events = Vec::new();
        let mut frame = core.directory_start().expect("ancre");
        loop {
            let step = core.directory_page(relay.page(&frame)).unwrap();
            events.extend(step.out.events);
            match step.frame {
                Some(f) if !step.done => frame = f,
                _ => return events,
            }
        }
    }

    fn phone_read(phone: &mut Sponsor, relay: &Relay) {
        let request = Frame::DirectoryGet { known: phone.secure.directory_known(), after: None };
        let Frame::Directory { me, admin, users, .. } = relay.page(&request) else {
            unreachable!()
        };
        let (fx, _) = phone.secure.apply_directory(&me, &admin, &users);
        assert!(!fx.iter().any(|f| matches!(f, Effect::Emit(Event::DirectoryRejected { .. }))));
    }

    fn admin_pair() -> (Sponsor, CoreState, String, Relay, UserId) {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault, &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        core.process(1, welcome(&mut phone, &offer.qr));

        let admin = UserId::generate();
        let mut relay = Relay { admin: Some(admin.clone()), ..Relay::default() };
        let mut root = person(&admin, "Alice", ot_proto::Role::Admin);
        root.devices = vec![device(&phone.id, "téléphone"), device(&id.parse().unwrap(), "chrome")];
        relay.users.push(root);

        let get = core.admin_init_start().unwrap();
        assert!(core.adopt_admin_root(&admin.to_string()).is_err(), "rien d'envoyé");
        let res = core.admin_root(relay.page(&get)).unwrap();
        assert!(!res.adopted);
        let Some(Frame::AdminInit { cert, entry }) = res.frame else { panic!("admin_init") };
        assert!(core.directory_start().is_none(), "pas d'ancre avant la réponse du relais");
        assert!(core.adopt_admin_root(&UserId::generate().to_string()).is_err());
        core.adopt_admin_root(&admin.to_string()).unwrap();
        assert!(core.admin_init_start().is_err(), "ancre déjà là");
        let user = relay.user(&admin);
        user.cert = Some(cert.clone());
        user.log.push(ot_proto::LogEntry { seq: 1, entry });

        let events = read(&mut core, &relay);
        assert!(events.contains(&EventOut::DirectoryChanged), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, EventOut::DirectoryRejected { .. })));
        let people = core.people();
        assert_eq!((people.len(), people[0].role, people[0].devices.len()), (1, "admin", 2));

        phone.secure.adopt_existing_root(&admin, &cert).unwrap();
        phone_read(&mut phone, &relay);
        (phone, core, id, relay, admin)
    }

    #[test]
    fn an_existing_root_is_adopted_only_from_my_group() {
        let (_phone, _core, _, relay, admin) = admin_pair();
        let get = Frame::DirectoryGet { known: Default::default(), after: None };

        let (vault, id) = extension();
        let mut other = CoreState::open(vault, &id).unwrap().0;
        assert!(other.admin_init_start().is_err(), "sans groupe");
        let mut stranger = sponsor();
        let offer = other.offer("chrome-2").unwrap();
        other.process(1, welcome(&mut stranger, &offer.qr));
        let why = other.admin_root(relay.page(&get)).unwrap_err();
        assert!(why.contains("hors de mon groupe"), "{why}");
        assert!(other.directory_start().is_none());

        let Frame::Directory { admin: a, users, .. } = relay.page(&get) else { unreachable!() };
        let page = Frame::Directory { me: UserId::generate(), admin: a, users, next: None };
        let why = other.admin_root(page).unwrap_err();
        assert!(why.contains("pas à l'admin"), "{why}");
        let page = Frame::Directory { me: admin.clone(), admin, users: Vec::new(), next: None };
        assert!(other.admin_root(page).unwrap_err().contains("manque"));
    }

    #[test]
    fn a_second_device_of_the_group_gets_the_anchor_from_its_sponsor() {
        let (mut phone, _core, _, _relay, _) = admin_pair();
        let (vault, id) = extension();
        let mut second = CoreState::open(vault, &id).unwrap().0;
        let offer = second.offer("chrome-2").unwrap();
        second.process(1, welcome(&mut phone, &offer.qr));
        assert!(second.awaits_directory(), "lecture de l'annuaire due");
        assert!(second.admin_init_start().is_err(), "ancre déjà là");
        assert!(second.directory_start().is_some());
    }

    #[test]
    fn removing_the_device_that_created_a_service_republishes_it() {
        let (mut phone, mut core, _, mut relay, admin) = admin_pair();
        let now = OffsetDateTime::now_utc();

        let svc = UserId::generate();
        let mut notifier = Secure::load(
            DeviceId::generate(),
            Identity::generate(),
            Arc::new(MemoryKeyring::new()),
        )
        .0;
        let offer = notifier.offer("notifier-1").unwrap();
        let cert = phone.secure.service_cert(&svc, "Notifier", &offer, now).unwrap();
        let mut user = person(&svc, "Notifier", ot_proto::Role::Service);
        user.cert = Some(cert.clone());
        user.devices = vec![device(&offer.device_id, "notifier-1")];
        relay.users.push(user);
        let NextEntry::Entry(own) =
            phone.secure.next_entry(&Publish::Certify(svc.clone(), cert)).unwrap()
        else {
            panic!("entrée attendue")
        };
        relay.append(&Frame::RosterPublish {
            user: own.user.clone(),
            seq: own.entry.seq,
            entry: own.entry.entry.clone(),
        });
        phone.secure.entry_published(&own);
        let member = ot_core::crypto::Member {
            device_id: offer.device_id.clone(),
            name: offer.name.clone(),
            sign_key: offer.sign_key,
            exchange_key: offer.exchange_key,
            added_epoch: 0,
        };
        let NextEntry::Entry(own) = phone.secure.next_service_entry(&svc, Some(&member)).unwrap()
        else {
            panic!("entrée du service attendue")
        };
        relay.append(&Frame::RosterPublish {
            user: own.user.clone(),
            seq: own.entry.seq,
            entry: own.entry.entry.clone(),
        });

        read(&mut core, &relay);
        assert!(core.people().iter().any(|p| p.name == "Notifier" && p.role == "service"));
        let phone_id = phone.id.to_string();
        assert_eq!(core.certified_by(&phone_id).unwrap(), vec![svc.to_string()]);
        assert!(core.services_to_republish().is_empty(), "signataire encore là");
        assert!(core.recertify(&admin.to_string(), None).is_err(), "la racine ne se re-signe pas");

        let (epoch, _) = core.rotate(std::slice::from_ref(&phone_id)).unwrap();
        assert_eq!(epoch, 2);
        let publish = core.next_entry(PublishIn::IfBehind).unwrap();
        relay.append(publish.frame.as_ref().unwrap());
        core.entry_published().unwrap();
        relay.user(&admin).devices.retain(|d| d.id.to_string() != phone_id);
        assert_eq!(core.services_to_republish(), vec![svc.to_string()], "D47");

        for user in core.certified_by(&phone_id).unwrap() {
            let cert = core.recertify(&user, None).unwrap();
            relay.user(&user.parse().unwrap()).cert = Some(cert.clone());
            let what = PublishIn::Certify { user: user.parse().unwrap(), cert };
            let publish = core.next_entry(what).unwrap();
            relay.append(publish.frame.as_ref().unwrap());
            core.entry_published().unwrap();
        }

        let publish = core.next_service_entry(&svc.to_string()).unwrap();
        let frame = publish.frame.unwrap();
        let Frame::RosterPublish { seq, .. } = &frame else { panic!("{frame:?}") };
        assert_eq!(*seq, 2);
        assert!(core.service_entry_published(&svc.to_string(), 3).is_err(), "autre entrée");
        relay.append(&frame);
        core.service_entry_published(&svc.to_string(), 2).unwrap();
        assert!(core.service_entry_published(&svc.to_string(), 2).is_err(), "une seule fois");
        assert!(core.services_to_republish().is_empty());
        let again = core.next_service_entry(&svc.to_string()).unwrap();
        assert!(again.frame.is_none() && again.head.is_some_and(|h| h.seq == 2));

        let events = read(&mut core, &relay);
        assert!(
            !events.iter().any(|e| matches!(e, EventOut::DirectoryRejected { .. })),
            "{events:?}"
        );
        assert!(core.people().iter().any(|p| p.name == "Notifier"));
        assert!(core.certified_by(&phone_id).unwrap().is_empty());
        assert!(core.services_to_republish().is_empty());
    }

    #[test]
    fn an_upload_to_a_person_keeps_its_target_and_seals_for_its_devices() {
        let (phone, core, _, _, admin) = admin_pair();
        let sha = [7; 32];
        let blob = BlobId::generate().to_string();
        let user = admin.to_string();
        let start = |to: &[String], to_user: Option<&str>| {
            core.start_upload(
                "f",
                "a.bin",
                "application/x",
                40,
                16,
                &sha,
                to,
                to_user,
                &blob,
                "2099-01-01T00:00:00Z",
            )
        };
        assert!(start(&[], Some(&user)).is_err(), "sans appareil résolu");
        assert!(start(&[], Some("pas-une-personne")).is_err());

        let to = vec![phone.id.to_string()];
        let up = start(&to, Some(&user)).unwrap();
        assert_eq!((up.to.clone(), up.to_user.clone()), (to.clone(), Some(user.clone())));
        let json = serde_json::to_value(core.upload("f", 40).unwrap().unwrap()).unwrap();
        assert_eq!(json["to_user"], user);
        assert_eq!(json["to"][0], to[0]);
        let env = core.seal_file("f").unwrap();
        assert_eq!(env.to, vec![phone.id.clone()]);
        let stranger = DeviceId::generate().to_string();
        start(std::slice::from_ref(&stranger), Some(&user)).unwrap();
        let why = core.seal_file("f").unwrap_err();
        assert!(why.contains("annuaire vérifié"), "{why}");

        start(&[], None).unwrap();
        let json = serde_json::to_value(core.upload("f", 40).unwrap().unwrap()).unwrap();
        assert!(json.get("to_user").is_none(), "{json}");
    }
}
