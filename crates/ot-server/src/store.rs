//! Accès à la base SQLite du relais : appareils, messages en attente,
//! livraisons, blobs (étape 5) et purge ; personnes, réseau et invitations
//! (étape 10) dans [`crate::users`]. Toutes les fonctions prennent l'heure
//! courante en paramètre (`now`, en millisecondes) pour rester testables.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use ot_proto::{BlobId, DeviceId, DeviceKind, Feature, Role, UserId, UserState};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, Row, SqlitePool};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("aucun appareil {0:?}")]
    UnknownDevice(String),
    #[error("destinataire inconnu ou révoqué : {0}")]
    UnknownRecipient(DeviceId),
    #[error("le nom {0:?} est déjà pris par un appareil actif")]
    NameTaken(String),
    #[error("cette clé publique est déjà enregistrée")]
    KeyTaken,
    #[error("cet identifiant d'appareil est déjà utilisé")]
    IdTaken,
    #[error("msg_id déjà utilisé par un autre appareil")]
    MsgIdConflict,
    #[error("jeton d'appairage inconnu, expiré ou déjà utilisé")]
    PairingInvalid,
    #[error("aucun destinataire actif")]
    NoRecipient,
    #[error("quota du relais atteint")]
    QuotaExceeded,
    #[error("fichier inconnu, expiré ou supprimé")]
    UnknownBlob,
    /// Destinataire hors réseau (étape 10).
    #[error("destinataire hors réseau : {0}")]
    NotAllowed(DeviceId),
    /// Opération interdite à cette personne (rôle, état, appartenance).
    #[error("interdit : {0}")]
    Forbidden(&'static str),
    /// Requête incohérente (trop de destinataires, dernier appareil par
    /// défaut, lien vers soi…).
    #[error("{0}")]
    Invalid(&'static str),
    /// Quota de la personne atteint (appareils, blobs, invitations).
    #[error("quota de la personne atteint : {0}")]
    UserQuota(&'static str),
    #[error("aucune personne {0:?}")]
    UnknownUser(String),
    #[error("{0:?} désigne plusieurs appareils actifs : donner l'identifiant")]
    Ambiguous(String),
    /// Écriture concurrente du journal de roster.
    #[error("seq attendu : {0}")]
    Conflict(u64),
    /// L'admin n'a pas encore publié son certificat (`admin_init`).
    #[error("l'admin n'a pas encore de certificat")]
    NoAnchor,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub public_key: [u8; 32],
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
    /// Personne de l'appareil (étape 10), pour toujours.
    pub user_id: UserId,
    /// Catégorie déclarée ; `None` : devinée au nom par les clients.
    pub kind: Option<DeviceKind>,
    pub reachable: bool,
    pub is_default: bool,
    pub features: Vec<Feature>,
}

/// Colonnes lues par [`device_from_row`].
pub(crate) const DEVICE_COLUMNS: &str = "id, name, public_key, created_at, last_seen_at, \
     revoked_at, user_id, kind, reachable, is_default, features";

/// Profil d'un nouvel appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NewProfile {
    pub reachable: bool,
    pub is_default: bool,
}

impl NewProfile {
    /// Appareil ajouté par appairage : joignable, pas par défaut.
    pub const PAIRED: Self = Self { reachable: true, is_default: false };
    /// Premier appareil d'une personne invitée.
    pub const FIRST: Self = Self { reachable: true, is_default: true };
    /// Appareil de service : ne reçoit jamais rien.
    pub const SERVICE: Self = Self { reachable: false, is_default: false };
}

/// Message à stocker ; `envelope` est l'enveloppe sérialisée, jamais relue.
#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    pub msg_id: &'a str,
    pub sender: &'a DeviceId,
    /// Vide : les autres appareils actifs de la personne de l'expéditeur.
    pub to: &'a [DeviceId],
    pub envelope: &'a [u8],
    pub expires_at: i64,
    /// Octets en attente de livraison permis à la personne de l'expéditeur,
    /// celui-ci compris (étape 10 : membre ou service ; l'admin n'a pas de
    /// limite propre). `u64::MAX` : sans limite.
    pub pending_quota: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inserted {
    pub seq: i64,
    /// `true` si le `msg_id` existait déjà (envoi rejoué).
    pub duplicate: bool,
    /// Destinataires résolus, à réveiller.
    pub recipients: Vec<DeviceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub seq: i64,
    pub envelope: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeStats {
    /// Messages acquittés par tous leurs destinataires.
    pub acked: u64,
    /// Messages arrivés en fin de vie.
    pub expired: u64,
    /// Jetons d'appairage expirés, utilisés ou non.
    pub pairings: u64,
    /// Invitations expirées, utilisées ou non (étape 10).
    pub invitations: u64,
}

/// Appareil créé par un jeton d'appairage (ou d'invitation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redeemed {
    pub device: Device,
    /// Appareil qui a créé le jeton.
    pub sponsor: DeviceId,
}

/// Blob à créer : géométrie du chiffré et destinataires.
#[derive(Debug, Clone)]
pub struct NewBlob<'a> {
    pub id: &'a BlobId,
    pub owner: &'a DeviceId,
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    /// Vide : les autres appareils actifs de la personne du propriétaire.
    pub to: &'a [DeviceId],
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub id: BlobId,
    pub owner: DeviceId,
    pub size: u64,
    pub chunk_size: u32,
    pub chunks: u32,
    pub created_at: i64,
    pub expires_at: i64,
    pub completed_at: Option<i64>,
    pub recipients: Vec<DeviceId>,
}

