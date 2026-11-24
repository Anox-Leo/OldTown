use ot_crypto::directory::LogHead;
use ot_proto::{AdminUser, DeviceId, DeviceKind, ErrorCode, Frame, Role, UserId};
use time::OffsetDateTime;

use crate::client::Client;
use crate::invite::PersonInvitation;
use crate::secure::{NextEntry, Publish};
use crate::types::ClientError;
use crate::uri::InviteUri;

const PUBLISH_ATTEMPTS: usize = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceSettings {
    pub kind: Option<DeviceKind>,
    pub reachable: Option<bool>,
    pub default: Option<bool>,
}

fn unexpected(frame: &Frame) -> ClientError {
    ClientError::Invalid(format!("réponse inattendue du relais : {}", frame.type_name()))
}

impl Client {
    pub async fn publish(&self, what: Publish) -> Result<Option<LogHead>, ClientError> {
        let _one = self.publish_lock.lock().await;
        for attempt in 0..PUBLISH_ATTEMPTS {
            let w = what.clone();
            let next = if attempt == 0 {
                self.with_fresh(move |s| s.next_entry(&w)).await?
            } else {
                self.after_read(move |s| s.next_entry(&w)).await?
            };
            let own = match next {
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
                    tracing::info!(seq = head.seq, "journal de roster publié");
                    self.with_effects(move |s| Ok(s.entry_published(&own))).await?;
                    self.refresh_directory();
                    return Ok(Some(head));
                }
                Ok(other) => return Err(unexpected(&other)),
                Err(ClientError::Rejected { code: ErrorCode::Conflict, .. }) => {
                    tracing::debug!(attempt, "publication en conflit : relecture");
                }
                Err(e) => return Err(e),
            }
        }
        Err(ClientError::Invalid("journal publié en conflit : réessayer plus tard".into()))
    }

    pub(crate) async fn certify_invite(&self, user: UserId) -> Result<(), ClientError> {
        let now = OffsetDateTime::now_utc();
        let u = user.clone();
        let (cert, certified) = self.with_secure(move |s| s.invite_cert(&u, now)).await?;
        if !certified {
            let frame = Frame::UserCertify { user: user.clone(), cert: cert.clone() };
            match self.call(frame).await? {
                Frame::UserCertified { .. } => {}
                other => return Err(unexpected(&other)),
            }
            let u = user.clone();
            self.with_secure(move |s| s.invite_certified(&u)).await?;
        }
        let head = self
            .publish(Publish::Certify(user.clone(), cert))
            .await?
            .ok_or_else(|| ClientError::Invalid("journal de l'admin introuvable".into()))?;
        let u = user.clone();
        let env = self.with_secure(move |s| s.invite_welcome(&u, head)).await?;
        self.send_envelope(env).await?;
        self.with_effects(move |s| s.invite_done(&user)).await
    }

    pub async fn invite(
        &self,
        name: &str,
        links: Vec<UserId>,
    ) -> Result<PersonInvitation, ClientError> {
        let frame = Frame::UserInvite { name: name.to_owned(), role: Role::Member, links };
        let (user, token, expires_at) = match self.call(frame).await? {
            Frame::UserInvited { user, token, expires_at } => (user, token, expires_at),
            other => return Err(unexpected(&other)),
        };
        self.invitation(user, name.trim().to_owned(), token, expires_at).await
    }

    pub async fn reinvite(&self, user: &UserId) -> Result<PersonInvitation, ClientError> {
        let name = self
            .users()
            .await?
            .into_iter()
            .find(|u| &u.id == user)
            .map(|u| u.name)
            .ok_or_else(|| ClientError::NoRecipient("personne inconnue du relais".into()))?;
        let (user, token, expires_at) =
            match self.call(Frame::UserReinvite { user: user.clone() }).await? {
                Frame::UserInvited { user, token, expires_at } => (user, token, expires_at),
                other => return Err(unexpected(&other)),
            };
        self.invitation(user, name, token, expires_at).await
    }

    async fn invitation(
        &self,
        user: UserId,
        name: String,
        token: [u8; ot_crypto::TOKEN_LEN],
        expires_at: OffsetDateTime,
    ) -> Result<PersonInvitation, ClientError> {
        let now = OffsetDateTime::now_utc();
        let u = user.clone();
        let code =
            self.with_secure(move |s| s.create_invite(u, &name, token, expires_at, now)).await?;
        let code = code.encode().to_string();
        let uri = InviteUri { server_url: self.server_url.clone(), code: code.clone() }.encode();
        tracing::info!(user = %user, "invitation créée");
        Ok(PersonInvitation { user, code, uri, expires_at })
    }

    pub async fn users(&self) -> Result<Vec<AdminUser>, ClientError> {
        match self.call(Frame::UsersGet).await? {
            Frame::Users { users } => Ok(users),
            other => Err(unexpected(&other)),
        }
    }

    pub async fn link_set(
        &self,
        from: &UserId,
        to: &UserId,
        allowed: bool,
    ) -> Result<(), ClientError> {
        let frame = Frame::LinkSet { from: from.clone(), to: to.clone(), allowed };
        match self.call(frame).await? {
            Frame::LinkUpdated { .. } => {}
            other => return Err(unexpected(&other)),
        }
        self.refresh_directory();
        Ok(())
    }

    pub async fn revoke_user(&self, user: &UserId) -> Result<(), ClientError> {
        match self.call(Frame::UserRevoke { user: user.clone() }).await? {
            Frame::UserRevoked { .. } => {}
            other => return Err(unexpected(&other)),
        }
        self.publish(Publish::Revoke(user.clone())).await?;
        self.refresh_directory();
        Ok(())
    }

    pub async fn rename_user(&self, user: &UserId, name: &str) -> Result<(), ClientError> {
        let frame = Frame::UserRename { user: user.clone(), name: name.to_owned() };
        match self.call(frame).await? {
            Frame::UserRenamed { .. } => {}
            other => return Err(unexpected(&other)),
        }
        let (u, n) = (user.clone(), name.trim().to_owned());
        let now = OffsetDateTime::now_utc();
        match self.with_fresh(move |s| s.recertify(&u, Some(&n), now)).await {
            Ok(cert) => self.certify(user, cert).await,
            Err(ClientError::NoRecipient(_) | ClientError::Invalid(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn certify(&self, user: &UserId, cert: ot_proto::SignedBlob) -> Result<(), ClientError> {
        let frame = Frame::UserCertify { user: user.clone(), cert: cert.clone() };
        match self.call(frame).await? {
            Frame::UserCertified { .. } => {}
            other => return Err(unexpected(&other)),
        }
        self.publish(Publish::Certify(user.clone(), cert)).await.map(|_| ())
    }

    pub async fn update_device(
        &self,
        device: &DeviceId,
        settings: DeviceSettings,
    ) -> Result<(), ClientError> {
        let frame = Frame::DeviceUpdate {
            device: device.clone(),
            kind: settings.kind,
            reachable: settings.reachable,
            default: settings.default,
            features: None,
        };
        match self.call(frame).await? {
            Frame::DeviceUpdated { .. } => {}
            other => return Err(unexpected(&other)),
        }
        self.refresh_directory();
        Ok(())
    }

    pub async fn remove_device(&self, device: &DeviceId) -> Result<(), ClientError> {
        let mine = self.group().await?.is_some_and(|g| g.member(device).is_some());
        if mine {
            self.rotate(vec![device.clone()]).await?;
            self.publish(Publish::IfBehind).await?;
        }
        match self.call(Frame::DeviceRevoke { device: device.clone() }).await? {
            Frame::DeviceRevoked { .. } => {}
            other => return Err(unexpected(&other)),
        }
        if mine {
            let d = device.clone();
            let users = self.with_secure(move |s| Ok(s.certified_by(&d))).await?;
            let now = OffsetDateTime::now_utc();
            for user in users {
                let u = user.clone();
                let cert = self.with_secure(move |s| s.recertify(&u, None, now)).await?;
                self.certify(&user, cert).await?;
            }
        }
        self.refresh_directory();
        Ok(())
    }

    pub async fn mute_service(&self, service: &UserId, muted: bool) -> Result<(), ClientError> {
        let frame = Frame::ServiceMute { service: service.clone(), muted };
        match self.call(frame).await? {
            Frame::ServiceMuted { .. } => {}
            other => return Err(unexpected(&other)),
        }
        self.refresh_directory();
        Ok(())
    }
}
