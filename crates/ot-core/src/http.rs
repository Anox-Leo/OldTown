//! Client HTTP/1.1 de l'API des fichiers (étape 5, note 0026), sur `hyper` :
//! même relais, même écoute et mêmes autorités TLS que le WebSocket. Une
//! connexion est gardée ouverte et réutilisée tant qu'elle répond.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::{self, SendRequest};
use hyper::{Method, Request, header};
use hyper_util::rt::TokioIo;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::tls::TlsRoots;

/// Délai d'ouverture d'une connexion (TCP et TLS).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Adresse HTTP du relais, tirée de son URL (`ws`, `wss`, `http`, `https`).
#[derive(Clone)]
pub(crate) struct HttpTarget {
    host: String,
    port: u16,
    /// Valeur de l'en-tête `Host`.
    authority: String,
    tls: Option<(TlsConnector, ServerName<'static>)>,
}

impl HttpTarget {
    pub(crate) fn new(server_url: &str, roots: &TlsRoots) -> Result<Self, String> {
        let url = server_url.trim();
        let (scheme, rest) = url.split_once("://").unwrap_or(("ws", url));
        let secure = match scheme {
            "ws" | "http" => false,
            "wss" | "https" => true,
            other => return Err(format!("schéma d'URL inconnu : {other}")),
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let (host, port) = split_host_port(authority, if secure { 443 } else { 80 })?;
        let tls = if secure {
            let name = ServerName::try_from(host.clone())
                .map_err(|_| format!("nom de serveur invalide : {host}"))?;
            Some((TlsConnector::from(roots.config.clone()), name))
        } else {
            None
        };
        Ok(Self { host, port, authority: authority.to_owned(), tls })
    }
}

/// `hôte:port`, `hôte` seul ou `[ipv6]:port`.
fn split_host_port(authority: &str, default_port: u16) -> Result<(String, u16), String> {
    let bad = || format!("adresse du relais invalide : {authority:?}");
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or_else(bad)?;
        (host, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err(bad());
    }
    let port = match port {
        Some(p) => p.parse().map_err(|_| bad())?,
        None => default_port,
    };
    Ok((host.to_owned(), port))
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Connexion au relais, ouverte à la première requête et rouverte après une
/// erreur.
pub(crate) struct HttpConn {
    target: HttpTarget,
    sender: Option<SendRequest<Full<Bytes>>>,
}

impl HttpConn {
    pub(crate) fn new(target: HttpTarget) -> Self {
        Self { target, sender: None }
    }

    async fn open(&self) -> Result<SendRequest<Full<Bytes>>, String> {
        let t = &self.target;
        let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect((t.host.as_str(), t.port)))
            .await
            .map_err(|_| "délai de connexion dépassé".to_owned())?
            .map_err(|e| format!("connexion impossible : {e}"))?;
        let _ = tcp.set_nodelay(true);
        let io: Box<dyn Io> = match &t.tls {
            Some((connector, name)) => Box::new(
                timeout(CONNECT_TIMEOUT, connector.connect(name.clone(), tcp))
                    .await
                    .map_err(|_| "délai de la poignée de main TLS dépassé".to_owned())?
                    .map_err(|e| format!("TLS : {e}"))?,
            ),
            None => Box::new(tcp),
        };
        let (sender, conn) =
            http1::handshake(TokioIo::new(io)).await.map_err(|e| format!("HTTP : {e}"))?;
        tokio::spawn(async move {
            // Fin de la connexion : la prochaine requête en rouvrira une.
            let _ = conn.await;
        });
        Ok(sender)
    }

    /// Envoie une requête et lit toute la réponse, en `limit` au plus. Une
    /// erreur ferme la connexion.
    pub(crate) async fn request(
        &mut self,
        method: Method,
        path: &str,
        headers: Vec<(&'static str, String)>,
        body: Bytes,
        limit: Duration,
    ) -> Result<(u16, Bytes), String> {
        let res = timeout(limit, self.request_inner(method, path, headers, body))
            .await
            .unwrap_or_else(|_| Err("délai de réponse dépassé".to_owned()));
        if res.is_err() {
            self.sender = None;
        }
        res
    }

    async fn request_inner(
        &mut self,
        method: Method,
        path: &str,
        headers: Vec<(&'static str, String)>,
        body: Bytes,
    ) -> Result<(u16, Bytes), String> {
        let sender = match &mut self.sender {
            Some(s) if !s.is_closed() => s,
            _ => self.sender.insert(self.open().await?),
        };
        sender.ready().await.map_err(|e| format!("HTTP : {e}"))?;
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &self.target.authority)
            .header(header::CONTENT_LENGTH, body.len());
        for (name, value) in headers {
            req = req.header(name, value);
        }
        let req = req.body(Full::new(body)).map_err(|e| format!("requête invalide : {e}"))?;
        let res = sender.send_request(req).await.map_err(|e| format!("HTTP : {e}"))?;
        let status = res.status().as_u16();
        let body = res.into_body().collect().await.map_err(|e| format!("HTTP : {e}"))?;
        Ok((status, body.to_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_from_relay_urls() {
        let roots = TlsRoots::mozilla();
        let t = HttpTarget::new("wss://relais.example.ts.net:7878", &roots).unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.tls.is_some()),
            ("relais.example.ts.net", 7878, true)
        );
        let t = HttpTarget::new("ws://127.0.0.1:9000/", &roots).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.tls.is_some()), ("127.0.0.1", 9000, false));
        assert_eq!(t.authority, "127.0.0.1:9000");
        let t = HttpTarget::new("https://relais", &roots).unwrap();
        assert_eq!(t.port, 443);
        let t = HttpTarget::new("relais:7878", &roots).unwrap();
        assert_eq!((t.port, t.tls.is_some()), (7878, false));
        let t = HttpTarget::new("ws://[::1]:7878", &roots).unwrap();
        assert_eq!((t.host.as_str(), t.port), ("::1", 7878));
        assert!(HttpTarget::new("ftp://relais", &roots).is_err());
        assert!(HttpTarget::new("ws://:7878", &roots).is_err());
        assert!(HttpTarget::new("ws://relais:port", &roots).is_err());
    }
}
