mod common;

use std::sync::Arc;

use common::*;
use ed25519_dalek::SigningKey;
use ot_core::crypto::directory::{CertRef, RosterEntry, entry_hash};
use ot_core::{Client, Event, Events, Person};
use ot_proto::{DirectoryDevice, Feature, Frame, Role, UserId};
use ot_server::store::Store;

async fn server() -> (TestServer, Vec<TestDevice>) {
    let store = Store::in_memory().await.unwrap();
    let a1 = add_device(&store, "alice-phone").await;
    let a2 = add_device(&store, "alice-laptop").await;
    let x1 = add_device(&store, "other").await;
    make_group(&[&a1, &a2]);
    make_group(&[&x1]);
    (TestServer::start(store, test_config()).await, vec![a1, a2, x1])
}

async fn online(dev: &TestDevice, url: &str) -> (Client, Events) {
    let (client, mut events) = dev.client(url);
    wait_connected(&mut events).await;
    (client, events)
}

async fn read_people(client: &Client, events: &mut Events) -> Vec<Person> {
    loop {
        let people = within(client.directory()).await.unwrap();
        if !people.is_empty() {
            return people;
        }
        wait_for(events, |e| matches!(e, Event::DirectoryChanged)).await;
    }
}

fn admin_seq(dev: &TestDevice) -> Option<u64> {
    use ot_core::Keyring;
    let state = dev.keyring.load_directory().unwrap();
    state.admin.and_then(|(_, p)| p.chain).map(|c| c.seq)
}

#[tokio::test]
async fn admin_init_sets_the_anchor_and_other_admin_devices_adopt_it() {
    let (server, devs) = server().await;
    let (a1, a2, x1) = (&devs[0], &devs[1], &devs[2]);
    let (c1, mut e1) = online(a1, &server.url()).await;
    assert!(within(c1.directory()).await.unwrap().is_empty(), "sans ancre : rien");

    let admin = within(c1.admin_init()).await.unwrap();
    let people = read_people(&c1, &mut e1).await;
    assert_eq!(people.len(), 1);
    let me = &people[0];
    assert_eq!((&me.id, me.role), (&admin, Role::Admin));
    let mut devices: Vec<_> = me.devices.iter().map(|d| d.id.clone()).collect();
    devices.sort();
    let mut expected = vec![a1.id.clone(), a2.id.clone()];
    expected.sort();
    assert_eq!(devices, expected, "les deux appareils du roster vérifié");
    assert_eq!(admin_seq(a1), Some(1));
    assert!(within(c1.admin_init()).await.is_err(), "une seule fois");

    let (c2, mut e2) = online(a2, &server.url()).await;
    assert_eq!(within(c2.admin_init()).await.unwrap(), admin);
    assert_eq!(read_people(&c2, &mut e2).await[0].id, admin);

    let (cx, _ex) = online(x1, &server.url()).await;
    assert!(within(cx.admin_init()).await.is_err());
    assert!(within(cx.directory()).await.unwrap().is_empty());
    server.stop().await;
}

fn admin_entry(
    admin: &UserId,
    by: &TestDevice,
    seq: u64,
    prev: [u8; 32],
    padding: usize,
) -> ot_proto::SignedBlob {
    use ot_core::Keyring;
    let mut roster = by.keyring.load_roster().unwrap().unwrap();
    roster.version = seq;
    let certs = (0..padding).map(|_| CertRef { user: UserId::generate(), cert: [7; 32] }).collect();
    RosterEntry::new(admin.clone(), seq, prev, by.id.clone(), roster)
        .with_certs(certs)
        .sign(&SigningKey::from_bytes(&by.identity.to_bytes()))
        .unwrap()
}

