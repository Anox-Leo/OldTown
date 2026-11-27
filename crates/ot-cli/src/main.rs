mod files;
mod otp;
mod people;
mod transfer;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use ot_core::{
    BlobId, Client, ClientConfig, ContentKind, Dedup, DeviceId, Event, Events, FileInfo, GroupInfo,
    Identity, Origin, OutgoingFile, Person, Received, ReceivedOtp, TlsRoots,
};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

use crate::files::{Config, Home};
use crate::transfer::{FileSink, FileSource, guess_mime, human_size, source_id};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const PAIRING_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Parser)]
#[command(about = "OldTown : envoyer un texte d'un terminal à un autre")]
#[command(name = "ot", version)]
struct Cli {
    #[arg(help = "Dossier de configuration (par défaut celui de l'OS)")]
    #[arg(long, env = "OT_HOME", global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Crée la clé de l'appareil et la configuration, ou les met à jour")]
    Init {
        #[arg(help = "URL du relais, ex. ws://relais:7878")]
        #[arg(long)]
        server: Option<String>,
        #[arg(help = "Nom de cet appareil")]
        #[arg(long)]
        name: Option<String>,
        #[arg(help = "Identifiant renvoyé par `ot-server device add`")]
        #[arg(long)]
        device_id: Option<String>,
        #[arg(
            help = "Remplace la clé existante (l'appareil devra être réenregistré et réappairé)"
        )]
        #[arg(long)]
        force: bool,
    },
    #[command(
        about = "Envoie un texte ou un lien (lu sur l'entrée standard si absent), ou un fichier avec --file"
    )]
    Send {
        text: Vec<String>,
        #[arg(
            help = "Destinataire (nom d'un membre du groupe), répétable ; --to ou --to-user est obligatoire"
        )]
        #[arg(long = "to")]
        to: Vec<String>,
        #[arg(
            help = "Une autre personne (nom ou identifiant), à tous ses appareils joignables ; une copie part vers mes appareils"
        )]
        #[arg(long = "to-user", conflicts_with = "to")]
        to_user: Option<String>,
        #[arg(help = "Fichier à envoyer. Relancer la même commande reprend un envoi interrompu")]
        #[arg(long, conflicts_with = "text")]
        file: Option<PathBuf>,
    },
    #[command(about = "Affiche les messages reçus au fil de l'eau")]
    Listen {
        #[arg(help = "Une ligne JSON par message")]
        #[arg(long)]
        json: bool,
        #[arg(
            help = "Télécharge les fichiers reçus dans ce dossier ; sans cette option, ils sont seulement annoncés (`ot download`)"
        )]
        #[arg(long)]
        download_dir: Option<PathBuf>,
    },
    #[command(about = "Liste les fichiers reçus pas encore téléchargés")]
    Files,
    #[command(about = "Télécharge un fichier reçu (identifiant donné par `ot files`)")]
    Download {
        #[arg(help = "Identifiant `blob_…`")]
        blob: String,
        #[arg(help = "Dossier de destination (par défaut le dossier courant)")]
        #[arg(long)]
        to: Option<PathBuf>,
    },
    #[command(
        about = "Liste les appareils de ma personne enregistrés sur le relais et signale ceux qui ne sont pas membres du groupe"
    )]
    Devices,
    #[command(about = "Règle ou retire un appareil de ma personne (l'admin : de tous)")]
    #[command(subcommand)]
    Device(DeviceCommand),
    #[command(about = "Personnes que je peux joindre (annuaire vérifié) et leurs appareils")]
    Users,
    #[command(
        about = "Coupe (`on`) ou rouvre (`off`) un service lié à ma personne : ses codes et alertes ne m'arrivent plus"
    )]
    Mute {
        #[arg(help = "Service (nom ou identifiant)")]
        service: String,
        state: OnOff,
    },
    #[command(about = "Rejoindre OldTown par l'invitation d'une personne")]
    #[command(subcommand)]
    Invite(InviteCommand),
    #[command(
        about = "Administration (appareil de l'admin) : annuaire, personnes, liens, services"
    )]
    #[command(subcommand)]
    Admin(AdminCommand),
    #[command(about = "Groupe d'appareils et clé de groupe")]
    #[command(subcommand)]
    Group(GroupCommand),
    #[command(about = "Appairage d'un nouvel appareil par code")]
    #[command(subcommand)]
    Pair(PairCommand),
    #[command(
        about = "Relaie les codes à usage unique lus par otp-relay (socket Unix) vers des membres du groupe, ou vers des personnes. Tourne jusqu'à l'arrêt (service)"
    )]
    OtpBridge {
        #[arg(
            help = "Destinataire (nom d'un membre du groupe), répétable ; ou, dans `OT_OTP_TO`, des noms séparés par des virgules"
        )]
        #[arg(long = "to", env = "OT_OTP_TO", value_delimiter = ',')]
        to: Vec<String>,
        #[arg(
            help = "Personne destinataire (nom ou identifiant), à tous ses appareils joignables, répétable ; ou, dans `OT_OTP_TO_USER`, séparées par des virgules. Ne se combine pas avec --to"
        )]
        #[arg(long = "to-user", env = "OT_OTP_TO_USER", value_delimiter = ',')]
        to_user: Vec<String>,
        #[arg(
            help = "Socket d'otp-relay (par défaut `$XDG_RUNTIME_DIR/otp-relay/otp-relay.sock`)"
        )]
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(help = "Confiance minimale d'un code (0 à 1)")]
        #[arg(long, default_value_t = 0.6)]
        min_confidence: f64,
    },
}

