mod files;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use ot_core::{
    Client, ClientConfig, Content, ContentKind, Dedup, DeviceId, Event, Events, GroupInfo,
    Identity, Received,
};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

use crate::files::{Config, Home};

/// Délai maximal d'une commande ponctuelle (send, devices).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// Durée de validité d'un code d'appairage (fixée par le relais).
const PAIRING_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
        /// Remplace la clé existante (l'appareil devra être réenregistré et
        /// réappairé).
        #[arg(long)]
        force: bool,
    },
    /// Envoie un texte ou un lien (lu sur l'entrée standard si absent).
    Send {
        text: Vec<String>,
        /// Destinataire (nom d'un membre du groupe), répétable ; par défaut
        /// tous les autres.
        #[arg(long = "to")]
        to: Vec<String>,
    },
    /// Affiche les messages reçus au fil de l'eau.
    Listen {
        /// Une ligne JSON par message.
        #[arg(long)]
        json: bool,
    },
    /// Liste les appareils enregistrés sur le relais et signale ceux qui ne
    /// sont pas membres du groupe.
    Devices,
    /// Groupe d'appareils et clé de groupe.
    #[command(subcommand)]
    Group(GroupCommand),
    /// Appairage d'un nouvel appareil par code.
    #[command(subcommand)]
    Pair(PairCommand),
}

#[derive(Subcommand)]
enum GroupCommand {
    /// Crée le groupe sur le premier appareil (époque 1, seul membre).
    Init {
        /// Remplace un groupe existant : les autres appareils devront être
        /// réappairés.
        #[arg(long)]
        force: bool,
    },
    /// Affiche l'époque, les membres et les empreintes de leurs clés.
    Show,
    /// Tire une nouvelle clé de groupe et l'envoie aux membres restants.
    Rotate {
        /// Membre à exclure (nom), répétable.
        #[arg(long = "exclude")]
        exclude: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PairCommand {
    /// Parraine un nouvel appareil : affiche un code et attend sa demande
    /// (10 minutes au plus).
    New,
    /// Rejoint le groupe avec le code affiché par `ot pair new`.
    Join {
        /// URL du relais, ex. ws://relais:7878.
        #[arg(long)]
        server: String,
        /// Nom de cet appareil.
        #[arg(long)]
        name: String,
        /// Code `ot1-…`.
        code: String,
        /// Repart de zéro si cet appareil est déjà enregistré.
        #[arg(long)]
        force: bool,
    },
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
            Command::Group(GroupCommand::Init { force }) => group_init(&home, force),
            Command::Group(GroupCommand::Show) => group_show(&home),
            Command::Group(GroupCommand::Rotate { exclude }) => group_rotate(&home, exclude).await,
            Command::Pair(PairCommand::New) => pair_new(&home, offset).await,
            Command::Pair(PairCommand::Join { server, name, code, force }) => {
                pair_join(&home, server, name, code, force).await
            }
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
        // Nouvelle identité : l'ancien groupe ne la connaît pas.
        home.reset_group()?;
        let id = Identity::generate();
        home.save_key(&id)?;
        id
    } else {
        home.load_key()?
    };
    ot_core::ensure_exchange_key(&home.keyring())?;
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
        println!(
            "Pour rejoindre un groupe existant, préférer `ot pair join` (sans passer par le serveur)."
        );
        println!("Pour le tout premier appareil, l'enregistrer sur le serveur :");
        println!("  sudo -u ot-relay ot-server --db /var/lib/ot-relay/ot.db \\");
        println!("    device add {device_name} {}", identity.public_key_b64());
        println!("puis ici : ot init --device-id <identifiant affiché> && ot group init");
    }
    Ok(())
}

fn registered(home: &Home) -> Result<(Config, DeviceId)> {
    let config = home.load_config()?.context("appareil non configuré : lancer `ot init`")?;
    let device_id = config.device_id.clone().context("appareil non enregistré : voir `ot init`")?;
    Ok((config, device_id))
}

