use ot_crypto::directory::{
    CertRef, GENESIS_PREV, Genesis, RosterEntry, UserCert, anchor, cert_hash, entry_hash, log_head,
};
use ot_crypto::{
    Body, EpochKey, ExchangeKeyPair, FILE_CHUNK_SIZE, FileCipher, FileKey, GroupKey, InviteCode,
    Member, OpenKey, Otp, PairingCode, PairingOffer, PairingSecret, Randomness, Roster, SealKey,
    SigningKey, chunk_count, open, seal_with, token_id,
};
use ot_proto::{DeviceId, Encoding, Envelope, Payload, Role, SignedBlob, Ulid, UserId, b64};
use serde_json::{Value, json};
use time::macros::datetime;
use zeroize::Zeroizing;

const A: &str = "dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5";
const B: &str = "dev_01J9Z3M2N4P6Q8R0S2T4V6W8X0";
const C: &str = "dev_01J9Z3P0A2B4C6D8E0F2G4H6J8";
const MSG: &str = "01J9Z3Q5RT8YB6ZK4C2D7E9F1G";
const ALICE: &str = "usr_01J9Z4A1B2C3D4E5F6G7H8J9K0";
const BOB: &str = "usr_01J9Z4B1C2D3E4F5G6H7J8K9M0";
const NOTIFIER: &str = "usr_01J9Z4C1D2E3F4G5H6J7K8M9N0";
const BLOB: &str = "blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V";

fn dev(s: &str) -> DeviceId {
    s.parse().unwrap()
}

fn bytes<const N: usize>(start: u8) -> [u8; N] {
    std::array::from_fn(|i| start.wrapping_add(i as u8))
}

fn envelope(to: &[&str]) -> Envelope {
    Envelope {
        msg_id: Ulid::from_string(MSG).unwrap(),
        from: dev(A),
        to: to.iter().map(|s| dev(s)).collect(),
        created_at: datetime!(2026-10-10 08:00:00 UTC),
        payload: Payload { enc: Encoding::Xchacha20poly1305, data: Vec::new() },
    }
}

fn roster(epoch: u32, version: u64) -> Roster {
    let member = |id: &str, name: &str, seed: u8, added_epoch| Member {
        device_id: dev(id),
        name: name.into(),
        sign_key: SigningKey::from_bytes(&bytes(seed)).verifying_key().to_bytes(),
        exchange_key: ExchangeKeyPair::from_bytes(bytes(seed + 100)).public_key(),
        added_epoch,
    };
    Roster { epoch, version, members: vec![member(A, "mac", 1, 1), member(B, "windows", 2, 1)] }
}

struct Vector {
    name: &'static str,
    key: Value,
    to: &'static [&'static str],
    body: Body,
}