#[derive(Subcommand)]
enum GroupCommand {
    #[command(about = "Crée le groupe sur le premier appareil (époque 1, seul membre)")]
    Init {
        #[arg(help = "Remplace un groupe existant : les autres appareils devront être réappairés")]
        #[arg(long)]
        force: bool,
    },
    #[command(about = "Affiche l'époque, les membres et les empreintes de leurs clés")]
    Show,
    #[command(about = "Tire une nouvelle clé de groupe et l'envoie aux membres restants")]
    Rotate {
        #[arg(help = "Membre à exclure (nom), répétable")]
        #[arg(long = "exclude")]
        exclude: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PairCommand {
    #[command(
        about = "Parraine un nouvel appareil : affiche un code et attend sa demande (10 minutes au plus)"
    )]
    New {
        #[arg(help = "Affiche aussi le code en QR, à scanner par le téléphone")]
        #[arg(long)]
        qr: bool,
    },
    #[command(
        about = "Rejoint le groupe avec le code affiché par `ot pair new`, ou, avec --qr, en affichant un QR à scanner par un membre (le téléphone)"
    )]
    Join {
        #[arg(help = "URL du relais, ex. wss://relais:7878")]
        #[arg(long)]
        server: String,
        #[arg(help = "Nom de cet appareil")]
        #[arg(long)]
        name: String,
        #[arg(help = "Code `ot1-…` (sans --qr)")]
        #[arg(required_unless_present = "qr", conflicts_with = "qr")]
        code: Option<String>,
        #[arg(help = "Affiche un QR que le parrain scanne : rien à taper ici")]
        #[arg(long)]
        qr: bool,
        #[arg(
            help = "Avec --qr : appareil d'une personne de service (codes, alertes), sans groupe, ajouté par l'admin (`ot admin service add`)"
        )]
        #[arg(long, requires = "qr")]
        service: bool,
        #[arg(help = "Repart de zéro si cet appareil est déjà enregistré")]
        #[arg(long)]
        force: bool,
    },
    #[command(
        about = "Parraine l'appareil dont on a copié le texte du QR (`oldtown://sponsor?…`), affiché par `ot pair join --qr`"
    )]
    Sponsor {
        #[arg(help = "Texte du QR")]
        qr: String,
    },
}

#[derive(Subcommand)]
enum DeviceCommand {
    #[command(
        about = "Nature d'un appareil : phone, laptop, desktop, browser ou cli (une CLI n'est jamais joignable ni par défaut)"
    )]
    Kind {
        #[arg(help = "Appareil (nom ou identifiant)")]
        device: String,
        kind: String,
    },
    #[command(
        about = "Appareil principal de ma personne : le seul qui notifie et télécharge d'office"
    )]
    Default {
        #[arg(help = "Appareil (nom ou identifiant)")]
        device: String,
    },
    #[command(about = "Appareil joignable par les autres personnes")]
    Reachable { device: String, state: OnOff },
    #[command(
        about = "Retire un appareil : nouvelle clé de groupe sans lui, roster publié, puis révocation par le relais"
    )]
    Revoke { device: String },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Subcommand)]
enum InviteCommand {
    #[command(
        about = "Rejoint OldTown avec le code (`ot2-…`) ou le lien (`oldtown://invite?…`) reçu de l'admin, comme premier appareil de sa personne"
    )]
    Join {
        #[arg(help = "Code ou lien d'invitation")]
        code: String,
        #[arg(help = "Nom de cet appareil")]
        #[arg(long)]
        name: String,
        #[arg(help = "URL du relais (déjà dans un lien `oldtown://invite?…`)")]
        #[arg(long)]
        server: Option<String>,
        #[arg(help = "Repart de zéro si cet appareil est déjà enregistré")]
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    #[command(
        about = "Publie le certificat de l'admin (son empreinte est l'ancre de l'annuaire), ou adopte celui d'un autre appareil de l'admin"
    )]
    Init,
    #[command(
        about = "Invite une personne : affiche le code (et le QR), puis attend sa demande pour la certifier. Ctrl-C : l'invitation reste valable, un prochain `ot listen` ici la mènera à son terme"
    )]
    Invite {
        #[arg(help = "Nom de la personne")]
        name: String,
        #[arg(help = "Personne à lier dans les deux sens (nom ou identifiant), répétable")]
        #[arg(long = "link")]
        links: Vec<String>,
        #[arg(help = "Affiche aussi l'invitation en QR")]
        #[arg(long)]
        qr: bool,
    },
    #[command(
        about = "Nouvelle invitation pour une personne qui a perdu ses appareils (révoqués ; ses liens restent)"
    )]
    Reinvite {
        person: String,
        #[arg(long)]
        qr: bool,
    },
    #[command(about = "Toutes les personnes, leurs appareils et leurs liens, vus par le relais")]
    Users,
    #[command(
        about = "Liens entre personnes (dans les deux sens ; d'un service vers une personne, dans ce seul sens)"
    )]
    #[command(subcommand)]
    Link(LinkCommand),
    #[command(about = "Révoque une personne : ses appareils, ses liens, son certificat")]
    Revoke { person: String },
    #[command(about = "Renomme une personne (son certificat est re-signé)")]
    Rename { person: String, name: String },
    #[command(about = "Personnes de service (codes, alertes)")]
    #[command(subcommand)]
    Service(ServiceCommand),
}

