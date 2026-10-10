# OldTown

Échange de textes et de liens entre mes appareils, via un relais auto-hébergé
sur le laptop Ubuntu, joignable seulement par Tailscale. Conception :
[`docs/design.md`](docs/design.md) ; étape en cours : [`docs/etape-1.md`](docs/etape-1.md).

**Étape 1 : le contenu circule en clair** (`enc: "none"`). Tailscale chiffre le
réseau, mais la base du laptop garde les textes jusqu'à leur purge : pas de
données sensibles avant l'étape 2.

## Organisation

| Crate | Rôle |
| --- | --- |
| `crates/ot-proto` | Trames, enveloppe, codes d'erreur ; format figé par les tests de `tests/golden/` |
| `crates/ot-core` | Client : identité Ed25519, connexion, reconnexion, file d'envoi, dédoublonnage |
| `crates/ot-server` | Relais (axum + SQLite) et administration des appareils |
| `crates/ot-cli` | Binaire `ot` |

Le serveur ne dépend que de `ot-proto` et ne lit jamais `payload`.

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
# idem pour b, puis :
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

```sh
alias ots='sudo -u ot-relay OT_DB=/var/lib/ot-relay/ot.db /usr/local/bin/ot-server'
ots device add mac <clé publique>     # affiche l'identifiant dev_…
ots device list
ots device revoke mac                 # une session ouverte reste active jusqu'à sa reconnexion
```

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

## CLI `ot` (Mac, Windows, Linux)

Binaire produit par la CI (artefacts `ot-macOS-ARM64`, `ot-Windows-X64`) ou :

```sh
cargo install --path crates/ot-cli
```

```sh
ot init --server ws://relais:7878 --name mac   # crée la clé, affiche la commande à lancer sur le serveur
ot init --device-id dev_…                       # après `ot-server device add`
ot send "bonjour"                               # à tous mes autres appareils
ot send --to windows https://example.org        # un lien, à un seul appareil
echo "texte" | ot send
ot listen                                       # affiche les messages au fil de l'eau
ot devices
```

Fichiers (`--home` ou `OT_HOME` pour en changer) : `~/.config/oldtown` sous Linux,
`~/Library/Application Support/oldtown` sous macOS, `%APPDATA%\oldtown\config`
sous Windows. `device.key` contient la clé privée (`0600`), `seen.json` les
derniers messages reçus, pour ne jamais les afficher deux fois.

Une seule connexion est active par appareil : lancer `ot send` ou `ot devices`
sur un appareil qui fait tourner `ot listen` coupe brièvement ce dernier, qui se
reconnecte seul.
