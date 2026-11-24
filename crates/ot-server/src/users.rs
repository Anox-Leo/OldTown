use std::collections::HashSet;

use ot_proto::{
    AdminDevice, AdminUser, BlobId, DeviceId, DeviceKind, DirectoryDevice, DirectoryUser, Feature,
    Known, LogEntry, MAX_FEATURES, MAX_SIGNED_BYTES, Role, SignedBlob, UserId, UserState,
};
use sqlx::Row;

use crate::store::{
    DEVICE_COLUMNS, Device, NewProfile, Result, Store, StoreError, admin_id, device_from_row,
    ensure_default, insert_device, parse_device, parse_role, parse_state, parse_user, revoke_in,
};

pub const DIRECTORY_FRAME_BUDGET: usize = ot_proto::MAX_FRAME_BYTES - 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPage {
    pub admin: UserId,
    pub users: Vec<DirectoryUser>,
    pub next: Option<UserId>,
}

pub const OWNER_NAME: &str = "owner";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub role: Role,
    pub state: UserState,
    pub cert: Option<SignedBlob>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub device: DeviceId,
    pub user: UserId,
    pub role: Role,
    pub state: UserState,
}

impl Peer {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin && self.state == UserState::Active
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRedeemed {
    pub device: Device,
    pub user: UserId,
    pub inviter: DeviceId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserRevoked {
    pub devices: Vec<DeviceId>,
    pub blobs: Vec<BlobId>,
    pub notify: Vec<DeviceId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceChanges {
    pub kind: Option<DeviceKind>,
    pub reachable: Option<bool>,
    pub default: Option<bool>,
    pub features: Option<Vec<Feature>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: UserId,
    pub to: UserId,
    pub created_at: i64,
}

impl Store {
    pub(crate) async fn adopt_devices(&self, now: i64) -> Result<()> {
        let done: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin' AND state != 'revoked')
                AND NOT EXISTS (SELECT 1 FROM devices WHERE user_id IS NULL)
                AND NOT EXISTS (
                    SELECT 1 FROM users u WHERE u.role = 'admin' AND u.state != 'revoked'
                      AND EXISTS (SELECT 1 FROM devices d WHERE d.user_id = u.id
                                    AND d.revoked_at IS NULL AND d.reachable = 1)
                      AND NOT EXISTS (SELECT 1 FROM devices d WHERE d.user_id = u.id
                                        AND d.revoked_at IS NULL AND d.is_default = 1))",
        )
        .fetch_one(&self.pool)
        .await?;
        if done {
            return Ok(());
        }
        let mut tx = self.begin_write().await?;
        let has_admin: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin' AND state != 'revoked')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !has_admin {
            sqlx::query(
                "INSERT INTO users (id, name, role, state, created_at)
                 VALUES (?, ?, 'admin', 'active', ?)",
            )
            .bind(UserId::generate().as_str())
            .bind(OWNER_NAME)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        let admin = admin_id(&mut tx).await?;
        sqlx::query("UPDATE devices SET user_id = ? WHERE user_id IS NULL")
            .bind(admin.as_str())
            .execute(&mut *tx)
            .await?;
        ensure_default(&mut tx, &admin).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn user(&self, id: &UserId) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, name, role, state, cert, created_at, revoked_at FROM users WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(user_from_row).transpose()
    }

    pub async fn users(&self) -> Result<Vec<User>> {
        sqlx::query(
            "SELECT id, name, role, state, cert, created_at, revoked_at FROM users
             ORDER BY created_at, rowid",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(user_from_row)
        .collect()
    }

    pub async fn find_user(&self, name_or_id: &str) -> Result<User> {
        let row = sqlx::query(
            "SELECT id, name, role, state, cert, created_at, revoked_at FROM users
             WHERE (id = ?1 OR name = ?1) AND state != 'revoked'
             ORDER BY id = ?1 DESC LIMIT 1",
        )
        .bind(name_or_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(user_from_row)
            .transpose()?
            .ok_or_else(|| StoreError::UnknownUser(name_or_id.to_owned()))
    }

    pub async fn admin(&self) -> Result<User> {
        let mut conn = self.pool.acquire().await?;
        let id = admin_id(&mut conn).await?;
        drop(conn);
        self.user(&id).await?.ok_or_else(|| StoreError::UnknownUser("admin".into()))
    }

    pub async fn peer(&self, device: &DeviceId) -> Result<Option<Peer>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT u.id, u.role, u.state FROM devices d JOIN users u ON u.id = d.user_id
             WHERE d.id = ? AND d.revoked_at IS NULL AND u.state != 'revoked'",
        )
        .bind(device.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(user, role, state)| {
            Ok(Peer {
                device: device.clone(),
                user: parse_user(user)?,
                role: parse_role(&role)?,
                state: parse_state(&state)?,
            })
        })
        .transpose()
    }

    pub async fn links(&self) -> Result<Vec<Link>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT from_user, to_user, created_at FROM links ORDER BY created_at, from_user",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(from, to, created_at)| {
                Ok(Link { from: parse_user(from)?, to: parse_user(to)?, created_at })
            })
            .collect()
    }

    pub async fn concerned(&self, user: &UserId) -> Result<Vec<DeviceId>> {
        let mut conn = self.pool.acquire().await?;
        concerned_in(&mut conn, user).await
    }

    pub async fn directory(
        &self,
        me: &UserId,
        known: &Known,
        after: Option<&UserId>,
    ) -> Result<DirectoryPage> {
        let mut conn = self.pool.acquire().await?;
        let admin = admin_id(&mut conn).await?;
        let rows = sqlx::query(
            "SELECT u.id, u.name, u.role, u.state, u.cert, u.created_at, u.revoked_at,
                    EXISTS (SELECT 1 FROM links l WHERE (l.from_user = ?1 AND l.to_user = u.id)
                                                     OR (l.from_user = u.id AND l.to_user = ?1))
                        AS linked,
                    EXISTS (SELECT 1 FROM muted_services m
                            WHERE (m.user_id = ?1 AND m.service_id = u.id)
                               OR (m.user_id = u.id AND m.service_id = ?1)) AS muted
             FROM users u
             WHERE u.id = ?1 OR u.id = ?2
                OR (u.state = 'active' AND EXISTS (
                        SELECT 1 FROM links l WHERE (l.from_user = ?1 AND l.to_user = u.id)
                                                 OR (l.from_user = u.id AND l.to_user = ?1)))
                OR (?1 = ?2 AND u.state = 'active' AND u.role = 'service')
             ORDER BY u.id = ?1 DESC, u.id = ?2 DESC, u.created_at, u.rowid",
        )
        .bind(me.as_str())
        .bind(admin.as_str())
        .fetch_all(&mut *conn)
        .await?;

        let skip = after
            .and_then(|a| rows.iter().position(|r| r.get::<&str, _>("id") == a.as_str()))
            .map_or(0, |i| i + 1);
        let total = rows.len();

        let mut used = 0usize;
        let mut users: Vec<DirectoryUser> = Vec::new();
        let mut next = None;
        for (i, row) in rows.into_iter().enumerate().skip(skip) {
            let linked: bool = row.get("linked");
            let muted: bool = row.get("muted");
            let user = user_from_row(row)?;
            let mine = &user.id == me;
            let devices = if mine || linked {
                let query = if mine {
                    format!(
                        "SELECT {DEVICE_COLUMNS} FROM devices WHERE user_id = ? AND revoked_at IS NULL
                         ORDER BY created_at, rowid"
                    )
                } else {
                    format!(
                        "SELECT {DEVICE_COLUMNS} FROM devices
                         WHERE user_id = ? AND revoked_at IS NULL AND reachable = 1
                         ORDER BY created_at, rowid"
                    )
                };
                sqlx::query(sqlx::AssertSqlSafe(query))
                    .bind(user.id.as_str())
                    .fetch_all(&mut *conn)
                    .await?
                    .into_iter()
                    .map(|r| device_from_row(r).map(directory_device))
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };
            let mut entry = DirectoryUser {
                id: user.id,
                name: user.name,
                role: user.role,
                state: user.state,
                cert: user.cert,
                log: Vec::new(),
                more: false,
                muted: muted && !mine,
                devices,
            };

            let first = users.is_empty();
            let base = json_len(&entry) + 1;
            if !first && used + base > DIRECTORY_FRAME_BUDGET {
                next = users.last().map(|u: &DirectoryUser| u.id.clone());
                break;
            }
            used += base;

            let after_seq = known.get(&entry.id).copied().unwrap_or(0);
            let entries: Vec<(i64, Vec<u8>)> = sqlx::query_as(
                "SELECT seq, entry FROM roster_log WHERE user_id = ? AND seq > ? ORDER BY seq",
            )
            .bind(entry.id.as_str())
            .bind(i64::try_from(after_seq).unwrap_or(i64::MAX))
            .fetch_all(&mut *conn)
            .await?;
            for (seq, raw) in entries {
                let log = LogEntry { seq: seq as u64, entry: decode_signed(&raw)? };
                let len = json_len(&log) + 1;
                if used + len > DIRECTORY_FRAME_BUDGET && !(first && entry.log.is_empty()) {
                    entry.more = true;
                    break;
                }
                used += len;
                entry.log.push(log);
            }
            let full = entry.more;
            users.push(entry);
            if full && i + 1 < total {
                next = users.last().map(|u| u.id.clone());
                break;
            }
        }
        Ok(DirectoryPage { admin, users, next })
    }

    pub async fn admin_users(&self) -> Result<Vec<AdminUser>> {
        let users: Vec<User> =
            self.users().await?.into_iter().filter(|u| u.state != UserState::Revoked).collect();
        let mut out = Vec::with_capacity(users.len());
        for u in users {
            let devices = self
                .user_devices(&u.id)
                .await?
                .into_iter()
                .map(|d| AdminDevice {
                    id: d.id,
                    name: d.name,
                    kind: d.kind,
                    reachable: d.reachable,
                    default: d.is_default,
                    features: d.features,
                    online: false,
                    last_seen_at: d.last_seen_at,
                })
                .collect();
            let links: Vec<String> = sqlx::query_scalar(
                "SELECT to_user FROM links WHERE from_user = ? ORDER BY created_at, to_user",
            )
            .bind(u.id.as_str())
            .fetch_all(&self.pool)
            .await?;
            let muted: Vec<String> = sqlx::query_scalar(
                "SELECT service_id FROM muted_services WHERE user_id = ? ORDER BY service_id",
            )
            .bind(u.id.as_str())
            .fetch_all(&self.pool)
            .await?;
            out.push(AdminUser {
                id: u.id,
                name: u.name,
                role: u.role,
                state: u.state,
                created_at: u.created_at,
                devices,
                links: links.into_iter().map(parse_user).collect::<Result<_>>()?,
                muted: muted.into_iter().map(parse_user).collect::<Result<_>>()?,
            });
        }
        Ok(out)
    }

    pub async fn last_seq(&self, user: &UserId) -> Result<u64> {
        let mut conn = self.pool.acquire().await?;
        last_seq_in(&mut conn, user).await
    }

    pub async fn open_invitations(&self, now: i64) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM invitations WHERE used_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u64)
    }

    pub async fn user_counts(&self) -> Result<Vec<(Role, UserState, u64)>> {
        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT role, state, COUNT(*) FROM users GROUP BY role, state")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|(r, s, n)| Ok((parse_role(&r)?, parse_state(&s)?, n as u64)))
            .collect()
    }

    pub async fn admin_init(&self, cert: &SignedBlob, entry: &SignedBlob) -> Result<UserId> {
        check_signed(cert)?;
        check_signed(entry)?;
        let mut tx = self.begin_write().await?;
        let admin = admin_id(&mut tx).await?;
        let has_cert: bool = sqlx::query_scalar("SELECT cert IS NOT NULL FROM users WHERE id = ?")
            .bind(admin.as_str())
            .fetch_one(&mut *tx)
            .await?;
        if has_cert || last_seq_in(&mut tx, &admin).await? != 0 {
            return Err(StoreError::Invalid("le certificat de l'admin est déjà publié"));
        }
        sqlx::query("UPDATE users SET cert = ? WHERE id = ?")
            .bind(encode_signed(cert))
            .bind(admin.as_str())
            .execute(&mut *tx)
            .await?;
        insert_entry(&mut tx, &admin, 1, entry).await?;
        tx.commit().await?;
        Ok(admin)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invite_user(
        &self,
        by: &DeviceId,
        name: &str,
        links: &[UserId],
        token_hash: &[u8; 32],
        max_open: u64,
        now: i64,
        expires_at: i64,
    ) -> Result<UserId> {
        let mut tx = self.begin_write().await?;
        require_anchor(&mut tx).await?;
        check_open_invitations(&mut tx, max_open, now).await?;
        let user = UserId::generate();
        insert_user(&mut tx, &user, name, Role::Member, UserState::Pending, None, now).await?;
        for other in links {
            let target = user_in(&mut tx, other).await?;
            if target.state == UserState::Revoked || target.role == Role::Service {
                return Err(StoreError::Invalid("lien vers une personne révoquée ou un service"));
            }
            insert_link(&mut tx, &user, other, now).await?;
            insert_link(&mut tx, other, &user, now).await?;
        }
        insert_invitation(&mut tx, token_hash, &user, by, now, expires_at).await?;
        tx.commit().await?;
        Ok(user)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reinvite_user(
        &self,
        by: &DeviceId,
        user: &UserId,
        token_hash: &[u8; 32],
        max_open: u64,
        now: i64,
        expires_at: i64,
    ) -> Result<Vec<DeviceId>> {
        let mut tx = self.begin_write().await?;
        require_anchor(&mut tx).await?;
        let target = user_in(&mut tx, user).await?;
        if target.role != Role::Member || target.state == UserState::Revoked {
            return Err(StoreError::Invalid("seul un membre non révoqué se réinvite"));
        }
        sqlx::query("DELETE FROM invitations WHERE user_id = ?")
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        check_open_invitations(&mut tx, max_open, now).await?;
        let devices = active_devices_in(&mut tx, user).await?;
        for d in &devices {
            revoke_in(&mut tx, d, now).await?;
        }
        sqlx::query("DELETE FROM roster_log WHERE user_id = ?")
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE users SET state = 'pending', cert = NULL WHERE id = ?")
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        insert_invitation(&mut tx, token_hash, user, by, now, expires_at).await?;
        tx.commit().await?;
        Ok(devices)
    }

    pub async fn redeem_invitation(
        &self,
        token_hash: &[u8; 32],
        name: &str,
        public_key: &[u8; 32],
        now: i64,
    ) -> Result<InviteRedeemed> {
        let mut tx = self.begin_write().await?;
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT i.user_id, i.created_by FROM invitations i
             JOIN devices d ON d.id = i.created_by
             JOIN users u ON u.id = i.user_id
             WHERE i.token_hash = ? AND i.used_at IS NULL AND i.expires_at > ?
               AND d.revoked_at IS NULL AND u.state = 'pending'",
        )
        .bind(&token_hash[..])
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let (user, inviter) = row.ok_or(StoreError::PairingInvalid)?;
        let (user, inviter) = (parse_user(user)?, parse_device(inviter)?);
        if !active_devices_in(&mut tx, &user).await?.is_empty() {
            return Err(StoreError::PairingInvalid);
        }
        let device = insert_device(
            &mut tx,
            &DeviceId::generate(),
            &user,
            name,
            public_key,
            NewProfile::FIRST,
            now,
        )
        .await?;
        sqlx::query("UPDATE invitations SET used_at = ?, device_id = ? WHERE token_hash = ?")
            .bind(now)
            .bind(device.id.as_str())
            .bind(&token_hash[..])
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(InviteRedeemed { device, user, inviter })
    }

    pub async fn certify_user(&self, user: &UserId, cert: &SignedBlob) -> Result<()> {
        check_signed(cert)?;
        let mut tx = self.begin_write().await?;
        let target = user_in(&mut tx, user).await?;
        if target.role == Role::Admin || target.state == UserState::Revoked {
            return Err(StoreError::Invalid("personne révoquée, ou admin (admin_init)"));
        }
        if active_devices_in(&mut tx, user).await?.is_empty() {
            return Err(StoreError::Invalid("aucun appareil à certifier"));
        }
        sqlx::query("UPDATE users SET cert = ?, state = 'active' WHERE id = ?")
            .bind(encode_signed(cert))
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn publish_entry(
        &self,
        publisher: &DeviceId,
        user: &UserId,
        seq: u64,
        entry: &SignedBlob,
        log_quota: u64,
    ) -> Result<()> {
        check_signed(entry)?;
        let mut tx = self.begin_write().await?;
        let peer = peer_in(&mut tx, publisher).await?;
        let target = user_in(&mut tx, user).await?;
        if target.state != UserState::Active || target.cert.is_none() {
            return Err(if target.role == Role::Admin {
                StoreError::NoAnchor
            } else {
                StoreError::Forbidden("personne pas encore certifiée")
            });
        }
        let allowed = if &peer.user == user {
            target.role != Role::Service
        } else {
            peer.role == Role::Admin && target.role == Role::Service
        };
        if !allowed {
            return Err(StoreError::Forbidden("journal d'une autre personne"));
        }
        let next = last_seq_in(&mut tx, user).await? + 1;
        if seq != next {
            return Err(StoreError::Conflict(next));
        }
        if target.role != Role::Admin {
            let used: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(LENGTH(entry)), 0) FROM roster_log WHERE user_id = ?",
            )
            .bind(user.as_str())
            .fetch_one(&mut *tx)
            .await?;
            let size = encode_signed(entry).len() as u64;
            if (used as u64).saturating_add(size) > log_quota {
                return Err(StoreError::UserQuota("journal de roster"));
            }
        }
        insert_entry(&mut tx, user, seq, entry).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_device(
        &self,
        actor: &DeviceId,
        target: &DeviceId,
        changes: &DeviceChanges,
    ) -> Result<UserId> {
        if changes.features.is_some() && actor != target {
            return Err(StoreError::Invalid("seul l'appareil déclare ses capacités"));
        }
        if changes.features.as_ref().is_some_and(|f| f.len() > MAX_FEATURES) {
            return Err(StoreError::Invalid("trop de capacités"));
        }
        let mut tx = self.begin_write().await?;
        let peer = peer_in(&mut tx, actor).await?;
        let device = active_device_in(&mut tx, target).await?;
        if device.user_id != peer.user && !peer.is_admin() {
            return Err(StoreError::Forbidden("appareil d'une autre personne"));
        }
        let owner = user_in(&mut tx, &device.user_id).await?;

        let kind = match changes.kind {
            Some(DeviceKind::Unknown) | None => device.kind,
            Some(k) => Some(k),
        };
        let mut reachable = changes.reachable.unwrap_or(device.reachable);
        let mut is_default = changes.default.unwrap_or(device.is_default);
        let forced = kind == Some(DeviceKind::Cli) || owner.role == Role::Service;
        if forced {
            reachable = false;
            is_default = false;
        }
        if device.is_default && !is_default && !forced {
            let others: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM devices
                 WHERE user_id = ? AND revoked_at IS NULL AND is_default = 1 AND id != ?",
            )
            .bind(device.user_id.as_str())
            .bind(target.as_str())
            .fetch_one(&mut *tx)
            .await?;
            if others == 0 {
                return Err(StoreError::Invalid("il faut garder au moins un appareil par défaut"));
            }
        }
        let features = match &changes.features {
            Some(f) => {
                let mut f: Vec<&str> = f.iter().map(Feature::as_str).collect();
                f.sort_unstable();
                f.dedup();
                f.join(",")
            }
            None => device.features.iter().map(Feature::as_str).collect::<Vec<_>>().join(","),
        };
        sqlx::query(
            "UPDATE devices SET kind = ?, reachable = ?, is_default = ?, features = ? WHERE id = ?",
        )
        .bind(kind.map(DeviceKind::as_str))
        .bind(reachable)
        .bind(is_default)
        .bind(features)
        .bind(target.as_str())
        .execute(&mut *tx)
        .await?;
        ensure_default(&mut tx, &device.user_id).await?;
        tx.commit().await?;
        Ok(device.user_id)
    }

    pub async fn revoke_device_by(
        &self,
        actor: &DeviceId,
        target: &DeviceId,
        now: i64,
    ) -> Result<UserId> {
        if actor == target {
            return Err(StoreError::Invalid("un appareil ne se révoque pas lui-même"));
        }
        let mut tx = self.begin_write().await?;
        let peer = peer_in(&mut tx, actor).await?;
        let device = active_device_in(&mut tx, target).await?;
        if device.user_id != peer.user && !peer.is_admin() {
            return Err(StoreError::Forbidden("appareil d'une autre personne"));
        }
        revoke_in(&mut tx, target, now).await?;
        ensure_default(&mut tx, &device.user_id).await?;
        tx.commit().await?;
        Ok(device.user_id)
    }

    pub async fn set_link(
        &self,
        from: &UserId,
        to: &UserId,
        allowed: bool,
        now: i64,
    ) -> Result<()> {
        if from == to {
            return Err(StoreError::Invalid("lien d'une personne vers elle-même"));
        }
        let mut tx = self.begin_write().await?;
        let (a, b) = (user_in(&mut tx, from).await?, user_in(&mut tx, to).await?);
        if a.state == UserState::Revoked || b.state == UserState::Revoked {
            return Err(StoreError::Invalid("personne révoquée"));
        }
        if b.role == Role::Service {
            return Err(StoreError::Invalid(
                "un service ne reçoit rien : il est la source du lien",
            ));
        }
        let both = a.role != Role::Service;
        if allowed {
            insert_link(&mut tx, from, to, now).await?;
            if both {
                insert_link(&mut tx, to, from, now).await?;
            }
        } else {
            delete_link(&mut tx, from, to).await?;
            if both {
                delete_link(&mut tx, to, from).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn mute_service(&self, user: &UserId, service: &UserId, muted: bool) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let s = user_in(&mut tx, service).await?;
        let linked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM links WHERE from_user = ? AND to_user = ?)",
        )
        .bind(service.as_str())
        .bind(user.as_str())
        .fetch_one(&mut *tx)
        .await?;
        if s.role != Role::Service || !linked {
            return Err(StoreError::Invalid("aucun service lié à cette personne"));
        }
        let query = if muted {
            "INSERT OR IGNORE INTO muted_services (user_id, service_id) VALUES (?, ?)"
        } else {
            "DELETE FROM muted_services WHERE user_id = ? AND service_id = ?"
        };
        sqlx::query(query).bind(user.as_str()).bind(service.as_str()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn rename_user(&self, user: &UserId, name: &str) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let target = user_in(&mut tx, user).await?;
        if target.state == UserState::Revoked {
            return Err(StoreError::Invalid("personne révoquée"));
        }
        let res = sqlx::query("UPDATE users SET name = ? WHERE id = ?")
            .bind(name)
            .bind(user.as_str())
            .execute(&mut *tx)
            .await;
        match res {
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                return Err(StoreError::NameTaken(name.to_owned()));
            }
            r => r?,
        };
        tx.commit().await?;
        Ok(())
    }

    pub async fn revoke_user(&self, user: &UserId, now: i64) -> Result<UserRevoked> {
        let mut tx = self.begin_write().await?;
        let target = user_in(&mut tx, user).await?;
        if target.role == Role::Admin {
            return Err(StoreError::Invalid("l'admin ne se révoque pas"));
        }
        if target.state == UserState::Revoked {
            return Err(StoreError::UnknownUser(user.to_string()));
        }
        let notify = concerned_in(&mut tx, user).await?;
        let devices = active_devices_in(&mut tx, user).await?;
        for d in &devices {
            revoke_in(&mut tx, d, now).await?;
        }
        let blobs: Vec<String> = sqlx::query_scalar(
            "SELECT b.id FROM blobs b JOIN devices d ON d.id = b.owner_id WHERE d.user_id = ?",
        )
        .bind(user.as_str())
        .fetch_all(&mut *tx)
        .await?;
        for id in &blobs {
            sqlx::query("DELETE FROM blobs WHERE id = ?").bind(id).execute(&mut *tx).await?;
        }
        for query in [
            "DELETE FROM links WHERE from_user = ?1 OR to_user = ?1",
            "DELETE FROM muted_services WHERE user_id = ?1 OR service_id = ?1",
            "DELETE FROM invitations WHERE user_id = ?1",
        ] {
            sqlx::query(query).bind(user.as_str()).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE users SET state = 'revoked', revoked_at = ? WHERE id = ?")
            .bind(now)
            .bind(user.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let blobs = blobs
            .into_iter()
            .map(|b| b.parse().map_err(|e: ot_proto::InvalidBlobId| sqlx::Error::Decode(e.into())))
            .collect::<Result<_, _>>()?;
        Ok(UserRevoked { devices, blobs, notify })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_service(
        &self,
        user: &UserId,
        name: &str,
        cert: &SignedBlob,
        device: &DeviceId,
        public_key: &[u8; 32],
        device_name: &str,
        now: i64,
    ) -> Result<Device> {
        check_signed(cert)?;
        let mut tx = self.begin_write().await?;
        require_anchor(&mut tx).await?;
        insert_user(&mut tx, user, name, Role::Service, UserState::Active, Some(cert), now).await?;
        let d =
            insert_device(&mut tx, device, user, device_name, public_key, NewProfile::SERVICE, now)
                .await?;
        tx.commit().await?;
        Ok(d)
    }

    pub async fn add_service_device(
        &self,
        user: &UserId,
        device: &DeviceId,
        public_key: &[u8; 32],
        name: &str,
        now: i64,
    ) -> Result<Device> {
        let mut tx = self.begin_write().await?;
        let target = user_in(&mut tx, user).await?;
        if target.role != Role::Service || target.state != UserState::Active {
            return Err(StoreError::Invalid("pas une personne de service active"));
        }
        let d = insert_device(&mut tx, device, user, name, public_key, NewProfile::SERVICE, now)
            .await?;
        tx.commit().await?;
        Ok(d)
    }

    pub async fn admin_reset(&self) -> Result<u64> {
        let mut tx = self.begin_write().await?;
        let admin = admin_id(&mut tx).await?;
        sqlx::query("UPDATE users SET cert = NULL WHERE id = ?")
            .bind(admin.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM roster_log").execute(&mut *tx).await?;
        sqlx::query("DELETE FROM invitations").execute(&mut *tx).await?;
        let n = sqlx::query(
            "UPDATE users SET state = 'pending', cert = NULL WHERE id != ? AND state != 'revoked'",
        )
        .bind(admin.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(n)
    }
}

fn directory_device(d: Device) -> DirectoryDevice {
    DirectoryDevice {
        id: d.id,
        name: d.name,
        kind: d.kind,
        reachable: d.reachable,
        default: d.is_default,
        features: d.features,
        online: false,
    }
}

fn user_from_row(row: sqlx::sqlite::SqliteRow) -> Result<User> {
    let cert: Option<Vec<u8>> = row.get("cert");
    Ok(User {
        id: parse_user(row.get("id"))?,
        name: row.get("name"),
        role: parse_role(row.get::<&str, _>("role"))?,
        state: parse_state(row.get::<&str, _>("state"))?,
        cert: cert.as_deref().map(decode_signed).transpose()?,
        created_at: row.get("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}

fn json_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).expect("type du protocole sérialisable").len()
}

fn encode_signed(blob: &SignedBlob) -> Vec<u8> {
    serde_json::to_vec(blob).expect("SignedBlob est sérialisable")
}

fn decode_signed(bytes: &[u8]) -> Result<SignedBlob> {
    serde_json::from_slice(bytes).map_err(|e| sqlx::Error::Decode(e.into()).into())
}

fn check_signed(blob: &SignedBlob) -> Result<()> {
    if blob.is_empty() || blob.len() > MAX_SIGNED_BYTES {
        return Err(StoreError::Invalid("objet signé vide ou trop gros"));
    }
    Ok(())
}

async fn user_in(conn: &mut sqlx::SqliteConnection, id: &UserId) -> Result<User> {
    let row = sqlx::query(
        "SELECT id, name, role, state, cert, created_at, revoked_at FROM users WHERE id = ?",
    )
    .bind(id.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    row.map(user_from_row).transpose()?.ok_or_else(|| StoreError::UnknownUser(id.to_string()))
}

async fn peer_in(conn: &mut sqlx::SqliteConnection, device: &DeviceId) -> Result<Peer> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT u.id, u.role, u.state FROM devices d JOIN users u ON u.id = d.user_id
         WHERE d.id = ? AND d.revoked_at IS NULL AND u.state != 'revoked'",
    )
    .bind(device.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let (user, role, state) = row.ok_or_else(|| StoreError::UnknownDevice(device.to_string()))?;
    Ok(Peer {
        device: device.clone(),
        user: parse_user(user)?,
        role: parse_role(&role)?,
        state: parse_state(&state)?,
    })
}

async fn active_device_in(conn: &mut sqlx::SqliteConnection, id: &DeviceId) -> Result<Device> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ? AND revoked_at IS NULL"
    )))
    .bind(id.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    row.map(device_from_row).transpose()?.ok_or_else(|| StoreError::UnknownDevice(id.to_string()))
}

async fn active_devices_in(
    conn: &mut sqlx::SqliteConnection,
    user: &UserId,
) -> Result<Vec<DeviceId>> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM devices WHERE user_id = ? AND revoked_at IS NULL")
            .bind(user.as_str())
            .fetch_all(&mut *conn)
            .await?;
    ids.into_iter().map(parse_device).collect()
}

async fn concerned_in(conn: &mut sqlx::SqliteConnection, user: &UserId) -> Result<Vec<DeviceId>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM devices WHERE revoked_at IS NULL AND user_id IN (
             SELECT ?1
             UNION SELECT to_user FROM links WHERE from_user = ?1
             UNION SELECT from_user FROM links WHERE to_user = ?1
             UNION SELECT id FROM users WHERE role = 'admin' AND state != 'revoked')",
    )
    .bind(user.as_str())
    .fetch_all(&mut *conn)
    .await?;
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(id.clone())).map(parse_device).collect()
}

