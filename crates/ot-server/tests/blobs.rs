//! Étape 5, T2 : API HTTP des fichiers (note 0026). Dépôt et retrait par
//! morceaux, authentification signée sans session, droits, limites, purge.

mod common;

use std::path::Path;
use std::time::Duration;

use common::*;
use ot_proto::http::{BlobCreated, BlobStatus, CreateBlob, HttpErrorCode};
use ot_proto::{BlobId, Frame};
use ot_server::Config;
use ot_server::store::Store;
use tempfile::TempDir;

/// Géométrie de test : 80 octets chiffrés en morceaux de 32 (32, 32, 16).
const SIZE: u64 = 80;
const CHUNK: u32 = 32;

fn chunk(i: u32) -> Vec<u8> {
    let len = if i == 2 { 16 } else { 32 };
    vec![i as u8 + 1; len]
}

struct Setup {
    server: TestServer,
    dir: TempDir,
    mac: TestDevice,
    phone: TestDevice,
    windows: TestDevice,
}

async fn setup_with(config: impl FnOnce(&Path) -> Config) -> Setup {
    let dir = TempDir::new().unwrap();
    let store = Store::in_memory().await.unwrap();
    let mac = add_device(&store, "mac").await;
    let phone = add_device(&store, "telephone").await;
    let windows = add_device(&store, "windows").await;
    let server = TestServer::start(store, config(dir.path())).await;
    Setup { server, dir, mac, phone, windows }
}

async fn setup() -> Setup {
    setup_with(|dir| Config { blob_dir: dir.to_owned(), ..test_config() }).await
}

fn create_body(to: &[&TestDevice]) -> Vec<u8> {
    let req = CreateBlob {
        size: SIZE,
        chunk_size: CHUNK,
        chunks: 3,
        to: to.iter().map(|d| d.id.clone()).collect(),
    };
    serde_json::to_vec(&req).unwrap()
}

async fn create(s: &Setup, from: &TestDevice, to: &[&TestDevice]) -> BlobId {
    let res = signed(&s.server, from, "POST", "/v1/blobs", &create_body(to)).await;
    assert_eq!(res.status, 201, "{}", String::from_utf8_lossy(&res.body));
    res.json::<BlobCreated>().blob_id
}

async fn put(s: &Setup, dev: &TestDevice, id: &BlobId, i: u32, data: &[u8]) -> HttpResponse {
    signed(&s.server, dev, "PUT", &format!("/v1/blobs/{id}/chunks/{i}"), data).await
}

async fn upload_all(s: &Setup, id: &BlobId) {
    for i in 0..3 {
        assert_eq!(put(s, &s.mac, id, i, &chunk(i)).await.status, 204);
    }
}

async fn status(s: &Setup, dev: &TestDevice, id: &BlobId) -> HttpResponse {
    signed(&s.server, dev, "GET", &format!("/v1/blobs/{id}"), b"").await
}

fn chunk_files(s: &Setup, id: &BlobId) -> usize {
    std::fs::read_dir(s.dir.path().join(id.as_str())).map_or(0, |d| d.count())
}

#[tokio::test]
async fn upload_then_download_then_done_deletes_everything() {
    let s = setup().await;
    let id = create(&s, &s.mac, &[&s.phone]).await;

    // Reprise : l'état dit quels morceaux manquent.
    assert_eq!(put(&s, &s.mac, &id, 1, &chunk(1)).await.status, 204);
    let st: BlobStatus = status(&s, &s.mac, &id).await.json();
    assert_eq!((st.missing, st.complete), (vec![0, 2], false));
    upload_all(&s, &id).await;
    let st: BlobStatus = status(&s, &s.phone, &id).await.json();
    assert!(st.missing.is_empty() && st.complete);
    assert_eq!(chunk_files(&s, &id), 3);

    for i in 0..3 {
        let res =
            signed(&s.server, &s.phone, "GET", &format!("/v1/blobs/{id}/chunks/{i}"), b"").await;
        assert_eq!((res.status, res.body), (200, chunk(i)));
    }
    let res = signed(&s.server, &s.phone, "POST", &format!("/v1/blobs/{id}/done"), b"").await;
    assert_eq!(res.status, 204);

    // Le seul destinataire a fini : plus rien en base ni sur le disque.
    assert_eq!(status(&s, &s.mac, &id).await.error(), HttpErrorCode::NotFound);
    assert_eq!(chunk_files(&s, &id), 0);
    s.server.stop().await;
}

