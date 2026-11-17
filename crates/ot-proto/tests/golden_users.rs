//! Tests « golden » des trames de l'étape 10 (personnes, annuaire, réseau,
//! note 0052). Mêmes règles que `golden.rs` : un changement de format casse
//! ces tests volontairement. Noms neutres seulement (Alice, Bob, Notifier).

use std::collections::BTreeMap;

use ot_proto::{
    AdminDevice, AdminUser, DeviceId, DeviceKind, DirectoryDevice, DirectoryUser, ErrorCode,
    Feature, Frame, LogEntry, Role, SignedBlob, UserId, UserState, invite_redeem_message,
};
use time::macros::datetime;

const ALICE: &str = "usr_01J9Z4A1B2C3D4E5F6G7H8J9K0";
const BOB: &str = "usr_01J9Z4B1C2D3E4F5G6H7J8K9M0";
const NOTIFIER: &str = "usr_01J9Z4C1D2E3F4G5H6J7K8M9N0";
const ALICE_PHONE: &str = "dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5";
const BOB_PHONE: &str = "dev_01J9Z3M2N4P6Q8R0S2T4V6W8X0";
const NOTIFIER_1: &str = "dev_01J9Z3N5P7Q9R1S3T5V7W9X1Y3";

fn usr(s: &str) -> UserId {
    s.parse().unwrap()
}

fn dev(s: &str) -> DeviceId {
    s.parse().unwrap()
}

