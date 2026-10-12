//! Une connexion WebSocket : défi, authentification, puis boucle de session.

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket};
use ot_proto::{
    DeviceId, DeviceInfo, ErrorCode, Frame, MAX_ENVELOPE_BYTES, MAX_NAME_CHARS, valid_device_name,
};
use std::sync::atomic::Ordering::Relaxed;
use time::OffsetDateTime;
use tokio::time::{Instant, timeout_at};

use crate::metrics::Metrics;
use crate::registry::Session;
use crate::store::{NewMessage, StoreError};
use crate::{Inner, Relay, auth, now_ms};

/// Nombre de messages relus en base à chaque passe de livraison.
const DELIVERY_BATCH: u32 = 100;

/// Fin de session : la connexion doit être fermée.
struct Close(&'static str);

pub async fn run(relay: Relay, mut socket: WebSocket) {
    let inner = &*relay.inner;
    let device = match authenticate(inner, &mut socket).await {
        Ok(device) => device,
        Err(Close(reason)) => {
            tracing::debug!(reason, "connexion refusée");
            close(&mut socket, reason).await;
            return;
        }
    };

    let session = inner.registry.register(&device);
    inner.metrics.connections.fetch_add(1, Relaxed);
    tracing::info!(device = %device, "session ouverte");

    let mut s = Active { inner, device, cursor: 0, socket };
    let reason = match s.serve(&session).await {
        Ok(()) => "fin de session",
        Err(Close(reason)) => reason,
    };
    close(&mut s.socket, reason).await;

    inner.registry.unregister(&s.device, &session);
    inner.metrics.connections.fetch_sub(1, Relaxed);
    if let Err(e) = inner.store.touch_device(&s.device, now_ms()).await {
        tracing::warn!(error = %e, "last_seen_at non mis à jour");
    }
    tracing::info!(device = %s.device, reason, "session fermée");
}

async fn close(socket: &mut WebSocket, reason: &'static str) {
    let frame = CloseFrame { code: 1000, reason: Utf8Bytes::from_static(reason) };
    let _ = socket.send(Message::Close(Some(frame))).await;
}

async fn send(socket: &mut WebSocket, frame: &Frame) -> Result<(), Close> {
    socket.send(Message::text(frame.to_json())).await.map_err(|_| Close("écriture impossible"))
}

async fn authenticate(inner: &Inner, socket: &mut WebSocket) -> Result<DeviceId, Close> {
    let nonce = auth::new_nonce();
    let deadline = Instant::now() + inner.config.auth_timeout;
    send(socket, &Frame::Challenge { nonce }).await?;

    loop {
        let msg = match timeout_at(deadline, socket.recv()).await {
            Err(_) => return auth_failed(inner, socket, "délai d'authentification dépassé").await,
            Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_)))) => {
                return Err(Close("connexion fermée"));
            }
            Ok(Some(Ok(msg))) => msg,
        };
        let text = match msg {
            Message::Text(text) => text,
            Message::Binary(_) => {
                send(socket, &Frame::error(ErrorCode::BadFrame, "trame texte attendue", None))
                    .await?;
                continue;
            }
            _ => continue,
        };
        match Frame::from_json(&text) {
            Ok(Frame::Auth { device_id, sig }) => {
                let device = match inner.store.device(&device_id).await {
                    Ok(Some(d)) if d.revoked_at.is_none() => d,
                    Ok(_) => {
                        return auth_failed(inner, socket, "appareil inconnu ou révoqué").await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "lecture de l'appareil");
                        return auth_failed(inner, socket, "erreur interne").await;
                    }
                };
                if !auth::verify(&device.public_key, &nonce, &device_id, &sig) {
                    tracing::warn!(device = %device_id, "signature invalide");
                    return auth_failed(inner, socket, "signature invalide").await;
                }
                return Ok(device_id);
            }
            Ok(Frame::PairRedeem { token, name, public_key, sig }) => {
                return redeem(inner, socket, &nonce, &token, &name, &public_key, &sig).await;
            }
            Ok(other) => {
                let msg = format!("trame {} reçue avant auth", other.type_name());
                send(socket, &Frame::error(ErrorCode::UnexpectedFrame, msg, None)).await?;
            }
            Err(e) => send(socket, &Frame::error(e.code(), e.to_string(), None)).await?,
        }
    }
}

