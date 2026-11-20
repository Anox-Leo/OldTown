//! Commandes de l'étape 10 : personnes, invitation, administration,
//! services. Les noms se résolvent dans l'annuaire vérifié (personnes que je
//! peux joindre) ou, pour l'admin, dans la liste du relais (`users`) ; un
//! nom ambigu est refusé avec la liste des choix.

use std::io::{IsTerminal, Read};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ot_core::proto::{AdminUser, DeviceKind, UserState};
use ot_core::{
    Client, DeviceId, DeviceProfile, DeviceSettings, Event, Events, Identity, InviteCode,
    InviteUri, OutgoingFile, Person, Role, Target, TlsRoots, UserId,
};
use time::UtcOffset;
use time::macros::format_description;

use crate::files::{Config, Home};
use crate::transfer::{FileSource, guess_mime, human_size, source_id};
use crate::{
    AdminCommand, COMMAND_TIMEOUT, LinkCommand, OnOff, ServiceCommand, connected, print_qr,
    progress_bar, registered, start, within,
};

/// Attente de la certification par l'admin d'une invitation rachetée.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Member => "membre",
        Role::Service => "service",
    }
}

fn kind_name(kind: Option<DeviceKind>) -> &'static str {
    match kind {
        Some(DeviceKind::Phone) => "téléphone",
        Some(DeviceKind::Laptop) => "portable",
        Some(DeviceKind::Desktop) => "ordinateur",
        Some(DeviceKind::Browser) => "navigateur",
        Some(DeviceKind::Cli) => "ligne de commande",
        Some(DeviceKind::Unknown) | None => "appareil",
    }
}

/// Profil d'un appareil, en clair : nature, par défaut, joignable, à jour.
pub fn profile(d: &DeviceProfile) -> String {
    let mut parts = vec![kind_name(d.kind)];
    if d.default {
        parts.push("par défaut");
    }
    if !d.reachable {
        parts.push("injoignable");
    }
    if !d.up_to_date {
        parts.push("pas à jour");
    }
    parts.push(if d.online { "en ligne" } else { "hors ligne" });
    parts.join(", ")
}

/// Un seul élément de `items` désigné par `name` (identifiant, sinon nom) ;
/// sinon une erreur qui liste les choix.
fn pick<'a, T>(
    items: &'a [T],
    name: &str,
    what: &str,
    id: impl Fn(&T) -> String,
    label: impl Fn(&T) -> String,
) -> Result<&'a T> {
    let name = name.trim();
    if let Some(item) = items.iter().find(|i| id(i) == name) {
        return Ok(item);
    }
    let found: Vec<&T> = items.iter().filter(|i| label(i) == name).collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => {
            let names: Vec<String> = items.iter().map(&label).collect();
            if names.is_empty() {
                bail!("{what} introuvable : {name}")
            }
            bail!("{what} introuvable : {name} (choix : {})", names.join(", "))
        }
        many => {
            let ids: Vec<String> = many.iter().map(|i| id(i)).collect();
            bail!("{what} : nom ambigu, {name} ; préciser l'identifiant ({})", ids.join(", "))
        }
    }
}

/// Personne de l'annuaire vérifié désignée par son nom ou son identifiant.
pub fn find_person<'a>(people: &'a [Person], name: &str) -> Result<&'a Person> {
    pick(people, name, "personne", |p| p.id.to_string(), |p| p.name.clone())
}

/// Personne connue du relais (admin), hors révoquées.
fn find_user<'a>(users: &'a [AdminUser], name: &str) -> Result<&'a AdminUser> {
    let live: Vec<AdminUser> =
        users.iter().filter(|u| u.state != UserState::Revoked).cloned().collect();
    let found = pick(&live, name, "personne", |u| u.id.to_string(), |u| u.name.clone())?;
    users.iter().find(|u| u.id == found.id).context("personne disparue")
}

