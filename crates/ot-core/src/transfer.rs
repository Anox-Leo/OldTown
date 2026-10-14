//! Transferts de fichiers (étape 5, note 0026) : dépôt et retrait par
//! morceaux chiffrés, en parallèle, avec reprise. Les requêtes HTTP sont
//! signées et n'ouvrent pas de session (note 0009) : un transfert ne coupe
//! jamais la réception.
//!
//! L'appli fournit la source ([`ChunkSource`]) et la destination
//! ([`ChunkSink`]) : un fichier pour la CLI, un `ContentResolver` pour
//! Android. Leurs méthodes peuvent bloquer ; elles tournent hors du runtime.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use hyper::Method;
use ot_crypto::{CHUNK_TAG_LEN, FILE_CHUNK_SIZE, FileCipher, chunk_count, chunk_len, sealed_size};
use ot_proto::http::{BLOBS_PATH, BlobCreated, BlobStatus, CreateBlob, HttpError};
use ot_proto::{BlobId, DeviceId};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::client::Backoff;
use crate::files::{FileMeta, StoredFile, signed_headers};
use crate::http::{HttpConn, HttpTarget};
use crate::types::ClientError;
use crate::{Identity, TlsRoots};

/// Source d'un fichier à envoyer. Les lectures peuvent bloquer.
pub trait ChunkSource: Send + Sync + 'static {
    /// Taille du fichier, en octets.
    fn size(&self) -> u64;
    /// Lit `len` octets à partir de `offset` (moins seulement en fin de
    /// fichier).
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, String>;
}

/// Destination d'un fichier reçu. Les morceaux arrivent dans l'ordre ; le
/// fichier ne doit devenir visible qu'à [`ChunkSink::finish`], appelé une
/// fois le SHA-256 vérifié.
pub trait ChunkSink: Send + Sync + 'static {
    fn write_at(&self, offset: u64, data: &[u8]) -> Result<(), String>;
    fn finish(&self) -> Result<(), String>;
    /// Transfert abandonné : effacer ce qui a été écrit.
    fn abort(&self);
}

/// Progression d'un transfert, en octets de clair.
pub trait Progress: Send + Sync + 'static {
    fn progress(&self, done: u64, total: u64);
}

impl<F: Fn(u64, u64) + Send + Sync + 'static> Progress for F {
    fn progress(&self, done: u64, total: u64) {
        self(done, total)
    }
}

/// Réglages des transferts.
#[derive(Debug, Clone)]
pub struct TransferConfig {
    /// Morceaux en vol en même temps.
    pub parallel: usize,
    /// Taille des morceaux en clair des fichiers envoyés.
    pub chunk_size: u32,
    /// Sans aucun progrès pendant ce délai, le transfert échoue (il se
    /// reprendra en le relançant).
    pub stall_timeout: Duration,
    /// Attente entre deux essais après une erreur passagère.
    pub retry: Backoff,
    /// Durée maximale d'une requête.
    pub request_timeout: Duration,
    /// Intervalle minimal entre deux rapports de progression.
    pub progress_interval: Duration,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            parallel: 4,
            chunk_size: FILE_CHUNK_SIZE,
            stall_timeout: Duration::from_secs(10 * 60),
            retry: Backoff { initial: Duration::from_millis(500), max: Duration::from_secs(30) },
            request_timeout: Duration::from_secs(120),
            progress_interval: Duration::from_millis(250),
        }
    }
}

/// Fichier à envoyer.
#[derive(Debug, Clone)]
pub struct OutgoingFile {
    /// Identifie la source pour reprendre un dépôt interrompu (chemin, URI).
    pub source_id: String,
    pub name: String,
    pub mime: String,
    /// Destinataires (membres du roster) ; vide : tous les autres.
    pub to: Vec<DeviceId>,
}

/// Dernier progrès d'un transfert, partagé par ses tâches.
#[derive(Clone)]
struct Stall(Arc<Mutex<Instant>>);

impl Stall {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
    fn touch(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }
    fn since(&self) -> Duration {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).elapsed()
    }
}

/// Rapports de progression espacés d'au moins `progress_interval`, sauf le
/// dernier.
struct Reporter {
    progress: Arc<dyn Progress>,
    total: u64,
    done: AtomicU64,
    last: Mutex<Option<Instant>>,
    every: Duration,
}

