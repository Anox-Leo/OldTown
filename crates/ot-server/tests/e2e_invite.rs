//! Étape 10, T8 et § 4.2 à 4.6 : invitation, publication du journal et
//! administration de bout en bout, clients `ot-core` et relais en processus.
//! Noms neutres : Alice (admin, deux appareils), Bob et Carol (invités).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use ot_core::crypto::InviteCode;
use ot_core::{
    Client, Content, Event, Events, InviteRedeemed, Keyring, MemoryKeyring, Person, Target,
    TlsRoots, redeem_invite, redeem_pairing, save_pending_invite,
};
use ot_proto::UserId;
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
    let alice = {
        let (c1, mut e1) = a1.client(&server.url());
        wait_connected(&mut e1).await;
        let alice = within(c1.admin_init()).await.unwrap();
        within(c1.shutdown()).await;
        let (c2, mut e2) = a2.client(&server.url());
        wait_connected(&mut e2).await;
        within(c2.admin_init()).await.unwrap();
        within(c2.shutdown()).await;
        alice
    };
    Admin { server, alice, a: vec![a1, a2] }
}

async fn online(dev: &TestDevice, url: &str) -> (Client, Events) {
    let (client, mut events) = dev.client(url);
    wait_connected(&mut events).await;
    (client, events)
}

/// Premier appareil d'une personne invitée : jeton racheté, invitation
/// rangée ; son client n'est pas encore démarré.
async fn redeem(url: &str, code: &str, name: &str) -> (TestDevice, InviteRedeemed) {
    let identity = ot_core::Identity::generate();
    let parsed = InviteCode::parse(code).unwrap();
    let redeemed =
        within(redeem_invite(url, &TlsRoots::default(), &identity, name, &parsed)).await.unwrap();
    let keyring = Arc::new(MemoryKeyring::new());
    save_pending_invite(&*keyring, &parsed, &redeemed, name).unwrap();
    let dev = TestDevice { id: redeemed.device_id.clone(), name: name.into(), identity, keyring };
    (dev, redeemed)
}

/// Attend un état de l'annuaire vérifié de `client`.
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

/// `user` est vérifié avec `n` appareils à jour.
fn ready(people: &[Person], user: &UserId, n: usize) -> bool {
    person(people, user).is_some_and(|p| p.devices.iter().filter(|d| d.up_to_date).count() == n)
}

#[tokio::test]
async fn invited_person_is_certified_and_talks_both_ways() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let (_ca2, mut ea2) = online(&net.a[1], &url).await;

    let invitation = within(ca1.invite("Bob", vec![alice.clone()])).await.unwrap();
    assert!(invitation.code.starts_with("ot2-"));
    assert!(invitation.uri.starts_with("oldtown://invite?s="));
    let uri = ot_core::InviteUri::parse(&invitation.uri).unwrap();
    assert_eq!(uri.code, invitation.code);

    let (b1, redeemed) = redeem(&url, &invitation.code, "bob-phone").await;
    assert_eq!((&redeemed.user, &redeemed.inviter), (&invitation.user, &net.a[0].id));
    let bob = redeemed.user.clone();
    let (cb1, mut eb1) = online(&b1, &url).await;

    // L'admin certifie d'office ; l'invité crée son groupe.
    let joined = wait_for(&mut ea1, |e| matches!(e, Event::PersonJoined { .. })).await;
    let Event::PersonJoined { user, name, device_name, fingerprint } = joined else {
        unreachable!()
    };
    assert_eq!((&user, name.as_str(), device_name.as_str()), (&bob, "Bob", "bob-phone"));
    assert_eq!(fingerprint, ot_core::crypto::fingerprint(&b1.identity.public_key()));
    let ev = wait_for(&mut eb1, |e| matches!(e, Event::Joined { .. })).await;
    assert!(matches!(ev, Event::Joined { user } if user == bob));
    assert!(within(cb1.group()).await.unwrap().is_some(), "groupe créé");
    assert!(b1.keyring.load_pending_invite().unwrap().is_none(), "invitation terminée");
    assert!(net.a[0].keyring.load_invites().unwrap().is_empty());

    // Bob publie son journal (entrée 1) : les deux appareils d'Alice le
    // voient, et lui les voit.
    people_until(&ca1, &mut ea1, |p| ready(p, &bob, 1)).await;
    people_until(&cb1, &mut eb1, |p| ready(p, &alice, 2) && ready(p, &bob, 1)).await;

    let to_bob = Target::Person { user: bob.clone(), device: None };
    within(ca1.send_to(to_bob, &Content::text("bienvenue"))).await.unwrap();
    let m = next_message(&mut eb1).await;
    assert_eq!((m.content.body.as_str(), m.origin.from_user.as_ref()), ("bienvenue", Some(&alice)));
    let to_alice = Target::Person { user: alice.clone(), device: Some(net.a[1].id.clone()) };
    within(cb1.send_to(to_alice, &Content::text("merci"))).await.unwrap();
    // a2 reçoit d'abord la copie de l'envoi de a1 (D43), puis la réponse.
    let copy = next_message(&mut ea2).await;
    assert_eq!(
        (copy.content.body.as_str(), copy.origin.to_user.as_ref()),
        ("bienvenue", Some(&bob))
    );
    assert_eq!(next_message(&mut ea2).await.content.body, "merci");

    // Le roster de l'admin a pris la version de son journal (note 0057,
    // point 7) sur ses deux appareils : pas de fourche.
    let v1 = net.a[0].keyring.load_roster().unwrap().unwrap().version;
    let state = net.a[0].keyring.load_directory().unwrap();
    let chain = state.admin.unwrap().1.chain.unwrap();
    assert_eq!(v1, chain.roster.version);
    within(async {
        while net.a[1].keyring.load_roster().unwrap().unwrap().version != v1 {
            wait_for(&mut ea2, |e| matches!(e, Event::GroupChanged { .. })).await;
        }
    })
    .await;
    no_event(&mut ea2, |e| matches!(e, Event::DirectoryFork), Duration::from_millis(300)).await;
    net.server.stop().await;
}

