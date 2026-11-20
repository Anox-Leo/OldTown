//! Annuaire vérifié (étape 10, notes 0052, 0053 et 0056) : ce que le client
//! croit des autres personnes, vérifié jusqu'à l'ancre, jamais sur la parole
//! du relais.
//!
//! - L'**ancre** (empreinte du certificat de l'admin, portée par le code
//!   d'invitation) fixe le certificat racine.
//! - Le **journal de l'admin** part du `genesis` de ce certificat ; chaque
//!   entrée est signée par un membre de la précédente (règles de
//!   `check_replacement`, sans exiger d'y être soi-même). Une tête reçue à
//!   l'arrivée (`admin_head` des bienvenues) l'épingle : un journal plus
//!   court ou différent est refusé.
//! - Le **certificat d'une personne** ne vaut que si son empreinte est celle
//!   de la dernière entrée vérifiée de l'admin (`certs`) et s'il est signé
//!   par un appareil du roster *courant* de l'admin ; son journal part de son
//!   `genesis`.
//! - Le **journal d'un service** est signé par des appareils de l'admin :
//!   chaque entrée par un appareil d'un roster vérifié de l'admin, la
//!   dernière par un appareil de son roster courant (D47).
//! - Les **profils** d'appareils (non signés) ne sont gardés que pour les
//!   appareils du roster vérifié de leur personne.
//!
//! Aucune E/S ici : le client lit l'annuaire page par page
//! ([`DirectoryRead`]), passe chaque tour complet à [`DirectoryState::apply`]
//! et range [`DirectoryState`] dans son `Keyring`.

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use ot_crypto::directory::{
    CertRef, GENESIS_PREV, LogHead, RosterEntry, UserCert, cert_hash, entry_hash, verify_root,
};
use ot_crypto::{Member, Roster};
use ot_proto::{
    DeviceId, DeviceKind, DirectoryUser, FEATURE_USERS_V1, Frame, Known, LogEntry, Role,
    SignedBlob, UserId, UserState, b64,
};
use serde::{Deserialize, Serialize};

use crate::roster::check_keys_unchanged;

/// Refus d'un objet de l'annuaire. Aucun ne porte de clé.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DirectoryError {
    #[error("certificat de l'admin absent ou différent de l'ancre")]
    BadAnchor,
    #[error("certificat refusé : {0}")]
    BadCert(&'static str),
    #[error("entrée {seq} du journal refusée : {reason}")]
    BadEntry { seq: u64, reason: &'static str },
    /// Journal de l'admin plus court que la tête reçue à l'arrivée, ou
    /// différent d'elle : une fourche présentée par le relais.
    #[error("journal de l'admin {0} la tête reçue à l'arrivée")]
    AdminHead(&'static str),
}

/// Journal vérifié d'une personne : dernière entrée et son roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chain {
    pub seq: u64,
    /// `sha256(data)` de la dernière entrée : le `prev` de la suivante.
    #[serde(with = "b64::array")]
    pub head: [u8; 32],
    /// Appareil qui a signé la dernière entrée.
    pub signer: DeviceId,
    pub roster: Roster,
    /// Journal de l'admin : empreintes des certificats engagés par la
    /// dernière entrée (note 0056) ; vide pour les autres personnes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub certs: Vec<CertRef>,
}

impl Chain {
    /// Tête du journal (`seq` et hash de la dernière entrée).
    pub fn head(&self) -> LogHead {
        LogHead { seq: self.seq, hash: self.head }
    }
}

/// Qui signe les entrées d'un journal.
#[derive(Debug, Clone, Copy)]
pub enum Signers<'a> {
    /// Les membres de l'entrée précédente (le `genesis` pour la première).
    Members,
    /// Les appareils de l'admin, d'après tous ses rosters vérifiés
    /// (personne de service).
    Admin(&'a BTreeMap<DeviceId, [u8; 32]>),
}

fn duplicates(roster: &Roster) -> bool {
    roster
        .members
        .iter()
        .enumerate()
        .any(|(i, m)| roster.members[..i].iter().any(|o| o.device_id == m.device_id))
}

/// Vérifie la suite d'un journal à partir de `from` (ou de rien) : renvoie
/// le plus long préfixe valide, et la raison qui l'arrête s'il y en a une.
/// Une entrée invalide arrête tout : les suivantes en dépendent.
pub fn verify_log(
    cert: &UserCert,
    from: Option<&Chain>,
    entries: &[LogEntry],
    signers: Signers<'_>,
) -> (Option<Chain>, Option<DirectoryError>) {
    let service = cert.role == Role::Service;
    let mut chain = from.cloned();
    for e in entries {
        let expected = chain.as_ref().map_or(1, |c| c.seq + 1);
        let bad = |reason| DirectoryError::BadEntry { seq: e.seq, reason };
        if e.seq != expected {
            return (chain, Some(bad("hors séquence (tronqué ou rejoué)")));
        }
        let Ok(unverified) = RosterEntry::read_unverified(&e.entry) else {
            return (chain, Some(bad("illisible")));
        };
        if unverified.seq != e.seq || unverified.user != cert.user {
            return (chain, Some(bad("seq ou personne différents")));
        }
        let prev = chain.as_ref().map_or(GENESIS_PREV, |c| c.head);
        if unverified.prev != prev {
            return (chain, Some(bad("prev ne suit pas l'entrée précédente")));
        }
        let key = match (signers, &chain) {
            (Signers::Admin(history), _) => history.get(&unverified.signer).copied(),
            (Signers::Members, None) => {
                (unverified.signer == cert.genesis.device_id).then_some(cert.genesis.sign_key)
            }
            (Signers::Members, Some(c)) => c.roster.member(&unverified.signer).map(|m| m.sign_key),
        };
        let Some(key) = key else {
            return (chain, Some(bad("signataire non autorisé")));
        };
        let Ok(entry) = RosterEntry::open(&e.entry, &key) else {
            return (chain, Some(bad("signature invalide")));
        };
        let roster = &entry.roster;
        if duplicates(roster) {
            return (chain, Some(bad("appareil en double")));
        }
        if service != (roster.epoch == 0) {
            return (chain, Some(bad("époque 0 réservée aux services")));
        }
        if cert.role != Role::Admin && !entry.certs.is_empty() {
            return (chain, Some(bad("certs hors du journal de l'admin")));
        }
        match &chain {
            None => {
                let g = &cert.genesis;
                let ok = roster
                    .member(&g.device_id)
                    .is_some_and(|m| m.sign_key == g.sign_key && m.exchange_key == g.exchange_key);
                if !ok {
                    return (chain, Some(bad("premier roster sans le genesis du certificat")));
                }
            }
            Some(c) => {
                if (roster.epoch, roster.version) <= (c.roster.epoch, c.roster.version) {
                    return (chain, Some(bad("roster pas plus récent")));
                }
                if check_keys_unchanged(&c.roster, roster).is_err() {
                    return (chain, Some(bad("clés d'un membre changées")));
                }
            }
        }
        chain = Some(Chain {
            seq: e.seq,
            head: entry_hash(&e.entry),
            signer: entry.signer,
            roster: entry.roster,
            certs: entry.certs,
        });
    }
    (chain, None)
}

/// Ce que le client garde d'une personne entre deux lectures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownPerson {
    pub cert: SignedBlob,
    #[serde(default)]
    pub chain: Option<Chain>,
}

