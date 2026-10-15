# Plan : conversations à la WhatsApp sur Android

**État (15 octobre 2026)** : **réalisé** (A1 à A6), avec le tri par usage
de la feuille de partage ; écarts dans la note
[0033](decisions/0033-android-conversations-partage-par-usage.md). Reste
l'essai sur le téléphone (`actions-manuelles.md`).

Reprise sur l'appli Android de l'accueil en liste de conversations de
l'extension (notes [0030](decisions/0030-extension-conversation-par-canal.md)
et [0031](decisions/0031-extension-accueil-liste-de-conversations.md)). Plan
de cadrage : le relire contre le code avant de coder, et noter tout écart
dans une note de décision (la 0033 a priori). Lire aussi
`apps/android/CLAUDE.md`.

## But

- **Accueil** : en-tête « OldTown », boutons Groupe et Réglages (comme
  aujourd'hui), les bandeaux d'alerte, puis une ligne par conversation :
  « Tous mes appareils » en premier, puis un appareil par ligne, du plus
  récemment actif au moins récent. Chaque ligne : avatar rond (pictogramme
  du type d'appareil sur une couleur propre à l'appareil, point vert s'il
  est en ligne), nom en demi-gras, heure du dernier élément à droite, et
  aperçu du dernier élément sur une ligne, coupé (« Vous : » devant un
  envoi, nom de l'expéditeur dans « Tous », trombone devant un fichier).
- **Conversation** (nouvel écran) : en-tête flèche de retour, avatar, nom et
  dessous « en ligne » / « hors ligne » (pour « Tous » : « N en ligne »).
  Bulles envoyées à droite (couleur primaire), reçues à gauche (nom de
  l'expéditeur dans « Tous »), heure sous la bulle, « envoi… » tant que
  l'envoi n'est pas accepté. Zone d'envoi en bas : trombone, champ de
  texte, bouton d'envoi, vers le canal ouvert. Toucher une bulle ouvre la
  feuille actuelle de l'élément (`ItemSheet` : copier, ouvrir, partager,
  télécharger).
- Mêmes règles que l'extension, pour que les deux se ressemblent : même
  rangement des éléments (`channelOf`), même devinette du type d'appareil,
  même palette et même calcul de couleur (un appareil a la même couleur
  dans l'extension et sur le téléphone).

## Ce qui manque aujourd'hui

L'extension tient déjà tout en TypeScript ; sur Android, l'historique vient
de Rust (`ot-ffi`) et ne connaît que les éléments **reçus**, sans leurs
destinataires :

- `ot_ffi::Received` n'a ni `to` ni `outgoing` : impossible de ranger un
  reçu dans « Tous » ou dans le canal de l'expéditeur, ni d'afficher ses
  propres envois.
- `ot_core::Received` / `ReceivedFile` n'exposent pas non plus `to` (lu
  dans l'enveloppe, `crates/ot-core/src/secure.rs`), alors que le client
  TypeScript de l'extension le lit directement dans l'enveloppe.
- `send_text` et `send_file` n'écrivent rien dans l'historique.
- L'historique garde 50 éléments (Rust et `RelayHub.HISTORY_LEN`),
  l'extension 100.

La présence existe déjà : `OtClient.devices()` (utilisée par
`GroupScreen`). Aucun changement de format sur le fil ni du relais : `to`
est déjà en clair dans l'enveloppe.

## Tâches

### ✅ A1. Destinataires dans les éléments reçus (Rust)

- **Fichiers** : `crates/ot-core/src/types.rs`, `crates/ot-core/src/secure.rs`,
  `crates/ot-ffi/src/types.rs`, `crates/ot-ffi/src/client.rs`,
  `crates/ot-ffi/src/tests.rs`.
- **Faire** : ajouter `to: Vec<DeviceId>` à `ot_core::Received` et à
  l'information de fichier reçu, rempli depuis l'enveloppe ; dans
  `ot_ffi::Received`, ajouter `to: Vec<String>` (vide : tous) et
  `outgoing: bool`, `pending: bool`, tous `#[serde(default)]` pour relire un
  `history.json` ancien. Vérifier que la CLI (`ot-cli`) compile toujours et
  n'affiche rien de nouveau.
- **Tests** : un reçu adressé à moi seul porte `to == [moi]` ; un envoi à
  tous porte `to` vide ; un ancien historique sans ces champs se relit
  (comme `old_history_without_files_still_reads`).
- **Fini quand** : `cargo test --workspace` passe ; les golden et les
  vecteurs ne bougent pas.

### ✅ A2. Envois dans l'historique (Rust)

- **Fichiers** : `crates/ot-ffi/src/client.rs`, `crates/ot-ffi/src/local.rs`,
  `crates/ot-ffi/src/tests.rs`.
- **Faire** : `send_text` range l'élément (`outgoing = true`, `from` = cet
  appareil, `to`, `pending = true`) avant l'envoi, puis le passe à
  `pending = false` à l'accusé (`accepted`), et le retire si l'envoi échoue
  définitivement (erreur renvoyée à l'appelant). Même chose pour
  `send_file`, rangé une fois le fichier déposé (comme l'extension : pas de
  bulle pour un dépôt en cours, la progression reste dans `transfers`).
  Signaler l'élément ajouté ou mis à jour par un événement (par exemple
  `Event::Sent { item }`), pour que `RelayService` mette `RelayHub` à jour
  sans relire tout l'historique. Historique porté à 100 éléments.
  Ne jamais notifier un élément envoyé.
- **Tests** : un texte envoyé apparaît dans `history()`, `pending` puis
  accepté ; un fichier envoyé apparaît avec `outgoing` ; la limite de 100.
- **Fini quand** : tests Rust verts ; décision notée (note 0033) : envois
  dans l'historique Android, nouveaux champs, taille 100.

### ✅ A3. Règles pures en Kotlin

- **Fichiers** : `apps/android/app/src/main/kotlin/dev/oldtown/android/ui/Conversations.kt`
  (nouveau), `apps/android/app/src/test/kotlin/dev/oldtown/android/ConversationsTest.kt`.
- **Faire**, sans Android (testable sous JVM), en recopiant les règles de
  `apps/extension/src/app/devices.ts` et `channelOf` (`src/shared/api.ts`) :
  - `channelOf(item, me)` : envoyé → son unique destinataire, sinon `""` ;
    reçu → l'expéditeur s'il m'était adressé à moi seul, sinon `""`.
  - `deviceKind(name)` : mêmes expressions (accents et casse retirés),
    navigateur d'abord, puis téléphone, fixe, portable par défaut ; `Group`
    pour « Tous ».
  - `avatarColor(id)` : même palette de 10 couleurs, même FNV-1a 32 bits
    sur les caractères UTF-16 de l'identifiant (calcul en `Int`, modulo non
    signé) : la même couleur que dans l'extension.
  - `conversations(members, items, me)` et `preview(item, channel)`.
- **Tests** : les cas de `devices.spec.ts`, plus une couleur attendue
  calculée par l'extension pour un identifiant fixe (même résultat des deux
  côtés).
