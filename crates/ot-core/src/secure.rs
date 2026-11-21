//! Chiffrement de bout en bout côté client : chaque message sortant est
//! chiffré et signé, chaque message entrant est déchiffré et vérifié contre
//! le roster avant d'être remis à l'appli. Les messages système (`roster`,
//! `group_key`, `pair_*`) sont traités ici.

use std::sync::Arc;

use ot_crypto::directory::{
    CertRef, GENESIS_PREV, Genesis, LogHead, RosterEntry, UserCert, anchor, cert_hash, entry_hash,
    verify_root,
};
use ot_crypto::{
    Body, CryptoError, EpochKey, ExchangeKeyPair, GroupKey, Header, InviteCode, Member, OpenKey,
    Opened, Otp, PairingCode, PairingOffer, PairingSecret, Roster, SealKey, fingerprint, open,
    seal,
};
use ot_proto::{
    BlobId, Content, ContentKind, DeviceId, DeviceInfo, DeviceKind, DirectoryUser, Encoding,
    Envelope, Feature, Frame, Known, LogEntry, MAX_RECIPIENTS, Payload, Role, SignedBlob, UserId,
};
use time::OffsetDateTime;

use crate::Identity;
use crate::directory::{Chain, DeviceProfile, DirectoryEvent, DirectoryState, Person};
use crate::files::{
    FileInfo, FileMeta, PendingUpload, ReceivedFile, StoredFile, add_offer, prune_offers,
    stored_from_body,
};
use crate::invite::{Guest, InviteRecord, PendingInvite, Welcome};
use crate::keyring::{
    GroupKeys, Keyring, KeyringError, MAX_QUARANTINE, Quarantined, RETIRED_KEY_LIFETIME,
};
use crate::roster::{self, RosterError};
use crate::types::{ClientError, Event, Origin, Received, ReceivedOtp};

mod service;
#[cfg(feature = "net")]
pub(crate) use service::service_member;

/// État du groupe, vu de cet appareil (pour l'affichage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInfo {
    pub epoch: u32,
    /// Empreinte courte de la clé de groupe courante.
    pub key_fingerprint: String,
    pub roster_version: u64,
    pub members: Vec<MemberInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    pub device_id: DeviceId,
    pub name: String,
    /// Empreinte courte de la clé Ed25519.
    pub sign_fingerprint: String,
    /// Empreinte courte de la clé X25519.
    pub exchange_fingerprint: String,
    pub added_epoch: u32,
}

impl GroupInfo {
    fn new(keys: &GroupKeys, roster: &Roster) -> Self {
        Self {
            epoch: keys.current,
            key_fingerprint: keys.current_key().map(GroupKey::fingerprint).unwrap_or_default(),
            roster_version: roster.version,
            members: roster
                .members
                .iter()
                .map(|m| MemberInfo {
                    device_id: m.device_id.clone(),
                    name: m.name.clone(),
                    sign_fingerprint: fingerprint(&m.sign_key),
                    exchange_fingerprint: fingerprint(&m.exchange_key),
                    added_epoch: m.added_epoch,
                })
                .collect(),
        }
    }

    pub fn member(&self, id: &DeviceId) -> Option<&MemberInfo> {
        self.members.iter().find(|m| &m.device_id == id)
    }

    /// Appareils annoncés par le relais mais absents du roster : relais
    /// compromis, ou appairage inachevé. Ils ne reçoivent rien de lisible.
    pub fn strangers(&self, devices: &[DeviceInfo]) -> Vec<DeviceInfo> {
        devices.iter().filter(|d| self.member(&d.id).is_none()).cloned().collect()
    }
}

/// Erreur de `init_group`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupError {
    #[error("un groupe existe déjà (époque {0}) ; --force pour le remplacer")]
    Exists(u32),
    #[error(transparent)]
    Keyring(#[from] KeyringError),
}

/// Paire X25519 de l'appareil, créée et enregistrée si elle manque.
pub fn ensure_exchange_key(keyring: &dyn Keyring) -> Result<ExchangeKeyPair, KeyringError> {
    if let Some(key) = keyring.load_exchange_key()? {
        return Ok(key);
    }
    let key = ExchangeKeyPair::generate();
    keyring.save_exchange_key(&key)?;
    Ok(key)
}

/// Crée le groupe sur le premier appareil : époque 1 et un roster qui ne
/// contient que lui. Refuse d'écraser un groupe existant sans `force`.
pub fn init_group(
    keyring: &dyn Keyring,
    device_id: &DeviceId,
    name: &str,
    identity: &Identity,
    force: bool,
) -> Result<GroupInfo, GroupError> {
    if let Some(keys) = keyring.load_group_keys()?
        && !force
    {
        return Err(GroupError::Exists(keys.current));
    }
    let exchange = ensure_exchange_key(keyring)?;
    let roster = roster::genesis(Member {
        device_id: device_id.clone(),
        name: name.to_owned(),
        sign_key: identity.public_key(),
        exchange_key: exchange.public_key(),
        added_epoch: 1,
    });
    let keys = GroupKeys::new(1, GroupKey::generate());
    keyring.save_group_keys(&keys)?;
    keyring.save_roster(&roster)?;
    keyring.save_quarantine(&[])?;
    Ok(GroupInfo::new(&keys, &roster))
}

/// État du groupe enregistré, sans connexion ; `None` sans groupe.
pub fn group_info(keyring: &dyn Keyring) -> Result<Option<GroupInfo>, KeyringError> {
    match (keyring.load_group_keys()?, keyring.load_roster()?) {
        (Some(keys), Some(roster)) => Ok(Some(GroupInfo::new(&keys, &roster))),
        _ => Ok(None),
    }
}

/// Ce que le client doit faire après le traitement d'un message : émettre un
/// événement, ou envoyer une enveloppe (message système : roster, réponse
/// d'appairage).
#[derive(Debug)]
pub enum Effect {
    Emit(Event),
    Send(Envelope),
}

/// Refus d'un message entrant.
enum Reject {
    /// Rejeu ou message périmé : acquitté sans bruit.
    Ignored(&'static str),
    /// Rejeté et signalé à l'appli.
    Reason(String),
}

impl From<CryptoError> for Reject {
    fn from(e: CryptoError) -> Self {
        Self::Reason(e.to_string())
    }
}

impl From<RosterError> for Reject {
    fn from(e: RosterError) -> Self {
        match e {
            RosterError::Stale => Self::Ignored("roster déjà connu"),
            e => Self::Reason(e.to_string()),
        }
    }
}

impl From<&str> for Reject {
    fn from(s: &str) -> Self {
        Self::Reason(s.to_owned())
    }
}

enum Handled {
    Done,
    /// Nouvelles clés : la quarantaine est à réessayer.
    KeysChanged,
    Quarantine,
}

/// Parrainage en cours : le secret ne vit qu'en mémoire.
struct Sponsoring {
    token_id: [u8; 8],
    secret: PairingSecret,
    expires_at: OffsetDateTime,
}

/// Appairage en cours côté nouvel appareil.
struct Joining {
    token_id: [u8; 8],
    secret: PairingSecret,
    sponsor: DeviceId,
}

/// Cœur de groupe sans réseau : scelle les messages sortants, ouvre et
/// vérifie les messages entrants, tient roster, époques, quarantaine et
/// appairages en cours. Chaque changement est enregistré dans le
/// [`Keyring`] avant de rendre la main. Utilisé par le [`crate::Client`]
/// réseau et, compilé en WebAssembly, par l'extension (note 0023).
pub struct Secure {
    me: DeviceId,
    identity: Identity,
    exchange: ExchangeKeyPair,
    keys: Option<GroupKeys>,
    roster: Option<Roster>,
    quarantine: Vec<Quarantined>,
    keyring: Arc<dyn Keyring>,
    sponsoring: Vec<Sponsoring>,
    joining: Option<Joining>,
    /// QR inversé affiché : secret `S` attendu dans la réponse du parrain.
    joining_qr: Option<PairingSecret>,
    /// Annuaire vérifié (étape 10), rangé dans le keyring.
    directory: DirectoryState,
    /// Personnes vérifiées au dernier tour complet de lecture.
    people: Vec<Person>,
    /// Fourche de ma personne déjà signalée (une alerte par fourche).
    fork: bool,
    /// Ma personne, d'après le relais (dernière lecture de l'annuaire).
    me_user: Option<UserId>,
    /// Heure du dernier tour complet de lecture.
    people_at: Option<OffsetDateTime>,
    /// Invitations créées par cet appareil (admin), rangées dans le keyring.
    invites: Vec<InviteRecord>,
    /// Invitation rachetée par cet appareil (invité), rangée dans le keyring.
    pending_invite: Option<PendingInvite>,
    /// Appareil de service : bienvenue reçue, à confirmer par l'annuaire.
    service_join: Option<service::ServiceJoin>,
}

/// Ce qu'une entrée de mon journal doit changer (§ 4.2, note 0056).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Publish {
    /// Mon roster local, s'il est plus récent que le journal publié.
    IfBehind,
    /// Admin : engager ce certificat (neuf ou re-signé) de cette personne.
    Certify(UserId, SignedBlob),
    /// Admin : retirer l'empreinte d'une personne révoquée.
    Revoke(UserId),
}

/// Entrée de mon journal signée par cet appareil, à publier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnEntry {
    pub user: UserId,
    pub entry: LogEntry,
    /// Roster qu'elle porte : adopté par mon groupe une fois publiée.
    pub roster: Roster,
    pub certs: Vec<CertRef>,
}

impl OwnEntry {
    pub fn head(&self) -> LogHead {
        LogHead { seq: self.entry.seq, hash: entry_hash(&self.entry.entry) }
    }
}

/// Suite d'une publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextEntry {
    /// Rien à publier ; la tête de mon journal si elle engage déjà le
    /// changement demandé.
    Done(Option<LogHead>),
    Entry(Box<OwnEntry>),
}

/// Âge au-delà duquel l'annuaire est relu avant un envoi à une personne.
pub const DIRECTORY_MAX_AGE: time::Duration = time::Duration::seconds(60);

fn body_of(content: &Content) -> Body {
    match content.kind {
        ContentKind::Text => Body::Text { body: content.body.clone() },
        ContentKind::Link => Body::Link { body: content.body.clone() },
    }
}

impl Secure {
    /// Charge l'état depuis le keyring. Une erreur de lecture laisse
    /// l'appareil sans groupe (il refuse alors d'envoyer) et est signalée.
    pub fn load(me: DeviceId, identity: Identity, keyring: Arc<dyn Keyring>) -> (Self, Vec<Event>) {
        let mut events = Vec::new();
        let mut report = |e: KeyringError| events.push(Event::KeyringError { message: e.0 });
        let exchange = ensure_exchange_key(&*keyring).unwrap_or_else(|e| {
            report(e);
            ExchangeKeyPair::generate()
        });
        let keys = keyring.load_group_keys().unwrap_or_else(|e| {
            report(e);
            None
        });
        let roster = keyring.load_roster().unwrap_or_else(|e| {
            report(e);
            None
        });
        let quarantine = keyring.load_quarantine().unwrap_or_else(|e| {
            report(e);
            Vec::new()
        });
        let directory = keyring.load_directory().unwrap_or_else(|e| {
            report(e);
            DirectoryState::default()
        });
        let mut secure = Self {
            me,
            identity,
            exchange,
            keys,
            roster,
            quarantine,
            keyring,
            sponsoring: Vec::new(),
            joining: None,
            joining_qr: None,
            directory,
            people: Vec::new(),
            fork: false,
            me_user: None,
            people_at: None,
            invites: Vec::new(),
            pending_invite: None,
            service_join: None,
        };
        secure.invites = secure.keyring.load_invites().unwrap_or_else(|e| {
            events.push(Event::KeyringError { message: e.0 });
            Vec::new()
        });
        secure.pending_invite = secure.keyring.load_pending_invite().unwrap_or_else(|e| {
            events.push(Event::KeyringError { message: e.0 });
            None
        });
        let now = OffsetDateTime::now_utc();
        if secure.keys.as_mut().is_some_and(|k| k.prune(now)) {
            let mut fx = Vec::new();
            secure.save_keys(&mut fx);
            events.extend(fx.into_iter().filter_map(|f| match f {
                Effect::Emit(e) => Some(e),
                Effect::Send(_) => None,
            }));
        }
        (secure, events)
    }

    pub fn info(&self) -> Option<GroupInfo> {
        Some(GroupInfo::new(self.keys.as_ref()?, self.roster.as_ref()?))
    }

    /// L'appareil connaît l'ancre de l'annuaire : il le lit et le vérifie.
    /// Sans elle (installation à une personne), l'annuaire reste vide.
    pub fn has_anchor(&self) -> bool {
        self.directory.anchor.is_some()
    }

    /// Dernier `seq` vérifié par personne, pour `directory_get`.
    pub fn directory_known(&self) -> Known {
        self.directory.known()
    }

    /// Personnes vérifiées au dernier tour complet de lecture (moi compris).
    pub fn people(&self) -> Vec<Person> {
        self.people.clone()
    }

    /// État vérifié de l'annuaire (ancre, journaux).
    pub fn directory_state(&self) -> &DirectoryState {
        &self.directory
    }

    /// L'annuaire a été lu il y a moins de [`DIRECTORY_MAX_AGE`] : un envoi à
    /// une personne peut partir sans relecture.
    pub fn directory_fresh(&self, now: OffsetDateTime) -> bool {
        self.people_at.is_some_and(|t| now - t < DIRECTORY_MAX_AGE)
    }

    /// Mon profil sur le relais n'annonce pas encore `users-v1` : à
    /// déclarer (`device_update`), seul l'appareil le peut.
    pub fn needs_features(&self) -> bool {
        self.my_profile().is_some_and(|d| !d.up_to_date)
    }

