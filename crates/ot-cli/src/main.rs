mod files;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use ot_core::{Client, ClientConfig, Content, ContentKind, Dedup, DeviceInfo, Event, Events};
use time::UtcOffset;
use time::macros::format_description;

use crate::files::{Config, Home};

/// Délai maximal d'une commande ponctuelle (send, devices).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);

/// OldTown : envoyer un texte d'un terminal à un autre.
#[derive(Parser)]
#[command(name = "ot", version)]
struct Cli {
    /// Dossier de configuration (par défaut celui de l'OS).
    #[arg(long, env = "OT_HOME", global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Crée la clé de l'appareil et la configuration, ou les met à jour.
    Init {
        /// URL du relais, ex. ws://relais:7878.
        #[arg(long)]
        server: Option<String>,
        /// Nom de cet appareil.
        #[arg(long)]
        name: Option<String>,
        /// Identifiant renvoyé par `ot-server device add`.
        #[arg(long)]
        device_id: Option<String>,
        /// Remplace la clé existante (l'appareil devra être réenregistré).
        #[arg(long)]
        force: bool,
    },
    /// Envoie un texte ou un lien (lu sur l'entrée standard si absent).
    Send {
        text: Vec<String>,
        /// Destinataire (nom d'appareil), répétable ; par défaut tous les autres.
        #[arg(long = "to")]
        to: Vec<String>,
    },
    /// Affiche les messages reçus au fil de l'eau.
    Listen {
        /// Une ligne JSON par message.
        #[arg(long)]
        json: bool,
    },
    /// Liste les appareils enregistrés sur le relais.
    Devices,
}

fn main() -> Result<()> {
    // L'heure locale doit être lue avant de démarrer des threads.
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let cli = Cli::parse();
    let home = Home::locate(cli.home)?;
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async move {
        match cli.command {
            Command::Init { server, name, device_id, force } => {
                init(&home, server, name, device_id, force)
            }
            Command::Send { text, to } => send(&home, text, to).await,
            Command::Listen { json } => listen(&home, json, offset).await,
            Command::Devices => devices(&home).await,
        }
    })
}

fn init(
    home: &Home,
    server: Option<String>,
    name: Option<String>,
    device_id: Option<String>,
    force: bool,
) -> Result<()> {
    let existing = home.load_config()?;
    let server_url = server
        .or_else(|| existing.as_ref().map(|c| c.server_url.clone()))
        .context("--server est requis au premier lancement")?;
    let device_name = name
        .or_else(|| existing.as_ref().map(|c| c.device_name.clone()))
        .context("--name est requis au premier lancement")?;

    let new_key = force || !home.has_key();
    let identity = if new_key {
        let id = ot_core::Identity::generate();
        home.save_key(&id)?;
        id
    } else {
        home.load_key()?
    };
    let device_id = match device_id {
        Some(id) => Some(id.parse().map_err(|e| anyhow!("{e}"))?),
        None if new_key => None,
        None => existing.and_then(|c| c.device_id),
    };
    let registered = device_id.is_some();
    home.save_config(&Config { server_url, device_name: device_name.clone(), device_id })?;

    println!("Configuration : {}", home.path().display());
    println!("Clé publique  : {}", identity.public_key_b64());
    if !registered {
        println!();
        println!("Sur le serveur, enregistrer l'appareil :");
        println!("  sudo -u ot-relay ot-server --db /var/lib/ot-relay/ot.db \\");
        println!("    device add {device_name} {}", identity.public_key_b64());
        println!("puis ici : ot init --device-id <identifiant affiché>");
    }
    Ok(())
}

fn start(home: &Home) -> Result<(Client, Events)> {
    let config = home.load_config()?.context("appareil non configuré : lancer `ot init`")?;
    let device_id = config.device_id.context("appareil non enregistré : voir `ot init`")?;
    let mut client = ClientConfig::new(config.server_url, device_id, home.load_key()?);
    client.seen = home.load_seen();
    Ok(Client::start(client))
}

/// Attend la connexion, en échouant vite si l'authentification est refusée.
async fn connected(events: &mut Events) -> Result<()> {
    let wait = async {
        loop {
            match events.next().await {
                Some(Event::Connected { .. }) => return Ok(()),
                Some(Event::AuthFailed { message }) => {
                    bail!("authentification refusée : {message}")
                }
                Some(Event::Disconnected { reason }) => {
                    eprintln!("connexion impossible : {reason}")
                }
                Some(Event::Message(_)) => {}
                None => bail!("client arrêté"),
            }
        }
    };
    tokio::time::timeout(COMMAND_TIMEOUT, wait).await.context("relais injoignable")?
}

