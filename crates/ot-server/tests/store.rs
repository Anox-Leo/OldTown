use ot_proto::DeviceId;
use ot_server::store::{NewMessage, PurgeStats, Store, StoreError};

const DAY: i64 = 24 * 3600 * 1000;

async fn setup() -> (Store, DeviceId, DeviceId, DeviceId) {
    let store = Store::in_memory().await.unwrap();
    let a = store.add_device("mac", &[1; 32], 0).await.unwrap().id;
    let b = store.add_device("windows", &[2; 32], 0).await.unwrap().id;
    let c = store.add_device("android", &[3; 32], 0).await.unwrap().id;
    (store, a, b, c)
}

fn msg<'a>(id: &'a str, from: &'a DeviceId, to: &'a [DeviceId]) -> NewMessage<'a> {
    NewMessage { msg_id: id, sender: from, to, envelope: b"{}", expires_at: 7 * DAY }
}

#[tokio::test]
async fn devices_add_list_revoke() {
    let (store, a, _, _) = setup().await;
    assert!(matches!(store.add_device("mac", &[9; 32], 1).await, Err(StoreError::NameTaken(_))));
    assert!(matches!(store.add_device("autre", &[1; 32], 1).await, Err(StoreError::KeyTaken)));

    let revoked = store.revoke_device("mac", 5).await.unwrap();
    assert_eq!(revoked.id, a);
    assert_eq!(revoked.revoked_at, Some(5));
    assert!(matches!(store.revoke_device("mac", 6).await, Err(StoreError::UnknownDevice(_))));

    // Le nom redevient libre, la ligne révoquée reste pour l'audit.
    store.add_device("mac", &[4; 32], 7).await.unwrap();
    assert_eq!(store.devices().await.unwrap().len(), 4);
}

#[tokio::test]
async fn insert_is_idempotent() {
    let (store, a, b, _) = setup().await;
    let first = store.insert_message(msg("m1", &a, &[]), 1).await.unwrap();
    assert!(!first.duplicate);
    assert_eq!(first.recipients.len(), 2);

    let again = store.insert_message(msg("m1", &a, &[]), 2).await.unwrap();
    assert!(again.duplicate);
    assert_eq!(again.seq, first.seq);
    assert_eq!(store.pending(&b, 0, 100, 3).await.unwrap().len(), 1);

    assert!(matches!(
        store.insert_message(msg("m1", &b, &[]), 3).await,
        Err(StoreError::MsgIdConflict)
    ));
}

#[tokio::test]
async fn recipients_resolution() {
    let (store, a, b, c) = setup().await;
    let to = [b.clone(), a.clone(), b.clone()];
    let ins = store.insert_message(msg("m1", &a, &to), 1).await.unwrap();
    assert_eq!(ins.recipients, vec![b.clone()]);
    assert!(store.pending(&c, 0, 100, 2).await.unwrap().is_empty());

    store.revoke_device("android", 2).await.unwrap();
    assert!(matches!(
        store.insert_message(msg("m2", &a, std::slice::from_ref(&c)), 3).await,
        Err(StoreError::UnknownRecipient(id)) if id == c
    ));
    let all = store.insert_message(msg("m3", &a, &[]), 4).await.unwrap();
    assert_eq!(all.recipients, vec![b]);
}

