//! Cœur de groupe de l'extension : [`ot_core::Secure`] au-dessus du coffre,
//! plus les appairages en attente rangés pour survivre à l'arrêt du service
//! worker (note 0023). Entrées et sorties en JSON ; aucune clé n'en sort.

use std::sync::Arc;

use ot_core::crypto::{CHUNK_TAG_LEN, chunk_count, chunk_len, sealed_size};
use ot_core::crypto::{PairingCode, PairingSecret};
use ot_core::files::new_file_meta;
use ot_core::{
    BlobId, ClientError, Content, ContentKind, DeviceId, Effect, Event, FileInfo, GroupInfo,
    PendingUpload, SecretStore, Secure, StoreKeyring,
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
    /// Fichier annoncé (étape 5) ; sa clé reste dans le coffre.
    File {
        seq: i64,
        #[serde(flatten)]
        file: FileOut,
    },
}

/// Fichier annoncé, sans clé.
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
        }
    }
}

/// Géométrie d'un fichier, pour le relais (tailles chiffrées) et la boucle
/// de transfert (tailles en clair).
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

/// Dépôt en cours, sans clé.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UploadOut {
    pub blob_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    #[serde(flatten)]
    pub geometry: Geometry,
}

impl UploadOut {
    fn new(p: &PendingUpload) -> Result<Self, String> {
        Ok(Self {
            blob_id: p.meta.blob_id.to_string(),
            expires_at: p.expires_at,
            geometry: Geometry::new(p.meta.size, p.meta.chunk_size)?,
        })
    }
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
            Event::File(f) => Self::File { seq: f.seq, file: f.file.into() },
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

    // --- Fichiers (étape 5, note 0026) --------------------------------------

    /// Dépôt en cours de cette source, s'il reste valable.
    pub fn upload(&self, source_id: &str, size: u64) -> Result<Option<UploadOut>, String> {
        let now = OffsetDateTime::now_utc();
        self.secure
            .upload(source_id, size, now)
            .map_err(err)?
            .as_ref()
            .map(UploadOut::new)
            .transpose()
    }

    /// Nouveau dépôt, après la création du blob par le relais : tire la clé
    /// du fichier et la range dans le coffre.
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
        blob_id: &str,
        expires_at: &str,
    ) -> Result<UploadOut, String> {
        let sha256: [u8; 32] = sha256.try_into().map_err(|_| "SHA-256 de 32 octets attendu")?;
        let blob: BlobId = blob_id.parse().map_err(err)?;
        let to =
            to.iter().map(|id| id.parse::<DeviceId>().map_err(err)).collect::<Result<_, _>>()?;
        let expires_at =
            OffsetDateTime::parse(expires_at, &time::format_description::well_known::Rfc3339)
                .map_err(err)?;
        let meta =
            new_file_meta(blob, name, mime, size, chunk_size, sha256).ok_or("fichier trop gros")?;
        let pending = PendingUpload { source_id: source_id.to_owned(), to, expires_at, meta };
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

    /// Chiffre le morceau `index` d'un dépôt en cours.
    pub fn seal_chunk(&self, source_id: &str, index: u32, plain: &[u8]) -> Result<Vec<u8>, String> {
        let p = self.pending_upload(source_id)?;
        let m = &p.meta;
        if chunk_len(m.size, m.chunk_size, index) != Some(plain.len() as u32) {
            return Err(format!("morceau {index} de taille inattendue"));
        }
        let cipher = ot_core::crypto::FileCipher::new(&m.key, &m.nonce);
        cipher.seal_chunk(index, index + 1 == m.chunks, plain).map_err(err)
    }

    /// Annonce (enveloppe chiffrée) d'un dépôt terminé. Le dépôt reste
    /// noté jusqu'à [`Self::forget_upload`], une fois l'annonce acceptée.
    pub fn seal_file(&self, source_id: &str) -> Result<Envelope, String> {
        let p = self.pending_upload(source_id)?;
        self.secure.seal_file(&p.meta, p.to).map_err(err)
    }

    pub fn forget_upload(&self, source_id: &str) -> Result<(), String> {
        self.secure.forget_upload(source_id).map_err(err)
    }

    /// Fichiers annoncés encore téléchargeables.
    pub fn files(&self) -> Result<Vec<FileOut>, String> {
        Ok(self.secure.files().map_err(err)?.into_iter().map(FileOut::from).collect())
    }

    fn stored(&self, blob_id: &str) -> Result<ot_core::StoredFile, String> {
        let blob: BlobId = blob_id.parse().map_err(err)?;
        self.secure.file(&blob).map_err(err)?.ok_or_else(|| "fichier inconnu".to_owned())
    }

    /// Géométrie d'un fichier annoncé.
    pub fn file_geometry(&self, blob_id: &str) -> Result<Geometry, String> {
        let f = self.stored(blob_id)?;
        Geometry::new(f.size, f.chunk_size)
    }

    /// Déchiffre le morceau `index` d'un fichier annoncé ; refuse un morceau
    /// altéré, déplacé ou de taille inattendue.
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

    /// Le SHA-256 du fichier reçu est-il celui annoncé ?
    pub fn check_file(&self, blob_id: &str, sha256: &[u8]) -> Result<bool, String> {
        Ok(self.stored(blob_id)?.sha256 == sha256)
    }

    pub fn forget_file(&self, blob_id: &str) -> Result<(), String> {
        let blob: BlobId = blob_id.parse().map_err(err)?;
        self.secure.forget_file(&blob).map_err(err)
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
    fn file_upload_and_download_keep_keys_inside() {
        let mut phone = sponsor();
        let (vault, id) = extension();
        let (mut core, _) = CoreState::open(vault.clone(), &id).unwrap();
        let offer = core.offer("chrome").unwrap();
        core.process(1, welcome(&mut phone, &offer.qr));

        // L'extension envoie un fichier de 40 octets en morceaux de 16.
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

        // Le téléphone reçoit l'annonce ; puis renvoie la même au cœur de
        // l'extension pour tester le retrait.
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
        // Ce qui sort vers le JavaScript ne contient aucune clé.
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
}