impl Blob {
    pub fn is_recipient(&self, device: &DeviceId) -> bool {
        self.recipients.contains(device)
    }
}

#[derive(Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
}

impl Store {
    /// Ouvre (ou crée) la base au chemin donné et applique les migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .synchronous(SqliteSynchronous::Normal);
        // Le mode WAL est enregistré dans le fichier : il se règle une fois,
        // ici, sur une connexion seule. Demandé à chaque connexion du pool, il
        // pouvait échouer aussitôt en SQLITE_BUSY quand le pool en ouvrait une
        // pendant une écriture : changer de journal demande un verrou exclusif
        // que busy_timeout ne fait pas attendre (vu sous Windows en CI).
        let conn = opts
            .clone()
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(SqliteJournalMode::Wal)
            .connect()
            .await?;
        conn.close().await?;
        Self::connect(opts, 4).await
    }

    /// Base en mémoire, pour les tests.
    pub async fn in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?;
        // Une seule connexion : chaque connexion en mémoire a sa propre base.
        Self::connect(opts, 1).await
    }

    async fn connect(opts: SqliteConnectOptions, max: u32) -> Result<Self> {
        let opts = opts.foreign_keys(true).busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(max)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let store = Self { pool };
        store.adopt_devices(crate::now_ms()).await?;
        Ok(store)
    }

    /// Transaction d'écriture. `BEGIN IMMEDIATE` prend le verrou d'écriture
    /// dès le début : une transaction différée qui lit puis écrit échouerait
    /// aussitôt (SQLITE_BUSY, sans attente) si une autre connexion a écrit
    /// entre-temps, la purge par exemple.
    pub(crate) async fn begin_write(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    // --- Appareils ---------------------------------------------------------

    /// Ajoute un appareil à l'admin (`ot-server device add`, tests).
    pub async fn add_device(&self, name: &str, public_key: &[u8; 32], now: i64) -> Result<Device> {
        let mut tx = self.begin_write().await?;
        let admin = admin_id(&mut tx).await?;
        let d = insert_device(
            &mut tx,
            &DeviceId::generate(),
            &admin,
            name,
            public_key,
            NewProfile::PAIRED,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(d)
    }

    /// Ajoute un appareil à une personne donnée (tests, recours).
    pub async fn add_device_for(
        &self,
        user: &UserId,
        name: &str,
        public_key: &[u8; 32],
        now: i64,
    ) -> Result<Device> {
        let mut tx = self.begin_write().await?;
        let d = insert_device(
            &mut tx,
            &DeviceId::generate(),
            user,
            name,
            public_key,
            NewProfile::PAIRED,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(d)
    }

    /// Enregistre un appareil sous l'identifiant qu'il a choisi (QR inversé :
    /// il le connaît avant d'être enregistré), dans la personne de son
    /// parrain. Au-delà de `max_devices` appareils actifs pour un membre :
    /// [`StoreError::UserQuota`].
    pub async fn register_device(
        &self,
        sponsor: &DeviceId,
        id: &DeviceId,
        name: &str,
        public_key: &[u8; 32],
        max_devices: usize,
        now: i64,
    ) -> Result<Device> {
        let mut tx = self.begin_write().await?;
        let user = sponsor_user(&mut tx, sponsor).await?;
        check_device_quota(&mut tx, &user, max_devices).await?;
        let d =
            insert_device(&mut tx, id, &user, name, public_key, NewProfile::PAIRED, now).await?;
        tx.commit().await?;
        Ok(d)
    }

    pub async fn device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLUMNS} FROM devices WHERE id = ?"
        )))
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(device_from_row).transpose()
    }

    /// Tous les appareils, révoqués compris, par date de création.
    pub async fn devices(&self) -> Result<Vec<Device>> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLUMNS} FROM devices ORDER BY created_at, rowid"
        )))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(device_from_row)
        .collect()
    }

    /// Appareils actifs d'une personne, par date de création.
    pub async fn user_devices(&self, user: &UserId) -> Result<Vec<Device>> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DEVICE_COLUMNS} FROM devices WHERE user_id = ? AND revoked_at IS NULL
             ORDER BY created_at, rowid"
        )))
        .bind(user.as_str())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(device_from_row)
        .collect()
    }

    /// Révoque un appareil actif désigné par son nom ou son identifiant, et
    /// supprime ses livraisons en attente. Un nom porté par plusieurs
    /// appareils actifs (de personnes différentes) est refusé ; un
    /// identifiant exact passe avant un nom.
    pub async fn revoke_device(&self, name_or_id: &str, now: i64) -> Result<Device> {
        let mut tx = self.begin_write().await?;
        // L'identifiant exact d'abord : un nom qui imite l'identifiant d'un
        // autre appareil (choisi par un membre) ne bloque pas sa révocation.
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM devices WHERE revoked_at IS NULL
               AND (id = ?1 OR (name = ?1 AND NOT EXISTS (
                        SELECT 1 FROM devices WHERE id = ?1 AND revoked_at IS NULL)))",
        )
        .bind(name_or_id)
        .fetch_all(&mut *tx)
        .await?;
        let id = match ids.as_slice() {
            [] => return Err(StoreError::UnknownDevice(name_or_id.to_owned())),
            [id] => parse_device(id.clone())?,
            _ => return Err(StoreError::Ambiguous(name_or_id.to_owned())),
        };
        revoke_in(&mut tx, &id, now).await?;
        let (user, _) = device_user(&mut tx, &id).await?;
        ensure_default(&mut tx, &user).await?;
        tx.commit().await?;
        Ok(self.device(&id).await?.expect("l'appareil vient d'être lu"))
    }

    pub async fn touch_device(&self, id: &DeviceId, now: i64) -> Result<()> {
        sqlx::query("UPDATE devices SET last_seen_at = ? WHERE id = ?")
            .bind(now)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- Appairage ----------------------------------------------------------

    /// Enregistre un jeton d'appairage (par son SHA-256) créé par `sponsor`.
    pub async fn create_pairing(
        &self,
        sponsor: &DeviceId,
        token_hash: &[u8; 32],
        now: i64,
        expires_at: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO pairings (token_hash, sponsor_id, created_at, expires_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&token_hash[..])
        .bind(sponsor.as_str())
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Consomme un jeton valide et crée l'appareil dans la personne du
    /// parrain, en une transaction. Un nom déjà pris, ou le quota d'appareils
    /// d'un membre atteint, laissent le jeton utilisable.
    pub async fn redeem_pairing(
        &self,
        token_hash: &[u8; 32],
        name: &str,
        public_key: &[u8; 32],
        max_devices: usize,
        now: i64,
    ) -> Result<Redeemed> {
        let mut tx = self.begin_write().await?;
        let sponsor: Option<(String, String)> = sqlx::query_as(
            "SELECT p.sponsor_id, d.user_id FROM pairings p JOIN devices d ON d.id = p.sponsor_id
             JOIN users u ON u.id = d.user_id
             WHERE p.token_hash = ? AND p.used_at IS NULL AND p.expires_at > ?
               AND d.revoked_at IS NULL AND u.state = 'active'",
        )
        .bind(&token_hash[..])
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let (sponsor, user) = sponsor.ok_or(StoreError::PairingInvalid)?;
        let user = parse_user(user)?;
        check_device_quota(&mut tx, &user, max_devices).await?;
        let device = insert_device(
            &mut tx,
            &DeviceId::generate(),
            &user,
            name,
            public_key,
            NewProfile::PAIRED,
            now,
        )
        .await?;
        sqlx::query("UPDATE pairings SET used_at = ?, device_id = ? WHERE token_hash = ?")
            .bind(now)
            .bind(device.id.as_str())
            .bind(&token_hash[..])
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let sponsor = sponsor
            .parse()
            .map_err(|e: ot_proto::InvalidDeviceId| sqlx::Error::Decode(e.into()))?;
        Ok(Redeemed { device, sponsor })
    }

    // --- Messages ----------------------------------------------------------

    /// Stocke un message et crée une livraison par destinataire. Un `msg_id`
    /// déjà connu du même expéditeur renvoie le `seq` existant sans rien écrire.
    pub async fn insert_message(&self, msg: NewMessage<'_>, now: i64) -> Result<Inserted> {
        let mut tx = self.begin_write().await?;

        let existing = sqlx::query("SELECT seq, sender_id FROM messages WHERE msg_id = ?")
            .bind(msg.msg_id)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(row) = existing {
            if row.get::<String, _>("sender_id") != msg.sender.as_str() {
                return Err(StoreError::MsgIdConflict);
            }
            return Ok(Inserted { seq: row.get("seq"), duplicate: true, recipients: Vec::new() });
        }

        let recipients = route(&mut tx, msg.sender, msg.to).await?;
        check_pending_quota(&mut tx, msg.sender, msg.envelope.len(), msg.pending_quota, now)
            .await?;

        let seq: i64 = sqlx::query_scalar(
            "INSERT INTO messages (msg_id, sender_id, created_at, expires_at, size, envelope)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING seq",
        )
        .bind(msg.msg_id)
        .bind(msg.sender.as_str())
        .bind(now)
        .bind(msg.expires_at)
        .bind(msg.envelope.len() as i64)
        .bind(msg.envelope)
        .fetch_one(&mut *tx)
        .await?;

        for id in &recipients {
            sqlx::query("INSERT INTO deliveries (seq, device_id) VALUES (?, ?)")
                .bind(seq)
                .bind(id.as_str())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(Inserted { seq, duplicate: false, recipients })
    }

    /// Messages non acquittés et non expirés pour `device`, de `seq` > `after`,
    /// dans l'ordre de `seq`.
    pub async fn pending(
        &self,
        device: &DeviceId,
        after: i64,
        limit: u32,
        now: i64,
    ) -> Result<Vec<Pending>> {
        let rows = sqlx::query(
            "SELECT m.seq, m.envelope FROM deliveries d JOIN messages m ON m.seq = d.seq
             WHERE d.device_id = ? AND d.acked_at IS NULL AND m.seq > ? AND m.expires_at > ?
             ORDER BY m.seq LIMIT ?",
        )
        .bind(device.as_str())
        .bind(after)
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Pending { seq: r.get("seq"), envelope: r.get("envelope") })
            .collect())
    }

    pub async fn pending_count(&self, device: &DeviceId, now: i64) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM deliveries d JOIN messages m ON m.seq = d.seq
             WHERE d.device_id = ? AND d.acked_at IS NULL AND m.expires_at > ?",
        )
        .bind(device.as_str())
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u64)
    }

    /// Nombre total de livraisons en attente, tous appareils confondus.
    pub async fn total_pending(&self, now: i64) -> Result<u64> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM deliveries d JOIN messages m ON m.seq = d.seq
             WHERE d.acked_at IS NULL AND m.expires_at > ?",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u64)
    }

    pub async fn mark_delivered(&self, seq: i64, device: &DeviceId, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE deliveries SET delivered_at = ?
             WHERE seq = ? AND device_id = ? AND delivered_at IS NULL",
        )
        .bind(now)
        .bind(seq)
        .bind(device.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Acquitte `seq` pour `device`. Renvoie `false` si rien n'a changé
    /// (déjà acquitté, purgé ou inconnu).
    pub async fn ack(&self, seq: i64, device: &DeviceId, now: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE deliveries SET acked_at = ?, delivered_at = COALESCE(delivered_at, ?)
             WHERE seq = ? AND device_id = ? AND acked_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(seq)
        .bind(device.as_str())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Supprime les messages expirés, puis ceux dont toutes les livraisons
    /// sont acquittées.
    pub async fn purge(&self, now: i64) -> Result<PurgeStats> {
        let mut tx = self.begin_write().await?;
        let expired = sqlx::query("DELETE FROM messages WHERE expires_at <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let acked = sqlx::query(
            "DELETE FROM messages WHERE NOT EXISTS (
                 SELECT 1 FROM deliveries d WHERE d.seq = messages.seq AND d.acked_at IS NULL
             )",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let pairings = sqlx::query("DELETE FROM pairings WHERE expires_at <= ?")
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        // Une invitation utilisée vit encore `INVITE_EXCHANGE_MS` après son
        // usage : elle relie l'invité à l'admin le temps de l'échange, même
        // si le jeton aurait expiré entre-temps.
        let invitations = sqlx::query(
            "DELETE FROM invitations
             WHERE (used_at IS NULL AND expires_at <= ?1) OR (used_at IS NOT NULL AND used_at <= ?2)",
        )
        .bind(now)
        .bind(now.saturating_sub(INVITE_EXCHANGE_MS))
        .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(PurgeStats { acked, expired, pairings, invitations })
    }

    // --- Blobs (étape 5) -----------------------------------------------------

    /// Crée un blob et ses destinataires, si les quotas le permettent : la
    /// somme des tailles des blobs vivants, celui-ci compris, ne doit pas
    /// dépasser `quota`, ni, pour ceux d'un membre ou d'un service,
    /// `user_quota`. Mêmes règles
    /// de réseau qu'un envoi. Renvoie les destinataires résolus.
    pub async fn create_blob(
        &self,
        blob: NewBlob<'_>,
        quota: u64,
        user_quota: u64,
        now: i64,
    ) -> Result<Vec<DeviceId>> {
        let mut tx = self.begin_write().await?;
        let recipients = route(&mut tx, blob.owner, blob.to).await?;
        if recipients.is_empty() {
            return Err(StoreError::NoRecipient);
        }
        let used: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(size), 0) FROM blobs WHERE expires_at > ?")
                .bind(now)
                .fetch_one(&mut *tx)
                .await?;
        if (used as u64).saturating_add(blob.size) > quota {
            return Err(StoreError::QuotaExceeded);
        }
        let (user, role) = device_user(&mut tx, blob.owner).await?;
        if role != Role::Admin {
            let mine: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(b.size), 0) FROM blobs b JOIN devices d ON d.id = b.owner_id
                 WHERE d.user_id = ? AND b.expires_at > ?",
            )
            .bind(user.as_str())
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;
            if (mine as u64).saturating_add(blob.size) > user_quota {
                return Err(StoreError::UserQuota("espace des fichiers"));
            }
        }
        sqlx::query(
            "INSERT INTO blobs (id, owner_id, size, chunk_size, chunks, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(blob.id.as_str())
        .bind(blob.owner.as_str())
        .bind(blob.size as i64)
        .bind(blob.chunk_size)
        .bind(blob.chunks)
        .bind(now)
        .bind(blob.expires_at)
        .execute(&mut *tx)
        .await?;
        for id in &recipients {
            sqlx::query("INSERT INTO blob_recipients (blob_id, device_id) VALUES (?, ?)")
                .bind(blob.id.as_str())
                .bind(id.as_str())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(recipients)
    }

    /// Blob non expiré, avec ses destinataires.
    pub async fn blob(&self, id: &BlobId, now: i64) -> Result<Option<Blob>> {
        let Some(row) = sqlx::query(
            "SELECT owner_id, size, chunk_size, chunks, created_at, expires_at, completed_at
             FROM blobs WHERE id = ? AND expires_at > ?",
        )
        .bind(id.as_str())
        .bind(now)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let recipients: Vec<String> = sqlx::query_scalar(
            "SELECT device_id FROM blob_recipients WHERE blob_id = ? ORDER BY device_id",
        )
        .bind(id.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(Blob {
            id: id.clone(),
            owner: parse_device(row.get("owner_id"))?,
            size: row.get::<i64, _>("size") as u64,
            chunk_size: row.get("chunk_size"),
            chunks: row.get("chunks"),
            created_at: row.get("created_at"),
            expires_at: row.get("expires_at"),
            completed_at: row.get("completed_at"),
            recipients: recipients.into_iter().map(parse_device).collect::<Result<_>>()?,
        }))
    }

    /// Indices des morceaux déposés, dans l'ordre.
    pub async fn blob_chunks(&self, id: &BlobId) -> Result<Vec<u32>> {
        Ok(sqlx::query_scalar("SELECT idx FROM blob_chunks WHERE blob_id = ? ORDER BY idx")
            .bind(id.as_str())
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn has_blob_chunk(&self, id: &BlobId, idx: u32) -> Result<bool> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM blob_chunks WHERE blob_id = ? AND idx = ?")
                .bind(id.as_str())
                .bind(idx)
                .fetch_one(&self.pool)
                .await?;
        Ok(n > 0)
    }

    /// Note le morceau `idx` comme déposé (idempotent) ; marque le blob
    /// complet quand tous ses morceaux sont là. Renvoie `true` si cet appel
    /// l'a rendu complet.
    pub async fn add_blob_chunk(&self, id: &BlobId, idx: u32, now: i64) -> Result<bool> {
        let mut tx = self.begin_write().await?;
        let chunks: Option<u32> = sqlx::query_scalar("SELECT chunks FROM blobs WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await?;
        let chunks = chunks.ok_or(StoreError::UnknownBlob)?;
        sqlx::query("INSERT OR IGNORE INTO blob_chunks (blob_id, idx) VALUES (?, ?)")
            .bind(id.as_str())
            .bind(idx)
            .execute(&mut *tx)
            .await?;
        let have: u32 = sqlx::query_scalar("SELECT COUNT(*) FROM blob_chunks WHERE blob_id = ?")
            .bind(id.as_str())
            .fetch_one(&mut *tx)
            .await?;
        let completed = if have >= chunks {
            sqlx::query("UPDATE blobs SET completed_at = ? WHERE id = ? AND completed_at IS NULL")
                .bind(now)
                .bind(id.as_str())
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0
        } else {
            false
        };
        tx.commit().await?;
        Ok(completed)
    }

    /// Note que `device` a tout retiré. Renvoie `true` si plus aucun
    /// destinataire actif n'attend : le blob peut être supprimé.
    pub async fn blob_done(&self, id: &BlobId, device: &DeviceId, now: i64) -> Result<bool> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE blob_recipients SET done_at = ?
             WHERE blob_id = ? AND device_id = ? AND done_at IS NULL",
        )
        .bind(now)
        .bind(id.as_str())
        .bind(device.as_str())
        .execute(&mut *tx)
        .await?;
        let waiting: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM blob_recipients r JOIN devices d ON d.id = r.device_id
             WHERE r.blob_id = ? AND r.done_at IS NULL AND d.revoked_at IS NULL",
        )
        .bind(id.as_str())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(waiting == 0)
    }

    /// Supprime un blob (ses morceaux notés et ses destinataires suivent).
    /// Renvoie `false` s'il n'existait plus.
    pub async fn delete_blob(&self, id: &BlobId) -> Result<bool> {
        let res = sqlx::query("DELETE FROM blobs WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Supprime les blobs expirés, et les blobs complets que plus aucun
    /// destinataire actif n'attend. Renvoie les identifiants supprimés, dont
    /// il reste à effacer les morceaux sur disque.
    pub async fn purge_blobs(&self, now: i64) -> Result<Vec<BlobId>> {
        let mut tx = self.begin_write().await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM blobs b WHERE b.expires_at <= ?1
                OR (b.completed_at IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM blob_recipients r JOIN devices d ON d.id = r.device_id
                    WHERE r.blob_id = b.id AND r.done_at IS NULL AND d.revoked_at IS NULL))",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            sqlx::query("DELETE FROM blobs WHERE id = ?").bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        ids.into_iter().map(parse_blob).collect()
    }

    /// Tous les identifiants de blobs connus, expirés compris.
    pub async fn blob_ids(&self) -> Result<Vec<BlobId>> {
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM blobs").fetch_all(&self.pool).await?;
        ids.into_iter().map(parse_blob).collect()
    }

    /// Octets réservés par les blobs vivants (somme des tailles chiffrées).
    pub async fn blob_bytes(&self, now: i64) -> Result<u64> {
        let n: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(size), 0) FROM blobs WHERE expires_at > ?")
                .bind(now)
                .fetch_one(&self.pool)
                .await?;
        Ok(n as u64)
    }

    /// Supprime tous les messages, livrés ou non, puis compacte la base
    /// (`VACUUM`) pour que les pages libérées ne gardent rien. Sert à la
    /// bascule vers le chiffrement (note 0014). Renvoie le nombre de messages.
    pub async fn purge_all_messages(&self) -> Result<u64> {
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM deliveries").execute(&mut *tx).await?;
        let n = sqlx::query("DELETE FROM messages").execute(&mut *tx).await?.rows_affected();
        tx.commit().await?;
        sqlx::query("VACUUM").execute(&self.pool).await?;
        Ok(n)
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

pub(crate) fn parse_device(id: String) -> Result<DeviceId> {
    id.parse().map_err(|e: ot_proto::InvalidDeviceId| sqlx::Error::Decode(e.into()).into())
}

pub(crate) fn parse_user(id: String) -> Result<UserId> {
    id.parse().map_err(|e: ot_proto::InvalidUserId| sqlx::Error::Decode(e.into()).into())
}

fn parse_blob(id: String) -> Result<BlobId> {
    id.parse().map_err(|e: ot_proto::InvalidBlobId| sqlx::Error::Decode(e.into()).into())
}

pub(crate) async fn insert_device(
    conn: &mut sqlx::SqliteConnection,
    id: &DeviceId,
    user: &UserId,
    name: &str,
    public_key: &[u8; 32],
    profile: NewProfile,
    now: i64,
) -> Result<Device> {
    // Le premier appareil joignable d'une personne sans appareil par défaut
    // le devient (appareil ajouté à l'admin après la migration, par exemple).
    let mut profile = profile;
    if profile.reachable && !profile.is_default {
        profile.is_default = needs_default(conn, user).await?;
    }
    let res = sqlx::query(
        "INSERT INTO devices (id, name, public_key, created_at, user_id, reachable, is_default)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.as_str())
    .bind(name)
    .bind(&public_key[..])
    .bind(now)
    .bind(user.as_str())
    .bind(profile.reachable)
    .bind(profile.is_default)
    .execute(&mut *conn)
    .await;
    match res {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(if e.message().contains("public_key") {
                StoreError::KeyTaken
            } else if e.message().contains("devices.id") {
                StoreError::IdTaken
            } else {
                StoreError::NameTaken(name.to_owned())
            });
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Device {
        id: id.clone(),
        name: name.to_owned(),
        public_key: *public_key,
        created_at: now,
        last_seen_at: None,
        revoked_at: None,
        user_id: user.clone(),
        kind: None,
        reachable: profile.reachable,
        is_default: profile.is_default,
        features: Vec::new(),
    })
}

/// Révoque un appareil actif et supprime ses livraisons en attente.
pub(crate) async fn revoke_in(
    conn: &mut sqlx::SqliteConnection,
    id: &DeviceId,
    now: i64,
) -> Result<()> {
    sqlx::query("UPDATE devices SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM deliveries WHERE device_id = ? AND acked_at IS NULL")
        .bind(id.as_str())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Identifiant de l'admin non révoqué (toujours présent après l'ouverture).
pub(crate) async fn admin_id(conn: &mut sqlx::SqliteConnection) -> Result<UserId> {
    let id: Option<String> =
        sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin' AND state != 'revoked'")
            .fetch_optional(&mut *conn)
            .await?;
    parse_user(id.ok_or_else(|| StoreError::UnknownUser("admin".into()))?)
}

/// Personne et rôle d'un appareil.
pub(crate) async fn device_user(
    conn: &mut sqlx::SqliteConnection,
    device: &DeviceId,
) -> Result<(UserId, Role)> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT u.id, u.role FROM devices d JOIN users u ON u.id = d.user_id WHERE d.id = ?",
    )
    .bind(device.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let (user, role) = row.ok_or_else(|| StoreError::UnknownDevice(device.to_string()))?;
    Ok((parse_user(user)?, parse_role(&role)?))
}

/// Personne d'un parrain actif qui peut ajouter un appareil à son groupe :
/// ni service (pas de groupe), ni personne en attente.
async fn sponsor_user(conn: &mut sqlx::SqliteConnection, sponsor: &DeviceId) -> Result<UserId> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT u.id, u.role, u.state FROM devices d JOIN users u ON u.id = d.user_id
         WHERE d.id = ? AND d.revoked_at IS NULL",
    )
    .bind(sponsor.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let (user, role, state) = row.ok_or_else(|| StoreError::UnknownDevice(sponsor.to_string()))?;
    if parse_role(&role)? == Role::Service {
        return Err(StoreError::Forbidden("un service n'a pas de groupe"));
    }
    if parse_state(&state)? != UserState::Active {
        return Err(StoreError::Forbidden("personne pas encore certifiée"));
    }
    parse_user(user)
}