#[tokio::test]
async fn pending_order_and_ack() {
    let (store, a, b, c) = setup().await;
    let mut seqs = Vec::new();
    for i in 0..5 {
        seqs.push(store.insert_message(msg(&format!("m{i}"), &a, &[]), i).await.unwrap().seq);
    }
    let pending = store.pending(&b, 0, 100, 10).await.unwrap();
    assert_eq!(pending.iter().map(|p| p.seq).collect::<Vec<_>>(), seqs);
    assert_eq!(store.pending(&b, seqs[2], 100, 10).await.unwrap().len(), 2);
    assert_eq!(store.pending(&b, 0, 2, 10).await.unwrap().len(), 2);

    store.mark_delivered(seqs[0], &b, 11).await.unwrap();
    assert!(store.ack(seqs[0], &b, 12).await.unwrap());
    assert!(!store.ack(seqs[0], &b, 13).await.unwrap(), "double ack");
    assert!(!store.ack(999, &b, 13).await.unwrap(), "seq inconnu");
    assert_eq!(store.pending_count(&b, 14).await.unwrap(), 4);
    assert_eq!(store.pending_count(&c, 14).await.unwrap(), 5);
    assert_eq!(store.total_pending(14).await.unwrap(), 9);
}

#[tokio::test]
async fn purge_acked_and_expired() {
    let (store, a, b, c) = setup().await;
    let s1 = store.insert_message(msg("m1", &a, &[]), 1).await.unwrap().seq;
    let s2 = store.insert_message(msg("m2", &a, &[]), 2).await.unwrap().seq;
    store.insert_message(NewMessage { expires_at: 100, ..msg("m3", &a, &[]) }, 3).await.unwrap();

    // m1 acquitté par un seul des deux destinataires : il reste.
    store.ack(s1, &b, 10).await.unwrap();
    assert_eq!(store.purge(50).await.unwrap(), PurgeStats::default());

    store.ack(s1, &c, 60).await.unwrap();
    assert_eq!(store.purge(70).await.unwrap(), PurgeStats { acked: 1, expired: 0 });

    // m3 expire ; un message expiré n'est plus proposé même avant la purge.
    assert_eq!(store.pending(&b, 0, 100, 100).await.unwrap().len(), 1);
    assert_eq!(store.purge(100).await.unwrap(), PurgeStats { acked: 0, expired: 1 });
    assert_eq!(store.pending(&b, 0, 100, 101).await.unwrap()[0].seq, s2);

    // Un seq n'est jamais réutilisé après purge.
    store.ack(s2, &b, 102).await.unwrap();
    store.ack(s2, &c, 102).await.unwrap();
    store.purge(103).await.unwrap();
    let s4 = store.insert_message(msg("m4", &a, &[]), 104).await.unwrap().seq;
    assert!(s4 > s2);
}

#[tokio::test]
async fn revoke_drops_pending_deliveries() {
    let (store, a, b, _) = setup().await;
    store.insert_message(msg("m1", &a, &[]), 1).await.unwrap();
    store.revoke_device(b.as_str(), 2).await.unwrap();
    assert_eq!(store.pending_count(&b, 3).await.unwrap(), 0);
}

#[tokio::test]
async fn data_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ot.db");
    let store = Store::open(&path).await.unwrap();
    let a = store.add_device("mac", &[1; 32], 0).await.unwrap().id;
    let b = store.add_device("windows", &[2; 32], 0).await.unwrap().id;
    store.insert_message(msg("m1", &a, &[]), 1).await.unwrap();
    store.close().await;

    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.pending(&b, 0, 10, 2).await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_do_not_fail() {
    // Insertions et purges simultanées sur une base fichier (WAL, plusieurs
    // connexions) : aucune ne doit échouer en SQLITE_BUSY.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("ot.db")).await.unwrap();
    let a = store.add_device("mac", &[1; 32], 0).await.unwrap().id;
    store.add_device("windows", &[2; 32], 0).await.unwrap();

    let mut tasks = Vec::new();
    for t in 0..4 {
        let (store, a) = (store.clone(), a.clone());
        tasks.push(tokio::spawn(async move {
            for i in 0..50 {
                let id = format!("m{t}-{i}");
                store.insert_message(msg(&id, &a, &[]), i).await.unwrap();
            }
        }));
    }
    let purger = store.clone();
    tasks.push(tokio::spawn(async move {
        for i in 0..100 {
            purger.purge(i).await.unwrap();
        }
    }));
    for t in tasks {
        t.await.unwrap();
    }
}
