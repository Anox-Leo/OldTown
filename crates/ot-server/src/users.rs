//! Personnes, réseau, invitations et journal de roster (étape 10, note
//! 0052), côté base. Le relais tient l'autorisation (qui peut joindre qui) ;
//! les certificats et les entrées du journal sont des octets opaques qu'il
//! range et rend sans les lire : les clients les vérifient.

use std::collections::HashSet;

use ot_proto::{
    AdminDevice, AdminUser, BlobId, DeviceId, DeviceKind, DirectoryDevice, DirectoryUser, Feature,
    Known, LogEntry, MAX_FEATURES, MAX_SIGNED_BYTES, Role, SignedBlob, UserId, UserState,
};
use sqlx::Row;

use crate::store::{
    DEVICE_COLUMNS, Device, NewProfile, Result, Store, StoreError, admin_id, device_from_row,
    insert_device, parse_device, parse_role, parse_state, parse_user, revoke_in,
};

/// Taille brute des entrées de journal rendues par une trame `directory` :
/// en base64 et avec le reste de la trame, elle tient sous `MAX_FRAME_BYTES`.
const DIRECTORY_LOG_BUDGET: usize = 64 * 1024;

/// Nom de la personne `admin` créée à la migration (`ot-server user rename`).
pub const OWNER_NAME: &str = "owner";

/// Une personne.
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

/// L'appareil d'une session et sa personne, relus en base à chaque
/// décision (une personne en attente devient active pendant sa session).
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

/// Premier appareil d'une personne invitée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRedeemed {
    pub device: Device,
    pub user: UserId,
    /// Appareil de l'admin qui a créé l'invitation.
    pub inviter: DeviceId,
}

/// Ce qu'une révocation de personne laisse à faire hors de la base.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserRevoked {
    /// Appareils révoqués : sessions à fermer.
    pub devices: Vec<DeviceId>,
    /// Blobs supprimés : morceaux à effacer du disque.
    pub blobs: Vec<BlobId>,
    /// Appareils à prévenir (`directory_changed`), calculés avant de retirer
    /// les liens.
    pub notify: Vec<DeviceId>,
}

/// Changements demandés par `device_update`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceChanges {
    pub kind: Option<DeviceKind>,
    pub reachable: Option<bool>,
    pub default: Option<bool>,
    pub features: Option<Vec<Feature>>,
}

/// Lien entre deux personnes, tel que listé par `ot-server link list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: UserId,
    pub to: UserId,
    pub created_at: i64,
}

impl Store {
    /// Reprise de l'existant (étape 10) : sans admin, en crée un, `owner`,
    /// et lui rattache tout appareil sans personne. Idempotent.
    pub(crate) async fn adopt_devices(&self, now: i64) -> Result<()> {
        // Lecture d'abord : une ouverture ordinaire ne prend pas le verrou
        // d'écriture (`ot-server device list` pendant que le relais écrit).
        let done: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin' AND state != 'revoked')
                AND NOT EXISTS (SELECT 1 FROM devices WHERE user_id IS NULL)",
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
        tx.commit().await?;
        Ok(())
    }

    // --- Lecture ------------------------------------------------------------