/// Aucun événement `pred` pendant `d`.
async fn no_event(events: &mut Events, pred: impl Fn(&Event) -> bool, d: Duration) {
    let deadline = tokio::time::Instant::now() + d;
    while let Ok(Some(e)) = tokio::time::timeout_at(deadline, events.next()).await {
        assert!(!pred(&e), "événement inattendu : {e:?}");
    }
}

#[tokio::test]
async fn request_waits_for_the_inviting_device_and_needs_the_secret() {
    let net = admin().await;
    let url = net.server.url();
    let (ca1, _ea1) = online(&net.a[0], &url).await;
    let invitation = within(ca1.invite("Bob", vec![net.alice.clone()])).await.unwrap();
    within(ca1.shutdown()).await;

    // Un faux code : bon jeton, mauvais `S`. La demande ne s'ouvre pas chez
    // l'admin ; rien n'est certifié.
    let mut forged = InviteCode::parse(&invitation.code).unwrap();
    forged.secret = ot_core::crypto::PairingSecret::generate();
    let (b1, _) = redeem(&url, &forged.encode(), "bob-phone").await;
    let (_cb1, mut eb1) = online(&b1, &url).await;
    // L'autre appareil de l'admin n'a pas `S` : la demande ne lui est pas
    // adressée, il ne certifie rien.
    let (_ca2, mut ea2) = online(&net.a[1], &url).await;
    no_event(&mut ea2, |e| matches!(e, Event::PersonJoined { .. }), Duration::from_millis(300))
        .await;

    // L'appareil qui a invité revient : il refuse la demande forgée.
    let (_ca1, mut ea1) = online(&net.a[0], &url).await;
    let ev = wait_for(&mut ea1, |e| matches!(e, Event::Rejected { .. })).await;
    assert!(matches!(ev, Event::Rejected { reason, .. } if reason == "déchiffrement impossible"));
    no_event(&mut eb1, |e| matches!(e, Event::Joined { .. }), Duration::from_millis(300)).await;
    assert_eq!(net.a[0].keyring.load_invites().unwrap().len(), 1, "toujours en attente");
    net.server.stop().await;
}

#[tokio::test]
async fn invitation_received_while_admin_offline_is_handled_on_return() {
    let net = admin().await;
    let url = net.server.url();
    let (ca1, _) = online(&net.a[0], &url).await;
    let invitation = within(ca1.invite("Bob", vec![net.alice.clone()])).await.unwrap();
    within(ca1.shutdown()).await;

    let (b1, redeemed) = redeem(&url, &invitation.code, "bob-phone").await;
    let (cb1, _) = online(&b1, &url).await;
    // Bob redémarre avant la bienvenue : sa demande repart.
    within(cb1.shutdown()).await;
    let (cb1, mut eb1) = online(&b1, &url).await;

    let (_ca1, mut ea1) = online(&net.a[0], &url).await;
    wait_for(&mut ea1, |e| matches!(e, Event::PersonJoined { .. })).await;
    wait_for(&mut eb1, |e| matches!(e, Event::Joined { .. })).await;
    people_until(&cb1, &mut eb1, |p| ready(p, &redeemed.user, 1)).await;
    net.server.stop().await;
}

