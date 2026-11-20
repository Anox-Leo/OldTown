//! Étape 10, § 6.1 et 6.2 : personnes de service de bout en bout, clients
//! `ot-core` et relais en processus. Noms neutres : Alice (admin, deux
//! appareils), Bob (invité), service « Notifier » (`notifier-1`,
//! `notifier-2`).

mod common;

use std::sync::Arc;

use common::*;
use ot_core::crypto::InviteCode;
use ot_core::{
    Client, Event, Events, Identity, Keyring, MemoryKeyring, Otp, Person, TlsRoots, redeem_invite,
    save_pending_invite,
};
use ot_proto::{DeviceId, ErrorCode, UserId};
use ot_server::store::Store;

struct Admin {
    server: TestServer,
    alice: UserId,
    a: Vec<TestDevice>,
}

/// Alice et ses deux appareils, racine publiée par `a1`, ancre adoptée
/// par `a2`.
async fn admin() -> Admin {
    let store = Store::in_memory().await.unwrap();
    let a1 = add_device(&store, "alice-phone").await;
    let a2 = add_device(&store, "alice-laptop").await;
    make_group(&[&a1, &a2]);
    let server = TestServer::start(store, test_config()).await;
    let mut alice = None;
    for d in [&a1, &a2] {
        let (c, mut e) = d.client(&server.url());
        wait_connected(&mut e).await;
        alice = Some(within(c.admin_init()).await.unwrap());
        within(c.shutdown()).await;
    }
    Admin { server, alice: alice.unwrap(), a: vec![a1, a2] }
}

async fn online(dev: &TestDevice, url: &str) -> (Client, Events) {
    let (client, mut events) = dev.client(url);
    wait_connected(&mut events).await;
    (client, events)
}

/// Bob, invité par `admin` et certifié ; son client tourne.
async fn bob(admin: &Client, admin_events: &mut Events, alice: &UserId, url: &str) -> Guest {
    let invitation = within(admin.invite("Bob", vec![alice.clone()])).await.unwrap();
    let identity = Identity::generate();
    let code = InviteCode::parse(&invitation.code).unwrap();
    let redeemed = within(redeem_invite(url, &TlsRoots::default(), &identity, "bob-phone", &code))
        .await
        .unwrap();
    let keyring = Arc::new(MemoryKeyring::new());
    save_pending_invite(&*keyring, &code, &redeemed, "bob-phone").unwrap();
    let dev =
        TestDevice { id: redeemed.device_id.clone(), name: "bob-phone".into(), identity, keyring };
    let (client, mut events) = online(&dev, url).await;
    wait_for(admin_events, |e| matches!(e, Event::PersonJoined { .. })).await;
    wait_for(&mut events, |e| matches!(e, Event::Joined { .. })).await;
    Guest { user: redeemed.user, dev, client, events }
}

struct Guest {
    user: UserId,
    dev: TestDevice,
    client: Client,
    events: Events,
}

/// Nouvel appareil de service : QR inversé affiché, client démarré (son
/// authentification échoue jusqu'à l'enregistrement par l'admin).
async fn service_device(url: &str, name: &str) -> (TestDevice, Client, Events, String) {
    let dev = TestDevice {
        id: DeviceId::generate(),
        name: name.into(),
        identity: Identity::generate(),
        keyring: Arc::new(MemoryKeyring::new()),
    };
    let (client, events) = dev.client(url);
    let qr = within(client.pair_offer(name)).await.unwrap();
    (dev, client, events, qr)
}

async fn joined(events: &mut Events) -> UserId {
    let ev =
        wait_for(events, |e| matches!(e, Event::Joined { .. } | Event::InviteFailed { .. })).await;
    match ev {
        Event::Joined { user } => user,
        other => panic!("bienvenue de service refusée : {other:?}"),
    }
}

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

fn person<'a>(people: &'a [Person], user: &UserId) -> Option<&'a Person> {
    people.iter().find(|p| &p.id == user)
}

/// `user` est vérifié et a `n` appareils dans son roster.
fn listed(people: &[Person], user: &UserId, n: usize) -> bool {
    person(people, user).is_some_and(|p| p.roster.members.len() == n)
}

fn code(digits: &str) -> Otp {
    Otp {
        code: digits.into(),
        display: digits.into(),
        sender_name: "Example".into(),
        sender_domain: "example.org".into(),
        sender_verified: true,
        expires_at: time::OffsetDateTime::now_utc().unix_timestamp() + 600,
    }
}

