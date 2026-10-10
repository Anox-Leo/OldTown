# ot-cli

Binaire `ot` : outil d'usage en terminal et banc d'essai du protocole
(`init`, `send`, `listen`, `devices` ; `group` et `pair` à l'étape 2).

- Mince couche au-dessus de `ot-core` : pas de logique de protocole ici.
- Fichiers locaux dans `files.rs` (`config.toml`, `device.key` en `0600`,
  `seen.json`) ; dossier choisi par `--home` ou `OT_HOME`, sinon celui de
  l'OS. Tout nouveau secret est créé en `0600`.
- Chaque commande ponctuelle ouvre une session et coupe brièvement un
  `ot listen` de la même identité (note 0009) : sur une machine qui fait
  aussi tourner l'appli, la CLI est un appareil distinct (D12).
- Messages pour l'utilisateur en français, erreurs avec une piste de
  correction (voir la section « Dépannage » du README).
- Toute commande ou option nouvelle se documente dans le README, section
  « CLI ».