    /// `device_update` que cet appareil doit au relais : ses capacités si
    /// son profil ne les a pas, et `kind` si son profil n'a pas de
    /// catégorie (jamais celle choisie dans les réglages). `None` : rien.
    pub fn declaration(&self, kind: Option<DeviceKind>) -> Option<Frame> {
        let profile = self.my_profile()?;
        let features = (!profile.up_to_date).then(|| vec![Feature::users_v1()]);
        let kind = kind.filter(|_| profile.kind.is_none());
        if features.is_none() && kind.is_none() {
            return None;
        }
        Some(Frame::DeviceUpdate {
            device: self.me.clone(),
            kind,
            reachable: None,
            default: None,
            features,
        })
    }

    fn my_profile(&self) -> Option<&DeviceProfile> {
        self.my_person()?.devices.iter().find(|d| d.id == self.me)
    }

    fn my_person(&self) -> Option<&Person> {
        let me = self.me_user.as_ref()?;
        self.people.iter().find(|p| &p.id == me)
    }

    /// Les autres personnes vérifiées.
    fn others(&self) -> impl Iterator<Item = &Person> {
        let me = self.me_user.clone();
        self.people.iter().filter(move |p| Some(&p.id) != me.as_ref())
    }

    /// Appareils destinataires d'un envoi à `user` (§ 4.3) : `device` s'il
    /// est joignable et à jour ; sinon ses appareils par défaut joignables
    /// et à jour, ou à défaut tous ses appareils joignables et à jour (D42).
    /// Puis la copie (D43) : mes appareils à jour, hors CLI et hors
    /// celui-ci. Seuls des appareils des rosters vérifiés en sortent.
    pub fn recipients(
        &self,
        user: &UserId,
        device: Option<&DeviceId>,
    ) -> Result<Vec<DeviceId>, ClientError> {
        if self.me_user.as_ref() == Some(user) {
            return Err(ClientError::Invalid("c'est ma personne : envoyer à mes appareils".into()));
        }
        let person = self
            .others()
            .find(|p| &p.id == user)
            .ok_or_else(|| ClientError::NoRecipient("personne absente de l'annuaire".into()))?;
        if person.role == Role::Service {
            return Err(ClientError::Invalid("un service ne reçoit pas de message".into()));
        }
        let usable = |d: &&crate::DeviceProfile| d.reachable && d.up_to_date;
        let mut to: Vec<DeviceId> = match device {
            Some(id) => {
                let d = person.devices.iter().find(|d| &d.id == id).ok_or_else(|| {
                    ClientError::NoRecipient(format!(
                        "{id} n'est pas un appareil de {}",
                        person.name
                    ))
                })?;
                if !usable(&d) {
                    return Err(ClientError::NoRecipient(format!(
                        "{} n'est pas joignable ou pas à jour",
                        d.name
                    )));
                }
                vec![d.id.clone()]
            }
            None => {
                let defaults: Vec<_> =
                    person.devices.iter().filter(usable).filter(|d| d.default).collect();
                let chosen = if defaults.is_empty() {
                    person.devices.iter().filter(usable).collect()
                } else {
                    defaults
                };
                chosen.into_iter().map(|d| d.id.clone()).collect()
            }
        };
        if to.is_empty() {
            return Err(ClientError::NoRecipient(format!(
                "aucun appareil de {} n'est joignable et à jour",
                person.name
            )));
        }
        to.truncate(MAX_RECIPIENTS);
        to.extend(self.copies().into_iter().take(MAX_RECIPIENTS - to.len()));
        Ok(to)
    }

    /// Mes appareils qui reçoivent la copie d'un envoi à une personne : à
    /// jour, hors CLI, hors celui-ci, membres de mon roster. Aucun pour un
    /// service (pas de groupe).
    fn copies(&self) -> Vec<DeviceId> {
        let (Some(me), Some(roster)) = (self.my_person(), &self.roster) else {
            return Vec::new();
        };
        if me.role == Role::Service {
            return Vec::new();
        }
        me.devices
            .iter()
            .filter(|d| d.up_to_date && d.kind != Some(DeviceKind::Cli) && d.id != self.me)
            .filter(|d| roster.contains(&d.id))
            .map(|d| d.id.clone())
            .collect()
    }

    /// Clé X25519 d'un appareil : mon roster d'abord, puis les rosters
    /// vérifiés des autres personnes.
    fn exchange_key(&self, id: &DeviceId) -> Option<[u8; 32]> {
        if let Some(m) = self.roster.as_ref().and_then(|r| r.member(id)) {
            return Some(m.exchange_key);
        }
        self.others().find_map(|p| p.roster.member(id)).map(|m| m.exchange_key)
    }

    /// Clé de signature et personne d'un expéditeur : mon roster d'abord
    /// (ma personne), puis les rosters vérifiés des autres personnes.
    fn sender(&self, id: &DeviceId) -> Option<([u8; 32], Origin)> {
        if let Some(m) = self.roster.as_ref().and_then(|r| r.member(id)) {
            return Some((m.sign_key, Origin::default()));
        }
        self.others().find_map(|p| {
            let m = p.roster.member(id)?;
            let origin = Origin {
                from_user: Some(p.id.clone()),
                from_service: p.role == Role::Service,
                to_user: None,
            };
            Some((m.sign_key, origin))
        })
    }

    /// Copie d'un de mes envois : la personne du premier destinataire hors
    /// de mon roster.
    fn addressee(&self, to: &[DeviceId]) -> Option<UserId> {
        let roster = self.roster.as_ref();
        let other = to.iter().find(|id| !roster.is_some_and(|r| r.contains(id)))?;
        self.others().find(|p| p.roster.contains(other)).map(|p| p.id.clone())
    }

    /// Scelle `body` en `0x05` pour ces appareils, dans cet ordre.
    fn seal_multi(&self, to: Vec<DeviceId>, body: &Body) -> Result<Envelope, ClientError> {
        let recipients = to
            .iter()
            .map(|id| {
                let key = self.exchange_key(id).ok_or_else(|| {
                    ClientError::NoRecipient(format!("{id} absent de l'annuaire vérifié"))
                })?;
                Ok((id.clone(), key))
            })
            .collect::<Result<Vec<_>, ClientError>>()?;
        self.seal_env(to, SealKey::Multi { recipients: &recipients }, body).map_err(crypto)
    }

    /// Chiffre un contenu pour une autre personne (`0x05`), copie à mes
    /// appareils comprise.
    pub fn seal_content_to(
        &self,
        content: &Content,
        user: &UserId,
        device: Option<&DeviceId>,
    ) -> Result<Envelope, ClientError> {
        let to = self.recipients(user, device)?;
        self.seal_multi(to, &body_of(content))
    }

    /// Chiffre un code à usage unique pour des personnes (pont d'un service,
    /// ou d'un membre), en un seul message `0x05`.
    pub fn seal_otp_to(&self, otp: &Otp, users: &[UserId]) -> Result<Envelope, ClientError> {
        let mut to: Vec<DeviceId> = Vec::new();
        for user in users {
            for id in self.recipients(user, None)? {
                if !to.contains(&id) {
                    to.push(id);
                }
            }
        }
        if to.len() > MAX_RECIPIENTS {
            return Err(ClientError::Invalid("plus de 32 appareils destinataires".into()));
        }
        self.seal_multi(to, &Body::Otp(otp.clone()))
    }

    /// Chiffre l'annonce d'un fichier déposé pour ces appareils, déjà
    /// résolus par [`Secure::recipients`] (le blob leur est ouvert).
    pub fn seal_file_to(
        &self,
        meta: &FileMeta,
        to: Vec<DeviceId>,
    ) -> Result<Envelope, ClientError> {
        self.seal_multi(to, &meta.body())
    }

    // --- Invitation, côté admin (§ 4.5) ---