#[derive(Subcommand)]
enum LinkCommand {
    #[command(about = "Pose le lien de `from` vers `to` (et retour, entre deux personnes)")]
    Add { from: String, to: String },
    #[command(about = "Retire le lien")]
    Remove { from: String, to: String },
}

#[derive(Subcommand)]
enum ServiceCommand {
    #[command(
        about = "Ajoute l'appareil de service affiché par `ot pair join --qr --service` au service nommé, créé à son premier appareil"
    )]
    Add {
        #[arg(help = "Nom du service")]
        service: String,
        #[arg(help = "Texte du QR (`oldtown://sponsor?…`)")]
        #[arg(long)]
        qr: String,
    },
}

fn main() -> Result<()> {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let cli = Cli::parse();
    let home = Home::locate(cli.home)?;
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async move {
        match cli.command {
            Command::Init { server, name, device_id, force } => {
                init(&home, server, name, device_id, force)
            }
            Command::Send { file: Some(path), to_user: Some(user), .. } => {
                people::send_file_to_user(&home, &path, &user).await
            }
            Command::Send { text, to_user: Some(user), file: None, .. } => {
                people::send_to_user(&home, text, &user).await
            }
            Command::Send { file: Some(path), to, .. } => send_file(&home, &path, to).await,
            Command::Send { text, to, file: None, .. } => send(&home, text, to).await,
            Command::Listen { json, download_dir } => {
                listen(&home, json, download_dir, offset).await
            }
            Command::Files => list_files(&home, offset).await,
            Command::Download { blob, to } => download(&home, &blob, to).await,
            Command::Devices => devices(&home).await,
            Command::Device(DeviceCommand::Kind { device, kind }) => {
                people::device_kind(&home, &device, &kind).await
            }
            Command::Device(DeviceCommand::Default { device }) => {
                people::device_principal(&home, &device).await
            }
            Command::Device(DeviceCommand::Reachable { device, state }) => {
                people::device_reachable(&home, &device, state).await
            }
            Command::Device(DeviceCommand::Revoke { device }) => {
                people::device_revoke(&home, &device).await
            }
            Command::Users => people::users(&home).await,
            Command::Mute { service, state } => people::mute(&home, &service, state).await,
            Command::Invite(InviteCommand::Join { code, name, server, force }) => {
                people::invite_join(&home, &code, &name, server, force).await
            }
            Command::Admin(cmd) => people::admin(&home, cmd, offset).await,
            Command::Group(GroupCommand::Init { force }) => group_init(&home, force),
            Command::Group(GroupCommand::Show) => group_show(&home),
            Command::Group(GroupCommand::Rotate { exclude }) => group_rotate(&home, exclude).await,
            Command::Pair(PairCommand::New { qr }) => pair_new(&home, qr, offset).await,
            Command::Pair(PairCommand::Join { server, name, code: Some(code), force, .. }) => {
                pair_join(&home, server, name, code, force).await
            }
            Command::Pair(PairCommand::Join {
                server, name, code: None, force, service, ..
            }) => pair_join_qr(&home, server, name, force, service).await,
            Command::Pair(PairCommand::Sponsor { qr }) => pair_sponsor(&home, &qr).await,
            Command::OtpBridge { to, to_user, socket, min_confidence } => {
                otp_bridge(&home, to, to_user, socket, min_confidence).await
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
        None => existing.as_ref().and_then(|c| c.device_id.clone()),
    };
    let registered = device_id.is_some();
    let role = existing.as_ref().and_then(|c| c.role);
    home.save_config(&Config { server_url, device_name: device_name.clone(), device_id, role })?;

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

fn group(home: &Home) -> Result<GroupInfo> {
    ot_core::group_info(&home.keyring())?.ok_or_else(|| anyhow!(ot_core::ClientError::NoGroup))
}

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

fn require_target(to: &[String]) -> Result<()> {
    if to.is_empty() {
        bail!(
            "aucun destinataire : donner --to <appareil> (un de mes appareils) ou --to-user <personne>"
        );
    }
    Ok(())
}

async fn send(home: &Home, text: Vec<String>, to: Vec<String>) -> Result<()> {
    require_target(&to)?;
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
    let content = ot_core::classify(&body);
    let group = group(home)?;
    let recipients = to.iter().map(|name| resolve(&group, name)).collect::<Result<_>>()?;

    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let accepted = within(client.send(&content, recipients)).await?;
    client.shutdown().await;
    eprintln!("envoyé ({})", accepted.msg_id);
    Ok(())
}

async fn send_file(home: &Home, path: &std::path::Path, to: Vec<String>) -> Result<()> {
    require_target(&to)?;
    let group = group(home)?;
    let recipients = to.iter().map(|name| resolve(&group, name)).collect::<Result<_>>()?;
    let source = Arc::new(FileSource::open(path)?);
    let name =
        path.file_name().and_then(|n| n.to_str()).context("nom de fichier illisible")?.to_owned();
    let size = ot_core::ChunkSource::size(&*source);
    let file = OutgoingFile {
        source_id: source_id(path)?,
        name: name.clone(),
        mime: guess_mime(path).to_owned(),
        to: recipients,
        to_user: None,
    };

    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    eprintln!("envoi de {name} ({})…", human_size(size));
    let res = client.send_file(file, source, progress_bar()).await;
    client.shutdown().await;
    let accepted = res.map_err(|e| {
        anyhow!("{e}\nRelancer la même commande reprend l'envoi là où il s'est arrêté.")
    })?;
    eprintln!("\rfichier envoyé ({})", accepted.msg_id);
    Ok(())
}

fn progress_bar() -> Arc<dyn ot_core::Progress> {
    let tty = std::io::stderr().is_terminal();
    Arc::new(move |done: u64, total: u64| {
        if tty && total > 0 {
            eprint!("\r{:>3} % ({} / {})", done * 100 / total, human_size(done), human_size(total));
        }
    })
}

async fn list_files(home: &Home, offset: UtcOffset) -> Result<()> {
    let (client, _events) = start(home)?;
    let files = client.files().await;
    client.shutdown().await;
    let files = files?;
    if files.is_empty() {
        eprintln!("aucun fichier en attente");
    }
    let group = ot_core::group_info(&home.keyring()).ok().flatten();
    let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]");
    for f in files {
        let when = f.created_at.to_offset(offset).format(&fmt)?;
        let from = name_of(group.as_ref(), &f.from);
        println!("{}  [{when}] {from} : {} ({})", f.blob_id, f.name, human_size(f.size));
    }
    Ok(())
}

async fn download(home: &Home, blob: &str, to: Option<PathBuf>) -> Result<()> {
    let blob: BlobId = blob.parse().map_err(|e| anyhow!("{e}"))?;
    let dir = match to {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let (client, _events) = start(home)?;
    let res = fetch(&client, &blob, &dir).await;
    client.shutdown().await;
    let path = res?;
    eprintln!("\renregistré : {}", path.display());
    Ok(())
}

async fn fetch(client: &Client, blob: &BlobId, dir: &std::path::Path) -> Result<PathBuf> {
    let files = client.files().await?;
    let info = files
        .iter()
        .find(|f| &f.blob_id == blob)
        .context("fichier inconnu sur cet appareil (voir `ot files`)")?;
    let sink = Arc::new(FileSink::create(dir, &info.name)?);
    client.download(blob, sink.clone(), progress_bar()).await?;
    let saved = sink.saved.lock().unwrap_or_else(|e| e.into_inner()).clone();
    saved.context("fichier non enregistré")
}

async fn within<T, E: std::error::Error + Send + Sync + 'static>(
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T> {
    Ok(tokio::time::timeout(COMMAND_TIMEOUT, fut).await.context("pas de réponse du relais")??)
}

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

fn name_of<'a>(group: Option<&'a GroupInfo>, id: &'a DeviceId) -> &'a str {
    group.and_then(|g| g.member(id)).map_or(id.as_str(), |m| m.name.as_str())
}

#[derive(Default)]
struct Names {
    group: Option<GroupInfo>,
    people: Vec<Person>,
}

impl Names {
    fn device(&self, id: &DeviceId) -> String {
        if let Some(m) = self.group.as_ref().and_then(|g| g.member(id)) {
            return m.name.clone();
        }
        self.people
            .iter()
            .find_map(|p| p.roster.member(id).map(|m| format!("{}/{}", p.name, m.name)))
            .unwrap_or_else(|| id.to_string())
    }

    fn user(&self, id: &ot_core::UserId) -> String {
        self.people.iter().find(|p| &p.id == id).map_or_else(|| id.to_string(), |p| p.name.clone())
    }

    fn origin(&self, o: &Origin, line: &mut serde_json::Value) {
        if let Some(u) = &o.from_user {
            line["from_user"] = u.to_string().into();
            line["from_user_name"] = self.user(u).into();
        }
        if o.from_service {
            line["from_service"] = true.into();
        }
        if let Some(u) = &o.to_user {
            line["to_user"] = u.to_string().into();
            line["to_user_name"] = self.user(u).into();
        }
    }

    fn copy(&self, o: &Origin) -> String {
        o.to_user.as_ref().map_or_else(String::new, |u| format!("(copie, à {}) ", self.user(u)))
    }
}

fn print_event(event: &Event, names: &Names, json: bool, offset: UtcOffset) -> Result<bool> {
    match event {
        Event::Connected { pending } => eprintln!("connecté, {pending} message(s) en attente"),
        Event::Disconnected { reason } => eprintln!("déconnecté : {reason} ; nouvel essai…"),
        Event::AuthFailed { message } => eprintln!("authentification refusée : {message}"),
        Event::Rejected { from, reason, .. } => {
            eprintln!("⚠ message de {} rejeté : {reason}", names.device(from))
        }
        Event::GroupChanged { epoch } => eprintln!("groupe mis à jour (époque {epoch})"),
        Event::PairingCompleted { name, .. } => eprintln!("« {name} » a rejoint le groupe"),
        Event::KeyringError { message } => eprintln!("⚠ {message}"),
        Event::DirectoryChanged => {}
        Event::DirectoryRejected { user, reason } => {
            eprintln!("⚠ annuaire : {} refusé : {reason}", names.user(user))
        }
        Event::DirectoryFork => {
            eprintln!("⚠ le relais publie pour cet appareil un autre groupe que le sien")
        }
        Event::KeysChanged { user } => {
            eprintln!("⚠ {} a de nouvelles clés (réinvitée ?)", names.user(user))
        }
        Event::PersonJoined { name, device_name, fingerprint, .. } => {
            eprintln!("« {name} » a rejoint OldTown ({device_name}, empreinte {fingerprint})")
        }
        Event::Joined { user } => eprintln!("invitation acceptée : personne {user} certifiée"),
        Event::InviteFailed { reason } => eprintln!("⚠ invitation : {reason}"),
        Event::Read { .. } => {}
        Event::Message(m) => {
            print_message(m, names, json, offset)?;
            return Ok(true);
        }
        Event::File(f) => {
            print_file(&f.file, names, json, offset)?;
            return Ok(true);
        }
        Event::Otp(o) => {
            print_otp(o, names, json, offset)?;
            return Ok(true);
        }
    }
    Ok(false)
}

async fn refresh_names(client: &Client, event: &Event, names: &mut Names) {
    match event {
        Event::GroupChanged { .. } => {
            if let Ok(Ok(g)) = tokio::time::timeout(COMMAND_TIMEOUT, client.group()).await {
                names.group = g;
            }
        }
        Event::DirectoryChanged | Event::Connected { .. } => {
            if let Ok(Ok(p)) = tokio::time::timeout(COMMAND_TIMEOUT, client.directory()).await {
                names.people = p;
            }
        }
        _ => {}
    }
}

fn is_service(home: &Home) -> bool {
    home.load_config().ok().flatten().is_some_and(|c| c.role == Some(ot_core::Role::Service))
}

fn print_otp(o: &ReceivedOtp, names: &Names, json: bool, offset: UtcOffset) -> Result<()> {
    let from = names.device(&o.from);
    if json {
        let mut line = serde_json::json!({
            "msg_id": o.msg_id.to_string(),
            "from": o.from,
            "from_name": from,
            "created_at": o.created_at.format(&Rfc3339)?,
            "kind": "otp",
            "code": o.otp.code,
            "display": o.otp.display,
            "sender_name": o.otp.sender_name,
            "sender_domain": o.otp.sender_domain,
            "sender_verified": o.otp.sender_verified,
            "expires_at": o.otp.expires_at,
            "auto_fill": o.auto_fill(),
        });
        names.origin(&o.origin, &mut line);
        println!("{line}");
    } else {
        let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
        let when = o.created_at.to_offset(offset).format(&fmt)?;
        let verified = if o.otp.sender_verified { "" } else { ", expéditeur non vérifié" };
        let copy = names.copy(&o.origin);
        println!("[{when}] {copy}{from} 🔑 {} ({}{verified})", o.otp.display, o.otp.sender_domain);
    }
    Ok(())
}

fn print_file(f: &FileInfo, names: &Names, json: bool, offset: UtcOffset) -> Result<()> {
    let from = names.device(&f.from);
    if json {
        let mut line = serde_json::json!({
            "msg_id": f.msg_id.to_string(),
            "from": f.from,
            "from_name": from,
            "created_at": f.created_at.format(&Rfc3339)?,
            "kind": "file",
            "blob_id": f.blob_id,
            "name": f.name,
            "mime": f.mime,
            "size": f.size,
        });
        names.origin(&f.origin, &mut line);
        println!("{line}");
    } else {
        let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
        let when = f.created_at.to_offset(offset).format(&fmt)?;
        println!(
            "[{when}] {}{from} 📎 {} ({}) : ot download {}",
            names.copy(&f.origin),
            f.name,
            human_size(f.size),
            f.blob_id
        );
    }
    Ok(())
}

fn print_message(m: &Received, names: &Names, json: bool, offset: UtcOffset) -> Result<()> {
    let from = names.device(&m.from);
    if json {
        let mut line = serde_json::json!({
            "msg_id": m.msg_id.to_string(),
            "from": m.from,
            "from_name": from,
            "created_at": m.created_at.format(&Rfc3339)?,
            "kind": m.content.kind,
            "body": m.content.body,
        });
        names.origin(&m.origin, &mut line);
        println!("{line}");
    } else {
        let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
        let when = m.created_at.to_offset(offset).format(&fmt)?;
        let copy = names.copy(&m.origin);
        match m.content.kind {
            ContentKind::Link => println!("[{when}] {copy}{from} 🔗 {}", m.content.body),
            ContentKind::Text => println!("[{when}] {copy}{from} : {}", m.content.body),
        }
    }
    Ok(())
}

async fn listen(
    home: &Home,
    json: bool,
    download_dir: Option<PathBuf>,
    offset: UtcOffset,
) -> Result<()> {
    let (client, mut events) = start(home)?;
    let mut seen = Dedup::with_seen(Dedup::DEFAULT_CAPACITY, home.load_seen());
    let mut names =
        Names { group: ot_core::group_info(&home.keyring()).ok().flatten(), people: Vec::new() };
    if names.group.is_none() && !is_service(home) {
        eprintln!("⚠ {}", ot_core::ClientError::NoGroup);
    }

    loop {
        let event = tokio::select! {
            ev = events.next() => ev,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(event) = event else { break };
        refresh_names(&client, &event, &mut names).await;
        if let (Event::File(f), Some(dir)) = (&event, &download_dir) {
            let (client, dir, blob) = (client.clone(), dir.clone(), f.file.blob_id.clone());
            tokio::spawn(async move {
                match fetch(&client, &blob, &dir).await {
                    Ok(path) => eprintln!("\rfichier enregistré : {}", path.display()),
                    Err(e) => eprintln!("\r⚠ fichier {blob} non téléchargé : {e:#}"),
                }
            });
        }
        let msg_id = match &event {
            Event::Message(m) => Some(m.msg_id),
            Event::File(f) => Some(f.file.msg_id),
            Event::Otp(o) => Some(o.msg_id),
            _ => None,
        };
        if print_event(&event, &names, json, offset)?
            && let Some(msg_id) = msg_id
        {
            seen.insert(msg_id);
            if let Err(e) = home.save_seen(seen.iter()) {
                eprintln!("état non sauvegardé : {e:#}");
            }
        }
    }
    client.shutdown().await;
    Ok(())
}

const OTP_SOCKET_MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Clone)]
enum OtpTarget {
    Devices(Vec<String>),
    People(Vec<String>),
}

async fn otp_bridge(
    home: &Home,
    to: Vec<String>,
    to_user: Vec<String>,
    socket: Option<PathBuf>,
    min_confidence: f64,
) -> Result<()> {
    if cfg!(not(unix)) {
        bail!("otp-bridge lit un socket Unix : il ne tourne pas sous Windows");
    }
    if !(0.0..=1.0).contains(&min_confidence) {
        bail!("--min-confidence doit être compris entre 0 et 1");
    }
    let clean = |names: Vec<String>| -> Vec<String> {
        names.iter().map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).collect()
    };
    let target = match (clean(to), clean(to_user)) {
        (to, users) if !to.is_empty() && !users.is_empty() => {
            bail!("--to (appareils de mon groupe) et --to-user (personnes) ne se combinent pas")
        }
        (to, _) if !to.is_empty() => OtpTarget::Devices(to),
        (_, users) if !users.is_empty() => OtpTarget::People(users),
        _ => bail!("aucun destinataire : --to ou --to-user (ou OT_OTP_TO, OT_OTP_TO_USER)"),
    };
    let socket = match socket {
        Some(path) => path,
        None => otp::default_socket()?,
    };
    if let OtpTarget::Devices(to) = &target {
        let group = group(home)?;
        for name in to {
            resolve(&group, name)?;
        }
    }

    let (client, mut events) = start(home)?;
    let names = match &target {
        OtpTarget::Devices(_) => Vec::new(),
        OtpTarget::People(names) => names.clone(),
    };
    if !names.is_empty() {
        match tokio::time::timeout(COMMAND_TIMEOUT, client.fresh_directory()).await {
            Ok(Ok(people)) => {
                if people.is_empty() {
                    client.shutdown().await;
                    bail!(
                        "pas d'annuaire sur cet appareil : --to-user demande une installation \
                         à plusieurs personnes (voir `ot pair join --qr --service`)"
                    );
                }
                for name in &names {
                    if let Err(e) = people::find_person(&people, name) {
                        client.shutdown().await;
                        return Err(e);
                    }
                }
            }
            Ok(Err(e)) => eprintln!("⚠ annuaire illisible : {e}"),
            Err(_) => eprintln!("⚠ relais injoignable : les noms seront vérifiés au premier code"),
        }
    }
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                Event::Connected { .. } => eprintln!("relais : connecté"),
                Event::Disconnected { reason } => {
                    eprintln!("relais : déconnecté ({reason}) ; nouvel essai…")
                }
                Event::AuthFailed { message } => {
                    eprintln!("relais : authentification refusée : {message}")
                }
                Event::KeyringError { message } => eprintln!("⚠ {message}"),
                _ => {}
            }
        }
    });

    let mut sent = home.load_otp_sent();
    let mut backoff = Duration::from_secs(1);
    loop {
        let session =
            otp_session(home, &client, &socket, &target, min_confidence, &mut sent, &mut backoff);
        let result = tokio::select! {
            r = session => r,
            _ = tokio::signal::ctrl_c() => break,
        };
        match result {
            Ok(()) => eprintln!("otp-relay : connexion fermée"),
            Err(e) => eprintln!("otp-relay : {e:#}"),
        }
        eprintln!("otp-relay : nouvel essai dans {} s", backoff.as_secs());
        tokio::select! {
            () = tokio::time::sleep(backoff) => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        backoff = (backoff * 2).min(OTP_SOCKET_MAX_BACKOFF);
    }
    client.shutdown().await;
    Ok(())
}