/// Personnes de `--to-user` à qui un code peut partir : celles qui ont coupé
/// le service, absentes de l'annuaire ou sans appareil joignable et à jour
/// sont sautées, et signalées (par identifiant, jamais par le code).
pub fn otp_recipients(people: &[Person], names: &[String]) -> Vec<UserId> {
    let mut users = Vec::new();
    for name in names {
        match find_person(people, name) {
            Ok(p) if p.muted => eprintln!("{} a coupé ce service : sautée", p.id),
            Ok(p) if !p.devices.iter().any(|d| d.reachable && d.up_to_date) => {
                eprintln!("{} n'a aucun appareil joignable et à jour : sautée", p.id)
            }
            Ok(p) if !users.contains(&p.id) => users.push(p.id.clone()),
            Ok(_) => {}
            Err(e) => eprintln!("{e} : sautée"),
        }
    }
    users
}

/// Ma personne dans l'annuaire : celle dont le roster porte cet appareil.
fn me<'a>(people: &'a [Person], device: &DeviceId) -> Option<&'a Person> {
    people.iter().find(|p| p.roster.contains(device))
}

/// Annuaire vérifié frais, ou une erreur qui dit pourquoi il est vide.
async fn directory(client: &Client) -> Result<Vec<Person>> {
    let people = within(client.fresh_directory()).await?;
    if people.is_empty() {
        bail!(
            "annuaire vide : cet appareil n'a pas d'annuaire (installation à une personne, ou \
             `ot admin init` pas encore fait sur l'appareil de l'admin)"
        );
    }
    Ok(people)
}

/// Destinataire `--to-user` (et `--device`) d'un envoi.
async fn target(
    client: &Client,
    user: &str,
    device: Option<&str>,
) -> Result<(UserId, Option<DeviceId>)> {
    let people = directory(client).await?;
    let person = find_person(&people, user)?;
    let device = match device {
        Some(name) => Some(
            pick(&person.devices, name, "appareil", |d| d.id.to_string(), |d| d.name.clone())?
                .id
                .clone(),
        ),
        None => None,
    };
    Ok((person.id.clone(), device))
}

pub async fn send_to_user(
    home: &Home,
    text: Vec<String>,
    user: &str,
    device: Option<&str>,
) -> Result<()> {
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
    let (client, mut events) = start(home)?;
    let res = async {
        connected(&mut events).await?;
        let (user, device) = target(&client, user, device).await?;
        within(client.send_to(Target::Person { user, device }, &content)).await
    }
    .await;
    client.shutdown().await;
    eprintln!("envoyé ({})", res?.msg_id);
    Ok(())
}

pub async fn send_file_to_user(
    home: &Home,
    path: &Path,
    user: &str,
    device: Option<&str>,
) -> Result<()> {
    let source = Arc::new(FileSource::open(path)?);
    let name =
        path.file_name().and_then(|n| n.to_str()).context("nom de fichier illisible")?.to_owned();
    let size = ot_core::ChunkSource::size(&*source);
    let (client, mut events) = start(home)?;
    let res = async {
        connected(&mut events).await?;
        let (user, device) = target(&client, user, device).await?;
        let file = OutgoingFile {
            source_id: source_id(path)?,
            name: name.clone(),
            mime: guess_mime(path).to_owned(),
            to: device.into_iter().collect(),
            to_user: Some(user),
        };
        eprintln!("envoi de {name} ({})…", human_size(size));
        client.send_file(file, source, progress_bar()).await.map_err(|e| {
            anyhow!("{e}\nRelancer la même commande reprend l'envoi là où il s'est arrêté.")
        })
    }
    .await;
    client.shutdown().await;
    eprintln!("\rfichier envoyé ({})", res?.msg_id);
    Ok(())
}

pub async fn users(home: &Home) -> Result<()> {
    let (_, me_id) = registered(home)?;
    let (client, mut events) = start(home)?;
    let people = async {
        connected(&mut events).await?;
        directory(&client).await
    }
    .await;
    client.shutdown().await;
    let people = people?;
    let mine = me(&people, &me_id).map(|p| p.id.clone());
    for p in &people {
        let role = match p.role {
            Role::Member => String::new(),
            r => format!(" ({})", role_name(r)),
        };
        let you = if Some(&p.id) == mine.as_ref() { "  (moi)" } else { "" };
        let muted = if p.muted { "  coupé" } else { "" };
        println!("{}{role}  {}{you}{muted}", p.name, p.id);
        for d in &p.devices {
            let this = if d.id == me_id { "  (cet appareil)" } else { "" };
            println!("  {:<16} {}  {}{this}", d.name, d.id, profile(d));
        }
    }
    Ok(())
}

