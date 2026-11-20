//! Personnes de service (étape 10, § 6.1 et 6.2), partie sans réseau.
//!
//! Une personne de service n'a pas de groupe : ses appareils ont une
//! identité et un annuaire, et scellent en `0x05`. Son journal est tenu par
//! les appareils de l'admin (règle 3 du § 3.2, D47).
//!
//! Le déroulé (le [`crate::Client`] le mène) :
//!
//! 1. appareil de service : QR inversé ([`Secure::offer`]), comme un
//!    appareil qui rejoint un groupe ;
//! 2. admin : [`Secure::check_service_offer`], certificat
//!    ([`Secure::service_cert`]), `service_create` ou `service_device`,
//!    entrée de mon journal qui engage le certificat, entrée du journal du
//!    service ([`Secure::next_service_entry`]), puis `service_welcome`
//!    scellé en `0x04` sous le secret du QR ([`Secure::service_welcome`]) ;
//! 3. appareil de service : bienvenue ouverte et rangée, annuaire lu depuis
//!    l'ancre qu'elle porte jusqu'à `admin_head` ; une fois son certificat
//!    et son roster confirmés par l'annuaire, et la signature de la
//!    bienvenue vérifiée par la clé de l'appareil de l'admin qui l'envoie,
//!    [`Event::Joined`].

use ot_crypto::directory::{GENESIS_PREV, Genesis, LogHead, RosterEntry, UserCert, entry_hash};
use ot_crypto::{Body, Member, Opened, PairingOffer, Roster, SealKey};
use ot_proto::{DeviceId, Envelope, LogEntry, Role, SignedBlob, UserId};
use time::OffsetDateTime;

use super::{Effect, Handled, NextEntry, OwnEntry, Reject, Secure, crypto};
use crate::directory::{Chain, DirectoryState};
use crate::roster;
use crate::types::{ClientError, Event};

/// Bienvenue d'un appareil de service, ouverte sous le secret du QR et
/// rangée en mémoire jusqu'à la lecture de l'annuaire : sa signature se
/// vérifie alors par la clé de l'appareil de l'admin qui l'a envoyée, lue
/// dans le roster vérifié de l'admin.
pub(super) struct ServiceJoin {
    user: UserId,
    sender: DeviceId,
    cert: SignedBlob,
    opened: Opened,
}

impl Secure {
    /// Cet appareil est un appareil de l'admin, membre de son roster publié
    /// et vérifié : lui seul signe pour un service.
    fn admin_device(&self) -> Result<(), ClientError> {
        let me_user = self
            .me_user
            .as_ref()
            .ok_or_else(|| ClientError::Invalid("annuaire pas encore lu".into()))?;
        if self.directory.admin_id() != Some(me_user) {
            return Err(ClientError::Invalid("réservé à l'admin".into()));
        }
        let mine = self.directory.admin_roster().and_then(|r| r.member(&self.me));
        if mine.is_none_or(|m| m.sign_key != self.identity.public_key()) {
            return Err(ClientError::Invalid(
                "cet appareil n'est pas encore dans le roster publié de l'admin".into(),
            ));
        }
        Ok(())
    }

    /// Admin : vérifie le QR d'un appareil de service avant de le faire
    /// enregistrer par le relais (signature de l'offre, appareil étranger à
    /// mon groupe).
    pub fn check_service_offer(&self, offer: &PairingOffer) -> Result<(), ClientError> {
        self.admin_device()?;
        offer.verify().map_err(|e| ClientError::Pairing(e.to_string()))?;
        let mine = self.roster.as_ref().is_some_and(|r| {
            r.members.iter().any(|m| {
                m.device_id == offer.device_id
                    || m.sign_key == offer.sign_key
                    || m.exchange_key == offer.exchange_key
            })
        });
        if mine {
            return Err(ClientError::Pairing("appareil déjà membre de mon groupe".into()));
        }
        Ok(())
    }

    /// Admin : certificat d'une personne de service dont l'appareil de
    /// `offer` est le premier (`genesis`), signé par cet appareil.
    pub fn service_cert(
        &self,
        user: &UserId,
        name: &str,
        offer: &PairingOffer,
        now: OffsetDateTime,
    ) -> Result<SignedBlob, ClientError> {
        self.admin_device()?;
        let genesis = Genesis {
            device_id: offer.device_id.clone(),
            name: offer.name.clone(),
            sign_key: offer.sign_key,
            exchange_key: offer.exchange_key,
        };
        UserCert::new(
            user.clone(),
            name.trim(),
            Role::Service,
            genesis,
            now.unix_timestamp(),
            self.me.clone(),
        )
        .sign(self.identity.signing_key())
        .map_err(crypto)
    }

    /// Certificat vérifié (engagé par le journal de l'admin) d'un service.
    pub fn service_known_cert(&self, user: &UserId) -> Option<SignedBlob> {
        let known = self.directory.people.get(user)?;
        let cert = UserCert::read_unverified(&known.cert).ok()?;
        (cert.role == Role::Service).then(|| known.cert.clone())
    }