#[tokio::test]
async fn a_new_device_of_a_person_is_published_and_seen_by_others() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let invitation = within(ca1.invite("Bob", vec![alice.clone()])).await.unwrap();
    let (b1, redeemed) = redeem(&url, &invitation.code, "bob-phone").await;
    let bob = redeemed.user.clone();
    let (cb1, mut eb1) = online(&b1, &url).await;
    wait_for(&mut eb1, |e| matches!(e, Event::Joined { .. })).await;
    people_until(&ca1, &mut ea1, |p| ready(p, &bob, 1)).await;

    // Bob appaire un second appareil : le parrain publie le roster (§ 4.2).
    let pairing = within(cb1.pair_create()).await.unwrap();
    let identity = ot_core::Identity::generate();
    let redeemed =
        within(redeem_pairing(&url, &TlsRoots::default(), &identity, "bob-laptop", &pairing.code))
            .await
            .unwrap();
    let b2 = TestDevice {
        id: redeemed.device_id.clone(),
        name: "bob-laptop".into(),
        identity,
        keyring: Arc::new(MemoryKeyring::new()),
    };
    let anchor = b1.keyring.load_directory().unwrap().anchor.unwrap();
    b2.keyring.save_directory(&ot_core::directory::DirectoryState::with_anchor(anchor)).unwrap();
    let (cb2, mut eb2) = online(&b2, &url).await;
    within(cb2.pair_join(&pairing.code, redeemed.sponsor, "bob-laptop")).await.unwrap();
    wait_for(&mut eb2, |e| matches!(e, Event::GroupChanged { .. })).await;

    let people = people_until(&ca1, &mut ea1, |p| ready(p, &bob, 2)).await;
    assert_eq!(person(&people, &bob).unwrap().roster.members.len(), 2);
    // Alice écrit aux deux appareils de Bob (aucun par défaut sauf le
    // premier : seul lui) ; b2 visé directement reçoit aussi.
    let target = Target::Person { user: bob.clone(), device: Some(b2.id.clone()) };
    within(ca1.send_to(target, &Content::text("au laptop"))).await.unwrap();
    assert_eq!(next_message(&mut eb2).await.content.body, "au laptop");
    net.server.stop().await;
}

#[tokio::test]
async fn admin_renames_revokes_and_reinvites() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let invite = |name: &'static str| {
        let c = ca1.clone();
        let links = vec![alice.clone()];
        async move { within(c.invite(name, links)).await.unwrap() }
    };
    let bob_inv = invite("Bob").await;
    let (b1, rb) = redeem(&url, &bob_inv.code, "bob-phone").await;
    let (_cb1, mut eb1) = online(&b1, &url).await;
    wait_for(&mut eb1, |e| matches!(e, Event::Joined { .. })).await;
    let carol_inv = invite("Carol").await;
    let (c1, rc) = redeem(&url, &carol_inv.code, "carol-phone").await;
    let (cc1, mut ec1) = online(&c1, &url).await;
    wait_for(&mut ec1, |e| matches!(e, Event::Joined { .. })).await;
    let (bob, carol) = (rb.user.clone(), rc.user.clone());
    people_until(&ca1, &mut ea1, |p| ready(p, &bob, 1) && ready(p, &carol, 1)).await;

    // Liens : Bob ↔ Carol posés par l'admin ; Carol voit Bob.
    within(ca1.link_set(&bob, &carol, true)).await.unwrap();
    people_until(&cc1, &mut ec1, |p| ready(p, &bob, 1)).await;
    let users = within(ca1.users()).await.unwrap();
    assert!(users.iter().any(|u| u.id == bob && u.links.contains(&carol)));

    // Renommer : le nom vu par les autres est celui du certificat re-signé.
    within(ca1.rename_user(&bob, "Robert")).await.unwrap();
    people_until(&cc1, &mut ec1, |p| person(p, &bob).is_some_and(|b| b.name == "Robert")).await;

    // Réinviter Bob (appareil perdu) : nouvelles clés, signalées.
    let again = within(ca1.reinvite(&bob)).await.unwrap();
    let (b1bis, _) = redeem(&url, &again.code, "bob-phone-2").await;
    let (_cb1bis, mut eb1bis) = online(&b1bis, &url).await;
    wait_for(&mut eb1bis, |e| matches!(e, Event::Joined { .. })).await;
    let ev = wait_for(&mut ec1, |e| matches!(e, Event::KeysChanged { .. })).await;
    assert!(matches!(ev, Event::KeysChanged { user } if user == bob));
    let people = people_until(&cc1, &mut ec1, |p| ready(p, &bob, 1)).await;
    assert_eq!(person(&people, &bob).unwrap().devices[0].id, b1bis.id);

    // Révoquer Bob : il disparaît de l'annuaire de Carol et de l'admin.
    within(ca1.revoke_user(&bob)).await.unwrap();
    people_until(&cc1, &mut ec1, |p| person(p, &bob).is_none()).await;
    people_until(&ca1, &mut ea1, |p| person(p, &bob).is_none()).await;
    let state = net.a[0].keyring.load_directory().unwrap();
    let certs = state.admin.unwrap().1.chain.unwrap().certs;
    assert!(certs.iter().all(|c| c.user != bob), "empreinte retirée du journal");
    net.server.stop().await;
}