pub async fn mute(home: &Home, service: &str, state: OnOff) -> Result<()> {
    let muted = matches!(state, OnOff::On);
    let (client, mut events) = start(home)?;
    let res = async {
        connected(&mut events).await?;
        let people = directory(&client).await?;
        let p = find_person(&people, service)?;
        if p.role != Role::Service {
            bail!("{} n'est pas un service", p.name);
        }
        within(client.mute_service(&p.id, muted)).await?;
        anyhow::Ok(p.name.clone())
    }
    .await;
    client.shutdown().await;
    let name = res?;
    if muted {
        println!("{name} coupé : ses envois ne vous arrivent plus.");
    } else {
        println!("{name} rouvert.");
    }
    Ok(())
}

/// Appareil désigné pour `ot device …` : de mon groupe d'abord, puis (pour
/// l'admin) de n'importe quelle personne connue du relais.
async fn find_device(client: &Client, name: &str) -> Result<DeviceId> {
    let members = within(client.group()).await?.map(|g| g.members).unwrap_or_default();
    let mine = pick(&members, name, "appareil", |m| m.device_id.to_string(), |m| m.name.clone());
    if let Ok(m) = &mine {
        return Ok(m.device_id.clone());
    }
    // L'admin règle aussi les appareils des autres personnes ; un membre
    // n'a que les siens.
    let Ok(users) = within(client.users()).await else {
        return mine.map(|m| m.device_id.clone());
    };
    let all: Vec<(String, DeviceId)> = users
        .iter()
        .filter(|u| u.state != UserState::Revoked)
        .flat_map(|u| u.devices.iter().map(|d| (d.name.clone(), d.id.clone())))
        .collect();
    let found = pick(&all, name, "appareil", |d| d.1.to_string(), |d| d.0.clone())?;
    Ok(found.1.clone())
}

async fn update(home: &Home, device: &str, settings: DeviceSettings) -> Result<()> {
    let (client, mut events) = start(home)?;
    let res = async {
        connected(&mut events).await?;
        let id = find_device(&client, device).await?;
        within(client.update_device(&id, settings)).await?;
        anyhow::Ok(id)
    }
    .await;
    client.shutdown().await;
    eprintln!("profil de {} mis à jour", res?);
    Ok(())
}

pub async fn device_kind(home: &Home, device: &str, kind: &str) -> Result<()> {
    let kind =
        kind.parse::<DeviceKind>().ok().filter(|k| *k != DeviceKind::Unknown).ok_or_else(|| {
            anyhow!("nature inconnue : {kind} (phone, laptop, desktop, browser, cli)")
        })?;
    update(home, device, DeviceSettings { kind: Some(kind), ..DeviceSettings::default() }).await
}

pub enum Flag {
    Default,
    Reachable,
}

pub async fn device_flag(home: &Home, device: &str, flag: Flag, state: OnOff) -> Result<()> {
    let on = matches!(state, OnOff::On);
    let settings = match flag {
        Flag::Default => DeviceSettings { default: Some(on), ..DeviceSettings::default() },
        Flag::Reachable => DeviceSettings { reachable: Some(on), ..DeviceSettings::default() },
    };
    update(home, device, settings).await
}

pub async fn device_revoke(home: &Home, device: &str) -> Result<()> {
    let (client, mut events) = start(home)?;
    let res = async {
        connected(&mut events).await?;
        let id = find_device(&client, device).await?;
        if &id == client.device_id() {
            bail!("impossible de retirer cet appareil depuis lui-même");
        }
        // Rotation, publication, révocation, re-certification : plusieurs
        // allers-retours.
        tokio::time::timeout(COMMAND_TIMEOUT * 4, client.remove_device(&id))
            .await
            .context("pas de réponse du relais")??;
        anyhow::Ok(id)
    }
    .await;
    client.shutdown().await;
    let id = res?;
    println!("{id} retiré : il ne lit plus les messages suivants et le relais l'a révoqué.");
    Ok(())
}