fn vectors() -> Vec<Vector> {
    vec![
        Vector {
            name: "group_text",
            key: json!({"scheme": "group", "epoch": 1, "key": b64::encode(&bytes::<32>(10))}),
            to: &[],
            body: Body::Text { body: "bonjour".into() },
        },
        Vector {
            name: "group_link",
            key: json!({"scheme": "group", "epoch": 7, "key": b64::encode(&bytes::<32>(10))}),
            to: &[B, C],
            body: Body::Link { body: "https://example.org/".into() },
        },
        Vector {
            name: "group_roster",
            key: json!({"scheme": "group", "epoch": 1, "key": b64::encode(&bytes::<32>(10))}),
            to: &[],
            body: Body::Roster { roster: roster(1, 2) },
        },
        Vector {
            name: "pairing_request",
            key: json!({"scheme": "pairing", "token": b64::encode(&bytes::<10>(20)),
                        "secret": b64::encode(&bytes::<16>(30))}),
            to: &[B],
            body: Body::PairRequest {
                name: "windows".into(),
                sign_key: SigningKey::from_bytes(&bytes(2)).verifying_key().to_bytes(),
                exchange_key: ExchangeKeyPair::from_bytes(bytes(102)).public_key(),
            },
        },
        Vector {
            name: "pairing_welcome",
            key: json!({"scheme": "pairing", "token": b64::encode(&bytes::<10>(20)),
                        "secret": b64::encode(&bytes::<16>(30))}),
            to: &[B],
            body: Body::PairWelcome {
                keys: vec![EpochKey { epoch: 1, key: GroupKey::from_bytes(bytes(10)) }],
                roster: roster(1, 2),
                anchor: Some(bytes(160)),
            },
        },
        Vector {
            name: "device_group_key",
            key: json!({"scheme": "device", "recipient_secret": b64::encode(&bytes::<32>(102)),
                        "ephemeral": b64::encode(&bytes::<32>(50))}),
            to: &[B],
            body: Body::GroupKey {
                epoch: 2,
                key: GroupKey::from_bytes(bytes(60)),
                roster: roster(2, 3),
            },
        },
        Vector {
            name: "pairing_qr_welcome",
            key: json!({"scheme": "pair_qr", "recipient_secret": b64::encode(&bytes::<32>(102)),
                        "secret": b64::encode(&bytes::<16>(30)),
                        "ephemeral": b64::encode(&bytes::<32>(50))}),
            to: &[B],
            body: Body::PairWelcome {
                keys: vec![EpochKey { epoch: 1, key: GroupKey::from_bytes(bytes(10)) }],
                roster: roster(1, 2),
                anchor: Some(bytes(160)),
            },
        },
        Vector {
            name: "group_file",
            key: json!({"scheme": "group", "epoch": 1, "key": b64::encode(&bytes::<32>(10))}),
            to: &[B],
            body: Body::File {
                blob_id: BLOB.parse().unwrap(),
                name: "été 2026.jpg".into(),
                mime: "image/jpeg".into(),
                size: 2_500_000,
                chunk_size: FILE_CHUNK_SIZE,
                chunks: 3,
                key: FileKey::from_bytes(bytes(80)),
                nonce: bytes(120),
                sha256: bytes(140),
            },
        },
        Vector {
            name: "multi_text",
            key: multi_key(&[(B, 102), (C, 103), (D, 104)]),
            to: &[B, C, D],
            body: Body::Text { body: "bonjour Bob".into() },
        },
        Vector {
            name: "multi_file",
            key: multi_key(&[(B, 102)]),
            to: &[B],
            body: Body::File {
                blob_id: BLOB.parse().unwrap(),
                name: "photo.jpg".into(),
                mime: "image/jpeg".into(),
                size: 2_500_000,
                chunk_size: FILE_CHUNK_SIZE,
                chunks: 3,
                key: FileKey::from_bytes(bytes(80)),
                nonce: bytes(120),
                sha256: bytes(140),
            },
        },
        Vector {
            name: "multi_otp",
            key: multi_key(&[(B, 102), (C, 103)]),
            to: &[B, C],
            body: Body::Otp(Otp {
                code: "482913".into(),
                display: "G-482913".into(),
                sender_name: "Example".into(),
                sender_domain: "example.org".into(),
                sender_verified: true,
                expires_at: 1_793_628_791,
            }),
        },
        Vector {
            name: "invite_request",
            key: json!({"scheme": "invite", "token": b64::encode(&bytes::<10>(20)),
                        "secret": b64::encode(&bytes::<16>(30))}),
            to: &[B],
            body: Body::InviteRequest {
                user: BOB.parse().unwrap(),
                device_id: dev(A),
                name: "bob-phone".into(),
                sign_key: SigningKey::from_bytes(&bytes(1)).verifying_key().to_bytes(),
                exchange_key: ExchangeKeyPair::from_bytes(bytes(101)).public_key(),
            },
        },
        Vector {
            name: "invite_welcome",
            key: json!({"scheme": "invite", "token": b64::encode(&bytes::<10>(20)),
                        "secret": b64::encode(&bytes::<16>(30))}),
            to: &[B],
            body: Body::InviteWelcome { cert: member_cert(), admin_head: admin_head() },
        },
        Vector {
            name: "service_welcome",
            key: json!({"scheme": "pair_qr", "recipient_secret": b64::encode(&bytes::<32>(102)),
                        "secret": b64::encode(&bytes::<16>(30)),
                        "ephemeral": b64::encode(&bytes::<32>(50))}),
            to: &[B],
            body: Body::ServiceWelcome {
                user: NOTIFIER.parse().unwrap(),
                cert: service_cert(),
                anchor: anchor(&admin_cert()),
                admin_head: admin_head(),
            },
        },
    ]
}

const D: &str = "dev_01J9Z3R1S3T5V7W9X1Y3Z5A7B9";