#[cfg(unix)]
async fn otp_connect(socket: &std::path::Path) -> Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(socket)
        .await
        .with_context(|| format!("socket {} injoignable", socket.display()))
}

#[cfg(not(unix))]
async fn otp_connect(socket: &std::path::Path) -> Result<tokio::io::Empty> {
    bail!("socket {} : pas de socket Unix sous Windows", socket.display())
}

async fn otp_session(
    home: &Home,
    client: &Client,
    socket: &std::path::Path,
    target: &OtpTarget,
    min_confidence: f64,
    sent: &mut otp::Sent,
    backoff: &mut Duration,
) -> Result<()> {
    use tokio::io::AsyncBufReadExt;

    let stream = otp_connect(socket).await?;
    *backoff = Duration::from_secs(1);
    let mut lines = tokio::io::BufReader::new(stream).lines();
    while let Some(line) = lines.next_line().await? {
        let ev = match otp::Line::parse(&line) {
            Err(e) => {
                eprintln!("otp-relay : ligne illisible (colonne {})", e.column());
                continue;
            }
            Ok(otp::Line::Hello { protocol }) => {
                if protocol != otp::PROTOCOL {
                    eprintln!(
                        "⚠ otp-relay parle le protocole {protocol}, ce pont attend le {}",
                        otp::PROTOCOL
                    );
                }
                continue;
            }
            Ok(otp::Line::Status { state }) => {
                eprintln!("otp-relay : IMAP {state}");
                continue;
            }
            Ok(otp::Line::Other) => continue,
            Ok(otp::Line::Otp(ev)) => ev,
        };
        let now = OffsetDateTime::now_utc();
        sent.prune(now);
        let code = match otp::judge(&ev, now, min_confidence, sent) {
            otp::Verdict::Send(code) => code,
            otp::Verdict::AlreadySent => continue,
            otp::Verdict::Skip(why) => {
                eprintln!("code de {} ignoré : {why}", ev.domain());
                continue;
            }
        };
        sent.insert(ev.id.clone(), code.expires_at);
        if let Err(e) = home.save_otp_sent(sent) {
            eprintln!("⚠ codes envoyés non sauvegardés : {e:#}");
        }
        tokio::spawn(send_otp(client.clone(), target.clone(), code));
    }
    Ok(())
}