/// Refuse un appareil de plus pour un membre qui en a déjà `max`.
pub(crate) async fn check_device_quota(
    conn: &mut sqlx::SqliteConnection,
    user: &UserId,
    max: usize,
) -> Result<()> {
    let (role, count): (String, i64) = sqlx::query_as(
        "SELECT u.role, (SELECT COUNT(*) FROM devices d
                         WHERE d.user_id = u.id AND d.revoked_at IS NULL)
         FROM users u WHERE u.id = ?",
    )
    .bind(user.as_str())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| StoreError::UnknownUser(user.to_string()))?;
    if parse_role(&role)? == Role::Member && count as usize >= max {
        return Err(StoreError::UserQuota("appareils"));
    }
    Ok(())
}

/// Durée de l'échange d'invitation après l'usage du jeton : l'invitation
/// utilisée relie l'invité à l'admin jusqu'à ce que l'invité publie son
/// journal, au plus 48 h (purge).
pub const INVITE_EXCHANGE_MS: i64 = 48 * 3600 * 1000;

/// Refuse un message qui porterait les octets en attente de la personne de
/// l'expéditeur (membre ou service) au-delà de `quota`.
async fn check_pending_quota(
    conn: &mut sqlx::SqliteConnection,
    sender: &DeviceId,
    size: usize,
    quota: u64,
    now: i64,
) -> Result<()> {
    if quota == u64::MAX {
        return Ok(());
    }
    let (user, role) = device_user(conn, sender).await?;
    if role == Role::Admin {
        return Ok(());
    }
    let used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(m.size), 0) FROM messages m JOIN devices d ON d.id = m.sender_id
         WHERE d.user_id = ? AND m.expires_at > ?
           AND EXISTS (SELECT 1 FROM deliveries l WHERE l.seq = m.seq AND l.acked_at IS NULL)",
    )
    .bind(user.as_str())
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    if (used as u64).saturating_add(size as u64) > quota {
        return Err(StoreError::UserQuota("messages en attente"));
    }
    Ok(())
}