pub async fn invite_join(
    home: &Home,
    code: &str,
    name: &str,
    server: Option<String>,
    force: bool,
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
    let (server, code) = if code.trim().starts_with(ot_core::INVITE_URI_PREFIX) {
        let uri = InviteUri::parse(code)?;
        (server.unwrap_or(uri.server_url), uri.code)
    } else {
        let server = server.context("--server est requis avec un code ot2-… (pas avec un lien)")?;
        (server, code.trim().to_owned())
    };
    let parsed = InviteCode::parse(&code).map_err(|e| anyhow!("code d'invitation : {e}"))?;
    let identity = Identity::generate();
    let redeemed =
        ot_core::redeem_invite(&server, &TlsRoots::default(), &identity, &name, &parsed).await?;

    home.reset_group()?;
    home.save_key(&identity)?;
    home.save_config(&Config {
        server_url: server,
        device_name: name.clone(),
        device_id: Some(redeemed.device_id.clone()),
        role: None,
    })?;
    ot_core::ensure_exchange_key(&home.keyring())?;
    ot_core::save_pending_invite(&home.keyring(), &parsed, &redeemed, &name)?;
    println!("Appareil « {name} » enregistré ({}).", redeemed.device_id);
    println!(
        "Empreinte de cet appareil : {} (à comparer avec celle que l'admin verra).",
        ot_core::crypto::fingerprint(&identity.public_key())
    );
    println!("Demande envoyée à l'admin ; en attente de sa certification…");

    let (client, mut events) = start(home)?;
    let joined = tokio::time::timeout(JOIN_TIMEOUT, joined(&mut events)).await;
    let res = match joined {
        Ok(Ok(user)) => {
            // L'entrée 1 de mon journal part juste après : l'attendre, pour
            // être visible des autres dès maintenant.
            let me_id = redeemed.device_id.clone();
            let visible = async {
                loop {
                    let people = client.directory().await.unwrap_or_default();
                    if me(&people, &me_id).is_some() {
                        return;
                    }
                    if !wait_directory(&mut events).await {
                        return;
                    }
                }
            };
            if tokio::time::timeout(COMMAND_TIMEOUT, visible).await.is_err() {
                eprintln!("⚠ mon journal sera publié au prochain lancement");
            }
            Ok(user)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!(
            "pas encore de réponse de l'admin : l'invitation reste en attente ; un prochain \
             `ot listen` ici la terminera"
        )),
    };
    client.shutdown().await;
    let user = res?;
    println!("Personne certifiée ({user}) : bienvenue.");
    println!("Voir qui je peux joindre : ot users");
    Ok(())
}

/// Attend `Joined` (invitation acceptée), ou un échec.
async fn joined(events: &mut Events) -> Result<UserId> {
    loop {
        match events.next().await {
            Some(Event::Joined { user }) => return Ok(user),
            Some(Event::InviteFailed { reason }) => bail!("invitation refusée : {reason}"),
            Some(Event::AuthFailed { message }) => {
                eprintln!("authentification refusée : {message}")
            }
            Some(Event::DirectoryRejected { user, reason }) => {
                eprintln!("⚠ annuaire : {user} refusé : {reason}")
            }
            Some(Event::KeyringError { message }) => eprintln!("⚠ {message}"),
            Some(_) => {}
            None => bail!("client arrêté"),
        }
    }
}

/// Attend le prochain `DirectoryChanged` ; `false` si le client s'arrête.
async fn wait_directory(events: &mut Events) -> bool {
    loop {
        match events.next().await {
            Some(Event::DirectoryChanged) => return true,
            Some(_) => {}
            None => return false,
        }
    }
}

