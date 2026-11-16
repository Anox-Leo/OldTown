//! Note 0045 : `ot otp-bridge` relaie un code lu sur le socket d'otp-relay
//! (ici un faux socket) à un membre du groupe, qui le reçoit par `listen`.
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

use common::{device, group, relay};

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