async fn auth_failed(
    inner: &Inner,
    socket: &mut WebSocket,
    message: &'static str,
) -> Result<DeviceId, Close> {
    Metrics::inc(&inner.metrics.auth_failures);
    send(socket, &Frame::error(ErrorCode::AuthFailed, message, None)).await?;
    Err(Close("auth_failed"))
}

fn invalid_name() -> String {
    format!("nom invalide : 1 à {MAX_NAME_CHARS} caractères imprimables")
}

/// Enregistre un appareil avec un jeton d'appairage, répond `pair_redeemed`
/// puis ferme : l'appareil se reconnecte ensuite avec `auth`.
async fn redeem(
    inner: &Inner,
    socket: &mut WebSocket,
    nonce: &[u8; ot_proto::NONCE_LEN],
    token: &[u8; ot_proto::PAIR_TOKEN_LEN],
    name: &str,
    public_key: &[u8; 32],
    sig: &[u8; 64],
) -> Result<DeviceId, Close> {
    let name = name.trim();
    if !valid_device_name(name) {
        return pairing_rejected(inner, socket, ErrorCode::BadFrame, invalid_name()).await;
    }
    let hash = auth::token_hash(token);
    if !auth::verify_redeem(public_key, nonce, &hash, sig) {
        tracing::warn!("appairage : signature invalide");
        return pairing_rejected(
            inner,
            socket,
            ErrorCode::PairingInvalid,
            "signature invalide".into(),
        )
        .await;
    }
    match inner.store.redeem_pairing(&hash, name, public_key, now_ms()).await {
        Ok(r) => {
            Metrics::inc(&inner.metrics.pairings_redeemed);
            tracing::info!(device = %r.device.id, sponsor = %r.sponsor, "appareil appairé");
            let frame = Frame::PairRedeemed { device_id: r.device.id, sponsor: r.sponsor };
            send(socket, &frame).await?;
            Err(Close("appairage terminé"))
        }
        Err(StoreError::PairingInvalid) => {
            let msg = "jeton inconnu, expiré ou déjà utilisé".into();
            pairing_rejected(inner, socket, ErrorCode::PairingInvalid, msg).await
        }
        Err(StoreError::KeyTaken) => {
            let msg = "clé publique déjà enregistrée".into();
            pairing_rejected(inner, socket, ErrorCode::PairingInvalid, msg).await
        }
        Err(StoreError::NameTaken(name)) => {
            let msg = format!("le nom {name:?} est déjà pris");
            pairing_rejected(inner, socket, ErrorCode::NameTaken, msg).await
        }
        Err(e) => {
            tracing::error!(error = %e, "appairage");
            pairing_rejected(inner, socket, ErrorCode::Internal, "erreur interne".into()).await
        }
    }
}

async fn pairing_rejected(
    inner: &Inner,
    socket: &mut WebSocket,
    code: ErrorCode,
    message: String,
) -> Result<DeviceId, Close> {
    Metrics::inc(&inner.metrics.pairings_rejected);
    send(socket, &Frame::error(code, message, None)).await?;
    Err(Close("appairage refusé"))
}

struct Active<'a> {
    inner: &'a Inner,
    device: DeviceId,
    /// Dernier `seq` livré pendant cette session.
    cursor: i64,
    socket: WebSocket,
}