async fn send_otp(client: Client, target: OtpTarget, code: ot_core::Otp) {
    let domain = code.sender_domain.clone();
    let left = code.expires_at - OffsetDateTime::now_utc().unix_timestamp();
    let send = async {
        match &target {
            OtpTarget::Devices(to) => {
                let group = client.group().await?.ok_or(ot_core::ClientError::NoGroup)?;
                let recipients =
                    to.iter().map(|name| resolve(&group, name)).collect::<Result<_>>()?;
                client.send_otp(&code, recipients).await?;
            }
            OtpTarget::People(names) => {
                let people = client.fresh_directory().await?;
                let users = people::otp_recipients(&people, names);
                if users.is_empty() {
                    bail!("aucune des personnes de --to-user n'est joignable");
                }
                client.send_otp_to(&users, &code).await?;
            }
        }
        anyhow::Ok(())
    };
    let wait = Duration::from_secs(u64::try_from(left).unwrap_or(0).max(1));
    match tokio::time::timeout(wait, send).await {
        Ok(Ok(())) => eprintln!("code de {domain} envoyé"),
        Ok(Err(e)) => eprintln!("⚠ code de {domain} non envoyé : {e:#}"),
        Err(_) => eprintln!("⚠ code de {domain} expiré avant d'atteindre le relais"),
    }
}

