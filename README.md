# OldTown

Échange de textes et de liens entre mes appareils, via un relais auto-hébergé
sur le laptop Ubuntu, joignable seulement par Tailscale.

## Documentation

| Document | Contenu |
| --- | --- |
| [`docs/design.md`](docs/design.md) | Conception et feuille de route (jalons 1 à 8) |
| [`docs/etape-1.md`](docs/etape-1.md) … [`docs/etape-8.md`](docs/etape-8.md) | Plan de chaque étape ; l'étape 1 est terminée, l'[étape 2](docs/etape-2.md) attend sa bascule réelle |
| [`docs/revue-etapes-0-1.md`](docs/revue-etapes-0-1.md) | Retour d'expérience de l'étape 1 et choix revus |
| [`docs/decisions/`](docs/decisions/README.md) | Décisions prises (écarts aux plans) |
| [`docs/decisions-a-prendre.md`](docs/decisions-a-prendre.md) | Questions encore ouvertes, avec la recommandation appliquée |
| [`SECURITY.md`](SECURITY.md) | Modèle de menace, conditions avant publication |
| [`CLAUDE.md`](CLAUDE.md) | Consignes pour les agents de développement |

**Étape 2 : le contenu est chiffré de bout en bout** sous une clé de groupe que
seuls mes appareils détiennent ; le relais ne stocke que des octets chiffrés et
les clients refusent tout message en clair. Une installation de l'étape 1 doit
d'abord passer par la [bascule](#passage-au-chiffrement-étape-2) : avant elle,
la base du relais peut encore contenir des textes en clair.

## Organisation

| Crate | Rôle |
| --- | --- |
| `crates/ot-proto` | Trames, enveloppe, codes d'erreur ; format figé par les tests de `tests/golden/` |
| `crates/ot-crypto` | Clés, chiffrement et signature de `payload`, code d'appairage ; format figé par `tests/vectors/` |
| `crates/ot-core` | Client : identité, connexion, reconnexion, file d'envoi, dédoublonnage, chiffrement, roster, appairage |
| `crates/ot-server` | Relais (axum + SQLite) et administration des appareils |
| `crates/ot-cli` | Binaire `ot` |

Le serveur ne dépend que de `ot-proto` et ne lit jamais `payload` ; il ne
dépend jamais de `ot-crypto` (vérifié en CI).

