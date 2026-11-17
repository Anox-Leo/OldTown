//! Schéma `0x05` (étape 10, note 0052) : une clé de message scellée pour
//! chaque appareil destinataire. Aller-retour, destinataire absent, table et
//! `env.to` altérés, limites.

use ot_crypto::{
    Body, CryptoError, ExchangeKeyPair, Header, OpenKey, SealKey, SigningKey, open, seal,
};
use ot_proto::{DeviceId, Encoding, Envelope, Payload, Ulid};

struct Dest {
    id: DeviceId,
    key: ExchangeKeyPair,
}

fn dests(n: usize) -> Vec<Dest> {
    (0..n).map(|_| Dest { id: DeviceId::generate(), key: ExchangeKeyPair::generate() }).collect()
}

fn envelope(to: &[Dest]) -> Envelope {
    Envelope {
        msg_id: Ulid::generate(),
        from: DeviceId::generate(),
        to: to.iter().map(|d| d.id.clone()).collect(),
        created_at: time::OffsetDateTime::now_utc().replace_nanosecond(0).unwrap(),
        payload: Payload { enc: Encoding::Xchacha20poly1305, data: Vec::new() },
    }
}

fn table(to: &[Dest]) -> Vec<(DeviceId, [u8; 32])> {
    to.iter().map(|d| (d.id.clone(), d.key.public_key())).collect()
}

fn signer() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}

fn body() -> Body {
    Body::Text { body: "bonjour".into() }
}

fn sealed(to: &[Dest]) -> Envelope {
    let mut env = envelope(to);
    env.payload =
        seal(&env, SealKey::Multi { recipients: &table(to) }, &signer(), &body()).unwrap();
    env
}

fn open_as(env: &Envelope, d: &Dest) -> Result<Body, CryptoError> {
    let opened = open(env, OpenKey::Multi { me: &d.id, key: &d.key })?;
    opened.verify(&signer().verifying_key().to_bytes())?;
    opened.body()
}

#[test]
fn every_recipient_opens_the_same_message() {
    let to = dests(3);
    let env = sealed(&to);
    for d in &to {
        assert_eq!(open_as(&env, d).unwrap(), body());
    }
    let Header::Multi { recipients, .. } = Header::parse(&env.payload).unwrap() else {
        panic!("en-tête 0x05 attendu")
    };
    assert_eq!(recipients.len(), 3);
    assert_eq!(Header::parse(&env.payload).unwrap().recipients(), env.to);
    // 64 octets par destinataire.
    let one = sealed(&dests(1));
    assert_eq!(env.payload.data.len() - one.payload.data.len(), 2 * 64);
}

#[test]
fn an_absent_recipient_cannot_open() {
    let to = dests(2);
    let env = sealed(&to);
    let stranger = &dests(1)[0];
    assert_eq!(open_as(&env, stranger), Err(CryptoError::Decrypt));
    // Mon identifiant, mais pas ma clé.
    let thief = Dest { id: to[0].id.clone(), key: ExchangeKeyPair::generate() };
    assert_eq!(open_as(&env, &thief), Err(CryptoError::Decrypt));
    // La signature est celle de l'expéditeur, pas d'un autre.
    let opened = open(&env, OpenKey::Multi { me: &to[0].id, key: &to[0].key }).unwrap();
    let other = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
    assert_eq!(opened.verify(&other), Err(CryptoError::BadSignature));
}