async fn devices(home: &Home) -> Result<()> {
    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let me = client.device_id().clone();
    let list = within(client.list_devices()).await;
    let people = tokio::time::timeout(COMMAND_TIMEOUT, client.fresh_directory()).await;
    client.shutdown().await;
    let list = list?;
    let people = people.ok().and_then(Result::ok).unwrap_or_default();
    let person = people.iter().find(|p| p.roster.contains(&me));
    let group = ot_core::group_info(&home.keyring())?;

    if let Some(p) = person {
        println!("Personne : {} ({}, {})", p.name, people::role_name(p.role), p.id);
    }
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
        let profile = person
            .and_then(|p| p.devices.iter().find(|x| x.id == d.id))
            .map(|x| format!("  {}", people::profile(x)))
            .unwrap_or_default();
        println!("{:<16} {}  {state}{profile}{member}", d.name, d.id);
    }
    let Some(group) = group else {
        if !is_service(home) {
            eprintln!("⚠ {}", ot_core::ClientError::NoGroup);
        }
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

fn print_qr(text: &str) -> Result<()> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(text.as_bytes()).context("texte trop long pour un QR")?;
    let image = code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!("{image}");
    Ok(())
}

async fn pair_new(home: &Home, qr: bool, offset: UtcOffset) -> Result<()> {
    let mut names = Names { group: Some(group(home)?), people: Vec::new() };
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
    if qr {
        let uri = ot_core::PairUri {
            server_url: config.server_url.clone(),
            code: invitation.code.clone(),
        };
        println!();
        println!("Ou scanner ce QR avec l'appli Android (« Rejoindre un groupe ») :");
        print_qr(&uri.encode())?;
    }

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
        refresh_names(&client, &event, &mut names).await;
        if print_event(&event, &names, false, offset)?
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
    client.shutdown().await;
    result
}

