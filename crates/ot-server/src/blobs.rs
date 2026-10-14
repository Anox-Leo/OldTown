//! API HTTP des fichiers (étape 5, note 0026) : dépôt et retrait de blobs
//! chiffrés, par morceaux. Chaque requête est signée par l'appareil, sans
//! session : un transfert ne coupe jamais la session WebSocket (note 0009).
//!
//! Les morceaux sont sur disque, un fichier par morceau
//! (`<blob_dir>/<blob_id>/<index>`), écrits sous un nom temporaire puis
//! renommés : un morceau est entier ou absent. La base ne note un morceau
//! qu'une fois renommé.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use ot_proto::http::{
    BLOBS_PATH, BlobCreated, BlobStatus, CreateBlob, HEADER_DEVICE, HEADER_NONCE, HEADER_SIGNATURE,
    HEADER_TIME, HTTP_WINDOW_MS, HttpError, HttpErrorCode, MAX_SEALED_CHUNK, blob_chunk_count,
    blob_chunk_len,
};
use ot_proto::{BlobId, DeviceId, b64};
use time::OffsetDateTime;

use crate::auth::{self, NonceError};
use crate::metrics::Metrics;
use crate::store::{Blob, NewBlob, StoreError};
use crate::{Relay, now_ms};

/// Surcoût d'un morceau chiffré (étiquette Poly1305), pour estimer la taille
/// en clair d'un blob à partir de sa taille chiffrée.
const CHUNK_TAG_LEN: u64 = 16;

/// Routes de l'API, à fusionner dans le routeur du relais. Corps limité à un
/// morceau chiffré, plus une marge pour le JSON de création.
pub fn router() -> Router<Relay> {
    Router::new()
        .route(BLOBS_PATH, post(create))
        .route("/v1/blobs/{id}", get(status).delete(cancel))
        .route("/v1/blobs/{id}/chunks/{n}", put(upload).get(download))
        .route("/v1/blobs/{id}/done", post(done))
        .layer(DefaultBodyLimit::max(MAX_SEALED_CHUNK as usize + 4096))
}

/// Erreur de l'API : code et message lisible, sans contenu ni secret.
#[derive(Debug)]
pub struct ApiError {
    code: HttpErrorCode,
    message: String,
}

impl ApiError {
    fn new(code: HttpErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    fn bad(message: impl Into<String>) -> Self {
        Self::new(HttpErrorCode::BadRequest, message)
    }

    fn not_found() -> Self {
        Self::new(HttpErrorCode::NotFound, "fichier inconnu, expiré ou supprimé")
    }

    fn internal(what: &'static str, e: impl std::fmt::Display) -> Self {
        tracing::error!(error = %e, "{what}");
        Self::new(HttpErrorCode::Internal, "erreur interne du relais")
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::UnknownRecipient(id) => Self::bad(format!("destinataire inconnu : {id}")),
            StoreError::NoRecipient => Self::bad("aucun destinataire actif"),
            StoreError::QuotaExceeded => Self::new(
                HttpErrorCode::QuotaExceeded,
                "espace du relais plein : réessayer quand d'autres fichiers auront été retirés",
            ),
            StoreError::UnknownBlob => Self::not_found(),
            e => Self::internal("base inaccessible", e),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.code.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(HttpError { error: self.code, message: self.message })).into_response()
    }
}

type ApiResult = Result<Response, ApiError>;