## Développement

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check            # cargo install --locked cargo-deny
```

Essai local avec deux appareils fictifs :

```sh
export OT_DB=/tmp/ot.db
cargo run -p ot-server -- serve --listen 127.0.0.1:7878 &
cargo run -p ot-cli -- --home /tmp/a init --server ws://127.0.0.1:7878 --name a
cargo run -p ot-server -- device add a <clé publique affichée>
cargo run -p ot-cli -- --home /tmp/a init --device-id <identifiant affiché>
cargo run -p ot-cli -- --home /tmp/a group init
cargo run -p ot-cli -- --home /tmp/a pair new &      # affiche un code ot1-…
cargo run -p ot-cli -- --home /tmp/b pair join --server ws://127.0.0.1:7878 --name b <code>
cargo run -p ot-cli -- --home /tmp/b listen &
cargo run -p ot-cli -- --home /tmp/a send bonjour
```

## Serveur (laptop Ubuntu)

Le service tourne sous un utilisateur système dédié `ot-relay` et n'écoute que
sur l'IP Tailscale du serveur. Dans les exemples, `relais` est son nom MagicDNS.

```sh
sudo useradd --system --home /var/lib/ot-relay --shell /usr/sbin/nologin ot-relay
```

```sh
cargo build --release -p ot-server
sudo install -m 0755 target/release/ot-server /usr/local/bin/
sudo install -D -m 0755 deploy/ot-backup.sh /usr/local/lib/oldtown/ot-backup.sh
sudo install -D -m 0644 README.md /usr/local/share/doc/oldtown/README.md
sudo install -m 0644 deploy/ot-server.service deploy/ot-backup.service deploy/ot-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
# Écouter sur l'IP Tailscale (`tailscale ip -4`) plutôt que 127.0.0.1 :
sudo systemctl edit ot-server     # [Service] puis Environment=OT_LISTEN=<ip tailscale>:7878
sudo systemctl enable --now ot-server.service ot-backup.timer
```

Vérifier :

```sh
systemctl status ot-server
journalctl -u ot-server -f
curl http://relais:7878/healthz      # ok
curl http://relais:7878/metrics
```

Mise à jour : `cargo build --release -p ot-server`, réinstaller le binaire, puis
`sudo systemctl restart ot-server`. Les migrations s'appliquent au démarrage.

### Appareils

Seul le tout premier appareil s'enregistre sur le serveur ; les suivants
rejoignent le groupe par `ot pair` (voir [CLI](#cli-ot-mac-windows-linux)),
sans session SSH.

```sh
alias ots='sudo -u ot-relay OT_DB=/var/lib/ot-relay/ot.db /usr/local/bin/ot-server'
ots device add mac <clé publique>     # affiche l'identifiant dev_…
ots device list
ots device revoke mac                 # une session ouverte reste active jusqu'à sa reconnexion
ots messages purge                    # purge immédiate des messages acquittés ou expirés
```

Un jeton d'appairage vaut 10 minutes et une seule fois ; le relais n'en garde
que le SHA-256. `/metrics` compte les jetons créés, utilisés et refusés
(`ot_pairings_total`).

### Sauvegarde

`ot-backup.timer` écrit chaque jour dans `/var/lib/ot-relay/backups/` la liste des
appareils (des `INSERT` SQL), jamais les messages. 14 fichiers sont conservés.
Restaurer sur une base neuve :

```sh
sudo systemctl stop ot-server
sudo -u ot-relay mv /var/lib/ot-relay/ot.db /var/lib/ot-relay/ot.db.old
ots device list                       # crée la base vide
sudo -u ot-relay sqlite3 /var/lib/ot-relay/ot.db < /var/lib/ot-relay/backups/devices-<date>.sql
sudo systemctl start ot-server
```

### Passage au chiffrement (étape 2)

À faire une fois, pour une installation de l'étape 1 :

1. Mettre à jour le relais (voir plus haut) **et** la CLI sur chaque appareil ;
   les anciens clients ne savent ni chiffrer ni appairer.
2. Supprimer tous les messages, qui peuvent être en clair, puis compacter la
   base (relais arrêté) :

   ```sh
   sudo systemctl stop ot-server
   ots messages purge --all
   sudo systemctl start ot-server
   ```

3. Sur le premier appareil (déjà enregistré) : `ot group init`.
4. Les autres appareils de l'étape 1 sont révoqués (`ots device revoke <nom>`)
   puis réappairés : `ot pair new` sur le premier, `ot pair join` sur chacun
   (même nom possible, l'ancien étant révoqué).
5. Vérifier avec `ot group show` sur deux appareils (mêmes époque et empreinte
   de clé) et `ot devices` (aucun appareil « hors du groupe »).

## CLI `ot` (Mac, Windows, Linux)

Binaire produit par la CI (artefacts `ot-macOS-ARM64`, `ot-Windows-X64`) ou :

```sh
cargo install --path crates/ot-cli
```

Premier appareil (une seule fois, avec le serveur) :

```sh
ot init --server ws://relais:7878 --name mac   # crée les clés, affiche la commande à lancer sur le serveur
ot init --device-id dev_…                       # après `ot-server device add`
ot group init                                   # crée le groupe : époque 1, seul membre
```

Chaque appareil suivant, par code d'appairage :

```sh
ot pair new                                     # sur un membre : affiche un code ot1-…, attend 10 min
ot pair join --server ws://relais:7878 --name windows ot1-…   # sur le nouvel appareil
```

Le code contient un secret : le transmettre par un canal de confiance
(copier-coller dans une session SSH, à la main), jamais par un service tiers.
Le relais peut refuser un appairage, pas l'usurper.

Usage courant :

```sh
ot send "bonjour"                               # à tous mes autres appareils
ot send --to windows https://example.org        # un lien, à un seul membre du groupe
echo "texte" | ot send
ot listen                                       # affiche les messages au fil de l'eau
ot devices                                      # appareils du relais ; signale ceux hors du groupe
ot group show                                   # époque, membres, empreintes de leurs clés
ot group rotate --exclude android               # nouvelle clé ; android ne lit plus la suite
```

Après `ot group rotate --exclude <nom>`, révoquer aussi l'appareil sur le
serveur (`ots device revoke <nom>`). Les anciennes clés restent 7 jours pour lire
les messages en retard.

Tout est chiffré et signé avant de partir ; un message en clair, d'un appareil
hors du groupe ou mal signé est rejeté (et signalé par `ot listen`). Un message
dont la clé n'est pas encore arrivée est mis de côté puis affiché à son arrivée.

Fichiers (`--home` ou `OT_HOME` pour en changer) : `~/.config/oldtown` sous Linux,
`~/Library/Application Support/oldtown` sous macOS, `%APPDATA%\oldtown\config`
sous Windows :

| Fichier | Contenu |
| --- | --- |
| `config.toml` | Relais, nom et identifiant de l'appareil |
| `device.key` (`0600`) | Clé privée Ed25519 : authentification et signature |
| `exchange.key` (`0600`) | Clé privée X25519 : réception d'une clé de groupe |
| `group.json` (`0600`) | Clés de groupe par époque |
| `roster.json` (`0600`) | Membres du groupe et leurs clés publiques |
| `quarantine.json` (`0600`) | Messages en attente de leur clé (100 au plus) |
| `seen.json` | Derniers messages reçus, pour ne jamais les afficher deux fois |

Une seule connexion est active par appareil : lancer `ot send` ou `ot devices`
sur un appareil qui fait tourner `ot listen` coupe brièvement ce dernier, qui se
reconnecte seul.

## Dépannage

| Symptôme | Cause probable | Correctif |
| --- | --- | --- |
| `tailscale ping relais` répond, mais `curl http://relais:7878/healthz` échoue (« could not connect », ou « Défaillance générale » au `ping` sous Windows) | Un autre VPN actif capte le trafic vers `100.x` avant Tailscale | Couper l'autre VPN, ou l'exclure de la plage `100.64.0.0/10` (ou de l'appli Tailscale) par son réglage de « split tunneling » |
| Même symptôme juste après l'installation de Tailscale sous Windows | La carte réseau virtuelle n'est pas encore active | Redémarrer le PC |
| `relais` ne se résout pas, mais l'IP `100.x` répond | DNS du tailnet (MagicDNS) non utilisé par l'appareil | Activer *Use Tailscale DNS* dans le client Tailscale |
| Le service ne démarre pas, « Address not available » dans `journalctl -u ot-server` | L'IP Tailscale n'est pas encore montée au démarrage | Rien à faire : systemd réessaie toutes les 5 s |
| `authentification refusée` côté `ot` | Appareil non enregistré, révoqué, ou `device_id` erroné | `ot-server device list` sur le serveur, puis `ot init --device-id …` |
| `cet appareil n'a pas de groupe` | Ni `ot group init` ni `ot pair join` | `ot group init` sur le premier appareil, `ot pair join` sur les autres |
| `ot pair join` : `PairingInvalid` | Code faux, expiré (10 min), déjà utilisé, ou parrain révoqué | Nouveau code avec `ot pair new` |
| `ot pair join` : `NameTaken` | Un appareil actif porte déjà ce nom | Autre `--name`, ou révoquer l'ancien ; le code reste valable |
| `ot pair join` attend sans fin | `ot pair new` n'est plus lancé chez le parrain | Relancer `ot pair new`, puis `ot pair join --force` avec le nouveau code |
| `message … rejeté : expéditeur hors roster` | Appareil exclu, ou roster pas encore reçu | `ot group show` ; réappairer l'appareil si besoin |
| `ot devices` : « hors du groupe » | Appairage inachevé, ou appareil ajouté au relais sans appairage | Le révoquer sur le serveur s'il est inconnu |

Pour localiser une panne réseau sous Windows :

```powershell
Find-NetRoute -RemoteIPAddress <ip tailscale du relais> | Select-Object -First 1 InterfaceAlias
Test-NetConnection <ip tailscale du relais> -Port 7878
```

La carte de sortie doit être `Tailscale` ; sinon, un autre logiciel (VPN, virtualisation) utilise la même plage d'adresses.
