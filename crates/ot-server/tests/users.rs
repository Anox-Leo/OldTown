//! Étape 10 (note 0052) : personnes, réseau, invitation, quotas et annuaire
//! côté relais. Noms neutres : Alice (admin), Bob et Carol (membres),
//! Notifier (service).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use ed25519_dalek::{Signer, SigningKey};
use ot_core::{Identity, MemoryKeyring};
use ot_proto::http::{CreateBlob, HttpErrorCode};
use ot_proto::{
    BlobId, Content, DeviceId, DeviceKind, Envelope, ErrorCode, Feature, Frame, Known, Role,
    SignedBlob, UserId, UserState, invite_redeem_message, pair_register_message,
};
use ot_server::store::{NewBlob, NewMessage, Store, StoreError};
use ot_server::users::DeviceChanges;
use ot_server::{Config, UserLimits, token_hash};

const HOUR: i64 = 3600 * 1000;

fn blob(seed: u8) -> SignedBlob {
    SignedBlob { data: format!("{{\"n\":{seed}}}").into_bytes(), sig: [seed; 64] }
}

fn device(id: DeviceId, name: &str, identity: Identity) -> TestDevice {
    TestDevice { id, name: name.into(), identity, keyring: Arc::new(MemoryKeyring::new()) }
}

fn sign(identity: &Identity, msg: &[u8]) -> [u8; 64] {
    SigningKey::from_bytes(&identity.to_bytes()).sign(msg).to_bytes()
}

fn msg<'a>(id: &'a str, from: &'a DeviceId, to: &'a [DeviceId]) -> NewMessage<'a> {
    NewMessage { msg_id: id, sender: from, to, envelope: b"{}", expires_at: i64::MAX }
}

/// Alice (admin, deux appareils), Bob (membre, deux appareils), Carol
/// (membre), Notifier (service). Liens : Alice ↔ Bob, Alice ↔ Carol,
/// Notifier → Alice, Notifier → Bob. Pas de lien Bob ↔ Carol.
struct Net {
    store: Store,
    alice: UserId,
    alice_phone: TestDevice,
    alice_laptop: TestDevice,
    bob: UserId,
    bob_phone: TestDevice,
    bob_laptop: TestDevice,
    carol: UserId,
    carol_phone: TestDevice,
    notifier: UserId,
    notifier_1: TestDevice,
}

/// Invite, fait rejoindre et certifie un membre, lié à `links`.
async fn member(
    store: &Store,
    by: &TestDevice,
    name: &str,
    device_name: &str,
    links: &[UserId],
) -> (UserId, TestDevice) {
    let identity = Identity::generate();
    let token: [u8; 10] = identity.public_key()[..10].try_into().unwrap();
    let hash = token_hash(&token);
    let user = store.invite_user(&by.id, name, links, &hash, 5, 0, 48 * HOUR).await.unwrap();
    let r = store.redeem_invitation(&hash, device_name, &identity.public_key(), 1).await.unwrap();
    assert_eq!((&r.user, &r.inviter), (&user, &by.id));
    store.certify_user(&user, &blob(10)).await.unwrap();
    // Fin de l'échange d'invitation : le premier roster est publié.
    store.publish_entry(&r.device.id, &user, 1, &blob(11)).await.unwrap();
    (user, device(r.device.id, device_name, identity))
}

async fn extra_device(store: &Store, user: &UserId, name: &str) -> TestDevice {
    let identity = Identity::generate();
    let d = store.add_device_for(user, name, &identity.public_key(), 2).await.unwrap();
    device(d.id, name, identity)
}

async fn net() -> Net {
    let store = Store::in_memory().await.unwrap();
    let alice_phone = add_device(&store, "alice-phone").await;
    let alice_laptop = add_device(&store, "alice-laptop").await;
    let alice = store.admin_init(&blob(1), &blob(2)).await.unwrap();
    store.rename_user(&alice, "Alice").await.unwrap();

    let (bob, bob_phone) =
        member(&store, &alice_phone, "Bob", "bob-phone", std::slice::from_ref(&alice)).await;
    let bob_laptop = extra_device(&store, &bob, "bob-laptop").await;
    let (carol, carol_phone) =
        member(&store, &alice_phone, "Carol", "carol-phone", std::slice::from_ref(&alice)).await;

    let notifier = UserId::generate();
    let identity = Identity::generate();
    let id = DeviceId::generate();
    store
        .create_service(
            &notifier,
            "Notifier",
            &blob(3),
            &id,
            &identity.public_key(),
            "notifier-1",
            3,
        )
        .await
        .unwrap();
    let notifier_1 = device(id, "notifier-1", identity);
    store.set_link(&notifier, &alice, true, 4).await.unwrap();
    store.set_link(&notifier, &bob, true, 4).await.unwrap();

    Net {
        store,
        alice,
        alice_phone,
        alice_laptop,
        bob,
        bob_phone,
        bob_laptop,
        carol,
        carol_phone,
        notifier,
        notifier_1,
    }
}

