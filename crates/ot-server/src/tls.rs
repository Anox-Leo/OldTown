//! TLS du relais (`wss`, note 0015) : certificat et clé lus en PEM, rechargés
//! sans redémarrage quand les fichiers changent (renouvellement par
//! `tailscale cert`), et poignées de main faites hors de la boucle d'accueil.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// Délai maximal d'une poignée de main TLS.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Connexions chiffrées prêtes, en attente d'être servies.
const READY_QUEUE: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("lecture de {path} : {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{0} : aucun certificat PEM")]
    NoCertificate(PathBuf),
    #[error("{0} : clé privée PEM illisible")]
    BadKey(PathBuf),
    #[error("certificat ou clé refusé : {0}")]
    Rustls(#[from] rustls::Error),
}

/// Fournisseur crypto du relais : `ring`, comme côté client.
fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Certificat et clé du relais, rechargeables à chaud.
#[derive(Clone)]
pub struct Tls {
    inner: Arc<TlsInner>,
}

struct TlsInner {
    cert_path: PathBuf,
    key_path: PathBuf,
    resolver: Arc<Resolver>,
    config: Arc<ServerConfig>,
    /// Dates de modification lues au dernier chargement.
    loaded: Mutex<(Option<SystemTime>, Option<SystemTime>)>,
}

#[derive(Debug)]
struct Resolver {
    current: RwLock<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.read().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

impl Tls {
    /// Charge la chaîne de certificats et la clé privée (PEM).
    pub fn load(cert_path: &Path, key_path: &Path) -> Result<Self, TlsError> {
        let key = load_pair(cert_path, key_path)?;
        let resolver = Arc::new(Resolver { current: RwLock::new(Arc::new(key)) });
        let mut config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(resolver.clone());
        // WebSocket sur HTTP/1.1 seulement.
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Self {
            inner: Arc::new(TlsInner {
                cert_path: cert_path.to_owned(),
                key_path: key_path.to_owned(),
                resolver,
                config: Arc::new(config),
                loaded: Mutex::new((modified(cert_path), modified(key_path))),
            }),
        })
    }

    /// Relit les fichiers s'ils ont changé depuis le dernier chargement.
    /// Renvoie `true` si un nouveau certificat est en service. En cas
    /// d'erreur (fichiers en cours d'écriture, par exemple), l'ancien reste.
    pub fn reload_if_changed(&self) -> Result<bool, TlsError> {
        let inner = &self.inner;
        let now = (modified(&inner.cert_path), modified(&inner.key_path));
        let mut loaded = inner.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if *loaded == now {
            return Ok(false);
        }
        let key = load_pair(&inner.cert_path, &inner.key_path)?;
        *inner.resolver.current.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(key);
        *loaded = now;
        Ok(true)
    }

    fn acceptor(&self) -> TlsAcceptor {
        TlsAcceptor::from(self.inner.config.clone())
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read(path: &Path) -> Result<Vec<u8>, TlsError> {
    std::fs::read(path).map_err(|source| TlsError::Read { path: path.to_owned(), source })
}

fn load_pair(cert_path: &Path, key_path: &Path) -> Result<CertifiedKey, TlsError> {
    let certs = CertificateDer::pem_slice_iter(&read(cert_path)?)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsError::NoCertificate(cert_path.to_owned()))?;
    if certs.is_empty() {
        return Err(TlsError::NoCertificate(cert_path.to_owned()));
    }
    let key = PrivateKeyDer::from_pem_slice(&read(key_path)?)
        .map_err(|_| TlsError::BadKey(key_path.to_owned()))?;
    let key = CertifiedKey::from_der(certs, key, &provider())?;
    Ok(key)
}

/// [`axum::serve::Listener`] qui ne rend que des connexions dont la poignée
/// de main TLS a réussi. Une connexion lente ne bloque pas les autres.
pub struct TlsListener {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
    task: JoinHandle<()>,
}

impl TlsListener {
    pub fn new(listener: TcpListener, tls: Tls) -> io::Result<Self> {
        let local = listener.local_addr()?;
        let (tx, ready) = mpsc::channel(READY_QUEUE);
        let task = tokio::spawn(accept_loop(listener, tls, tx));
        Ok(Self { ready, local, task })
    }
}

impl Drop for TlsListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_loop(
    listener: TcpListener,
    tls: Tls,
    ready: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    loop {
        let (tcp, addr) = match listener.accept().await {
            Ok(ok) => ok,
            Err(e) => {
                // Trop de fichiers ouverts, par exemple : on souffle un peu.
                tracing::warn!(error = %e, "accept");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let (acceptor, ready) = (tls.acceptor(), ready.clone());
        tokio::spawn(async move {
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await {
                Ok(Ok(stream)) => {
                    let _ = ready.send((stream, addr)).await;
                }
                Ok(Err(e)) => tracing::debug!(%addr, error = %e, "poignée de main TLS refusée"),
                Err(_) => tracing::debug!(%addr, "poignée de main TLS trop lente"),
            }
        });
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(conn) => conn,
            // La boucle d'accueil ne s'arrête qu'avec le listener lui-même.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local)
    }
}
