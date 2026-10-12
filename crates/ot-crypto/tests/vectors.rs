//! Vecteurs figés : chaque fichier de `tests/vectors` fixe clés, aléa et
//! entrées, et donne l'enveloppe chiffrée attendue. Un vecteur qui casse
//! signale un changement du format chiffré : il doit être délibéré et
//! accompagné d'une note de décision.
//!
//! Pour régénérer (changement de format voulu seulement) :
//! `OT_REGENERATE_VECTORS=1 cargo test -p ot-crypto --test vectors`.

use ot_crypto::{
    Body, EpochKey, ExchangeKeyPair, GroupKey, Member, OpenKey, PairingCode, PairingOffer,
    PairingSecret, Randomness, Roster, SealKey, SigningKey, open, seal_with, token_id,
};
use ot_proto::{DeviceId, Encoding, Envelope, Payload, Ulid, b64};
use serde_json::{Value, json};
use time::macros::datetime;
use zeroize::Zeroizing;

const A: &str = "dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5";
const B: &str = "dev_01J9Z3M2N4P6Q8R0S2T4V6W8X0";
const C: &str = "dev_01J9Z3P0A2B4C6D8E0F2G4H6J8";
const MSG: &str = "01J9Z3Q5RT8YB6ZK4C2D7E9F1G";

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

/// Vecteur : nom, clé de chiffrement, enveloppe (destinataires), corps.
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
            },
        },
    ]
}

const NONCE_START: u8 = 70;
const SIGNING_SEED: u8 = 1;

fn seal_vector(v: &Vector) -> Envelope {
    let signer = SigningKey::from_bytes(&bytes(SIGNING_SEED));
    let mut env = envelope(v.to);
    let k = &v.key;
    let arr = |field: &str| b64::decode(k[field].as_str().unwrap()).unwrap();
    let mut rand = Randomness { nonce: bytes(NONCE_START), ephemeral: Zeroizing::new([0; 32]) };
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
        other => panic!("schéma inconnu : {other}"),
    };
    env.payload = payload.unwrap();
    env
}

fn open_vector(v: &Vector, env: &Envelope) -> Body {
    let k = &v.key;
    let arr = |field: &str| b64::decode(k[field].as_str().unwrap()).unwrap();
    let opened = match k["scheme"].as_str().unwrap() {
        "group" => open(env, OpenKey::Group(&GroupKey::from_bytes(arr("key").try_into().unwrap()))),
        "pairing" => open(
            env,
            OpenKey::Pairing(&PairingSecret::from_bytes(arr("secret").try_into().unwrap())),
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

        // Le fichier figé s'ouvre et redonne le corps attendu.
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
