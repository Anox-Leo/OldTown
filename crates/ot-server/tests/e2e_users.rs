//! Étape 10, T7 : envois entre personnes (`0x05`) de bout en bout, clients
//! `ot-core` et relais en processus. Noms neutres : Alice (admin, deux
//! appareils), Bob (membre, trois appareils, dont un jamais connecté et
//! seul par défaut) et Carol (membre, liée à Alice seulement).

mod common;

use std::sync::Arc;

use common::*;
use ed25519_dalek::SigningKey;
use ot_core::crypto::directory::{
    CertRef, GENESIS_PREV, Genesis, RosterEntry, UserCert, anchor, cert_hash, entry_hash,
};
use ot_core::directory::DirectoryState;
use ot_core::{
    Client, ClientConfig, ClientError, Content, DeviceKind, Event, Events, Keyring, Origin,
    OutgoingFile, Person, Target, ensure_exchange_key,
};
use ot_proto::{Role, UserId};
use ot_server::store::Store;
use ot_server::users::DeviceChanges;
use ot_server::{Config, token_hash};
use tempfile::TempDir;

const HOUR: i64 = 3600 * 1000;

fn key(dev: &TestDevice) -> SigningKey {
    SigningKey::from_bytes(&dev.identity.to_bytes())
}

struct Net {
    server: TestServer,
    _dir: TempDir,
    alice: UserId,
    bob: UserId,
    carol: UserId,
    a: Vec<TestDevice>,
    b: Vec<TestDevice>,
    c: Vec<TestDevice>,
}

/// Une personne invitée par `by`, liée à `links`, rachetée par son premier
/// appareil (`names[0]`), avec ses autres appareils et son groupe, et son
/// certificat signé par `by` (pas encore engagé par le journal de l'admin).
async fn invite(
    store: &Store,
    by: &TestDevice,
    name: &str,
    links: &[UserId],
    names: &[&str],
    seed: u8,
) -> (UserId, Vec<TestDevice>, ot_proto::SignedBlob) {
    let identity = ot_core::Identity::generate();
    let hash = token_hash(&[seed; 10]);
    let user = store.invite_user(&by.id, name, links, &hash, 5, 0, 48 * HOUR).await.unwrap();
    let r = store.redeem_invitation(&hash, names[0], &identity.public_key(), 1).await.unwrap();
    let first = TestDevice {
        id: r.device.id,
        name: names[0].into(),
        identity,
        keyring: Arc::new(ot_core::MemoryKeyring::new()),
    };
    let mut devs = vec![first];
    for n in &names[1..] {
        devs.push(add_device_for(store, &user, n).await);
    }
    make_group(&devs.iter().collect::<Vec<_>>());
    let genesis = Genesis {
        device_id: devs[0].id.clone(),
        name: devs[0].name.clone(),
        sign_key: devs[0].identity.public_key(),
        exchange_key: ensure_exchange_key(&*devs[0].keyring).unwrap().public_key(),
    };
    let cert = UserCert::new(user.clone(), name, Role::Member, genesis, 1, by.id.clone())
        .sign(&key(by))
        .unwrap();
    store.certify_user(&user, &cert).await.unwrap();
    (user, devs, cert)
}

