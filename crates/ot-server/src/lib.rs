//! Serveur relais OldTown : authentifie les appareils, stocke les enveloppes
//! et les livre à leurs destinataires. Il ne lit jamais `payload`.

mod auth;
pub mod blobs;
pub mod metrics;
mod registry;
mod session;
pub mod store;
pub mod tls;

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::auth::NonceCache;
use crate::blobs::BlobDir;
use crate::metrics::Metrics;
use crate::registry::Registry;
use crate::store::{PurgeStats, Store};

pub use crate::auth::token_hash;

#[derive(Debug, Clone)]
pub struct Config {
    /// Durée de vie d'un message non acquitté.
    pub ttl: Duration,
    pub purge_interval: Duration,
    /// Intervalle des pings WebSocket ; une session muette pendant
    /// deux intervalles et demi est fermée.
    pub ping_interval: Duration,
    /// Délai pour répondre au défi.
    pub auth_timeout: Duration,
    /// Durée de validité d'un jeton d'appairage.
    pub pairing_ttl: Duration,
    /// Intervalle de vérification du certificat TLS (rechargé s'il a changé).
    pub tls_reload_interval: Duration,
    /// Dossier des morceaux de fichiers (étape 5), créé au premier dépôt.
    pub blob_dir: PathBuf,
    /// Taille maximale d'un fichier, en clair.
    pub max_file_size: u64,
    /// Somme maximale des tailles chiffrées des fichiers en transit.
    pub blob_quota: u64,
    /// Durée de vie d'un fichier déposé.
    pub blob_ttl: Duration,
}

/// Nombre maximal de nonces HTTP retenus pendant la fenêtre.
const MAX_HTTP_NONCES: usize = 100_000;

impl Default for Config {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(7 * 24 * 3600),
            purge_interval: Duration::from_secs(10 * 60),
            ping_interval: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(30),
            pairing_ttl: Duration::from_secs(10 * 60),
            tls_reload_interval: Duration::from_secs(10 * 60),
            blob_dir: PathBuf::from("blobs"),
            max_file_size: 2 << 30,
            blob_quota: 20 << 30,
            blob_ttl: Duration::from_secs(48 * 3600),
        }
    }
}

/// État partagé du relais.
#[derive(Clone)]
pub struct Relay {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    config: Config,
    registry: Registry,
    metrics: Metrics,
    shutdown: watch::Sender<bool>,
    blob_dir: BlobDir,
    nonces: NonceCache,
}

impl Relay {
    pub fn new(store: Store, config: Config) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                blob_dir: BlobDir::new(config.blob_dir.clone()),
                nonces: NonceCache::new(MAX_HTTP_NONCES),
                config,
                registry: Registry::default(),
                metrics: Metrics::default(),
                shutdown: watch::Sender::new(false),
            }),
        }
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route(ot_proto::WS_PATH, get(ws_handler))
            .route("/healthz", get(healthz))
            .route("/metrics", get(metrics_handler))
            .merge(blobs::router())
            .with_state(self.clone())
    }

    /// Lance une purge (messages, jetons, fichiers) et met à jour les
    /// compteurs. Renvoie les chiffres des messages et des jetons.
    pub async fn purge_once(&self) -> store::Result<PurgeStats> {
        let stats = self.inner.store.purge(now_ms()).await?;
        let m = &self.inner.metrics;
        m.purged_acked.fetch_add(stats.acked, std::sync::atomic::Ordering::Relaxed);
        m.purged_expired.fetch_add(stats.expired, std::sync::atomic::Ordering::Relaxed);
        if stats != PurgeStats::default() {
            tracing::info!(
                acked = stats.acked,
                expired = stats.expired,
                pairings = stats.pairings,
                "purge"
            );
        }
        let blobs = self.inner.store.purge_blobs(now_ms()).await?;
        for id in &blobs {
            self.inner.blob_dir.remove(id).await;
        }
        if !blobs.is_empty() {
            m.blobs_purged.fetch_add(blobs.len() as u64, std::sync::atomic::Ordering::Relaxed);
            tracing::info!(blobs = blobs.len(), "purge des fichiers");
        }
        Ok(stats)
    }

    /// Supprime un blob et ses morceaux (`reason` : `done`, `cancelled`).
    pub(crate) async fn remove_blob(
        &self,
        id: &ot_proto::BlobId,
        reason: &str,
    ) -> store::Result<()> {
        if self.inner.store.delete_blob(id).await? {
            self.inner.blob_dir.remove(id).await;
            let m = &self.inner.metrics;
            Metrics::inc(if reason == "done" { &m.blobs_done } else { &m.blobs_cancelled });
            tracing::info!(blob = %id, reason, "fichier supprimé");
        }
        Ok(())
    }

    /// Efface les morceaux sur disque que la base ne connaît plus (arrêt
    /// brutal pendant une suppression) et les fichiers temporaires.
    pub async fn clean_blob_dir(&self) -> std::io::Result<usize> {
        let ids = self.inner.store.blob_ids().await.map_err(std::io::Error::other)?;
        self.inner.blob_dir.clean(ids.into_iter().collect::<HashSet<_>>()).await
    }

    pub async fn metrics_text(&self) -> store::Result<String> {
        let now = now_ms();
        let pending = self.inner.store.total_pending(now).await?;
        let blob_bytes = self.inner.store.blob_bytes(now).await?;
        Ok(self.inner.metrics.render(pending, blob_bytes))
    }

    /// Ferme la session ouverte d'un appareil (après révocation, par exemple).
    pub fn disconnect(&self, device: &ot_proto::DeviceId) {
        self.inner.registry.kick(device);
    }
}