fn multi_key(recipients: &[(&str, u8)]) -> Value {
    let list: Vec<Value> = recipients
        .iter()
        .map(|(d, seed)| json!({"device": d, "secret": b64::encode(&bytes::<32>(*seed))}))
        .collect();
    json!({"scheme": "multi", "recipients": list,
           "ephemeral": b64::encode(&bytes::<32>(50)),
           "message_key": b64::encode(&bytes::<32>(90))})
}

fn admin_cert() -> SignedBlob {
    let key = SigningKey::from_bytes(&bytes(1));
    let genesis = Genesis {
        device_id: dev(A),
        name: "alice-phone".into(),
        sign_key: key.verifying_key().to_bytes(),
        exchange_key: ExchangeKeyPair::from_bytes(bytes(101)).public_key(),
    };
    UserCert::new(ALICE.parse().unwrap(), "Alice", Role::Admin, genesis, 1_763_300_000, dev(A))
        .sign(&key)
        .unwrap()
}

fn member_cert() -> SignedBlob {
    let genesis = Genesis {
        device_id: dev(B),
        name: "bob-phone".into(),
        sign_key: SigningKey::from_bytes(&bytes(2)).verifying_key().to_bytes(),
        exchange_key: ExchangeKeyPair::from_bytes(bytes(102)).public_key(),
    };
    UserCert::new(BOB.parse().unwrap(), "Bob", Role::Member, genesis, 1_763_300_100, dev(A))
        .sign(&SigningKey::from_bytes(&bytes(1)))
        .unwrap()
}

fn service_cert() -> SignedBlob {
    let genesis = Genesis {
        device_id: dev(C),
        name: "notifier-1".into(),
        sign_key: SigningKey::from_bytes(&bytes(3)).verifying_key().to_bytes(),
        exchange_key: ExchangeKeyPair::from_bytes(bytes(103)).public_key(),
    };
    UserCert::new(
        NOTIFIER.parse().unwrap(),
        "Notifier",
        Role::Service,
        genesis,
        1_763_300_200,
        dev(A),
    )
    .sign(&SigningKey::from_bytes(&bytes(1)))
    .unwrap()
}

fn member(id: &str, name: &str, seed: u8) -> Member {
    Member {
        device_id: dev(id),
        name: name.into(),
        sign_key: SigningKey::from_bytes(&bytes(seed)).verifying_key().to_bytes(),
        exchange_key: ExchangeKeyPair::from_bytes(bytes(seed + 100)).public_key(),
        added_epoch: 1,
    }
}

fn admin_log() -> Vec<SignedBlob> {
    let alice = SigningKey::from_bytes(&bytes(1));
    let user: UserId = ALICE.parse().unwrap();
    let roster = |version| Roster { epoch: 1, version, members: vec![member(A, "alice-phone", 1)] };
    let e1 =
        RosterEntry::new(user.clone(), 1, GENESIS_PREV, dev(A), roster(1)).sign(&alice).unwrap();
    let certs = vec![
        CertRef { user: BOB.parse().unwrap(), cert: cert_hash(&member_cert()) },
        CertRef { user: NOTIFIER.parse().unwrap(), cert: cert_hash(&service_cert()) },
    ];
    let e2 = RosterEntry::new(user, 2, entry_hash(&e1), dev(A), roster(2))
        .with_certs(certs)
        .sign(&alice)
        .unwrap();
    vec![e1, e2]
}

fn admin_head() -> ot_crypto::directory::LogHead {
    log_head(admin_log().last().unwrap()).unwrap()
}

const NONCE_START: u8 = 70;
const SIGNING_SEED: u8 = 1;

