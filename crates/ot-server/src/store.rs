//! Accès à la base SQLite du relais : appareils, messages en attente,
//! livraisons, blobs (étape 5) et purge. Toutes les fonctions prennent l'heure courante en
//! paramètre (`now`, en millisecondes) pour rester testables.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use ot_proto::{BlobId, DeviceId};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

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
}

/// Message à stocker ; `envelope` est l'enveloppe sérialisée, jamais relue.
#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    pub msg_id: &'a str,
    pub sender: &'a DeviceId,
    /// Vide : tous les autres appareils actifs.
    pub to: &'a [DeviceId],
    pub envelope: &'a [u8],
    pub expires_at: i64,
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
}

/// Appareil créé par un jeton d'appairage.
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
    /// Vide : tous les autres appareils actifs.
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
    pool: SqlitePool,
}

impl Store {
    /// Ouvre (ou crée) la base au chemin donné et applique les migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
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
        Ok(Self { pool })
    }

    /// Transaction d'écriture. `BEGIN IMMEDIATE` prend le verrou d'écriture
    /// dès le début : une transaction différée qui lit puis écrit échouerait
    /// aussitôt (SQLITE_BUSY, sans attente) si une autre connexion a écrit
    /// entre-temps, la purge par exemple.
    async fn begin_write(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    // --- Appareils ---------------------------------------------------------

    pub async fn add_device(&self, name: &str, public_key: &[u8; 32], now: i64) -> Result<Device> {
        let mut conn = self.pool.acquire().await?;
        insert_device(&mut conn, &DeviceId::generate(), name, public_key, now).await
    }

    /// Enregistre un appareil sous l'identifiant qu'il a choisi (QR inversé :
    /// il le connaît avant d'être enregistré).
    pub async fn register_device(
        &self,
        id: &DeviceId,
        name: &str,
        public_key: &[u8; 32],
        now: i64,
    ) -> Result<Device> {
        let mut conn = self.pool.acquire().await?;
        insert_device(&mut conn, id, name, public_key, now).await
    }

    pub async fn device(&self, id: &DeviceId) -> Result<Option<Device>> {
        let row = sqlx::query(
            "SELECT id, name, public_key, created_at, last_seen_at, revoked_at
             FROM devices WHERE id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(device_from_row).transpose()
    }

    /// Tous les appareils, révoqués compris, par date de création.
    pub async fn devices(&self) -> Result<Vec<Device>> {
        sqlx::query(
            "SELECT id, name, public_key, created_at, last_seen_at, revoked_at
             FROM devices ORDER BY created_at, rowid",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(device_from_row)
        .collect()
    }

    /// Révoque un appareil actif désigné par son nom ou son identifiant, et
    /// supprime ses livraisons en attente.
    pub async fn revoke_device(&self, name_or_id: &str, now: i64) -> Result<Device> {
        let mut tx = self.begin_write().await?;
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM devices WHERE (id = ?1 OR name = ?1) AND revoked_at IS NULL",
        )
        .bind(name_or_id)
        .fetch_optional(&mut *tx)
        .await?;
        let id = id.ok_or_else(|| StoreError::UnknownDevice(name_or_id.to_owned()))?;
        sqlx::query("UPDATE devices SET revoked_at = ? WHERE id = ?")
            .bind(now)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM deliveries WHERE device_id = ? AND acked_at IS NULL")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let id = DeviceId::from_str(&id).map_err(|e| sqlx::Error::Decode(e.into()))?;
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

    /// Consomme un jeton valide et crée l'appareil, en une transaction. Un nom
    /// déjà pris laisse le jeton utilisable.
    pub async fn redeem_pairing(
        &self,
        token_hash: &[u8; 32],
        name: &str,
        public_key: &[u8; 32],
        now: i64,
    ) -> Result<Redeemed> {
        let mut tx = self.begin_write().await?;
        let sponsor: Option<String> = sqlx::query_scalar(
            "SELECT p.sponsor_id FROM pairings p JOIN devices d ON d.id = p.sponsor_id
             WHERE p.token_hash = ? AND p.used_at IS NULL AND p.expires_at > ?
               AND d.revoked_at IS NULL",
        )
        .bind(&token_hash[..])
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let sponsor = sponsor.ok_or(StoreError::PairingInvalid)?;
        let device = insert_device(&mut tx, &DeviceId::generate(), name, public_key, now).await?;
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

        let active: Vec<String> =
            sqlx::query_scalar("SELECT id FROM devices WHERE revoked_at IS NULL AND id != ?")
                .bind(msg.sender.as_str())
                .fetch_all(&mut *tx)
                .await?;
        let recipients = resolve_recipients(msg.sender, msg.to, &active)?;

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
        tx.commit().await?;
        Ok(PurgeStats { acked, expired, pairings })
    }

    // --- Blobs (étape 5) -----------------------------------------------------

    /// Crée un blob et ses destinataires, si le quota le permet : la somme des
    /// tailles des blobs vivants, celui-ci compris, ne doit pas dépasser
    /// `quota`. Renvoie les destinataires résolus.
    pub async fn create_blob(
        &self,
        blob: NewBlob<'_>,
        quota: u64,
        now: i64,
    ) -> Result<Vec<DeviceId>> {
        let mut tx = self.begin_write().await?;
        let active: Vec<String> =
            sqlx::query_scalar("SELECT id FROM devices WHERE revoked_at IS NULL AND id != ?")
                .bind(blob.owner.as_str())
                .fetch_all(&mut *tx)
                .await?;
        let recipients = resolve_recipients(blob.owner, blob.to, &active)?;
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

/// Destinataires d'un envoi : `to` vide vaut tous les appareils actifs (hors
/// expéditeur) ; sinon chacun doit être actif, sans doublon ni expéditeur.
fn resolve_recipients(
    sender: &DeviceId,
    to: &[DeviceId],
    active: &[String],
) -> Result<Vec<DeviceId>> {
    if to.is_empty() {
        return Ok(active.iter().filter_map(|id| id.parse().ok()).collect());
    }
    let mut out = Vec::with_capacity(to.len());
    for id in to {
        if id == sender || out.contains(id) {
            continue;
        }
        if !active.iter().any(|a| a == id.as_str()) {
            return Err(StoreError::UnknownRecipient(id.clone()));
        }
        out.push(id.clone());
    }
    Ok(out)
}

fn parse_device(id: String) -> Result<DeviceId> {
    id.parse().map_err(|e: ot_proto::InvalidDeviceId| sqlx::Error::Decode(e.into()).into())
}

fn parse_blob(id: String) -> Result<BlobId> {
    id.parse().map_err(|e: ot_proto::InvalidBlobId| sqlx::Error::Decode(e.into()).into())
}

async fn insert_device(
    conn: &mut sqlx::SqliteConnection,
    id: &DeviceId,
    name: &str,
    public_key: &[u8; 32],
    now: i64,
) -> Result<Device> {
    let res =
        sqlx::query("INSERT INTO devices (id, name, public_key, created_at) VALUES (?, ?, ?, ?)")
            .bind(id.as_str())
            .bind(name)
            .bind(&public_key[..])
            .bind(now)
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
    })
}

fn device_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Device> {
    let id: String = row.get("id");
    let key: Vec<u8> = row.get("public_key");
    Ok(Device {
        id: id.parse().map_err(|e: ot_proto::InvalidDeviceId| sqlx::Error::Decode(e.into()))?,
        name: row.get("name"),
        public_key: key
            .try_into()
            .map_err(|_| sqlx::Error::Decode("clé publique de taille invalide".into()))?,
        created_at: row.get("created_at"),
        last_seen_at: row.get("last_seen_at"),
        revoked_at: row.get("revoked_at"),
    })
}
