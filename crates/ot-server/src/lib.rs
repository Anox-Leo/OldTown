//! Serveur relais OldTown : authentifie les appareils, stocke les enveloppes
//! et les livre à leurs destinataires. Il ne lit jamais `payload`.

mod auth;
pub mod metrics;
mod registry;
mod session;
pub mod store;

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::metrics::Metrics;
use crate::registry::Registry;
use crate::store::{PurgeStats, Store};

pub use crate::auth::verify as verify_auth;

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(7 * 24 * 3600),
            purge_interval: Duration::from_secs(10 * 60),
            ping_interval: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(30),
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
}

impl Relay {
    pub fn new(store: Store, config: Config) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
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
            .with_state(self.clone())
    }

    /// Lance une purge et met à jour les compteurs.
    pub async fn purge_once(&self) -> store::Result<PurgeStats> {
        let stats = self.inner.store.purge(now_ms()).await?;
        let m = &self.inner.metrics;
        m.purged_acked.fetch_add(stats.acked, std::sync::atomic::Ordering::Relaxed);
        m.purged_expired.fetch_add(stats.expired, std::sync::atomic::Ordering::Relaxed);
        if stats != PurgeStats::default() {
            tracing::info!(acked = stats.acked, expired = stats.expired, "purge");
        }
        Ok(stats)
    }

    pub async fn metrics_text(&self) -> store::Result<String> {
        let pending = self.inner.store.total_pending(now_ms()).await?;
        Ok(self.inner.metrics.render(pending))
    }

    /// Ferme la session ouverte d'un appareil (après révocation, par exemple).
    pub fn disconnect(&self, device: &ot_proto::DeviceId) {
        self.inner.registry.kick(device);
    }
}

/// Sert le relais sur `listener` jusqu'à ce que `shutdown` se termine, en
/// purgeant périodiquement. Les sessions ouvertes sont fermées à l'arrêt.
pub async fn serve(
    relay: Relay,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
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