    /// Admin : entrée suivante du journal du service `user`, signée par cet
    /// appareil. Avec `add`, l'appareil entre dans son roster (première
    /// entrée pour un service neuf) ; sans, le même roster est republié
    /// (version + 1) si la dernière entrée vient d'un appareil retiré de
    /// l'admin (D47, note 0054). Rien à faire : `Done`, avec la tête.
    pub fn next_service_entry(
        &self,
        user: &UserId,
        add: Option<&Member>,
    ) -> Result<NextEntry, ClientError> {
        self.admin_device()?;
        let known = self.directory.people.get(user);
        if let Some(k) = known {
            let cert = UserCert::read_unverified(&k.cert).map_err(crypto)?;
            if cert.role != Role::Service {
                return Err(ClientError::Invalid("ce n'est pas une personne de service".into()));
            }
        }
        let chain = known.and_then(|k| k.chain.as_ref());
        let (seq, prev, roster) = match (chain, add) {
            (None, None) => return Ok(NextEntry::Done(None)),
            (None, Some(m)) => {
                (1, GENESIS_PREV, Roster { epoch: 0, version: 1, members: vec![m.clone()] })
            }
            (Some(c), add) => {
                let current =
                    self.directory.admin_roster().is_some_and(|r| r.member(&c.signer).is_some());
                let roster = match add.map(|m| (m, c.roster.member(&m.device_id))) {
                    Some((m, Some(old)))
                        if old.sign_key != m.sign_key || old.exchange_key != m.exchange_key =>
                    {
                        return Err(ClientError::Invalid(
                            "appareil déjà au service avec d'autres clés".into(),
                        ));
                    }
                    Some((m, None)) => roster::with_member(&c.roster, m.clone())
                        .map_err(|e| ClientError::Invalid(e.to_string()))?,
                    // Déjà dans le roster, ou republication : seulement si la
                    // dernière entrée ne vaut plus (signataire retiré).
                    _ if current => return Ok(NextEntry::Done(Some(c.head()))),
                    _ => Roster { version: c.roster.version + 1, ..c.roster.clone() },
                };
                (c.seq + 1, c.head, roster)
            }
        };
        let entry = RosterEntry::new(user.clone(), seq, prev, self.me.clone(), roster.clone())
            .sign(self.identity.signing_key())
            .map_err(crypto)?;
        Ok(NextEntry::Entry(Box::new(OwnEntry {
            user: user.clone(),
            entry: LogEntry { seq, entry },
            roster,
            certs: Vec::new(),
        })))
    }

    /// Entrée du journal d'un service acceptée par le relais : elle prolonge
    /// le journal vérifié connu (une relecture la vérifie de toute façon).
    pub fn service_entry_published(&mut self, own: &OwnEntry) -> Vec<Effect> {
        let mut fx = Vec::new();
        let chain = Chain {
            seq: own.entry.seq,
            head: entry_hash(&own.entry.entry),
            signer: self.me.clone(),
            roster: own.roster.clone(),
            certs: Vec::new(),
        };
        if self.directory.people.contains_key(&own.user) {
            self.directory.extend_own(&own.user, chain);
            if let Err(e) = self.keyring.save_directory(&self.directory) {
                fx.push(Effect::Emit(Event::KeyringError { message: e.0 }));
            }
        }
        fx
    }