async fn send(home: &Home, text: Vec<String>, to: Vec<String>) -> Result<()> {
    let body = if text.is_empty() {
        if std::io::stdin().is_terminal() {
            bail!("rien à envoyer : donner un texte ou passer par l'entrée standard");
        }
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s.trim_end_matches('\n').to_owned()
    } else {
        text.join(" ")
    };
    if body.is_empty() {
        bail!("texte vide");
    }
    let content = classify(body);

    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let recipients = if to.is_empty() {
        Vec::new()
    } else {
        let devices = within(client.list_devices()).await?;
        to.iter().map(|name| resolve(&devices, name)).collect::<Result<_>>()?
    };
    let accepted = within(client.send(&content, recipients)).await?;
    client.shutdown().await;
    eprintln!("envoyé ({})", accepted.msg_id);
    Ok(())
}

async fn within<T, E: std::error::Error + Send + Sync + 'static>(
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T> {
    Ok(tokio::time::timeout(COMMAND_TIMEOUT, fut).await.context("pas de réponse du relais")??)
}

/// Un texte d'un seul mot commençant par http(s):// est un lien.
fn classify(body: String) -> Content {
    let trimmed = body.trim();
    let is_link = (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
        && !trimmed.contains(char::is_whitespace);
    if is_link { Content::link(trimmed) } else { Content::text(body) }
}

fn resolve(devices: &[DeviceInfo], name: &str) -> Result<ot_core::DeviceId> {
    devices
        .iter()
        .find(|d| d.name == name || d.id.as_str() == name)
        .map(|d| d.id.clone())
        .ok_or_else(|| {
            let names: Vec<_> = devices.iter().map(|d| d.name.as_str()).collect();
            anyhow!("appareil inconnu : {name} (connus : {})", names.join(", "))
        })
}

async fn listen(home: &Home, json: bool, offset: UtcOffset) -> Result<()> {
    let (client, mut events) = start(home)?;
    let mut seen = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, home.load_seen());
    let mut names: Vec<DeviceInfo> = Vec::new();
    let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

    loop {
        let event = tokio::select! {
            ev = events.next() => ev,
            _ = tokio::signal::ctrl_c() => break,
        };
        match event {
            None => break,
            Some(Event::Connected { pending }) => {
                eprintln!("connecté, {pending} message(s) en attente");
                // Les noms servent à l'affichage ; leur absence n'est pas bloquante.
                if let Ok(Ok(list)) =
                    tokio::time::timeout(COMMAND_TIMEOUT, client.list_devices()).await
                {
                    names = list;
                }
            }
            Some(Event::Disconnected { reason }) => {
                eprintln!("déconnecté : {reason} ; nouvel essai…")
            }
            Some(Event::AuthFailed { message }) => {
                eprintln!("authentification refusée : {message}")
            }
            Some(Event::Message(m)) => {
                let from = names
                    .iter()
                    .find(|d| d.id == m.from)
                    .map_or(m.from.as_str(), |d| d.name.as_str());
                if json {
                    let line = serde_json::json!({
                        "msg_id": m.msg_id.to_string(),
                        "from": m.from,
                        "from_name": from,
                        "created_at": m.created_at.format(&time::format_description::well_known::Rfc3339)?,
                        "kind": m.content.as_ref().map(|c| c.kind),
                        "body": m.content.as_ref().map(|c| c.body.as_str()),
                    });
                    println!("{line}");
                } else {
                    let when = m.created_at.to_offset(offset).format(&fmt)?;
                    match &m.content {
                        Some(c) if c.kind == ContentKind::Link => {
                            println!("[{when}] {from} 🔗 {}", c.body)
                        }
                        Some(c) => println!("[{when}] {from} : {}", c.body),
                        None => println!("[{when}] {from} : (contenu illisible)"),
                    }
                }
                seen.insert(m.msg_id);
                if let Err(e) = home.save_seen(seen.iter()) {
                    eprintln!("état non sauvegardé : {e:#}");
                }
            }
        }
    }
    client.shutdown().await;
    Ok(())
}

async fn devices(home: &Home) -> Result<()> {
    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let me = client.device_id().clone();
    for d in within(client.list_devices()).await? {
        let state = if d.id == me {
            "cet appareil"
        } else if d.online {
            "en ligne"
        } else {
            "hors ligne"
        };
        println!("{:<16} {}  {state}", d.name, d.id);
    }
    client.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_detected() {
        assert_eq!(classify("https://example.org/a?b".into()).kind, ContentKind::Link);
        assert_eq!(classify(" http://x.fr ".into()), Content::link("http://x.fr"));
        assert_eq!(classify("voir https://x.fr".into()).kind, ContentKind::Text);
        assert_eq!(classify("bonjour".into()).kind, ContentKind::Text);
    }
}
