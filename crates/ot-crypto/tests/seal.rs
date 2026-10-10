//! Aller-retour et altérations, pour les trois schémas.

use ot_crypto::{
    Body, CryptoError, ExchangeKeyPair, GroupKey, Header, OpenKey, PairingSecret, SealKey,
    SigningKey, open, seal,
};
use ot_proto::{Content, DeviceId, Encoding, Envelope, Payload, Ulid};
use time::Duration;

fn envelope() -> Envelope {
    let mut env = Envelope::new_plain(
        DeviceId::generate(),
        vec![DeviceId::generate(), DeviceId::generate()],
        &Content::text("x"),
    );
    env.payload = Payload { enc: Encoding::Xchacha20poly1305, data: Vec::new() };
    env
}

fn body() -> Body {
    Body::Text { body: "bonjour".into() }
}

type SealFn = Box<dyn Fn(&Envelope, &SigningKey) -> Payload>;
type OpenFn = Box<dyn Fn(&Envelope) -> Result<ot_crypto::Opened, CryptoError>>;
type Tamper = Box<dyn Fn(&mut Envelope)>;

struct Case {
    seal: SealFn,
    open: OpenFn,
    /// Ouverture avec une autre clé du même type.
    open_wrong: OpenFn,
}

fn cases() -> Vec<(&'static str, Case)> {
    let group = GroupKey::generate();
    let other_group = GroupKey::generate();
    let secret = PairingSecret::generate();
    let other_secret = PairingSecret::generate();
    let recipient = ExchangeKeyPair::generate();
    let other_recipient = ExchangeKeyPair::generate();
    let recipient_pub = recipient.public_key();
    vec![
        (
            "groupe",
            Case {
                seal: Box::new({
                    let k = group.clone();
                    move |env, sk| {
                        seal(env, SealKey::Group { epoch: 3, key: &k }, sk, &body()).unwrap()
                    }
                }),
                open: Box::new({
                    let k = group.clone();
                    move |env| open(env, OpenKey::Group(&k))
                }),
                open_wrong: Box::new(move |env| open(env, OpenKey::Group(&other_group))),
            },
        ),
        (
            "appairage",
            Case {
                seal: Box::new({
                    let s = secret.clone();
                    move |env, sk| {
                        seal(env, SealKey::Pairing { token_id: [9; 8], secret: &s }, sk, &body())
                            .unwrap()
                    }
                }),
                open: Box::new({
                    let s = secret.clone();
                    move |env| open(env, OpenKey::Pairing(&s))
                }),
                open_wrong: Box::new(move |env| open(env, OpenKey::Pairing(&other_secret))),
            },
        ),
        (
            "appareil",
            Case {
                seal: Box::new(move |env, sk| {
                    seal(env, SealKey::Device { recipient: recipient_pub }, sk, &body()).unwrap()
                }),
                open: Box::new(move |env| open(env, OpenKey::Device(&recipient))),
                open_wrong: Box::new(move |env| open(env, OpenKey::Device(&other_recipient))),
            },
        ),
    ]
}

fn signer() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}

#[test]
fn roundtrip_and_signature() {
    let sk = signer();
    for (name, case) in cases() {
        let mut env = envelope();
        env.payload = (case.seal)(&env, &sk);
        let opened = (case.open)(&env).unwrap_or_else(|e| panic!("{name} : {e}"));
        assert_eq!(opened.body().unwrap(), body(), "{name}");
        opened.verify(&sk.verifying_key().to_bytes()).unwrap();
        let other = SigningKey::from_bytes(&[43; 32]).verifying_key().to_bytes();
        assert_eq!(opened.verify(&other), Err(CryptoError::BadSignature), "{name}");
        assert!(Header::parse(&env.payload).is_ok());
    }
}

#[test]
fn every_tampering_is_rejected() {
    let sk = signer();
    for (name, case) in cases() {
        let mut env = envelope();
        env.payload = (case.seal)(&env, &sk);
        let header_len = match Header::parse(&env.payload).unwrap() {
            Header::Group { .. } => 5,
            Header::Pairing { .. } => 9,
            Header::Device { .. } => 33,
        };
        let tampered: Vec<(&str, Tamper)> = vec![
            ("en-tête", Box::new(|e: &mut Envelope| e.payload.data[1] ^= 1)),
            ("nonce", Box::new(move |e: &mut Envelope| e.payload.data[header_len] ^= 1)),
            ("chiffré", Box::new(|e: &mut Envelope| *e.payload.data.last_mut().unwrap() ^= 1)),
            ("msg_id", Box::new(|e: &mut Envelope| e.msg_id = Ulid::generate())),
            ("from", Box::new(|e: &mut Envelope| e.from = DeviceId::generate())),
            ("to", Box::new(|e: &mut Envelope| e.to.reverse())),
            ("to vide", Box::new(|e: &mut Envelope| e.to.clear())),
            ("created_at", Box::new(|e: &mut Envelope| e.created_at += Duration::SECOND)),
        ];
        for (what, tamper) in tampered {
            let mut bad = env.clone();
            tamper(&mut bad);
            assert!((case.open)(&bad).is_err(), "{name} : altération de {what} acceptée");
        }
        assert_eq!((case.open_wrong)(&env).err(), Some(CryptoError::Decrypt), "{name}");
    }
}

#[test]
fn plaintext_and_garbage_are_refused() {
    let key = GroupKey::generate();
    let mut env = envelope();
    env.payload = Payload::plain(&Content::text("en clair"));
    assert_eq!(open(&env, OpenKey::Group(&key)).err(), Some(CryptoError::Plaintext));
    assert_eq!(Header::parse(&env.payload), Err(CryptoError::Plaintext));

    for data in [vec![], vec![1, 0, 0, 0, 1], vec![7; 200], vec![1; 40]] {
        env.payload = Payload { enc: Encoding::Xchacha20poly1305, data };
        assert!(open(&env, OpenKey::Group(&key)).is_err());
    }
}

#[test]
fn key_of_another_scheme_is_refused() {
    let sk = signer();
    let key = GroupKey::generate();
    let mut env = envelope();
    env.payload = seal(&env, SealKey::Group { epoch: 1, key: &key }, &sk, &body()).unwrap();
    let secret = PairingSecret::generate();
    assert_eq!(open(&env, OpenKey::Pairing(&secret)).err(), Some(CryptoError::Decrypt));
}

#[test]
fn low_order_recipient_is_refused() {
    let env = envelope();
    // Point d'ordre faible : l'échange donnerait un secret nul.
    let res = seal(&env, SealKey::Device { recipient: [0; 32] }, &signer(), &body());
    assert_eq!(res.err(), Some(CryptoError::WeakKey));
}

#[test]
fn nonces_are_fresh() {
    let sk = signer();
    let key = GroupKey::generate();
    let env = envelope();
    let a = seal(&env, SealKey::Group { epoch: 1, key: &key }, &sk, &body()).unwrap();
    let b = seal(&env, SealKey::Group { epoch: 1, key: &key }, &sk, &body()).unwrap();
    assert_ne!(a.data, b.data);
}