pub async fn admin(home: &Home, cmd: AdminCommand, offset: UtcOffset) -> Result<()> {
    let (client, mut events) = start(home)?;
    let res = async {
        connected(&mut events).await?;
        match cmd {
            AdminCommand::Init => admin_init(&client).await,
            AdminCommand::Invite { name, links, qr } => {
                invite(&client, &mut events, Some((&name, links)), None, qr, offset).await
            }
            AdminCommand::Reinvite { person, qr } => {
                invite(&client, &mut events, None, Some(&person), qr, offset).await
            }
            AdminCommand::Users => admin_users(&client).await,
            AdminCommand::Link(LinkCommand::Add { from, to }) => {
                link(&client, &from, &to, true).await
            }
            AdminCommand::Link(LinkCommand::Remove { from, to }) => {
                link(&client, &from, &to, false).await
            }
            AdminCommand::Revoke { person } => {
                let users = within(client.users()).await?;
                let u = find_user(&users, &person)?;
                within(client.revoke_user(&u.id)).await?;
                println!("{} révoquée : appareils, liens et certificat retirés.", u.name);
                Ok(())
            }
            AdminCommand::Rename { person, name } => {
                let users = within(client.users()).await?;
                let u = find_user(&users, &person)?;
                within(client.rename_user(&u.id, &name)).await?;
                println!("{} s'appelle maintenant {}.", u.name, name.trim());
                Ok(())
            }
            AdminCommand::Service(ServiceCommand::Add { service, qr }) => {
                // Enregistrement, deux publications et la bienvenue.
                let added = tokio::time::timeout(
                    COMMAND_TIMEOUT * 4,
                    client.add_service_device(&service, &qr),
                )
                .await
                .context("pas de réponse du relais")??;
                if added.created {
                    println!("Service « {} » créé ({}).", service.trim(), added.user);
                }
                println!(
                    "Appareil « {} » ({}) ajouté au service ; liens à poser : \
                     ot admin link add '{}' <personne>",
                    added.device_name,
                    added.device_id,
                    service.trim()
                );
                Ok(())
            }
        }
    }
    .await;
    // Les dernières trames (bienvenue, publications) sont déjà écrites : le
    // relais les lit avant la fermeture.
    client.shutdown().await;
    res
}

async fn admin_init(client: &Client) -> Result<()> {
    let admin = within(client.admin_init()).await.map_err(|e| {
        anyhow!("{e}\nSeul un appareil du groupe de l'admin (la personne migrée) le peut.")
    })?;
    println!("Annuaire ouvert : personne admin {admin}.");
    println!("Inviter quelqu'un : ot admin invite <nom> --link <personne>");
    Ok(())
}

/// Invitation (ou réinvitation) : affiche le code, puis attend la demande
/// de la personne et sa certification, menées par ce client.
async fn invite(
    client: &Client,
    events: &mut Events,
    new: Option<(&str, Vec<String>)>,
    again: Option<&str>,
    qr: bool,
    offset: UtcOffset,
) -> Result<()> {
    let users = within(client.users()).await?;
    let invitation = match (new, again) {
        (Some((name, links)), _) => {
            let links = links
                .iter()
                .map(|l| find_user(&users, l).map(|u| u.id.clone()))
                .collect::<Result<Vec<_>>>()?;
            within(client.invite(name, links)).await?
        }
        (None, Some(person)) => {
            let u = find_user(&users, person)?;
            within(client.reinvite(&u.id)).await?
        }
        (None, None) => bail!("rien à inviter"),
    };
    let until = invitation
        .expires_at
        .to_offset(offset)
        .format(&format_description!("[year]-[month]-[day] [hour]:[minute]"))?;
    println!("Invitation valable jusqu'au {until}, une seule fois. Elle contient un secret : la");
    println!("transmettre à cette personne seule, par un canal de confiance.");
    println!();
    println!("  {}", invitation.uri);
    println!();
    println!("Sur son ordinateur :");
    println!("  ot invite join '{}' --name <nom de l'appareil>", invitation.uri);
    if qr {
        println!();
        println!("Ou ce QR :");
        print_qr(&invitation.uri)?;
    }
    println!();
    println!("En attente de sa demande (Ctrl-C : l'invitation reste valable, un prochain");
    println!("`ot listen` ici la certifiera)…");

    let wait = async {
        loop {
            match events.next().await {
                Some(Event::PersonJoined { user, name, device_name, fingerprint })
                    if user == invitation.user =>
                {
                    return Ok((name, device_name, fingerprint));
                }
                Some(Event::InviteFailed { reason }) => eprintln!("⚠ invitation : {reason}"),
                Some(Event::Disconnected { reason }) => {
                    eprintln!("déconnecté : {reason} ; nouvel essai…")
                }
                Some(_) => {}
                None => bail!("client arrêté"),
            }
        }
    };
    let (name, device_name, fingerprint) = tokio::select! {
        r = wait => r?,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("attente interrompue : l'invitation reste valable");
            return Ok(());
        }
    };
    println!("« {name} » a rejoint OldTown avec « {device_name} ».");
    println!("Empreinte de son appareil : {fingerprint} (à comparer de vive voix).");
    Ok(())
}