impl Active<'_> {
    async fn serve(&mut self, session: &Session) -> Result<(), Close> {
        let pending = self.inner.store.pending_count(&self.device, now_ms()).await.map_err(db)?;
        send(&mut self.socket, &Frame::Welcome { device_id: self.device.clone(), pending }).await?;
        self.deliver().await?;

        let ping_every = self.inner.config.ping_interval;
        let mut ping = tokio::time::interval_at(Instant::now() + ping_every, ping_every);
        let mut last_rx = Instant::now();
        let mut shutdown = self.inner.shutdown.subscribe();
        let stop = async move {
            let _ = shutdown.wait_for(|stop| *stop).await;
        };
        tokio::pin!(stop);

        loop {
            tokio::select! {
                msg = self.socket.recv() => {
                    last_rx = Instant::now();
                    match msg {
                        None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return Ok(()),
                        Some(Ok(Message::Text(text))) => self.handle(&text).await?,
                        Some(Ok(Message::Binary(_))) => {
                            let e = Frame::error(ErrorCode::BadFrame, "trame texte attendue", None);
                            send(&mut self.socket, &e).await?;
                        }
                        Some(Ok(_)) => {} // ping et pong
                    }
                }
                () = session.wake.notified() => self.deliver().await?,
                () = session.kick.notified() => return Err(Close("remplacée par une nouvelle session")),
                () = &mut stop => return Err(Close("arrêt du serveur")),
                _ = ping.tick() => {
                    if last_rx.elapsed() > ping_every * 5 / 2 {
                        return Err(Close("plus de réponse"));
                    }
                    self.socket.send(Message::Ping(Default::default())).await
                        .map_err(|_| Close("écriture impossible"))?;
                }
            }
        }
    }

    /// Livre dans l'ordre tous les messages en attente au-delà du curseur.
    async fn deliver(&mut self) -> Result<(), Close> {
        loop {
            let now = now_ms();
            let batch = self
                .inner
                .store
                .pending(&self.device, self.cursor, DELIVERY_BATCH, now)
                .await
                .map_err(db)?;
            for p in &batch {
                let env = match serde_json::from_slice(&p.envelope) {
                    Ok(env) => env,
                    Err(e) => {
                        tracing::error!(seq = p.seq, error = %e, "enveloppe illisible en base");
                        self.cursor = p.seq;
                        continue;
                    }
                };
                send(&mut self.socket, &Frame::Deliver { seq: p.seq, env }).await?;
                self.cursor = p.seq;
                Metrics::inc(&self.inner.metrics.messages_delivered);
                self.inner.store.mark_delivered(p.seq, &self.device, now).await.map_err(db)?;
            }
            if batch.len() < DELIVERY_BATCH as usize {
                return Ok(());
            }
        }
    }

    async fn handle(&mut self, text: &str) -> Result<(), Close> {
        let frame = match Frame::from_json(text) {
            Ok(frame) => frame,
            Err(e) => {
                return send(&mut self.socket, &Frame::error(e.code(), e.to_string(), None)).await;
            }
        };
        let reply = match frame {
            Frame::Send { env } => self.on_send(env).await?,
            Frame::Ack { seq } => {
                if self.inner.store.ack(seq, &self.device, now_ms()).await.map_err(db)? {
                    Metrics::inc(&self.inner.metrics.messages_acked);
                }
                None
            }
            Frame::ListDevices => Some(devices(self.inner).await?),
            Frame::Ping => Some(Frame::Pong),
            Frame::PairCreate => Some(self.pair_create().await?),
            Frame::PairRegister { device_id, name, public_key, sig } => {
                Some(self.pair_register(device_id, &name, &public_key, &sig).await)
            }
            Frame::Error { code, .. } => {
                tracing::debug!(device = %self.device, ?code, "erreur signalée par le client");
                None
            }
            other => {
                let msg = format!("trame {} inattendue", other.type_name());
                Some(Frame::error(ErrorCode::UnexpectedFrame, msg, None))
            }
        };
        match reply {
            Some(frame) => send(&mut self.socket, &frame).await,
            None => Ok(()),
        }
    }

    /// Tire un jeton d'appairage ; seul son SHA-256 est stocké.
    async fn pair_create(&mut self) -> Result<Frame, Close> {
        let token = auth::new_token();
        let now = now_ms();
        let ttl = self.inner.config.pairing_ttl.as_millis() as i64;
        let expires_at = now.saturating_add(ttl);
        let stored = self
            .inner
            .store
            .create_pairing(&self.device, &auth::token_hash(&token), now, expires_at)
            .await;
        if let Err(e) = stored {
            tracing::error!(error = %e, "création d'un jeton d'appairage");
            return Ok(Frame::error(ErrorCode::Internal, "erreur interne", None));
        }
        Metrics::inc(&self.inner.metrics.pairings_created);
        tracing::info!(sponsor = %self.device, "jeton d'appairage créé");
        let expires_at =
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(expires_at) * 1_000_000)
                .map_err(|_| Close("horloge invalide"))?;
        Ok(Frame::PairCreated { token, expires_at })
    }

    /// Parrain : enregistre l'appareil dont il a scanné le QR inversé. Le nom
    /// n'est pas retouché : la signature porte dessus. Une erreur laisse la
    /// session ouverte.
    async fn pair_register(
        &mut self,
        device_id: DeviceId,
        name: &str,
        public_key: &[u8; 32],
        sig: &[u8; 64],
    ) -> Frame {
        let reference = Some(device_id.to_string());
        let reject = |metrics: &Metrics, code, message: String| {
            Metrics::inc(&metrics.pairings_rejected);
            Frame::error(code, message, reference.clone())
        };
        let m = &self.inner.metrics;
        if !valid_device_name(name) {
            return reject(m, ErrorCode::BadFrame, invalid_name());
        }
        if !auth::verify_register(public_key, &device_id, name, sig) {
            tracing::warn!(sponsor = %self.device, "enregistrement : signature invalide");
            return reject(m, ErrorCode::PairingInvalid, "signature invalide".into());
        }
        match self.inner.store.register_device(&device_id, name, public_key, now_ms()).await {
            Ok(d) => {
                Metrics::inc(&m.pairings_registered);
                tracing::info!(device = %d.id, sponsor = %self.device, "appareil enregistré par son parrain");
                Frame::PairRegistered { device_id: d.id }
            }
            Err(StoreError::KeyTaken) => {
                reject(m, ErrorCode::PairingInvalid, "clé publique déjà enregistrée".into())
            }
            Err(StoreError::IdTaken) => {
                reject(m, ErrorCode::PairingInvalid, "identifiant déjà utilisé".into())
            }
            Err(StoreError::NameTaken(name)) => {
                reject(m, ErrorCode::NameTaken, format!("le nom {name:?} est déjà pris"))
            }
            Err(e) => {
                tracing::error!(error = %e, "enregistrement d'un appareil");
                reject(m, ErrorCode::Internal, "erreur interne".into())
            }
        }
    }

    async fn on_send(&mut self, env: ot_proto::Envelope) -> Result<Option<Frame>, Close> {
        let msg_id = env.msg_id.to_string();
        let reject =
            |code, message: String| Ok(Some(Frame::error(code, message, Some(msg_id.clone()))));

        if env.from != self.device {
            return reject(
                ErrorCode::SenderMismatch,
                "from ne correspond pas à l'appareil authentifié".into(),
            );
        }
        let bytes = serde_json::to_vec(&env).map_err(|_| Close("sérialisation impossible"))?;
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return reject(
                ErrorCode::EnvelopeTooLarge,
                format!("enveloppe de {} octets, maximum {MAX_ENVELOPE_BYTES}", bytes.len()),
            );
        }

        let now = now_ms();
        let ttl = self.inner.config.ttl.as_millis() as i64;
        let new = NewMessage {
            msg_id: &msg_id,
            sender: &self.device,
            to: &env.to,
            envelope: &bytes,
            expires_at: now.saturating_add(ttl),
        };
        let inserted = match self.inner.store.insert_message(new, now).await {
            Ok(i) => i,
            Err(StoreError::UnknownRecipient(id)) => {
                return reject(ErrorCode::UnknownRecipient, format!("destinataire inconnu : {id}"));
            }
            Err(StoreError::MsgIdConflict) => {
                return reject(ErrorCode::MsgIdConflict, "msg_id déjà utilisé".into());
            }
            Err(e) => {
                tracing::error!(error = %e, "stockage du message");
                return reject(ErrorCode::Internal, "erreur interne".into());
            }
        };

        if !inserted.duplicate {
            Metrics::inc(&self.inner.metrics.messages_received);
            tracing::info!(
                from = %self.device,
                msg_id,
                seq = inserted.seq,
                size = bytes.len(),
                recipients = inserted.recipients.len(),
                "message reçu"
            );
        }
        for id in &inserted.recipients {
            self.inner.registry.wake(id);
        }
        Ok(Some(Frame::Accepted { msg_id: env.msg_id, seq: inserted.seq }))
    }
}

async fn devices(inner: &Inner) -> Result<Frame, Close> {
    let devices = inner.store.devices().await.map_err(db)?;
    let devices = devices
        .into_iter()
        .filter(|d| d.revoked_at.is_none())
        .map(|d| DeviceInfo { online: inner.registry.is_online(&d.id), id: d.id, name: d.name })
        .collect();
    Ok(Frame::Devices { devices })
}

fn db(e: StoreError) -> Close {
    tracing::error!(error = %e, "erreur de base");
    Close("erreur interne")
}