/// Base au format v1.0.0 (migrations 0001 à 0003), remplie à la main.
async fn v1_database(path: &std::path::Path) {
    use sqlx::ConnectOptions;
    let old = tempfile::tempdir().unwrap();
    for name in ["0001_init.sql", "0002_pairings.sql", "0003_blobs.sql"] {
        let src = format!("{}/migrations/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::copy(src, old.path().join(name)).unwrap();
    }
    let migrator = sqlx::migrate::Migrator::new(old.path()).await.unwrap();
    let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    migrator.run(&mut conn).await.unwrap();
    for (i, name) in ["phone", "laptop", "old"].iter().enumerate() {
        sqlx::query("INSERT INTO devices (id, name, public_key, created_at) VALUES (?, ?, ?, ?)")
            .bind(format!("dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H{i}"))
            .bind(name)
            .bind(vec![i as u8 + 1; 32])
            .bind(i as i64)
            .execute(&mut conn)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE devices SET revoked_at = 9 WHERE name = 'old'")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO messages (msg_id, sender_id, created_at, expires_at, size, envelope)
         VALUES ('m1', 'dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H0', 1, 9999999999999, 2, X'7B7D')",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO deliveries (seq, device_id) VALUES (1, 'dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H1')",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blobs (id, owner_id, size, chunk_size, chunks, created_at, expires_at)
         VALUES ('blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V', 'dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H0', 16, 16, 1, 1,
                 9999999999999)",
    )
    .execute(&mut conn)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_from_v1_0_0_keeps_everything_under_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ot.db");
    v1_database(&path).await;

    let store = Store::open(&path).await.unwrap();
    let admin = store.admin().await.unwrap();
    assert_eq!(
        (admin.name.as_str(), admin.role, admin.state),
        ("owner", Role::Admin, UserState::Active)
    );
    assert!(admin.cert.is_none(), "pas de certificat avant admin_init");
    let devices = store.devices().await.unwrap();
    assert_eq!(devices.len(), 3, "révoqués compris");
    for d in &devices {
        assert_eq!(d.user_id, admin.id);
        assert!(d.reachable && !d.is_default && d.kind.is_none() && d.features.is_empty());
    }
    let laptop: DeviceId = "dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H1".parse().unwrap();
    assert_eq!(store.pending(&laptop, 0, 10, 2).await.unwrap().len(), 1, "message intact");
    let blob: BlobId = "blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V".parse().unwrap();
    assert!(store.blob(&blob, 2).await.unwrap().is_some(), "fichier intact");

    // Une installation à une personne continue comme avant : `to: []`.
    let phone: DeviceId = "dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H0".parse().unwrap();
    let ins = store.insert_message(msg("m2", &phone, &[]), 3).await.unwrap();
    assert_eq!(ins.recipients, vec![laptop]);

    // Rouvrir ne recrée rien.
    store.close().await;
    let again = Store::open(&path).await.unwrap();
    assert_eq!(again.users().await.unwrap().len(), 1);
    assert_eq!(again.admin().await.unwrap().id, admin.id);
}

#[tokio::test]
async fn device_names_are_unique_per_person_only() {
    let n = net().await;
    extra_device(&n.store, &n.bob, "tel").await;
    extra_device(&n.store, &n.carol, "tel").await;
    let dup = n.store.add_device_for(&n.bob, "tel", &[42; 32], 5).await;
    assert!(matches!(dup, Err(StoreError::NameTaken(_))));
    // Recours SSH : un nom porté par deux personnes est ambigu.
    assert!(matches!(n.store.revoke_device("tel", 6).await, Err(StoreError::Ambiguous(_))));
}

#[tokio::test]
async fn routing_follows_the_network() {
    let n = net().await;
    let s = &n.store;
    let ok = |r: Result<ot_server::store::Inserted, StoreError>| r.unwrap().recipients;
    let refused = |r: Result<ot_server::store::Inserted, StoreError>, id: &DeviceId| match r {
        Err(StoreError::NotAllowed(d)) => assert_eq!(&d, id),
        other => panic!("not_allowed attendu pour {id}, reçu {other:?}"),
    };

    // Ma personne : `to: []`, sans changement.
    assert_eq!(
        ok(s.insert_message(msg("a0", &n.alice_phone.id, &[]), 10).await),
        vec![n.alice_laptop.id.clone()]
    );
    assert_eq!(
        ok(s.insert_message(msg("b0", &n.bob_phone.id, &[]), 10).await),
        vec![n.bob_laptop.id.clone()]
    );
    // Personnes liées.
    let to = [n.bob_phone.id.clone(), n.alice_laptop.id.clone()];
    assert_eq!(ok(s.insert_message(msg("a1", &n.alice_phone.id, &to), 10).await).len(), 2);
    ok(s.insert_message(msg("b1", &n.bob_phone.id, std::slice::from_ref(&n.alice_phone.id)), 10)
        .await);
    // Pas de lien Bob ↔ Carol : tout l'envoi est refusé, rien n'est stocké.
    let to = [n.alice_phone.id.clone(), n.carol_phone.id.clone()];
    refused(s.insert_message(msg("b2", &n.bob_phone.id, &to), 10).await, &n.carol_phone.id);
    refused(
        s.insert_message(msg("c1", &n.carol_phone.id, std::slice::from_ref(&n.bob_phone.id)), 10)
            .await,
        &n.bob_phone.id,
    );
    let alice_pending = s.pending_count(&n.alice_phone.id, 11).await.unwrap();
    assert_eq!(alice_pending, 1, "seul b1 attend : b2 n'a rien laissé");

    // Appareil non joignable : refusé aux autres, pas à sa personne.
    let off = DeviceChanges { reachable: Some(false), ..DeviceChanges::default() };
    s.update_device(&n.bob_phone.id, &n.bob_laptop.id, &off).await.unwrap();
    refused(
        s.insert_message(msg("a2", &n.alice_phone.id, std::slice::from_ref(&n.bob_laptop.id)), 12)
            .await,
        &n.bob_laptop.id,
    );
    ok(s.insert_message(msg("b3", &n.bob_phone.id, std::slice::from_ref(&n.bob_laptop.id)), 12)
        .await);

    // Service : sens unique, jamais destinataire, coupé par le membre.
    ok(s.insert_message(msg("n1", &n.notifier_1.id, std::slice::from_ref(&n.bob_phone.id)), 13)
        .await);
    refused(
        s.insert_message(msg("n2", &n.notifier_1.id, std::slice::from_ref(&n.carol_phone.id)), 13)
            .await,
        &n.carol_phone.id,
    );
    refused(
        s.insert_message(msg("a3", &n.alice_phone.id, std::slice::from_ref(&n.notifier_1.id)), 13)
            .await,
        &n.notifier_1.id,
    );
    assert!(matches!(
        s.insert_message(msg("n3", &n.notifier_1.id, &[]), 13).await,
        Err(StoreError::Invalid(_))
    ));
    s.mute_service(&n.bob, &n.notifier, true).await.unwrap();
    refused(
        s.insert_message(msg("n4", &n.notifier_1.id, std::slice::from_ref(&n.bob_phone.id)), 14)
            .await,
        &n.bob_phone.id,
    );
    // L'admin repose le lien : le service reste coupé tant que Bob ne le rouvre pas.
    s.set_link(&n.notifier, &n.bob, true, 15).await.unwrap();
    refused(
        s.insert_message(msg("n5", &n.notifier_1.id, std::slice::from_ref(&n.bob_phone.id)), 15)
            .await,
        &n.bob_phone.id,
    );
    s.mute_service(&n.bob, &n.notifier, false).await.unwrap();
    ok(s.insert_message(msg("n6", &n.notifier_1.id, std::slice::from_ref(&n.bob_phone.id)), 16)
        .await);

    // Plus de 32 destinataires.
    let many: Vec<DeviceId> = (0..33).map(|_| DeviceId::generate()).collect();
    assert!(matches!(
        s.insert_message(msg("a4", &n.alice_phone.id, &many), 17).await,
        Err(StoreError::Invalid(_))
    ));

    // Lien retiré : refusé dans les deux sens.
    s.set_link(&n.alice, &n.carol, false, 18).await.unwrap();
    refused(
        s.insert_message(msg("a5", &n.alice_phone.id, std::slice::from_ref(&n.carol_phone.id)), 18)
            .await,
        &n.carol_phone.id,
    );
    refused(
        s.insert_message(msg("c2", &n.carol_phone.id, std::slice::from_ref(&n.alice_phone.id)), 18)
            .await,
        &n.alice_phone.id,
    );

    // Personne révoquée : ses appareils n'existent plus pour le relais.
    s.revoke_user(&n.carol, 19).await.unwrap();
    assert!(matches!(
        s.insert_message(msg("a6", &n.alice_phone.id, std::slice::from_ref(&n.carol_phone.id)), 19)
            .await,
        Err(StoreError::UnknownRecipient(_))
    ));
}

#[tokio::test]
async fn a_pending_person_only_reaches_its_inviter() {
    let n = net().await;
    let s = &n.store;
    let hash = token_hash(&[77; 10]);
    let dave =
        s.invite_user(&n.alice_phone.id, "Dave", &[], &hash, 5, 20, 48 * HOUR).await.unwrap();
    let id = Identity::generate();
    let dave_phone =
        s.redeem_invitation(&hash, "dave-phone", &id.public_key(), 21).await.unwrap().device.id;

    // L'invitation relie ses deux appareils, même sans lien entre personnes.
    s.insert_message(msg("d1", &dave_phone, std::slice::from_ref(&n.alice_phone.id)), 22)
        .await
        .unwrap();
    s.insert_message(msg("a1", &n.alice_phone.id, std::slice::from_ref(&dave_phone)), 22)
        .await
        .unwrap();
    for (i, to) in [&n.alice_laptop.id, &n.bob_phone.id].into_iter().enumerate() {
        let r = s
            .insert_message(msg(&format!("d{}", i + 2), &dave_phone, std::slice::from_ref(to)), 22)
            .await;
        assert!(matches!(r, Err(StoreError::Forbidden(_))), "{r:?}");
    }
    let r = s
        .insert_message(msg("a2", &n.alice_laptop.id, std::slice::from_ref(&dave_phone)), 22)
        .await;
    assert!(matches!(r, Err(StoreError::NotAllowed(_))), "personne en attente : {r:?}");

    // Certifiée mais sans lien : l'invitation reste le seul chemin, le temps
    // que l'admin envoie sa réponse.
    s.certify_user(&dave, &blob(11)).await.unwrap();
    s.insert_message(msg("a3", &n.alice_phone.id, std::slice::from_ref(&dave_phone)), 23)
        .await
        .unwrap();
    s.insert_message(msg("d4", &dave_phone, std::slice::from_ref(&n.alice_phone.id)), 23)
        .await
        .unwrap();
    let r = s
        .insert_message(msg("d5", &dave_phone, std::slice::from_ref(&n.alice_laptop.id)), 23)
        .await;
    assert!(matches!(r, Err(StoreError::NotAllowed(_))), "{r:?}");
    // Dave publie son premier roster : l'échange est fini, plus de chemin.
    s.publish_entry(&dave_phone, &dave, 1, &blob(12)).await.unwrap();
    for (id, from, to) in
        [("d6", &dave_phone, &n.alice_phone.id), ("a4", &n.alice_phone.id, &dave_phone)]
    {
        let r = s.insert_message(msg(id, from, std::slice::from_ref(to)), 24).await;
        assert!(matches!(r, Err(StoreError::NotAllowed(_))), "{r:?}");
    }
}

#[tokio::test]
async fn blobs_follow_the_network_and_member_quota() {
    let n = net().await;
    let s = &n.store;
    fn new<'a>(
        id: &'a BlobId,
        owner: &'a TestDevice,
        to: &'a [DeviceId],
        size: u64,
    ) -> NewBlob<'a> {
        NewBlob { id, owner: &owner.id, size, chunk_size: 16, chunks: 1, to, expires_at: i64::MAX }
    }
    let (b1, b2, b3, b4) =
        (BlobId::generate(), BlobId::generate(), BlobId::generate(), BlobId::generate());
    let to_alice = [n.alice_phone.id.clone()];
    let to_carol = [n.carol_phone.id.clone()];
    s.create_blob(new(&b1, &n.bob_phone, &to_alice, 600), 10_000, 1_000, 1).await.unwrap();
    assert!(matches!(
        s.create_blob(new(&b2, &n.bob_phone, &to_carol, 10), 10_000, 1_000, 1).await,
        Err(StoreError::NotAllowed(_))
    ));
    // Quota du membre : sa somme, tous appareils confondus.
    assert!(matches!(
        s.create_blob(new(&b3, &n.bob_laptop, &to_alice, 500), 10_000, 1_000, 1).await,
        Err(StoreError::UserQuota(_))
    ));
    // Sans effet sur l'admin, qui n'a que le quota global.
    let to_bob = [n.bob_phone.id.clone()];
    s.create_blob(new(&b4, &n.alice_phone, &to_bob, 5_000), 10_000, 1_000, 1).await.unwrap();
}