async fn admin_users(client: &Client) -> Result<()> {
    let users = within(client.users()).await?;
    let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]");
    let name_of = |id: &UserId| {
        users.iter().find(|u| &u.id == id).map_or_else(|| id.to_string(), |u| u.name.clone())
    };
    for u in &users {
        let state = match u.state {
            UserState::Active => "",
            UserState::Pending => "  en attente",
            UserState::Revoked => "  révoquée",
        };
        println!("{} ({})  {}{state}", u.name, role_name(u.role), u.id);
        for d in &u.devices {
            let seen = if d.online {
                "en ligne".to_owned()
            } else {
                d.last_seen_at
                    .and_then(|ms| {
                        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
                            .ok()
                    })
                    .and_then(|t| t.format(&fmt).ok())
                    .map_or_else(|| "hors ligne".to_owned(), |t| format!("vu le {t}"))
            };
            let mut flags = vec![kind_name(d.kind)];
            if d.default {
                flags.push("par défaut");
            }
            if !d.reachable {
                flags.push("injoignable");
            }
            println!("  {:<16} {}  {}, {seen}", d.name, d.id, flags.join(", "));
        }
        if !u.links.is_empty() {
            let links: Vec<String> = u.links.iter().map(&name_of).collect();
            println!("  → {}", links.join(", "));
        }
        if !u.muted.is_empty() {
            let muted: Vec<String> = u.muted.iter().map(&name_of).collect();
            println!("  coupé : {}", muted.join(", "));
        }
    }
    Ok(())
}

async fn link(client: &Client, from: &str, to: &str, allowed: bool) -> Result<()> {
    let users = within(client.users()).await?;
    let (a, b) = (find_user(&users, from)?, find_user(&users, to)?);
    within(client.link_set(&a.id, &b.id, allowed)).await?;
    let arrow = if a.role == Role::Service || b.role == Role::Service { "→" } else { "↔" };
    let what = if allowed { "posé" } else { "retiré" };
    println!("Lien {} {arrow} {} {what}.", a.name, b.name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(id, n)| ((*id).to_owned(), (*n).to_owned())).collect()
    }

    fn find<'a>(items: &'a [(String, String)], name: &str) -> Result<&'a (String, String)> {
        pick(items, name, "personne", |i| i.0.clone(), |i| i.1.clone())
    }

    #[test]
    fn names_resolve_by_id_then_unique_name() {
        let items = names(&[("usr_1", "Bob"), ("usr_2", "Carol"), ("usr_3", "Carol")]);
        assert_eq!(find(&items, "Bob").unwrap().0, "usr_1");
        assert_eq!(find(&items, " usr_3 ").unwrap().0, "usr_3");
        let err = find(&items, "Carol").unwrap_err().to_string();
        assert!(err.contains("nom ambigu") && err.contains("usr_2, usr_3"), "{err}");
        let err = find(&items, "Dave").unwrap_err().to_string();
        assert!(err.contains("introuvable") && err.contains("Bob, Carol, Carol"), "{err}");
        assert!(find(&[], "Dave").unwrap_err().to_string().ends_with("introuvable : Dave"));
    }
}
