# ot-cli

Binaire `ot` : outil d'usage en terminal et banc d'essai du protocole
(`init`, `send`, `listen`, `devices` ; `group` et `pair` à l'étape 2 ;
`pair new --qr`, `pair join --qr` et `pair sponsor` à l'étape 3 ;
`send --file`, `listen --download-dir`, `files` et `download` à l'étape 5).

- Mince couche au-dessus de `ot-core` : pas de logique de protocole ici.
- Fichiers locaux dans `files.rs` (`config.toml`, `device.key`,
  `seen.json`, et le `SecretStore` sur fichiers `FileStore`, sous le
  `StoreKeyring` d'`ot-core` : `exchange.key`, `group.json`, `roster.json`,
  `quarantine.json`, `files.json`, `uploads.json`, tous en `0600`) ;
  source et destination des fichiers dans `transfer.rs` (fichier reçu écrit
  en `.<nom>.oldtown-part`, renommé sous un nom libre une fois vérifié) ;
  dossier choisi par `--home` ou `OT_HOME`, sinon celui de l'OS. Tout nouveau secret est créé en
  `0600` et passe par des tampons `Zeroizing`.
- Les destinataires (`--to`, `--exclude`) se résolvent dans le roster, jamais
  dans la liste du relais.
- Chaque commande ponctuelle ouvre une session et coupe brièvement un
  `ot listen` de la même identité (note 0009). Sur un ordinateur qui a aussi
  l'extension Chrome, la CLI est un appareil distinct (note 0017).
- Tests : unitaires dans les modules ; `tests/files.rs` lance le binaire
  `ot` pour deux appareils contre un relais en processus.
- Messages pour l'utilisateur en français, erreurs avec une piste de
  correction (voir la section « Dépannage » du README).
- Toute commande ou option nouvelle se documente dans le README, section
  « CLI ».