#[tokio::test]
async fn blob_creation_out_of_network_is_403_not_allowed() {
    let n = net().await;
    let (bob, carol) = (n.bob_phone, n.carol_phone);
    let server = TestServer::start(n.store, test_config()).await;
    let req = CreateBlob { size: 16, chunk_size: 16, chunks: 1, to: vec![carol.id.clone()] };
    let res = signed(&server, &bob, "POST", "/v1/blobs", &serde_json::to_vec(&req).unwrap()).await;
    assert_eq!(res.status, 403);
    let err: ot_proto::http::HttpError = res.json();
    assert_eq!(err.error, HttpErrorCode::NotAllowed);
}

#[tokio::test]
async fn member_quotas_devices_and_invitations() {
    let n = net().await;
    let s = &n.store;
    // Bob a deux appareils : six de plus avec une limite de 8.
    for i in 0..6 {
        let token = [100 + i; 10];
        s.create_pairing(&n.bob_phone.id, &token_hash(&token), 30, 40 * HOUR).await.unwrap();
        s.redeem_pairing(&token_hash(&token), &format!("bob-{i}"), &[200 + i; 32], 8, 31)
            .await
            .unwrap();
    }
    let token = [99; 10];
    s.create_pairing(&n.bob_phone.id, &token_hash(&token), 30, 40 * HOUR).await.unwrap();
    assert!(matches!(
        s.redeem_pairing(&token_hash(&token), "bob-9", &[250; 32], 8, 31).await,
        Err(StoreError::UserQuota(_))
    ));
    // Même chemin par QR inversé.
    let r = s
        .register_device(&n.bob_phone.id, &DeviceId::generate(), "bob-qr", &[251; 32], 8, 32)
        .await;
    assert!(matches!(r, Err(StoreError::UserQuota(_))));
    // L'admin n'a que la limite globale.
    for i in 0..8 {
        s.register_device(
            &n.alice_phone.id,
            &DeviceId::generate(),
            &format!("a-{i}"),
            &[150 + i; 32],
            8,
            33,
        )
        .await
        .unwrap();
    }
    // Un service ne parraine aucun appareil (pas de groupe).
    let r =
        s.register_device(&n.notifier_1.id, &DeviceId::generate(), "x", &[252; 32], 8, 33).await;
    assert!(matches!(r, Err(StoreError::Forbidden(_))));

    // Invitations utilisables : 5 au plus (deux déjà utilisées par Bob et Carol).
    for i in 0..5 {
        let hash = token_hash(&[i; 10]);
        s.invite_user(&n.alice_phone.id, &format!("p{i}"), &[], &hash, 5, 40, 48 * HOUR)
            .await
            .unwrap();
    }
    let sixth =
        s.invite_user(&n.alice_phone.id, "p6", &[], &token_hash(&[9; 10]), 5, 41, 48 * HOUR).await;
    assert!(matches!(sixth, Err(StoreError::UserQuota(_))));
}