async fn require_anchor(conn: &mut sqlx::SqliteConnection) -> Result<()> {
    let ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users
                        WHERE role = 'admin' AND state = 'active' AND cert IS NOT NULL)",
    )
    .fetch_one(&mut *conn)
    .await?;
    if ok { Ok(()) } else { Err(StoreError::NoAnchor) }
}

async fn check_open_invitations(
    conn: &mut sqlx::SqliteConnection,
    max_open: u64,
    now: i64,
) -> Result<()> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invitations WHERE used_at IS NULL AND expires_at > ?",
    )
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    if n as u64 >= max_open {
        return Err(StoreError::UserQuota("invitations en attente"));
    }
    Ok(())
}

async fn insert_user(
    conn: &mut sqlx::SqliteConnection,
    id: &UserId,
    name: &str,
    role: Role,
    state: UserState,
    cert: Option<&SignedBlob>,
    now: i64,
) -> Result<()> {
    let res = sqlx::query(
        "INSERT INTO users (id, name, role, state, cert, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.as_str())
    .bind(name)
    .bind(role.as_str())
    .bind(state.as_str())
    .bind(cert.map(encode_signed))
    .bind(now)
    .execute(&mut *conn)
    .await;
    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(if e.message().contains("users.id") {
                StoreError::IdTaken
            } else {
                StoreError::NameTaken(name.to_owned())
            })
        }
        Err(e) => Err(e.into()),
    }
}

