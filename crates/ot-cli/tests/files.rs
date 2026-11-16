//! Étape 5, T4 : envoi et réception de fichiers par la CLI, contre un relais
//! en processus. Deux appareils réels (`ot init`), enregistrés sur le relais
//! et mis dans un même groupe.

mod common;

use std::time::Duration;

use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use common::{device, group, ot, relay};

#[tokio::test]
async fn file_between_two_cli_devices() {
    let root = TempDir::new().unwrap();
    let (store, url) = relay(root.path()).await;

    let (home_a, a) = device(&store, root.path(), &url, "mac").await;
    let (home_b, b) = device(&store, root.path(), &url, "windows").await;
    group(&[(&home_a, a), (&home_b, b)]);

    // 3 Mio de contenu, envoyé de « mac » à « windows ».
    let data: Vec<u8> =
        (0..3u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let src = root.path().join("rapport final.pdf");
    std::fs::write(&src, &data).unwrap();
    ot(&home_a, &["send", "--file", src.to_str().unwrap(), "--to", "windows"]).await;

    // `listen` annonce le fichier et la commande pour le télécharger.
    let mut listen = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&home_b)
        .arg("listen")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(listen.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(20), lines.next_line())
        .await
        .expect("annonce du fichier")
        .unwrap()
        .unwrap();
    assert!(line.contains("rapport final.pdf") && line.contains("3,0 Mo"), "{line}");
    let blob = line.rsplit(' ').next().unwrap().to_owned();
    assert!(blob.starts_with("blob_"), "{line}");
    let _ = listen.kill().await;

    let listed = String::from_utf8(ot(&home_b, &["files"]).await.stdout).unwrap();
    assert!(listed.starts_with(&blob) && listed.contains("rapport final.pdf"), "{listed}");
    let out_dir = root.path().join("reçus");
    ot(&home_b, &["download", &blob, "--to", out_dir.to_str().unwrap()]).await;
    assert!(std::fs::read(out_dir.join("rapport final.pdf")).unwrap() == data);
    // Plus rien en attente : l'offre est oubliée après le téléchargement.
    let listed = String::from_utf8(ot(&home_b, &["files"]).await.stdout).unwrap();
    assert!(listed.trim().is_empty(), "{listed}");

    // `listen --download-dir` télécharge tout seul, sans écraser le premier.
    let mut listen = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&home_b)
        .args(["listen", "--download-dir", out_dir.to_str().unwrap()])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    ot(&home_a, &["send", "--file", src.to_str().unwrap()]).await;
    let second = out_dir.join("rapport final (2).pdf");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !second.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("fichier téléchargé par listen");
    assert!(std::fs::read(&second).unwrap() == data);
    let _ = listen.kill().await;
}