impl Reporter {
    fn new(progress: Arc<dyn Progress>, total: u64, done: u64, every: Duration) -> Self {
        Self { progress, total, done: AtomicU64::new(done), last: Mutex::new(None), every }
    }

    fn add(&self, n: u64) {
        let done = self.done.fetch_add(n, Ordering::Relaxed) + n;
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if done >= self.total || last.is_none_or(|t| t.elapsed() >= self.every) {
            *last = Some(Instant::now());
            drop(last);
            self.progress.progress(done, self.total);
        }
    }

    fn start(&self) {
        self.progress.progress(self.done.load(Ordering::Relaxed), self.total);
    }
}

/// Accès à l'API des fichiers pour un appareil.
#[derive(Clone)]
pub(crate) struct Transfers {
    target: Result<HttpTarget, String>,
    device_id: DeviceId,
    identity: Identity,
    pub(crate) config: TransferConfig,
}

impl Transfers {
    pub(crate) fn new(
        server_url: &str,
        tls: &TlsRoots,
        device_id: DeviceId,
        identity: Identity,
        config: TransferConfig,
    ) -> Self {
        Self { target: HttpTarget::new(server_url, tls), device_id, identity, config }
    }

    fn conn(&self) -> Result<HttpConn, ClientError> {
        self.target.clone().map(HttpConn::new).map_err(ClientError::Invalid)
    }