fn seal_vector(v: &Vector) -> Envelope {
    let signer = SigningKey::from_bytes(&bytes(SIGNING_SEED));
    let mut env = envelope(v.to);
    let k = &v.key;
    let arr = |field: &str| b64::decode(k[field].as_str().unwrap()).unwrap();
    let mut rand = Randomness {
        nonce: bytes(NONCE_START),
        ephemeral: Zeroizing::new([0; 32]),
        message_key: Zeroizing::new([0; 32]),
    };
    let payload = match k["scheme"].as_str().unwrap() {
        "group" => {
            let key = GroupKey::from_bytes(arr("key").try_into().unwrap());
            let epoch = k["epoch"].as_u64().unwrap() as u32;
            seal_with(&env, SealKey::Group { epoch, key: &key }, &signer, &v.body, rand)
        }
        "pairing" => {
            let secret = PairingSecret::from_bytes(arr("secret").try_into().unwrap());
            let token_id = token_id(&arr("token").try_into().unwrap());
            seal_with(&env, SealKey::Pairing { token_id, secret: &secret }, &signer, &v.body, rand)
        }
        "device" => {
            let recipient =
                ExchangeKeyPair::from_bytes(arr("recipient_secret").try_into().unwrap());
            *rand.ephemeral = arr("ephemeral").try_into().unwrap();
            let key = SealKey::Device { recipient: recipient.public_key() };
            seal_with(&env, key, &signer, &v.body, rand)
        }
        "pair_qr" => {
            let recipient =
                ExchangeKeyPair::from_bytes(arr("recipient_secret").try_into().unwrap());
            let secret = PairingSecret::from_bytes(arr("secret").try_into().unwrap());
            *rand.ephemeral = arr("ephemeral").try_into().unwrap();
            let key = SealKey::PairQr { recipient: recipient.public_key(), secret: &secret };
            seal_with(&env, key, &signer, &v.body, rand)
        }
        "invite" => {
            let secret = PairingSecret::from_bytes(arr("secret").try_into().unwrap());
            let token_id = token_id(&arr("token").try_into().unwrap());
            seal_with(&env, SealKey::Invite { token_id, secret: &secret }, &signer, &v.body, rand)
        }
        "multi" => {
            *rand.ephemeral = arr("ephemeral").try_into().unwrap();
            *rand.message_key = arr("message_key").try_into().unwrap();
            let recipients: Vec<(DeviceId, [u8; 32])> =
                multi_recipients(k).into_iter().map(|(d, pair)| (d, pair.public_key())).collect();
            seal_with(&env, SealKey::Multi { recipients: &recipients }, &signer, &v.body, rand)
        }
        other => panic!("schéma inconnu : {other}"),
    };
    env.payload = payload.unwrap();
    env
}

fn multi_recipients(k: &Value) -> Vec<(DeviceId, ExchangeKeyPair)> {
    k["recipients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let secret = b64::decode(r["secret"].as_str().unwrap()).unwrap();
            (
                dev(r["device"].as_str().unwrap()),
                ExchangeKeyPair::from_bytes(secret.try_into().unwrap()),
            )
        })
        .collect()
}

fn open_vector(v: &Vector, env: &Envelope) -> Body {
    let k = &v.key;
    let arr = |field: &str| b64::decode(k[field].as_str().unwrap()).unwrap();
    if k["scheme"] == "multi" {
        let signer = SigningKey::from_bytes(&bytes(SIGNING_SEED)).verifying_key().to_bytes();
        let bodies: Vec<Body> = multi_recipients(k)
            .iter()
            .map(|(me, key)| {
                let opened = open(env, OpenKey::Multi { me, key }).unwrap();
                opened.verify(&signer).unwrap();
                opened.body().unwrap()
            })
            .collect();
        assert!(bodies.windows(2).all(|w| w[0] == w[1]));
        return bodies.into_iter().next().unwrap();
    }
    let opened = match k["scheme"].as_str().unwrap() {
        "group" => open(env, OpenKey::Group(&GroupKey::from_bytes(arr("key").try_into().unwrap()))),
        "pairing" => open(
            env,
            OpenKey::Pairing(&PairingSecret::from_bytes(arr("secret").try_into().unwrap())),
        ),
        "invite" => open(
            env,
            OpenKey::Invite(&PairingSecret::from_bytes(arr("secret").try_into().unwrap())),
        ),
        "pair_qr" => open(
            env,
            OpenKey::PairQr(
                &ExchangeKeyPair::from_bytes(arr("recipient_secret").try_into().unwrap()),
                &PairingSecret::from_bytes(arr("secret").try_into().unwrap()),
            ),
        ),
        _ => open(
            env,
            OpenKey::Device(&ExchangeKeyPair::from_bytes(
                arr("recipient_secret").try_into().unwrap(),
            )),
        ),
    }
    .unwrap();
    opened
        .verify(&SigningKey::from_bytes(&bytes(SIGNING_SEED)).verifying_key().to_bytes())
        .unwrap();
    opened.body().unwrap()
}