#[tokio::test]
async fn invitation_tokens_expire_are_single_use_and_cancelled_by_revocation() {
    let n = net().await;
    let s = &n.store;
    let (h1, h2, h3) = (token_hash(&[1; 10]), token_hash(&[2; 10]), token_hash(&[3; 10]));
    s.invite_user(
        &n.alice_phone.id,
        "Eve",
        std::slice::from_ref(&n.alice),
        &h1,
        5,
        100,
        100 + HOUR,
    )
    .await
    .unwrap();
    let frank =
        s.invite_user(&n.alice_phone.id, "Frank", &[], &h2, 5, 100, 100 + HOUR).await.unwrap();
    s.invite_user(&n.alice_phone.id, "Grace", &[], &h3, 5, 100, 100 + HOUR).await.unwrap();
    // Nom déjà pris par une personne.
    let dup = s
        .invite_user(&n.alice_phone.id, "Bob", &[], &token_hash(&[4; 10]), 5, 100, 100 + HOUR)
        .await;
    assert!(matches!(dup, Err(StoreError::NameTaken(_))));

    assert!(
        matches!(
            s.redeem_invitation(&h1, "eve-phone", &[61; 32], 100 + HOUR).await,
            Err(StoreError::PairingInvalid)
        ),
        "expirée"
    );
    s.redeem_invitation(&h3, "grace-phone", &[63; 32], 101).await.unwrap();
    assert!(
        matches!(
            s.redeem_invitation(&h3, "grace-tablet", &[64; 32], 102).await,
            Err(StoreError::PairingInvalid)
        ),
        "déjà utilisée"
    );
    s.revoke_user(&frank, 103).await.unwrap();
    assert!(
        matches!(
            s.redeem_invitation(&h2, "frank-phone", &[62; 32], 104).await,
            Err(StoreError::PairingInvalid)
        ),
        "annulée par la révocation"
    );
    // Une clé déjà connue ne consomme pas le jeton.
    let h5 = token_hash(&[5; 10]);
    s.invite_user(&n.alice_phone.id, "Heidi", &[], &h5, 5, 105, 105 + HOUR).await.unwrap();
    let taken = n.bob_phone.identity.public_key();
    assert!(matches!(
        s.redeem_invitation(&h5, "heidi", &taken, 106).await,
        Err(StoreError::KeyTaken)
    ));
    s.redeem_invitation(&h5, "heidi", &[65; 32], 107).await.unwrap();
}

#[tokio::test]
async fn invitations_need_the_admin_certificate() {
    let store = Store::in_memory().await.unwrap();
    let a = add_device(&store, "alice-phone").await;
    let r = store.invite_user(&a.id, "Bob", &[], &token_hash(&[1; 10]), 5, 0, HOUR).await;
    assert!(matches!(r, Err(StoreError::NoAnchor)));
    store.admin_init(&blob(1), &blob(2)).await.unwrap();
    assert!(matches!(store.admin_init(&blob(1), &blob(2)).await, Err(StoreError::Invalid(_))));
    store.invite_user(&a.id, "Bob", &[], &token_hash(&[1; 10]), 5, 0, HOUR).await.unwrap();
}

#[tokio::test]
async fn reinvite_revokes_devices_and_forgets_keys_but_keeps_links() {
    let n = net().await;
    let s = &n.store;
    let hash = token_hash(&[33; 10]);
    let revoked =
        s.reinvite_user(&n.alice_phone.id, &n.bob, &hash, 5, 50, 50 + HOUR).await.unwrap();
    assert_eq!(revoked.len(), 2);
    let bob = s.user(&n.bob).await.unwrap().unwrap();
    assert_eq!((bob.state, bob.cert), (UserState::Pending, None));
    assert_eq!(s.last_seq(&n.bob).await.unwrap(), 0, "journal oublié");
    let r = s.redeem_invitation(&hash, "bob-phone", &[71; 32], 51).await.unwrap();
    s.certify_user(&n.bob, &blob(21)).await.unwrap();
    s.insert_message(msg("a1", &n.alice_phone.id, std::slice::from_ref(&r.device.id)), 52)
        .await
        .unwrap();
    s.publish_entry(&r.device.id, &n.bob, 1, &blob(22)).await.unwrap();
}

#[tokio::test]
async fn roster_log_is_append_only_and_owned() {
    let n = net().await;
    let s = &n.store;
    // Le journal de l'admin commence à admin_init (seq 1).
    assert!(matches!(
        s.publish_entry(&n.alice_phone.id, &n.alice, 1, &blob(30)).await,
        Err(StoreError::Conflict(2))
    ));
    s.publish_entry(&n.alice_laptop.id, &n.alice, 2, &blob(31)).await.unwrap();
    s.publish_entry(&n.bob_laptop.id, &n.bob, 2, &blob(32)).await.unwrap();
    assert!(matches!(
        s.publish_entry(&n.bob_phone.id, &n.bob, 2, &blob(33)).await,
        Err(StoreError::Conflict(3))
    ));
    assert!(matches!(
        s.publish_entry(&n.bob_phone.id, &n.carol, 2, &blob(34)).await,
        Err(StoreError::Forbidden(_))
    ));
    // L'admin publie pour un service, pas pour un membre ; le service jamais.
    s.publish_entry(&n.alice_phone.id, &n.notifier, 1, &blob(35)).await.unwrap();
    assert!(matches!(
        s.publish_entry(&n.alice_phone.id, &n.bob, 3, &blob(36)).await,
        Err(StoreError::Forbidden(_))
    ));
    assert!(matches!(
        s.publish_entry(&n.notifier_1.id, &n.notifier, 2, &blob(37)).await,
        Err(StoreError::Forbidden(_))
    ));
    let huge = SignedBlob { data: vec![b'x'; 20_000], sig: [0; 64] };
    assert!(matches!(
        s.publish_entry(&n.bob_phone.id, &n.bob, 3, &huge).await,
        Err(StoreError::Invalid(_))
    ));
}

