//! Chiffrement de bout en bout côté client : chaque message sortant est
//! chiffré et signé, chaque message entrant est déchiffré et vérifié contre
//! le roster avant d'être remis à l'appli. Les messages système (`roster`,
//! `group_key`, `pair_*`) sont traités ici.

use std::sync::Arc;

use ot_crypto::{
    Body, CryptoError, EpochKey, ExchangeKeyPair, GroupKey, Header, Member, OpenKey, PairingCode,
    PairingSecret, Roster, SealKey, fingerprint, open, seal,
};
use ot_proto::{Content, ContentKind, DeviceId, DeviceInfo, Encoding, Envelope, Payload};
use time::OffsetDateTime;

use crate::Identity;
use crate::client::{ClientError, Event, Received};
use crate::keyring::{
    GroupKeys, Keyring, KeyringError, MAX_QUARANTINE, Quarantined, RETIRED_KEY_LIFETIME,
};
use crate::roster::{self, RosterError};

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

/// Ce que le client doit faire après le traitement d'un message.
#[derive(Debug)]
pub(crate) enum Effect {
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

pub(crate) struct Secure {
    me: DeviceId,
    identity: Identity,
    exchange: ExchangeKeyPair,
    keys: Option<GroupKeys>,
    roster: Option<Roster>,
    quarantine: Vec<Quarantined>,
    keyring: Arc<dyn Keyring>,
    sponsoring: Vec<Sponsoring>,
    joining: Option<Joining>,
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
        };
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
        let body = match content.kind {
            ContentKind::Text => Body::Text { body: content.body.clone() },
            ContentKind::Link => Body::Link { body: content.body.clone() },
        };
        self.seal_env(to, SealKey::Group { epoch: keys.current, key }, &body).map_err(crypto)
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
                let content = match opened.body()? {
                    Body::Text { body } => Content::text(body),
                    Body::Link { body } => Content::link(body),
                    Body::Roster { roster } => return self.on_roster(env, epoch, roster, fx),
                    other => {
                        return Err(Reject::Reason(format!(
                            "{} inattendu sous la clé de groupe",
                            other.kind()
                        )));
                    }
                };
                fx.push(Effect::Emit(Event::Message(Received {
                    seq,
                    msg_id: env.msg_id,
                    from: env.from.clone(),
                    created_at: env.created_at,
                    content,
                })));
                Ok(Handled::Done)
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
                } else {
                    Err("appairage inconnu ou expiré".into())
                }
            }
            Header::Device { .. } => self.on_group_key(env, now, fx),
        }
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
        let (keys, roster) = self.group().map_err(|e| Reject::Reason(e.to_string()))?;
        let member = Member {
            device_id: env.from.clone(),
            name: name.clone(),
            sign_key,
            exchange_key,
            added_epoch: keys.current,
        };
        let next = roster::with_member(roster, member)?;
        let epoch = keys.current;
        let current = keys.current_key().ok_or("aucune clé courante")?.clone();
        let welcome = Body::PairWelcome {
            keys: keys
                .epochs
                .iter()
                .map(|e| EpochKey { epoch: e.epoch, key: e.key.clone() })
                .collect(),
            roster: next.clone(),
        };
        let sponsoring = self.sponsoring.remove(i);
        let pairing =
            SealKey::Pairing { token_id: sponsoring.token_id, secret: &sponsoring.secret };
        let welcome = self.seal_env(vec![env.from.clone()], pairing, &welcome)?;
        let group = SealKey::Group { epoch, key: &current };
        let broadcast = self.seal_env(vec![], group, &Body::Roster { roster: next.clone() })?;
        self.set_roster(next, fx);
        fx.push(Effect::Send(welcome));
        fx.push(Effect::Send(broadcast));
        fx.push(Effect::Emit(Event::PairingCompleted { device_id: env.from.clone(), name }));
        fx.push(Effect::Emit(Event::GroupChanged { epoch }));
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

    /// Met de côté un message d'époque inconnue (au plus `MAX_QUARANTINE`,
    /// pas plus de 7 jours).
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
            let known = |q: &Quarantined| match Header::parse(&q.env.payload) {
                Ok(Header::Group { epoch }) => {
                    self.keys.as_ref().is_some_and(|k| k.get(epoch).is_some())
                }
                _ => true,
            };
            let (ready, waiting): (Vec<_>, Vec<_>) =
                std::mem::take(&mut self.quarantine).into_iter().partition(known);
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
}
