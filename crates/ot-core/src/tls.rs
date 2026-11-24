use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::CertificateDer;
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::client::Ws;

#[derive(Clone)]
pub struct TlsRoots {
    pub(crate) config: Arc<ClientConfig>,
}

impl TlsRoots {
    pub fn with_extra(extra: &[Vec<u8>]) -> Result<Self, String> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for der in extra {
            roots
                .add(CertificateDer::from(der.clone()))
                .map_err(|e| format!("certificat racine refusé : {e}"))?;
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS : {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self { config: Arc::new(config) })
    }

    pub fn mozilla() -> Self {
        Self::with_extra(&[]).expect("configuration TLS par défaut")
    }
}

impl Default for TlsRoots {
    fn default() -> Self {
        Self::mozilla()
    }
}

impl std::fmt::Debug for TlsRoots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TlsRoots")
    }
}

pub(crate) async fn connect(
    url: &str,
    tls: &TlsRoots,
    config: Option<WebSocketConfig>,
) -> Result<Ws, String> {
    let connector = Connector::Rustls(tls.config.clone());
    tokio_tungstenite::connect_async_tls_with_config(url, config, true, Some(connector))
        .await
        .map(|(ws, _)| ws)
        .map_err(|e| e.to_string())
}