fn signed(seed: u8) -> SignedBlob {
    SignedBlob { data: format!("{{\"v\":1,\"n\":{seed}}}").into_bytes(), sig: [seed; 64] }
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
fn golden_directory_frames() {
    check(
        "directory_get",
        Frame::DirectoryGet { known: BTreeMap::from([(usr(ALICE), 20), (usr(BOB), 3)]) },
    );
    check(
        "directory",
        Frame::Directory {
            me: usr(ALICE),
            admin: usr(ALICE),
            users: vec![DirectoryUser {
                id: usr(BOB),
                name: "Bob".into(),
                role: Role::Member,
                state: UserState::Active,
                cert: Some(signed(1)),
                log: vec![LogEntry { seq: 4, entry: signed(2) }],
                more: false,
                muted: false,
                devices: vec![DirectoryDevice {
                    id: dev(BOB_PHONE),
                    name: "bob-phone".into(),
                    kind: Some(DeviceKind::Phone),
                    reachable: true,
                    default: true,
                    features: vec![Feature::users_v1()],
                    online: true,
                }],
            }],
        },
    );
    check("directory_changed", Frame::DirectoryChanged);
    check("roster_publish", Frame::RosterPublish { user: usr(ALICE), seq: 21, entry: signed(3) });
    check("roster_published", Frame::RosterPublished { user: usr(ALICE), seq: 21 });
    check(
        "device_update",
        Frame::DeviceUpdate {
            device: dev(ALICE_PHONE),
            kind: Some(DeviceKind::Cli),
            reachable: Some(false),
            default: Some(false),
            features: Some(vec![Feature::users_v1()]),
        },
    );
    check("device_updated", Frame::DeviceUpdated { device: dev(ALICE_PHONE) });
    check("device_revoke", Frame::DeviceRevoke { device: dev(ALICE_PHONE) });
    check("device_revoked", Frame::DeviceRevoked { device: dev(ALICE_PHONE) });
    check("service_mute", Frame::ServiceMute { service: usr(NOTIFIER), muted: true });
    check("service_muted", Frame::ServiceMuted { service: usr(NOTIFIER), muted: true });
}

#[test]
fn golden_admin_frames() {
    check("admin_init", Frame::AdminInit { cert: signed(4), entry: signed(5) });
    check("admin_initialized", Frame::AdminInitialized { user: usr(ALICE) });
    check(
        "user_invite",
        Frame::UserInvite { name: "Bob".into(), role: Role::Member, links: vec![usr(ALICE)] },
    );
    check("user_reinvite", Frame::UserReinvite { user: usr(BOB) });
    check(
        "user_invited",
        Frame::UserInvited {
            user: usr(BOB),
            token: std::array::from_fn(|i| 40 + i as u8),
            expires_at: datetime!(2026-11-18 10:00:00 UTC),
        },
    );
    check("user_certify", Frame::UserCertify { user: usr(BOB), cert: signed(6) });
    check("user_certified", Frame::UserCertified { user: usr(BOB) });
    check(
        "service_create",
        Frame::ServiceCreate {
            user: usr(NOTIFIER),
            name: "Notifier".into(),
            device: dev(NOTIFIER_1),
            public_key: [5; 32],
            device_name: "notifier-1".into(),
            sig: [7; 64],
            cert: signed(8),
        },
    );
    check(
        "service_device",
        Frame::ServiceDevice {
            user: usr(NOTIFIER),
            device: dev(NOTIFIER_1),
            public_key: [5; 32],
            name: "notifier-1".into(),
            sig: [7; 64],
        },
    );
    check("user_revoke", Frame::UserRevoke { user: usr(BOB) });
    check("user_revoked", Frame::UserRevoked { user: usr(BOB) });
    check("user_rename", Frame::UserRename { user: usr(BOB), name: "Bobby".into() });
    check("user_renamed", Frame::UserRenamed { user: usr(BOB) });
    check("link_set", Frame::LinkSet { from: usr(NOTIFIER), to: usr(BOB), allowed: true });
    check("link_updated", Frame::LinkUpdated { from: usr(NOTIFIER), to: usr(BOB), allowed: true });
    check("users_get", Frame::UsersGet);
    check(
        "users",
        Frame::Users {
            users: vec![AdminUser {
                id: usr(BOB),
                name: "Bob".into(),
                role: Role::Member,
                state: UserState::Pending,
                created_at: 1_763_300_000_000,
                devices: vec![AdminDevice {
                    id: dev(BOB_PHONE),
                    name: "bob-phone".into(),
                    kind: None,
                    reachable: true,
                    default: true,
                    features: vec![],
                    online: false,
                    last_seen_at: Some(1_763_300_500_000),
                }],
                links: vec![usr(ALICE)],
                muted: vec![usr(NOTIFIER)],
            }],
        },
    );
    check(
        "invite_redeem",
        Frame::InviteRedeem {
            token: std::array::from_fn(|i| 40 + i as u8),
            name: "bob-phone".into(),
            public_key: [5; 32],
            sig: [7; 64],
        },
    );
    check(
        "invite_redeemed",
        Frame::InviteRedeemed { user: usr(BOB), device: dev(BOB_PHONE), inviter: dev(ALICE_PHONE) },
    );
}

#[test]
fn new_error_codes() {
    for (code, text) in [
        (ErrorCode::Forbidden, "forbidden"),
        (ErrorCode::NotAllowed, "not_allowed"),
        (ErrorCode::QuotaExceeded, "quota_exceeded"),
        (ErrorCode::UserRevoked, "user_revoked"),
        (ErrorCode::Conflict, "conflict"),
    ] {
        let json = Frame::error(code, "x", None).to_json();
        assert!(json.contains(&format!("\"code\":\"{text}\"")), "{json}");
        assert_eq!(Frame::from_json(&json).unwrap(), Frame::error(code, "x", None));
    }
}

#[test]
fn optional_fields_may_be_absent() {
    // `known` absent : tout le journal.
    assert_eq!(
        Frame::from_json(r#"{"v":1,"t":"directory_get"}"#).unwrap(),
        Frame::DirectoryGet { known: BTreeMap::new() }
    );
    // `device_update` ne porte que ce qui change.
    let only_kind =
        format!(r#"{{"v":1,"t":"device_update","device":"{ALICE_PHONE}","kind":"cli"}}"#);
    assert_eq!(
        Frame::from_json(&only_kind).unwrap(),
        Frame::DeviceUpdate {
            device: dev(ALICE_PHONE),
            kind: Some(DeviceKind::Cli),
            reachable: None,
            default: None,
            features: None,
        }
    );
    // Une catégorie inconnue (version future) se lit, sans erreur.
    let future =
        format!(r#"{{"v":1,"t":"device_update","device":"{ALICE_PHONE}","kind":"tablet"}}"#);
    assert!(matches!(
        Frame::from_json(&future).unwrap(),
        Frame::DeviceUpdate { kind: Some(DeviceKind::Unknown), .. }
    ));
    // Un champ inconnu reste une erreur.
    let extra = format!(r#"{{"v":1,"t":"device_revoke","device":"{ALICE_PHONE}","x":1}}"#);
    assert!(Frame::from_json(&extra).is_err());
}

#[test]
fn invite_redeem_message_layout() {
    let (nonce, hash) = ([9u8; 32], [3u8; 32]);
    let msg = invite_redeem_message(&nonce, &hash);
    assert_eq!(&msg[..12], b"ot-invite-v1");
    assert_eq!(&msg[12..44], &nonce);
    assert_eq!(&msg[44..], &hash);
}

#[test]
fn signed_blob_is_base64() {
    let blob = signed(1);
    let json = serde_json::to_string(&blob).unwrap();
    assert!(json.starts_with(r#"{"data":"eyJ2"#), "{json}");
    assert_eq!(serde_json::from_str::<SignedBlob>(&json).unwrap(), blob);
    assert!(serde_json::from_str::<SignedBlob>(r#"{"data":"AA==","sig":"AA=="}"#).is_err());
}