/// État vérifié, rangé par le `Keyring` (`directory.json`). Sans ancre
/// (installation à une personne), il reste vide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryState {
    #[serde(default, with = "anchor_b64")]
    pub anchor: Option<[u8; 16]>,
    #[serde(default)]
    pub admin: Option<(UserId, KnownPerson)>,
    #[serde(default)]
    pub people: BTreeMap<UserId, KnownPerson>,
    /// Tout appareil vu dans un roster vérifié de l'admin, avec sa clé.
    #[serde(default)]
    pub admin_history: BTreeMap<DeviceId, [u8; 32]>,
    /// Tête du journal de l'admin reçue à l'arrivée (`admin_head` de
    /// `invite_welcome` ou `service_welcome`), tant que le journal vérifié
    /// ne l'a pas atteinte ; ensuite le chaînage par `prev` la garde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_head: Option<LogHead>,
    /// Clé du `genesis` des personnes que le relais ne montre plus (lien
    /// retiré, révocation, réinvitation en cours) : à leur retour avec
    /// d'autres clés, `KeysChanged` (une personne réinvitée passe par un
    /// état où le relais la cache).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", with = "keys_b64")]
    pub forgotten: BTreeMap<UserId, [u8; 32]>,
}

mod keys_b64 {
    use std::collections::BTreeMap;

    use ot_proto::{UserId, b64};
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

    pub fn serialize<S: Serializer>(
        v: &BTreeMap<UserId, [u8; 32]>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        v.iter()
            .map(|(k, key)| (k.clone(), b64::encode(key)))
            .collect::<BTreeMap<_, _>>()
            .serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<UserId, [u8; 32]>, D::Error> {
        let raw = BTreeMap::<UserId, String>::deserialize(d)?;
        raw.into_iter()
            .map(|(k, v)| {
                let bytes = b64::decode(&v).map_err(de::Error::custom)?;
                let key: [u8; 32] =
                    bytes.try_into().map_err(|_| de::Error::custom("clé de 32 octets attendue"))?;
                Ok((k, key))
            })
            .collect()
    }
}

mod anchor_b64 {
    use ot_proto::b64;
    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(v: &Option<[u8; 16]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(a) => s.serialize_some(&b64::encode(a)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 16]>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|s| b64::decode_array(&s).ok_or_else(|| de::Error::custom("ancre : 16 octets")))
            .transpose()
    }
}

/// Résultat d'un tour de lecture de l'annuaire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Personnes vérifiées (moi compris). `None` : le journal de l'admin
    /// n'est pas encore lu jusqu'au bout, les autres personnes n'ont pas été
    /// jugées ; garder la vue précédente.
    pub people: Option<Vec<Person>>,
    pub events: Vec<DirectoryEvent>,
    /// Un autre tour est à faire, avec [`DirectoryState::known`] à jour : un
    /// journal coupé (`more`) ou à relire depuis le début (`Refetch`).
    pub again: bool,
}

/// Événements d'une lecture de l'annuaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryEvent {
    /// Quelque chose de vérifié a changé.
    Changed,
    /// Un objet a été refusé ; la personne reste à sa dernière entrée valide.
    Rejected { user: UserId, reason: DirectoryError },
    /// La personne a de nouvelles clés (réinvitation) : à dire, comme Signal.
    KeysChanged { user: UserId },
    /// Journal de cette personne à relire depuis le début (`known` sans elle).
    Refetch { user: UserId },
}

/// Profil d'un appareil d'une personne vérifiée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfile {
    pub id: DeviceId,
    pub name: String,
    pub kind: Option<DeviceKind>,
    pub reachable: bool,
    pub default: bool,
    /// Annonce `users-v1` : sait ouvrir `0x05`.
    pub up_to_date: bool,
    pub online: bool,
    /// Clé X25519, lue dans le roster vérifié (jamais dans le profil).
    pub exchange_key: [u8; 32],
    pub sign_key: [u8; 32],
}

/// Une personne vérifiée, telle que les applis la montrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: UserId,
    /// Nom du certificat (signé par l'admin), pas celui du relais.
    pub name: String,
    pub role: Role,
    pub roster: Roster,
    /// Profils du relais, seulement pour les appareils du roster vérifié.
    pub devices: Vec<DeviceProfile>,
    pub muted: bool,
}

impl DirectoryState {
    /// État d'un client qui connaît l'ancre (code d'invitation, ou admin
    /// qui vient de publier son certificat).
    pub fn with_anchor(anchor: [u8; 16]) -> Self {
        Self { anchor: Some(anchor), ..Self::default() }
    }

    /// État d'un nouvel appareil (invité, service) : l'ancre du code et la
    /// tête du journal de l'admin qui engage son certificat.
    pub fn with_admin_head(anchor: [u8; 16], head: LogHead) -> Self {
        Self { anchor: Some(anchor), admin_head: Some(head), ..Self::default() }
    }

    /// Dernier `seq` vérifié par personne, à envoyer dans `directory_get`.
    pub fn known(&self) -> ot_proto::Known {
        let mut known = ot_proto::Known::new();
        let all = self.admin.iter().map(|(id, p)| (id, p)).chain(self.people.iter());
        for (id, p) in all {
            if let Some(c) = &p.chain {
                known.insert(id.clone(), c.seq);
            }
        }
        known
    }

    /// Identifiant de l'admin, une fois sa racine vérifiée.
    pub fn admin_id(&self) -> Option<&UserId> {
        self.admin.as_ref().map(|(id, _)| id)
    }

    /// Certificat (vérifié) et journal de `user`, l'admin compris.
    pub fn known_person(&self, user: &UserId) -> Option<&KnownPerson> {
        match &self.admin {
            Some((id, p)) if id == user => Some(p),
            _ => self.people.get(user),
        }
    }

    /// Ajoute à son journal une entrée que cet appareil vient de signer et
    /// que le relais a acceptée : elle suit la dernière entrée vérifiée.
    pub fn extend_own(&mut self, user: &UserId, chain: Chain) {
        let admin = self.admin.as_ref().is_some_and(|(id, _)| id == user);
        if admin {
            for m in &chain.roster.members {
                self.admin_history.insert(m.device_id.clone(), m.sign_key);
            }
        }
        let person =
            if admin { self.admin.as_mut().map(|(_, p)| p) } else { self.people.get_mut(user) };
        if let Some(p) = person {
            p.chain = Some(chain);
        }
    }

    /// Roster courant vérifié de l'admin.
    pub fn admin_roster(&self) -> Option<&Roster> {
        self.admin.as_ref().and_then(|(_, p)| p.chain.as_ref()).map(|c| &c.roster)
    }