#[tokio::test]
async fn device_profiles_follow_their_rules() {
    let n = net().await;
    let s = &n.store;
    let mine =
        |kind, reachable, default| DeviceChanges { kind, reachable, default, features: None };

    // `cli` n'est jamais joignable ni par défaut.
    s.update_device(
        &n.alice_phone.id,
        &n.alice_laptop.id,
        &mine(Some(DeviceKind::Cli), Some(true), Some(true)),
    )
    .await
    .unwrap();
    let laptop = s.device(&n.alice_laptop.id).await.unwrap().unwrap();
    assert_eq!(
        (laptop.kind, laptop.reachable, laptop.is_default),
        (Some(DeviceKind::Cli), false, false)
    );
    // Une catégorie inconnue ne change rien.
    s.update_device(
        &n.alice_phone.id,
        &n.alice_laptop.id,
        &mine(Some(DeviceKind::Unknown), None, None),
    )
    .await
    .unwrap();
    assert_eq!(s.device(&n.alice_laptop.id).await.unwrap().unwrap().kind, Some(DeviceKind::Cli));

    // Bob a un appareil par défaut (son premier) : il ne peut pas le perdre.
    let r = s.update_device(&n.bob_phone.id, &n.bob_phone.id, &mine(None, None, Some(false))).await;
    assert!(matches!(r, Err(StoreError::Invalid(_))), "{r:?}");
    s.update_device(&n.bob_phone.id, &n.bob_laptop.id, &mine(None, None, Some(true)))
        .await
        .unwrap();
    s.update_device(&n.bob_phone.id, &n.bob_phone.id, &mine(None, None, Some(false)))
        .await
        .unwrap();

    // Seul l'appareil déclare ses capacités ; personne ne règle ceux d'un autre.
    let features =
        DeviceChanges { features: Some(vec![Feature::users_v1()]), ..DeviceChanges::default() };
    assert!(matches!(
        s.update_device(&n.bob_phone.id, &n.bob_laptop.id, &features).await,
        Err(StoreError::Invalid(_))
    ));
    s.update_device(&n.bob_laptop.id, &n.bob_laptop.id, &features).await.unwrap();
    assert_eq!(
        s.device(&n.bob_laptop.id).await.unwrap().unwrap().features,
        vec![Feature::users_v1()]
    );
    assert!(matches!(
        s.update_device(&n.carol_phone.id, &n.bob_phone.id, &mine(None, Some(false), None)).await,
        Err(StoreError::Forbidden(_))
    ));
    // L'admin règle tous les appareils.
    s.update_device(
        &n.alice_phone.id,
        &n.carol_phone.id,
        &mine(Some(DeviceKind::Phone), None, None),
    )
    .await
    .unwrap();

    // Retrait : sa personne (l'admin : tous), jamais soi-même.
    assert!(matches!(
        s.revoke_device_by(&n.bob_phone.id, &n.bob_phone.id, 60).await,
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        s.revoke_device_by(&n.carol_phone.id, &n.bob_laptop.id, 60).await,
        Err(StoreError::Forbidden(_))
    ));
    assert_eq!(s.revoke_device_by(&n.bob_phone.id, &n.bob_laptop.id, 60).await.unwrap(), n.bob);
    s.revoke_device_by(&n.alice_phone.id, &n.carol_phone.id, 61).await.unwrap();
}

#[tokio::test]
async fn directory_is_filtered_by_the_network() {
    let n = net().await;
    let s = &n.store;
    let off = DeviceChanges { reachable: Some(false), ..DeviceChanges::default() };
    s.update_device(&n.bob_phone.id, &n.bob_laptop.id, &off).await.unwrap();
    s.publish_entry(&n.bob_phone.id, &n.bob, 2, &blob(41)).await.unwrap();

    let ids =
        |users: &[ot_proto::DirectoryUser]| users.iter().map(|u| u.id.clone()).collect::<Vec<_>>();

    // Carol : elle-même, l'admin ; ni Bob ni le service.
    let (admin, users) = s.directory(&n.carol, &Known::new()).await.unwrap();
    assert_eq!(admin, n.alice);
    assert_eq!(ids(&users), vec![n.carol.clone(), n.alice.clone()]);
    assert_eq!(users[1].devices.len(), 2, "Alice est liée à Carol");

    // Bob : lui-même (tous ses appareils), Alice, le service.
    let (_, users) = s.directory(&n.bob, &Known::new()).await.unwrap();
    assert_eq!(ids(&users), vec![n.bob.clone(), n.alice.clone(), n.notifier.clone()]);
    assert_eq!(users[0].devices.len(), 2, "mes appareils, non joignables compris");
    assert_eq!(users[0].log.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
    assert!(users[2].devices.is_empty(), "un service n'a aucun appareil joignable");

    // Alice voit seulement l'appareil joignable de Bob, et la suite du journal.
    let known = Known::from([(n.bob.clone(), 1)]);
    let (_, users) = s.directory(&n.alice, &known).await.unwrap();
    let bob = users.iter().find(|u| u.id == n.bob).unwrap();
    assert_eq!(
        bob.devices.iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
        vec![n.bob_phone.id.clone()]
    );
    assert_eq!(bob.log.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2]);
    assert!(users.iter().any(|u| u.id == n.carol), "Alice est liée à Carol");

    // Un admin non lié n'apparaît que par son certificat et son journal.
    s.set_link(&n.alice, &n.carol, false, 70).await.unwrap();
    let (_, users) = s.directory(&n.carol, &Known::new()).await.unwrap();
    assert_eq!(ids(&users), vec![n.carol.clone(), n.alice.clone()]);
    assert!(users[1].devices.is_empty() && users[1].cert.is_some() && !users[1].log.is_empty());

    // Le service voit la coupure du point de vue du membre.
    s.mute_service(&n.bob, &n.notifier, true).await.unwrap();
    let (_, users) = s.directory(&n.notifier, &Known::new()).await.unwrap();
    assert!(users.iter().find(|u| u.id == n.bob).unwrap().muted);
    let (_, users) = s.directory(&n.bob, &Known::new()).await.unwrap();
    assert!(users.iter().find(|u| u.id == n.notifier).unwrap().muted);
}

#[tokio::test]
async fn directory_pages_long_logs() {
    let n = net().await;
    let s = &n.store;
    let big = |seed| SignedBlob { data: vec![seed; 15_000], sig: [seed; 64] };
    for seq in 2..=10 {
        s.publish_entry(&n.bob_phone.id, &n.bob, seq, &big(seq as u8)).await.unwrap();
    }
    let mut known = Known::new();
    let mut pages = 0;
    loop {
        let (_, users) = s.directory(&n.bob, &known).await.unwrap();
        pages += 1;
        let me = &users[0];
        let frame =
            Frame::Directory { me: n.bob.clone(), admin: n.alice.clone(), users: users.clone() };
        assert!(frame.to_json().len() < ot_proto::MAX_FRAME_BYTES, "trame trop grosse");
        if let Some(last) = me.log.last() {
            known.insert(n.bob.clone(), last.seq);
        }
        if !me.more {
            break;
        }
    }
    assert_eq!(known.get(&n.bob), Some(&10));
    assert!(pages >= 3, "{pages} pages");
}

#[tokio::test]
async fn revoking_a_person_cleans_up_everything() {
    let n = net().await;
    let s = &n.store;
    s.insert_message(msg("a1", &n.alice_phone.id, std::slice::from_ref(&n.bob_phone.id)), 80)
        .await
        .unwrap();
    let b = BlobId::generate();
    let to = [n.alice_phone.id.clone()];
    s.create_blob(
        NewBlob {
            id: &b,
            owner: &n.bob_phone.id,
            size: 16,
            chunk_size: 16,
            chunks: 1,
            to: &to,
            expires_at: i64::MAX,
        },
        1 << 20,
        1 << 20,
        80,
    )
    .await
    .unwrap();
    s.mute_service(&n.bob, &n.notifier, true).await.unwrap();

    let revoked = s.revoke_user(&n.bob, 81).await.unwrap();
    assert_eq!(revoked.devices.len(), 2);
    assert_eq!(revoked.blobs, vec![b.clone()]);
    assert!(revoked.notify.contains(&n.alice_phone.id), "l'admin est prévenue");
    assert!(s.blob(&b, 82).await.unwrap().is_none());
    assert_eq!(s.pending_count(&n.bob_phone.id, 82).await.unwrap(), 0);
    assert!(s.links().await.unwrap().iter().all(|l| l.from != n.bob && l.to != n.bob));
    assert_eq!(s.user(&n.bob).await.unwrap().unwrap().state, UserState::Revoked);
    assert!(s.peer(&n.bob_phone.id).await.unwrap().is_none());
    // Son nom redevient libre ; l'admin ne se révoque pas.
    s.invite_user(&n.alice_phone.id, "Bob", &[], &token_hash(&[8; 10]), 5, 83, 83 + HOUR)
        .await
        .unwrap();
    assert!(matches!(s.revoke_user(&n.alice, 84).await, Err(StoreError::Invalid(_))));
}