async fn next_otp(events: &mut Events) -> ot_core::ReceivedOtp {
    match wait_for(events, |e| matches!(e, Event::Otp(_))).await {
        Event::Otp(o) => o,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn service_devices_are_added_and_send_codes_to_people() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let mut bob = bob(&ca1, &mut ea1, &alice, &url).await;

    // Premier appareil : le service est créé.
    let (n1, cn1, mut en1, qr) = service_device(&url, "notifier-1").await;
    let added = within(ca1.add_service_device("Notifier", &qr)).await.unwrap();
    assert!(added.created);
    assert_eq!((&added.device_id, added.device_name.as_str()), (&n1.id, "notifier-1"));
    let notifier = added.user.clone();
    assert_eq!(joined(&mut en1).await, notifier);
    assert!(within(cn1.group()).await.unwrap().is_none(), "un service n'a pas de groupe");
    // Relancé avec le même QR : rien de neuf, pas d'erreur.
    let again = within(ca1.add_service_device("Notifier", &qr)).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.user, notifier);

    // Liens à sens unique vers Alice et Bob ; le service les voit, et eux
    // le voient avec son appareil.
    within(ca1.link_set(&notifier, &alice, true)).await.unwrap();
    within(ca1.link_set(&notifier, &bob.user, true)).await.unwrap();
    people_until(&cn1, &mut en1, |p| {
        person(p, &alice).is_some() && person(p, &bob.user).is_some_and(|b| !b.devices.is_empty())
    })
    .await;
    let seen = people_until(&bob.client, &mut bob.events, |p| listed(p, &notifier, 1)).await;
    assert_eq!(person(&seen, &notifier).unwrap().name, "Notifier");

    // Un code pour les deux personnes : un message, origine « service »,
    // remplissage d'office permis (O2).
    within(cn1.send_otp_to(&[alice.clone(), bob.user.clone()], &code("482913"))).await.unwrap();
    let got = next_otp(&mut bob.events).await;
    assert_eq!(got.otp.code, "482913");
    assert_eq!(got.origin.from_user.as_ref(), Some(&notifier));
    assert!(got.origin.from_service && got.auto_fill());
    assert_eq!(next_otp(&mut ea1).await.otp.code, "482913");

    // Second appareil du même service.
    let (n2, cn2, mut en2, qr2) = service_device(&url, "notifier-2").await;
    let added2 = within(ca1.add_service_device("Notifier", &qr2)).await.unwrap();
    assert!(!added2.created);
    assert_eq!(joined(&mut en2).await, notifier);
    people_until(&bob.client, &mut bob.events, |p| listed(p, &notifier, 2)).await;
    people_until(&cn2, &mut en2, |p| person(p, &bob.user).is_some()).await;
    within(cn2.send_otp_to(std::slice::from_ref(&bob.user), &code("135790"))).await.unwrap();
    let got = next_otp(&mut bob.events).await;
    assert_eq!((got.otp.code.as_str(), &got.from), ("135790", &n2.id));

    // Bob coupe le service : le service le voit, et le relais refuse ses
    // envois vers Bob.
    within(bob.client.mute_service(&notifier, true)).await.unwrap();
    people_until(&cn1, &mut en1, |p| person(p, &bob.user).is_some_and(|b| b.muted)).await;
    let refused = within(cn1.send_otp_to(std::slice::from_ref(&bob.user), &code("246802"))).await;
    assert!(
        matches!(refused, Err(ot_core::ClientError::Rejected { code: ErrorCode::NotAllowed, .. })),
        "{refused:?}"
    );
    drop((n1, bob.dev));
    net.server.stop().await;
}

#[tokio::test]
async fn a_service_is_republished_after_its_admin_device_is_removed() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let (ca2, mut ea2) = online(&net.a[1], &url).await;
    // L'admin doit avoir lu son annuaire avant d'agir.
    people_until(&ca2, &mut ea2, |p| person(p, &alice).is_some()).await;

    // Le service est créé par a2 : certificat et journal signés par a2.
    let (_n1, cn1, mut en1, qr) = service_device(&url, "notifier-1").await;
    let notifier = within(ca2.add_service_device("Notifier", &qr)).await.unwrap().user;
    joined(&mut en1).await;
    within(ca2.link_set(&notifier, &alice, true)).await.unwrap();
    people_until(&ca1, &mut ea1, |p| listed(p, &notifier, 1)).await;
    within(ca2.shutdown()).await;

    // a1 retire a2 : il re-signe le certificat du service et republie son
    // journal (D47) ; le service reste visible et joignable.
    within(ca1.remove_device(&net.a[1].id)).await.unwrap();
    let a1 = net.a[0].id.clone();
    within(async {
        loop {
            let state = net.a[0].keyring.load_directory().unwrap();
            let known = state.people.get(&notifier).and_then(|k| k.chain.clone());
            let cert = state
                .people
                .get(&notifier)
                .map(|k| ot_core::crypto::directory::UserCert::read_unverified(&k.cert).unwrap());
            if known.is_some_and(|c| c.signer == a1) && cert.is_some_and(|c| c.issuer == a1) {
                break;
            }
            wait_for(&mut ea1, |e| matches!(e, Event::DirectoryChanged)).await;
        }
    })
    .await;
    people_until(&ca1, &mut ea1, |p| listed(p, &notifier, 1)).await;
    people_until(&cn1, &mut en1, |p| {
        person(p, &alice).is_some_and(|a| a.roster.member(&net.a[1].id).is_none())
    })
    .await;
    within(cn1.send_otp_to(std::slice::from_ref(&alice), &code("975310"))).await.unwrap();
    assert_eq!(next_otp(&mut ea1).await.otp.code, "975310");
    net.server.stop().await;
}

#[tokio::test]
async fn only_the_admin_adds_service_devices() {
    let net = admin().await;
    let url = net.server.url();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let mut bob = bob(&ca1, &mut ea1, &net.alice, &url).await;
    people_until(&bob.client, &mut bob.events, |p| person(p, &bob.user).is_some()).await;
    let (_n1, _cn1, _en1, qr) = service_device(&url, "notifier-1").await;
    let refused = within(bob.client.add_service_device("Notifier", &qr)).await;
    assert!(matches!(refused, Err(ot_core::ClientError::Invalid(_))), "{refused:?}");
    // Un QR altéré est refusé avant tout envoi au relais.
    let mut tampered = ot_core::PairingOffer::parse(&qr).unwrap();
    tampered.name = "notifier-x".into();
    let refused = within(ca1.add_service_device("Notifier", &tampered.encode())).await;
    assert!(refused.is_err(), "{refused:?}");
    net.server.stop().await;
}