    /// `user_invite` (ou `user_reinvite`) accepté : tire `S`, range
    /// l'invitation et rend le code (jeton, `S`, ancre). Une invitation
    /// précédente de la même personne est remplacée.
    pub fn create_invite(
        &mut self,
        user: UserId,
        name: &str,
        token: [u8; ot_crypto::TOKEN_LEN],
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<InviteCode, ClientError> {
        self.group()?;
        let anchor = self
            .directory
            .anchor
            .ok_or_else(|| ClientError::Invalid("pas d'annuaire : `admin init` d'abord".into()))?;
        let code = InviteCode { token, secret: PairingSecret::generate(), anchor };
        let mut invites = self.invites.clone();
        invites.retain(|r| r.alive(now) && r.user != user);
        invites.push(InviteRecord {
            user,
            name: name.to_owned(),
            token_id: code.token_id(),
            secret: *code.secret.as_bytes(),
            expires_at,
            guest: None,
            cert: None,
            certified: false,
        });
        self.keyring.save_invites(&invites).map_err(keyring_err)?;
        self.invites = invites;
        Ok(code)
    }

    /// Invitations dont la demande est arrivée : à certifier.
    pub fn invites_to_certify(&self) -> Vec<UserId> {
        self.invites.iter().filter(|r| r.guest.is_some()).map(|r| r.user.clone()).collect()
    }

    fn invite_index(&self, user: &UserId) -> Result<usize, ClientError> {
        self.invites
            .iter()
            .position(|r| &r.user == user && r.guest.is_some())
            .ok_or_else(|| ClientError::Invalid("aucune demande d'invitation pour elle".into()))
    }

    fn save_invite_list(&self) -> Result<(), ClientError> {
        self.keyring.save_invites(&self.invites).map_err(keyring_err)
    }

    /// Certificat de l'invité, signé par cet appareil la première fois puis
    /// rangé : une reprise après coupure envoie le même. Rend aussi si le
    /// relais l'a déjà accepté.
    pub fn invite_cert(
        &mut self,
        user: &UserId,
        now: OffsetDateTime,
    ) -> Result<(SignedBlob, bool), ClientError> {
        let i = self.invite_index(user)?;
        if let Some(cert) = &self.invites[i].cert {
            return Ok((cert.clone(), self.invites[i].certified));
        }
        let r = &self.invites[i];
        let Some(g) = &r.guest else {
            return Err(ClientError::Invalid("demande d'invitation absente".into()));
        };
        let genesis = Genesis {
            device_id: g.device_id.clone(),
            name: g.name.clone(),
            sign_key: g.sign_key,
            exchange_key: g.exchange_key,
        };
        let cert = UserCert::new(
            user.clone(),
            r.name.clone(),
            Role::Member,
            genesis,
            now.unix_timestamp(),
            self.me.clone(),
        )
        .sign(self.identity.signing_key())
        .map_err(crypto)?;
        self.invites[i].cert = Some(cert.clone());
        self.save_invite_list()?;
        Ok((cert, false))
    }

    /// `user_certify` accepté pour cette invitation.
    pub fn invite_certified(&mut self, user: &UserId) -> Result<(), ClientError> {
        let i = self.invite_index(user)?;
        self.invites[i].certified = true;
        self.save_invite_list()
    }

    /// Bienvenue scellée sous `S` pour l'appareil invité : son certificat et
    /// la tête de mon journal qui l'engage.
    pub fn invite_welcome(&self, user: &UserId, head: LogHead) -> Result<Envelope, ClientError> {
        let r = &self.invites[self.invite_index(user)?];
        let (Some(g), Some(cert)) = (&r.guest, &r.cert) else {
            return Err(ClientError::Invalid("certificat pas encore signé".into()));
        };
        let secret = r.secret();
        let body = Body::InviteWelcome { cert: cert.clone(), admin_head: head };
        let key = SealKey::Invite { token_id: r.token_id, secret: &secret };
        self.seal_env(vec![g.device_id.clone()], key, &body).map_err(crypto)
    }

    /// Bienvenue acceptée par le relais : l'invitation est terminée.
    pub fn invite_done(&mut self, user: &UserId) -> Result<Vec<Effect>, ClientError> {
        let i = self.invite_index(user)?;
        let r = self.invites.remove(i);
        self.save_invite_list()?;
        let Some(g) = &r.guest else { return Ok(Vec::new()) };
        tracing::info!(user = %r.user, device = %g.device_id, "personne invitée certifiée");
        Ok(vec![Effect::Emit(Event::PersonJoined {
            user: r.user.clone(),
            name: r.name.clone(),
            device_name: g.name.clone(),
            fingerprint: fingerprint(&g.sign_key),
        })])
    }

    /// Demande d'un invité, vérifiée par `S` (que seul cet appareil et
    /// l'invité connaissent) et par sa propre clé ; rangée **avant**
    /// l'acquittement, la suite est menée par le client.
    fn on_invite_request(
        &mut self,
        i: usize,
        env: &Envelope,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let opened = open(env, OpenKey::Invite(&self.invites[i].secret()))?;
        let Body::InviteRequest { user, device_id, name, sign_key, exchange_key } =
            opened.body()?
        else {
            return Err("invite_request attendu".into());
        };
        opened.verify(&sign_key)?;
        let r = &self.invites[i];
        if user != r.user || device_id != env.from {
            return Err("demande d'invitation pour une autre personne ou un autre appareil".into());
        }
        let guest = Guest { device_id, name, sign_key, exchange_key };
        match &r.guest {
            Some(g) if g == &guest => {
                return Err(Reject::Ignored("demande d'invitation déjà reçue"));
            }
            Some(_) => return Err("invitation déjà utilisée par un autre appareil".into()),
            None => {}
        }
        let mut invites = self.invites.clone();
        invites[i].guest = Some(guest);
        if let Err(e) = self.keyring.save_invites(&invites) {
            // Pas rangée : rejetée, l'invité la renverra à sa prochaine connexion.
            fx.push(Effect::Emit(Event::KeyringError { message: e.0.clone() }));
            return Err(Reject::Reason(e.0));
        }
        self.invites = invites;
        Ok(Handled::Done)
    }

    // --- Invitation, côté invité (§ 4.5) ---

    /// Demande à l'admin qui a invité, tant que la bienvenue n'est pas là
    /// (renvoyée à chaque connexion).
    pub fn invite_request(&self) -> Option<Result<Envelope, ClientError>> {
        let p = self.pending_invite.as_ref().filter(|p| p.welcome.is_none())?;
        let body = Body::InviteRequest {
            user: p.user.clone(),
            device_id: self.me.clone(),
            name: p.name.clone(),
            sign_key: self.identity.public_key(),
            exchange_key: self.exchange.public_key(),
        };
        let secret = p.secret();
        let key = SealKey::Invite { token_id: p.token_id, secret: &secret };
        Some(self.seal_env(vec![p.inviter.clone()], key, &body).map_err(crypto))
    }

    /// Une invitation attend sa bienvenue ou sa confirmation par l'annuaire.
    pub fn is_invited(&self) -> bool {
        self.pending_invite.is_some()
    }

    /// Bienvenue reçue : à confirmer par une lecture de l'annuaire.
    pub fn awaits_directory(&self) -> bool {
        self.pending_invite.as_ref().is_some_and(|p| p.welcome.is_some()) || self.awaits_service()
    }

    /// Bienvenue de l'admin : ouverte sous `S`, signée par l'appareil qui a
    /// invité (clé lue dans le roster vérifié de l'admin), certificat pour
    /// moi. Rangée avec la tête du journal de l'admin, que l'annuaire devra
    /// atteindre ; le groupe se crée une fois le certificat confirmé.
    fn on_invite_welcome(
        &mut self,
        env: &Envelope,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let p = self.pending_invite.as_ref().ok_or("aucune invitation en cours")?;
        if env.from != p.inviter {
            return Err("bienvenue d'un autre appareil que celui qui a invité".into());
        }
        if p.welcome.is_some() {
            return Err(Reject::Ignored("bienvenue déjà reçue"));
        }
        let Some(inviter) = self.directory.admin_roster().and_then(|r| r.member(&env.from)) else {
            // Le roster de l'admin n'est pas encore lu (ou pas à jour).
            return Ok(Handled::Quarantine);
        };
        let opened = open(env, OpenKey::Invite(&p.secret()))?;
        opened.verify(&inviter.sign_key)?;
        let Body::InviteWelcome { cert, admin_head } = opened.body()? else {
            return Err("invite_welcome attendu".into());
        };
        let c = UserCert::read_unverified(&cert).map_err(|e| Reject::Reason(e.to_string()))?;
        let g = &c.genesis;
        if c.user != p.user
            || c.role != Role::Member
            || g.device_id != self.me
            || g.sign_key != self.identity.public_key()
            || g.exchange_key != self.exchange.public_key()
        {
            fx.push(Effect::Emit(Event::InviteFailed {
                reason: "certificat qui ne désigne pas cet appareil".into(),
            }));
            return Err("certificat qui ne désigne pas cet appareil".into());
        }
        // La tête doit être atteinte par la lecture ; déjà dépassée, la
        // chaîne vérifiée depuis l'ancre la contient.
        let chain_seq = self.directory.admin.as_ref().and_then(|(_, k)| k.chain.as_ref());
        match chain_seq {
            Some(c) if c.seq == admin_head.seq && c.head != admin_head.hash => {
                fx.push(Effect::Emit(Event::InviteFailed {
                    reason: "journal de l'admin différent de celui de la bienvenue".into(),
                }));
                return Err("journal de l'admin différent de la bienvenue".into());
            }
            Some(c) if c.seq >= admin_head.seq => {}
            _ => self.directory.admin_head = Some(admin_head),
        }
        let mut pending = p.clone();
        pending.welcome = Some(Welcome { cert, admin_head });
        let saved = self
            .keyring
            .save_pending_invite(Some(&pending))
            .and_then(|()| self.keyring.save_directory(&self.directory));
        if let Err(e) = saved {
            return Err(Reject::Reason(e.0));
        }
        self.pending_invite = Some(pending);
        Ok(Handled::Done)
    }

    /// Après un tour complet de lecture : si l'annuaire confirme le
    /// certificat de la bienvenue (tête de l'admin atteinte, empreinte
    /// engagée, signataire dans son roster courant), crée mon groupe.
    /// L'entrée 1 de mon journal part ensuite (`Publish::IfBehind`).
    fn finish_invite(&mut self, fx: &mut Vec<Effect>) {
        let Some(p) = &self.pending_invite else { return };
        let Some(w) = &p.welcome else { return };
        if self.directory.admin_head.is_some() {
            return;
        }
        let confirmed = self.directory.known_person(&p.user).is_some_and(|k| k.cert == w.cert);
        if !confirmed {
            return;
        }
        let user = p.user.clone();
        if self.keys.is_none() {
            let member = Member {
                device_id: self.me.clone(),
                name: p.name.clone(),
                sign_key: self.identity.public_key(),
                exchange_key: self.exchange.public_key(),
                added_epoch: 1,
            };
            self.keys = Some(GroupKeys::new(1, GroupKey::generate()));
            self.save_keys(fx);
            self.set_roster(roster::genesis(member), fx);
            fx.push(Effect::Emit(Event::GroupChanged { epoch: 1 }));
        } else {
            tracing::warn!("invitation confirmée sur un appareil qui a déjà un groupe");
        }
        if let Err(e) = self.keyring.save_pending_invite(None) {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
        self.pending_invite = None;
        tracing::info!(user = %user, "invitation terminée : personne certifiée");
        fx.push(Effect::Emit(Event::Joined { user }));
    }

    // --- Mon journal (§ 4.2) et les certificats engagés par l'admin ---

    /// Prochaine entrée de mon journal pour `what`, signée par cet appareil,
    /// ou rien à faire. Mon roster local est publié tel quel s'il est plus
    /// récent ; sinon sa version passe au-dessus de celle du journal, et mon
    /// groupe l'adopte une fois publiée (note 0057, point 7).
    pub fn next_entry(&self, what: &Publish) -> Result<NextEntry, ClientError> {
        let quiet = matches!(what, Publish::IfBehind);
        let refuse = |why: &str| {
            if quiet { Ok(NextEntry::Done(None)) } else { Err(ClientError::Invalid(why.into())) }
        };
        let Some(me_user) = &self.me_user else {
            return refuse("annuaire pas encore lu");
        };
        if self.directory.admin_head.is_some() || self.people_at.is_none() {
            return refuse("annuaire pas encore lu jusqu'au bout");
        }
        let is_admin = self.directory.admin_id() == Some(me_user);
        if !quiet && !is_admin {
            return Err(ClientError::Invalid("réservé à l'admin".into()));
        }
        let Some(known) = self.directory.known_person(me_user) else {
            return refuse("ma personne n'est pas encore certifiée");
        };
        let Some(local) = &self.roster else { return refuse("cet appareil n'a pas de groupe") };
        let cert = UserCert::read_unverified(&known.cert).map_err(crypto)?;
        if cert.role == Role::Service {
            return refuse("le journal d'un service est tenu par l'admin");
        }
        let chain = known.chain.as_ref();
        let head = chain.map(Chain::head);
        let mut certs = chain.map(|c| c.certs.clone()).unwrap_or_default();
        match what {
            Publish::IfBehind => {}
            Publish::Certify(user, blob) => {
                let hash = cert_hash(blob);
                if certs.iter().any(|c| &c.user == user && c.cert == hash) {
                    return Ok(NextEntry::Done(head));
                }
                certs.retain(|c| &c.user != user);
                certs.push(CertRef { user: user.clone(), cert: hash });
            }
            Publish::Revoke(user) => {
                if !certs.iter().any(|c| &c.user == user) {
                    return Ok(NextEntry::Done(head));
                }
                certs.retain(|c| &c.user != user);
            }
        }
        let mut roster = local.clone();
        let (seq, prev) = match chain {
            None => {
                if cert.genesis.device_id != self.me {
                    return refuse("seul le premier appareil ouvre le journal");
                }
                (1, GENESIS_PREV)
            }
            Some(c) => {
                let mine = c.roster.member(&self.me);
                if mine.is_none_or(|m| m.sign_key != self.identity.public_key()) {
                    return refuse("cet appareil n'est pas dans le roster publié");
                }
                let behind = (roster.epoch, roster.version) <= (c.roster.epoch, c.roster.version);
                if quiet && behind {
                    return Ok(NextEntry::Done(None));
                }
                if roster.epoch < c.roster.epoch {
                    return refuse("roster local plus ancien que le journal publié");
                }
                if behind {
                    roster.version = roster.version.max(c.roster.version) + 1;
                }
                roster::check_keys_unchanged(&c.roster, &roster)
                    .map_err(|e| ClientError::Invalid(e.to_string()))?;
                (c.seq + 1, c.head)
            }
        };
        let entry = RosterEntry::new(me_user.clone(), seq, prev, self.me.clone(), roster.clone())
            .with_certs(certs)
            .sign(self.identity.signing_key())
            .map_err(crypto)?;
        // Les empreintes, triées comme dans l'entrée signée.
        let certs = RosterEntry::read_unverified(&entry).map_err(crypto)?.certs;
        Ok(NextEntry::Entry(Box::new(OwnEntry {
            user: me_user.clone(),
            entry: LogEntry { seq, entry },
            roster,
            certs,
        })))
    }

    /// Mon roster local est plus récent que mon journal publié.
    pub fn needs_publish(&self) -> bool {
        matches!(self.next_entry(&Publish::IfBehind), Ok(NextEntry::Entry(_)))
    }

    /// Entrée acceptée par le relais : elle prolonge mon journal vérifié ;
    /// si sa version dépasse mon roster local, mon groupe l'adopte (message
    /// `roster` à mes autres appareils).
    pub fn entry_published(&mut self, own: &OwnEntry) -> Vec<Effect> {
        let mut fx = Vec::new();
        let chain = Chain {
            seq: own.entry.seq,
            head: entry_hash(&own.entry.entry),
            signer: self.me.clone(),
            roster: own.roster.clone(),
            certs: own.certs.clone(),
        };
        self.directory.extend_own(&own.user, chain);
        if let Err(e) = self.keyring.save_directory(&self.directory) {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
        let adopt = self.roster.as_ref().is_some_and(|r| r != &own.roster);
        if let (true, Some(keys)) = (adopt, &self.keys)
            && let Some(key) = keys.current_key()
            && keys.current == own.roster.epoch
        {
            let group = SealKey::Group { epoch: keys.current, key };
            match self.seal_env(vec![], group, &Body::Roster { roster: own.roster.clone() }) {
                Ok(env) => fx.push(Effect::Send(env)),
                Err(e) => tracing::warn!(error = %e, "roster publié non diffusé"),
            }
            let epoch = keys.current;
            self.set_roster(own.roster.clone(), &mut fx);
            fx.push(Effect::Emit(Event::GroupChanged { epoch }));
        }
        fx
    }

    /// Certificat d'une personne re-signé par cet appareil (même `genesis`) :
    /// après le retrait de l'appareil de l'admin qui l'avait signé, ou un
    /// renommage (`name`).
    pub fn recertify(
        &self,
        user: &UserId,
        name: Option<&str>,
        now: OffsetDateTime,
    ) -> Result<SignedBlob, ClientError> {
        let known = self
            .directory
            .people
            .get(user)
            .ok_or_else(|| ClientError::NoRecipient("personne absente de l'annuaire".into()))?;
        let mut cert = UserCert::read_unverified(&known.cert).map_err(crypto)?;
        if cert.role == Role::Admin {
            return Err(ClientError::Invalid("le certificat de l'admin est l'ancre".into()));
        }
        if let Some(name) = name {
            cert.name = name.to_owned();
        }
        cert.issuer = self.me.clone();
        cert.issued_at = now.unix_timestamp().max(cert.issued_at);
        cert.sign(self.identity.signing_key()).map_err(crypto)
    }

    /// Personnes dont le certificat a été signé par `issuer`.
    pub fn certified_by(&self, issuer: &DeviceId) -> Vec<UserId> {
        self.directory
            .people
            .iter()
            .filter(|(_, k)| UserCert::read_unverified(&k.cert).is_ok_and(|c| &c.issuer == issuer))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Applique un tour complet de lecture de l'annuaire. Renvoie les effets
    /// (événements) et `true` si un autre tour est à faire, `known` à jour.
    pub fn apply_directory(
        &mut self,
        me: &UserId,
        admin: &UserId,
        users: &[DirectoryUser],
    ) -> (Vec<Effect>, bool) {
        let mut fx = Vec::new();
        let now = OffsetDateTime::now_utc();
        self.me_user = Some(me.clone());
        let before = self.directory.clone();
        let applied = self.directory.apply(admin, users);
        if self.directory != before
            && let Err(e) = self.keyring.save_directory(&self.directory)
        {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
        for event in applied.events {
            let event = match event {
                DirectoryEvent::Changed => Event::DirectoryChanged,
                DirectoryEvent::Rejected { user, reason } => {
                    tracing::warn!(user = %user, %reason, "objet de l'annuaire refusé");
                    Event::DirectoryRejected { user, reason: reason.to_string() }
                }
                DirectoryEvent::KeysChanged { user } => Event::KeysChanged { user },
                DirectoryEvent::Refetch { .. } => continue,
            };
            fx.push(Effect::Emit(event));
        }
        if let Some(people) = applied.people {
            // Profils et présence ne sont pas dans l'état rangé : un changement
            // de la vue compte aussi.
            if people != self.people
                && !fx.iter().any(|f| matches!(f, Effect::Emit(Event::DirectoryChanged)))
            {
                fx.push(Effect::Emit(Event::DirectoryChanged));
            }
            self.people = people;
            self.people_at = Some(now);
            // Des expéditeurs `0x05` inconnus le sont peut-être plus.
            self.retry_quarantine(now, &mut fx);
            self.finish_invite(&mut fx);
            self.finish_service(&mut fx);
        }
        let fork = self.roster.as_ref().is_some_and(|r| self.directory.is_fork(me, r));
        if fork && !self.fork {
            tracing::warn!("le journal publié de ma personne diffère de mon roster");
            fx.push(Effect::Emit(Event::DirectoryFork));
        }
        self.fork = fork;
        (fx, applied.again)
    }

    /// Objets de `admin_init` (note 0052) : certificat racine auto-signé par
    /// cet appareil (son `genesis`) et entrée 1 du journal de l'admin, son
    /// roster courant. Rien n'est rangé avant [`Secure::adopt_admin_root`].
    pub fn admin_root(
        &self,
        admin: &UserId,
        name: &str,
        now: OffsetDateTime,
    ) -> Result<(SignedBlob, SignedBlob), ClientError> {
        let (_, roster) = self.group()?;
        if self.has_anchor() {
            return Err(ClientError::Invalid("l'annuaire a déjà une ancre".into()));
        }
        let me = roster.member(&self.me).ok_or_else(|| ClientError::NotMember(self.me.clone()))?;
        let genesis = Genesis {
            device_id: self.me.clone(),
            name: me.name.clone(),
            sign_key: self.identity.public_key(),
            exchange_key: self.exchange.public_key(),
        };
        let key = self.identity.signing_key();
        let cert = UserCert::new(
            admin.clone(),
            name,
            Role::Admin,
            genesis,
            now.unix_timestamp(),
            self.me.clone(),
        )
        .sign(key)
        .map_err(crypto)?;
        let entry =
            RosterEntry::new(admin.clone(), 1, GENESIS_PREV, self.me.clone(), roster.clone())
                .sign(key)
                .map_err(crypto)?;
        Ok((cert, entry))
    }

    /// Certificat racine déjà publié par un autre appareil de l'admin :
    /// adopté seulement s'il est pour `admin`, auto-signé, et si son
    /// `genesis` est membre de mon roster avec les mêmes clés. Mon roster
    /// est vérifié par mon groupe : le relais ne peut pas y glisser un
    /// appareil.
    pub fn adopt_existing_root(
        &mut self,
        admin: &UserId,
        cert: &SignedBlob,
    ) -> Result<Vec<Effect>, ClientError> {
        let (_, roster) = self.group()?;
        let root = verify_root(cert, &anchor(cert)).map_err(crypto)?;
        let g = &root.genesis;
        let mine = roster
            .member(&g.device_id)
            .is_some_and(|m| m.sign_key == g.sign_key && m.exchange_key == g.exchange_key);
        if &root.user != admin || !mine {
            return Err(ClientError::Invalid(
                "certificat de l'admin publié par un appareil hors de mon groupe".into(),
            ));
        }
        Ok(self.adopt_admin_root(cert))
    }

    /// `admin_init` accepté par le relais : l'ancre est celle de ce
    /// certificat ; le journal se vérifie ensuite à la lecture.
    pub fn adopt_admin_root(&mut self, cert: &SignedBlob) -> Vec<Effect> {
        let mut fx = Vec::new();
        self.directory = DirectoryState::with_anchor(anchor(cert));
        if let Err(e) = self.keyring.save_directory(&self.directory) {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
        fx
    }

    fn group(&self) -> Result<(&GroupKeys, &Roster), ClientError> {
        match (&self.keys, &self.roster) {
            (Some(k), Some(r)) => Ok((k, r)),
            _ => Err(ClientError::NoGroup),
        }
    }

    fn seal_env(
        &self,
        to: Vec<DeviceId>,
        key: SealKey<'_>,
        body: &Body,
    ) -> Result<Envelope, CryptoError> {
        let empty = Payload { enc: Encoding::Xchacha20poly1305, data: Vec::new() };
        let mut env = Envelope::new(self.me.clone(), to, empty);
        env.payload = seal(&env, key, self.identity.signing_key(), body)?;
        Ok(env)
    }

    /// Chiffre un contenu sous l'époque courante. Les destinataires doivent
    /// être membres du roster ; vide : tous les autres appareils.
    pub fn seal_content(
        &self,
        content: &Content,
        to: Vec<DeviceId>,
    ) -> Result<Envelope, ClientError> {
        let (keys, roster) = self.group()?;
        if let Some(id) = to.iter().find(|id| !roster.contains(id)) {
            return Err(ClientError::NotMember(id.clone()));
        }
        let key = keys.current_key().ok_or(ClientError::NoGroup)?;
        let body = body_of(content);
        self.seal_env(to, SealKey::Group { epoch: keys.current, key }, &body).map_err(crypto)
    }

    /// Chiffre un code à usage unique (corps `otp`, note 0045) sous l'époque
    /// courante, pour des membres du roster (vide : tous).
    pub fn seal_otp(&self, otp: &Otp, to: Vec<DeviceId>) -> Result<Envelope, ClientError> {
        let (keys, roster) = self.group()?;
        if let Some(id) = to.iter().find(|id| !roster.contains(id)) {
            return Err(ClientError::NotMember(id.clone()));
        }
        let key = keys.current_key().ok_or(ClientError::NoGroup)?;
        let body = Body::Otp(otp.clone());
        self.seal_env(to, SealKey::Group { epoch: keys.current, key }, &body).map_err(crypto)
    }

    /// Chiffre l'annonce d'un fichier déposé (corps `file`, étape 5) sous
    /// l'époque courante, pour des membres du roster (vide : tous).
    pub fn seal_file(&self, meta: &FileMeta, to: Vec<DeviceId>) -> Result<Envelope, ClientError> {
        let (keys, roster) = self.group()?;
        if let Some(id) = to.iter().find(|id| !roster.contains(id)) {
            return Err(ClientError::NotMember(id.clone()));
        }
        let key = keys.current_key().ok_or(ClientError::NoGroup)?;
        self.seal_env(to, SealKey::Group { epoch: keys.current, key }, &meta.body()).map_err(crypto)
    }

    /// Offre reçue pour ce blob, avec sa clé (pour le téléchargement).
    pub fn file(&self, blob_id: &BlobId) -> Result<Option<StoredFile>, ClientError> {
        let files = self.keyring.load_files().map_err(keyring_err)?;
        Ok(files.into_iter().find(|f| &f.blob_id == blob_id))
    }

    /// Fichiers annoncés encore connus, du plus ancien au plus récent.
    pub fn files(&self) -> Result<Vec<FileInfo>, ClientError> {
        let mut files = self.keyring.load_files().map_err(keyring_err)?;
        prune_offers(&mut files, OffsetDateTime::now_utc());
        Ok(files.iter().map(StoredFile::info).collect())
    }

    /// Oublie une offre (fichier téléchargé, ou expiré sur le relais).
    pub fn forget_file(&self, blob_id: &BlobId) -> Result<(), ClientError> {
        let mut files = self.keyring.load_files().map_err(keyring_err)?;
        let before = files.len();
        files.retain(|f| &f.blob_id != blob_id);
        if files.len() != before {
            self.keyring.save_files(&files).map_err(keyring_err)?;
        }
        Ok(())
    }

    /// Dépôt en cours pour cette source, s'il n'a pas expiré et si la taille
    /// n'a pas changé.
    pub fn upload(
        &self,
        source_id: &str,
        size: u64,
        now: OffsetDateTime,
    ) -> Result<Option<PendingUpload>, ClientError> {
        let uploads = self.keyring.load_uploads().map_err(keyring_err)?;
        Ok(uploads
            .into_iter()
            .find(|u| u.source_id == source_id && u.meta.size == size && u.expires_at > now))
    }

    /// Tous les dépôts en cours de cette source, expirés compris.
    pub fn uploads_of(&self, source_id: &str) -> Result<Vec<PendingUpload>, ClientError> {
        let uploads = self.keyring.load_uploads().map_err(keyring_err)?;
        Ok(uploads.into_iter().filter(|u| u.source_id == source_id).collect())
    }

    /// Enregistre un dépôt en cours (remplace celui de la même source) et
    /// oublie les dépôts expirés.
    pub fn save_upload(
        &self,
        upload: PendingUpload,
        now: OffsetDateTime,
    ) -> Result<(), ClientError> {
        let mut uploads = self.keyring.load_uploads().map_err(keyring_err)?;
        uploads.retain(|u| u.source_id != upload.source_id && u.expires_at > now);
        uploads.push(upload);
        self.keyring.save_uploads(&uploads).map_err(keyring_err)
    }

    /// Oublie le dépôt en cours de cette source (terminé ou abandonné).
    pub fn forget_upload(&self, source_id: &str) -> Result<(), ClientError> {
        let mut uploads = self.keyring.load_uploads().map_err(keyring_err)?;
        let before = uploads.len();
        uploads.retain(|u| u.source_id != source_id);
        if uploads.len() != before {
            self.keyring.save_uploads(&uploads).map_err(keyring_err)?;
        }
        Ok(())
    }

    /// Le relais a créé un jeton : tire le secret et attend la demande.
    pub fn sponsor(
        &mut self,
        token: [u8; ot_crypto::TOKEN_LEN],
        expires_at: OffsetDateTime,
    ) -> Result<PairingCode, ClientError> {
        self.group()?;
        let code = PairingCode { token, secret: PairingSecret::generate() };
        let now = OffsetDateTime::now_utc();
        self.sponsoring.retain(|s| s.expires_at > now);
        self.sponsoring.push(Sponsoring {
            token_id: code.token_id(),
            secret: code.secret.clone(),
            expires_at,
        });
        Ok(code)
    }

    /// Nouvel appareil : prépare la demande d'appairage pour le parrain.
    pub fn start_join(
        &mut self,
        code: &PairingCode,
        sponsor: DeviceId,
        name: &str,
    ) -> Result<Envelope, ClientError> {
        let body = Body::PairRequest {
            name: name.to_owned(),
            sign_key: self.identity.public_key(),
            exchange_key: self.exchange.public_key(),
        };
        let key = SealKey::Pairing { token_id: code.token_id(), secret: &code.secret };
        let env = self.seal_env(vec![sponsor.clone()], key, &body).map_err(crypto)?;
        self.joining =
            Some(Joining { token_id: code.token_id(), secret: code.secret.clone(), sponsor });
        Ok(env)
    }

    /// Nouvel appareil, QR inversé : prépare l'offre à afficher et attend la
    /// réponse authentifiée par son secret.
    pub fn offer(&mut self, name: &str) -> Result<PairingOffer, ClientError> {
        let secret = PairingSecret::generate();
        let offer = PairingOffer::new(
            self.me.clone(),
            name,
            self.identity.signing_key(),
            self.exchange.public_key(),
            secret.clone(),
        )
        .map_err(|e| ClientError::Invalid(e.to_string()))?;
        self.joining_qr = Some(secret);
        Ok(offer)
    }

    /// Nouvel appareil, QR inversé : reprend une offre affichée avant un
    /// redémarrage (secret rangé par l'appli, note 0023).
    pub fn restore_offer(&mut self, secret: PairingSecret) {
        self.joining_qr = Some(secret);
    }

    /// Nouvel appareil, appairage par code : reprend l'attente de la réponse
    /// du parrain après un redémarrage, sans renvoyer la demande.
    pub fn restore_join(&mut self, code: &PairingCode, sponsor: DeviceId) {
        self.joining =
            Some(Joining { token_id: code.token_id(), secret: code.secret.clone(), sponsor });
    }

    /// Un appairage (par code ou QR inversé) attend encore sa réponse.
    pub fn is_joining(&self) -> bool {
        self.joining.is_some() || self.joining_qr.is_some()
    }

    /// Parrain, QR inversé : vérifie l'offre scannée avant de demander
    /// l'enregistrement au relais.
    pub fn check_offer(&self, offer: &PairingOffer) -> Result<(), ClientError> {
        let (_, roster) = self.group()?;
        offer.verify().map_err(|e| ClientError::Pairing(e.to_string()))?;
        let known = roster.members.iter().any(|m| {
            m.device_id == offer.device_id
                || m.sign_key == offer.sign_key
                || m.exchange_key == offer.exchange_key
        });
        if known {
            return Err(ClientError::Pairing("appareil déjà membre du groupe".into()));
        }
        Ok(())
    }

    /// Parrain, QR inversé, une fois l'appareil enregistré par le relais :
    /// l'ajoute au roster, lui scelle clés et roster (schéma `0x04`), et
    /// diffuse le roster. Renvoie la réponse au nouvel appareil à part, pour
    /// suivre son acceptation.
    pub fn welcome_offer(
        &mut self,
        offer: &PairingOffer,
    ) -> Result<(Envelope, Vec<Effect>), ClientError> {
        let member = Member {
            device_id: offer.device_id.clone(),
            name: offer.name.clone(),
            sign_key: offer.sign_key,
            exchange_key: offer.exchange_key,
            added_epoch: 0, // fixée par `admit`
        };
        let key = SealKey::PairQr { recipient: offer.exchange_key, secret: &offer.secret };
        let mut fx = Vec::new();
        let welcome = self.admit(member, key, &mut fx)?;
        Ok((welcome, fx))
    }

    /// Parrain : ajoute `member` au roster (à l'époque courante), lui scelle
    /// sous `key` les clés encore utiles et le nouveau roster, et diffuse ce
    /// roster à tous. Renvoie la réponse au nouvel appareil ; la diffusion et
    /// les événements vont dans `fx`.
    fn admit(
        &mut self,
        mut member: Member,
        key: SealKey<'_>,
        fx: &mut Vec<Effect>,
    ) -> Result<Envelope, ClientError> {
        let (keys, roster) = self.group()?;
        let epoch = keys.current;
        member.added_epoch = epoch;
        let (device_id, name) = (member.device_id.clone(), member.name.clone());
        let next =
            roster::with_member(roster, member).map_err(|e| ClientError::Pairing(e.to_string()))?;
        let current = keys.current_key().ok_or(ClientError::NoGroup)?.clone();
        let body = Body::PairWelcome {
            keys: keys
                .epochs
                .iter()
                .map(|e| EpochKey { epoch: e.epoch, key: e.key.clone() })
                .collect(),
            roster: next.clone(),
        };
        let welcome = self.seal_env(vec![device_id.clone()], key, &body).map_err(crypto)?;
        let group = SealKey::Group { epoch, key: &current };
        let broadcast =
            self.seal_env(vec![], group, &Body::Roster { roster: next.clone() }).map_err(crypto)?;
        self.set_roster(next, fx);
        fx.push(Effect::Send(broadcast));
        fx.push(Effect::Emit(Event::PairingCompleted { device_id, name }));
        fx.push(Effect::Emit(Event::GroupChanged { epoch }));
        Ok(welcome)
    }

    /// Tire une nouvelle clé de groupe, la scelle pour chaque membre restant
    /// et l'adopte. Renvoie l'époque et les messages `group_key` à envoyer.
    pub fn rotate(
        &mut self,
        exclude: &[DeviceId],
        now: OffsetDateTime,
    ) -> Result<(u32, Vec<Envelope>, Vec<Effect>), ClientError> {
        let (keys, roster) = self.group()?;
        if exclude.contains(&self.me) {
            return Err(ClientError::Invalid("impossible de s'exclure soi-même".into()));
        }
        if let Some(id) = exclude.iter().find(|id| !roster.contains(id)) {
            return Err(ClientError::NotMember(id.clone()));
        }
        let epoch = keys
            .current
            .checked_add(1)
            .ok_or(ClientError::Invalid("époque maximale atteinte".into()))?;
        let key = GroupKey::generate();
        let next = roster::rotated(roster, epoch, exclude);
        let body = Body::GroupKey { epoch, key: key.clone(), roster: next.clone() };
        let mut envs = Vec::new();
        for m in next.members.iter().filter(|m| m.device_id != self.me) {
            let sealed = SealKey::Device { recipient: m.exchange_key };
            envs.push(self.seal_env(vec![m.device_id.clone()], sealed, &body).map_err(crypto)?);
        }
        let mut fx = Vec::new();
        if let Some(keys) = self.keys.as_mut() {
            keys.advance(epoch, key, Some(self.me.clone()), now);
        }
        self.save_keys(&mut fx);
        self.set_roster(next, &mut fx);
        fx.push(Effect::Emit(Event::GroupChanged { epoch }));
        Ok((epoch, envs, fx))
    }

    /// Traite un message livré par le relais. Il sera acquitté quoi qu'il
    /// arrive : remis, mis de côté, ou rejeté.
    pub fn process(&mut self, seq: i64, env: Envelope, now: OffsetDateTime) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.process_one(seq, env, now, &mut fx) {
            self.retry_quarantine(now, &mut fx);
        }
        fx
    }

    /// Renvoie `true` si les clés ont changé.
    fn process_one(
        &mut self,
        seq: i64,
        env: Envelope,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> bool {
        match self.handle(seq, &env, now, fx) {
            Ok(Handled::Done) => false,
            Ok(Handled::KeysChanged) => true,
            Ok(Handled::Quarantine) => {
                self.put_aside(seq, env, now, fx);
                false
            }
            Err(Reject::Ignored(why)) => {
                tracing::debug!(seq, msg_id = %env.msg_id, why, "message ignoré");
                false
            }
            Err(Reject::Reason(reason)) => {
                tracing::warn!(seq, msg_id = %env.msg_id, from = %env.from, reason, "message rejeté");
                fx.push(Effect::Emit(Event::Rejected {
                    msg_id: env.msg_id,
                    from: env.from,
                    reason,
                }));
                false
            }
        }
    }

    fn handle(
        &mut self,
        seq: i64,
        env: &Envelope,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        match Header::parse(&env.payload)? {
            Header::Group { epoch } => {
                let Some(entry) = self.keys.as_ref().and_then(|k| k.get(epoch)) else {
                    return Ok(Handled::Quarantine);
                };
                let opened = open(env, OpenKey::Group(&entry.key))?;
                let roster = self.roster.as_ref().ok_or("aucun roster")?;
                let sender = roster.member(&env.from).ok_or("expéditeur hors roster")?;
                opened.verify(&sender.sign_key)?;
                match opened.body()? {
                    Body::Roster { roster } => self.on_roster(env, epoch, roster, fx),
                    body => {
                        self.deliver(seq, env, body, Origin::default(), now, fx)?.map_err(|kind| {
                            Reject::Reason(format!("{kind} inattendu sous la clé de groupe"))
                        })
                    }
                }
            }
            Header::Pairing { token_id } => {
                if let Some(i) = self
                    .sponsoring
                    .iter()
                    .position(|s| s.token_id == token_id && s.expires_at > now)
                {
                    self.on_pair_request(i, env, fx)
                } else if self.joining.as_ref().is_some_and(|j| j.token_id == token_id) {
                    self.on_pair_welcome(env, now, fx)
                } else if let Some(i) =
                    self.invites.iter().position(|r| r.token_id == token_id && r.alive(now))
                {
                    self.on_invite_request(i, env, fx)
                } else if self.pending_invite.as_ref().is_some_and(|p| p.token_id == token_id) {
                    self.on_invite_welcome(env, fx)
                } else {
                    Err("appairage inconnu ou expiré".into())
                }
            }
            Header::Device { .. } => self.on_group_key(env, now, fx),
            Header::PairQr { .. } => {
                let Some(secret) = &self.joining_qr else {
                    return Err("appairage par QR inconnu ou terminé".into());
                };
                let opened = open(env, OpenKey::PairQr(&self.exchange, secret))?;
                match opened.body()? {
                    body @ Body::ServiceWelcome { .. } => {
                        self.on_service_welcome(env, opened, body)
                    }
                    _ => self.adopt_welcome(env, opened, now, fx),
                }
            }
            // Étape 10 : d'une autre personne (annuaire vérifié), ou d'un de
            // mes appareils (copie d'un envoi, D43). Expéditeur inconnu :
            // mis de côté jusqu'à la prochaine lecture de l'annuaire.
            Header::Multi { .. } => {
                let Some((sign_key, mut origin)) = self.sender(&env.from) else {
                    return Ok(Handled::Quarantine);
                };
                let opened = open(env, OpenKey::Multi { me: &self.me, key: &self.exchange })?;
                opened.verify(&sign_key)?;
                if origin.from_user.is_none() {
                    origin.to_user = self.addressee(&env.to);
                }
                let body = opened.body()?;
                self.deliver(seq, env, body, origin, now, fx)?
                    .map_err(|kind| Reject::Reason(format!("{kind} inattendu en 0x05")))
            }
        }
    }

    /// Remet à l'appli un corps de contenu (texte, lien, fichier, code).
    /// Tout autre corps n'est pas remis : `Ok(Err(nom du corps))`.
    fn deliver(
        &mut self,
        seq: i64,
        env: &Envelope,
        body: Body,
        origin: Origin,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> Result<Result<Handled, &'static str>, Reject> {
        let content = match body {
            Body::Text { body } => Content::text(body),
            Body::Link { body } => Content::link(body),
            body @ Body::File { .. } => {
                return self.on_file(seq, env, body, origin, now, fx).map(Ok);
            }
            Body::Otp(otp) => {
                fx.push(Effect::Emit(Event::Otp(ReceivedOtp {
                    seq,
                    msg_id: env.msg_id,
                    from: env.from.clone(),
                    to: env.to.clone(),
                    created_at: env.created_at,
                    otp,
                    origin,
                })));
                return Ok(Ok(Handled::Done));
            }
            other => return Ok(Err(other.kind())),
        };
        fx.push(Effect::Emit(Event::Message(Received {
            seq,
            msg_id: env.msg_id,
            from: env.from.clone(),
            to: env.to.clone(),
            created_at: env.created_at,
            content,
            origin,
        })));
        Ok(Ok(Handled::Done))
    }

    fn on_roster(
        &mut self,
        env: &Envelope,
        epoch: u32,
        new: Roster,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        if new.epoch != epoch {
            return Err("roster d'une autre époque que sa clé".into());
        }
        let current = self.roster.as_ref().ok_or("aucun roster")?;
        roster::check_replacement(current, &new, &env.from, &self.me)?;
        self.set_roster(new, fx);
        fx.push(Effect::Emit(Event::GroupChanged { epoch }));
        Ok(Handled::Done)
    }

    /// Parrain : vérifie la demande, ajoute l'appareil, lui envoie la clé et
    /// le roster, puis le roster à tous.
    /// Fichier annoncé : l'offre (clé comprise) est rangée avant que le
    /// message soit acquitté, puis signalée à l'appli sans la clé.
    fn on_file(
        &mut self,
        seq: i64,
        env: &Envelope,
        body: Body,
        origin: Origin,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let stored =
            stored_from_body(env.msg_id, env.from.clone(), env.created_at, now, body, origin)
                .ok_or("corps de fichier attendu")?;
        let file = stored.info();
        let mut files = self.keyring.load_files().map_err(|e| Reject::Reason(e.0))?;
        add_offer(&mut files, stored, now);
        self.keyring.save_files(&files).map_err(|e| Reject::Reason(e.0))?;
        fx.push(Effect::Emit(Event::File(ReceivedFile { seq, file, to: env.to.clone() })));
        Ok(Handled::Done)
    }

    fn on_pair_request(
        &mut self,
        i: usize,
        env: &Envelope,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let opened = open(env, OpenKey::Pairing(&self.sponsoring[i].secret))?;
        let Body::PairRequest { name, sign_key, exchange_key } = opened.body()? else {
            return Err("pair_request attendu".into());
        };
        opened.verify(&sign_key)?;
        // `added_epoch` est fixée par `admit`.
        let member =
            Member { device_id: env.from.clone(), name, sign_key, exchange_key, added_epoch: 0 };
        let (token_id, secret) = (self.sponsoring[i].token_id, self.sponsoring[i].secret.clone());
        let pairing = SealKey::Pairing { token_id, secret: &secret };
        // La réponse au nouvel appareil part avant la diffusion du roster.
        let mut admitted = Vec::new();
        let welcome = self
            .admit(member, pairing, &mut admitted)
            .map_err(|e| Reject::Reason(e.to_string()))?;
        self.sponsoring.remove(i);
        fx.push(Effect::Send(welcome));
        fx.extend(admitted);
        Ok(Handled::Done)
    }

    /// Nouvel appareil : adopte les clés et le roster du parrain.
    fn on_pair_welcome(
        &mut self,
        env: &Envelope,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let joining = self.joining.as_ref().ok_or("aucun appairage en cours")?;
        if env.from != joining.sponsor {
            return Err("pair_welcome d'un autre appareil que le parrain".into());
        }
        let opened = open(env, OpenKey::Pairing(&joining.secret))?;
        self.adopt_welcome(env, opened, now, fx)
    }

    /// Nouvel appareil : vérifie la réponse ouverte (par le code ou par le QR
    /// inversé) et adopte clés et roster.
    fn adopt_welcome(
        &mut self,
        env: &Envelope,
        opened: Opened,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let Body::PairWelcome { keys, roster } = opened.body()? else {
            return Err("pair_welcome attendu".into());
        };
        let sponsor = roster.member(&env.from).ok_or("parrain absent du roster")?;
        opened.verify(&sponsor.sign_key)?;
        roster::check_shape(&roster, &self.me)?;
        let me = roster.member(&self.me).ok_or("appareil absent du roster")?;
        if me.sign_key != self.identity.public_key()
            || me.exchange_key != self.exchange.public_key()
        {
            return Err("le roster ne porte pas les clés de cet appareil".into());
        }
        let current = keys
            .iter()
            .find(|k| k.epoch == roster.epoch)
            .ok_or("clé de l'époque courante absente")?;
        let mut group = GroupKeys::new(roster.epoch, current.key.clone());
        for k in keys.iter().filter(|k| k.epoch != roster.epoch) {
            group.insert_retired(k.epoch, k.key.clone(), None, now);
        }
        let epoch = roster.epoch;
        self.keys = Some(group);
        self.joining = None;
        self.joining_qr = None;
        self.save_keys(fx);
        self.set_roster(roster, fx);
        fx.push(Effect::Emit(Event::GroupChanged { epoch }));
        Ok(Handled::KeysChanged)
    }

    /// Nouvelle clé de groupe scellée pour cet appareil. La plus haute époque
    /// gagne ; à égalité, celle tirée par le plus petit `device_id`.
    fn on_group_key(
        &mut self,
        env: &Envelope,
        now: OffsetDateTime,
        fx: &mut Vec<Effect>,
    ) -> Result<Handled, Reject> {
        let opened = open(env, OpenKey::Device(&self.exchange))?;
        let roster = self.roster.as_ref().ok_or("aucun groupe")?;
        let sender = roster.member(&env.from).ok_or("expéditeur hors roster")?;
        opened.verify(&sender.sign_key)?;
        let Body::GroupKey { epoch, key, roster: next } = opened.body()? else {
            return Err("group_key attendu".into());
        };
        if next.epoch != epoch {
            return Err("roster d'une autre époque que la clé".into());
        }
        roster::check_shape(&next, &self.me)?;
        roster::check_keys_unchanged(roster, &next)?;
        let keys = self.keys.as_mut().ok_or("aucun groupe")?;
        let from = Some(env.from.clone());
        let adopt_roster = if epoch > keys.current {
            keys.advance(epoch, key, from, now);
            true
        } else if epoch == keys.current {
            let Some(current) = keys.get(epoch) else {
                return Err("époque courante introuvable".into());
            };
            if current.key == key {
                return Err(Reject::Ignored("clé déjà connue"));
            }
            // Rotation concurrente : le plus petit `device_id` l'emporte.
            if !current.author.as_ref().is_some_and(|a| env.from < *a) {
                return Err(Reject::Ignored("rotation concurrente perdue"));
            }
            keys.advance(epoch, key, from, now);
            true
        } else {
            keys.insert_retired(epoch, key, from, now);
            false
        };
        self.save_keys(fx);
        if adopt_roster {
            self.set_roster(next, fx);
        }
        let current = self.keys.as_ref().map_or(epoch, |k| k.current);
        fx.push(Effect::Emit(Event::GroupChanged { epoch: current }));
        Ok(Handled::KeysChanged)
    }

    /// Met de côté un message d'époque ou d'expéditeur inconnus (au plus
    /// `MAX_QUARANTINE`, pas plus de 7 jours).
    fn put_aside(&mut self, seq: i64, env: Envelope, now: OffsetDateTime, fx: &mut Vec<Effect>) {
        tracing::debug!(seq, msg_id = %env.msg_id, "époque inconnue, message mis de côté");
        self.quarantine.retain(|q| now - q.received_at < RETIRED_KEY_LIFETIME);
        self.quarantine.push(Quarantined { seq, received_at: now, env });
        let excess = self.quarantine.len().saturating_sub(MAX_QUARANTINE);
        for dropped in self.quarantine.drain(..excess) {
            tracing::warn!(msg_id = %dropped.env.msg_id, "quarantaine pleine, message abandonné");
            fx.push(Effect::Emit(Event::Rejected {
                msg_id: dropped.env.msg_id,
                from: dropped.env.from,
                reason: "quarantaine pleine".into(),
            }));
        }
        self.save_quarantine(fx);
    }

    /// Réessaie les messages mis de côté dont la clé est maintenant connue.
    fn retry_quarantine(&mut self, now: OffsetDateTime, fx: &mut Vec<Effect>) {
        loop {
            let all = std::mem::take(&mut self.quarantine);
            let known = |q: &Quarantined| match Header::parse(&q.env.payload) {
                Ok(Header::Group { epoch }) => {
                    self.keys.as_ref().is_some_and(|k| k.get(epoch).is_some())
                }
                Ok(Header::Multi { .. }) => self.sender(&q.env.from).is_some(),
                _ => true,
            };
            let (ready, waiting): (Vec<_>, Vec<_>) = all.into_iter().partition(known);
            self.quarantine = waiting;
            if ready.is_empty() {
                return;
            }
            self.save_quarantine(fx);
            let mut changed = false;
            for q in ready {
                changed |= self.process_one(q.seq, q.env, now, fx);
            }
            if !changed {
                return;
            }
        }
    }

    fn set_roster(&mut self, roster: Roster, fx: &mut Vec<Effect>) {
        if let Err(e) = self.keyring.save_roster(&roster) {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
        self.roster = Some(roster);
    }

    fn save_keys(&mut self, fx: &mut Vec<Effect>) {
        if let Some(keys) = &self.keys
            && let Err(e) = self.keyring.save_group_keys(keys)
        {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
    }

    fn save_quarantine(&mut self, fx: &mut Vec<Effect>) {
        if let Err(e) = self.keyring.save_quarantine(&self.quarantine) {
            fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
        }
    }
}

/// Erreur du keyring, remontée telle quelle à l'appli.
fn keyring_err(e: KeyringError) -> ClientError {
    ClientError::Invalid(e.to_string())
}

fn crypto(e: CryptoError) -> ClientError {
    ClientError::Crypto(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryKeyring;

    struct Dev {
        id: DeviceId,
        identity: Identity,
        keyring: Arc<MemoryKeyring>,
        secure: Secure,
    }

    impl Dev {
        fn new() -> Self {
            let id = DeviceId::generate();
            let identity = Identity::generate();
            let keyring = Arc::new(MemoryKeyring::new());
            let (secure, _) = Secure::load(id.clone(), identity.clone(), keyring.clone());
            Self { id, identity, keyring, secure }
        }

        fn member(&self, name: &str) -> Member {
            Member {
                device_id: self.id.clone(),
                name: name.into(),
                sign_key: self.identity.public_key(),
                exchange_key: self.secure.exchange.public_key(),
                added_epoch: 1,
            }
        }

        fn reload(&mut self) {
            let keyring: Arc<dyn Keyring> = self.keyring.clone();
            self.secure = Secure::load(self.id.clone(), self.identity.clone(), keyring).0;
        }

        fn send(&self, text: &str) -> Envelope {
            self.secure.seal_content(&Content::text(text), vec![]).unwrap()
        }

        fn receive(&mut self, env: Envelope) -> Vec<Effect> {
            self.secure.process(1, env, OffsetDateTime::now_utc())
        }
    }

    /// Appareils déjà membres d'un même groupe, époque 1.
    fn group(n: usize) -> Vec<Dev> {
        let mut devs: Vec<Dev> = (0..n).map(|_| Dev::new()).collect();
        let roster = Roster {
            epoch: 1,
            version: 1,
            members: devs.iter().enumerate().map(|(i, d)| d.member(&format!("d{i}"))).collect(),
        };
        let keys = GroupKeys::new(1, GroupKey::generate());
        for d in &mut devs {
            d.keyring.save_group_keys(&keys).unwrap();
            d.keyring.save_roster(&roster).unwrap();
            d.reload();
        }
        devs
    }

    fn texts(fx: &[Effect]) -> Vec<String> {
        fx.iter()
            .filter_map(|f| match f {
                Effect::Emit(Event::Message(m)) => Some(m.content.body.clone()),
                _ => None,
            })
            .collect()
    }

    fn rejected(fx: &[Effect]) -> Vec<String> {
        fx.iter()
            .filter_map(|f| match f {
                Effect::Emit(Event::Rejected { reason, .. }) => Some(reason.clone()),
                _ => None,
            })
            .collect()
    }

    fn sends(fx: &[Effect]) -> Vec<Envelope> {
        fx.iter()
            .filter_map(|f| match f {
                Effect::Send(env) => Some(env.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn two_members_exchange() {
        let mut d = group(2);
        let env = d[0].send("bonjour");
        assert_eq!(env.payload.enc, Encoding::Xchacha20poly1305);
        assert!(!String::from_utf8_lossy(&env.payload.data).contains("bonjour"));
        let fx = d[1].receive(env);
        assert_eq!(texts(&fx), ["bonjour"]);
    }

    #[test]
    fn otp_reaches_its_recipient_only() {
        let mut d = group(3);
        let otp = Otp {
            code: "482913".into(),
            display: "G-482913".into(),
            sender_name: "Google".into(),
            sender_domain: "google.com".into(),
            sender_verified: true,
            expires_at: 1_793_628_791,
        };
        let env = d[0].secure.seal_otp(&otp, vec![d[1].id.clone()]).unwrap();
        assert!(!String::from_utf8_lossy(&env.payload.data).contains("482913"));
        assert_eq!(env.to, [d[1].id.clone()]);
        let fx = d[1].receive(env);
        let got: Vec<_> = fx
            .iter()
            .filter_map(|f| match f {
                Effect::Emit(Event::Otp(o)) => Some(o.otp.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(got, std::slice::from_ref(&otp));
        let outsider = DeviceId::generate();
        assert!(matches!(
            d[0].secure.seal_otp(&otp, vec![outsider]),
            Err(ClientError::NotMember(_))
        ));
    }

    #[test]
    fn plaintext_is_rejected() {
        let mut d = group(2);
        let env = Envelope::new_plain(d[0].id.clone(), vec![], &Content::text("en clair"));
        let fx = d[1].receive(env);
        assert!(texts(&fx).is_empty());
        assert_eq!(rejected(&fx), ["message en clair refusé"]);
    }

    #[test]
    fn sender_outside_roster_is_rejected() {
        let mut d = group(2);
        // Un appareil qui a la clé de groupe mais n'est pas au roster de B.
        let mut stranger = Dev::new();
        stranger
            .keyring
            .save_group_keys(&d[0].keyring.load_group_keys().unwrap().unwrap())
            .unwrap();
        let mut roster = d[0].keyring.load_roster().unwrap().unwrap();
        roster.members.push(stranger.member("intrus"));
        stranger.keyring.save_roster(&roster).unwrap();
        stranger.reload();
        let fx = d[1].receive(stranger.send("je suis des vôtres"));
        assert!(texts(&fx).is_empty());
        assert_eq!(rejected(&fx), ["expéditeur hors roster"]);
    }

    #[test]
    fn forged_signature_is_rejected() {
        let mut d = group(2);
        // Chiffré sous la bonne clé, au nom de A, mais signé par une autre clé.
        let keys = d[0].keyring.load_group_keys().unwrap().unwrap();
        let mut env = d[0].send("x");
        let forger = Identity::generate();
        let key = SealKey::Group { epoch: 1, key: keys.current_key().unwrap() };
        env.payload =
            seal(&env, key, forger.signing_key(), &Body::Text { body: "faux".into() }).unwrap();
        let fx = d[1].receive(env);
        assert!(texts(&fx).is_empty());
        assert_eq!(rejected(&fx), ["signature invalide"]);
    }

    #[test]
    fn no_group_no_send() {
        let d = Dev::new();
        let err = d.secure.seal_content(&Content::text("x"), vec![]).unwrap_err();
        assert_eq!(err, ClientError::NoGroup);
    }

    #[test]
    fn targeted_send_only_to_members() {
        let d = group(2);
        let outsider = DeviceId::generate();
        let err =
            d[0].secure.seal_content(&Content::text("x"), vec![outsider.clone()]).unwrap_err();
        assert_eq!(err, ClientError::NotMember(outsider));
        assert!(d[0].secure.seal_content(&Content::text("x"), vec![d[1].id.clone()]).is_ok());
    }

    #[test]
    fn quarantine_is_released_when_key_arrives() {
        let mut d = group(3);
        let (epoch, group_keys, _) = d[0].secure.rotate(&[], OffsetDateTime::now_utc()).unwrap();
        assert_eq!(epoch, 2);
        let late = d[0].send("après rotation");

        // Le message arrive avant sa clé : mis de côté, sans bruit.
        let fx = d[1].receive(late);
        assert!(texts(&fx).is_empty() && rejected(&fx).is_empty());
        assert_eq!(d[1].keyring.load_quarantine().unwrap().len(), 1);

        // Puis la clé : le message sort de quarantaine.
        let for_b = group_keys.into_iter().find(|e| e.to == [d[1].id.clone()]).unwrap();
        let fx = d[1].receive(for_b);
        assert_eq!(texts(&fx), ["après rotation"]);
        assert!(d[1].keyring.load_quarantine().unwrap().is_empty());
        assert_eq!(d[1].secure.info().unwrap().epoch, 2);
    }

    #[test]
    fn quarantine_survives_restart() {
        let mut d = group(2);
        let (_, group_keys, _) = d[0].secure.rotate(&[], OffsetDateTime::now_utc()).unwrap();
        let later = d[0].send("plus tard");
        d[1].receive(later);
        d[1].reload();
        let fx = d[1].receive(group_keys.into_iter().next().unwrap());
        assert_eq!(texts(&fx), ["plus tard"]);
    }

    #[test]
    fn rotation_excludes_a_member() {
        let mut d = group(3);
        let c = d[2].id.clone();
        let old = d[0].send("avant");
        let (_, group_keys, _) =
            d[0].secure.rotate(std::slice::from_ref(&c), OffsetDateTime::now_utc()).unwrap();
        assert_eq!(group_keys.len(), 1, "seul B reçoit la nouvelle clé");
        let fx = d[1].receive(group_keys.into_iter().next().unwrap());
        assert!(matches!(fx.last(), Some(Effect::Emit(Event::GroupChanged { epoch: 2 }))));
        assert!(!d[1].secure.info().unwrap().members.iter().any(|m| m.device_id == c));

        // C ne lit pas le nouveau message, B si ; B lit encore l'ancien.
        let new = d[0].send("après");
        let fx = d[2].receive(new.clone());
        assert!(texts(&fx).is_empty());
        assert_eq!(texts(&d[1].receive(new)), ["après"]);
        assert_eq!(texts(&d[1].receive(old)), ["avant"]);
        // C, exclu, ne peut plus écrire à B.
        let from_c = d[2].send("toujours là ?");
        assert_eq!(rejected(&d[1].receive(from_c)), ["expéditeur hors roster"]);
    }

    #[test]
    fn concurrent_rotations_converge_on_smallest_device_id() {
        let mut d = group(3);
        let now = OffsetDateTime::now_utc();
        let (_, from_0, _) = d[0].secure.rotate(&[], now).unwrap();
        let (_, from_1, _) = d[1].secure.rotate(&[], now).unwrap();
        let winner = if d[0].id < d[1].id { 0 } else { 1 };
        let for_dev = |envs: &[Envelope], i: usize| {
            envs.iter().find(|e| e.to == [d[i].id.clone()]).unwrap().clone()
        };
        let (a, b) = (for_dev(&from_1, 0), for_dev(&from_0, 1));
        let (c0, c1) = (for_dev(&from_0, 2), for_dev(&from_1, 2));
        d[0].receive(a);
        d[1].receive(b);
        d[2].receive(c0);
        d[2].receive(c1);
        let fp: Vec<String> = d.iter().map(|x| x.secure.info().unwrap().key_fingerprint).collect();
        assert!(fp.iter().all(|f| f == &fp[winner]), "{fp:?}");
        let ok = d[0].send("ok");
        assert_eq!(texts(&d[2].receive(ok)), ["ok"]);
    }

    #[test]
    fn pairing_adds_member_and_shares_key() {
        let mut d = group(2);
        let mut new = Dev::new();
        let expires = OffsetDateTime::now_utc() + time::Duration::minutes(10);
        let code = d[0].secure.sponsor([7; 10], expires).unwrap();

        let request = new.secure.start_join(&code, d[0].id.clone(), "nouveau").unwrap();
        let fx = d[0].receive(request);
        assert!(fx.iter().any(
            |f| matches!(f, Effect::Emit(Event::PairingCompleted { name, .. }) if name == "nouveau")
        ));
        let out = sends(&fx);
        assert_eq!(out.len(), 2, "pair_welcome au nouveau, roster à tous");

        let fx = new.receive(out[0].clone());
        assert!(matches!(fx.last(), Some(Effect::Emit(Event::GroupChanged { epoch: 1 }))));
        // Un tiers apprend le nouveau membre par le message `roster`.
        d[1].receive(out[1].clone());
        assert_eq!(texts(&d[1].receive(new.send("salut"))), ["salut"]);
        assert_eq!(texts(&new.receive(d[1].send("bienvenue"))), ["bienvenue"]);
        // Le roster rediffusé est déjà connu du nouveau : ignoré sans bruit.
        assert!(rejected(&new.receive(out[1].clone())).is_empty());
    }

    #[test]
    fn tampered_pairing_fails() {
        let mut d = group(1);
        let mut new = Dev::new();
        let expires = OffsetDateTime::now_utc() + time::Duration::minutes(10);
        let code = d[0].secure.sponsor([7; 10], expires).unwrap();
        let mut request = new.secure.start_join(&code, d[0].id.clone(), "nouveau").unwrap();
        *request.payload.data.last_mut().unwrap() ^= 1;
        let fx = d[0].receive(request);
        assert_eq!(rejected(&fx), ["déchiffrement impossible"]);
        assert_eq!(d[0].secure.info().unwrap().members.len(), 1);

        // Le jeton reste valable : une demande intacte passe, mais une
        // réponse altérée ne donne pas le groupe.
        let request = new.secure.start_join(&code, d[0].id.clone(), "nouveau").unwrap();
        let mut welcome = sends(&d[0].receive(request)).remove(0);
        welcome.payload.data[20] ^= 1;
        assert_eq!(rejected(&new.receive(welcome)), ["déchiffrement impossible"]);
        assert!(new.secure.info().is_none());
    }

    #[test]
    fn expired_pairing_is_refused() {
        let mut d = group(1);
        let mut new = Dev::new();
        let expired = OffsetDateTime::now_utc() - time::Duration::SECOND;
        let code = d[0].secure.sponsor([7; 10], expired).unwrap();
        let request = new.secure.start_join(&code, d[0].id.clone(), "nouveau").unwrap();
        assert_eq!(rejected(&d[0].receive(request)), ["appairage inconnu ou expiré"]);
    }

    #[test]
    fn pairing_by_reverse_qr() {
        let mut d = group(2);
        let mut new = Dev::new();
        let offer = new.secure.offer("windows").unwrap();
        // Le parrain relit le QR scanné.
        let scanned = PairingOffer::parse(&offer.encode()).unwrap();
        d[0].secure.check_offer(&scanned).unwrap();
        let (welcome, fx) = d[0].secure.welcome_offer(&scanned).unwrap();
        assert!(matches!(Header::parse(&welcome.payload), Ok(Header::PairQr { .. })));
        assert!(fx.iter().any(
            |f| matches!(f, Effect::Emit(Event::PairingCompleted { name, .. }) if name == "windows")
        ));
        let broadcast = sends(&fx).remove(0);

        let fx = new.receive(welcome);
        assert!(matches!(fx.last(), Some(Effect::Emit(Event::GroupChanged { epoch: 1 }))));
        d[1].receive(broadcast);
        assert_eq!(texts(&d[1].receive(new.send("salut"))), ["salut"]);
        assert_eq!(texts(&new.receive(d[1].send("bienvenue"))), ["bienvenue"]);
        // Une seconde offre du même appareil est refusée.
        assert!(d[0].secure.check_offer(&scanned).is_err());
    }

    #[test]
    fn reverse_qr_welcome_needs_the_secret() {
        let mut d = group(1);
        let mut new = Dev::new();
        let offer = new.secure.offer("windows").unwrap();
        // Un relais qui connaît les clés publiques mais pas `S` fabrique une
        // réponse : elle ne s'ouvre pas.
        let mut forged = offer.clone();
        forged.secret = PairingSecret::generate();
        let (welcome, _) = d[0].secure.welcome_offer(&forged).unwrap();
        assert_eq!(rejected(&new.receive(welcome)), ["déchiffrement impossible"]);
        assert!(new.secure.info().is_none());
    }

    #[test]
    fn unsolicited_reverse_qr_welcome_is_rejected() {
        let mut d = group(1);
        let mut new = Dev::new();
        let offer = new.secure.offer("windows").unwrap();
        let (welcome, _) = d[0].secure.welcome_offer(&offer).unwrap();
        new.reload(); // l'offre n'existe qu'en mémoire
        assert_eq!(rejected(&new.receive(welcome)), ["appairage par QR inconnu ou terminé"]);
    }

    #[test]
    fn restored_offer_accepts_the_welcome() {
        let mut d = group(1);
        let mut new = Dev::new();
        let offer = new.secure.offer("windows").unwrap();
        let (welcome, _) = d[0].secure.welcome_offer(&offer).unwrap();
        new.reload();
        assert!(!new.secure.is_joining());
        new.secure.restore_offer(offer.secret.clone());
        assert!(new.secure.is_joining());
        let fx = new.receive(welcome);
        assert!(matches!(fx.last(), Some(Effect::Emit(Event::GroupChanged { epoch: 1 }))));
        assert!(!new.secure.is_joining());
    }

    #[test]
    fn restored_join_accepts_the_welcome() {
        let mut d = group(1);
        let mut new = Dev::new();
        let expires = OffsetDateTime::now_utc() + time::Duration::minutes(10);
        let code = d[0].secure.sponsor([7; 10], expires).unwrap();
        let request = new.secure.start_join(&code, d[0].id.clone(), "nouveau").unwrap();
        let welcome = sends(&d[0].receive(request)).remove(0);
        new.reload();
        assert_eq!(rejected(&new.receive(welcome.clone())), ["appairage inconnu ou expiré"]);
        new.secure.restore_join(&code, d[0].id.clone());
        let fx = new.receive(welcome);
        assert!(matches!(fx.last(), Some(Effect::Emit(Event::GroupChanged { epoch: 1 }))));
    }

    #[test]
    fn init_group_refuses_to_overwrite() {
        let keyring = MemoryKeyring::new();
        let id = DeviceId::generate();
        let identity = Identity::generate();
        let info = init_group(&keyring, &id, "mac", &identity, false).unwrap();
        assert_eq!((info.epoch, info.members.len()), (1, 1));
        assert_eq!(init_group(&keyring, &id, "mac", &identity, false), Err(GroupError::Exists(1)));
        let again = init_group(&keyring, &id, "mac", &identity, true).unwrap();
        assert_ne!(again.key_fingerprint, info.key_fingerprint);
    }

    fn profile(d: &Dev) -> ot_proto::DirectoryDevice {
        ot_proto::DirectoryDevice {
            id: d.id.clone(),
            name: "relais".into(),
            kind: None,
            reachable: true,
            default: false,
            features: vec![ot_proto::Feature::users_v1()],
            online: false,
        }
    }

    fn admin_user(
        admin: &UserId,
        cert: &SignedBlob,
        log: Vec<ot_proto::LogEntry>,
        devs: &[&Dev],
    ) -> DirectoryUser {
        DirectoryUser {
            id: admin.clone(),
            name: "relais".into(),
            role: Role::Admin,
            state: ot_proto::UserState::Active,
            cert: Some(cert.clone()),
            log,
            more: false,
            muted: false,
            devices: devs.iter().map(|d| profile(d)).collect(),
        }
    }

    fn emitted(fx: &[Effect], pred: impl Fn(&Event) -> bool) -> usize {
        fx.iter().filter(|f| matches!(f, Effect::Emit(e) if pred(e))).count()
    }

    #[test]
    fn admin_root_is_kept_and_the_directory_survives_a_restart() {
        let mut devs = group(2);
        let admin = UserId::generate();
        assert!(Dev::new().secure.admin_root(&admin, "Alice", OffsetDateTime::now_utc()).is_err());
        let (cert, entry) =
            devs[0].secure.admin_root(&admin, "Alice", OffsetDateTime::now_utc()).unwrap();
        assert!(!devs[0].secure.has_anchor(), "rien avant la réponse du relais");
        devs[0].secure.adopt_admin_root(&cert);
        assert!(devs[0].secure.admin_root(&admin, "Alice", OffsetDateTime::now_utc()).is_err());

        let log = vec![ot_proto::LogEntry { seq: 1, entry }];
        let users = [admin_user(&admin, &cert, log, &[&devs[0], &devs[1]])];
        let (fx, again) = devs[0].secure.apply_directory(&admin, &admin, &users);
        assert!(!again);
        assert_eq!(emitted(&fx, |e| matches!(e, Event::DirectoryChanged)), 1, "{fx:?}");
        assert_eq!(emitted(&fx, |e| matches!(e, Event::DirectoryRejected { .. })), 0);
        let people = devs[0].secure.people();
        assert_eq!((people.len(), people[0].name.as_str()), (1, "Alice"));
        assert_eq!(people[0].devices.len(), 2);

        devs[0].reload();
        assert!(devs[0].secure.has_anchor());
        assert_eq!(devs[0].secure.directory_known().get(&admin), Some(&1));

        // L'autre appareil du groupe adopte la racine ; un appareil d'un
        // autre groupe, non.
        assert!(devs[1].secure.adopt_existing_root(&admin, &cert).is_ok());
        assert!(devs[1].secure.has_anchor());
        let mut stranger = group(1).remove(0);
        assert!(stranger.secure.adopt_existing_root(&admin, &cert).is_err());
        assert!(stranger.secure.adopt_existing_root(&UserId::generate(), &cert).is_err());
        stranger.reload();
        assert!(!stranger.secure.has_anchor());
    }

    #[test]
    fn a_fork_of_my_person_is_reported_once() {
        let mut devs = group(2);
        let admin = UserId::generate();
        let (cert, entry) =
            devs[0].secure.admin_root(&admin, "Alice", OffsetDateTime::now_utc()).unwrap();
        devs[0].secure.adopt_admin_root(&cert);
        let log = vec![ot_proto::LogEntry { seq: 1, entry }];
        let users = [admin_user(&admin, &cert, log, &[&devs[0]])];
        let (fx, _) = devs[0].secure.apply_directory(&admin, &admin, &users);
        assert_eq!(emitted(&fx, |e| matches!(e, Event::DirectoryFork)), 0);

        // Mon roster local, même version, n'a plus d1 mais un autre appareil.
        let other = Dev::new();
        let roster = Roster {
            epoch: 1,
            version: 1,
            members: vec![devs[0].member("d0"), other.member("autre")],
        };
        devs[0].keyring.save_roster(&roster).unwrap();
        devs[0].reload();
        let users = [admin_user(&admin, &cert, Vec::new(), &[&devs[0]])];
        let (fx, _) = devs[0].secure.apply_directory(&admin, &admin, &users);
        assert_eq!(emitted(&fx, |e| matches!(e, Event::DirectoryFork)), 1);
        let (fx, _) = devs[0].secure.apply_directory(&admin, &admin, &users);
        assert_eq!(emitted(&fx, |e| matches!(e, Event::DirectoryFork)), 0, "une seule alerte");
    }

    // --- Étape 10, T7 : envois entre personnes (`0x05`). ---

    /// Profil d'un appareil sur le relais.
    fn device_profile(
        d: &Dev,
        up_to_date: bool,
        default: bool,
        kind: Option<DeviceKind>,
    ) -> ot_proto::DirectoryDevice {
        ot_proto::DirectoryDevice {
            id: d.id.clone(),
            name: "x".into(),
            kind,
            reachable: true,
            default,
            features: if up_to_date { vec![ot_proto::Feature::users_v1()] } else { vec![] },
            online: true,
        }
    }

    /// Alice (admin, son groupe `a`) et Bob (membre, son groupe `b`),
    /// certificats et journaux signés comme sur un vrai relais.
    struct People {
        alice: UserId,
        bob: UserId,
        a: Vec<Dev>,
        b: Vec<Dev>,
        users: Vec<DirectoryUser>,
    }

    impl People {
        fn new(na: usize, nb: usize) -> Self {
            use ot_crypto::directory::{CertRef, cert_hash, entry_hash};
            let (a, b) = (group(na), group(nb));
            let now = OffsetDateTime::now_utc();
            let alice = UserId::generate();
            let bob = UserId::generate();
            let (root, entry1) = a[0].secure.admin_root(&alice, "Alice", now).unwrap();
            let b0 = &b[0];
            let genesis = Genesis {
                device_id: b0.id.clone(),
                name: "d0".into(),
                sign_key: b0.identity.public_key(),
                exchange_key: b0.secure.exchange.public_key(),
            };
            let a_key = a[0].identity.signing_key();
            let bob_cert = UserCert::new(
                bob.clone(),
                "Bob",
                Role::Member,
                genesis,
                now.unix_timestamp(),
                a[0].id.clone(),
            )
            .sign(a_key)
            .unwrap();
            let mut roster = a[0].secure.roster.clone().unwrap();
            roster.version += 1;
            let entry2 =
                RosterEntry::new(alice.clone(), 2, entry_hash(&entry1), a[0].id.clone(), roster)
                    .with_certs(vec![CertRef { user: bob.clone(), cert: cert_hash(&bob_cert) }])
                    .sign(a_key)
                    .unwrap();
            let bob_roster = b0.secure.roster.clone().unwrap();
            let bob1 = RosterEntry::new(bob.clone(), 1, GENESIS_PREV, b0.id.clone(), bob_roster)
                .sign(b0.identity.signing_key())
                .unwrap();
            let log = |entries: Vec<SignedBlob>| {
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(i, entry)| ot_proto::LogEntry { seq: i as u64 + 1, entry })
                    .collect()
            };
            let mut admin = admin_user(&alice, &root, log(vec![entry1, entry2]), &[]);
            admin.devices = a.iter().map(|d| device_profile(d, true, false, None)).collect();
            let member = DirectoryUser {
                id: bob.clone(),
                name: "Bob".into(),
                role: Role::Member,
                cert: Some(bob_cert),
                log: log(vec![bob1]),
                devices: b.iter().map(|d| device_profile(d, true, false, None)).collect(),
                ..admin.clone()
            };
            let mut people = Self { alice, bob, a, b, users: vec![admin, member] };
            for d in people.a.iter_mut().chain(people.b.iter_mut()) {
                d.secure.adopt_admin_root(&root);
            }
            people.read_all();
            people
        }

        /// Chaque appareil relit l'annuaire ; le relais ne sert que les
        /// entrées après `known`.
        fn read_all(&mut self) {
            let (alice, bob) = (&self.alice, &self.bob);
            let devs =
                self.a.iter_mut().map(|d| (alice, d)).chain(self.b.iter_mut().map(|d| (bob, d)));
            for (me, d) in devs {
                let known = d.secure.directory_known();
                let mut users = self.users.clone();
                for u in &mut users {
                    let after = known.get(&u.id).copied().unwrap_or(0);
                    u.log.retain(|e| e.seq > after);
                }
                let (fx, again) = d.secure.apply_directory(me, alice, &users);
                assert!(!again, "{fx:?}");
                assert_eq!(emitted(&fx, |e| matches!(e, Event::DirectoryRejected { .. })), 0);
            }
        }

        /// Change le profil d'un appareil de Bob (`i`) ou d'Alice.
        fn set_profile(&mut self, user: usize, i: usize, p: ot_proto::DirectoryDevice) {
            self.users[user].devices[i] = p;
            self.read_all();
        }
    }

    fn messages(fx: &[Effect]) -> Vec<Received> {
        fx.iter()
            .filter_map(|f| match f {
                Effect::Emit(Event::Message(m)) => Some(m.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_between_two_people_both_ways_with_a_copy() {
        let mut p = People::new(2, 2);
        let (alice, bob) = (p.alice.clone(), p.bob.clone());
        let env = p.a[0].secure.seal_content_to(&Content::text("salut Bob"), &bob, None).unwrap();
        assert!(matches!(Header::parse(&env.payload), Ok(Header::Multi { .. })));
        let expected: Vec<_> = vec![p.b[0].id.clone(), p.b[1].id.clone(), p.a[1].id.clone()];
        assert_eq!(env.to, expected, "appareils de Bob, puis la copie");

        for d in &mut p.b {
            let got = messages(&d.receive(env.clone()));
            assert_eq!(got.len(), 1);
            assert_eq!(got[0].content.body, "salut Bob");
            let origin = &got[0].origin;
            assert_eq!((&origin.from_user, origin.from_service), (&Some(alice.clone()), false));
            assert_eq!(origin.to_user, None);
        }
        // La copie : de ma personne, adressée à Bob.
        let copy = messages(&p.a[1].receive(env));
        assert_eq!(copy[0].origin, Origin { to_user: Some(bob.clone()), ..Origin::default() });

        // Réponse de Bob, depuis son autre appareil.
        let reply = p.b[1].secure.seal_content_to(&Content::link("https://x.test"), &alice, None);
        let reply = reply.unwrap();
        assert_eq!(reply.to, [p.a[0].id.clone(), p.a[1].id.clone(), p.b[0].id.clone()]);
        let got = messages(&p.a[0].receive(reply));
        assert_eq!(
            (got[0].content.kind, got[0].origin.from_user.as_ref()),
            (ContentKind::Link, Some(&bob))
        );
    }

    #[test]
    fn declaration_adds_the_kind_only_when_the_profile_has_none() {
        let mut p = People::new(1, 1);
        let kind_of = |f: Option<Frame>| match f {
            Some(Frame::DeviceUpdate { kind, features, .. }) => Some((kind, features.is_some())),
            _ => None,
        };
        let me = |p: &People| p.a[0].secure.declaration(Some(DeviceKind::Browser));

        p.set_profile(0, 0, device_profile(&p.a[0], false, true, None));
        assert_eq!(kind_of(me(&p)), Some((Some(DeviceKind::Browser), true)));
        assert_eq!(kind_of(p.a[0].secure.declaration(None)), Some((None, true)));

        p.set_profile(0, 0, device_profile(&p.a[0], true, true, None));
        assert_eq!(kind_of(me(&p)), Some((Some(DeviceKind::Browser), false)));
        assert!(p.a[0].secure.declaration(None).is_none());

        let chosen = device_profile(&p.a[0], true, true, Some(DeviceKind::Desktop));
        p.set_profile(0, 0, chosen);
        assert!(me(&p).is_none(), "une catégorie choisie n'est jamais remplacée");
    }

    #[test]
    fn recipients_follow_defaults_and_skip_devices_not_up_to_date() {
        let mut p = People::new(3, 3);
        let bob = p.bob.clone();
        // b1 par défaut : seul destinataire (plus la copie).
        let d1 = device_profile(&p.b[1], true, true, None);
        p.set_profile(1, 1, d1);
        let to = p.a[0].secure.recipients(&bob, None).unwrap();
        assert_eq!(to[0], p.b[1].id);
        assert!(!to.contains(&p.b[0].id) && !to.contains(&p.b[2].id));

        // b1 pas à jour : les autres appareils joignables et à jour.
        let d1 = device_profile(&p.b[1], false, true, None);
        p.set_profile(1, 1, d1);
        let to = p.a[0].secure.recipients(&bob, None).unwrap();
        assert_eq!(&to[..2], [p.b[0].id.clone(), p.b[2].id.clone()]);
        let err = p.a[0].secure.recipients(&bob, Some(&p.b[1].id)).unwrap_err();
        assert!(matches!(err, ClientError::NoRecipient(_)), "{err}");
        assert_eq!(p.a[0].secure.recipients(&bob, Some(&p.b[2].id)).unwrap()[0], p.b[2].id);

        // Copie : ni la CLI, ni un appareil pas à jour, ni moi.
        let cli = device_profile(&p.a[1], true, false, Some(DeviceKind::Cli));
        p.set_profile(0, 1, cli);
        let old = device_profile(&p.a[2], false, false, None);
        p.set_profile(0, 2, old);
        let to = p.a[0].secure.recipients(&bob, None).unwrap();
        assert!(to.iter().all(|id| p.b.iter().any(|d| &d.id == id)), "aucune copie");

        // Aucun appareil à jour : rien ne part.
        for i in 0..3 {
            let d = device_profile(&p.b[i], false, false, None);
            p.users[1].devices[i] = d;
        }
        p.read_all();
        let err = p.a[0].secure.recipients(&bob, None).unwrap_err();
        assert!(matches!(err, ClientError::NoRecipient(_)), "{err}");
        // Ni à soi-même, ni à une personne inconnue.
        let alice = p.alice.clone();
        assert!(matches!(p.a[0].secure.recipients(&alice, None), Err(ClientError::Invalid(_))));
        let nobody = UserId::generate();
        assert!(matches!(
            p.a[0].secure.recipients(&nobody, None),
            Err(ClientError::NoRecipient(_))
        ));
    }

    #[test]
    fn unknown_sender_waits_for_the_directory() {
        let mut p = People::new(1, 1);
        let alice = p.alice.clone();
        let env = p.b[0].secure.seal_content_to(&Content::text("tôt"), &alice, None).unwrap();
        // Alice redémarre : l'annuaire vérifié (en mémoire) est à relire.
        p.a[0].reload();
        let fx = p.a[0].receive(env);
        assert!(messages(&fx).is_empty() && rejected(&fx).is_empty());
        assert_eq!(p.a[0].keyring.load_quarantine().unwrap().len(), 1);
        let (fx, _) = p.a[0].secure.apply_directory(&alice, &alice, &p.users);
        assert_eq!(messages(&fx)[0].content.body, "tôt", "{fx:?}");
        assert!(p.a[0].keyring.load_quarantine().unwrap().is_empty());
    }

    #[test]
    fn tampered_multi_messages_are_rejected() {
        let mut p = People::new(1, 2);
        let bob = p.bob.clone();
        let env = p.a[0].secure.seal_content_to(&Content::text("x"), &bob, None).unwrap();
        // Le relais réordonne `to` : l'en-tête ne correspond plus.
        let mut swapped = env.clone();
        swapped.to.reverse();
        assert!(messages(&p.b[0].receive(swapped)).is_empty());
        // Il retire Bob 1 de la table : l'AEAD casse pour tous.
        let mut cut = env.clone();
        cut.to.truncate(1);
        assert!(messages(&p.b[0].receive(cut)).is_empty());
        // Un message de Bob signé par un autre que son appareil.
        let forger = Identity::generate();
        let mut forged =
            p.b[0].secure.seal_content_to(&Content::text("y"), &p.alice, None).unwrap();
        let recipients: Vec<_> = forged
            .to
            .iter()
            .map(|id| (id.clone(), p.b[0].secure.exchange_key(id).unwrap()))
            .collect();
        forged.payload = seal(
            &forged,
            SealKey::Multi { recipients: &recipients },
            forger.signing_key(),
            &Body::Text { body: "faux".into() },
        )
        .unwrap();
        assert_eq!(rejected(&p.a[0].receive(forged)), ["signature invalide"]);
    }

    #[test]
    fn files_and_codes_between_people_carry_their_origin() {
        let mut p = People::new(1, 1);
        let (alice, bob) = (p.alice.clone(), p.bob.clone());
        let meta = FileMeta {
            blob_id: "blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V".parse().unwrap(),
            name: "a.pdf".into(),
            mime: "application/pdf".into(),
            size: 3,
            chunk_size: 16,
            chunks: 1,
            key: ot_crypto::FileKey::from_bytes([1; 32]),
            nonce: [2; ot_crypto::FILE_NONCE_LEN],
            sha256: [3; 32],
        };
        let to = p.a[0].secure.recipients(&bob, None).unwrap();
        let env = p.a[0].secure.seal_file_to(&meta, to).unwrap();
        let fx = p.b[0].receive(env);
        assert!(rejected(&fx).is_empty(), "{:?}", rejected(&fx));
        let file = fx
            .iter()
            .find_map(|f| match f {
                Effect::Emit(Event::File(f)) => Some(f.file.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(file.origin.from_user, Some(alice.clone()));
        // Rangée avec son origine : relue après un redémarrage.
        p.b[0].reload();
        assert_eq!(p.b[0].secure.files().unwrap()[0].origin.from_user, Some(alice));
        p.read_all();

        // Un code d'une autre personne n'est jamais rempli d'office (O2).
        let otp = Otp {
            code: "482913".into(),
            display: "G-482913".into(),
            sender_name: "Google".into(),
            sender_domain: "google.com".into(),
            sender_verified: true,
            expires_at: 1_793_628_791,
        };
        let env = p.a[0].secure.seal_otp_to(&otp, std::slice::from_ref(&bob)).unwrap();
        let fx = p.b[0].receive(env);
        assert!(rejected(&fx).is_empty(), "{:?}", rejected(&fx));
        let got = fx
            .iter()
            .find_map(|f| match f {
                Effect::Emit(Event::Otp(o)) => Some(o.clone()),
                _ => None,
            })
            .unwrap();
        assert!(!got.auto_fill());
        let mine = ReceivedOtp { origin: Origin::default(), ..got.clone() };
        assert!(mine.auto_fill());
        let service = ReceivedOtp { origin: Origin { from_service: true, ..got.origin }, ..got };
        assert!(service.auto_fill());
    }

    #[test]
    fn features_are_declared_once_my_profile_is_known() {
        let mut p = People::new(1, 1);
        assert!(!p.a[0].secure.needs_features());
        let old = device_profile(&p.a[0], false, false, None);
        p.set_profile(0, 0, old);
        assert!(p.a[0].secure.needs_features());
        assert!(p.a[0].secure.directory_fresh(OffsetDateTime::now_utc()));
        assert!(!p.a[0].secure.directory_fresh(OffsetDateTime::now_utc() + DIRECTORY_MAX_AGE));
    }

    // --- Étape 10, § 4.2 : mon journal. ---

    #[test]
    fn admin_entries_are_idempotent_and_the_group_adopts_their_version() {
        let mut p = People::new(2, 1);
        let bob = p.bob.clone();
        let cert = p.users[1].cert.clone().unwrap();
        // Déjà engagé, rien d'à jour à publier.
        let head = p.a[0]
            .secure
            .directory_state()
            .admin
            .as_ref()
            .unwrap()
            .1
            .chain
            .as_ref()
            .unwrap()
            .head();
        assert_eq!(
            p.a[0].secure.next_entry(&Publish::Certify(bob.clone(), cert)).unwrap(),
            NextEntry::Done(Some(head))
        );
        assert!(!p.a[0].secure.needs_publish(), "roster local (v1) derrière le journal (v2)");

        // Révocation : entrée 3, roster local porté au-dessus du journal.
        let NextEntry::Entry(own) =
            p.a[0].secure.next_entry(&Publish::Revoke(bob.clone())).unwrap()
        else {
            panic!("entrée attendue")
        };
        assert_eq!((own.entry.seq, own.roster.version), (3, 3));
        assert!(own.certs.is_empty());
        let fx = p.a[0].secure.entry_published(&own);
        assert_eq!(p.a[0].secure.info().unwrap().roster_version, 3);
        // Le roster adopté part à l'autre appareil, qui le prend.
        let broadcast = sends(&fx).remove(0);
        p.a[1].receive(broadcast);
        assert_eq!(p.a[1].secure.info().unwrap().roster_version, 3);
        assert_eq!(
            p.a[0].secure.next_entry(&Publish::Revoke(bob)).unwrap(),
            NextEntry::Done(Some(own.head()))
        );
        // Un membre n'engage pas de certificat.
        let alice = p.alice.clone();
        let blob = p.users[0].cert.clone().unwrap();
        assert!(p.b[0].secure.next_entry(&Publish::Certify(alice, blob)).is_err());
    }

    #[test]
    fn a_new_member_publishes_only_from_the_published_roster() {
        let mut p = People::new(1, 2);
        // Bob ajoute un appareil (roster v2) : b0, membre du roster publié,
        // publie ; le nouveau ne peut pas signer l'entrée.
        let mut roster = p.b[0].secure.roster.clone().unwrap();
        let new = Dev::new();
        roster.members.push(new.member("nouveau"));
        roster.version = 2;
        p.b[0].keyring.save_roster(&roster).unwrap();
        p.b[0].reload();
        p.read_all();
        assert!(p.b[0].secure.needs_publish());
        let NextEntry::Entry(own) = p.b[0].secure.next_entry(&Publish::IfBehind).unwrap() else {
            panic!("entrée attendue")
        };
        assert_eq!((own.entry.seq, own.roster.version, own.roster.members.len()), (2, 2, 3));
        p.b[0].secure.entry_published(&own);
        assert!(!p.b[0].secure.needs_publish());
    }
}