fn start(home: &Home) -> Result<(Client, Events)> {
    let (config, device_id) = registered(home)?;
    let mut client =
        ClientConfig::new(config.server_url, device_id, home.load_key()?, Arc::new(home.keyring()));
    client.seen = home.load_seen();
    Ok(Client::start(client))
}

/// État du groupe enregistré, ou une erreur qui dit comment en avoir un.
fn group(home: &Home) -> Result<GroupInfo> {
    ot_core::group_info(&home.keyring())?.ok_or_else(|| anyhow!(ot_core::ClientError::NoGroup))
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
                Some(Event::KeyringError { message }) => eprintln!("{message}"),
                Some(_) => {}
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
    // Les destinataires sont choisis dans le roster, jamais dans la liste du relais.
    let group = group(home)?;
    let recipients = to.iter().map(|name| resolve(&group, name)).collect::<Result<_>>()?;

    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
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

/// Membre du groupe désigné par son nom ou son identifiant.
fn resolve(group: &GroupInfo, name: &str) -> Result<DeviceId> {
    group
        .members
        .iter()
        .find(|m| m.name == name || m.device_id.as_str() == name)
        .map(|m| m.device_id.clone())
        .ok_or_else(|| {
            let names: Vec<_> = group.members.iter().map(|m| m.name.as_str()).collect();
            anyhow!("{name} n'est pas membre du groupe (membres : {})", names.join(", "))
        })
}

/// Nom d'affichage d'un appareil, tiré du roster.
fn name_of<'a>(group: Option<&'a GroupInfo>, id: &'a DeviceId) -> &'a str {
    group.and_then(|g| g.member(id)).map_or(id.as_str(), |m| m.name.as_str())
}

/// Affichage commun à `listen` et `pair new`. Renvoie `true` pour un message.
fn print_event(
    event: &Event,
    group: Option<&GroupInfo>,
    json: bool,
    offset: UtcOffset,
) -> Result<bool> {
    match event {
        Event::Connected { pending } => eprintln!("connecté, {pending} message(s) en attente"),
        Event::Disconnected { reason } => eprintln!("déconnecté : {reason} ; nouvel essai…"),
        Event::AuthFailed { message } => eprintln!("authentification refusée : {message}"),
        Event::Rejected { from, reason, .. } => {
            eprintln!("⚠ message de {} rejeté : {reason}", name_of(group, from))
        }
        Event::GroupChanged { epoch } => eprintln!("groupe mis à jour (époque {epoch})"),
        Event::PairingCompleted { name, .. } => eprintln!("« {name} » a rejoint le groupe"),
        Event::KeyringError { message } => eprintln!("⚠ {message}"),
        Event::Message(m) => {
            print_message(m, name_of(group, &m.from), json, offset)?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn print_message(m: &Received, from: &str, json: bool, offset: UtcOffset) -> Result<()> {
    if json {
        let line = serde_json::json!({
            "msg_id": m.msg_id.to_string(),
            "from": m.from,
            "from_name": from,
            "created_at": m.created_at.format(&Rfc3339)?,
            "kind": m.content.kind,
            "body": m.content.body,
        });
        println!("{line}");
    } else {
        let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
        let when = m.created_at.to_offset(offset).format(&fmt)?;
        match m.content.kind {
            ContentKind::Link => println!("[{when}] {from} 🔗 {}", m.content.body),
            ContentKind::Text => println!("[{when}] {from} : {}", m.content.body),
        }
    }
    Ok(())
}

async fn listen(home: &Home, json: bool, offset: UtcOffset) -> Result<()> {
    let (client, mut events) = start(home)?;
    let mut seen = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, home.load_seen());
    let mut group = ot_core::group_info(&home.keyring()).ok().flatten();
    if group.is_none() {
        eprintln!("⚠ {}", ot_core::ClientError::NoGroup);
    }

    loop {
        let event = tokio::select! {
            ev = events.next() => ev,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(event) = event else { break };
        if let Event::GroupChanged { .. } = event
            && let Ok(Ok(g)) = tokio::time::timeout(COMMAND_TIMEOUT, client.group()).await
        {
            group = g;
        }
        if print_event(&event, group.as_ref(), json, offset)?
            && let Event::Message(m) = &event
        {
            seen.insert(m.msg_id);
            if let Err(e) = home.save_seen(seen.iter()) {
                eprintln!("état non sauvegardé : {e:#}");
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
    let list = within(client.list_devices()).await?;
    client.shutdown().await;
    let group = ot_core::group_info(&home.keyring())?;

    for d in &list {
        let state = if d.id == me {
            "cet appareil"
        } else if d.online {
            "en ligne"
        } else {
            "hors ligne"
        };
        let member = match &group {
            Some(g) if g.member(&d.id).is_none() => "  ⚠ hors du groupe",
            _ => "",
        };
        println!("{:<16} {}  {state}{member}", d.name, d.id);
    }
    let Some(group) = group else {
        eprintln!("⚠ {}", ot_core::ClientError::NoGroup);
        return Ok(());
    };
    let strangers = group.strangers(&list);
    if !strangers.is_empty() {
        eprintln!();
        eprintln!("⚠ Appareils enregistrés sur le relais mais absents du groupe : appairage");
        eprintln!("  inachevé, ou relais compromis. Ils ne peuvent rien lire ni rien envoyer");
        eprintln!("  de recevable. Si vous ne les reconnaissez pas, les révoquer sur le serveur");
        eprintln!("  (`ot-server device revoke <nom>`).");
    }
    for m in group.members.iter().filter(|m| !list.iter().any(|d| d.id == m.device_id)) {
        eprintln!(
            "⚠ « {} » est membre du groupe mais inconnu du relais (révoqué ?) : \
             `ot group rotate --exclude {}`",
            m.name, m.name
        );
    }
    Ok(())
}

fn group_init(home: &Home, force: bool) -> Result<()> {
    let (config, device_id) = registered(home)?;
    let identity = home.load_key()?;
    let info =
        ot_core::init_group(&home.keyring(), &device_id, &config.device_name, &identity, force)?;
    println!("Groupe créé (époque {}), seul membre : {}", info.epoch, config.device_name);
    println!("Ajouter un appareil : `ot pair new` ici, puis `ot pair join` sur l'autre.");
    Ok(())
}

fn group_show(home: &Home) -> Result<()> {
    let (_, me) = registered(home)?;
    let group = group(home)?;
    println!(
        "Époque {}  clé {}  roster v{}",
        group.epoch, group.key_fingerprint, group.roster_version
    );
    for m in &group.members {
        let this = if m.device_id == me { "  (cet appareil)" } else { "" };
        println!(
            "  {:<16} {}  signature {}  échange {}  depuis l'époque {}{this}",
            m.name, m.device_id, m.sign_fingerprint, m.exchange_fingerprint, m.added_epoch
        );
    }
    Ok(())
}

async fn group_rotate(home: &Home, exclude: Vec<String>) -> Result<()> {
    let group = group(home)?;
    let excluded = exclude.iter().map(|name| resolve(&group, name)).collect::<Result<Vec<_>>>()?;
    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let epoch = within(client.rotate(excluded)).await?;
    client.shutdown().await;
    println!("Nouvelle clé de groupe : époque {epoch}");
    for name in &exclude {
        println!(
            "« {name} » ne lira plus les messages suivants. Sur le serveur, le révoquer aussi :"
        );
        println!("  sudo -u ot-relay ot-server --db /var/lib/ot-relay/ot.db device revoke {name}");
    }
    Ok(())
}

async fn pair_new(home: &Home, offset: UtcOffset) -> Result<()> {
    let mut group = Some(group(home)?);
    let (config, _) = registered(home)?;
    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let invitation = within(client.pair_create()).await?;
    let until =
        invitation.expires_at.to_offset(offset).format(&format_description!("[hour]:[minute]"))?;

    println!("Code d'appairage, valable jusqu'à {until} et une seule fois :");
    println!();
    println!("  {}", invitation.code);
    println!();
    println!("Sur le nouvel appareil (le code contient un secret : le transmettre par un");
    println!("canal de confiance, pas par un service tiers) :");
    println!("  ot pair join --server {} --name <nom> {}", config.server_url, invitation.code);

    let mut seen = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, home.load_seen());
    let left = Duration::try_from(invitation.expires_at - OffsetDateTime::now_utc())
        .unwrap_or(Duration::ZERO)
        .min(PAIRING_TIMEOUT);
    let deadline = tokio::time::Instant::now() + left;
    let result = loop {
        let event = tokio::select! {
            ev = tokio::time::timeout_at(deadline, events.next()) => match ev {
                Ok(ev) => ev,
                Err(_) => break Err(anyhow!("code expiré sans demande d'appairage")),
            },
            _ = tokio::signal::ctrl_c() => break Err(anyhow!("appairage abandonné")),
        };
        let Some(event) = event else { break Err(anyhow!("client arrêté")) };
        if let Event::GroupChanged { .. } = event
            && let Ok(Ok(g)) = tokio::time::timeout(COMMAND_TIMEOUT, client.group()).await
        {
            group = g;
        }
        // Les messages reçus pendant l'attente sont affichés, pas perdus.
        if print_event(&event, group.as_ref(), false, offset)?
            && let Event::Message(m) = &event
        {
            seen.insert(m.msg_id);
            if let Err(e) = home.save_seen(seen.iter()) {
                eprintln!("état non sauvegardé : {e:#}");
            }
        }
        if let Event::PairingCompleted { .. } = event {
            break Ok(());
        }
    };
    // pair_welcome et roster sont déjà écrits sur la connexion, avant
    // l'arrêt : le relais les lit dans l'ordre.
    client.shutdown().await;
    result
}

async fn pair_join(
    home: &Home,
    server: String,
    name: String,
    code: String,
    force: bool,
) -> Result<()> {
    if let Some(Config { device_id: Some(id), .. }) = home.load_config()?
        && !force
    {
        bail!("cet appareil est déjà enregistré ({id}) ; --force pour repartir de zéro");
    }
    let name = name.trim().to_owned();
    let identity = Identity::generate();
    let redeemed = ot_core::redeem_pairing(&server, &identity, &name, &code).await?;

    home.reset_group()?;
    home.save_key(&identity)?;
    home.save_config(&Config {
        server_url: server,
        device_name: name.clone(),
        device_id: Some(redeemed.device_id.clone()),
    })?;
    println!(
        "Appareil « {name} » enregistré ({}). Demande envoyée au parrain…",
        redeemed.device_id
    );

    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    within(client.pair_join(&code, redeemed.sponsor, &name)).await?;
    let wait = async {
        loop {
            match events.next().await {
                Some(Event::GroupChanged { epoch }) => return Ok(epoch),
                Some(Event::Rejected { reason, .. }) => eprintln!("⚠ message rejeté : {reason}"),
                Some(Event::KeyringError { message }) => eprintln!("⚠ {message}"),
                Some(_) => {}
                None => bail!("client arrêté"),
            }
        }
    };
    let res = tokio::time::timeout(PAIRING_TIMEOUT, wait).await;
    client.shutdown().await;
    let epoch = res.map_err(|_| {
        anyhow!(
            "pas de réponse du parrain : relancer `ot pair new` sur un membre, puis \
             `ot pair join --force` ici avec le nouveau code"
        )
    })??;
    println!("Groupe rejoint (époque {epoch}).");
    group_show(home)
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