    /// Requête signée. Les erreurs passagères (réseau, `5xx`) sont réessayées
    /// tant que le transfert a progressé depuis moins de `stall_timeout`.
    async fn call(
        &self,
        conn: &mut HttpConn,
        method: Method,
        path: &str,
        body: Bytes,
        stall: &Stall,
    ) -> Result<Bytes, ClientError> {
        let mut attempt = 0;
        loop {
            let headers = signed_headers(
                &self.identity,
                &self.device_id,
                method.as_str(),
                path,
                &body,
                now_ms(),
            );
            let failure = match conn
                .request(method.clone(), path, headers, body.clone(), self.config.request_timeout)
                .await
            {
                Ok((status, bytes)) if (200..300).contains(&status) => {
                    stall.touch();
                    return Ok(bytes);
                }
                Ok((404, _)) => return Err(ClientError::Expired),
                Ok((status, bytes)) => {
                    let message = serde_json::from_slice::<HttpError>(&bytes)
                        .map(|e| e.message)
                        .unwrap_or_else(|_| format!("erreur HTTP {status}"));
                    if status < 500 {
                        return Err(ClientError::Transfer(format!(
                            "refusé par le relais : {message}"
                        )));
                    }
                    message
                }
                Err(e) => e,
            };
            if stall.since() >= self.config.stall_timeout {
                return Err(ClientError::Transfer(format!("relais injoignable : {failure}")));
            }
            tracing::debug!(path, attempt, error = %failure, "requête de fichier à réessayer");
            tokio::time::sleep(self.config.retry.delay(attempt)).await;
            attempt = attempt.saturating_add(1);
        }
    }

    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        conn: &mut HttpConn,
        method: Method,
        path: &str,
        body: Bytes,
        stall: &Stall,
    ) -> Result<T, ClientError> {
        let bytes = self.call(conn, method, path, body, stall).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| ClientError::Transfer(format!("réponse du relais illisible : {e}")))
    }

    /// Crée le blob d'un fichier de `size` octets en clair.
    pub(crate) async fn create(
        &self,
        size: u64,
        to: &[DeviceId],
    ) -> Result<(BlobCreated, u32), ClientError> {
        let chunk_size = self.config.chunk_size;
        let invalid = || ClientError::Invalid("fichier trop gros".into());
        let chunks = chunk_count(size, chunk_size).ok_or_else(invalid)?;
        let req = CreateBlob {
            size: sealed_size(size, chunk_size).ok_or_else(invalid)?,
            chunk_size: chunk_size + CHUNK_TAG_LEN,
            chunks,
            to: to.to_vec(),
        };
        // Invariant : `CreateBlob` est toujours sérialisable.
        let body = Bytes::from(serde_json::to_vec(&req).expect("CreateBlob sérialisable"));
        let mut conn = self.conn()?;
        let created = self.json(&mut conn, Method::POST, BLOBS_PATH, body, &Stall::new()).await?;
        Ok((created, chunks))
    }

    pub(crate) async fn status(&self, blob: &BlobId) -> Result<BlobStatus, ClientError> {
        let mut conn = self.conn()?;
        let path = format!("{BLOBS_PATH}/{blob}");
        self.json(&mut conn, Method::GET, &path, Bytes::new(), &Stall::new()).await
    }

    pub(crate) async fn cancel(&self, blob: &BlobId) -> Result<(), ClientError> {
        let mut conn = self.conn()?;
        let path = format!("{BLOBS_PATH}/{blob}");
        match self.call(&mut conn, Method::DELETE, &path, Bytes::new(), &Stall::new()).await {
            Ok(_) | Err(ClientError::Expired) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn done(&self, blob: &BlobId) -> Result<(), ClientError> {
        let mut conn = self.conn()?;
        let path = format!("{BLOBS_PATH}/{blob}/done");
        self.call(&mut conn, Method::POST, &path, Bytes::new(), &Stall::new()).await.map(|_| ())
    }

    /// Dépose les morceaux `missing`, `parallel` à la fois.
    pub(crate) async fn upload(
        &self,
        meta: &FileMeta,
        missing: Vec<u32>,
        source: Arc<dyn ChunkSource>,
        progress: Arc<dyn Progress>,
    ) -> Result<(), ClientError> {
        let missing_bytes: u64 = missing
            .iter()
            .filter_map(|&i| chunk_len(meta.size, meta.chunk_size, i))
            .map(u64::from)
            .sum();
        let reporter = Arc::new(Reporter::new(
            progress,
            meta.size,
            meta.size - missing_bytes,
            self.config.progress_interval,
        ));
        reporter.start();
        let queue = Arc::new(Mutex::new(VecDeque::from(missing)));
        let cipher = Arc::new(FileCipher::new(&meta.key, &meta.nonce));
        let stall = Stall::new();
        let mut tasks = JoinSet::new();
        for _ in 0..self.config.parallel.max(1) {
            let (this, queue, cipher, source, stall, reporter) = (
                self.clone(),
                queue.clone(),
                cipher.clone(),
                source.clone(),
                stall.clone(),
                reporter.clone(),
            );
            let (blob, size, chunk_size, chunks) =
                (meta.blob_id.clone(), meta.size, meta.chunk_size, meta.chunks);
            tasks.spawn(async move {
                let mut conn = this.conn()?;
                loop {
                    let Some(idx) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
                    else {
                        return Ok(());
                    };
                    let len = chunk_len(size, chunk_size, idx).unwrap_or(0);
                    let offset = u64::from(idx) * u64::from(chunk_size);
                    let src = source.clone();
                    let plain =
                        Zeroizing::new(blocking(move || src.read_at(offset, len)).await?.map_err(
                            |e| ClientError::Transfer(format!("lecture du fichier : {e}")),
                        )?);
                    if plain.len() != len as usize {
                        return Err(ClientError::Transfer(
                            "le fichier a changé pendant l'envoi".into(),
                        ));
                    }
                    let sealed = cipher
                        .seal_chunk(idx, idx + 1 == chunks, &plain)
                        .map_err(|e| ClientError::Crypto(e.to_string()))?;
                    let path = format!("{BLOBS_PATH}/{blob}/chunks/{idx}");
                    this.call(&mut conn, Method::PUT, &path, Bytes::from(sealed), &stall).await?;
                    reporter.add(u64::from(len));
                }
            });
        }
        join_all(tasks).await
    }

    /// Retire, déchiffre et écrit dans l'ordre tous les morceaux d'un
    /// fichier, puis vérifie son SHA-256. Ne finalise pas la destination.
    pub(crate) async fn download(
        &self,
        file: &StoredFile,
        sink: Arc<dyn ChunkSink>,
        progress: Arc<dyn Progress>,
    ) -> Result<(), ClientError> {
        let parallel = self.config.parallel.max(1);
        let reporter = Reporter::new(progress, file.size, 0, self.config.progress_interval);
        reporter.start();
        // Morceaux retirés mais pas encore écrits : la fenêtre borne la
        // mémoire quand un morceau en retard bloque l'écriture.
        let window = Arc::new(Semaphore::new(parallel * 2));
        let next = Arc::new(Mutex::new(0u32));
        let cipher = Arc::new(file.cipher());
        let stall = Stall::new();
        type Chunk = (u32, Zeroizing<Vec<u8>>, OwnedSemaphorePermit);
        let (tx, mut rx) = mpsc::channel::<Result<Chunk, ClientError>>(parallel * 2);
        let mut tasks = JoinSet::new();
        for _ in 0..parallel {
            let (this, window, next, cipher, stall, tx) = (
                self.clone(),
                window.clone(),
                next.clone(),
                cipher.clone(),
                stall.clone(),
                tx.clone(),
            );
            let (blob, size, chunk_size, chunks) =
                (file.blob_id.clone(), file.size, file.chunk_size, file.chunks);
            tasks.spawn(async move {
                let run = async {
                    let mut conn = this.conn()?;
                    loop {
                        // Invariant : le sémaphore n'est jamais fermé.
                        let permit = window.clone().acquire_owned().await.expect("fenêtre ouverte");
                        let idx = {
                            let mut n = next.lock().unwrap_or_else(|e| e.into_inner());
                            if *n >= chunks {
                                return Ok(());
                            }
                            *n += 1;
                            *n - 1
                        };
                        let path = format!("{BLOBS_PATH}/{blob}/chunks/{idx}");
                        let sealed =
                            this.call(&mut conn, Method::GET, &path, Bytes::new(), &stall).await?;
                        let plain = cipher
                            .open_chunk(idx, idx + 1 == chunks, &sealed)
                            .map_err(|_| ClientError::Transfer(format!("morceau {idx} altéré")))?;
                        if Some(plain.len() as u32) != chunk_len(size, chunk_size, idx) {
                            return Err(ClientError::Transfer(format!("morceau {idx} incohérent")));
                        }
                        if tx.send(Ok((idx, plain, permit))).await.is_err() {
                            return Ok(());
                        }
                    }
                };
                if let Err(e) = run.await {
                    let _ = tx.send(Err(e)).await;
                }
            });
        }
        drop(tx);

        let mut hasher = Sha256::new();
        let mut pending: BTreeMap<u32, (Zeroizing<Vec<u8>>, OwnedSemaphorePermit)> =
            BTreeMap::new();
        let mut expected = 0;
        while expected < file.chunks {
            let Some(item) = rx.recv().await else {
                return Err(ClientError::Transfer("transfert interrompu".into()));
            };
            let (idx, plain, permit) = item?;
            pending.insert(idx, (plain, permit));
            while let Some((plain, permit)) = pending.remove(&expected) {
                hasher.update(&plain);
                let offset = u64::from(expected) * u64::from(file.chunk_size);
                let (dst, len) = (sink.clone(), plain.len() as u64);
                blocking(move || dst.write_at(offset, &plain))
                    .await?
                    .map_err(|e| ClientError::Transfer(format!("écriture du fichier : {e}")))?;
                drop(permit);
                reporter.add(len);
                expected += 1;
            }
        }
        tasks.abort_all();
        let digest: [u8; 32] = hasher.finalize().into();
        if digest != file.sha256 {
            return Err(ClientError::Transfer(
                "fichier altéré : son empreinte SHA-256 ne correspond pas".into(),
            ));
        }
        Ok(())
    }
}

/// Heure courante en millisecondes, pour signer les requêtes.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// SHA-256 d'une source, lue dans l'ordre par morceaux.
pub(crate) async fn hash_source(
    source: Arc<dyn ChunkSource>,
    chunk_size: u32,
) -> Result<[u8; 32], ClientError> {
    blocking(move || {
        let size = source.size();
        let mut hasher = Sha256::new();
        let mut offset = 0;
        while offset < size {
            let len = (size - offset).min(u64::from(chunk_size)) as u32;
            let data = Zeroizing::new(source.read_at(offset, len)?);
            if data.len() != len as usize {
                return Err("fichier plus court que prévu".to_owned());
            }
            hasher.update(&*data);
            offset += u64::from(len);
        }
        Ok(hasher.finalize().into())
    })
    .await?
    .map_err(|e| ClientError::Transfer(format!("lecture du fichier : {e}")))
}

/// Exécute un appel bloquant (fichier, Kotlin) hors du runtime.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ClientError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ClientError::Transfer(format!("tâche interrompue : {e}")))
}

/// Attend toutes les tâches ; la première erreur arrête les autres.
async fn join_all(mut tasks: JoinSet<Result<(), ClientError>>) -> Result<(), ClientError> {
    while let Some(res) = tasks.join_next().await {
        let res = res.map_err(|e| ClientError::Transfer(format!("tâche interrompue : {e}")))?;
        if let Err(e) = res {
            tasks.abort_all();
            return Err(e);
        }
    }
    Ok(())
}