/// Vérifie la signature de la requête et renvoie l'appareil qui l'a faite.
/// Toujours le même message en cas d'échec, sauf pour l'horloge, que
/// l'utilisateur peut corriger ; le détail ne va que dans les logs.
async fn authenticate(
    relay: &Relay,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<DeviceId, ApiError> {
    let inner = &*relay.inner;
    let refuse = |reason: &'static str| {
        Metrics::inc(&inner.metrics.http_auth_failures);
        tracing::debug!(reason, path = uri.path(), "requête HTTP refusée");
        ApiError::new(HttpErrorCode::Unauthorized, "authentification refusée")
    };
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let device: DeviceId =
        header(HEADER_DEVICE).and_then(|s| s.parse().ok()).ok_or_else(|| refuse("appareil"))?;
    let time: u64 =
        header(HEADER_TIME).and_then(|s| s.parse().ok()).ok_or_else(|| refuse("heure"))?;
    let nonce = header(HEADER_NONCE)
        .and_then(b64::decode_array::<{ ot_proto::http::HTTP_NONCE_LEN }>)
        .ok_or_else(|| refuse("nonce"))?;
    let sig = header(HEADER_SIGNATURE)
        .and_then(b64::decode_array::<64>)
        .ok_or_else(|| refuse("signature"))?;

    let now = now_ms();
    if i64::try_from(time).map_or(true, |t| (t - now).abs() > HTTP_WINDOW_MS) {
        refuse("heure hors fenêtre");
        return Err(ApiError::new(
            HttpErrorCode::Unauthorized,
            "authentification refusée : l'horloge de l'appareil est décalée de plus d'une minute",
        ));
    }
    let known = inner.store.device(&device).await?;
    let Some(dev) = known.filter(|d| d.revoked_at.is_none()) else {
        return Err(refuse("appareil inconnu ou révoqué"));
    };
    if !auth::verify_http(&dev.public_key, method.as_str(), uri.path(), time, &nonce, body, &sig) {
        return Err(refuse("signature fausse"));
    }
    match inner.nonces.check(&device, nonce, now) {
        Ok(()) => Ok(device),
        Err(NonceError::Replayed) => Err(refuse("nonce rejoué")),
        Err(NonceError::Full) => {
            tracing::warn!("trop de requêtes HTTP dans la fenêtre");
            Err(ApiError::new(HttpErrorCode::Busy, "relais saturé, réessayer plus tard"))
        }
    }
}

/// Blob vivant désigné par l'URL, ou `404`.
async fn load(relay: &Relay, id: &str) -> Result<Blob, ApiError> {
    let id: BlobId = id.parse().map_err(|_| ApiError::not_found())?;
    relay.inner.store.blob(&id, now_ms()).await?.ok_or_else(ApiError::not_found)
}

fn forbidden(what: &str) -> ApiError {
    ApiError::new(HttpErrorCode::Forbidden, format!("seul {what} peut faire cette opération"))
}

fn chunk_index(blob: &Blob, n: &str) -> Result<u32, ApiError> {
    n.parse::<u32>().ok().filter(|&i| i < blob.chunks).ok_or_else(|| {
        ApiError::bad(format!("morceau {n:?} hors du fichier ({} morceaux)", blob.chunks))
    })
}