    /// Admin : services dont la dernière entrée vient d'un appareil retiré
    /// de l'admin, à republier (D47). Vide hors d'un appareil de l'admin.
    pub fn services_to_republish(&self) -> Vec<UserId> {
        if self.admin_device().is_err() {
            return Vec::new();
        }
        let Some(admin) = self.directory.admin_roster() else { return Vec::new() };
        self.directory
            .people
            .iter()
            .filter(|(_, k)| {
                UserCert::read_unverified(&k.cert).is_ok_and(|c| c.role == Role::Service)
            })
            .filter(|(_, k)| k.chain.as_ref().is_some_and(|c| admin.member(&c.signer).is_none()))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Admin : bienvenue de l'appareil de service de `offer`, scellée en
    /// `0x04` sous le secret de son QR : sa personne, son certificat,
    /// l'ancre, et la tête de mon journal qui engage le certificat.
    pub fn service_welcome(
        &self,
        user: &UserId,
        cert: &SignedBlob,
        admin_head: LogHead,
        offer: &PairingOffer,
    ) -> Result<Envelope, ClientError> {
        let anchor = self
            .directory
            .anchor
            .ok_or_else(|| ClientError::Invalid("pas d'annuaire : `admin init` d'abord".into()))?;
        let body =
            Body::ServiceWelcome { user: user.clone(), cert: cert.clone(), anchor, admin_head };
        let key = SealKey::PairQr { recipient: offer.exchange_key, secret: &offer.secret };
        self.seal_env(vec![offer.device_id.clone()], key, &body).map_err(crypto)
    }

    // --- Côté appareil de service ---

    /// Appareil de service : bienvenue de l'admin, ouverte par le secret de
    /// mon QR (lui seul et l'admin qui l'a scanné le connaissent). Rangée
    /// avec l'ancre et la tête du journal de l'admin ; elle se confirme à la
    /// lecture de l'annuaire ([`Secure::finish_service`]).
    pub(super) fn on_service_welcome(
        &mut self,
        env: &Envelope,
        opened: Opened,
        body: Body,
    ) -> Result<Handled, Reject> {
        let Body::ServiceWelcome { user, cert, anchor, admin_head } = body else {
            return Err("service_welcome attendu".into());
        };
        if self.keys.is_some() {
            return Err("cet appareil a déjà un groupe".into());
        }
        if self.service_join.is_some() {
            return Err(Reject::Ignored("bienvenue déjà reçue"));
        }
        let c = UserCert::read_unverified(&cert).map_err(|e| Reject::Reason(e.to_string()))?;
        let g = &c.genesis;
        let keys_differ = g.sign_key != self.identity.public_key()
            || g.exchange_key != self.exchange.public_key();
        if c.user != user || c.role != Role::Service || (g.device_id == self.me && keys_differ) {
            return Err("certificat de service qui ne convient pas à cet appareil".into());
        }
        match self.directory.anchor {
            Some(a) if a != anchor => return Err("ancre différente de celle déjà connue".into()),
            Some(_) => {
                let chain = self.directory.admin.as_ref().and_then(|(_, k)| k.chain.as_ref());
                match chain {
                    Some(c) if c.seq == admin_head.seq && c.head != admin_head.hash => {
                        return Err("journal de l'admin différent de la bienvenue".into());
                    }
                    Some(c) if c.seq >= admin_head.seq => {}
                    _ => self.directory.admin_head = Some(admin_head),
                }
            }
            None => self.directory = DirectoryState::with_admin_head(anchor, admin_head),
        }
        self.keyring.save_directory(&self.directory).map_err(|e| Reject::Reason(e.0))?;
        // Une seule bienvenue par QR.
        self.joining_qr = None;
        self.service_join = Some(ServiceJoin { user, sender: env.from.clone(), cert, opened });
        Ok(Handled::Done)
    }

    /// Une bienvenue de service attend sa confirmation par l'annuaire.
    pub(super) fn awaits_service(&self) -> bool {
        self.service_join.is_some()
    }

    /// Après un tour complet de lecture : si l'annuaire confirme la
    /// bienvenue (tête de l'admin atteinte, certificat engagé, cet appareil
    /// dans le roster vérifié du service, signature de l'appareil de l'admin
    /// qui l'a envoyée), l'appareil est rangé comme service.
    pub(super) fn finish_service(&mut self, fx: &mut Vec<Effect>) {
        let Some(p) = &self.service_join else { return };
        if self.directory.admin_head.is_some() {
            return;
        }
        let fail = |fx: &mut Vec<Effect>, reason: &str| {
            tracing::warn!(reason, "bienvenue de service refusée");
            fx.push(Effect::Emit(Event::InviteFailed { reason: reason.to_owned() }));
        };
        if self.me_user.as_ref() != Some(&p.user) {
            fail(fx, "le relais rattache cet appareil à une autre personne");
            self.service_join = None;
            return;
        }
        let confirmed = self.directory.known_person(&p.user).is_some_and(|k| k.cert == p.cert);
        let me = self.identity.public_key();
        let listed = self.my_person().is_some_and(|s| {
            s.role == Role::Service
                && s.roster.member(&self.me).is_some_and(|m| {
                    m.sign_key == me && m.exchange_key == self.exchange.public_key()
                })
        });
        if !confirmed || !listed {
            // Pas encore : l'admin publie le journal du service avant la
            // bienvenue, la lecture suivante le verra.
            return;
        }
        let Some(key) = self.directory.admin_roster().and_then(|r| r.member(&p.sender)) else {
            fail(fx, "bienvenue d'un appareil hors du roster de l'admin");
            self.service_join = None;
            return;
        };
        if p.opened.verify(&key.sign_key).is_err() {
            fail(fx, "bienvenue de service mal signée");
            self.service_join = None;
            return;
        }
        let user = p.user.clone();
        self.service_join = None;
        tracing::info!(user = %user, "appareil de service rangé");
        fx.push(Effect::Emit(Event::Joined { user }));
    }
}

/// Membre du roster d'un service pour l'appareil de `offer` (époque 0).
#[cfg(feature = "net")]
pub(crate) fn service_member(offer: &PairingOffer) -> Member {
    Member {
        device_id: offer.device_id.clone(),
        name: offer.name.clone(),
        sign_key: offer.sign_key,
        exchange_key: offer.exchange_key,
        added_epoch: 0,
    }
}