/// Alice publie sa racine par son client ; Bob et Carol sont invités,
/// certifiés par Alice (une entrée de son journal engage leurs certificats)
/// et publient leur journal, comme à la fin d'une invitation (T8). Liens :
/// Alice ↔ Bob, Alice ↔ Carol, rien entre Bob et Carol.
async fn net() -> Net {
    let dir = TempDir::new().unwrap();
    let store = Store::in_memory().await.unwrap();
    let a1 = add_device(&store, "alice-phone").await;
    let a2 = add_device(&store, "alice-laptop").await;
    make_group(&[&a1, &a2]);
    let config = Config { blob_dir: dir.path().join("blobs"), ..test_config() };
    let server = TestServer::start(store, config).await;
    let (c1, mut e1) = a1.client(&server.url());
    wait_connected(&mut e1).await;
    let alice = within(c1.admin_init()).await.unwrap();
    within(c1.shutdown()).await;

    let store = server.relay.store();
    let page = store.directory(&alice, &Default::default(), None).await.unwrap();
    let root = page.users[0].cert.clone().unwrap();
    let entry1 = page.users[0].log[0].entry.clone();

    let links = [alice.clone()];
    let names = ["bob-phone", "bob-laptop", "bob-tablet"];
    let (bob, b, bob_cert) = invite(store, &a1, "Bob", &links, &names, 1).await;
    let (carol, c, carol_cert) = invite(store, &a1, "Carol", &links, &["carol-phone"], 2).await;
    // b3 seul par défaut : jamais connecté, il n'est pas à jour ; les envois
    // retombent sur les autres appareils joignables et à jour (D42).
    for (dev, default) in [(&b[2], true), (&b[0], false)] {
        let changes = DeviceChanges { default: Some(default), ..DeviceChanges::default() };
        store.update_device(&b[0].id, &dev.id, &changes).await.unwrap();
    }

    // L'entrée 2 d'Alice engage les deux certificats (triés par personne).
    let mut certs = vec![
        CertRef { user: bob.clone(), cert: cert_hash(&bob_cert) },
        CertRef { user: carol.clone(), cert: cert_hash(&carol_cert) },
    ];
    certs.sort_by(|x, y| x.user.cmp(&y.user));
    let mut roster = a1.keyring.load_roster().unwrap().unwrap();
    roster.version += 1;
    let entry2 = RosterEntry::new(alice.clone(), 2, entry_hash(&entry1), a1.id.clone(), roster)
        .with_certs(certs)
        .sign(&key(&a1))
        .unwrap();
    store.publish_entry(&a1.id, &alice, 2, &entry2, u64::MAX).await.unwrap();
    for (user, devs) in [(&bob, &b), (&carol, &c)] {
        let first = &devs[0];
        let roster = first.keyring.load_roster().unwrap().unwrap();
        let entry = RosterEntry::new(user.clone(), 1, GENESIS_PREV, first.id.clone(), roster)
            .sign(&key(first))
            .unwrap();
        store.publish_entry(&first.id, user, 1, &entry, u64::MAX).await.unwrap();
    }

    // Tous connaissent l'ancre (un invité la tient de son code).
    let a = vec![a1, a2];
    for d in a.iter().chain(&b).chain(&c) {
        d.keyring.save_directory(&DirectoryState::with_anchor(anchor(&root))).unwrap();
    }
    Net { server, _dir: dir, alice, bob, carol, a, b, c }
}

async fn add_device_for(store: &Store, user: &UserId, name: &str) -> TestDevice {
    let identity = ot_core::Identity::generate();
    let d = store.add_device_for(user, name, &identity.public_key(), 2).await.unwrap();
    TestDevice {
        id: d.id,
        name: name.into(),
        identity,
        keyring: Arc::new(ot_core::MemoryKeyring::new()),
    }
}

fn config(dev: &TestDevice, url: &str) -> ClientConfig {
    let mut c = dev.config(url);
    c.transfer.chunk_size = 64 * 1024;
    c
}

async fn online(dev: &TestDevice, url: &str) -> (Client, Events) {
    let (client, mut events) = Client::start(config(dev, url));
    wait_connected(&mut events).await;
    (client, events)
}

/// Attend que l'annuaire vérifié de `client` réponde à `pred`.
async fn people_until(
    client: &Client,
    events: &mut Events,
    pred: impl Fn(&[Person]) -> bool,
) -> Vec<Person> {
    within(async {
        loop {
            let people = client.directory().await.unwrap();
            if pred(&people) {
                return people;
            }
            wait_for(events, |e| matches!(e, Event::DirectoryChanged)).await;
        }
    })
    .await
}

/// Les appareils `ids` de `user` annoncent `users-v1`.
fn up_to_date(people: &[Person], user: &UserId, n: usize) -> bool {
    people
        .iter()
        .find(|p| &p.id == user)
        .is_some_and(|p| p.devices.iter().filter(|d| d.up_to_date).count() == n)
}