    pub async fn user(&self, id: &UserId) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, name, role, state, cert, created_at, revoked_at FROM users WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(user_from_row).transpose()
    }

    /// Toutes les personnes, révoquées comprises, par date de création.
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

    /// Personne non révoquée désignée par son nom ou son identifiant.
    pub async fn find_user(&self, name_or_id: &str) -> Result<User> {
        let row = sqlx::query(
            "SELECT id, name, role, state, cert, created_at, revoked_at FROM users
             WHERE (id = ?1 OR name = ?1) AND state != 'revoked'",
        )
        .bind(name_or_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(user_from_row)
            .transpose()?
            .ok_or_else(|| StoreError::UnknownUser(name_or_id.to_owned()))
    }

    /// La personne non révoquée de rôle `admin`.
    pub async fn admin(&self) -> Result<User> {
        let mut conn = self.pool.acquire().await?;
        let id = admin_id(&mut conn).await?;
        drop(conn);
        self.user(&id).await?.ok_or_else(|| StoreError::UnknownUser("admin".into()))
    }

    /// Appareil actif et sa personne non révoquée.
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

    /// Tous les liens.
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

    /// Appareils actifs à prévenir quand quelque chose change pour `user` :
    /// les siens, ceux des personnes liées dans un sens ou dans l'autre, et
    /// ceux de l'admin.
    pub async fn concerned(&self, user: &UserId) -> Result<Vec<DeviceId>> {
        let mut conn = self.pool.acquire().await?;
        concerned_in(&mut conn, user).await
    }

    /// Annuaire vu par `me` : ma personne (tous mes appareils actifs), l'admin
    /// (l'ancre), et les personnes actives liées dans un sens ou dans l'autre
    /// (leurs appareils joignables seulement). L'admin non lié n'y figure que
    /// par son certificat et son journal, sans appareils. `online` reste à
    /// `false` : la session le remplit.
    pub async fn directory(
        &self,
        me: &UserId,
        known: &Known,
    ) -> Result<(UserId, Vec<DirectoryUser>)> {
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
             ORDER BY u.id = ?1 DESC, u.id = ?2 DESC, u.created_at, u.rowid",
        )
        .bind(me.as_str())
        .bind(admin.as_str())
        .fetch_all(&mut *conn)
        .await?;

        let mut budget = DIRECTORY_LOG_BUDGET;
        let mut used_any = false;
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
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

            let after = known.get(&user.id).copied().unwrap_or(0);
            let entries: Vec<(i64, Vec<u8>)> = sqlx::query_as(
                "SELECT seq, entry FROM roster_log WHERE user_id = ? AND seq > ? ORDER BY seq",
            )
            .bind(user.id.as_str())
            .bind(i64::try_from(after).unwrap_or(i64::MAX))
            .fetch_all(&mut *conn)
            .await?;
            let mut log = Vec::new();
            let mut more = false;
            for (seq, entry) in entries {
                // Une entrée au moins par trame, pour que le client avance.
                if used_any && entry.len() > budget {
                    more = true;
                    break;
                }
                budget = budget.saturating_sub(entry.len());
                used_any = true;
                log.push(LogEntry { seq: seq as u64, entry: decode_signed(&entry)? });
            }

            users.push(DirectoryUser {
                id: user.id,
                name: user.name,
                role: user.role,
                state: user.state,
                cert: user.cert,
                log,
                more,
                muted: muted && !mine,
                devices,
            });
        }
        Ok((admin, users))
    }

    /// Toutes les personnes non révoquées vues par l'admin, avec tous leurs
    /// appareils actifs, leurs liens et les services qu'elles ont coupés.
    /// `online` reste à `false` : la session le remplit.
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

    /// Dernier `seq` du journal de roster d'une personne (0 : vide).
    pub async fn last_seq(&self, user: &UserId) -> Result<u64> {
        let mut conn = self.pool.acquire().await?;
        last_seq_in(&mut conn, user).await
    }

    /// Invitations encore utilisables (ni utilisées, ni expirées).
    pub async fn open_invitations(&self, now: i64) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM invitations WHERE used_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u64)
    }

    /// Nombre de personnes par (rôle, état), pour `ot_users`.
    pub async fn user_counts(&self) -> Result<Vec<(Role, UserState, u64)>> {
        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT role, state, COUNT(*) FROM users GROUP BY role, state")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|(r, s, n)| Ok((parse_role(&r)?, parse_state(&s)?, n as u64)))
            .collect()
    }

    // --- Administration -----------------------------------------------------

    /// Publie le certificat racine de l'admin et la première entrée de son
    /// journal ; une seule fois.
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

    /// Crée une personne invitée (`pending`), ses liens symétriques vers
    /// `links` et son jeton, haché. Au plus `max_open` invitations
    /// utilisables à la fois.
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

    /// Réinvite une personne : ses appareils sont révoqués, son certificat,
    /// son journal et ses anciennes invitations oubliés, ses liens gardés ;
    /// elle repasse en attente avec un nouveau jeton. Renvoie les appareils
    /// révoqués.
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

    /// Consomme un jeton d'invitation et crée le premier appareil de la
    /// personne, en une transaction. Un nom refusé laisse le jeton
    /// utilisable.
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

    /// Enregistre le certificat d'un membre ou d'un service : une personne
    /// en attente (avec au moins un appareil) devient active ; une personne
    /// active reçoit un certificat re-signé.
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

    /// Ajoute une entrée au journal de roster de `user`, publiée par
    /// `publisher` (appareil de cette personne, ou de l'admin pour un
    /// service). `seq` doit suivre la dernière entrée.
    pub async fn publish_entry(
        &self,
        publisher: &DeviceId,
        user: &UserId,
        seq: u64,
        entry: &SignedBlob,
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
        insert_entry(&mut tx, user, seq, entry).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Règle le profil d'un appareil actif de la personne de `actor` (de
    /// n'importe qui pour l'admin). Renvoie la personne de l'appareil.
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

        // Une catégorie inconnue (version plus récente) ne change rien.
        let kind = match changes.kind {
            Some(DeviceKind::Unknown) | None => device.kind,
            Some(k) => Some(k),
        };
        let mut reachable = changes.reachable.unwrap_or(device.reachable);
        let mut is_default = changes.default.unwrap_or(device.is_default);
        if kind == Some(DeviceKind::Cli) || owner.role == Role::Service {
            reachable = false;
            is_default = false;
        }
        if device.is_default && !is_default {
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
        tx.commit().await?;
        Ok(device.user_id)
    }

    /// Révoque un appareil de la personne de `actor` (de n'importe qui pour
    /// l'admin), jamais `actor` lui-même. Renvoie la personne de l'appareil.
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
        tx.commit().await?;
        Ok(device.user_id)
    }

    /// Pose ou retire un lien : entre deux personnes, les deux sens ; depuis
    /// un service, le seul sens service → personne.
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

    /// Un membre coupe (`muted`) ou rouvre un service lié à lui.
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

    /// Révoque une personne (jamais l'admin) : ses appareils, ses liens, ses
    /// invitations, ses services coupés, ses livraisons en attente et ses
    /// blobs, en une transaction.
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

    /// Crée une personne de service, active, avec son certificat et son
    /// premier appareil (QR inversé scanné par l'admin).
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

    /// Ajoute un appareil à une personne de service active.
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

    /// Recours par SSH après la perte de tous les appareils de l'admin :
    /// oublie son certificat et son journal ; toutes les autres personnes non
    /// révoquées repassent en attente, sans certificat ni journal (à
    /// réinviter ; un service est à recréer). Renvoie le nombre de personnes
    /// remises en attente.
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

/// Profil d'annuaire d'un appareil ; `online` est rempli par la session.
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
