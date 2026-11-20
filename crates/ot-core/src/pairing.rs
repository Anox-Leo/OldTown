//! Appairage côté nouvel appareil, première moitié : s'enregistrer auprès du
//! relais avec le jeton du code (`pair_redeem`, ou `invite_redeem` pour le
//! premier appareil d'une personne invitée, étape 10). La suite (demande
//! chiffrée au parrain ou à l'admin, réponse) passe par le
//! [`crate::Client`] une fois authentifié.

use futures_util::{SinkExt, StreamExt};
use ot_crypto::{InviteCode, PairingCode, token_hash};
use ot_proto::{DeviceId, ErrorCode, Frame};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use crate::ClientError;
use crate::Identity;
use crate::client::{HANDSHAKE_TIMEOUT, Ws, ws_url};
use crate::invite::InviteRedeemed;
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
    let token = code.token;
    let answer = |frame| match frame {
        Frame::PairRedeemed { device_id, sponsor } => Some(Redeemed { device_id, sponsor }),
        _ => None,
    };
    let redeem = |nonce: &[u8; ot_proto::NONCE_LEN]| Frame::PairRedeem {
        token,
        name: name.to_owned(),
        public_key: identity.public_key(),
        sig: identity.sign_pair_redeem(nonce, &token_hash(&token)),
    };
    redeem_with(server_url, tls, redeem, answer).await
}

/// Premier appareil d'une personne invitée (étape 10) : s'enregistre dans
/// sa personne avec le jeton du code `ot2-…`. Le secret `S` et l'ancre ne
/// quittent pas l'appareil ; à ranger ensuite par
/// [`crate::save_pending_invite`] avant de démarrer le client.
pub async fn redeem_invite(
    server_url: &str,
    tls: &TlsRoots,
    identity: &Identity,
    name: &str,
    code: &InviteCode,
) -> Result<InviteRedeemed, ClientError> {
    let token = code.token;
    let answer = |frame| match frame {
        Frame::InviteRedeemed { user, device, inviter } => {
            Some(InviteRedeemed { user, device_id: device, inviter })
        }
        _ => None,
    };
    let redeem = |nonce: &[u8; ot_proto::NONCE_LEN]| Frame::InviteRedeem {
        token,
        name: name.to_owned(),
        public_key: identity.public_key(),
        sig: identity.sign_invite_redeem(nonce, &token_hash(&token)),
    };
    redeem_with(server_url, tls, redeem, answer).await
}

/// Connexion, défi, trame de rachat signée, réponse ; le relais ferme
/// ensuite.
async fn redeem_with<T>(
    server_url: &str,
    tls: &TlsRoots,
    redeem: impl Fn(&[u8; ot_proto::NONCE_LEN]) -> Frame,
    answer: impl Fn(Frame) -> Option<T>,
) -> Result<T, ClientError> {
    let url = ws_url(server_url);
    let mut ws = match timeout(HANDSHAKE_TIMEOUT, tls::connect(&url, tls, None)).await {
        Ok(Ok(ok)) => ok,
        Ok(Err(e)) => return Err(ClientError::Connection(format!("{url} : {e}"))),
        Err(_) => return Err(ClientError::Connection(format!("{url} : délai dépassé"))),
    };
    let res = timeout(HANDSHAKE_TIMEOUT, exchange(&mut ws, redeem, answer))
        .await
        .unwrap_or_else(|_| Err(ClientError::Connection("pas de réponse du relais".into())));
    let _ = ws.close(None).await;
    res
}

async fn exchange<T>(
    ws: &mut Ws,
    redeem: impl Fn(&[u8; ot_proto::NONCE_LEN]) -> Frame,
    answer: impl Fn(Frame) -> Option<T>,
) -> Result<T, ClientError> {
    loop {
        match recv(ws).await? {
            Frame::Challenge { nonce } => {
                let frame = redeem(&nonce);
                ws.send(Message::text(frame.to_json()))
                    .await
                    .map_err(|e| ClientError::Connection(e.to_string()))?;
            }
            frame @ (Frame::PairRedeemed { .. } | Frame::InviteRedeemed { .. }) => {
                if let Some(done) = answer(frame) {
                    return Ok(done);
                }
                tracing::debug!("réponse de rachat inattendue ignorée");
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