    /// Applique un tour complet de lecture (toutes les pages, `next`
    /// suivi) : vérifie ce qui est nouveau, garde le reste. Sans ancre, rien
    /// n'est cru.
    pub fn apply(&mut self, admin_id: &UserId, users: &[DirectoryUser]) -> Applied {
        let mut events = Vec::new();
        let mut again = false;
        let Some(anchor) = self.anchor else { return Applied::default() };
        let reject = |events: &mut Vec<_>, user: &UserId, reason| {
            events.push(DirectoryEvent::Rejected { user: user.clone(), reason });
        };

        // 1. La racine, contre l'ancre ; sans elle, rien d'autre ne vaut.
        let Some(admin_raw) = users.iter().find(|u| &u.id == admin_id) else {
            return Applied { people: None, events, again: false };
        };
        let root = match admin_raw.cert.as_ref().map(|c| (c, verify_root(c, &anchor))) {
            Some((blob, Ok(cert))) if &cert.user == admin_id => (blob.clone(), cert),
            _ => {
                reject(&mut events, admin_id, DirectoryError::BadAnchor);
                return Applied { people: None, events, again: false };
            }
        };
        if self.admin.as_ref().is_some_and(|(id, _)| id != admin_id) {
            // L'ancre est la même, l'identifiant non : impossible sans bug.
            reject(&mut events, admin_id, DirectoryError::BadAnchor);
            return Applied { people: None, events, again: false };
        }

        // 2. Le journal de l'admin, et l'historique de ses appareils.
        let known_chain = self.admin.as_ref().and_then(|(_, p)| p.chain.clone());
        let (chain, error) =
            verify_log(&root.1, known_chain.as_ref(), &admin_raw.log, Signers::Members);
        if let Some(reason) = error {
            reject(&mut events, admin_id, reason);
        }
        // 2 bis. La tête épinglée : l'entrée `seq` doit avoir ce hash.
        if let Some(pin) = self.admin_head {
            let at_pin = admin_raw.log.iter().find(|e| e.seq == pin.seq);
            match &chain {
                Some(c) if c.seq >= pin.seq => {
                    // Atteinte pendant ce tour (sinon la tête serait déjà
                    // effacée) : l'entrée est dans ce journal, vérifiée.
                    if at_pin.is_none_or(|e| entry_hash(&e.entry) != pin.hash) {
                        reject(&mut events, admin_id, DirectoryError::AdminHead("diffère de"));
                        return Applied { people: None, events, again: false };
                    }
                    self.admin_head = None;
                }
                _ if admin_raw.more => {}
                _ => {
                    reject(&mut events, admin_id, DirectoryError::AdminHead("plus court que"));
                    return Applied { people: None, events, again: false };
                }
            }
        }
        if chain != known_chain {
            events.push(DirectoryEvent::Changed);
        }
        self.record_admin_history(&root.1, known_chain.as_ref(), &admin_raw.log, chain.as_ref());
        self.admin = Some((admin_id.clone(), KnownPerson { cert: root.0, chain }));
        if admin_raw.more || self.admin_head.is_some() {
            // Les certificats se jugent contre la dernière entrée de l'admin :
            // pas avant de l'avoir lue.
            events.dedup();
            return Applied { people: None, events, again: true };
        }
        let admin_chain = self.admin.as_ref().and_then(|(_, p)| p.chain.clone());
        let admin_roster = admin_chain.as_ref().map(|c| &c.roster);

        // 3. Les autres personnes.
        let mut people = Vec::new();
        if let Some(p) = person(admin_raw, &root.1, admin_roster) {
            people.push(p);
        }
        for u in users.iter().filter(|u| &u.id != admin_id) {
            let Some(blob) = u.cert.as_ref().filter(|_| u.state == UserState::Active) else {
                continue;
            };
            let cert = match check_cert(blob, &u.id, admin_chain.as_ref()) {
                Ok(cert) => cert,
                Err(reason) => {
                    reject(&mut events, &u.id, reason);
                    continue;
                }
            };
            let stored = self.people.get(&u.id);
            let old = stored.and_then(|p| UserCert::read_unverified(&p.cert).ok());
            // Contrôle secondaire (l'empreinte dans `certs` suffit déjà) : un
            // certificat plus ancien que celui déjà vu est celui d'avant une
            // réinvitation, rejoué par le relais.
            if old.as_ref().is_some_and(|o| cert.issued_at < o.issued_at) {
                reject(
                    &mut events,
                    &u.id,
                    DirectoryError::BadCert("plus ancien que le certificat connu"),
                );
                continue;
            }
            let same_genesis = old.is_some_and(|o| o.genesis == cert.genesis);
            if stored.is_none()
                && self.forgotten.get(&u.id).is_some_and(|k| *k != cert.genesis.sign_key)
            {
                events.push(DirectoryEvent::KeysChanged { user: u.id.clone() });
            }
            self.forgotten.remove(&u.id);
            let from = if same_genesis { stored.and_then(|p| p.chain.clone()) } else { None };
            if stored.is_some() && !same_genesis {
                events.push(DirectoryEvent::KeysChanged { user: u.id.clone() });
                if u.log.first().is_some_and(|e| e.seq != 1) {
                    // Le relais a filtré avec l'ancien `known` : tout relire.
                    self.people
                        .insert(u.id.clone(), KnownPerson { cert: blob.clone(), chain: None });
                    events.push(DirectoryEvent::Refetch { user: u.id.clone() });
                    again = true;
                    continue;
                }
            }
            let signers = if cert.role == Role::Service {
                Signers::Admin(&self.admin_history)
            } else {
                Signers::Members
            };
            let (chain, error) = verify_log(&cert, from.as_ref(), &u.log, signers);
            if let Some(reason) = error {
                reject(&mut events, &u.id, reason);
            } else if u.more {
                again = true;
            }
            if chain != from {
                events.push(DirectoryEvent::Changed);
            }
            self.people
                .insert(u.id.clone(), KnownPerson { cert: blob.clone(), chain: chain.clone() });
            // Service : la tête doit être signée par un appareil d'admin actuel
            // (D47) ; sinon il attend que l'admin republie.
            if cert.role == Role::Service {
                let current = chain
                    .as_ref()
                    .zip(admin_roster)
                    .is_some_and(|(c, r)| r.member(&c.signer).is_some());
                if !current {
                    if chain.is_some() {
                        reject(
                            &mut events,
                            &u.id,
                            DirectoryError::BadEntry {
                                seq: chain.as_ref().map_or(0, |c| c.seq),
                                reason: "dernière entrée signée par un appareil retiré de l'admin",
                            },
                        );
                    }
                    continue;
                }
            }
            if let Some(p) = person(u, &cert, chain.as_ref().map(|c| &c.roster)) {
                people.push(p);
            }
        }
        // Les personnes que le relais ne montre plus (lien retiré,
        // révocation) sont oubliées.
        let shown: Vec<&UserId> = users.iter().map(|u| &u.id).collect();
        let before = self.people.len();
        let gone: Vec<UserId> =
            self.people.keys().filter(|id| !shown.contains(id)).cloned().collect();
        for id in gone {
            if let Some(p) = self.people.remove(&id)
                && let Ok(cert) = UserCert::read_unverified(&p.cert)
            {
                self.forgotten.insert(id, cert.genesis.sign_key);
            }
        }
        if self.people.len() != before {
            events.push(DirectoryEvent::Changed);
        }
        events.dedup();
        Applied { people: Some(people), events, again }
    }

    /// Remplacée dans le client par `Secure::next_entry` (§ 4.2, note 0060),
    /// qui publie aussi mon roster local ; gardée pour ses tests de règles.
    ///
    /// Entrée suivante du journal de l'admin (note 0056) : même roster,
    /// `version + 1`, `certs` mis à jour par `change`, signée par l'appareil
    /// courant, membre du roster vérifié de l'admin. À publier
    /// (`roster_publish`) après chaque `user_certify`, `service_create` et
    /// `user_revoke`. Renvoie l'entrée et le roster qu'elle porte : l'appareil
    /// l'adopte et l'envoie à son groupe, pour que la prochaine version locale
    /// reste au-dessus de celle du journal.
    pub fn next_admin_entry(
        &self,
        signer: &DeviceId,
        key: &SigningKey,
        change: CertChange<'_>,
    ) -> Result<(LogEntry, Roster), DirectoryError> {
        let refused = |reason| DirectoryError::BadEntry { seq: 0, reason };
        let (admin, chain) = self
            .admin
            .as_ref()
            .and_then(|(id, p)| Some((id, p.chain.as_ref()?)))
            .ok_or(refused("journal de l'admin pas encore vérifié"))?;
        if self.admin_head.is_some() {
            return Err(refused("journal de l'admin pas encore lu jusqu'à la tête reçue"));
        }
        if chain.roster.member(signer).is_none_or(|m| m.sign_key != key.verifying_key().to_bytes())
        {
            return Err(refused("appareil hors du roster de l'admin"));
        }
        let mut certs = chain.certs.clone();
        match change {
            CertChange::Certify(user, blob) => {
                if user == admin {
                    return Err(refused("le certificat de l'admin n'est pas dans certs"));
                }
                certs.retain(|c| &c.user != user);
                certs.push(CertRef { user: user.clone(), cert: cert_hash(blob) });
            }
            CertChange::Revoke(user) => certs.retain(|c| &c.user != user),
            CertChange::None => {}
        }
        let roster = Roster { version: chain.roster.version + 1, ..chain.roster.clone() };
        let seq = chain.seq + 1;
        let entry =
            RosterEntry::new(admin.clone(), seq, chain.head, signer.clone(), roster.clone())
                .with_certs(certs)
                .sign(key)
                .map_err(|_| refused("entrée invalide"))?;
        Ok((LogEntry { seq, entry }, roster))
    }

