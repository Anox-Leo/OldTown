//! Appairage côté nouvel appareil, première moitié : s'enregistrer auprès du
//! relais avec le jeton du code (`pair_redeem`). La suite (demande chiffrée
//! au parrain, réponse) passe par le [`crate::Client`] une fois authentifié.

use futures_util::{SinkExt, StreamExt};
use ot_crypto::{PairingCode, token_hash};
use ot_proto::{DeviceId, ErrorCode, Frame};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use crate::ClientError;
use crate::Identity;
use crate::client::{HANDSHAKE_TIMEOUT, Ws, ws_url};
use crate::tls::{self, TlsRoots};

/// Appareil enregistré par le relais.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redeemed {
    pub device_id: DeviceId,
    /// Parrain, à qui envoyer la demande d'appairage.
    pub sponsor: DeviceId,
}

/// Enregistre `identity` sous le nom `name` avec le jeton du code. Le secret
/// du code ne quitte pas l'appareil. Le relais ferme ensuite la connexion.
pub async fn redeem_pairing(
    server_url: &str,
    tls: &TlsRoots,
    identity: &Identity,
    name: &str,
    code: &str,
) -> Result<Redeemed, ClientError> {
    let code = PairingCode::parse(code).map_err(|e| ClientError::Pairing(e.to_string()))?;
    let url = ws_url(server_url);
    let mut ws = match timeout(HANDSHAKE_TIMEOUT, tls::connect(&url, tls, None)).await {
        Ok(Ok(ok)) => ok,
        Ok(Err(e)) => return Err(ClientError::Connection(format!("{url} : {e}"))),
        Err(_) => return Err(ClientError::Connection(format!("{url} : délai dépassé"))),
    };
    let res = timeout(HANDSHAKE_TIMEOUT, exchange(&mut ws, identity, name, &code))
        .await
        .unwrap_or_else(|_| Err(ClientError::Connection("pas de réponse du relais".into())));
    let _ = ws.close(None).await;
    res
}

async fn exchange(
    ws: &mut Ws,
    identity: &Identity,
    name: &str,
    code: &PairingCode,
) -> Result<Redeemed, ClientError> {
    loop {
        match recv(ws).await? {
            Frame::Challenge { nonce } => {
                let sig = identity.sign_pair_redeem(&nonce, &token_hash(&code.token));
                let frame = Frame::PairRedeem {
                    token: code.token,
                    name: name.to_owned(),
                    public_key: identity.public_key(),
                    sig,
                };
                ws.send(Message::text(frame.to_json()))
                    .await
                    .map_err(|e| ClientError::Connection(e.to_string()))?;
            }
            Frame::PairRedeemed { device_id, sponsor } => {
                return Ok(Redeemed { device_id, sponsor });
            }
            Frame::Error { code, message, .. } => {
                return Err(match code {
                    ErrorCode::PairingInvalid | ErrorCode::NameTaken => {
                        ClientError::Rejected { code, message }
                    }
                    _ => ClientError::Pairing(message),
                });
            }
            other => tracing::debug!(t = other.type_name(), "trame ignorée pendant l'appairage"),
        }
    }
}

async fn recv(ws: &mut Ws) -> Result<Frame, ClientError> {
    loop {
        match ws.next().await {
            None | Some(Ok(Message::Close(_))) => {
                return Err(ClientError::Connection("connexion fermée par le relais".into()));
            }
            Some(Err(e)) => return Err(ClientError::Connection(e.to_string())),
            Some(Ok(Message::Text(text))) => match Frame::from_json(&text) {
                Ok(frame) => return Ok(frame),
                Err(e) => tracing::warn!(error = %e, "trame illisible ignorée"),
            },
            Some(Ok(_)) => {}
        }
    }
}