#[tokio::test]
async fn a_long_admin_log_is_read_in_several_rounds_and_followed() {
    let (server, devs) = server().await;
    let (a1, a2) = (&devs[0], &devs[1]);
    let (c1, mut e1) = online(a1, &server.url()).await;
    let admin = within(c1.admin_init()).await.unwrap();
    read_people(&c1, &mut e1).await;

    let store = server.relay.store();
    let first = store.directory(&admin, &Default::default(), None).await.unwrap();
    let mut prev = entry_hash(&first.users[0].log[0].entry);
    let last = 40;
    for seq in 2..=last {
        let entry = admin_entry(&admin, a1, seq, prev, 40);
        prev = entry_hash(&entry);
        store.publish_entry(&a1.id, &admin, seq, &entry, u64::MAX).await.unwrap();
    }
    let page = store.directory(&admin, &Default::default(), None).await.unwrap();
    assert!(page.users[0].more, "le test doit couper le journal");

    let (c2, mut e2) = online(a2, &server.url()).await;
    within(c2.admin_init()).await.unwrap();
    let people = read_people(&c2, &mut e2).await;
    assert_eq!(people[0].id, admin);
    assert_eq!(admin_seq(a2), Some(last), "lu jusqu'au bout, en plusieurs tours");

    let (mut raw, _) = Raw::login(&server, a1).await;
    let entry = admin_entry(&admin, a1, last + 1, prev, 0);
    raw.send(&Frame::RosterPublish { user: admin.clone(), seq: last + 1, entry }).await;
    let reply = loop {
        match raw.recv().await {
            Some(Frame::DirectoryChanged) => continue,
            other => break other,
        }
    };
    assert!(matches!(reply, Some(Frame::RosterPublished { .. })), "{reply:?}");
    within(async {
        while admin_seq(a2) != Some(last + 1) {
            wait_for(&mut e2, |e| matches!(e, Event::DirectoryChanged)).await;
        }
    })
    .await;
    raw.close().await;
    server.stop().await;
}

#[tokio::test]
async fn a_hostile_relay_gets_no_fake_device_accepted() {
    let (server, devs) = server().await;
    let (a1, a2) = (&devs[0], &devs[1]);
    let (c1, mut e1) = online(a1, &server.url()).await;
    let admin = within(c1.admin_init()).await.unwrap();
    read_people(&c1, &mut e1).await;
    within(c1.shutdown()).await;

    let intruder = add_device(server.relay.store(), "intrus").await;
    make_group(&[&intruder]);
    let first = server.relay.store().directory(&admin, &Default::default(), None).await.unwrap();
    let prev = entry_hash(&first.users[0].log[0].entry);
    let forged = admin_entry(&admin, &intruder, 2, prev, 0);
    let fake = intruder.id.clone();
    let tamper: TamperFn = Arc::new(move |dir, frame| match (dir, frame) {
        (Dir::ToClient, Frame::Directory { me, admin, mut users, next }) => {
            if let Some(u) = users.iter_mut().find(|u| u.id == admin) {
                u.devices.push(DirectoryDevice {
                    id: fake.clone(),
                    name: "intrus".into(),
                    kind: None,
                    reachable: true,
                    default: true,
                    features: vec![Feature::users_v1()],
                    online: true,
                });
                u.log.push(ot_proto::LogEntry { seq: 2, entry: forged.clone() });
            }
            Frame::Directory { me, admin, users, next }
        }
        (_, frame) => frame,
    });
    let url = proxy(&server, tamper).await;
    let (c2, mut e2) = online(a2, &url).await;
    within(c2.admin_init()).await.unwrap();
    let ev = wait_for(&mut e2, |e| matches!(e, Event::DirectoryRejected { .. })).await;
    assert!(matches!(ev, Event::DirectoryRejected { user, .. } if user == admin));
    let people = read_people(&c2, &mut e2).await;
    assert!(people[0].devices.iter().all(|d| d.id != intruder.id), "profil ignoré");
    assert!(people[0].roster.members.iter().all(|m| m.device_id != intruder.id));
    assert_eq!(admin_seq(a2), Some(1), "arrêté à la dernière entrée valide");
    server.stop().await;
}
