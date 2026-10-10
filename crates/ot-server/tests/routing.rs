//! T6 : routage, erreurs typées, sessions.

mod common;

use common::*;
use ot_proto::{Content, Envelope, ErrorCode, Frame, MAX_ENVELOPE_BYTES, Payload};
use ot_server::store::Store;

async fn setup() -> (TestServer, TestDevice, TestDevice) {
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "mac").await;
    let b = add_device(&store, "windows").await;
    (TestServer::start(store, test_config()).await, a, b)
}

fn rejected(frame: Option<Frame>, expected: ErrorCode, env: &Envelope) {
    match frame {
        Some(Frame::Error { code, reference, .. }) => {
            assert_eq!(code, expected);
            assert_eq!(reference, Some(env.msg_id.to_string()));
        }
        other => panic!("erreur {expected:?} attendue, reçu {other:?}"),
    }
}

#[tokio::test]
async fn send_accept_deliver_ack() {
    let (server, a, b) = setup().await;
    let (mut ra, _) = Raw::login(&server, &a).await;
    let (mut rb, _) = Raw::login(&server, &b).await;

    let env = Envelope::new_plain(a.id.clone(), vec![], &Content::text("bonjour"));
    ra.send(&Frame::Send { env: env.clone() }).await;
    let seq = match ra.recv().await {
        Some(Frame::Accepted { msg_id, seq }) if msg_id == env.msg_id => seq,
        other => panic!("accepted attendu, reçu {other:?}"),
    };
    assert_eq!(rb.recv().await, Some(Frame::Deliver { seq, env }));
    rb.send(&Frame::Ack { seq }).await;

    let store = server.relay.store();
    for _ in 0..50 {
        if store.pending_count(&b.id, ot_server::now_ms()).await.unwrap() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("l'ack n'a pas été enregistré");
}

#[tokio::test]
async fn oversized_envelope_is_rejected() {
    let (server, a, _) = setup().await;
    let (mut ra, _) = Raw::login(&server, &a).await;
    let mut env = Envelope::new_plain(a.id.clone(), vec![], &Content::text(""));
    env.payload = Payload { data: vec![b'x'; MAX_ENVELOPE_BYTES], ..env.payload };
    ra.send(&Frame::Send { env: env.clone() }).await;
    rejected(ra.recv().await, ErrorCode::EnvelopeTooLarge, &env);

    // La session continue.
    ra.send(&Frame::ListDevices).await;
    assert!(matches!(ra.recv().await, Some(Frame::Devices { .. })));
}

#[tokio::test]
async fn spoofed_sender_is_rejected() {
    let (server, a, b) = setup().await;
    let (mut ra, _) = Raw::login(&server, &a).await;
    let env = Envelope::new_plain(b.id.clone(), vec![], &Content::text("usurpation"));
    ra.send(&Frame::Send { env: env.clone() }).await;
    rejected(ra.recv().await, ErrorCode::SenderMismatch, &env);
    assert_eq!(server.relay.store().pending_count(&b.id, ot_server::now_ms()).await.unwrap(), 0);
}

#[tokio::test]
async fn unknown_recipient_is_rejected() {
    let (server, a, _) = setup().await;
    let (mut ra, _) = Raw::login(&server, &a).await;
    let ghost = ot_proto::DeviceId::generate();
    let env = Envelope::new_plain(a.id.clone(), vec![ghost], &Content::text("?"));
    ra.send(&Frame::Send { env: env.clone() }).await;
    rejected(ra.recv().await, ErrorCode::UnknownRecipient, &env);
}

#[tokio::test]
async fn unknown_and_unexpected_frames() {
    let (server, a, _) = setup().await;
    let (mut ra, _) = Raw::login(&server, &a).await;
    ra.send_text(r#"{"v":1,"t":"teleport"}"#.into()).await;
    assert!(matches!(ra.recv().await, Some(Frame::Error { code: ErrorCode::UnknownType, .. })));
    ra.send_text(r#"{"v":9,"t":"ack","seq":1}"#.into()).await;
    assert!(matches!(
        ra.recv().await,
        Some(Frame::Error { code: ErrorCode::UnsupportedVersion, .. })
    ));
    ra.send(&Frame::Welcome { device_id: a.id.clone(), pending: 0 }).await;
    assert!(matches!(ra.recv().await, Some(Frame::Error { code: ErrorCode::UnexpectedFrame, .. })));
}

#[tokio::test]
async fn new_session_replaces_old() {
    let (server, a, b) = setup().await;
    let (mut first, _) = Raw::login(&server, &a).await;
    let (mut second, _) = Raw::login(&server, &a).await;
    assert_eq!(first.recv().await, None, "l'ancienne session est fermée");

    let (mut rb, _) = Raw::login(&server, &b).await;
    let env = Envelope::new_plain(b.id.clone(), vec![a.id.clone()], &Content::text("x"));
    rb.send(&Frame::Send { env: env.clone() }).await;
    assert!(matches!(rb.recv().await, Some(Frame::Accepted { .. })));
    assert!(matches!(second.recv().await, Some(Frame::Deliver { env: e, .. }) if e == env));
}

#[tokio::test]
async fn list_devices_shows_presence() {
    let (server, a, b) = setup().await;
    let (mut ra, _) = Raw::login(&server, &a).await;
    ra.send(&Frame::ListDevices).await;
    let Some(Frame::Devices { devices }) = ra.recv().await else { panic!("devices attendu") };
    let online = |id| devices.iter().find(|d| &d.id == id).unwrap().online;
    assert!(online(&a.id));
    assert!(!online(&b.id));
}

#[tokio::test]
async fn healthz_and_metrics() {
    let (server, a, _) = setup().await;
    assert_eq!(http_get(server.addr, "/healthz").await, (200, "ok\n".into()));
    let _session = Raw::login(&server, &a).await;
    let (code, body) = http_get(server.addr, "/metrics").await;
    assert_eq!(code, 200);
    assert!(body.contains("ot_connections 1"), "{body}");
    assert!(body.contains("ot_messages_purged_total{reason=\"expired\"} 0"), "{body}");
}