/// Sert le relais en clair (`ws://`) sur `listener` jusqu'à ce que
/// `shutdown` se termine, en purgeant périodiquement. Les sessions ouvertes
/// sont fermées à l'arrêt.
pub async fn serve(
    relay: Relay,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    run(relay, listener, shutdown).await
}

/// Comme [`serve`], en TLS (`wss://`) ; le certificat est relu toutes les
/// `tls_reload_interval` s'il a changé sur le disque.
pub async fn serve_tls(
    relay: Relay,
    listener: TcpListener,
    tls: tls::Tls,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let reload = {
        let (tls, every) = (tls.clone(), relay.inner.config.tls_reload_interval);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
            loop {
                tick.tick().await;
                match tls.reload_if_changed() {
                    Ok(true) => tracing::info!("certificat TLS rechargé"),
                    Ok(false) => {}
                    Err(e) => tracing::error!(error = %e, "certificat TLS non rechargé"),
                }
            }
        })
    };
    let res = run(relay, tls::TlsListener::new(listener, tls)?, shutdown).await;
    reload.abort();
    res
}

async fn run<L>(
    relay: Relay,
    listener: L,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()>
where
    L: axum::serve::Listener,
    L::Addr: std::fmt::Debug,
{
    match relay.clean_blob_dir().await {
        Ok(0) => {}
        Ok(n) => tracing::info!(removed = n, "dossiers de fichiers orphelins effacés"),
        Err(e) => tracing::error!(error = %e, "nettoyage du dossier des fichiers impossible"),
    }
    let purge = {
        let relay = relay.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(relay.inner.config.purge_interval);
            loop {
                tick.tick().await;
                if let Err(e) = relay.purge_once().await {
                    tracing::error!(error = %e, "échec de la purge");
                }
            }
        })
    };
    let stop = relay.inner.shutdown.clone();
    let res = axum::serve(listener, relay.router())
        .with_graceful_shutdown(async move {
            shutdown.await;
            stop.send_replace(true);
        })
        .await;
    purge.abort();
    res
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

async fn ws_handler(State(relay): State<Relay>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(ot_proto::MAX_FRAME_BYTES)
        .max_frame_size(ot_proto::MAX_FRAME_BYTES)
        .on_upgrade(move |socket| session::run(relay, socket))
}

async fn healthz(State(relay): State<Relay>) -> Response {
    match relay.inner.store.ping().await {
        Ok(()) => "ok\n".into_response(),
        Err(e) => {
            tracing::error!(error = %e, "base inaccessible");
            (StatusCode::SERVICE_UNAVAILABLE, "base inaccessible\n").into_response()
        }
    }
}

async fn metrics_handler(State(relay): State<Relay>) -> Response {
    match relay.metrics_text().await {
        Ok(text) => ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], text)
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "métriques indisponibles");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}