#[tokio::test]
async fn deleted_only_when_every_recipient_is_done() {
    let s = setup().await;
    // `to` vide : tous les autres appareils actifs.
    let id = create(&s, &s.mac, &[]).await;
    upload_all(&s, &id).await;
    let done = format!("/v1/blobs/{id}/done");
    assert_eq!(signed(&s.server, &s.phone, "POST", &done, b"").await.status, 204);
    assert_eq!(status(&s, &s.windows, &id).await.status, 200);
    assert_eq!(signed(&s.server, &s.windows, "POST", &done, b"").await.status, 204);
    assert_eq!(chunk_files(&s, &id), 0);
    s.server.stop().await;
}

#[tokio::test]
async fn reupload_is_idempotent() {
    let s = setup().await;
    let id = create(&s, &s.mac, &[&s.phone]).await;
    assert_eq!(put(&s, &s.mac, &id, 0, &chunk(0)).await.status, 204);
    assert_eq!(put(&s, &s.mac, &id, 0, &chunk(0)).await.status, 204);
    let st: BlobStatus = status(&s, &s.mac, &id).await.json();
    assert_eq!(st.missing, vec![1, 2]);
    s.server.stop().await;
}

#[tokio::test]
async fn rights_are_enforced() {
    let s = setup().await;
    let id = create(&s, &s.mac, &[&s.phone]).await;
    // Seul l'expéditeur dépose et annule.
    assert_eq!(put(&s, &s.phone, &id, 0, &chunk(0)).await.error(), HttpErrorCode::Forbidden);
    let path = format!("/v1/blobs/{id}");
    let res = signed(&s.server, &s.phone, "DELETE", &path, b"").await;
    assert_eq!(res.error(), HttpErrorCode::Forbidden);

    // Retrait avant la fin du dépôt.
    let get0 = format!("/v1/blobs/{id}/chunks/0");
    let res = signed(&s.server, &s.phone, "GET", &get0, b"").await;
    assert_eq!((res.status, res.error()), (409, HttpErrorCode::Incomplete));

    upload_all(&s, &id).await;
    // Ni l'expéditeur ni un appareil hors des destinataires ne retirent.
    for dev in [&s.mac, &s.windows] {
        let res = signed(&s.server, dev, "GET", &get0, b"").await;
        assert_eq!(res.error(), HttpErrorCode::Forbidden);
    }
    assert_eq!(status(&s, &s.windows, &id).await.error(), HttpErrorCode::Forbidden);
    s.server.stop().await;
}

#[tokio::test]
async fn malformed_requests_are_refused() {
    let s = setup().await;
    let bad = |size: u64, chunk_size: u32, chunks: u32| {
        serde_json::to_vec(&CreateBlob { size, chunk_size, chunks, to: vec![] }).unwrap()
    };
    for body in [bad(80, 32, 4), bad(80, 0, 1), bad(80, (4 << 20) + 17, 1), b"{".to_vec()] {
        let res = signed(&s.server, &s.mac, "POST", "/v1/blobs", &body).await;
        assert_eq!(res.error(), HttpErrorCode::BadRequest);
    }
    // Destinataire inconnu.
    let other = add_device(&Store::in_memory().await.unwrap(), "ailleurs").await;
    let res = signed(&s.server, &s.mac, "POST", "/v1/blobs", &create_body(&[&other])).await;
    assert_eq!(res.error(), HttpErrorCode::BadRequest);

    let id = create(&s, &s.mac, &[&s.phone]).await;
    // Mauvaise longueur, index hors du fichier.
    assert_eq!(put(&s, &s.mac, &id, 0, &[0; 31]).await.error(), HttpErrorCode::BadRequest);
    assert_eq!(put(&s, &s.mac, &id, 3, &[0; 16]).await.error(), HttpErrorCode::BadRequest);
    // Blob inconnu, identifiant invalide.
    let unknown = BlobId::generate();
    assert_eq!(status(&s, &s.mac, &unknown).await.error(), HttpErrorCode::NotFound);
    let res = signed(&s.server, &s.mac, "GET", "/v1/blobs/blob_..", b"").await;
    assert_eq!(res.error(), HttpErrorCode::NotFound);
    s.server.stop().await;
}