#[test]
fn an_altered_table_or_recipient_list_is_refused() {
    let to = dests(3);
    let env = sealed(&to);

    // Une clé scellée altérée : son destinataire échoue, et les autres aussi
    // (la table entière est dans les données associées).
    let mut bad = env.clone();
    bad.payload.data[34 + 16] ^= 1;
    for d in &to {
        assert!(open_as(&bad, d).is_err());
    }

    // `env.to` réordonné ou amputé par le relais : refusé.
    let mut reordered = env.clone();
    reordered.to.swap(0, 1);
    for d in &to {
        assert!(matches!(open_as(&reordered, d), Err(CryptoError::Malformed(_))));
    }
    let mut shorter = env.clone();
    shorter.to.pop();
    assert!(matches!(open_as(&shorter, &to[0]), Err(CryptoError::Malformed(_))));

    // Le relais retire proprement un destinataire de la table et de `to` :
    // les données associées changent, plus personne n'ouvre.
    let mut stripped = env.clone();
    let mut data = stripped.payload.data[..33].to_vec();
    data.push(2);
    data.extend_from_slice(&env.payload.data[34..34 + 2 * 64]);
    data.extend_from_slice(&env.payload.data[34 + 3 * 64..]);
    stripped.payload.data = data;
    stripped.to.pop();
    for d in &to[..2] {
        assert_eq!(open_as(&stripped, d), Err(CryptoError::Decrypt));
    }

    // Un destinataire ajouté à la table (copie d'une ligne) : en double.
    let mut dup = env.clone();
    let mut data = dup.payload.data[..33].to_vec();
    data.push(4);
    data.extend_from_slice(&env.payload.data[34..34 + 3 * 64]);
    data.extend_from_slice(&env.payload.data[34..34 + 64]);
    data.extend_from_slice(&env.payload.data[34 + 3 * 64..]);
    dup.payload.data = data;
    dup.to.push(to[0].id.clone());
    assert!(matches!(Header::parse(&dup.payload), Err(CryptoError::Malformed(_))));
}

#[test]
fn limits_and_shape() {
    let sk = signer();
    // 33 destinataires : refusé.
    let many = dests(33);
    let env = envelope(&many);
    let res = seal(&env, SealKey::Multi { recipients: &table(&many) }, &sk, &body());
    assert!(matches!(res, Err(CryptoError::Malformed(_))));
    // 32 : accepté.
    let env = sealed(&dests(32));
    assert!(Header::parse(&env.payload).is_ok());
    // n = 0 ou 33 à la lecture.
    for n in [0u8, 33] {
        let mut bad = env.clone();
        bad.payload.data[33] = n;
        assert!(matches!(Header::parse(&bad.payload), Err(CryptoError::Malformed(_))));
    }
    // En double, ou `env.to` différent de la table, au scellement.
    let to = dests(2);
    let twice = vec![table(&to)[0].clone(), table(&to)[0].clone()];
    let mut env = envelope(&to);
    env.to = vec![to[0].id.clone(), to[0].id.clone()];
    let res = seal(&env, SealKey::Multi { recipients: &twice }, &sk, &body());
    assert!(matches!(res, Err(CryptoError::Malformed(_))));
    let env = envelope(&to);
    let res = seal(&env, SealKey::Multi { recipients: &table(&to)[..1] }, &sk, &body());
    assert!(matches!(res, Err(CryptoError::Malformed(_))));
    let res = seal(&envelope(&[]), SealKey::Multi { recipients: &[] }, &sk, &body());
    assert!(matches!(res, Err(CryptoError::Malformed(_))));
    // Clé de faible ordre.
    let weak = vec![(to[0].id.clone(), [0u8; 32])];
    let env = envelope(&to[..1]);
    assert_eq!(
        seal(&env, SealKey::Multi { recipients: &weak }, &sk, &body()).err(),
        Some(CryptoError::WeakKey)
    );
}

#[test]
fn only_person_to_person_bodies_are_admitted() {
    let to = dests(1);
    let env = envelope(&to);
    let roster =
        Body::Roster { roster: ot_crypto::Roster { epoch: 1, version: 1, members: vec![] } };
    let res = seal(&env, SealKey::Multi { recipients: &table(&to) }, &signer(), &roster);
    assert!(matches!(res, Err(CryptoError::Malformed(_))));
    let link = Body::Link { body: "https://example.org/".into() };
    let mut env = envelope(&to);
    env.payload = seal(&env, SealKey::Multi { recipients: &table(&to) }, &signer(), &link).unwrap();
    assert_eq!(open_as(&env, &to[0]).unwrap(), link);
}

#[test]
fn fresh_ephemeral_and_message_key_each_time() {
    let to = dests(2);
    let env = envelope(&to);
    let k = table(&to);
    let a = seal(&env, SealKey::Multi { recipients: &k }, &signer(), &body()).unwrap();
    let b = seal(&env, SealKey::Multi { recipients: &k }, &signer(), &body()).unwrap();
    assert_ne!(a.data[1..33], b.data[1..33], "clé éphémère neuve");
    assert_ne!(a.data[50..98], b.data[50..98], "clé scellée différente");
}