    /// Ajoute à l'historique les appareils de chaque roster de l'admin
    /// vérifié pendant cette lecture.
    fn record_admin_history(
        &mut self,
        cert: &UserCert,
        from: Option<&Chain>,
        log: &[LogEntry],
        to: Option<&Chain>,
    ) {
        let Some(to) = to else { return };
        let mut add = |m: &Member| {
            self.admin_history.insert(m.device_id.clone(), m.sign_key);
        };
        let first = from.map_or(1, |c| c.seq + 1);
        for e in log.iter().filter(|e| e.seq >= first && e.seq <= to.seq) {
            // Invariant : ces entrées viennent d'être vérifiées par verify_log.
            if let Ok(entry) = RosterEntry::read_unverified(&e.entry) {
                entry.roster.members.iter().for_each(&mut add);
            }
        }
        if let Some(m) = to.roster.member(&cert.genesis.device_id) {
            add(m);
        }
    }

    /// Fourche de ma personne : le journal publié par le relais porte, à
    /// `(epoch, version)` égal, d'autres membres que mon roster local.
    pub fn is_fork(&self, me: &UserId, local: &Roster) -> bool {
        let chain = if self.admin.as_ref().is_some_and(|(id, _)| id == me) {
            self.admin.as_ref().and_then(|(_, p)| p.chain.as_ref())
        } else {
            self.people.get(me).and_then(|p| p.chain.as_ref())
        };
        let Some(c) = chain else { return false };
        fn ids(r: &Roster) -> Vec<(&DeviceId, [u8; 32])> {
            let mut v: Vec<_> = r.members.iter().map(|m| (&m.device_id, m.sign_key)).collect();
            v.sort();
            v
        }
        (c.roster.epoch, c.roster.version) == (local.epoch, local.version)
            && ids(&c.roster) != ids(local)
    }
}

/// Tours de lecture au plus par lecture de l'annuaire (journaux coupés,
/// relectures) : au-delà, on garde ce qui est vérifié et on attend le
/// prochain `directory_changed`.
pub const MAX_DIRECTORY_ROUNDS: u32 = 8;
/// Pages au plus par lecture : un relais qui renvoie `next` sans fin ne fait
/// pas tourner le client.
pub const MAX_DIRECTORY_PAGES: u32 = 1024;

/// Lecture de l'annuaire en cours (note 0056) : les pages d'un tour, en
/// suivant `next`, puis d'autres tours, `known` à jour, tant qu'un journal
/// est coupé (`more`). Le `known` d'un tour est figé à son début : toutes
/// ses pages filtrent les journaux de la même façon.
#[derive(Debug, Clone)]
pub struct DirectoryRead {
    known: Known,
    first: Option<(UserId, UserId)>,
    users: Vec<DirectoryUser>,
    rounds: u32,
    pages: u32,
}

/// Suite d'une page lue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadStep {
    /// Trame `directory_get` à envoyer pour la page suivante.
    Request(Box<Frame>),
    /// Tour complet, à passer à [`DirectoryState::apply`].
    Round { me: UserId, admin: UserId, users: Vec<DirectoryUser> },
}

impl DirectoryRead {
    /// Commence une lecture ; renvoie la première demande.
    pub fn start(known: Known) -> (Self, Frame) {
        let read = Self { known, first: None, users: Vec::new(), rounds: 1, pages: 0 };
        let frame = read.request(None);
        (read, frame)
    }

    fn request(&self, after: Option<UserId>) -> Frame {
        Frame::DirectoryGet { known: self.known.clone(), after }
    }

    /// Range une page. Erreur : relais incohérent (autre `me` ou autre
    /// `admin` d'une page à l'autre, pages sans fin) ; la lecture s'arrête.
    pub fn page(
        &mut self,
        me: UserId,
        admin: UserId,
        users: Vec<DirectoryUser>,
        next: Option<UserId>,
    ) -> Result<ReadStep, &'static str> {
        self.pages += 1;
        if self.pages > MAX_DIRECTORY_PAGES {
            return Err("trop de pages");
        }
        match &self.first {
            Some((m, a)) if (m, a) != (&me, &admin) => return Err("me ou admin changés"),
            Some(_) => {}
            None => self.first = Some((me.clone(), admin.clone())),
        }
        // Une personne déjà lue dans ce tour : le relais est reparti du début
        // (personne disparue entre deux pages). On garde la dernière version.
        for u in users {
            self.users.retain(|o| o.id != u.id);
            self.users.push(u);
        }
        match next {
            Some(after) => Ok(ReadStep::Request(Box::new(self.request(Some(after))))),
            None => Ok(ReadStep::Round { me, admin, users: std::mem::take(&mut self.users) }),
        }
    }

    /// Nouveau tour avec `known` à jour ; `None` au-delà de
    /// [`MAX_DIRECTORY_ROUNDS`].
    pub fn next_round(&mut self, known: Known) -> Option<Frame> {
        if self.rounds >= MAX_DIRECTORY_ROUNDS {
            return None;
        }
        self.rounds += 1;
        self.known = known;
        self.users.clear();
        Some(self.request(None))
    }
}

/// Changement des certificats engagés par le journal de l'admin.
#[derive(Debug, Clone, Copy)]
pub enum CertChange<'a> {
    /// Certificat neuf ou re-signé de cette personne.
    Certify(&'a UserId, &'a SignedBlob),
    /// Personne révoquée : son empreinte sort de `certs`.
    Revoke(&'a UserId),
    /// Republication sans changement (service après une rotation, D47).
    None,
}

/// Certificat d'une personne autre que l'admin : son empreinte est celle de
/// la dernière entrée vérifiée de l'admin, il est signé par un appareil du
/// roster courant de l'admin, pour cette personne, jamais `admin`.
fn check_cert(
    blob: &SignedBlob,
    user: &UserId,
    admin_chain: Option<&Chain>,
) -> Result<UserCert, DirectoryError> {
    let unverified =
        UserCert::read_unverified(blob).map_err(|_| DirectoryError::BadCert("illisible"))?;
    if &unverified.user != user {
        return Err(DirectoryError::BadCert("pour une autre personne"));
    }
    if unverified.role == Role::Admin {
        return Err(DirectoryError::BadCert("un seul admin"));
    }
    let committed = admin_chain.and_then(|c| c.certs.iter().find(|r| &r.user == user));
    if committed.is_none_or(|r| r.cert != cert_hash(blob)) {
        return Err(DirectoryError::BadCert(
            "empreinte absente de la dernière entrée de l'admin (remplacé ou révoqué)",
        ));
    }
    let issuer = admin_chain
        .map(|c| &c.roster)
        .and_then(|r| r.member(&unverified.issuer))
        .ok_or(DirectoryError::BadCert("signataire hors du roster courant de l'admin"))?;
    UserCert::open(blob, &issuer.sign_key)
        .map_err(|_| DirectoryError::BadCert("signature invalide"))
}

