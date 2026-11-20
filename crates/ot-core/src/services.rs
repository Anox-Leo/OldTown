//! Personnes de service (étape 10, § 6.1 et 6.2), partie réseau : l'admin
//! ajoute un appareil de service (et crée le service au premier), et
//! republie le journal d'un service après le retrait d'un de ses appareils
//! (D47). Chaque objet est signé ou vérifié par [`crate::Secure`].

use ot_crypto::PairingOffer;
use ot_crypto::directory::LogHead;
use ot_proto::{DeviceId, ErrorCode, Frame, Role, UserId, UserState};
use time::OffsetDateTime;

use crate::client::Client;
use crate::secure::{NextEntry, Publish, service_member};
use crate::types::ClientError;

/// Tentatives d'une publication en conflit avant d'abandonner.
const PUBLISH_ATTEMPTS: usize = 4;

/// Appareil de service ajouté par [`Client::add_service_device`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAdded {
    pub user: UserId,
    pub device_id: DeviceId,
    pub device_name: String,
    /// La personne de service vient d'être créée (premier appareil).
    pub created: bool,
}

fn unexpected(frame: &Frame) -> ClientError {
    ClientError::Invalid(format!("réponse inattendue du relais : {}", frame.type_name()))
}

impl Client {
    /// Admin : ajoute l'appareil de service dont `qr` est le QR inversé
    /// (`ot pair join --qr --service`) à la personne de service `service`
    /// (nom ou identifiant), créée si elle n'existe pas. Enregistrement par
    /// le relais, certificat engagé par mon journal, entrée du journal du
    /// service, puis bienvenue scellée sous le secret du QR. Relancée avec
    /// le même QR après une coupure, elle reprend où elle s'était arrêtée.
    pub async fn add_service_device(
        &self,
        service: &str,
        qr: &str,
    ) -> Result<ServiceAdded, ClientError> {
        let offer = PairingOffer::parse(qr).map_err(|e| ClientError::Pairing(e.to_string()))?;
        let service = service.trim();
        if !ot_proto::valid_user_name(service) {
            return Err(ClientError::Invalid("nom de service invalide".into()));
        }
        let o = offer.clone();
        self.with_fresh(move |s| s.check_service_offer(&o)).await?;
        let users = self.users().await?;
        let existing = users
            .into_iter()
            .filter(|u| u.state != UserState::Revoked)
            .find(|u| u.name == service || u.id.as_str() == service);
        let now = OffsetDateTime::now_utc();
        let (user, cert, created) = match existing {
            Some(u) if u.role != Role::Service => {
                return Err(ClientError::Invalid(format!("{} n'est pas un service", u.name)));
            }
            None => {
                let user = UserId::generate();
                let (u, name, o) = (user.clone(), service.to_owned(), offer.clone());
                let cert = self.with_secure(move |s| s.service_cert(&u, &name, &o, now)).await?;
                let frame = Frame::ServiceCreate {
                    user: user.clone(),
                    name: service.to_owned(),
                    device: offer.device_id.clone(),
                    public_key: offer.sign_key,
                    device_name: offer.name.clone(),
                    sig: offer.sig,
                    cert: cert.clone(),
                };
                self.register(frame).await?;
                tracing::info!(user = %user, "personne de service créée");
                (user, cert, true)
            }
            Some(u) => {
                if !u.devices.iter().any(|d| d.id == offer.device_id) {
                    let frame = Frame::ServiceDevice {
                        user: u.id.clone(),
                        device: offer.device_id.clone(),
                        public_key: offer.sign_key,
                        name: offer.name.clone(),
                        sig: offer.sig,
                    };
                    self.register(frame).await?;
                }
                let id = u.id.clone();
                let known = self.after_read(move |s| Ok(s.service_known_cert(&id))).await?;
                let cert = match known {
                    Some(cert) => cert,
                    // Création coupée avant l'engagement du certificat (reprise
                    // avec le même QR) : cet appareil est son `genesis`, le
                    // certificat est re-signé et rendu au relais.
                    None if u.devices.iter().all(|d| d.id == offer.device_id) => {
                        let (id, name, o) = (u.id.clone(), u.name.clone(), offer.clone());
                        let cert =
                            self.with_secure(move |s| s.service_cert(&id, &name, &o, now)).await?;
                        let frame = Frame::UserCertify { user: u.id.clone(), cert: cert.clone() };
                        match self.call(frame).await? {
                            Frame::UserCertified { .. } => {}
                            other => return Err(unexpected(&other)),
                        }
                        cert
                    }
                    None => {
                        return Err(ClientError::Invalid(format!(
                            "le service {} n'est pas vérifié par l'annuaire",
                            u.name
                        )));
                    }
                };
                (u.id, cert, false)
            }
        };
        let head = self
            .publish(Publish::Certify(user.clone(), cert.clone()))
            .await?
            .ok_or_else(|| ClientError::Invalid("journal de l'admin introuvable".into()))?;
        self.publish_service(&user, Some(service_member(&offer))).await?;
        let (u, o) = (user.clone(), offer.clone());
        let env = self.with_secure(move |s| s.service_welcome(&u, &cert, head, &o)).await?;
        self.send_envelope(env).await?;
        self.refresh_directory();
        tracing::info!(user = %user, device = %offer.device_id, "appareil de service ajouté");
        Ok(ServiceAdded {
            user,
            device_id: offer.device_id.clone(),
            device_name: offer.name.clone(),
            created,
        })
    }

    /// `service_create` ou `service_device` : réponse `pair_registered`.
    async fn register(&self, frame: Frame) -> Result<(), ClientError> {
        match self.call(frame).await? {
            Frame::PairRegistered { .. } => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// Admin : publie l'entrée suivante du journal du service `user`
    /// ([`crate::Secure::next_service_entry`]), en relisant l'annuaire avant
    /// (le service vient peut-être d'être créé ou engagé) et après chaque
    /// `conflict`. Rend la tête, ou `None` s'il n'y avait rien à publier.
    pub(crate) async fn publish_service(
        &self,
        user: &UserId,
        add: Option<ot_crypto::Member>,
    ) -> Result<Option<LogHead>, ClientError> {
        let _one = self.publish_lock.lock().await;
        for attempt in 0..PUBLISH_ATTEMPTS {
            let (u, m) = (user.clone(), add.clone());
            let own = match self.after_read(move |s| s.next_service_entry(&u, m.as_ref())).await? {
                NextEntry::Done(head) => return Ok(head),
                NextEntry::Entry(own) => own,
            };
            let frame = Frame::RosterPublish {
                user: own.user.clone(),
                seq: own.entry.seq,
                entry: own.entry.entry.clone(),
            };
            match self.call(frame).await {
                Ok(Frame::RosterPublished { .. }) => {
                    let head = own.head();
                    tracing::info!(user = %user, seq = head.seq, "journal du service publié");
                    self.with_effects(move |s| Ok(s.service_entry_published(&own))).await?;
                    self.refresh_directory();
                    return Ok(Some(head));
                }
                Ok(other) => return Err(unexpected(&other)),
                Err(ClientError::Rejected { code: ErrorCode::Conflict, .. }) => {
                    tracing::debug!(attempt, "journal du service en conflit : relecture");
                }
                Err(e) => return Err(e),
            }
        }
        Err(ClientError::Invalid("journal du service en conflit : réessayer plus tard".into()))
    }
}
