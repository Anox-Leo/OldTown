# OldTown

Échange de textes et de liens entre mes appareils, via un relais auto-hébergé
sur un laptop Ubuntu, joignable seulement par Tailscale. Le contenu est chiffré
de bout en bout sous une clé de groupe que seuls mes appareils détiennent : le
relais ne stocke que des octets chiffrés et les clients refusent tout message
en clair.

**État** : étapes 1 (relais et CLI) et 2 (chiffrement et appairage) terminées ;
étape 3 (appli Android, `wss`, appairage par QR) livrée, en essai réel.
Clients : la CLI `ot` sous macOS, Windows et Linux, et l'appli Android
(APK à compiler, voir [Téléphone Android](#5-téléphone-android)). L'extension
Chrome (étape 4) est à venir.

Sommaire :
[Architecture](#architecture) ·
[Prérequis](#prérequis) ·
[Relais (Ubuntu)](#1-relais-ubuntu) ·
[CLI (macOS, Windows, Linux)](#2-cli-ot-macos-windows-linux) ·
[Premier appareil](#3-premier-appareil) ·
[Appareils suivants](#4-appareils-suivants) ·
[Téléphone Android](#5-téléphone-android) ·
[Utilisation](#utilisation) ·
[Administration du relais](#administration-du-relais) ·
[Mise à jour](#mise-à-jour) ·
[Développement](#développement) ·
[Dépannage](#dépannage)

## Architecture

```
 appareil A (ot) ──wss──►  relais (ot-server, SQLite)  ◄──wss── téléphone (appli)
        chiffre + signe        stocke et relaie, ne lit       vérifie + déchiffre
                               jamais le contenu
             └──────────── tailnet Tailscale (WireGuard) ────────────┘
```

| Crate | Rôle |
| --- | --- |
| `crates/ot-proto` | Trames, enveloppe, codes d'erreur ; format figé par `tests/golden/` |
| `crates/ot-crypto` | Clés, chiffrement et signature de `payload`, code d'appairage ; format figé par `tests/vectors/` |
| `crates/ot-core` | Client : identité, connexion, reconnexion, file d'envoi, dédoublonnage, chiffrement, roster, appairage |
| `crates/ot-server` | Relais (axum + SQLite) et administration des appareils |
| `crates/ot-cli` | Binaire `ot` |
| `crates/ot-ffi` | Pont UniFFI vers l'appli Android (liaisons Kotlin générées au build) |
| `apps/android` | Appli Android (Kotlin, Jetpack Compose) |

Le relais ne dépend que de `ot-proto` et ne lit jamais `payload` ; il ne
dépend jamais de `ot-crypto` (vérifié en CI).

| Document | Contenu |
| --- | --- |
| [`docs/design.md`](docs/design.md) | Conception et feuille de route (jalons 1 à 8) |
| [`docs/etape-1.md`](docs/etape-1.md) … [`docs/etape-8.md`](docs/etape-8.md) | Plan et état de chaque étape |
| [`docs/actions-manuelles.md`](docs/actions-manuelles.md) | Actions à faire à la main sur mes machines, dans l'ordre |
| [`docs/revue-etapes-0-1.md`](docs/revue-etapes-0-1.md) | Retour d'expérience de l'étape 1 et choix revus |
| [`docs/decisions/`](docs/decisions/README.md) | Décisions prises (écarts aux plans) |
| [`docs/decisions-a-prendre.md`](docs/decisions-a-prendre.md) | Questions encore ouvertes, avec la recommandation appliquée |
| [`SECURITY.md`](SECURITY.md) | Modèle de menace, conditions avant publication |
| [`CLAUDE.md`](CLAUDE.md) | Consignes pour les agents de développement |

## Prérequis

- **Tailscale** sur le relais et sur chaque appareil, tous dans le même
  tailnet, avec MagicDNS activé (*Use Tailscale DNS* dans chaque client). Dans
  ce document, `relais` est le nom MagicDNS du serveur.
- **Rust 1.88 ou plus** (via [rustup](https://rustup.rs)) pour compiler le
  relais, et la CLI si l'on n'utilise pas les binaires de la CI :
  - Ubuntu : `sudo apt install build-essential` (la base SQLite est compilée
    avec le relais) ;
  - macOS : `xcode-select --install` ;
  - Windows : les *Visual Studio Build Tools* (charge de travail « C++ »),
    proposés par `rustup-init.exe`.

## Installation

Ordre : le relais, puis la CLI sur chaque appareil, puis le premier appareil
(qui crée le groupe), puis les suivants par code d'appairage.

### 1. Relais (Ubuntu)

Le relais tourne en service systemd sous un utilisateur système dédié
`ot-relay`, et n'écoute que sur l'IP Tailscale du serveur, jamais sur
`0.0.0.0`.

1. Paquets et utilisateur :

   ```sh
   sudo apt install build-essential sqlite3     # sqlite3 sert à la sauvegarde
   sudo useradd --system --home /var/lib/ot-relay --shell /usr/sbin/nologin ot-relay
   ```

2. Compiler et installer, depuis une copie du dépôt :

   ```sh
   cargo build --release --locked -p ot-server
   sudo install -m 0755 target/release/ot-server /usr/local/bin/
   sudo install -D -m 0755 deploy/ot-backup.sh /usr/local/lib/oldtown/ot-backup.sh
   sudo install -D -m 0644 README.md /usr/local/share/doc/oldtown/README.md
   sudo install -m 0644 deploy/ot-server.service deploy/ot-backup.service deploy/ot-backup.timer /etc/systemd/system/
   sudo systemctl daemon-reload
   ```

3. Écouter sur l'IP Tailscale (`tailscale ip -4`) : l'unité livrée écoute sur
   `127.0.0.1:7878`, à surcharger localement.

   ```sh
   sudo systemctl edit ot-server
   ```

   ```ini
   [Service]
   Environment=OT_LISTEN=<ip tailscale>:7878
   ```

   Autres réglages possibles de la même façon : `OT_TTL` (durée de vie d'un
   message non acquitté, `7d` par défaut), `OT_PURGE_INTERVAL` (`10m`),
   `OT_LOG_FORMAT` (`json` dans l'unité).

4. TLS (`wss`, note 0015) : certificat Let's Encrypt du nom MagicDNS du
   serveur, obtenu et renouvelé par `tailscale cert`. Activer d'abord HTTPS
   dans la console d'administration Tailscale (*DNS* → *HTTPS Certificates*),
   puis :

   ```sh
   sudo install -d /etc/oldtown
   echo "OT_TLS_DOMAIN=$(tailscale status --json | jq -r '.Self.DNSName | rtrimstr(".")')" | sudo tee /etc/oldtown/tls.env
   sudo install -m 0644 deploy/ot-tls-renew.service deploy/ot-tls-renew.timer /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl start ot-tls-renew.service        # premier certificat
   sudo systemctl enable --now ot-tls-renew.timer   # renouvellement quotidien
   ```

   Puis, dans la surcharge de l'étape 3 :

   ```ini
   Environment=OT_TLS_CERT=/var/lib/ot-relay/tls/relais.crt
   Environment=OT_TLS_KEY=/var/lib/ot-relay/tls/relais.key
   ```

   Le relais relit le certificat quand il change (vérification toutes les
   10 minutes), sans redémarrage. Sans ces deux variables, il reste en `ws://`
   (essais locaux). Le nom du serveur apparaît dans les journaux publics de
   Certificate Transparency (accepté, note 0015).

5. Démarrer le relais et la sauvegarde quotidienne :

   ```sh
   sudo systemctl enable --now ot-server.service ot-backup.timer
   ```

6. Raccourci d'administration, à ajouter à `~/.bashrc` :

   ```sh
   alias ots='sudo -u ot-relay OT_DB=/var/lib/ot-relay/ot.db /usr/local/bin/ot-server'
   ```

7. Vérifier, depuis le serveur puis depuis un autre appareil du tailnet :

   ```sh
   systemctl status ot-server
   journalctl -u ot-server -f
   curl https://relais.exemple.ts.net:7878/healthz      # ok (http://… sans TLS)
   curl https://relais.exemple.ts.net:7878/metrics
   ```

   En TLS, utiliser le nom complet du certificat, pas l'IP ni le nom court.

Au démarrage de la machine, le service peut échouer une première fois tant que
l'IP Tailscale n'est pas montée ; systemd réessaie toutes les 5 s.

### 2. CLI `ot` (macOS, Windows, Linux)

**Obtenir le binaire.** Chaque push sur `main` dont la CI passe produit trois
artefacts, gardés 14 jours : `ot-macOS-ARM64`, `ot-Windows-X64`,
`ot-Linux-X64`. Avec la CLI GitHub :

```sh
gh run list --workflow CI --branch main --event push --status success --limit 1   # ID du run
gh run download <ID> -n ot-Windows-X64 -D ot-windows      # ou ot-macOS-ARM64, ot-Linux-X64
```

Ou, par le navigateur, page du run dans l'onglet *Actions*, section
*Artifacts* (un `.zip` à décompresser). Si les artefacts ont expiré, ou pour
une autre architecture (Mac Intel, Windows ARM), compiler sur l'appareil :

```sh
cargo install --locked --path crates/ot-cli     # installe ot dans ~/.cargo/bin, déjà dans le PATH
```

**Installer le binaire téléchargé.**

- macOS (Apple Silicon) :

  ```sh
  sudo install -d /usr/local/bin
  sudo install -m 0755 ot /usr/local/bin/ot
  xattr -d com.apple.quarantine /usr/local/bin/ot 2>/dev/null || true   # si téléchargé par le navigateur
  ```

- Windows (PowerShell, sans droits d'administrateur) :

  ```powershell
  $dir = "$env:LOCALAPPDATA\Programs\oldtown"
  New-Item -ItemType Directory -Force $dir | Out-Null
  Copy-Item .\ot.exe $dir
  Unblock-File "$dir\ot.exe"                     # si téléchargé par le navigateur
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  [Environment]::SetEnvironmentVariable("Path", "$userPath;$dir", "User")
  ```

  Rouvrir le terminal pour que le `PATH` soit pris en compte. Le binaire n'est
  pas signé : SmartScreen peut avertir au premier lancement.

- Linux (x86-64, binaire compilé sur la dernière Ubuntu LTS de GitHub) :

  ```sh
  sudo install -m 0755 ot /usr/local/bin/ot
  ```

Vérifier : `ot --version`.

### 3. Premier appareil

Une seule fois, sur l'appareil qui crée le groupe. C'est le seul qui
s'enregistre sur le relais par SSH.

1. Créer les clés de l'appareil :

   ```sh
   ot init --server wss://relais.exemple.ts.net:7878 --name mac
   ```

   La commande affiche la clé publique et la commande à lancer sur le serveur.

2. Sur le serveur, enregistrer l'appareil :

   ```sh
   ots device add mac <clé publique>     # affiche l'identifiant dev_…
   ```

3. Sur l'appareil, enregistrer l'identifiant puis créer le groupe :

   ```sh
   ot init --device-id dev_…
   ot group init                         # époque 1, seul membre
   ```

### 4. Appareils suivants

Par code d'appairage, sans passer par le serveur :

1. Sur un appareil déjà membre :

   ```sh
   ot pair new                           # affiche un code ot1-…, attend 10 minutes au plus
   ot pair new --qr                      # le même code, aussi en QR pour le téléphone
   ```

2. Sur le nouvel appareil, la CLI installée :

   ```sh
   ot pair join --server wss://relais.exemple.ts.net:7878 --name windows ot1-…
   ```

   Ou, sans rien taper, parrainé par le téléphone (QR inversé, note 0022) :

   ```sh
   ot pair join --qr --server wss://relais.exemple.ts.net:7878 --name windows
   ```

   Le QR s'affiche dans le terminal (y compris en SSH) ; sur le téléphone :
   *Groupe* → *Ajouter un appareil*, viser le QR, confirmer le nom. Sans
   téléphone, un autre membre peut parrainer avec le texte affiché sous le
   QR : `ot pair sponsor 'oldtown://sponsor?o=…'`.

3. Vérifier sur les deux appareils : `ot group show` (mêmes époque et
   empreintes) et `ot devices` (aucun appareil « hors du groupe »).

Le code contient un secret : le transmettre par un canal de confiance (à la
main, copier-coller dans une session SSH), jamais par un service tiers. Le
relais peut refuser un appairage, pas l'usurper. Un jeton vaut 10 minutes et
une seule fois.

### 5. Téléphone Android

Android 13 au moins. L'APK se compile depuis le dépôt (voir
[Développement](#développement)) et s'installe à la main (`adb install`,
ou en ouvrant l'APK sur le téléphone) ; pas de Play Store.

1. Tailscale installé et connecté sur le téléphone.
2. Sur un ordinateur du groupe : `ot pair new --qr`.
3. Ouvrir OldTown, choisir le nom du téléphone, *Scanner le QR* (ou *Saisir
   le code* : URL du relais et code `ot1-…`).
4. Accepter les notifications, et l'exemption d'optimisation de batterie
   proposée sur l'écran d'accueil : sans elle, les messages peuvent attendre
   le réveil du téléphone.

L'appli tient une connexion permanente au relais (service de premier plan,
notification « Connecté »), relancée au démarrage du téléphone. Elle reçoit
les textes et liens (notification avec *Copier*, *Ouvrir*, *Partager*, et les
50 derniers éléments dans l'écran d'accueil), envoie par le menu *Partager*
de n'importe quelle appli (et ses raccourcis par appareil), et parraine les
nouveaux appareils par QR. Les clés sont chiffrées par une clé du Keystore
Android et exclues des sauvegardes.

## Utilisation

```sh
ot send "bonjour"                        # à tous mes autres appareils
ot send --to windows https://example.org # à un seul membre (option répétable)
echo "texte" | ot send                   # lu sur l'entrée standard
ot listen                                # messages reçus au fil de l'eau (--json : une ligne JSON par message)
ot devices                               # appareils du relais ; signale ceux hors du groupe
ot group show                            # époque, membres, empreintes de leurs clés
ot pair sponsor 'oldtown://sponsor?o=…'  # parraine un appareil d'après le texte de son QR
```

Tout est chiffré et signé avant de partir ; un message en clair, d'un appareil
hors du groupe ou mal signé est rejeté et signalé par `ot listen`. Un message
dont la clé n'est pas encore arrivée est mis de côté puis affiché à son
arrivée. Un message non lu est gardé 7 jours par le relais.

Une seule connexion est active par appareil : lancer `ot send` ou `ot devices`
sur un appareil qui fait tourner `ot listen` coupe brièvement ce dernier, qui
se reconnecte seul.

**Fichiers de la CLI** (`--home` ou `OT_HOME` pour en changer) :
`~/.config/oldtown` sous Linux, `~/Library/Application Support/oldtown` sous
macOS, `%APPDATA%\oldtown\config` sous Windows.

| Fichier | Contenu |
| --- | --- |
| `config.toml` | Relais, nom et identifiant de l'appareil |
| `device.key` (`0600`) | Clé privée Ed25519 : authentification et signature |
| `exchange.key` (`0600`) | Clé privée X25519 : réception d'une clé de groupe |
| `group.json` (`0600`) | Clés de groupe par époque |
| `roster.json` (`0600`) | Membres du groupe et leurs clés publiques |
| `quarantine.json` (`0600`) | Messages en attente de leur clé (100 au plus) |
| `seen.json` | Derniers messages reçus, pour ne jamais les afficher deux fois |

## Administration du relais

```sh
ots device list                          # appareils, révoqués compris
ots device revoke mac                    # une session ouverte reste active jusqu'à sa reconnexion
ots messages purge                       # purge immédiate des messages acquittés ou expirés
```

`/metrics` compte notamment les appairages (`ot_pairings_total` : jetons
créés et utilisés, appareils enregistrés par un parrain, refus) ; le relais ne
garde que le SHA-256 des jetons.

**Retirer un appareil (perdu, volé, remplacé).** Depuis un autre membre, puis
sur le serveur :

```sh
ot group rotate --exclude android        # nouvelle clé ; android ne lit plus la suite
ots device revoke android
```

Les anciennes clés restent 7 jours sur les autres appareils, pour lire les
messages en retard.

**Sauvegarde.** `ot-backup.timer` écrit chaque jour dans
`/var/lib/ot-relay/backups/` la liste des appareils (des `INSERT` SQL), jamais
les messages ; 14 fichiers sont conservés. Restaurer sur une base neuve :

```sh
sudo systemctl stop ot-server
sudo -u ot-relay mv /var/lib/ot-relay/ot.db /var/lib/ot-relay/ot.db.old
ots device list                          # crée la base vide
sudo -u ot-relay sqlite3 /var/lib/ot-relay/ot.db < /var/lib/ot-relay/backups/devices-<date>.sql
sudo systemctl start ot-server
```

Les clés de groupe ne sont que sur les appareils : la base du relais ne permet
de rien déchiffrer. Sa perte ne fait perdre que les messages non livrés et,
sans sauvegarde, la liste des appareils.

## Mise à jour

- **Relais** : dans la copie du dépôt à jour,

  ```sh
  cargo build --release --locked -p ot-server
  sudo install -m 0755 target/release/ot-server /usr/local/bin/
  sudo systemctl restart ot-server
  ```

  Les migrations de la base s'appliquent au démarrage. Réinstaller aussi les
  fichiers de `deploy/` s'ils ont changé (étape 2 de l'installation, puis
  `sudo systemctl daemon-reload`).

- **CLI** : remplacer le binaire comme à l'installation (ou relancer
  `cargo install --locked --path crates/ot-cli`). Clés, groupe et
  configuration sont gardés. Passage au TLS : `ot init --server
  wss://relais.exemple.ts.net:7878` sur chaque appareil.

- **Android** : réinstaller l'APK (`adb install -r …`) ; clés et groupe sont
  gardés. L'URL du relais se change dans *Réglages*.

Le QR inversé (étape 3) demande un relais et des clients de l'étape 3 ; le
reste du protocole est inchangé.

## Développement

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check            # cargo install --locked cargo-deny
```

La CI lance ces vérifications sous Linux pour une pull request, et sous Linux,
macOS et Windows pour un push sur `main` ou un lancement manuel (onglet
*Actions*, *Run workflow*). Un changement qui ne touche que la documentation ne
lance rien.

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

**Appli Android** (`apps/android`, détails dans
[`apps/android/CLAUDE.md`](apps/android/CLAUDE.md)) : JDK 21, SDK Android
(API 37) et NDK r29, Rust avec les cibles Android et `cargo-ndk` :

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install --locked cargo-ndk
cd apps/android
./gradlew lint test assembleDebug        # APK : app/build/outputs/apk/debug/
./gradlew connectedDebugAndroidTest      # tests instrumentés, téléphone ou émulateur branché
```

Gradle compile le cœur Rust (`cargo ndk`, profil `android`) et génère les
liaisons Kotlin (`uniffi-bindgen`) ; Android Studio ouvre directement
`apps/android`. Réglages locaux dans `apps/android/local.properties` (jamais
versionné) : `cargo.dir` si `cargo` n'est pas dans `~/.cargo/bin`,
`oldtown.abis=arm64-v8a` pour ne compiler que pour le téléphone.

## Dépannage

| Symptôme | Cause probable | Correctif |
| --- | --- | --- |
| `tailscale ping relais` répond, mais `curl http://relais:7878/healthz` échoue (« could not connect », ou « Défaillance générale » au `ping` sous Windows) | Un autre VPN actif capte le trafic vers `100.x` avant Tailscale | Couper l'autre VPN, ou l'exclure de la plage `100.64.0.0/10` (ou de l'appli Tailscale) par son réglage de « split tunneling » |
| Même symptôme juste après l'installation de Tailscale sous Windows | La carte réseau virtuelle n'est pas encore active | Redémarrer le PC |
| `relais` ne se résout pas, mais l'IP `100.x` répond | DNS du tailnet (MagicDNS) non utilisé par l'appareil | Activer *Use Tailscale DNS* dans le client Tailscale |
| Le service ne démarre pas, « Address not available » dans `journalctl -u ot-server` | L'IP Tailscale n'est pas encore montée au démarrage | Rien à faire : systemd réessaie toutes les 5 s |
| `ot` introuvable après l'installation sous Windows | Terminal ouvert avant la modification du `PATH` | Rouvrir le terminal |
| macOS refuse de lancer `ot` (« impossible de vérifier le développeur ») | Attribut de quarantaine du téléchargement | `xattr -d com.apple.quarantine /usr/local/bin/ot` |
| `authentification refusée` côté `ot` | Appareil non enregistré, révoqué, ou `device_id` erroné | `ots device list` sur le serveur, puis `ot init --device-id …` |
| `cet appareil n'a pas de groupe` | Ni `ot group init` ni `ot pair join` | `ot group init` sur le premier appareil, `ot pair join` sur les autres |
| `ot pair join` : `PairingInvalid` | Code faux, expiré (10 min), déjà utilisé, ou parrain révoqué | Nouveau code avec `ot pair new` |
| `ot pair join` : `NameTaken` | Un appareil actif porte déjà ce nom | Autre `--name`, ou révoquer l'ancien ; le code reste valable |
| `ot pair join` attend sans fin | `ot pair new` n'est plus lancé chez le parrain | Relancer `ot pair new`, puis `ot pair join --force` avec le nouveau code |
| `message … rejeté : expéditeur hors roster` | Appareil exclu, ou roster pas encore reçu | `ot group show` ; réappairer l'appareil si besoin |
| `invalid peer certificate` | URL en IP ou nom court au lieu du nom du certificat, ou certificat pas encore obtenu | URL `wss://<nom complet>.ts.net:7878` ; `systemctl status ot-tls-renew` sur le serveur |
| `ot pair join --qr` attend sans fin | Personne n'a scanné le QR, ou le relais ne connaît pas `pair_register` (relais d'avant l'étape 3) | Scanner depuis le téléphone (*Groupe* → *Ajouter un appareil*) ; mettre le relais à jour |
| Android : « Relais injoignable » | Tailscale coupé sur le téléphone, ou URL fausse | Connecter Tailscale ; vérifier l'URL dans *Réglages* |
| Android : messages reçus seulement à l'ouverture de l'appli | Optimisation de batterie active | Accepter l'exemption proposée sur l'écran d'accueil (ou *Réglages* → *Batterie*) |
| Android : « En attente du parrain » qui dure | `ot pair new` arrêté avant la demande, ou une autre commande `ot` du parrain a pris sa session | Relancer l'appli (la demande repart) ; sinon *Réglages* → *Oublier cet appareil* et nouveau code |
| `ot devices` : « hors du groupe » | Appairage inachevé, ou appareil ajouté au relais sans appairage | Le révoquer sur le serveur s'il est inconnu |

Pour localiser une panne réseau sous Windows :

```powershell
Find-NetRoute -RemoteIPAddress <ip tailscale du relais> | Select-Object -First 1 InterfaceAlias
Test-NetConnection <ip tailscale du relais> -Port 7878
```

La carte de sortie doit être `Tailscale` ; sinon, un autre logiciel (VPN,
virtualisation) utilise la même plage d'adresses.
