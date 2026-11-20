//! Note 0045 : `ot otp-bridge` relaie un code lu sur le socket d'otp-relay
//! (ici un faux socket) à un membre du groupe, qui le reçoit par `listen`.
//! Étape 10 : depuis un appareil de service, vers des personnes
//! (`--to-user`).
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::process::Stdio;
use std::time::Duration;

use tempfile::TempDir;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::Command;

use common::{admin, device, group, invite, line_starting, ot, ot_fails, relay, service, spawn};

fn otp_line(id: &str, code: &str, expires_at: OffsetDateTime) -> String {
    let expires_at = expires_at.format(&Rfc3339).unwrap();
    format!(
        r#"{{"type":"otp","id":"{id}","code":"{code}","display":"G-{code}","confidence":1.0,"sender":{{"name":"Google","address":"noreply@google.com","domain":"google.com"}},"sender_verified":true,"auth":{{"dmarc":"pass"}},"to":"moi@example.org","subject":"code","received_at":"2026-11-02T14:03:11.000Z","detected_at":"2026-11-02T14:03:11.042Z","expires_at":"{expires_at}","source":{{"mailbox":"INBOX","uid":1}}}}"#
    )
}

#[tokio::test]
async fn bridge_relays_a_code_to_its_recipient() {
    let root = TempDir::new().unwrap();
    let (store, url) = relay(root.path()).await;
    let (home_bridge, bridge) = device(&store, root.path(), &url, "relais-otp").await;
    let (home_brave, brave) = device(&store, root.path(), &url, "brave").await;
    group(&[(&home_bridge, bridge), (&home_brave, brave)]);

    // Faux otp-relay : un code expiré (ignoré), puis un code valide.
    let socket = root.path().join("otp-relay.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let now = OffsetDateTime::now_utc();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let lines = [
            r#"{"type":"hello","protocol":1,"daemon":"otp-relay/test"}"#.to_owned(),
            r#"{"type":"status","state":"connected","detail":"","since":"x"}"#.to_owned(),
            otp_line("old", "111111", now - time::Duration::minutes(1)),
            otp_line("new", "482913", now + time::Duration::minutes(10)),
        ];
        for line in lines {
            stream.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        }
        // Le socket reste ouvert, comme celui du vrai daemon.
        std::future::pending::<()>().await;
    });

    let _bridge = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&home_bridge)
        .args(["otp-bridge", "--to", "brave", "--socket", socket.to_str().unwrap()])
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut listen = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&home_brave)
        .args(["listen", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(listen.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(20), lines.next_line())
        .await
        .expect("code reçu")
        .unwrap()
        .unwrap();
    let got: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(got["kind"], "otp", "{line}");
    assert_eq!(got["code"], "482913");
    assert_eq!(got["display"], "G-482913");
    assert_eq!(got["sender_domain"], "google.com");
    assert_eq!(got["sender_verified"], true);
    assert_eq!(got["from_name"], "relais-otp");

    // Le pont retient le code envoyé, dans un fichier privé.
    let sent = home_bridge.join("otp-sent.json");
    let text = std::fs::read_to_string(&sent).unwrap();
    assert!(text.contains("\"new\"") && !text.contains("\"old\""), "{text}");
    assert_eq!(std::fs::metadata(&sent).unwrap().permissions().mode() & 0o777, 0o600);
}

/// Faux otp-relay sur `socket` : le salut, puis ces codes ; reste ouvert.
fn fake_relay(socket: &std::path::Path, codes: Vec<(&'static str, &'static str)>) {
    let listener = UnixListener::bind(socket).unwrap();
    let now = OffsetDateTime::now_utc();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut lines =
            vec![r#"{"type":"hello","protocol":1,"daemon":"otp-relay/test"}"#.to_owned()];
        lines
            .extend(codes.iter().map(|(id, c)| otp_line(id, c, now + time::Duration::minutes(10))));
        for line in lines {
            stream.write_all(format!("{line}\n").as_bytes()).await.unwrap();
        }
        std::future::pending::<()>().await;
    });
}

#[tokio::test]
async fn bridge_relays_codes_to_people_and_skips_who_muted_it() {
    let root = TempDir::new().unwrap();
    let (store, url) = relay(root.path()).await;
    let alice = admin(&store, root.path(), &url).await;
    let bob = invite(&alice, root.path(), "Bob", &["Alice"], "bob-cli").await;
    let carol = invite(&alice, root.path(), "Carol", &["Alice"], "carol-cli").await;
    let notifier = service(&alice, root.path(), &url, "Notifier", "notifier-1").await;
    ot(&alice, &["admin", "link", "add", "Notifier", "Alice"]).await;
    ot(&alice, &["admin", "link", "add", "Notifier", "Bob"]).await;
    ot(&alice, &["admin", "link", "add", "Notifier", "Carol"]).await;
    ot(&carol, &["mute", "Notifier", "on"]).await;

    // --to et --to-user ne se combinent pas.
    let err = ot_fails(&notifier, &["otp-bridge", "--to", "x", "--to-user", "Bob"]).await;
    assert!(err.contains("ne se combinent pas"), "{err}");

    let socket = root.path().join("otp-relay.sock");
    fake_relay(&socket, vec![("c1", "482913")]);
    let mut bridge = Command::new(env!("CARGO_BIN_EXE_ot"))
        .arg("--home")
        .arg(&notifier)
        .args(["otp-bridge", "--socket", socket.to_str().unwrap()])
        .env("OT_OTP_TO_USER", "Alice,Bob,Carol")
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    // Un seul message pour Alice et Bob.
    for home in [&alice, &bob] {
        let (_listen, mut lines) = spawn(home, &["listen", "--json"]);
        let line = tokio::time::timeout(Duration::from_secs(20), line_starting(&mut lines, "{"))
            .await
            .expect("code reçu");
        let got: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!((&got["kind"], &got["code"]), (&"otp".into(), &"482913".into()), "{line}");
        assert_eq!(got["from_user_name"], "Notifier");
        assert_eq!((&got["from_service"], &got["auto_fill"]), (&true.into(), &true.into()));
        assert_eq!(got["from_name"], "Notifier/notifier-1");
    }

    // Carol a coupé le service : sautée, signalée par son identifiant ; aucun
    // code dans le journal du pont.
    let mut err = BufReader::new(bridge.stderr.take().unwrap()).lines();
    let mut log = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(l) = err.next_line().await.unwrap() {
            let done = l.contains("envoyé");
            log.push(l);
            if done {
                break;
            }
        }
    })
    .await
    .expect("envoi journalisé");
    let log = log.join("\n");
    assert!(log.contains("a coupé ce service : sautée"), "{log}");
    assert!(log.contains("usr_") && !log.contains("482913"), "{log}");
    let _ = bridge.kill().await;
}