/// Vrai si `user` (ni service ni révoqué) n'a aucun appareil actif par
/// défaut.
async fn needs_default(conn: &mut sqlx::SqliteConnection, user: &UserId) -> Result<bool> {
    let needs: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users u WHERE u.id = ?1 AND u.role != 'service'
                          AND u.state != 'revoked')
            AND NOT EXISTS (SELECT 1 FROM devices
                            WHERE user_id = ?1 AND revoked_at IS NULL AND is_default = 1)",
    )
    .bind(user.as_str())
    .fetch_one(&mut *conn)
    .await?;
    Ok(needs)
}

/// Une personne (hors service) qui a des appareils joignables en garde au
/// moins un par défaut : le plus ancien le devient si aucun ne l'est (après
/// la migration, ou la révocation du dernier appareil par défaut).
pub(crate) async fn ensure_default(conn: &mut sqlx::SqliteConnection, user: &UserId) -> Result<()> {
    if !needs_default(conn, user).await? {
        return Ok(());
    }
    sqlx::query(
        "UPDATE devices SET is_default = 1 WHERE id = (
             SELECT id FROM devices WHERE user_id = ? AND revoked_at IS NULL AND reachable = 1
             ORDER BY created_at, rowid LIMIT 1)",
    )
    .bind(user.as_str())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub(crate) fn parse_role(s: &str) -> Result<Role> {
    s.parse().map_err(|()| sqlx::Error::Decode(format!("rôle inconnu : {s:?}").into()).into())
}