/// Vue d'une personne : profils gardés pour les seuls appareils de son
/// roster vérifié, clés lues dans le roster.
fn person(raw: &DirectoryUser, cert: &UserCert, roster: Option<&Roster>) -> Option<Person> {
    let roster = roster?;
    let devices = raw
        .devices
        .iter()
        .filter_map(|d| {
            let m = roster.member(&d.id)?;
            Some(DeviceProfile {
                id: d.id.clone(),
                name: m.name.clone(),
                kind: d.kind,
                reachable: d.reachable,
                default: d.default,
                up_to_date: d.features.iter().any(|f| f.as_str() == FEATURE_USERS_V1),
                online: d.online,
                exchange_key: m.exchange_key,
                sign_key: m.sign_key,
            })
        })
        .collect();
    Some(Person {
        id: raw.id.clone(),
        name: cert.name.clone(),
        role: cert.role,
        roster: roster.clone(),
        devices,
        muted: raw.muted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_crypto::directory::{Genesis, anchor, log_head};
    use ot_crypto::{ExchangeKeyPair, SigningKey};
    use ot_proto::{DirectoryDevice, Feature};

    /// Un appareil de test : identité et membre de roster.
    struct Dev {
        id: DeviceId,
        sign: SigningKey,
        member: Member,
    }

    fn dev(name: &str, seed: u8) -> Dev {
        let sign = SigningKey::from_bytes(&[seed; 32]);
        let id = DeviceId::generate();
        let member = Member {
            device_id: id.clone(),
            name: name.into(),
            sign_key: sign.verifying_key().to_bytes(),
            exchange_key: ExchangeKeyPair::from_bytes([seed.wrapping_add(100); 32]).public_key(),
            added_epoch: 1,
        };
        Dev { id, sign, member }
    }

    fn genesis(d: &Dev) -> Genesis {
        Genesis {
            device_id: d.id.clone(),
            name: d.member.name.clone(),
            sign_key: d.member.sign_key,
            exchange_key: d.member.exchange_key,
        }
    }

    /// Journal en construction : entrées signées et chaînées.
    struct Log {
        user: UserId,
        entries: Vec<LogEntry>,
        head: [u8; 32],
    }

    impl Log {
        fn new(user: &UserId) -> Self {
            Self { user: user.clone(), entries: Vec::new(), head: GENESIS_PREV }
        }

        fn copy(&self) -> Self {
            Self { user: self.user.clone(), entries: self.entries.clone(), head: self.head }
        }

        fn push(&mut self, by: &Dev, epoch: u32, version: u64, members: &[&Dev]) {
            self.push_certs(by, epoch, version, members, &[]);
        }

        /// Entrée qui engage `certs` (journal de l'admin, note 0056).
        fn push_certs(
            &mut self,
            by: &Dev,
            epoch: u32,
            version: u64,
            members: &[&Dev],
            certs: &[(&UserId, &SignedBlob)],
        ) {
            let roster = Roster {
                epoch,
                version,
                members: members.iter().map(|d| d.member.clone()).collect(),
            };
            let seq = self.entries.len() as u64 + 1;
            let certs = certs
                .iter()
                .map(|(user, cert)| CertRef { user: (*user).clone(), cert: cert_hash(cert) })
                .collect();
            let blob = RosterEntry::new(self.user.clone(), seq, self.head, by.id.clone(), roster)
                .with_certs(certs)
                .sign(&by.sign)
                .unwrap();
            self.head = entry_hash(&blob);
            self.entries.push(LogEntry { seq, entry: blob });
        }
    }

    fn profile(d: &Dev) -> DirectoryDevice {
        DirectoryDevice {
            id: d.id.clone(),
            name: d.member.name.clone(),
            kind: None,
            reachable: true,
            default: true,
            features: vec![Feature::users_v1()],
            online: true,
        }
    }

    fn entry(
        id: &UserId,
        role: Role,
        cert: &SignedBlob,
        log: &Log,
        devices: &[&Dev],
    ) -> DirectoryUser {
        DirectoryUser {
            id: id.clone(),
            name: "relais".into(),
            role,
            state: UserState::Active,
            cert: Some(cert.clone()),
            log: log.entries.clone(),
            more: false,
            muted: false,
            devices: devices.iter().map(|d| profile(d)).collect(),
        }
    }

    /// Alice (admin, deux appareils), Bob (membre, deux appareils), Notifier
    /// (service, signé par Alice).
    struct World {
        alice: UserId,
        a1: Dev,
        a2: Dev,
        root: SignedBlob,
        alice_log: Log,
        bob: UserId,
        b1: Dev,
        b2: Dev,
        bob_cert: SignedBlob,
        bob_log: Log,
        notifier: UserId,
        n1: Dev,
        notifier_cert: SignedBlob,
        notifier_log: Log,
    }

    fn world() -> World {
        let (a1, a2, b1, b2, n1) = (
            dev("alice-phone", 1),
            dev("alice-laptop", 2),
            dev("bob-phone", 3),
            dev("bob-laptop", 4),
            dev("notifier-1", 5),
        );
        let alice = UserId::generate();
        let root =
            UserCert::new(alice.clone(), "Alice", Role::Admin, genesis(&a1), 1, a1.id.clone())
                .sign(&a1.sign)
                .unwrap();
        let mut alice_log = Log::new(&alice);
        alice_log.push(&a1, 1, 1, &[&a1]);
        alice_log.push(&a1, 1, 2, &[&a1, &a2]);
        let bob = UserId::generate();
        let bob_cert =
            UserCert::new(bob.clone(), "Bob", Role::Member, genesis(&b1), 2, a1.id.clone())
                .sign(&a1.sign)
                .unwrap();
        let mut bob_log = Log::new(&bob);
        bob_log.push(&b1, 1, 1, &[&b1]);
        bob_log.push(&b1, 1, 2, &[&b1, &b2]);
        let notifier = UserId::generate();
        let notifier_cert = UserCert::new(
            notifier.clone(),
            "Notifier",
            Role::Service,
            genesis(&n1),
            3,
            a2.id.clone(),
        )
        .sign(&a2.sign)
        .unwrap();
        let mut notifier_log = Log::new(&notifier);
        notifier_log.push(&a2, 0, 1, &[&n1]);
        alice_log.push_certs(
            &a1,
            1,
            3,
            &[&a1, &a2],
            &[(&bob, &bob_cert), (&notifier, &notifier_cert)],
        );
        World {
            alice,
            a1,
            a2,
            root,
            alice_log,
            bob,
            b1,
            b2,
            bob_cert,
            bob_log,
            notifier,
            n1,
            notifier_cert,
            notifier_log,
        }
    }

    impl World {
        fn users(&self) -> Vec<DirectoryUser> {
            vec![
                entry(&self.alice, Role::Admin, &self.root, &self.alice_log, &[&self.a1, &self.a2]),
                entry(
                    &self.bob,
                    Role::Member,
                    &self.bob_cert,
                    &self.bob_log,
                    &[&self.b1, &self.b2],
                ),
                entry(&self.notifier, Role::Service, &self.notifier_cert, &self.notifier_log, &[]),
            ]
        }

        fn state(&self) -> DirectoryState {
            DirectoryState::with_anchor(anchor(&self.root))
        }

        fn alice_log(&self) -> Log {
            self.alice_log.copy()
        }

        /// Entrée de l'admin qui engage ces certificats (même roster `[a1, a2]`
        /// sauf `members`).
        fn commit(
            &self,
            log: &mut Log,
            by: &Dev,
            epoch: u32,
            members: &[&Dev],
            certs: &[(&UserId, &SignedBlob)],
        ) {
            let version = log.entries.len() as u64 + 1;
            log.push_certs(by, epoch, version, members, certs);
        }
    }

    fn apply(
        state: &mut DirectoryState,
        admin: &UserId,
        users: &[DirectoryUser],
    ) -> (Vec<Person>, Vec<DirectoryEvent>) {
        let out = state.apply(admin, users);
        (out.people.unwrap_or_default(), out.events)
    }

    /// Entrées d'un journal après `known`, comme le relais les sert.
    fn since(log: &Log, known: u64) -> Vec<LogEntry> {
        log.entries.iter().filter(|e| e.seq > known).cloned().collect()
    }

    fn ids(people: &[Person]) -> Vec<UserId> {
        people.iter().map(|p| p.id.clone()).collect()
    }

    fn rejected(events: &[DirectoryEvent], user: &UserId) -> bool {
        events.iter().any(|e| matches!(e, DirectoryEvent::Rejected { user: u, .. } if u == user))
    }

    #[test]
    fn a_consistent_directory_is_accepted() {
        let w = world();
        let mut state = w.state();
        let (people, events) = apply(&mut state, &w.alice, &w.users());
        assert_eq!(ids(&people), vec![w.alice.clone(), w.bob.clone(), w.notifier.clone()]);
        assert!(events.contains(&DirectoryEvent::Changed));
        assert!(!events.iter().any(|e| matches!(e, DirectoryEvent::Rejected { .. })), "{events:?}");
        let bob = &people[1];
        assert_eq!(bob.name, "Bob", "nom du certificat, pas du relais");
        assert_eq!(bob.devices.len(), 2);
        assert!(bob.devices.iter().all(|d| d.up_to_date));
        assert_eq!(bob.devices[0].exchange_key, w.b1.member.exchange_key);
        assert_eq!(state.known().get(&w.bob), Some(&2));

        // Relire sans rien de nouveau ne change rien.
        let mut again = w.users();
        for u in &mut again {
            u.log.clear();
        }
        let (people, events) = apply(&mut state, &w.alice, &again);
        assert_eq!(people.len(), 3);
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn without_anchor_nothing_is_believed() {
        let w = world();
        let (people, events) = apply(&mut DirectoryState::default(), &w.alice, &w.users());
        assert!(people.is_empty() && events.is_empty());
        let mut wrong = DirectoryState::with_anchor([7; 16]);
        let (people, events) = apply(&mut wrong, &w.alice, &w.users());
        assert!(people.is_empty());
        assert!(rejected(&events, &w.alice));
    }

    #[test]
    fn a_device_injected_in_profiles_is_ignored() {
        let w = world();
        let intruder = dev("intrus", 9);
        let mut users = w.users();
        users[1].devices.push(profile(&intruder));
        let (people, _) = apply(&mut w.state(), &w.alice, &users);
        assert!(people[1].devices.iter().all(|d| d.id != intruder.id));
    }

    #[test]
    fn a_device_injected_in_the_log_is_refused() {
        let w = world();
        let intruder = dev("intrus", 9);
        // Le relais signe une entrée avec une clé à lui.
        let mut forged =
            Log { user: w.bob.clone(), entries: w.bob_log.entries.clone(), head: w.bob_log.head };
        forged.push(&intruder, 1, 3, &[&w.b1, &w.b2, &intruder]);
        let mut users = w.users();
        users[1].log = forged.entries.clone();
        users[1].devices.push(profile(&intruder));
        let (people, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        let bob = people.iter().find(|p| p.id == w.bob).unwrap();
        assert_eq!(bob.roster.version, 2, "arrêté à la dernière entrée valide");
        assert!(bob.devices.iter().all(|d| d.id != intruder.id));
    }

    #[test]
    fn truncated_or_replayed_logs_are_refused() {
        let w = world();
        let mut state = w.state();
        let mut users = w.users();
        users[1].log.remove(0);
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        assert!(people.iter().all(|p| p.id != w.bob), "aucune entrée valide : pas de Bob");

        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        // Rejeu d'une vieille entrée après la dernière connue.
        let mut users = w.users();
        users[1].log = vec![w.bob_log.entries[0].clone()];
        let (_, events) = apply(&mut state, &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        assert_eq!(state.known().get(&w.bob), Some(&2));
    }

    #[test]
    fn an_entry_signed_by_an_excluded_member_is_refused() {
        let w = world();
        let mut log =
            Log { user: w.bob.clone(), entries: w.bob_log.entries.clone(), head: w.bob_log.head };
        // Rotation qui exclut b2, puis b2 (volé) tente de se remettre.
        log.push(&w.b1, 2, 3, &[&w.b1]);
        log.push(&w.b2, 2, 4, &[&w.b1, &w.b2]);
        let mut users = w.users();
        users[1].log = log.entries.clone();
        let (people, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        let bob = people.iter().find(|p| p.id == w.bob).unwrap();
        assert_eq!(bob.roster.members.len(), 1);
        assert!(bob.devices.iter().all(|d| d.id != w.b2.id), "profil de b2 écarté");
    }

    #[test]
    fn a_certificate_from_a_removed_admin_device_falls() {
        let w = world();
        let both = [(&w.bob, &w.bob_cert), (&w.notifier, &w.notifier_cert)];
        let mut alice_log = w.alice_log();
        // Alice retire a1, qui avait signé le certificat de Bob ; son journal
        // engage toujours ce certificat.
        w.commit(&mut alice_log, &w.a2, 2, &[&w.a2], &both);
        let mut users = w.users();
        users[0].log = alice_log.entries.clone();
        let (people, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        assert!(people.iter().all(|p| p.id != w.bob));
        // La racine, elle, tient toujours (signée par a1, hors roster).
        assert!(people.iter().any(|p| p.id == w.alice));

        // a2 re-signe le certificat de Bob (même genesis) : refusé tant que
        // le journal de l'admin n'engage pas sa nouvelle empreinte, puis Bob
        // revient, sans « clés changées ».
        let resigned = UserCert::read_unverified(&w.bob_cert).unwrap();
        let resigned = UserCert { issuer: w.a2.id.clone(), ..resigned }.sign(&w.a2.sign).unwrap();
        let mut state = w.state();
        apply(&mut state, &w.alice, &users);
        users[0].log.clear();
        users[2].log.clear();
        users[1].cert = Some(resigned.clone());
        let (_, events) = apply(&mut state, &w.alice, &users);
        assert!(rejected(&events, &w.bob), "empreinte pas encore engagée");
        w.commit(
            &mut alice_log,
            &w.a2,
            2,
            &[&w.a2],
            &[(&w.bob, &resigned), (&w.notifier, &w.notifier_cert)],
        );
        users[0].log = since(&alice_log, 4);
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(!rejected(&events, &w.bob), "{events:?}");
        assert!(people.iter().any(|p| p.id == w.bob), "{events:?}");
        assert!(!events.iter().any(|e| matches!(e, DirectoryEvent::KeysChanged { .. })));
    }

    #[test]
    fn a_service_follows_d47() {
        let w = world();
        let mut state = w.state();
        let (people, _) = apply(&mut state, &w.alice, &w.users());
        assert!(people.iter().any(|p| p.id == w.notifier));

        // a2, qui a signé le journal du service, est retiré : le service
        // attend la republication par un appareil actuel de l'admin. a1
        // re-signe aussi son certificat, engagé par la même entrée.
        let cert = UserCert::read_unverified(&w.notifier_cert).unwrap();
        let cert = UserCert { issuer: w.a1.id.clone(), ..cert }.sign(&w.a1.sign).unwrap();
        let mut alice_log = w.alice_log();
        w.commit(
            &mut alice_log,
            &w.a1,
            2,
            &[&w.a1],
            &[(&w.bob, &w.bob_cert), (&w.notifier, &cert)],
        );
        let mut users = w.users();
        users[0].log = since(&alice_log, 3);
        users[1].log.clear();
        users[2].cert = Some(cert);
        users[2].log.clear();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(people.iter().all(|p| p.id != w.notifier), "{events:?}");
        assert!(rejected(&events, &w.notifier));

        // a1 republie une entrée (même roster, version + 1) : il revient,
        // l'entrée 1 signée par a2 (historique) reste valide.
        let mut log = w.notifier_log.copy();
        log.push(&w.a1, 0, 2, &[&w.n1]);
        users[0].log.clear();
        users[2].log = log.entries[1..].to_vec();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(people.iter().any(|p| p.id == w.notifier), "{events:?}");

        // Un appareil qui n'a jamais été à l'admin ne signe rien pour lui.
        let intruder = dev("intrus", 9);
        log.push(&intruder, 0, 3, &[&w.n1, &intruder]);
        users[2].log = log.entries[2..].to_vec();
        let (_, events) = apply(&mut state, &w.alice, &users);
        assert!(rejected(&events, &w.notifier));
    }

    #[test]
    fn reinvitation_changes_keys_and_restarts_the_log() {
        let w = world();
        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        let b3 = dev("bob-phone", 6);
        let cert =
            UserCert::new(w.bob.clone(), "Bob", Role::Member, genesis(&b3), 4, w.a1.id.clone())
                .sign(&w.a1.sign)
                .unwrap();
        let mut alice_log = w.alice_log();
        let both = [(&w.bob, &cert), (&w.notifier, &w.notifier_cert)];
        w.commit(&mut alice_log, &w.a1, 1, &[&w.a1, &w.a2], &both);
        let mut log = Log::new(&w.bob);
        log.push(&b3, 1, 1, &[&b3]);
        let mut users = w.users();
        users[0].log = since(&alice_log, 3);
        users[1].cert = Some(cert.clone());
        users[1].log = log.entries.clone();
        users[1].devices = vec![profile(&b3)];
        users[2].log.clear();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(events.contains(&DirectoryEvent::KeysChanged { user: w.bob.clone() }));
        let bob = people.iter().find(|p| p.id == w.bob).unwrap();
        assert_eq!(bob.devices[0].id, b3.id);
        assert_eq!(state.known().get(&w.bob), Some(&1));

        // Le relais rejoue l'ancien certificat et l'ancien journal : refusé.
        let mut replay = w.users();
        replay[0].log.clear();
        replay[2].log.clear();
        let (people, events) = apply(&mut state, &w.alice, &replay);
        assert!(rejected(&events, &w.bob));
        assert!(people.iter().all(|p| p.id != w.bob || p.devices.iter().all(|d| d.id != w.b1.id)));
        assert_eq!(
            UserCert::read_unverified(&state.people[&w.bob].cert).unwrap().genesis.device_id,
            b3.id
        );

        // Un client qui n'a jamais vu le nouveau certificat ne prend pas
        // l'ancien non plus (note 0056, point 13).
        let mut fresh = w.users();
        fresh[0].log = alice_log.entries.clone();
        let (people, events) = apply(&mut w.state(), &w.alice, &fresh);
        assert!(rejected(&events, &w.bob));
        assert!(people.iter().all(|p| p.id != w.bob));

        // Si le relais filtre encore avec l'ancien `known` : tout relire,
        // au tour suivant.
        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        log.push(&b3, 1, 2, &[&b3]);
        users[1].log = log.entries[1..].to_vec();
        let out = state.apply(&w.alice, &users);
        assert!(out.events.contains(&DirectoryEvent::Refetch { user: w.bob.clone() }));
        assert!(out.again);
        assert_eq!(state.known().get(&w.bob), None);
        users[0].log.clear();
        users[1].log = log.entries.clone();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(!rejected(&events, &w.bob), "{events:?}");
        assert_eq!(people.iter().find(|p| p.id == w.bob).unwrap().roster.version, 2);
    }

    #[test]
    fn certificates_for_someone_else_or_a_second_admin_are_refused() {
        let w = world();
        let mut users = w.users();
        // Le relais présente le certificat de Bob sous un autre identifiant.
        users[2].cert = Some(w.bob_cert.clone());
        let (_, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.notifier));
        // Un « admin » de plus, même signé par l'admin.
        let fake =
            UserCert::new(w.bob.clone(), "Bob", Role::Admin, genesis(&w.b1), 5, w.a1.id.clone())
                .sign(&w.a1.sign)
                .unwrap();
        let mut users = w.users();
        users[1].cert = Some(fake);
        let (_, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.bob));
    }

    #[test]
    fn people_no_longer_shown_are_forgotten_and_state_roundtrips() {
        let w = world();
        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        let json = serde_json::to_string(&state).unwrap();
        let back: DirectoryState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
        let users = w.users()[..2].to_vec();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert_eq!(people.len(), 2);
        assert!(events.contains(&DirectoryEvent::Changed));
        assert!(!state.people.contains_key(&w.notifier));
    }

    #[test]
    fn fork_of_my_own_person_is_detected() {
        let w = world();
        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        let bob_roster = state.people[&w.bob].chain.as_ref().unwrap().roster.clone();
        assert!(!state.is_fork(&w.bob, &bob_roster));
        let other = dev("autre", 8);
        let mut local = bob_roster.clone();
        local.members[1] = other.member.clone();
        assert!(state.is_fork(&w.bob, &local), "même version, autres membres");
        local.version += 1;
        assert!(!state.is_fork(&w.bob, &local), "version plus récente : pas une fourche");
    }

    #[test]
    fn an_uncommitted_or_revoked_certificate_is_refused() {
        let w = world();
        // Journal de l'admin sans l'entrée qui engage les certificats.
        let mut users = w.users();
        users[0].log.truncate(2);
        let (people, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.bob) && rejected(&events, &w.notifier));
        assert_eq!(ids(&people), vec![w.alice.clone()]);

        // Révocation de Bob : l'entrée suivante de l'admin ne l'engage plus,
        // même si le relais le sert encore, à un client ancien ou neuf.
        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        let (entry, roster) =
            state.next_admin_entry(&w.a1.id, &w.a1.sign, CertChange::Revoke(&w.bob)).unwrap();
        assert_eq!((entry.seq, roster.version), (4, 4));
        let mut users = w.users();
        users[0].log = vec![entry.clone()];
        users[1].log.clear();
        users[2].log.clear();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        assert!(people.iter().all(|p| p.id != w.bob));
        assert!(people.iter().any(|p| p.id == w.notifier), "{events:?}");
        let mut fresh = w.users();
        fresh[0].log.push(entry);
        let (people, _) = apply(&mut w.state(), &w.alice, &fresh);
        assert!(people.iter().all(|p| p.id != w.bob));
    }

    #[test]
    fn the_admin_entry_commits_a_new_certificate() {
        let w = world();
        let mut state = w.state();
        apply(&mut state, &w.alice, &w.users());
        let carol = UserId::generate();
        let c1 = dev("carol-phone", 7);
        let cert =
            UserCert::new(carol.clone(), "Carol", Role::Member, genesis(&c1), 5, w.a2.id.clone())
                .sign(&w.a2.sign)
                .unwrap();
        let (entry, _) = state
            .next_admin_entry(&w.a2.id, &w.a2.sign, CertChange::Certify(&carol, &cert))
            .unwrap();
        let mut carol_log = Log::new(&carol);
        carol_log.push(&c1, 1, 1, &[&c1]);
        let mut users = w.users();
        users[0].log = vec![entry];
        users[1].log.clear();
        users[2].log.clear();
        users.push(entry_of(&carol, &cert, &carol_log, &c1));
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(!events.iter().any(|e| matches!(e, DirectoryEvent::Rejected { .. })), "{events:?}");
        assert!(people.iter().any(|p| p.id == carol && p.name == "Carol"));
        let certs = &state.admin.as_ref().unwrap().1.chain.as_ref().unwrap().certs;
        assert_eq!(certs.len(), 3, "Bob et le service restent engagés");

        // Seul un appareil du roster de l'admin, avec sa clé, signe l'entrée.
        let other = dev("autre", 8);
        assert!(state.next_admin_entry(&other.id, &other.sign, CertChange::None).is_err());
        assert!(state.next_admin_entry(&w.a1.id, &w.a2.sign, CertChange::None).is_err());
        assert!(
            state
                .next_admin_entry(&w.a1.id, &w.a1.sign, CertChange::Certify(&w.alice, &cert))
                .is_err()
        );
        assert!(w.state().next_admin_entry(&w.a1.id, &w.a1.sign, CertChange::None).is_err());
    }

    fn entry_of(id: &UserId, cert: &SignedBlob, log: &Log, d: &Dev) -> DirectoryUser {
        entry(id, Role::Member, cert, log, &[d])
    }

    #[test]
    fn certs_outside_the_admin_log_are_refused() {
        let w = world();
        let mut log = w.bob_log.copy();
        log.push_certs(&w.b1, 1, 3, &[&w.b1, &w.b2], &[(&w.notifier, &w.notifier_cert)]);
        let mut users = w.users();
        users[1].log = log.entries.clone();
        let (people, events) = apply(&mut w.state(), &w.alice, &users);
        assert!(rejected(&events, &w.bob));
        assert_eq!(people.iter().find(|p| p.id == w.bob).unwrap().roster.version, 2);
    }

    #[test]
    fn the_admin_head_received_on_arrival_pins_the_admin_log() {
        let w = world();
        let head = log_head(&w.alice_log.entries[2].entry).unwrap();
        let pinned = || DirectoryState::with_admin_head(anchor(&w.root), head);

        // Journal complet : accepté, la tête n'est plus nécessaire.
        let mut state = pinned();
        let (people, events) = apply(&mut state, &w.alice, &w.users());
        assert_eq!(people.len(), 3, "{events:?}");
        assert_eq!(state.admin_head, None);

        // Journal plus court (le relais cache les dernières entrées) : rien.
        let mut users = w.users();
        users[0].log.truncate(2);
        let mut state = pinned();
        let (people, events) = apply(&mut state, &w.alice, &users);
        assert!(people.is_empty());
        assert!(events.contains(&DirectoryEvent::Rejected {
            user: w.alice.clone(),
            reason: DirectoryError::AdminHead("plus court que"),
        }));
        assert_eq!(state.admin_head, Some(head), "toujours épinglé");

        // Fourche : une autre entrée 3, bien signée par un ancien appareil
        // de l'admin allié au relais.
        let mut fork = Log {
            user: w.alice.clone(),
            entries: w.alice_log.entries[..2].to_vec(),
            head: entry_hash(&w.alice_log.entries[1].entry),
        };
        fork.push_certs(&w.a2, 1, 3, &[&w.a1, &w.a2], &[(&w.bob, &w.bob_cert)]);
        users[0].log = fork.entries.clone();
        let (people, events) = apply(&mut pinned(), &w.alice, &users);
        assert!(people.is_empty());
        assert!(events.contains(&DirectoryEvent::Rejected {
            user: w.alice.clone(),
            reason: DirectoryError::AdminHead("diffère de"),
        }));
    }

    #[test]
    fn cut_logs_ask_for_another_round() {
        let w = world();
        // Journal de l'admin coupé : les certificats attendent sa fin.
        let mut users = w.users();
        users[0].log.truncate(2);
        users[0].more = true;
        let mut state = w.state();
        let out = state.apply(&w.alice, &users);
        assert!(out.again && out.people.is_none(), "{out:?}");
        assert!(!out.events.iter().any(|e| matches!(e, DirectoryEvent::Rejected { .. })));
        assert_eq!(state.known().get(&w.alice), Some(&2));
        let mut users = w.users();
        users[0].log = since(&w.alice_log, 2);
        let out = state.apply(&w.alice, &users);
        assert!(!out.again);
        assert_eq!(out.people.unwrap().len(), 3);

        // Journal d'une autre personne coupé : elle est montrée à sa dernière
        // entrée lue, et un autre tour suit.
        let mut users = w.users();
        users[1].log.truncate(1);
        users[1].more = true;
        let mut state = w.state();
        let out = state.apply(&w.alice, &users);
        assert!(out.again);
        assert_eq!(state.known().get(&w.bob), Some(&1));
    }

    fn page(users: &[DirectoryUser]) -> Vec<DirectoryUser> {
        users.to_vec()
    }

    #[test]
    fn a_read_follows_next_then_rounds() {
        let w = world();
        let all = w.users();
        let mut known = Known::new();
        known.insert(w.alice.clone(), 1);
        let (mut read, first) = DirectoryRead::start(known.clone());
        assert_eq!(first, Frame::DirectoryGet { known: known.clone(), after: None });
        let step = read
            .page(w.bob.clone(), w.alice.clone(), page(&all[..2]), Some(w.bob.clone()))
            .unwrap();
        // Même `known` pour toutes les pages d'un tour.
        assert_eq!(
            step,
            ReadStep::Request(Box::new(Frame::DirectoryGet {
                known: known.clone(),
                after: Some(w.bob.clone())
            }))
        );
        // Une personne servie deux fois (le relais repart du début) : une seule.
        let step = read.page(w.bob.clone(), w.alice.clone(), page(&all[1..]), None).unwrap();
        let ReadStep::Round { me, admin, users } = step else { panic!("tour attendu") };
        assert_eq!((me, admin), (w.bob.clone(), w.alice.clone()));
        assert_eq!(users.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), ids_of(&all));

        // Un relais qui change `me` ou `admin` en cours de lecture : arrêt.
        let (mut read, _) = DirectoryRead::start(Known::new());
        read.page(w.bob.clone(), w.alice.clone(), page(&all[..1]), Some(w.alice.clone())).unwrap();
        assert!(read.page(w.alice.clone(), w.alice.clone(), Vec::new(), None).is_err());

        // Tours bornés.
        let (mut read, _) = DirectoryRead::start(Known::new());
        let mut rounds = 1;
        while read.next_round(Known::new()).is_some() {
            rounds += 1;
        }
        assert_eq!(rounds, MAX_DIRECTORY_ROUNDS);

        // Pages bornées.
        let (mut read, _) = DirectoryRead::start(Known::new());
        let mut res = Ok(ReadStep::Request(Box::new(Frame::DirectoryChanged)));
        for _ in 0..=MAX_DIRECTORY_PAGES {
            res = read.page(w.bob.clone(), w.alice.clone(), Vec::new(), Some(w.bob.clone()));
        }
        assert!(res.is_err());
    }

    fn ids_of(users: &[DirectoryUser]) -> Vec<UserId> {
        users.iter().map(|u| u.id.clone()).collect()
    }
}