async fn insert_link(
    conn: &mut sqlx::SqliteConnection,
    from: &UserId,
    to: &UserId,
    now: i64,
) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO links (from_user, to_user, created_at) VALUES (?, ?, ?)")
        .bind(from.as_str())
        .bind(to.as_str())
        .bind(now)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn delete_link(conn: &mut sqlx::SqliteConnection, from: &UserId, to: &UserId) -> Result<()> {
    sqlx::query("DELETE FROM links WHERE from_user = ? AND to_user = ?")
        .bind(from.as_str())
        .bind(to.as_str())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn insert_invitation(
    conn: &mut sqlx::SqliteConnection,
    token_hash: &[u8; 32],
    user: &UserId,
    by: &DeviceId,
    now: i64,
    expires_at: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO invitations (token_hash, user_id, created_by, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&token_hash[..])
    .bind(user.as_str())
    .bind(by.as_str())
    .bind(now)
    .bind(expires_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn last_seq_in(conn: &mut sqlx::SqliteConnection, user: &UserId) -> Result<u64> {
    let n: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM roster_log WHERE user_id = ?")
            .bind(user.as_str())
            .fetch_one(&mut *conn)
            .await?;
    Ok(n as u64)
}

async fn insert_entry(
    conn: &mut sqlx::SqliteConnection,
    user: &UserId,
    seq: u64,
    entry: &SignedBlob,
) -> Result<()> {
    let seq = i64::try_from(seq).map_err(|_| StoreError::Invalid("seq trop grand"))?;
    let res = sqlx::query("INSERT INTO roster_log (user_id, seq, entry) VALUES (?, ?, ?)")
        .bind(user.as_str())
        .bind(seq)
        .bind(encode_signed(entry))
        .execute(&mut *conn)
        .await;
    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(StoreError::Conflict(seq as u64 + 1))
        }
        Err(e) => Err(e.into()),
    }
}
