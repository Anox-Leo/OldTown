# ot-cli

Binaire `ot` : outil d'usage en terminal et banc d'essai du protocole
(`init`, `send`, `listen`, `devices` ; `group` et `pair` à l'étape 2 ;
`pair new --qr`, `pair join --qr` et `pair sponsor` à l'étape 3 ;
`send --file`, `listen --download-dir`, `files` et `download` à l'étape 5 ;
`otp-bridge`, note 0045 ; à l'étape 10, `users`, `send --to-user`, `device`,
`mute`, `invite join`, `admin …`, `pair join --qr --service` et
`otp-bridge --to-user`, dans `people.rs`).

- Mince couche au-dessus de `ot-core` : pas de logique de protocole ici.
- Fichiers locaux dans `files.rs` (`config.toml`, `device.key`,
  `seen.json`, et le `SecretStore` sur fichiers `FileStore`, sous le
  `StoreKeyring` d'`ot-core` : `exchange.key`, `group.json`, `roster.json`,
  `quarantine.json`, `files.json`, `uploads.json`, tous en `0600` ;
  `otp-sent.json`, codes déjà relayés par `otp-bridge`, en `0600`) ;
  source et destination des fichiers dans `transfer.rs` (fichier reçu écrit
  en `.<nom>.oldtown-part`, renommé sous un nom libre une fois vérifié) ;
  dossier choisi par `--home` ou `OT_HOME`, sinon celui de l'OS. Tout nouveau secret est créé en
  `0600` et passe par des tampons `Zeroizing`.
- `otp-bridge` (notes 0045, 0048) : socket d'otp-relay (`tools/otp-relay`,
  contrat `PROTOCOL.md`) lu dans `otp.rs` (lignes,
  filtrage `judge`, codes déjà envoyés `Sent`), envoi par
  `Client::send_otp`, une session tenue tant qu'il tourne. Jamais de code
  ni de ligne du socket dans la sortie.
- Les destinataires (`--to`, `--exclude`) se résolvent dans le roster, jamais
  dans la liste du relais. Les personnes (`--to-user`, `--device`, `mute`) se
  résolvent dans l'annuaire vérifié (`Client::fresh_directory`) ; les
  commandes d'administration dans la liste du relais (`Client::users`), qui
  n'engage rien : le cœur signe et vérifie. Un nom ambigu est refusé avec les
  identifiants (`people::pick`).
- Appareil de service : `config.toml` porte `role = "service"` ; pas de
  groupe, et aucun avertissement pour cela. `otp-bridge --to-user` saute les
  personnes qui ont coupé le service ou sans appareil joignable et à jour,
  par identifiant dans le journal. Avec `--device` (une seule personne), il
  vise cet appareil au lieu de ses appareils par défaut.
- Chaque commande ponctuelle ouvre une session et coupe brièvement un
  `ot listen` de la même identité (note 0009). Sur un ordinateur qui a aussi
  l'extension Chrome, la CLI est un appareil distinct (note 0017).
- Tests : unitaires dans les modules ; `tests/files.rs` lance le binaire
  `ot` pour deux appareils contre un relais en processus ; `tests/people.rs`
  et `tests/otp.rs` font le parcours de l'étape 10 par les commandes
  (`tests/common` : admin, invitation, appareil de service).
- Messages pour l'utilisateur en français, erreurs avec une piste de
  correction (voir la section « Dépannage » du README).
- Toute commande ou option nouvelle se documente dans le README, section
  « CLI ». Son aide s'écrit en attributs clap (`#[command(about = …)]`,
  `#[arg(help = …)]`) : aucun commentaire dans le code (note 0077).
