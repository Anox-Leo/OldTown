# ot-ffi

Pont UniFFI (mode macros, sans UDL) entre `ot-core` et l'appli Android
(note 0022). Expose : `OtClient` (connexion, envoi, groupe, appairage,
`reconnect_now`), des fonctions sans connexion (`join`, `device_state`,
`group_snapshot`, `history`, `set_server_url`, `forget_device`,
`classify_qr`, `qr_code`, `set_file_uri`) et les traits étrangers
`SecretStore`, `FileSource`, `FileSink` et `TransferListener`, implémentés en
Kotlin. Fichiers (étape 5) : `OtClient::send_file`, `download`,
`cancel_upload` ; un fichier reçu est un élément d'historique de type `File`
(`Received::file`, son URI une fois enregistré). Envois (note 0033) :
`send_text` et `send_file` rangent l'élément (`outgoing`, `pending` jusqu'à
l'accusé) et le signalent par `Event::Sent` (`Event::Unsent` s'il échoue) ;
chaque élément porte ses destinataires (`to`, vide : tous). Un code à usage
unique (note 0045) devient un élément `Text` dont le corps est le code seul.

Étape 10 : `people` (annuaire vérifié : catégorie déclarée `kind` des
appareils, `muted` des services), `refresh_directory`, envois à une
personne (`send_text_to_user`, `send_file_to_user`, l'élément porte
`to_user` et `to_device`), `admin_root(create)` (D51 : adopte une racine
publiée, ne la crée que si `create`), administration, invitation
(`join_invite`, `invite_pending`), non-lus (`opened`, `mark_opened`).
Lu partout (note 0079) : `OtClient::start` reçoit les capacités que
l'appli sait afficher (`read-v1`…) ; `mark_read` range l'ouverture et, si
elle avance, prévient mes autres appareils ; un « lu » reçu met à jour
`opened.json` puis donne `Event::Read`.

- Types exposés simples (`types.rs`) : chaînes, entiers, enregistrements.
  Jamais de clé (pas même celle d'un fichier) ; seuls `Invitation` et
  `QrKind::Join` portent un code d'appairage, que l'appli affiche.
- État local rangé dans le `SecretStore` (`local.rs`) : `device.json`,
  `device.key` (même format que la CLI), `pending_join.json`, `seen.json`,
  `history.json` (100 derniers, reçus et envoyés, note 0033),
  `opened.json` (dernière ouverture de chaque personne), plus les noms du
  `StoreKeyring`.
- Runtime tokio interne (`RUNTIME`) : toute fonction `async` exportée passe
  par `on_runtime`. Pas de `block_on` dans du code appelé par Kotlin.
- Pas de `[lints] workspace = true` : le code généré par UniFFI contient de
  l'`unsafe`. Le code écrit ici n'en contient pas.
- Les liaisons Kotlin sont générées au build Android par le binaire
  `uniffi-bindgen` (option `bindgen`), depuis la bibliothèque de l'hôte
  (`cargo build -p ot-ffi`) : la bibliothèque Android dépouillée n'a plus
  les métadonnées. Jamais versionnées.
- Tests (`src/tests.rs`) : l'API exportée contre un relais en processus.