#[tokio::test]
async fn text_and_file_between_two_people_both_ways() {
    let net = net().await;
    let url = net.server.url();
    let (alice, bob) = (net.alice.clone(), net.bob.clone());
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let (_ca2, mut ea2) = online(&net.a[1], &url).await;
    let (cb1, mut eb1) = online(&net.b[0], &url).await;
    let (cb2, mut eb2) = online(&net.b[1], &url).await;
    // b3 ne se connecte jamais : il n'annonce pas `users-v1`.

    // Chaque client déclare `users-v1` de lui-même ; les autres le voient.
    people_until(&ca1, &mut ea1, |p| up_to_date(p, &bob, 2) && up_to_date(p, &alice, 2)).await;
    people_until(&cb2, &mut eb2, |p| up_to_date(p, &alice, 2) && up_to_date(p, &bob, 2)).await;

    // Alice → Bob : ses appareils à jour (b3 sauté), copie à a2.
    let target = Target::Person { user: bob.clone(), device: None };
    let env = within(ca1.seal_to(target.clone(), &Content::text("bonjour Bob"))).await.unwrap();
    let mut to = env.to.clone();
    to.sort();
    let mut expected = vec![net.b[0].id.clone(), net.b[1].id.clone(), net.a[1].id.clone()];
    expected.sort();
    assert_eq!(to, expected, "b3, par défaut mais pas à jour, est sauté");
    within(ca1.send_envelope(env)).await.unwrap();
    for events in [&mut eb1, &mut eb2] {
        let m = next_message(events).await;
        assert_eq!(m.content.body, "bonjour Bob");
        assert_eq!(m.origin.from_user, Some(alice.clone()));
    }
    let copy = next_message(&mut ea2).await;
    assert_eq!(
        (copy.content.body.as_str(), &copy.origin),
        ("bonjour Bob", &Origin { to_user: Some(bob.clone()), ..Origin::default() })
    );

    // Bob répond à un appareil précis d'Alice.
    let target = Target::Person { user: alice.clone(), device: Some(net.a[1].id.clone()) };
    within(cb2.send_to(target, &Content::text("merci"))).await.unwrap();
    let m = next_message(&mut ea2).await;
    assert_eq!((m.content.body.as_str(), m.origin.from_user.as_ref()), ("merci", Some(&bob)));
    assert_eq!(next_message(&mut eb1).await.origin.to_user, Some(alice.clone()), "copie");

    // b3 visé directement : refusé avant tout envoi.
    let target = Target::Person { user: bob.clone(), device: Some(net.b[2].id.clone()) };
    let err = within(ca1.send_to(target, &Content::text("x"))).await.unwrap_err();
    assert!(matches!(err, ClientError::NoRecipient(_)), "{err}");

    // Carol reçoit d'Alice ; Bob, hors de son réseau, ne la voit pas.
    let (cc1, mut ec1) = online(&net.c[0], &url).await;
    people_until(&ca1, &mut ea1, |p| up_to_date(p, &net.carol, 1)).await;
    let target = Target::Person { user: net.carol.clone(), device: None };
    within(ca1.send_to(target, &Content::text("bonjour Carol"))).await.unwrap();
    let m = next_message(&mut ec1).await;
    assert_eq!(
        (m.content.body.as_str(), m.origin.from_user.as_ref()),
        ("bonjour Carol", Some(&alice))
    );
    let copy = next_message(&mut ea2).await;
    assert_eq!(copy.origin.to_user.as_ref(), Some(&net.carol));
    let seen_by_bob = within(cb1.directory()).await.unwrap();
    assert!(seen_by_bob.iter().all(|p| p.id != net.carol), "Carol hors du réseau de Bob");
    let target = Target::Person { user: net.carol.clone(), device: None };
    let err = within(cb1.send_to(target, &Content::text("x"))).await.unwrap_err();
    assert!(matches!(err, ClientError::NoRecipient(_)), "{err}");
    within(cc1.shutdown()).await;

    // Fichier, dans les deux sens.
    for (from, to_user, events) in [(&ca1, &bob, &mut eb1), (&cb1, &alice, &mut ea1)] {
        let data = sample(200_000);
        let file = OutgoingFile {
            source_id: "photo".into(),
            name: "photo.jpg".into(),
            mime: "image/jpeg".into(),
            to: vec![],
            to_user: Some(to_user.clone()),
        };
        let progress = Arc::new(|_: u64, _: u64| {});
        let (_, info) =
            within(from.send_file_info(file, MemSource::new(data.clone()), progress.clone()))
                .await
                .unwrap();
        assert_eq!(info.origin.to_user.as_ref(), Some(to_user));
        let got = match wait_for(events, |e| matches!(e, Event::File(_))).await {
            Event::File(f) => f.file,
            _ => unreachable!(),
        };
        assert!(got.origin.from_user.is_some());
        let receiver = if to_user == &bob { &cb1 } else { &ca1 };
        let sink = Arc::new(MemSink::default());
        within(receiver.download(&got.blob_id, sink.clone(), progress)).await.unwrap();
        assert_eq!(*sink.data.lock().unwrap(), data);
    }
    net.server.stop().await;
}

fn kind_of(people: &[Person], device: &ot_proto::DeviceId) -> Option<Option<DeviceKind>> {
    people.iter().flat_map(|p| &p.devices).find(|d| &d.id == device).map(|d| d.kind)
}

#[tokio::test]
async fn a_client_declares_its_configured_kind() {
    let net = net().await;
    let url = net.server.url();
    let (a0, a1) = (net.a[0].id.clone(), net.a[1].id.clone());
    let mut phone = config(&net.a[0], &url);
    phone.kind = Some(DeviceKind::Phone);
    let (_c0, mut e0) = Client::start(phone);
    wait_connected(&mut e0).await;
    let (c1, mut e1) = online(&net.a[1], &url).await;

    people_until(&c1, &mut e1, |p| kind_of(p, &a0) == Some(Some(DeviceKind::Phone))).await;
    let people = people_until(&c1, &mut e1, |p| up_to_date(p, &net.alice, 2)).await;
    assert_eq!(kind_of(&people, &a1), Some(None), "sans catégorie configurée, rien n'est déclaré");
}