async fn pair_join_qr(
    home: &Home,
    server: String,
    name: String,
    force: bool,
    service: bool,
) -> Result<()> {
    if let Some(Config { device_id: Some(id), .. }) = home.load_config()?
        && !force
    {
        bail!("cet appareil est déjà enregistré ({id}) ; --force pour repartir de zéro");
    }
    let name = name.trim().to_owned();
    if !ot_core::valid_device_name(&name) {
        bail!("nom invalide : 1 à 64 caractères imprimables");
    }
    let identity = Identity::generate();
    let device_id = DeviceId::generate();
    home.reset_group()?;
    home.save_key(&identity)?;
    home.save_config(&Config {
        server_url: server,
        device_name: name.clone(),
        device_id: Some(device_id.clone()),
        role: service.then_some(ot_core::Role::Service),
    })?;

    let (config, _) = registered(home)?;
    let mut client =
        ClientConfig::new(config.server_url, device_id, home.load_key()?, Arc::new(home.keyring()));
    client.backoff =
        ot_core::Backoff { initial: Duration::from_secs(1), max: Duration::from_secs(3) };
    let (client, mut events) = Client::start(client);
    let offer = within(client.pair_offer(&name)).await?;

    if service {
        print_qr(&offer)?;
        println!("Sur un appareil de l'admin :");
        println!("  ot admin service add <service> --qr '<texte ci-dessous>'");
        println!("  {offer}");
        println!();
        println!("En attente de l'admin (10 minutes au plus)…");
        let res = tokio::time::timeout(PAIRING_TIMEOUT, service_received(&mut events)).await;
        client.shutdown().await;
        let user = res.map_err(|_| {
            anyhow!("aucun admin en 10 minutes : relancer `ot pair join --qr --service --force`")
        })??;
        println!("Appareil de service « {name} » rangé (personne {user}).");
        println!("Il écrit aux personnes liées au service : ot otp-bridge --to-user <personne>");
        return Ok(());
    }

    println!("Sur le téléphone : OldTown → Groupe → « Ajouter un appareil », puis scanner :");
    print_qr(&offer)?;
    println!("Sans téléphone, sur un membre du groupe : ot pair sponsor '<texte ci-dessous>'");
    println!("  {offer}");
    println!();
    println!("En attente du parrain (10 minutes au plus)…");

    let res = tokio::time::timeout(PAIRING_TIMEOUT, group_received(&mut events)).await;
    client.shutdown().await;
    let epoch = res.map_err(|_| {
        anyhow!("aucun parrain en 10 minutes : relancer `ot pair join --qr --force`")
    })??;
    println!("Groupe rejoint (époque {epoch}).");
    group_show(home)
}

