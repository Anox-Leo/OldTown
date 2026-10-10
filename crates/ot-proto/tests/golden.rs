//! Tests « golden » : chaque fichier de `tests/golden` fige le format d'une
//! trame. Un changement de format casse ces tests volontairement.

use ot_proto::{
    Content, DecodeError, DeviceId, DeviceInfo, Encoding, Envelope, ErrorCode, Frame,
    MAX_ENVELOPE_BYTES, Payload, Ulid, auth_message, pair_redeem_message,
};
use time::macros::datetime;

const A: &str = "dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5";
const B: &str = "dev_01J9Z3M2N4P6Q8R0S2T4V6W8X0";
const MSG: &str = "01J9Z3Q5RT8YB6ZK4C2D7E9F1G";

fn dev(s: &str) -> DeviceId {
    s.parse().unwrap()
}

fn envelope() -> Envelope {
    Envelope {
        msg_id: Ulid::from_string(MSG).unwrap(),
        from: dev(A),
        to: vec![dev(B)],
        created_at: datetime!(2026-10-08 19:30:00 UTC),
        payload: Payload::plain(&Content::text("bonjour")),
    }
}

fn golden(name: &str) -> String {
    let path = format!("{}/tests/golden/{name}.json", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap().trim_end().to_owned()
}

fn check(name: &str, frame: Frame) {
    let text = golden(name);
    assert_eq!(Frame::from_json(&text).unwrap(), frame, "décodage de {name}");
    assert_eq!(frame.to_json(), text, "encodage de {name}");
    assert_eq!(frame.type_name(), name);
}

#[test]
fn golden_frames() {
    check("challenge", Frame::Challenge { nonce: std::array::from_fn(|i| i as u8) });
    check("auth", Frame::Auth { device_id: dev(A), sig: [7; 64] });
    check("welcome", Frame::Welcome { device_id: dev(A), pending: 3 });
    check("send", Frame::Send { env: envelope() });
    check("accepted", Frame::Accepted { msg_id: Ulid::from_string(MSG).unwrap(), seq: 42 });
    check("deliver", Frame::Deliver { seq: 42, env: envelope() });
    check("ack", Frame::Ack { seq: 42 });
    check("list_devices", Frame::ListDevices);
    check(
        "devices",
        Frame::Devices {
            devices: vec![
                DeviceInfo { id: dev(A), name: "mac".into(), online: true },
                DeviceInfo { id: dev(B), name: "windows".into(), online: false },
            ],
        },
    );
    check("pair_create", Frame::PairCreate);
    check(
        "pair_created",
        Frame::PairCreated {
            token: std::array::from_fn(|i| 20 + i as u8),
            expires_at: datetime!(2026-10-10 08:10:00 UTC),
        },
    );
    check(
        "pair_redeem",
        Frame::PairRedeem {
            token: std::array::from_fn(|i| 20 + i as u8),
            name: "windows".into(),
            public_key: [5; 32],
            sig: [7; 64],
        },
    );
    check("pair_redeemed", Frame::PairRedeemed { device_id: dev(B), sponsor: dev(A) });
    check(
        "error",
        Frame::error(
            ErrorCode::EnvelopeTooLarge,
            "enveloppe de 70000 octets, maximum 65536",
            Some(MSG.into()),
        ),
    );
}

#[test]
fn payload_roundtrip() {
    let env = envelope();
    assert_eq!(env.payload.enc, Encoding::None);
    assert_eq!(env.payload.plain_content(), Some(Content::text("bonjour")));
    let encrypted = Payload { enc: Encoding::Xchacha20poly1305, data: vec![1, 2, 3] };
    assert_eq!(encrypted.plain_content(), None);
    assert!(env.encoded_len() < MAX_ENVELOPE_BYTES);
}

#[test]
fn rejects_unknown_type() {
    let err = Frame::from_json(r#"{"v":1,"t":"teleport"}"#).unwrap_err();
    assert_eq!(err, DecodeError::UnknownType("teleport".into()));
    assert_eq!(err.code(), ErrorCode::UnknownType);
}

#[test]
fn rejects_other_versions() {
    let err = Frame::from_json(r#"{"v":2,"t":"ack","seq":1}"#).unwrap_err();
    assert_eq!(err, DecodeError::UnsupportedVersion(2));
    assert!(matches!(Frame::from_json(r#"{"t":"ack","seq":1}"#), Err(DecodeError::BadFrame(_))));
}

#[test]
fn rejects_malformed_frames() {
    for text in [
        "pas du json",
        "[1,2]",
        r#"{"v":1}"#,
        r#"{"v":1,"t":"ack"}"#,
        r#"{"v":1,"t":"ack","seq":1,"extra":true}"#,
        r#"{"v":1,"t":"challenge","nonce":"AAAA"}"#,
        r#"{"v":1,"t":"welcome","device_id":"dev_pasunulid","pending":0}"#,
    ] {
        assert!(
            matches!(Frame::from_json(text), Err(DecodeError::BadFrame(_))),
            "aurait dû être rejetée : {text}"
        );
    }
}

#[test]
fn pairing_error_codes() {
    for (code, text) in
        [(ErrorCode::PairingInvalid, "pairing_invalid"), (ErrorCode::NameTaken, "name_taken")]
    {
        let json = Frame::error(code, "x", None).to_json();
        assert!(json.contains(&format!("\"code\":\"{text}\"")), "{json}");
        assert_eq!(Frame::from_json(&json).unwrap(), Frame::error(code, "x", None));
    }
}

#[test]
fn unknown_error_codes_are_tolerated() {
    let frame = Frame::from_json(r#"{"v":1,"t":"error","code":"futur","message":"x"}"#).unwrap();
    assert_eq!(frame, Frame::error(ErrorCode::Unknown, "x", None));
}

#[test]
fn device_id_format() {
    let id = DeviceId::generate();
    assert!(id.as_str().starts_with("dev_"));
    assert_eq!(id.as_str().parse::<DeviceId>().unwrap(), id);
    assert!("01J9Z3K8V7Q2W4E6R8T0Y1V3H5".parse::<DeviceId>().is_err());
    assert!("dev_".parse::<DeviceId>().is_err());
}

#[test]
fn auth_message_layout() {
    let nonce = [9u8; 32];
    let msg = auth_message(&nonce, &dev(A));
    assert_eq!(&msg[..10], b"ot-auth-v1");
    assert_eq!(&msg[10..42], &nonce);
    assert_eq!(&msg[42..], A.as_bytes());
}

#[test]
fn pair_redeem_message_layout() {
    let nonce = [9u8; 32];
    let hash = [3u8; 32];
    let msg = pair_redeem_message(&nonce, &hash);
    assert_eq!(&msg[..10], b"ot-pair-v1");
    assert_eq!(&msg[10..42], &nonce);
    assert_eq!(&msg[42..], &hash);
}