#[tokio::test]
async fn no_recipient_is_refused() {
    let dir = TempDir::new().unwrap();
    let store = Store::in_memory().await.unwrap();
    let alone = add_device(&store, "seul").await;
    let config = Config { blob_dir: dir.path().to_owned(), ..test_config() };
    let server = TestServer::start(store, config).await;
    let res = signed(&server, &alone, "POST", "/v1/blobs", &create_body(&[])).await;
    assert_eq!(res.error(), HttpErrorCode::BadRequest);
    server.stop().await;
}

#[tokio::test]
async fn authentication_is_checked() {
    let s = setup().await;
    let body = create_body(&[&s.phone]);
    let now = ot_server::now_ms();
    let post = |headers: Vec<(&'static str, String)>| {
        let (addr, body) = (s.server.addr, body.clone());
        async move { http(addr, "POST", "/v1/blobs", &headers, &body).await }
    };

    // Sans en-têtes.
    assert_eq!(post(vec![]).await.status, 401);
    // Signature d'un autre corps.
    let headers = signed_headers(&s.mac, "POST", "/v1/blobs", b"autre", now, [1; 16]);
    assert_eq!(post(headers).await.error(), HttpErrorCode::Unauthorized);
    // Signature d'un autre chemin.
    let headers = signed_headers(&s.mac, "POST", "/v1/blobs/x", &body, now, [2; 16]);
    assert_eq!(post(headers).await.error(), HttpErrorCode::Unauthorized);
    // Heure hors de la fenêtre de ±60 s, dans les deux sens.
    for t in [now - 61_000, now + 61_000] {
        let res = post(signed_headers(&s.mac, "POST", "/v1/blobs", &body, t, [3; 16])).await;
        assert_eq!(res.error(), HttpErrorCode::Unauthorized);
        assert!(String::from_utf8_lossy(&res.body).contains("horloge"));
    }
    // Nonce rejoué.
    let headers = signed_headers(&s.mac, "POST", "/v1/blobs", &body, now, [4; 16]);
    assert_eq!(post(headers.clone()).await.status, 201);
    assert_eq!(post(headers).await.error(), HttpErrorCode::Unauthorized);
    // Appareil révoqué.
    s.server.relay.store().revoke_device("mac", now).await.unwrap();
    let headers = signed_headers(&s.mac, "POST", "/v1/blobs", &body, now, [5; 16]);
    assert_eq!(post(headers).await.error(), HttpErrorCode::Unauthorized);
    s.server.stop().await;
}

#[tokio::test]
async fn size_limit_and_quota() {
    let s = setup_with(|dir| Config {
        blob_dir: dir.to_owned(),
        max_file_size: 64,
        blob_quota: 200,
        ..test_config()
    })
    .await;
    // 3 morceaux de 32 = 96 octets chiffrés, 48 en clair : sous la limite ;
    // 5 morceaux = 160 octets chiffrés, 80 en clair : au-dessus.
    let ok = CreateBlob { size: 96, chunk_size: 32, chunks: 3, to: vec![] };
    let big = CreateBlob { size: 160, chunk_size: 32, chunks: 5, to: vec![] };
    let body = |r: &CreateBlob| serde_json::to_vec(r).unwrap();
    let res = signed(&s.server, &s.mac, "POST", "/v1/blobs", &body(&big)).await;
    assert_eq!((res.status, res.error()), (413, HttpErrorCode::TooLarge));

    for _ in 0..2 {
        assert_eq!(signed(&s.server, &s.mac, "POST", "/v1/blobs", &body(&ok)).await.status, 201);
    }
    // 2 × 96 = 192 octets réservés ; un troisième dépasserait 200.
    let res = signed(&s.server, &s.mac, "POST", "/v1/blobs", &body(&ok)).await;
    assert_eq!((res.status, res.error()), (507, HttpErrorCode::QuotaExceeded));
    let (_, metrics) = http_get(s.server.addr, "/metrics").await;
    assert!(metrics.contains("ot_blob_bytes 192"), "{metrics}");
    s.server.stop().await;
}