async fn group_received(events: &mut Events) -> Result<u32> {
    loop {
        match events.next().await {
            Some(Event::GroupChanged { epoch }) => return Ok(epoch),
            Some(Event::Rejected { reason, .. }) => eprintln!("⚠ message rejeté : {reason}"),
            Some(Event::KeyringError { message }) => eprintln!("⚠ {message}"),
            Some(_) => {}
            None => bail!("client arrêté"),
        }
    }
}

async fn service_received(events: &mut Events) -> Result<ot_core::UserId> {
    loop {
        match events.next().await {
            Some(Event::Joined { user }) => return Ok(user),
            Some(Event::InviteFailed { reason }) => {
                bail!("bienvenue de l'admin refusée : {reason}")
            }
            Some(Event::Rejected { reason, .. }) => eprintln!("⚠ message rejeté : {reason}"),
            Some(Event::DirectoryRejected { reason, .. }) => eprintln!("⚠ annuaire : {reason}"),
            Some(Event::KeyringError { message }) => eprintln!("⚠ {message}"),
            Some(_) => {}
            None => bail!("client arrêté"),
        }
    }
}

async fn pair_sponsor(home: &Home, qr: &str) -> Result<()> {
    group(home)?;
    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    let res = within(client.sponsor(qr)).await;
    client.shutdown().await;
    let (id, name) = res?;
    println!("« {name} » ({id}) a rejoint le groupe.");
    Ok(())
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
    let redeemed =
        ot_core::redeem_pairing(&server, &TlsRoots::default(), &identity, &name, &code).await?;

    home.reset_group()?;
    home.save_key(&identity)?;
    home.save_config(&Config {
        server_url: server,
        device_name: name.clone(),
        device_id: Some(redeemed.device_id.clone()),
        role: None,
    })?;
    println!(
        "Appareil « {name} » enregistré ({}). Demande envoyée au parrain…",
        redeemed.device_id
    );

    let (client, mut events) = start(home)?;
    connected(&mut events).await?;
    within(client.pair_join(&code, redeemed.sponsor, &name)).await?;
    let res = tokio::time::timeout(PAIRING_TIMEOUT, group_received(&mut events)).await;
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
