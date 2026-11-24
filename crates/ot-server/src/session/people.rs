use std::collections::BTreeSet;

use axum::extract::ws::WebSocket;
use ot_proto::{
    DeviceId, ErrorCode, Frame, Known, MAX_NAME_CHARS, NONCE_LEN, PAIR_TOKEN_LEN, Role, SignedBlob,
    UserId, UserState, valid_device_name, valid_user_name,
};
use time::OffsetDateTime;

use super::{Active, Close, Rejected, db, invalid_name, pairing_rejected, send};
use crate::metrics::Metrics;
use crate::store::StoreError;
use crate::users::{DeviceChanges, Peer};
use crate::{Inner, auth, now_ms};

enum Gate {
    Active,
    Group,
    Person,
    Admin,
}

impl Active<'_> {
    pub(super) async fn on_people_frame(&mut self, frame: Frame) -> Result<Option<Frame>, Close> {
        let gate = match &frame {
            Frame::DirectoryGet { .. } => None,
            Frame::RosterPublish { .. }
            | Frame::DeviceUpdate { .. }
            | Frame::DeviceRevoke { .. } => Some(Gate::Active),
            Frame::PairCreate | Frame::PairRegister { .. } => Some(Gate::Group),
            Frame::ServiceMute { .. } => Some(Gate::Person),
            Frame::AdminInit { .. }
            | Frame::UserInvite { .. }
            | Frame::UserReinvite { .. }
            | Frame::UserCertify { .. }
            | Frame::ServiceCreate { .. }
            | Frame::ServiceDevice { .. }
            | Frame::UserRevoke { .. }
            | Frame::UserRename { .. }
            | Frame::LinkSet { .. }
            | Frame::UsersGet => Some(Gate::Admin),
            other => {
                let msg = format!("trame {} inattendue", other.type_name());
                return Ok(Some(Frame::error(ErrorCode::UnexpectedFrame, msg, None)));
            }
        };
        let peer = self.peer().await?;
        if let Some(gate) = gate {
            let ok = peer.state == UserState::Active
                && match gate {
                    Gate::Active => true,
                    Gate::Group | Gate::Person => peer.role != Role::Service,
                    Gate::Admin => peer.role == Role::Admin,
                };
            if !ok {
                Metrics::inc(&self.inner.metrics.refused_forbidden);
                tracing::debug!(device = %self.device, t = frame.type_name(), "trame refusée");
                let msg = format!("trame {} interdite à cet appareil", frame.type_name());
                return Ok(Some(Frame::error(ErrorCode::Forbidden, msg, None)));
            }
        }

        let reply = match frame {
            Frame::DirectoryGet { known, after } => {
                self.directory(&peer, &known, after.as_ref()).await?
            }
            Frame::PairCreate => self.pair_create().await?,
            Frame::PairRegister { device_id, name, public_key, sig } => {
                self.pair_register(device_id, &name, &public_key, &sig).await
            }
            Frame::RosterPublish { user, seq, entry } => {
                self.roster_publish(user, seq, &entry).await?
            }
            Frame::DeviceUpdate { device, kind, reachable, default, features } => {
                let changes = DeviceChanges { kind, reachable, default, features };
                self.device_update(device, &changes).await?
            }
            Frame::DeviceRevoke { device } => self.device_revoke(device).await?,
            Frame::ServiceMute { service, muted } => {
                match self.inner.store.mute_service(&peer.user, &service, muted).await {
                    Ok(()) => {
                        self.notify(&peer.user).await?;
                        Frame::ServiceMuted { service, muted }
                    }
                    Err(e) => self.refused(e, Some(service.to_string())),
                }
            }
            Frame::AdminInit { cert, entry } => {
                match self.inner.store.admin_init(&cert, &entry).await {
                    Ok(user) => {
                        tracing::info!(user = %user, "certificat de l'admin publié");
                        self.inner.registry.directory_changed_all(&self.device);
                        Frame::AdminInitialized { user }
                    }
                    Err(e) => self.refused(e, None),
                }
            }
            Frame::UserInvite { name, role, links } => {
                self.user_invite(&name, role, &links).await?
            }
            Frame::UserReinvite { user } => self.user_reinvite(user).await?,
            Frame::UserCertify { user, cert } => self.user_certify(user, &cert).await?,
            Frame::ServiceCreate { user, name, device, public_key, device_name, sig, cert } => {
                self.service_create(user, &name, device, &public_key, &device_name, &sig, &cert)
                    .await?
            }
            Frame::ServiceDevice { user, device, public_key, name, sig } => {
                self.service_device(user, device, &public_key, &name, &sig).await?
            }
            Frame::UserRevoke { user } => self.user_revoke(user).await?,
            Frame::UserRename { user, name } => {
                let name = name.trim();
                if !valid_user_name(name) {
                    return Ok(Some(Frame::error(
                        ErrorCode::BadFrame,
                        invalid_user_name(),
                        Some(user.to_string()),
                    )));
                }
                match self.inner.store.rename_user(&user, name).await {
                    Ok(()) => {
                        self.notify(&user).await?;
                        Frame::UserRenamed { user }
                    }
                    Err(e) => self.refused(e, Some(user.to_string())),
                }
            }
            Frame::LinkSet { from, to, allowed } => self.link_set(from, to, allowed).await?,
            Frame::UsersGet => self.users().await?,
            other => unreachable!("trame {} non filtrée", other.type_name()),
        };
        Ok(Some(reply))
    }

    async fn peer(&mut self) -> Result<Peer, Close> {
        self.inner.store.peer(&self.device).await.map_err(db)?.ok_or(Close("appareil révoqué"))
    }

    async fn notify(&mut self, user: &UserId) -> Result<(), Close> {
        let admin = self.inner.store.admin().await.map_err(db)?;
        if &admin.id == user {
            self.inner.registry.directory_changed_all(&self.device);
            return Ok(());
        }
        let devices = self.inner.store.concerned(user).await.map_err(db)?;
        self.notify_devices(&devices);
        Ok(())
    }

    fn notify_devices<'a>(&self, devices: impl IntoIterator<Item = &'a DeviceId>) {
        let others = devices.into_iter().filter(|d| **d != self.device);
        self.inner.registry.directory_changed(others);
    }

    fn refused(&self, e: StoreError, reference: Option<String>) -> Frame {
        let m = &self.inner.metrics;
        let (code, message) = match e {
            StoreError::Forbidden(why) => {
                Metrics::inc(&m.refused_forbidden);
                (ErrorCode::Forbidden, why.to_owned())
            }
            StoreError::NotAllowed(id) => {
                Metrics::inc(&m.refused_not_allowed);
                (ErrorCode::NotAllowed, format!("hors réseau : {id}"))
            }
            StoreError::UserQuota(what) => {
                Metrics::inc(&m.refused_quota);
                (ErrorCode::QuotaExceeded, format!("quota atteint : {what}"))
            }
            StoreError::Conflict(next) => (ErrorCode::Conflict, format!("seq attendu : {next}")),
            StoreError::NameTaken(name) => {
                (ErrorCode::NameTaken, format!("le nom {name:?} est déjà pris"))
            }
            StoreError::NoAnchor => (
                ErrorCode::UnexpectedFrame,
                "l'admin n'a pas encore publié son certificat (admin_init)".into(),
            ),
            StoreError::KeyTaken => {
                (ErrorCode::PairingInvalid, "clé publique déjà enregistrée".into())
            }
            StoreError::IdTaken => (ErrorCode::PairingInvalid, "identifiant déjà utilisé".into()),
            e @ (StoreError::Invalid(_)
            | StoreError::UnknownUser(_)
            | StoreError::UnknownDevice(_)
            | StoreError::Ambiguous(_)) => (ErrorCode::BadFrame, e.to_string()),
            e => {
                tracing::error!(error = %e, "trame de l'étape 10");
                (ErrorCode::Internal, "erreur interne".into())
            }
        };
        Frame::error(code, message, reference)
    }

    async fn directory(
        &mut self,
        peer: &Peer,
        known: &Known,
        after: Option<&UserId>,
    ) -> Result<Frame, Close> {
        let page = self.inner.store.directory(&peer.user, known, after).await.map_err(db)?;
        let mut users = page.users;
        for u in &mut users {
            for d in &mut u.devices {
                d.online = self.inner.registry.is_online(&d.id);
            }
        }
        Ok(Frame::Directory { me: peer.user.clone(), admin: page.admin, users, next: page.next })
    }

    async fn roster_publish(
        &mut self,
        user: UserId,
        seq: u64,
        entry: &SignedBlob,
    ) -> Result<Frame, Close> {
        if peer_role_limited(self.role) && !self.inner.send_rate.take(&self.user) {
            Metrics::inc(&self.inner.metrics.refused_quota);
            let msg = "trop de publications : réessayer dans une minute";
            return Ok(Frame::error(ErrorCode::QuotaExceeded, msg, Some(seq.to_string())));
        }
        let quota = self.inner.config.users.log_quota;
        match self.inner.store.publish_entry(&self.device, &user, seq, entry, quota).await {
            Ok(()) => {
                tracing::info!(device = %self.device, user = %user, seq, "journal de roster publié");
                self.notify(&user).await?;
                Ok(Frame::RosterPublished { user, seq })
            }
            Err(e) => Ok(self.refused(e, Some(seq.to_string()))),
        }
    }

    async fn device_update(
        &mut self,
        device: DeviceId,
        changes: &DeviceChanges,
    ) -> Result<Frame, Close> {
        match self.inner.store.update_device(&self.device, &device, changes).await {
            Ok(owner) => {
                self.notify(&owner).await?;
                Ok(Frame::DeviceUpdated { device })
            }
            Err(e) => Ok(self.refused(e, Some(device.to_string()))),
        }
    }

    async fn device_revoke(&mut self, device: DeviceId) -> Result<Frame, Close> {
        match self.inner.store.revoke_device_by(&self.device, &device, now_ms()).await {
            Ok(owner) => {
                tracing::info!(by = %self.device, device = %device, "appareil révoqué");
                self.inner.registry.kick(&device);
                self.notify(&owner).await?;
                Ok(Frame::DeviceRevoked { device })
            }
            Err(e) => Ok(self.refused(e, Some(device.to_string()))),
        }
    }

    fn invite_token(&self) -> ([u8; PAIR_TOKEN_LEN], i64, i64) {
        let now = now_ms();
        let ttl = self.inner.config.users.invite_ttl.as_millis() as i64;
        (auth::new_token(), now, now.saturating_add(ttl))
    }

    async fn user_invite(
        &mut self,
        name: &str,
        role: Role,
        links: &[UserId],
    ) -> Result<Frame, Close> {
        let name = name.trim();
        if !valid_user_name(name) {
            return Ok(Frame::error(ErrorCode::BadFrame, invalid_user_name(), None));
        }
        if role != Role::Member {
            return Ok(Frame::error(ErrorCode::BadFrame, "seul un membre s'invite", None));
        }
        let (token, now, expires_at) = self.invite_token();
        let max = self.inner.config.users.max_open_invitations;
        let hash = auth::token_hash(&token);
        let invited =
            self.inner.store.invite_user(&self.device, name, links, &hash, max, now, expires_at);
        match invited.await {
            Ok(user) => {
                Metrics::inc(&self.inner.metrics.invitations_created);
                tracing::info!(by = %self.device, user = %user, "invitation créée");
                self.notify(&user).await?;
                Ok(Frame::UserInvited { user, token, expires_at: rfc3339(expires_at)? })
            }
            Err(e) => Ok(self.refused(e, None)),
        }
    }

    async fn user_reinvite(&mut self, user: UserId) -> Result<Frame, Close> {
        let (token, now, expires_at) = self.invite_token();
        let max = self.inner.config.users.max_open_invitations;
        let hash = auth::token_hash(&token);
        let reinvited =
            self.inner.store.reinvite_user(&self.device, &user, &hash, max, now, expires_at);
        match reinvited.await {
            Ok(revoked) => {
                Metrics::inc(&self.inner.metrics.invitations_created);
                tracing::info!(by = %self.device, user = %user, revoked = revoked.len(), "réinvitation");
                for d in &revoked {
                    self.inner.registry.kick(d);
                }
                self.notify(&user).await?;
                Ok(Frame::UserInvited { user, token, expires_at: rfc3339(expires_at)? })
            }
            Err(e) => Ok(self.refused(e, Some(user.to_string()))),
        }
    }

    async fn user_certify(&mut self, user: UserId, cert: &SignedBlob) -> Result<Frame, Close> {
        match self.inner.store.certify_user(&user, cert).await {
            Ok(()) => {
                Metrics::inc(&self.inner.metrics.invitations_certified);
                tracing::info!(by = %self.device, user = %user, "personne certifiée");
                self.notify(&user).await?;
                Ok(Frame::UserCertified { user })
            }
            Err(e) => Ok(self.refused(e, Some(user.to_string()))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn service_create(
        &mut self,
        user: UserId,
        name: &str,
        device: DeviceId,
        public_key: &[u8; 32],
        device_name: &str,
        sig: &[u8; 64],
        cert: &SignedBlob,
    ) -> Result<Frame, Close> {
        let reference = Some(device.to_string());
        let name = name.trim();
        if !valid_user_name(name) {
            return Ok(Frame::error(ErrorCode::BadFrame, invalid_user_name(), reference));
        }
        if !valid_device_name(device_name) {
            return Ok(Frame::error(ErrorCode::BadFrame, invalid_name(), reference));
        }
        if !auth::verify_register(public_key, &device, device_name, sig) {
            Metrics::inc(&self.inner.metrics.pairings_rejected);
            return Ok(Frame::error(ErrorCode::PairingInvalid, "signature invalide", reference));
        }
        let created = self.inner.store.create_service(
            &user,
            name,
            cert,
            &device,
            public_key,
            device_name,
            now_ms(),
        );
        match created.await {
            Ok(d) => {
                Metrics::inc(&self.inner.metrics.pairings_registered);
                tracing::info!(user = %user, device = %d.id, "personne de service créée");
                self.notify(&user).await?;
                Ok(Frame::PairRegistered { device_id: d.id })
            }
            Err(e) => Ok(self.refused(e, reference)),
        }
    }

    async fn service_device(
        &mut self,
        user: UserId,
        device: DeviceId,
        public_key: &[u8; 32],
        name: &str,
        sig: &[u8; 64],
    ) -> Result<Frame, Close> {
        let reference = Some(device.to_string());
        if !valid_device_name(name) {
            return Ok(Frame::error(ErrorCode::BadFrame, invalid_name(), reference));
        }
        if !auth::verify_register(public_key, &device, name, sig) {
            Metrics::inc(&self.inner.metrics.pairings_rejected);
            return Ok(Frame::error(ErrorCode::PairingInvalid, "signature invalide", reference));
        }
        match self.inner.store.add_service_device(&user, &device, public_key, name, now_ms()).await
        {
            Ok(d) => {
                Metrics::inc(&self.inner.metrics.pairings_registered);
                tracing::info!(user = %user, device = %d.id, "appareil de service enregistré");
                self.notify(&user).await?;
                Ok(Frame::PairRegistered { device_id: d.id })
            }
            Err(e) => Ok(self.refused(e, reference)),
        }
    }

    async fn user_revoke(&mut self, user: UserId) -> Result<Frame, Close> {
        match self.inner.store.revoke_user(&user, now_ms()).await {
            Ok(revoked) => {
                tracing::info!(
                    by = %self.device,
                    user = %user,
                    devices = revoked.devices.len(),
                    blobs = revoked.blobs.len(),
                    "personne révoquée"
                );
                for d in &revoked.devices {
                    self.inner.registry.kick(d);
                }
                for b in &revoked.blobs {
                    self.inner.blob_dir.remove(b).await;
                }
                self.notify_devices(&revoked.notify);
                Ok(Frame::UserRevoked { user })
            }
            Err(e) => Ok(self.refused(e, Some(user.to_string()))),
        }
    }

    async fn link_set(&mut self, from: UserId, to: UserId, allowed: bool) -> Result<Frame, Close> {
        let store = &self.inner.store;
        let mut devices: BTreeSet<DeviceId> = BTreeSet::new();
        for u in [&from, &to] {
            devices.extend(store.concerned(u).await.map_err(db)?);
        }
        match store.set_link(&from, &to, allowed, now_ms()).await {
            Ok(()) => {
                for u in [&from, &to] {
                    devices.extend(store.concerned(u).await.map_err(db)?);
                }
                tracing::info!(from = %from, to = %to, allowed, "lien modifié");
                self.notify_devices(&devices);
                Ok(Frame::LinkUpdated { from, to, allowed })
            }
            Err(e) => Ok(self.refused(e, None)),
        }
    }

    async fn users(&mut self) -> Result<Frame, Close> {
        let mut users = self.inner.store.admin_users().await.map_err(db)?;
        for u in &mut users {
            for d in &mut u.devices {
                d.online = self.inner.registry.is_online(&d.id);
            }
        }
        Ok(Frame::Users { users })
    }
}

fn peer_role_limited(role: Role) -> bool {
    role != Role::Admin
}

fn invalid_user_name() -> String {
    format!("nom de personne invalide : 1 à {MAX_NAME_CHARS} caractères imprimables")
}

fn rfc3339(ms: i64) -> Result<OffsetDateTime, Close> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .map_err(|_| Close("horloge invalide"))
}

pub(super) async fn redeem_invite(
    inner: &Inner,
    socket: &mut WebSocket,
    nonce: &[u8; NONCE_LEN],
    token: &[u8; PAIR_TOKEN_LEN],
    name: &str,
    public_key: &[u8; 32],
    sig: &[u8; 64],
) -> Result<std::convert::Infallible, Rejected> {
    let name = name.trim();
    if !valid_device_name(name) {
        return invite_rejected(inner, socket, ErrorCode::BadFrame, invalid_name()).await;
    }
    let hash = auth::token_hash(token);
    if !auth::verify_invite(public_key, nonce, &hash, sig) {
        tracing::warn!("invitation : signature invalide");
        let msg = "signature invalide".into();
        return invite_rejected(inner, socket, ErrorCode::PairingInvalid, msg).await;
    }
    match inner.store.redeem_invitation(&hash, name, public_key, now_ms()).await {
        Ok(r) => {
            Metrics::inc(&inner.metrics.invitations_redeemed);
            tracing::info!(user = %r.user, device = %r.device.id, inviter = %r.inviter, "invitation utilisée");
            let frame =
                Frame::InviteRedeemed { user: r.user, device: r.device.id, inviter: r.inviter };
            send(socket, &frame).await?;
            Err(Rejected::Closed(Close("invitation terminée")))
        }
        Err(StoreError::PairingInvalid) => {
            let msg = "invitation inconnue, expirée ou déjà utilisée".into();
            invite_rejected(inner, socket, ErrorCode::PairingInvalid, msg).await
        }
        Err(StoreError::KeyTaken) => {
            let msg = "clé publique déjà enregistrée".into();
            invite_rejected(inner, socket, ErrorCode::PairingInvalid, msg).await
        }
        Err(StoreError::NameTaken(name)) => {
            let msg = format!("le nom {name:?} est déjà pris");
            invite_rejected(inner, socket, ErrorCode::NameTaken, msg).await
        }
        Err(e) => {
            tracing::error!(error = %e, "invitation");
            invite_rejected(inner, socket, ErrorCode::Internal, "erreur interne".into()).await
        }
    }
}

async fn invite_rejected(
    inner: &Inner,
    socket: &mut WebSocket,
    code: ErrorCode,
    message: String,
) -> Result<std::convert::Infallible, Rejected> {
    Metrics::inc(&inner.metrics.invitations_rejected);
    pairing_rejected(inner, socket, code, message).await
}