#[tokio::test]
async fn cancel_deletes_chunks() {
    let s = setup().await;
    let cancelled = create(&s, &s.mac, &[&s.phone]).await;
    assert_eq!(put(&s, &s.mac, &cancelled, 0, &chunk(0)).await.status, 204);
    let res = signed(&s.server, &s.mac, "DELETE", &format!("/v1/blobs/{cancelled}"), b"").await;
    assert_eq!(res.status, 204);
    assert_eq!(chunk_files(&s, &cancelled), 0);
    assert_eq!(status(&s, &s.mac, &cancelled).await.error(), HttpErrorCode::NotFound);
    s.server.stop().await;
}

#[tokio::test]
async fn expiry_deletes_chunks() {
    // Durée de vie assez longue pour que le dépôt finisse avant, même sur un
    // exécuteur de CI lent (300 ms ne suffisaient pas sous macOS).
    let ttl = Duration::from_secs(1);
    let s =
        setup_with(|dir| Config { blob_dir: dir.to_owned(), blob_ttl: ttl, ..test_config() }).await;
    let expiring = create(&s, &s.mac, &[&s.phone]).await;
    upload_all(&s, &expiring).await;
    tokio::time::sleep(ttl + Duration::from_millis(100)).await;
    // Expiré : introuvable avant même la purge, puis effacé du disque.
    assert_eq!(status(&s, &s.phone, &expiring).await.error(), HttpErrorCode::NotFound);
    s.server.relay.purge_once().await.unwrap();
    assert_eq!(chunk_files(&s, &expiring), 0);
    s.server.stop().await;
}

#[tokio::test]
async fn leftovers_of_a_crash_are_cleaned_at_startup() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("ot.db");
    let blobs = dir.path().join("blobs");
    let config = || Config { blob_dir: blobs.clone(), ..test_config() };

    let store = open_store(&db).await;
    let mac = add_device(&store, "mac").await;
    let phone = add_device(&store, "telephone").await;
    let server = TestServer::start(store, config()).await;
    let res = signed(&server, &mac, "POST", "/v1/blobs", &create_body(&[&phone])).await;
    let id = res.json::<BlobCreated>().blob_id;
    let path = format!("/v1/blobs/{id}/chunks/0");
    assert_eq!(signed(&server, &mac, "PUT", &path, &chunk(0)).await.status, 204);
    server.stop().await;

    // Arrêt brutal simulé : un morceau à moitié écrit, et le dossier d'un
    // blob supprimé de la base avant ses morceaux.
    std::fs::write(blobs.join(id.as_str()).join(".1.42.tmp"), b"moitie").unwrap();
    let orphan = blobs.join(BlobId::generate().as_str());
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("0"), b"x").unwrap();
    // Un dossier qui n'est pas un blob n'est jamais touché.
    std::fs::create_dir_all(blobs.join("autre")).unwrap();

    let server = TestServer::start(open_store(&db).await, config()).await;
    // Le nettoyage précède le service : une réponse prouve qu'il est fait.
    assert_eq!(http_get(server.addr, "/healthz").await.0, 200);
    let left: Vec<_> = std::fs::read_dir(blobs.join(id.as_str()))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, vec!["0".to_owned()]);
    assert!(!orphan.exists());
    assert!(blobs.join("autre").exists());
    server.stop().await;
}

#[tokio::test]
async fn http_requests_do_not_touch_the_websocket_session() {
    let s = setup().await;
    let (mut raw, _) = Raw::login(&s.server, &s.mac).await;
    let id = create(&s, &s.mac, &[&s.phone]).await;
    upload_all(&s, &id).await;
    // La session du même appareil est toujours là et répond.
    raw.send(&Frame::Ping).await;
    assert_eq!(raw.recv().await, Some(Frame::Pong));
    raw.close().await;
    s.server.stop().await;
}

#[tokio::test]
async fn chunks_on_disk_are_private() {
    let s = setup().await;
    let id = create(&s, &s.mac, &[&s.phone]).await;
    upload_all(&s, &id).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = s.dir.path().join(id.as_str());
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let file = std::fs::metadata(dir.join("0")).unwrap();
        assert_eq!(file.permissions().mode() & 0o777, 0o600);
    }
    s.server.stop().await;
}