- **Fini quand** : `./gradlew test` passe.

### ✅ A4. Accueil en liste

- **Fichiers** : `ui/HomeScreen.kt`, `ui/components/Icons.kt`,
  `ui/components/Components.kt` (composant `Avatar`), `ui/MainActivity.kt`.
- **Faire** : remplacer le compositeur et la liste « Reçus » par la liste
  des conversations (`LazyColumn`, lignes pleine largeur, séparateur sous
  le texte et pas sous l'avatar, comme dans l'extension). Garder en tête de
  liste les bandeaux (`Alerts`) et la carte des transferts. Icônes au trait
  d'après Lucide à ajouter dans `OtIcons` : `Phone` (smartphone),
  `Desktop` (monitor), `Globe` (navigateur) ; `Laptop`, `Users` et `Back`
  existent. Avatar 48 dp, pictogramme blanc à 50 %. Présence : appeler
  `RelayHub.client(context).devices()` à l'affichage puis toutes les 15 s
  tant que l'écran est au premier plan et la connexion `Connected`
  (`LifecycleResumeEffect`), rien quand elle ne l'est pas. Toucher une
  ligne pousse `Screen.Conversation(channel)` sur la pile.
- **Tests** : instrumenté léger (Compose) si le coût est raisonnable :
  « Tous mes appareils » en premier, ordre par récence ; sinon couvert par
  A3.
- **Fini quand** : l'accueil ressemble à celui de l'extension, en thème
  sombre et clair.

### ✅ A5. Écran de conversation

- **Fichiers** : `ui/ConversationScreen.kt` (nouveau), `ui/MainActivity.kt`,
  `ui/HomeScreen.kt` (en sortir `ItemSheet`, `relativeTime` et le
  compositeur réutilisables).
- **Faire** : en-tête (retour, avatar 36 dp, nom, présence), liste des
  bulles du canal du plus ancien au plus récent, ancrée en bas
  (`reverseLayout` ou défilement au dernier à l'ouverture et à chaque
  nouvel élément), bulles de progression pour les envois de fichiers en
  cours, zone d'envoi en bas qui suit le clavier (`imePadding`) et envoie
  au canal (`to = []` pour « Tous », `[id]` sinon). Le retour système
  revient à l'accueil. Le canal ouvert peut devenir le destinataire par
  défaut de la feuille de partage si c'est simple (sinon, ne rien changer
  au partage et le noter).
- **Tests** : instrumenté : envoyer un texte depuis la conversation d'un
  appareil l'affiche à droite avec `to == [id]`.
- **Fini quand** : essai sur le téléphone contre le relais, ci-dessous.

### ✅ A6. Notification → conversation

- **Fichiers** : `notify/Notifications.kt`, `ui/MainActivity.kt`.
- **Faire** : toucher la notification d'un message ouvre l'appli sur la
  conversation de son canal (extra dans l'`Intent`, pas le contenu), comme
  l'extension (note 0032).
- **Fini quand** : essai manuel.

## Fin

- `./gradlew lint test assembleDebug` dans `apps/android`, et la validation
  Rust de `CLAUDE.md` (A1 et A2 touchent `ot-core` et `ot-ffi`).
- Essai : depuis l'extension, un message à « Téléphone » apparaît dans sa
  conversation sur le téléphone, un message à tous dans « Tous mes
  appareils » ; une réponse depuis le téléphone apparaît dans la
  conversation du navigateur côté extension ; les couleurs d'un même
  appareil sont identiques des deux côtés.
- `apps/android/CLAUDE.md` mis à jour (écran d'accueil, conversation,
  `Conversations.kt`), note 0033 ajoutée à `docs/decisions/README.md`.
- Hors périmètre : non-lus et badges, recherche, suppression d'un message,
  épinglage.