pub(crate) fn parse_state(s: &str) -> Result<UserState> {
    s.parse().map_err(|()| sqlx::Error::Decode(format!("état inconnu : {s:?}").into()).into())
}

/// Destinataires d'un envoi ou d'un fichier, selon le réseau (étape 10,
/// note 0052). `to` vide : les autres appareils actifs de la personne de
/// l'expéditeur. Sinon, chaque appareil nommé doit être actif et joignable
/// par cette personne ; l'expéditeur et les doublons sont ignorés, comme
/// avant l'étape 10. Au premier refus, rien n'est retenu.
pub(crate) async fn route(
    conn: &mut sqlx::SqliteConnection,
    sender: &DeviceId,
    to: &[DeviceId],
) -> Result<Vec<DeviceId>> {
    if to.len() > ot_proto::MAX_RECIPIENTS {
        return Err(StoreError::Invalid("plus de 32 destinataires"));
    }
    // Un expéditeur révoqué (par la CLI, sa session encore ouverte) n'envoie
    // plus rien : `UnknownDevice`, et la session se ferme.
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT u.id, u.role, u.state FROM devices d JOIN users u ON u.id = d.user_id
         WHERE d.id = ? AND d.revoked_at IS NULL",
    )
    .bind(sender.as_str())
    .fetch_optional(&mut *conn)
    .await?;
    let (user, role, state) = row.ok_or_else(|| StoreError::UnknownDevice(sender.to_string()))?;
    let (role, state) = (parse_role(&role)?, parse_state(&state)?);
    if state == UserState::Revoked {
        return Err(StoreError::Forbidden("personne révoquée"));
    }

    if to.is_empty() {
        if role == Role::Service {
            return Err(StoreError::Invalid(
                "un service n'a pas de groupe : nommer les destinataires",
            ));
        }
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM devices WHERE user_id = ? AND revoked_at IS NULL AND id != ?
             ORDER BY created_at, rowid",
        )
        .bind(&user)
        .bind(sender.as_str())
        .fetch_all(&mut *conn)
        .await?;
        return ids.into_iter().map(parse_device).collect();
    }

    let mut out: Vec<DeviceId> = Vec::with_capacity(to.len());
    for id in to {
        if id == sender || out.contains(id) {
            continue;
        }
        let target: Option<(String, bool, String, String, bool, bool, bool)> = sqlx::query_as(
            "SELECT d.user_id, d.reachable, u.role, u.state,
                    EXISTS (SELECT 1 FROM links l WHERE l.from_user = ?2 AND l.to_user = d.user_id),
                    EXISTS (SELECT 1 FROM muted_services m
                            WHERE m.user_id = d.user_id AND m.service_id = ?2),
                    EXISTS (SELECT 1 FROM invitations i WHERE i.used_at IS NOT NULL
                            AND ((i.created_by = ?3 AND i.device_id = d.id)
                              OR (i.created_by = d.id AND i.device_id = ?3))
                            AND NOT EXISTS (SELECT 1 FROM roster_log r
                                            WHERE r.user_id = i.user_id))
             FROM devices d JOIN users u ON u.id = d.user_id
             WHERE d.id = ?1 AND d.revoked_at IS NULL",
        )
        .bind(id.as_str())
        .bind(&user)
        .bind(sender.as_str())
        .fetch_optional(&mut *conn)
        .await?;
        let Some((to_user, reachable, to_role, to_state, linked, muted, invited)) = target else {
            return Err(StoreError::UnknownRecipient(id.clone()));
        };
        let (to_role, to_state) = (parse_role(&to_role)?, parse_state(&to_state)?);
        // Une invitation relie ses deux appareils le temps de l'échange : la
        // demande scellée de l'invité passe avant la certification, la réponse
        // de l'admin même s'il a décoché leur lien. L'échange finit quand
        // l'invité publie son journal (ou quand l'invitation expire).
        let allowed = if invited {
            true
        } else if state == UserState::Pending {
            return Err(StoreError::Forbidden(
                "personne en attente : seul l'appareil qui l'a invitée est joignable",
            ));
        } else if to_state != UserState::Active || to_role == Role::Service {
            false
        } else if to_user == user {
            role != Role::Service
        } else {
            linked && reachable && !(role == Role::Service && muted)
        };
        if !allowed {
            return Err(StoreError::NotAllowed(id.clone()));
        }
        out.push(id.clone());
    }
    Ok(out)
}

pub(crate) fn device_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Device> {
    let id: String = row.get("id");
    let key: Vec<u8> = row.get("public_key");
    let kind: Option<String> = row.get("kind");
    let features: String = row.get("features");
    Ok(Device {
        id: parse_device(id)?,
        name: row.get("name"),
        public_key: key
            .try_into()
            .map_err(|_| sqlx::Error::Decode("clé publique de taille invalide".into()))?,
        created_at: row.get("created_at"),
        last_seen_at: row.get("last_seen_at"),
        revoked_at: row.get("revoked_at"),
        user_id: parse_user(row.get("user_id"))?,
        kind: kind.and_then(|k| k.parse().ok()),
        reachable: row.get("reachable"),
        is_default: row.get("is_default"),
        // Une capacité illisible (écrite par une version future) est ignorée.
        features: features.split(',').filter_map(|f| f.parse().ok()).collect(),
    })
}
