# ot-ffi

Pont UniFFI (mode macros, sans UDL) entre `ot-core` et l'appli Android
(note 0022). Expose : `OtClient` (connexion, envoi, groupe, appairage,
`reconnect_now`), des fonctions sans connexion (`join`, `device_state`,
`group_snapshot`, `history`, `set_server_url`, `forget_device`,
`classify_qr`, `qr_code`, `set_file_uri`) et les traits étrangers
`SecretStore`, `FileSource`, `FileSink` et `TransferListener`, implémentés en
Kotlin. Fichiers (étape 5) : `OtClient::send_file`, `download`,
`cancel_upload` ; un fichier reçu est un élément d'historique de type `File`
(`Received::file`, son URI une fois enregistré).

- Types exposés simples (`types.rs`) : chaînes, entiers, enregistrements.
  Jamais de clé (pas même celle d'un fichier) ; seuls `Invitation` et
  `QrKind::Join` portent un code d'appairage, que l'appli affiche.
- État local rangé dans le `SecretStore` (`local.rs`) : `device.json`,
  `device.key` (même format que la CLI), `pending_join.json`, `seen.json`,
  `history.json` (50 derniers), plus les noms du `StoreKeyring`.
- Runtime tokio interne (`RUNTIME`) : toute fonction `async` exportée passe
  par `on_runtime`. Pas de `block_on` dans du code appelé par Kotlin.
- Pas de `[lints] workspace = true` : le code généré par UniFFI contient de
  l'`unsafe`. Le code écrit ici n'en contient pas.
- Les liaisons Kotlin sont générées au build Android par le binaire
  `uniffi-bindgen` (option `bindgen`), depuis la bibliothèque de l'hôte
  (`cargo build -p ot-ffi`) : la bibliothèque Android dépouillée n'a plus
  les métadonnées. Jamais versionnées.
- Tests (`src/tests.rs`) : l'API exportée contre un relais en processus.