#[tokio::test]
async fn removing_an_admin_device_recertifies_what_it_signed() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    // Carol est invitée par a2 : son certificat est signé par a2.
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let (ca2, _ea2) = online(&net.a[1], &url).await;
    let inv = within(ca2.invite("Carol", vec![alice.clone()])).await.unwrap();
    let (c1, rc) = redeem(&url, &inv.code, "carol-phone").await;
    let carol = rc.user.clone();
    let (cc1, mut ec1) = online(&c1, &url).await;
    wait_for(&mut ec1, |e| matches!(e, Event::Joined { .. })).await;
    people_until(&ca1, &mut ea1, |p| ready(p, &carol, 1)).await;
    within(ca2.shutdown()).await;

    // a1 retire a2 : rotation, roster publié, révocation, puis certificat
    // de Carol re-signé par a1. Carol reste visible de tous.
    within(ca1.remove_device(&net.a[1].id)).await.unwrap();
    let state = net.a[0].keyring.load_directory().unwrap();
    let chain = state.admin.as_ref().unwrap().1.chain.as_ref().unwrap();
    assert!(chain.roster.member(&net.a[1].id).is_none(), "a2 hors du roster publié");
    // Le certificat re-signé arrive à la relecture qui suit.
    let issuer = |dev: &TestDevice| {
        let state = dev.keyring.load_directory().unwrap();
        let cert = &state.people[&carol].cert;
        ot_core::crypto::directory::UserCert::read_unverified(cert).unwrap().issuer
    };
    within(async {
        while issuer(&net.a[0]) != net.a[0].id {
            wait_for(&mut ea1, |e| matches!(e, Event::DirectoryChanged)).await;
        }
    })
    .await;
    // Carol peut lire le roster sans a2 avant son certificat re-signé : entre
    // les deux, celui signé par a2 ne vaut plus. Elle attend les deux.
    people_until(&cc1, &mut ec1, |p| {
        person(p, &alice).is_some_and(|a| a.roster.member(&net.a[1].id).is_none())
            && ready(p, &carol, 1)
    })
    .await;
    let ca1_people = within(ca1.directory()).await.unwrap();
    assert!(ready(&ca1_people, &carol, 1));
    net.server.stop().await;
}

#[tokio::test]
async fn invitee_refuses_an_admin_log_cut_short_by_the_relay() {
    let net = admin().await;
    let url = net.server.url();
    let alice = net.alice.clone();
    let (ca1, mut ea1) = online(&net.a[0], &url).await;
    let invitation = within(ca1.invite("Bob", vec![alice.clone()])).await.unwrap();

    // Le relais ne sert à l'invité que la première entrée de l'admin : pas
    // celle qui engage son certificat (la tête de la bienvenue).
    let tamper: TamperFn = Arc::new(move |dir, frame| match (dir, frame) {
        (Dir::ToClient, ot_proto::Frame::Directory { me, admin, mut users, next }) => {
            for u in users.iter_mut().filter(|u| u.id == admin) {
                u.log.retain(|e| e.seq < 2);
                u.more = false;
            }
            ot_proto::Frame::Directory { me, admin, users, next }
        }
        (_, frame) => frame,
    });
    let hostile = proxy(&net.server, tamper).await;
    let (b1, _) = redeem(&url, &invitation.code, "bob-phone").await;
    let (cb1, mut eb1) = online(&b1, &hostile).await;
    wait_for(&mut ea1, |e| matches!(e, Event::PersonJoined { .. })).await;
    // Avant la bienvenue, son propre certificat est déjà refusé (empreinte
    // absente) ; après, c'est le journal de l'admin, plus court que la tête.
    let a = alice.clone();
    let ev =
        wait_for(&mut eb1, |e| matches!(e, Event::DirectoryRejected { user, .. } if *user == a))
            .await;
    assert!(matches!(ev, Event::DirectoryRejected { reason, .. } if reason.contains("plus court")));
    no_event(&mut eb1, |e| matches!(e, Event::Joined { .. }), Duration::from_millis(300)).await;
    assert!(within(cb1.group()).await.unwrap().is_none(), "aucun groupe sur une fourche");
    assert!(b1.keyring.load_pending_invite().unwrap().is_some(), "invitation gardée");
    net.server.stop().await;
}
