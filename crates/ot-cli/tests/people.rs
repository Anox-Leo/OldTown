//! Étape 10, T9 : parcours complet par la CLI contre un relais en processus.
//! Alice (admin) invite Bob ; ils s'écrivent, l'admin règle, renomme et
//! révoque. Noms neutres.

mod common;

use std::time::Duration;

use tempfile::TempDir;

use common::{admin, invite, line_starting, ot, ot_fails, ot_stdout, relay, spawn};

/// Premier message reçu par `ot listen --json` sur `home`.
async fn received(home: &std::path::Path) -> serde_json::Value {
    let (_listen, mut lines) = spawn(home, &["listen", "--json"]);
    let line = tokio::time::timeout(Duration::from_secs(20), line_starting(&mut lines, "{"))
        .await
        .expect("message reçu");
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn people_are_invited_talk_and_are_administered() {
    let root = TempDir::new().unwrap();
    let (store, url) = relay(root.path()).await;
    let alice = admin(&store, root.path(), &url).await;
    // `admin init` est idempotent sur l'appareil qui l'a fait : refusé, sans
    // rien casser.
    let again = ot_fails(&alice, &["admin", "init"]).await;
    assert!(again.contains("déjà publié"), "{again}");

    let bob = invite(&alice, root.path(), "Bob", &["Alice"], "bob-cli").await;

    // Chacun voit l'autre dans l'annuaire vérifié.
    let seen = ot_stdout(&bob, &["users"]).await;
    assert!(seen.contains("Alice (admin)") && seen.contains("Bob  "), "{seen}");
    assert!(seen.contains("(moi)") && seen.contains("bob-cli"), "{seen}");
    let seen = ot_stdout(&alice, &["users"]).await;
    assert!(seen.contains("Bob  ") && seen.contains("par défaut"), "{seen}");
    let mine = ot_stdout(&bob, &["devices"]).await;
    assert!(mine.contains("Personne : Bob (membre"), "{mine}");

    // Alice écrit à Bob, Bob répond à un appareil précis d'Alice.
    ot(&alice, &["send", "--to-user", "Bob", "bonjour", "Bob"]).await;
    let got = received(&bob).await;
    assert_eq!(got["body"], "bonjour Bob", "{got}");
    assert_eq!(got["from_user_name"], "Alice");
    assert_eq!(got["from_name"], "Alice/alice-laptop");
    ot(&bob, &["send", "--to-user", "Alice", "--device", "alice-laptop", "merci"]).await;
    let got = received(&alice).await;
    assert_eq!((&got["body"], &got["from_user_name"]), (&"merci".into(), &"Bob".into()), "{got}");

    // Un fichier à une personne : annoncé chez elle, téléchargeable.
    let path = root.path().join("note.txt");
    std::fs::write(&path, b"contenu du fichier").unwrap();
    ot(&alice, &["send", "--file", path.to_str().unwrap(), "--to-user", "Bob"]).await;
    let got = received(&bob).await;
    assert_eq!((&got["kind"], &got["name"]), (&"file".into(), &"note.txt".into()), "{got}");
    assert_eq!(got["from_user_name"], "Alice");
    let dir = root.path().join("reçus");
    std::fs::create_dir(&dir).unwrap();
    let blob = got["blob_id"].as_str().unwrap();
    ot(&bob, &["download", blob, "--to", dir.to_str().unwrap()]).await;
    assert_eq!(std::fs::read(dir.join("note.txt")).unwrap(), b"contenu du fichier");

    // Noms inconnus ou incohérents : refusés avec les choix.
    let err = ot_fails(&alice, &["send", "--to-user", "Carol", "x"]).await;
    assert!(err.contains("personne introuvable") && err.contains("Bob"), "{err}");
    let err = ot_fails(&alice, &["send", "--to-user", "Bob", "--device", "nope", "x"]).await;
    assert!(err.contains("appareil introuvable") && err.contains("bob-cli"), "{err}");
    let err = ot_fails(&alice, &["send", "--to", "alice-laptop", "--to-user", "Bob", "x"]).await;
    assert!(err.contains("--to-user"), "{err}");

    // L'admin voit tout, règle le profil d'un appareil de Bob, le renomme.
    let all = ot_stdout(&alice, &["admin", "users"]).await;
    assert!(all.contains("Bob (membre)") && all.contains("→ Alice"), "{all}");
    let err = ot_fails(&alice, &["device", "default", "bob-cli", "off"]).await;
    assert!(err.contains("au moins un appareil par défaut"), "{err}");
    ot(&alice, &["device", "reachable", "bob-cli", "off"]).await;
    let seen = ot_stdout(&bob, &["users"]).await;
    assert!(seen.lines().any(|l| l.contains("bob-cli") && l.contains("injoignable")), "{seen}");
    let seen = ot_stdout(&alice, &["users"]).await;
    assert!(!seen.contains("bob-cli"), "un appareil injoignable est caché aux autres : {seen}");
    ot(&alice, &["device", "reachable", "bob-cli", "on"]).await;
    ot(&bob, &["device", "kind", "bob-cli", "laptop"]).await;
    let seen = ot_stdout(&alice, &["users"]).await;
    let line = seen.lines().find(|l| l.contains("bob-cli")).unwrap();
    assert!(line.contains("portable") && line.contains("par défaut"), "{seen}");
    let err = ot_fails(&bob, &["device", "kind", "bob-cli", "toaster"]).await;
    assert!(err.contains("nature inconnue"), "{err}");
    ot(&alice, &["admin", "rename", "Bob", "Robert"]).await;
    let seen = ot_stdout(&bob, &["users"]).await;
    assert!(seen.contains("Robert  ") && !seen.contains("Bob  "), "{seen}");

    // Lien retiré, puis personne révoquée : elle disparaît de l'annuaire.
    ot(&alice, &["admin", "link", "remove", "Robert", "Alice"]).await;
    let err = ot_fails(&bob, &["send", "--to-user", "Alice", "x"]).await;
    assert!(err.contains("hors réseau") || err.contains("aucun"), "{err}");
    ot(&alice, &["admin", "revoke", "Robert"]).await;
    let seen = ot_stdout(&alice, &["users"]).await;
    assert!(!seen.contains("Robert"), "{seen}");
}