fn render(v: &Vector, env: &Envelope) -> String {
    let doc = json!({
        "signing_seed": b64::encode(&bytes::<32>(SIGNING_SEED)),
        "nonce": b64::encode(&bytes::<24>(NONCE_START)),
        "key": v.key,
        "body": serde_json::to_value(&v.body).unwrap(),
        "envelope": serde_json::to_value(env).unwrap(),
    });
    serde_json::to_string_pretty(&doc).unwrap() + "\n"
}

fn path(name: &str) -> String {
    format!("{}/tests/vectors/{name}.json", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn frozen_vectors() {
    let regenerate = std::env::var_os("OT_REGENERATE_VECTORS").is_some();
    for v in vectors() {
        let env = seal_vector(&v);
        let text = render(&v, &env);
        if regenerate {
            std::fs::write(path(v.name), &text).unwrap();
            continue;
        }
        let expected =
            std::fs::read_to_string(path(v.name)).unwrap_or_else(|e| panic!("{} : {e}", v.name));
        assert_eq!(text, expected, "vecteur {} modifié : le format chiffré a changé", v.name);

        let doc: Value = serde_json::from_str(&expected).unwrap();
        let frozen: Envelope = serde_json::from_value(doc["envelope"].clone()).unwrap();
        assert_eq!(open_vector(&v, &frozen), v.body, "{}", v.name);
        assert_eq!(serde_json::from_value::<Body>(doc["body"].clone()).unwrap(), v.body);
    }
}

#[test]
fn frozen_pairing_code() {
    let code = PairingCode { token: bytes(20), secret: PairingSecret::from_bytes(bytes(30)) };
    let text = code.encode();
    let file = path("pairing_code");
    if std::env::var_os("OT_REGENERATE_VECTORS").is_some() {
        let doc = json!({
            "token": b64::encode(&code.token),
            "secret": b64::encode(code.secret.as_bytes()),
            "token_id": b64::encode(&code.token_id()),
            "code": &*text,
        });
        std::fs::write(&file, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        return;
    }
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(doc["code"].as_str().unwrap(), &*text);
    assert_eq!(doc["token_id"].as_str().unwrap(), b64::encode(&code.token_id()));
    assert_eq!(PairingCode::parse(doc["code"].as_str().unwrap()).unwrap(), code);
}

#[test]
fn frozen_invite_code() {
    let code = InviteCode {
        token: bytes(20),
        secret: PairingSecret::from_bytes(bytes(30)),
        anchor: anchor(&admin_cert()),
    };
    let text = code.encode();
    let file = path("invite_code");
    if std::env::var_os("OT_REGENERATE_VECTORS").is_some() {
        let doc = json!({
            "token": b64::encode(&code.token),
            "secret": b64::encode(code.secret.as_bytes()),
            "anchor": b64::encode(&code.anchor),
            "token_id": b64::encode(&code.token_id()),
            "code": &*text,
        });
        std::fs::write(&file, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        return;
    }
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(doc["code"].as_str().unwrap(), &*text, "format du code d'invitation modifié");
    assert_eq!(doc["token_id"].as_str().unwrap(), b64::encode(&code.token_id()));
    assert_eq!(InviteCode::parse(doc["code"].as_str().unwrap()).unwrap(), code);
}

#[test]
fn frozen_directory_objects() {
    let alice = SigningKey::from_bytes(&bytes(1));
    let bob = SigningKey::from_bytes(&bytes(2));
    let bob_user: UserId = BOB.parse().unwrap();
    let roster1 = Roster { epoch: 1, version: 1, members: vec![member(B, "bob-phone", 2)] };
    let e1 =
        RosterEntry::new(bob_user.clone(), 1, GENESIS_PREV, dev(B), roster1).sign(&bob).unwrap();
    let roster2 = Roster {
        epoch: 1,
        version: 2,
        members: vec![member(B, "bob-phone", 2), member(D, "bob-laptop", 4)],
    };
    let e2 = RosterEntry::new(bob_user, 2, entry_hash(&e1), dev(B), roster2).sign(&bob).unwrap();
    let blob = |b: &SignedBlob| json!({"data": b64::encode(&b.data), "sig": b64::encode(&b.sig)});
    let doc = json!({
        "alice_signing_seed": b64::encode(&bytes::<32>(1)),
        "bob_signing_seed": b64::encode(&bytes::<32>(2)),
        "admin_cert": blob(&admin_cert()),
        "anchor": b64::encode(&anchor(&admin_cert())),
        "member_cert": blob(&member_cert()),
        "service_cert": blob(&service_cert()),
        "bob_log": [blob(&e1), blob(&e2)],
        "bob_log_hashes": [b64::encode(&entry_hash(&e1)), b64::encode(&entry_hash(&e2))],
        "alice_log": admin_log().iter().map(blob).collect::<Vec<_>>(),
        "alice_log_hashes": admin_log().iter().map(|e| b64::encode(&entry_hash(e))).collect::<Vec<_>>(),
        "member_cert_hash": b64::encode(&cert_hash(&member_cert())),
        "service_cert_hash": b64::encode(&cert_hash(&service_cert())),
    });
    let text = serde_json::to_string_pretty(&doc).unwrap() + "\n";
    let file = path("directory_objects");
    if std::env::var_os("OT_REGENERATE_VECTORS").is_some() {
        std::fs::write(&file, &text).unwrap();
        return;
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "format de l'annuaire signé modifié");
    let alice_pub = alice.verifying_key().to_bytes();
    let root = ot_crypto::directory::verify_root(&admin_cert(), &anchor(&admin_cert())).unwrap();
    assert_eq!(root.genesis.sign_key, alice_pub);
    assert_eq!(UserCert::open(&member_cert(), &alice_pub).unwrap().name, "Bob");
    let bob_pub = bob.verifying_key().to_bytes();
    assert_eq!(RosterEntry::open(&e2, &bob_pub).unwrap().prev, entry_hash(&e1));
    let head = RosterEntry::open(&admin_log()[1], &alice_pub).unwrap();
    assert_eq!(head.cert_of(&BOB.parse().unwrap()), Some(cert_hash(&member_cert())));
    assert_eq!(head.cert_of(&ALICE.parse().unwrap()), None, "la racine vaut par l'ancre");
}

#[test]
fn frozen_pairing_offer() {
    let signer = SigningKey::from_bytes(&bytes(2));
    let offer = PairingOffer::new(
        dev(B),
        "windows",
        &signer,
        ExchangeKeyPair::from_bytes(bytes(102)).public_key(),
        PairingSecret::from_bytes(bytes(30)),
    )
    .unwrap();
    let text = offer.encode();
    let file = path("pairing_offer");
    if std::env::var_os("OT_REGENERATE_VECTORS").is_some() {
        let doc = json!({
            "device_id": B,
            "name": "windows",
            "signing_seed": b64::encode(&bytes::<32>(2)),
            "exchange_secret": b64::encode(&bytes::<32>(102)),
            "secret": b64::encode(&bytes::<16>(30)),
            "qr": &*text,
        });
        std::fs::write(&file, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        return;
    }
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(doc["qr"].as_str().unwrap(), &*text, "format du QR inversé modifié");
    assert_eq!(PairingOffer::parse(doc["qr"].as_str().unwrap()).unwrap(), offer);
}

#[test]
fn frozen_file_chunks() {
    const CHUNK: u32 = 16;
    let key = FileKey::from_bytes(bytes(80));
    let nonce = bytes::<19>(120);
    let plain: Vec<u8> = (0..40).collect();
    let cipher = FileCipher::new(&key, &nonce);
    let chunks = chunk_count(plain.len() as u64, CHUNK).unwrap();
    let sealed: Vec<String> = plain
        .chunks(CHUNK as usize)
        .enumerate()
        .map(|(i, c)| {
            let i = i as u32;
            b64::encode(&cipher.seal_chunk(i, i == chunks - 1, c).unwrap())
        })
        .collect();
    let file = path("file_chunks");
    if std::env::var_os("OT_REGENERATE_VECTORS").is_some() {
        let doc = json!({
            "key": b64::encode(key.as_bytes()),
            "nonce": b64::encode(&nonce),
            "chunk_size": CHUNK,
            "plaintext": b64::encode(&plain),
            "chunks": sealed,
        });
        std::fs::write(&file, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
        return;
    }
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let frozen: Vec<String> = serde_json::from_value(doc["chunks"].clone()).unwrap();
    assert_eq!(frozen, sealed, "format chiffré des fichiers modifié");
    let mut opened = Vec::new();
    for (i, c) in frozen.iter().enumerate() {
        let i = i as u32;
        opened.extend_from_slice(
            &cipher.open_chunk(i, i == chunks - 1, &b64::decode(c).unwrap()).unwrap(),
        );
    }
    assert_eq!(opened, plain);
}