#[tokio::test]
async fn admin_reset_puts_everyone_back_to_pending() {
    let n = net().await;
    let s = &n.store;
    assert_eq!(s.admin_reset().await.unwrap(), 3, "Bob, Carol et le service");
    assert!(s.admin().await.unwrap().cert.is_none());
    assert_eq!(s.last_seq(&n.alice).await.unwrap(), 0);
    assert_eq!(s.user(&n.bob).await.unwrap().unwrap().state, UserState::Pending);
    let r = s
        .insert_message(msg("b1", &n.bob_phone.id, std::slice::from_ref(&n.alice_phone.id)), 90)
        .await;
    assert!(matches!(r, Err(StoreError::Forbidden(_))), "{r:?}");
    // L'admin réinitialise son ancre, puis réinvite.
    s.admin_init(&blob(1), &blob(2)).await.unwrap();
    s.reinvite_user(&n.alice_phone.id, &n.bob, &token_hash(&[9; 10]), 5, 91, 91 + HOUR)
        .await
        .unwrap();
}

// --- Par WebSocket -----------------------------------------------------------

/// Prochaine trame, en sautant les `directory_changed` poussés entre-temps.
async fn reply(raw: &mut Raw) -> Option<Frame> {
    loop {
        match raw.recv().await {
            Some(Frame::DirectoryChanged) => {}
            other => return other,
        }
    }
}

async fn error_code(raw: &mut Raw) -> ErrorCode {
    match raw.recv().await {
        Some(Frame::Error { code, .. }) => code,
        other => panic!("erreur attendue, reçu {other:?}"),
    }
}

#[tokio::test]
async fn admin_frames_are_forbidden_to_members_and_services() {
    let n = net().await;
    let (alice, bob, notifier, carol) =
        (n.alice.clone(), n.bob_phone, n.notifier_1, n.carol.clone());
    let server = TestServer::start(n.store, test_config()).await;
    let frames = vec![
        Frame::AdminInit { cert: blob(1), entry: blob(2) },
        Frame::UserInvite { name: "Mallory".into(), role: Role::Member, links: vec![] },
        Frame::UserReinvite { user: carol.clone() },
        Frame::UserCertify { user: carol.clone(), cert: blob(3) },
        Frame::ServiceCreate {
            user: UserId::generate(),
            name: "x".into(),
            device: DeviceId::generate(),
            public_key: [1; 32],
            device_name: "x".into(),
            sig: [0; 64],
            cert: blob(4),
        },
        Frame::ServiceDevice {
            user: UserId::generate(),
            device: DeviceId::generate(),
            public_key: [1; 32],
            name: "x".into(),
            sig: [0; 64],
        },
        Frame::UserRevoke { user: carol.clone() },
        Frame::UserRename { user: carol.clone(), name: "x".into() },
        Frame::LinkSet { from: carol.clone(), to: alice.clone(), allowed: true },
        Frame::UsersGet,
    ];
    for dev in [&bob, &notifier] {
        let (mut raw, _) = Raw::login(&server, dev).await;
        for f in &frames {
            raw.send(f).await;
            assert_eq!(
                error_code(&mut raw).await,
                ErrorCode::Forbidden,
                "{} par {}",
                f.type_name(),
                dev.name
            );
        }
        // La session continue.
        raw.send(&Frame::Ping).await;
        assert_eq!(raw.recv().await, Some(Frame::Pong));
    }
    // Un service ne crée pas de jeton d'appairage, ne coupe pas de service.
    let (mut raw, _) = Raw::login(&server, &notifier).await;
    raw.send(&Frame::PairCreate).await;
    assert_eq!(error_code(&mut raw).await, ErrorCode::Forbidden);
    raw.send(&Frame::ServiceMute { service: UserId::generate(), muted: true }).await;
    assert_eq!(error_code(&mut raw).await, ErrorCode::Forbidden);
}