fn rfc3339(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// `POST /v1/blobs` : crée un blob, si sa géométrie est cohérente, s'il
/// tient dans la limite de taille et dans le quota.
async fn create(
    State(relay): State<Relay>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    let device = authenticate(&relay, &method, &uri, &headers, &body).await?;
    let inner = &*relay.inner;
    let req: CreateBlob = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad(format!("requête invalide : {e}")))?;
    if req.chunk_size == 0
        || req.chunk_size > MAX_SEALED_CHUNK
        || blob_chunk_count(req.size, req.chunk_size) != Some(req.chunks)
    {
        return Err(ApiError::bad("tailles incohérentes"));
    }
    // La limite porte sur le fichier en clair : on retire les étiquettes.
    let plain = req.size.saturating_sub(u64::from(req.chunks) * CHUNK_TAG_LEN);
    if plain > inner.config.max_file_size {
        Metrics::inc(&inner.metrics.blobs_rejected);
        return Err(ApiError::new(
            HttpErrorCode::TooLarge,
            format!(
                "fichier de {plain} octets, maximum {} sur ce relais",
                inner.config.max_file_size
            ),
        ));
    }

    let id = BlobId::generate();
    let now = now_ms();
    let expires_at = now + inner.config.blob_ttl.as_millis() as i64;
    let blob = NewBlob {
        id: &id,
        owner: &device,
        size: req.size,
        chunk_size: req.chunk_size,
        chunks: req.chunks,
        to: &req.to,
        expires_at,
    };
    let recipients = match inner.store.create_blob(blob, inner.config.blob_quota, now).await {
        Ok(r) => r,
        Err(e) => {
            if matches!(e, StoreError::QuotaExceeded) {
                Metrics::inc(&inner.metrics.blobs_rejected);
                tracing::warn!(device = %device, size = req.size, "quota des fichiers atteint");
            }
            return Err(e.into());
        }
    };
    Metrics::inc(&inner.metrics.blobs_created);
    tracing::info!(
        blob = %id,
        device = %device,
        size = req.size,
        chunks = req.chunks,
        recipients = recipients.len(),
        "fichier créé"
    );
    let created = BlobCreated { blob_id: id, expires_at: rfc3339(expires_at) };
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

/// `PUT /v1/blobs/{id}/chunks/{n}` : dépose un morceau (propriétaire
/// seulement, idempotent).
async fn upload(
    State(relay): State<Relay>,
    UrlPath((id, n)): UrlPath<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    let device = authenticate(&relay, &method, &uri, &headers, &body).await?;
    let inner = &*relay.inner;
    let blob = load(&relay, &id).await?;
    if blob.owner != device {
        return Err(forbidden("l'expéditeur"));
    }
    let idx = chunk_index(&blob, &n)?;
    // Invariant : `idx` < `chunks`, donc le morceau existe.
    let expected = blob_chunk_len(blob.size, blob.chunk_size, idx).unwrap_or(0);
    if body.len() != expected as usize {
        return Err(ApiError::bad(format!(
            "morceau {idx} de {} octets, {expected} attendus",
            body.len()
        )));
    }
    if inner.store.has_blob_chunk(&blob.id, idx).await? {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    inner
        .blob_dir
        .write_chunk(&blob.id, idx, body)
        .await
        .map_err(|e| ApiError::internal("morceau non écrit", e))?;
    match inner.store.add_blob_chunk(&blob.id, idx, now_ms()).await {
        Ok(complete) => {
            Metrics::inc(&inner.metrics.blob_chunks_uploaded);
            tracing::debug!(blob = %blob.id, idx, "morceau déposé");
            if complete {
                tracing::info!(blob = %blob.id, "fichier complet");
            }
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        Err(StoreError::UnknownBlob) => {
            // Annulé ou purgé pendant l'écriture : ne rien laisser sur disque.
            inner.blob_dir.remove(&blob.id).await;
            Err(ApiError::not_found())
        }
        Err(e) => Err(e.into()),
    }
}

/// `GET /v1/blobs/{id}` : état du dépôt, pour reprendre (propriétaire et
/// destinataires).
async fn status(
    State(relay): State<Relay>,
    UrlPath(id): UrlPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    let device = authenticate(&relay, &method, &uri, &headers, &body).await?;
    let blob = load(&relay, &id).await?;
    if blob.owner != device && !blob.is_recipient(&device) {
        return Err(forbidden("l'expéditeur ou un destinataire"));
    }
    let have: HashSet<u32> = relay.inner.store.blob_chunks(&blob.id).await?.into_iter().collect();
    let missing = (0..blob.chunks).filter(|i| !have.contains(i)).collect();
    Ok(Json(BlobStatus {
        blob_id: blob.id,
        size: blob.size,
        chunk_size: blob.chunk_size,
        chunks: blob.chunks,
        missing,
        complete: blob.completed_at.is_some(),
        expires_at: rfc3339(blob.expires_at),
    })
    .into_response())
}

/// `GET /v1/blobs/{id}/chunks/{n}` : retire un morceau d'un blob complet
/// (destinataires seulement).
async fn download(
    State(relay): State<Relay>,
    UrlPath((id, n)): UrlPath<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    let device = authenticate(&relay, &method, &uri, &headers, &body).await?;
    let inner = &*relay.inner;
    let blob = load(&relay, &id).await?;
    if !blob.is_recipient(&device) {
        return Err(forbidden("un destinataire"));
    }
    if blob.completed_at.is_none() {
        return Err(ApiError::new(
            HttpErrorCode::Incomplete,
            "fichier pas encore entièrement déposé",
        ));
    }
    let idx = chunk_index(&blob, &n)?;
    let data = inner
        .blob_dir
        .read_chunk(&blob.id, idx)
        .await
        .map_err(|e| ApiError::internal("morceau illisible", e))?;
    Metrics::inc(&inner.metrics.blob_chunks_downloaded);
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

/// `POST /v1/blobs/{id}/done` : un destinataire a tout reçu ; le blob est
/// supprimé quand plus aucun destinataire actif ne l'attend.
async fn done(
    State(relay): State<Relay>,
    UrlPath(id): UrlPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    let device = authenticate(&relay, &method, &uri, &headers, &body).await?;
    let inner = &*relay.inner;
    let blob = load(&relay, &id).await?;
    if !blob.is_recipient(&device) {
        return Err(forbidden("un destinataire"));
    }
    if blob.completed_at.is_none() {
        return Err(ApiError::new(
            HttpErrorCode::Incomplete,
            "fichier pas encore entièrement déposé",
        ));
    }
    if inner.store.blob_done(&blob.id, &device, now_ms()).await? {
        relay.remove_blob(&blob.id, "done").await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /v1/blobs/{id}` : l'expéditeur annule son envoi.
async fn cancel(
    State(relay): State<Relay>,
    UrlPath(id): UrlPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult {
    let device = authenticate(&relay, &method, &uri, &headers, &body).await?;
    let blob = load(&relay, &id).await?;
    if blob.owner != device {
        return Err(forbidden("l'expéditeur"));
    }
    relay.remove_blob(&blob.id, "cancelled").await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Dossier des morceaux sur disque.
#[derive(Debug, Clone)]
pub struct BlobDir {
    root: PathBuf,
}

impl BlobDir {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn dir(&self, id: &BlobId) -> PathBuf {
        // `BlobId` n'est fait que de `blob_` et d'un ULID : jamais de `/` ni `..`.
        self.root.join(id.as_str())
    }

    /// Écrit le morceau sous un nom temporaire unique, le synchronise sur
    /// disque, puis le renomme : après un arrêt brutal, il est entier ou
    /// absent.
    pub async fn write_chunk(&self, id: &BlobId, idx: u32, data: Bytes) -> io::Result<()> {
        let dir = self.dir(id);
        tokio::task::spawn_blocking(move || {
            create_private_dir(&dir)?;
            let mut suffix = [0; 8];
            getrandom::fill(&mut suffix).map_err(io::Error::other)?;
            let tmp = dir.join(format!(".{idx}.{}.tmp", u64::from_le_bytes(suffix)));
            let res = (|| {
                let mut file = create_private_file(&tmp)?;
                file.write_all(&data)?;
                file.sync_all()?;
                std::fs::rename(&tmp, dir.join(idx.to_string()))
            })();
            if res.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            res
        })
        .await
        .map_err(io::Error::other)?
    }

    pub async fn read_chunk(&self, id: &BlobId, idx: u32) -> io::Result<Vec<u8>> {
        tokio::fs::read(self.dir(id).join(idx.to_string())).await
    }

    /// Efface les morceaux d'un blob ; une erreur est journalisée, pas
    /// remontée (le nettoyage au démarrage rattrapera le dossier).
    pub async fn remove(&self, id: &BlobId) {
        match tokio::fs::remove_dir_all(self.dir(id)).await {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => tracing::error!(blob = %id, error = %e, "morceaux non effacés"),
        }
    }

    /// Au démarrage : efface les dossiers de blobs que la base ne connaît pas
    /// (supprimés pendant un arrêt brutal) et les fichiers temporaires.
    /// Seuls les noms de la forme `blob_<ULID>` sont touchés.
    pub async fn clean(&self, known: HashSet<BlobId>) -> io::Result<usize> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || clean_dir(&root, &known))
            .await
            .map_err(io::Error::other)?
    }
}

fn clean_dir(root: &Path, known: &HashSet<BlobId>) -> io::Result<usize> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let Some(id) = entry.file_name().to_str().and_then(|n| n.parse::<BlobId>().ok()) else {
            continue;
        };
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if !known.contains(&id) {
            std::fs::remove_dir_all(entry.path())?;
            removed += 1;
            continue;
        }
        for chunk in std::fs::read_dir(entry.path())? {
            let chunk = chunk?;
            if chunk.file_name().to_str().is_some_and(|n| n.ends_with(".tmp")) {
                std::fs::remove_file(chunk.path())?;
            }
        }
    }
    Ok(removed)
}

/// Dossier lisible par le seul relais (`0700` sous Unix).
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Fichier lisible par le seul relais (`0600` sous Unix).
fn create_private_file(path: &Path) -> io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)
}
