use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use ot_proto::b64;
use ot_server::store::Store;
use ot_server::{Config, Relay, now_ms};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing_subscriber::EnvFilter;

/// Serveur relais OldTown.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Fichier de la base SQLite.
    #[arg(long, env = "OT_DB", default_value = "ot.db", global = true)]
    db: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lance le relais.
    Serve {
        /// Adresse d'écoute : l'IP Tailscale du serveur, jamais 0.0.0.0.
        #[arg(long, env = "OT_LISTEN", default_value = "127.0.0.1:7878")]
        listen: SocketAddr,
        /// Durée de vie d'un message non acquitté (ex. 7d, 24h).
        #[arg(long, env = "OT_TTL", default_value = "7d", value_parser = parse_duration)]
        ttl: Duration,
        /// Intervalle de purge.
        #[arg(long, env = "OT_PURGE_INTERVAL", default_value = "10m", value_parser = parse_duration)]
        purge_interval: Duration,
        /// Format des logs.
        #[arg(long, env = "OT_LOG_FORMAT", default_value = "text")]
        log_format: LogFormat,
    },
    /// Gère les appareils autorisés.
    #[command(subcommand)]
    Device(DeviceCommand),
    /// Gère les messages stockés.
    #[command(subcommand)]
    Messages(MessagesCommand),
}

#[derive(Subcommand)]
enum MessagesCommand {
    /// Supprime les messages acquittés ou expirés (comme la purge périodique).
    Purge {
        /// Supprime tous les messages, livrés ou non, puis compacte la base
        /// (bascule vers le chiffrement). Arrêter le relais avant.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum DeviceCommand {
    /// Enregistre un appareil à partir de la clé publique affichée par `ot init`.
    Add { name: String, public_key: String },
    /// Liste les appareils, révoqués compris.
    List,
    /// Révoque un appareil (nom ou identifiant).
    Revoke { device: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { listen, ttl, purge_interval, log_format } => {
            init_logs(log_format);
            serve(cli.db, listen, Config { ttl, purge_interval, ..Config::default() }).await
        }
        Command::Device(cmd) => device(&cli.db, cmd).await,
        Command::Messages(cmd) => messages(&cli.db, cmd).await,
    }
}

fn init_logs(format: LogFormat) {
    let filter = EnvFilter::try_from_env("OT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Text => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}

async fn serve(db: PathBuf, listen: SocketAddr, config: Config) -> Result<()> {
    let store = Store::open(&db).await.with_context(|| format!("ouverture de {}", db.display()))?;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("écoute sur {listen}"))?;
    tracing::info!(%listen, db = %db.display(), ttl = ?config.ttl, "relais démarré");
    ot_server::serve(Relay::new(store.clone(), config), listener, shutdown_signal()).await?;
    store.close().await;
    tracing::info!("relais arrêté");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("gestionnaire SIGTERM");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
}

async fn device(db: &Path, cmd: DeviceCommand) -> Result<()> {
    let store = Store::open(db).await.with_context(|| format!("ouverture de {}", db.display()))?;
    match cmd {
        DeviceCommand::Add { name, public_key } => {
            let Some(key) = b64::decode_array::<32>(public_key.trim()) else {
                bail!("clé publique invalide : 32 octets en base64 attendus");
            };
            ed25519_dalek::VerifyingKey::from_bytes(&key)
                .context("ce n'est pas une clé publique Ed25519 valide")?;
            let d = store.add_device(name.trim(), &key, now_ms()).await?;
            println!("{}", d.id);
            eprintln!("Appareil « {} » ajouté. Sur l'appareil, lancer :", d.name);
            eprintln!("  ot init --device-id {}", d.id);
        }
        DeviceCommand::List => {
            for d in store.devices().await? {
                let state = match d.revoked_at {
                    Some(t) => format!("révoqué le {}", fmt_ms(t)),
                    None => match d.last_seen_at {
                        Some(t) => format!("vu le {}", fmt_ms(t)),
                        None => "jamais connecté".into(),
                    },
                };
                println!("{}  {:<16} {}  {}", d.id, d.name, b64::encode(&d.public_key), state);
            }
        }
        DeviceCommand::Revoke { device } => {
            let d = store.revoke_device(&device, now_ms()).await?;
            println!("{} révoqué", d.id);
            eprintln!("Une session déjà ouverte reste active jusqu'à sa prochaine reconnexion.");
        }
    }
    store.close().await;
    Ok(())
}

async fn messages(db: &Path, cmd: MessagesCommand) -> Result<()> {
    let store = Store::open(db).await.with_context(|| format!("ouverture de {}", db.display()))?;
    match cmd {
        MessagesCommand::Purge { all: true } => {
            let n = store.purge_all_messages().await?;
            println!("{n} message(s) supprimé(s), base compactée");
        }
        MessagesCommand::Purge { all: false } => {
            let stats = store.purge(now_ms()).await?;
            println!(
                "{} acquitté(s), {} expiré(s), {} jeton(s) d'appairage supprimé(s)",
                stats.acked, stats.expired, stats.pairings
            );
        }
    }
    store.close().await;
    Ok(())
}

fn fmt_ms(ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| ms.to_string())
}

/// Durée au format `<nombre><unité>`, unité parmi `ms`, `s`, `m`, `h`, `d`.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: u64 = n.parse().map_err(|_| format!("durée invalide : {s:?}"))?;
    let ms = match unit {
        "ms" => 1,
        "s" | "" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(format!("unité inconnue dans {s:?} (ms, s, m, h, d)")),
    };
    Ok(Duration::from_millis(n * ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(7 * 86_400));
        assert_eq!(parse_duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
        assert!(parse_duration("3 jours").is_err());
        assert!(parse_duration("h").is_err());
    }
}