/// Parcours complet d'une invitation entre deux clients bruts : jeton,
/// premier appareil, session en attente, certification, session active.
#[tokio::test]
async fn invitation_round_trip_over_websocket() {
    let store = Store::in_memory().await.unwrap();
    let alice = add_device(&store, "alice-phone").await;
    let server = TestServer::start(store, test_config()).await;
    let (mut admin, _) = Raw::login(&server, &alice).await;

    admin.send(&Frame::UserInvite { name: "Bob".into(), role: Role::Member, links: vec![] }).await;
    assert_eq!(error_code(&mut admin).await, ErrorCode::UnexpectedFrame, "admin_init d'abord");
    admin.send(&Frame::AdminInit { cert: blob(1), entry: blob(2) }).await;
    let Some(Frame::AdminInitialized { user: alice_user }) = admin.recv().await else {
        panic!("admin_initialized attendu")
    };
    admin
        .send(&Frame::UserInvite {
            name: "Bob".into(),
            role: Role::Member,
            links: vec![alice_user.clone()],
        })
        .await;
    let Some(Frame::UserInvited { user: bob_user, token, .. }) = admin.recv().await else {
        panic!("user_invited attendu")
    };

    // Premier appareil de Bob : signature de l'invitation, puis fermeture.
    let identity = Identity::generate();
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    let bad = sign(&identity, &invite_redeem_message(&[0; 32], &token_hash(&token)));
    raw.send(&Frame::InviteRedeem {
        token,
        name: "bob-phone".into(),
        public_key: identity.public_key(),
        sig: bad,
    })
    .await;
    assert_eq!(error_code(&mut raw).await, ErrorCode::PairingInvalid);
    let mut raw = Raw::connect(&server).await;
    let nonce2 = raw.challenge().await;
    assert_ne!(nonce, nonce2);
    let sig = sign(&identity, &invite_redeem_message(&nonce2, &token_hash(&token)));
    raw.send(&Frame::InviteRedeem {
        token,
        name: "bob-phone".into(),
        public_key: identity.public_key(),
        sig,
    })
    .await;
    let Some(Frame::InviteRedeemed { user, device: bob_id, inviter }) = raw.recv().await else {
        panic!("invite_redeemed attendu")
    };
    assert_eq!((user, &inviter), (bob_user.clone(), &alice.id));
    assert_eq!(raw.recv().await, None, "fermée après la réponse");

    // Réutilisé : refusé.
    let mut again = Raw::connect(&server).await;
    let nonce3 = again.challenge().await;
    let other = Identity::generate();
    let sig = sign(&other, &invite_redeem_message(&nonce3, &token_hash(&token)));
    again
        .send(&Frame::InviteRedeem {
            token,
            name: "bob-2".into(),
            public_key: other.public_key(),
            sig,
        })
        .await;
    assert_eq!(error_code(&mut again).await, ErrorCode::PairingInvalid);

    // Session en attente : seulement l'invitant, l'annuaire, ack, ping.
    let bob = device(bob_id.clone(), "bob-phone", identity);
    let (mut b, _) = Raw::login(&server, &bob).await;
    b.send(&Frame::DeviceUpdate {
        device: bob_id.clone(),
        kind: Some(DeviceKind::Phone),
        reachable: None,
        default: None,
        features: None,
    })
    .await;
    assert_eq!(error_code(&mut b).await, ErrorCode::Forbidden);
    b.send(&Frame::RosterPublish { user: bob_user.clone(), seq: 1, entry: blob(5) }).await;
    assert_eq!(error_code(&mut b).await, ErrorCode::Forbidden);
    b.send(&Frame::PairCreate).await;
    assert_eq!(error_code(&mut b).await, ErrorCode::Forbidden);
    b.send(&Frame::DirectoryGet { known: Known::new() }).await;
    let Some(Frame::Directory { me, admin: anchor, users }) = b.recv().await else {
        panic!("directory attendu")
    };
    assert_eq!((me, anchor), (bob_user.clone(), alice_user.clone()));
    assert_eq!(users.len(), 2, "Bob et l'admin");
    let env =
        Envelope::new_plain(bob_id.clone(), vec![alice.id.clone()], &Content::text("demande"));
    b.send(&Frame::Send { env: env.clone() }).await;
    assert!(matches!(b.recv().await, Some(Frame::Accepted { .. })));
    assert!(matches!(admin.recv().await, Some(Frame::Deliver { .. })));

    // Certification : la même session devient active.
    admin.send(&Frame::UserCertify { user: bob_user.clone(), cert: blob(6) }).await;
    assert_eq!(admin.recv().await, Some(Frame::UserCertified { user: bob_user.clone() }));
    // Bob est prévenu que son annuaire a changé.
    assert_eq!(b.recv().await, Some(Frame::DirectoryChanged));
    b.send(&Frame::RosterPublish { user: bob_user.clone(), seq: 1, entry: blob(7) }).await;
    assert_eq!(b.recv().await, Some(Frame::RosterPublished { user: bob_user.clone(), seq: 1 }));
    b.send(&Frame::RosterPublish { user: bob_user.clone(), seq: 1, entry: blob(8) }).await;
    assert_eq!(error_code(&mut b).await, ErrorCode::Conflict);
    b.send(&Frame::DeviceUpdate {
        device: bob_id.clone(),
        kind: Some(DeviceKind::Phone),
        reachable: None,
        default: None,
        features: Some(vec![Feature::users_v1()]),
    })
    .await;
    assert_eq!(b.recv().await, Some(Frame::DeviceUpdated { device: bob_id.clone() }));

    // L'admin voit tout ; `list_devices` ne montre que sa personne.
    admin.send(&Frame::UsersGet).await;
    let Some(Frame::Users { users }) = reply(&mut admin).await else { panic!("users attendu") };
    let bob_seen = users.iter().find(|u| u.id == bob_user).unwrap();
    assert_eq!(bob_seen.state, UserState::Active);
    assert!(bob_seen.devices[0].online);
    assert_eq!(bob_seen.links, vec![alice_user.clone()]);
    b.send(&Frame::ListDevices).await;
    let Some(Frame::Devices { devices }) = b.recv().await else { panic!("devices attendu") };
    assert_eq!(devices.iter().map(|d| d.id.clone()).collect::<Vec<_>>(), vec![bob_id.clone()]);
    admin.send(&Frame::ListDevices).await;
    let Some(Frame::Devices { devices }) = reply(&mut admin).await else {
        panic!("devices attendu")
    };
    assert_eq!(devices.iter().map(|d| d.id.clone()).collect::<Vec<_>>(), vec![alice.id.clone()]);

    // Révocation : la session de Bob est fermée, sa reconnexion refusée.
    admin.send(&Frame::UserRevoke { user: bob_user.clone() }).await;
    assert_eq!(reply(&mut admin).await, Some(Frame::UserRevoked { user: bob_user }));
    within(async {
        while let Some(f) = b.recv().await {
            assert!(matches!(f, Frame::DirectoryChanged), "{f:?}");
        }
    })
    .await;
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    raw.send(&Frame::Auth {
        device_id: bob_id.clone(),
        sig: bob.identity.sign_challenge(&nonce, &bob_id),
    })
    .await;
    assert_eq!(error_code(&mut raw).await, ErrorCode::AuthFailed);
}

#[tokio::test]
async fn expired_invitation_is_refused_over_websocket() {
    let store = Store::in_memory().await.unwrap();
    let alice = add_device(&store, "alice-phone").await;
    let config = Config {
        users: UserLimits { invite_ttl: Duration::from_millis(1), ..UserLimits::default() },
        ..test_config()
    };
    let server = TestServer::start(store, config).await;
    let (mut admin, _) = Raw::login(&server, &alice).await;
    admin.send(&Frame::AdminInit { cert: blob(1), entry: blob(2) }).await;
    admin.recv().await;
    admin.send(&Frame::UserInvite { name: "Bob".into(), role: Role::Member, links: vec![] }).await;
    let Some(Frame::UserInvited { token, .. }) = admin.recv().await else { panic!() };
    tokio::time::sleep(Duration::from_millis(10)).await;
    let identity = Identity::generate();
    let mut raw = Raw::connect(&server).await;
    let nonce = raw.challenge().await;
    let sig = sign(&identity, &invite_redeem_message(&nonce, &token_hash(&token)));
    raw.send(&Frame::InviteRedeem {
        token,
        name: "bob-phone".into(),
        public_key: identity.public_key(),
        sig,
    })
    .await;
    assert_eq!(error_code(&mut raw).await, ErrorCode::PairingInvalid);
    // `role` autre que `member` : refusé.
    admin.send(&Frame::UserInvite { name: "Root".into(), role: Role::Admin, links: vec![] }).await;
    assert_eq!(error_code(&mut admin).await, ErrorCode::BadFrame);
}

#[tokio::test]
async fn send_rate_limits_members_only() {
    let n = net().await;
    let (alice, bob) = (n.alice_phone, n.bob_phone);
    let config =
        Config { users: UserLimits { send_rate: 3, ..UserLimits::default() }, ..test_config() };
    let server = TestServer::start(n.store, config).await;
    let (mut rb, _) = Raw::login(&server, &bob).await;
    let (mut ra, _) = Raw::login(&server, &alice).await;
    let send = |from: &TestDevice, to: &TestDevice| {
        Envelope::new_plain(from.id.clone(), vec![to.id.clone()], &Content::text("x"))
    };
    for _ in 0..3 {
        rb.send(&Frame::Send { env: send(&bob, &alice) }).await;
        assert!(matches!(rb.recv().await, Some(Frame::Accepted { .. })));
    }
    let env = send(&bob, &alice);
    rb.send(&Frame::Send { env: env.clone() }).await;
    match rb.recv().await {
        Some(Frame::Error { code: ErrorCode::QuotaExceeded, reference, .. }) => {
            assert_eq!(reference, Some(env.msg_id.to_string()));
        }
        other => panic!("quota_exceeded attendu, reçu {other:?}"),
    }
    // L'admin n'a pas de limite propre.
    for _ in 0..10 {
        ra.send(&Frame::Send { env: send(&alice, &bob) }).await;
        loop {
            match ra.recv().await {
                Some(Frame::Accepted { .. }) => break,
                Some(Frame::Deliver { .. }) => {}
                other => panic!("accepted attendu, reçu {other:?}"),
            }
        }
    }
}

#[tokio::test]
async fn sends_out_of_network_are_refused_over_websocket() {
    let n = net().await;
    let (bob, carol) = (n.bob_phone, n.carol_phone);
    let server = TestServer::start(n.store, test_config()).await;
    let (mut rb, _) = Raw::login(&server, &bob).await;
    let env = Envelope::new_plain(bob.id.clone(), vec![carol.id.clone()], &Content::text("x"));
    rb.send(&Frame::Send { env: env.clone() }).await;
    match rb.recv().await {
        Some(Frame::Error { code: ErrorCode::NotAllowed, reference, .. }) => {
            assert_eq!(reference, Some(env.msg_id.to_string()));
        }
        other => panic!("not_allowed attendu, reçu {other:?}"),
    }
}

#[tokio::test]
async fn link_changes_and_device_revocation_are_pushed() {
    let n = net().await;
    let (alice, bob_user, carol_user) = (n.alice_phone, n.bob.clone(), n.carol.clone());
    let (bob, bob_laptop, carol) = (n.bob_phone, n.bob_laptop, n.carol_phone);
    let server = TestServer::start(n.store, test_config()).await;
    let (mut ra, _) = Raw::login(&server, &alice).await;
    let (mut rb, _) = Raw::login(&server, &bob).await;
    let (mut rc, _) = Raw::login(&server, &carol).await;
    let (mut rl, _) = Raw::login(&server, &bob_laptop).await;

    ra.send(&Frame::LinkSet { from: bob_user.clone(), to: carol_user.clone(), allowed: true })
        .await;
    loop {
        match ra.recv().await {
            Some(Frame::LinkUpdated { allowed: true, .. }) => break,
            Some(Frame::DirectoryChanged) => {}
            other => panic!("link_updated attendu, reçu {other:?}"),
        }
    }
    assert_eq!(rb.recv().await, Some(Frame::DirectoryChanged));
    assert_eq!(rc.recv().await, Some(Frame::DirectoryChanged));
    let env = Envelope::new_plain(bob.id.clone(), vec![carol.id.clone()], &Content::text("x"));
    rb.send(&Frame::Send { env }).await;
    assert!(matches!(rb.recv().await, Some(Frame::Accepted { .. })));

    // Bob retire son portable : session fermée, ses autres appareils prévenus.
    assert_eq!(rl.recv().await, Some(Frame::DirectoryChanged));
    rb.send(&Frame::DeviceRevoke { device: bob_laptop.id.clone() }).await;
    loop {
        match rb.recv().await {
            Some(Frame::DeviceRevoked { device }) => {
                assert_eq!(device, bob_laptop.id);
                break;
            }
            Some(Frame::DirectoryChanged) => {}
            other => panic!("device_revoked attendu, reçu {other:?}"),
        }
    }
    within(async { while rl.recv().await.is_some() {} }).await;
}

#[tokio::test]
async fn services_are_created_by_the_admin_with_a_signed_qr() {
    let store = Store::in_memory().await.unwrap();
    let alice = add_device(&store, "alice-phone").await;
    store.admin_init(&blob(1), &blob(2)).await.unwrap();
    let alice_user = store.admin().await.unwrap().id;
    let server = TestServer::start(store, test_config()).await;
    let (mut ra, _) = Raw::login(&server, &alice).await;

    let identity = Identity::generate();
    let (user, id) = (UserId::generate(), DeviceId::generate());
    let sig = sign(&identity, &pair_register_message(&id, &identity.public_key(), "notifier-1"));
    let create = |sig| Frame::ServiceCreate {
        user: user.clone(),
        name: "Notifier".into(),
        device: id.clone(),
        public_key: identity.public_key(),
        device_name: "notifier-1".into(),
        sig,
        cert: blob(3),
    };
    ra.send(&create([0; 64])).await;
    assert_eq!(error_code(&mut ra).await, ErrorCode::PairingInvalid);
    ra.send(&create(sig)).await;
    assert_eq!(ra.recv().await, Some(Frame::PairRegistered { device_id: id.clone() }));
    ra.send(&Frame::RosterPublish { user: user.clone(), seq: 1, entry: blob(4) }).await;
    loop {
        match ra.recv().await {
            Some(Frame::RosterPublished { seq: 1, .. }) => break,
            Some(Frame::DirectoryChanged) => {}
            other => panic!("roster_published attendu, reçu {other:?}"),
        }
    }
    ra.send(&Frame::LinkSet { from: alice_user.clone(), to: user.clone(), allowed: true }).await;
    assert_eq!(error_code(&mut ra).await, ErrorCode::BadFrame, "un service ne reçoit rien");
    ra.send(&Frame::LinkSet { from: user.clone(), to: alice_user.clone(), allowed: true }).await;
    loop {
        match ra.recv().await {
            Some(Frame::LinkUpdated { .. }) => break,
            Some(Frame::DirectoryChanged) => {}
            other => panic!("link_updated attendu, reçu {other:?}"),
        }
    }

    let notifier = device(id.clone(), "notifier-1", identity);
    let (mut rn, _) = Raw::login(&server, &notifier).await;
    let env = Envelope::new_plain(id.clone(), vec![alice.id.clone()], &Content::text("bilan"));
    rn.send(&Frame::Send { env }).await;
    assert!(matches!(rn.recv().await, Some(Frame::Accepted { .. })));
    let env = Envelope::new_plain(id.clone(), vec![], &Content::text("à qui ?"));
    rn.send(&Frame::Send { env }).await;
    assert_eq!(error_code(&mut rn).await, ErrorCode::BadFrame);
    // Le service lit l'annuaire de qui il peut joindre.
    rn.send(&Frame::DirectoryGet { known: Known::new() }).await;
    let Some(Frame::Directory { users, .. }) = rn.recv().await else { panic!() };
    let alice_seen = users.iter().find(|u| u.id == alice_user).unwrap();
    assert!(alice_seen.devices.iter().any(|d| d.id == alice.id && d.online));
}
