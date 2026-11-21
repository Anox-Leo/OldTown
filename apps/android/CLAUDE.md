# apps/android

Appli Android (étape 3) : Kotlin, Jetpack Compose, `minSdk` 33, `targetSdk`
et `compileSdk` 37. Tout le protocole et la crypto restent en Rust
(`crates/ot-ffi`, UniFFI) ; Kotlin ne fait qu'afficher, stocker et relayer.

- **Build** : `./gradlew lint test assembleDebug` (et
  `connectedDebugAndroidTest` avec un téléphone ou un émulateur). Gradle
  appelle `cargo ndk` (tâche `cargoNdk`, profil `android`) puis
  `uniffi-bindgen` (tâche `uniffiBindings`, liaisons dans
  `app/build/generated/uniffi`, **jamais versionnées**). Prérequis : JDK 21,
  SDK API 37, NDK r29 (`ndkVersion`), cibles Rust Android, `cargo-ndk`.
  `local.properties` (non versionné) : `sdk.dir`, `cargo.dir`,
  `oldtown.abis`, `oldtown.signing.*`.
- **Une seule connexion** (note 0009) : `relay/RelayService` est le seul à
  créer un `OtClient`. Les écrans, la feuille de partage et les
  notifications passent par `relay/RelayHub` (état observable, `client()`,
  `send()`), jamais par un client à eux.
- **Secrets** : `secrets/KeystoreSecretStore` (AES-256-GCM, clé non
  exportable du Keystore, fichiers dans `noBackupFilesDir`, nom en donnée
  associée). Rien n'est sauvegardé par Android (`allowBackup=false`, règles
  d'extraction). Jamais de contenu, de clé ni de code d'appairage dans
  `Log`.
- **Service** `specialUse` (note 0016, 0022), sans type sous Android 13 ;
  démarré à l'ouverture et au boot ; `NetworkWatcher` appelle
  `reconnectNow()` quand le réseau par défaut change.
- **Notifications** : canal « Connexion » (permanente, discrète) et
  « Messages » ; actions choisies par `notify/actionsFor` (testée sans
  Android). « Copier » relit l'élément par son `msg_id` : le texte ne passe
  pas par l'`Intent`. « Ouvrir » seulement pour `http(s)://`.
- **Partage** : `share/ShareActivity` (feuille du bas : texte, ou fichiers
  `SEND`/`SEND_MULTIPLE`, URI `content://` seulement) et un raccourci Direct
  Share par destinataire (`share/Shortcuts`, identifiant `to:<canal>`).
  Destinataires : « Tous » en tête, puis mes appareils (sans ceux en ligne
  de commande), puis les autres personnes (sans les services), chacun du
  plus utilisé au moins utilisé (`shareTargets`, note 0033), raccourcis
  rangés de même.
- **Conversations** (note 0033, comme l'extension) : l'accueil
  (`ui/HomeScreen`) liste « Tous mes appareils » puis un appareil par
  ligne, du plus récemment actif au moins récent, avec présence (relue
  toutes les 15 s au premier plan) ; `ui/ConversationScreen` montre les
  bulles du canal (envois à droite) et envoie dans ce canal ; une
  notification ouvre la conversation de son canal (`EXTRA_CHANNEL`). Règles
  pures dans `ui/Conversations.kt` (`channelOf`, `inChannel`,
  `deviceKind`, `avatarColor`, `conversations`, `personConversations`,
  `railPeople`, `unread`, `initials`, `shareTargets`), recopiées de
  l'extension et testées sous JVM avec les cas de `people.spec.ts` : les
  garder identiques à `devices.ts`, `channelOf` et `inChannel`. La catégorie
  d'un appareil est celle qu'il déclare dans l'annuaire (note 0068), jamais
  devinée au nom.
- **Personnes** (étape 10) : `RelayHub` tient l'annuaire vérifié
  (`people`, relu à chaque `DirectoryChanged`), les non-lus (`opened.json`,
  rangé par `ot-ffi`), les alertes de l'annuaire et la racine à publier
  (D51 : adoptée d'office, publiée sur « Publier l'annuaire »). Barre
  latérale de 72 dp (`ui/Rail`) dès qu'il y a une autre personne ou pour
  l'admin ; canal `user:<id>` ou `user:<id>/<appareil>` ; un service n'a
  que sa conversation, sans zone d'envoi. Notifications des autres
  personnes sur leur canal (« Autres personnes »), titre « Bob · bob-phone ».
  Réglages : `PeopleSettings`, `MyDevicesScreen` ; administration dans
  `ui/admin` (personnes, inviter par QR ou lien, liens en interrupteurs,
  appareil de service). Le lien `oldtown://invite` ouvre « J'ai reçu une
  invitation » sans rien envoyer avant un appui.
  L'historique (100 éléments) contient aussi les envois (`outgoing`,
  `pending`), signalés par `Event.Sent` / `Event.Unsent`.
- **Fichiers** (étape 5, notes 0027 et 0028) : envoi par le menu *Partager*
  ou le trombone d'une conversation (sélecteur `OpenMultipleDocuments`) ;
  les transferts tournent dans `RelayService` (actions `SEND_FILES`,
  `DOWNLOAD`), qui garde l'accès aux URI
  partagées et survit aux écrans ; `files/` : `UriSource` (lecture à un
  décalage par un `ContentResolver`), `DownloadSink` (MediaStore,
  `Download/OldTown`, `IS_PENDING` jusqu'à la vérification), `FileTransfers`
  (progression, notification « Transferts »), règles pures `FileRules`
  (téléchargement d'office sous 20 Mo sur réseau non facturé).
- **UI** : thème « nuit » (sombre) et « parchemin » (clair) de la charte
  (`ui/theme`) : `Color.kt` et `Theme.kt` lisent `Tokens.kt`, les
  ressources XML lisent `res/values*/tokens.xml`, tous deux générés par
  `tools/design-tokens/gen.mjs` depuis `design/tokens.json` (jamais modifiés à
  la main, aucune couleur en dur dans un écran hors QR). Signe et logotype :
  `ui/components/Brand.kt` (`Mark`, `Logo`) et les icônes de `res/drawable`,
  tracés recopiés de `branding/` et vérifiés par `BrandTest`. Composants
  maison dans `ui/components` (pas de Material Icons : icônes au trait
  d'après Lucide), textes en français. Navigation par une pile d'écrans
  dans `MainActivity`.
- **Tests** : unitaires JVM dans `app/src/test` (logique pure : actions,
  partage, règles des fichiers, conversations) ; instrumentés dans `app/src/androidTest`
  (Keystore, service, feuille de partage, écriture MediaStore). Ce qui
  touche au protocole se teste en Rust. Les tests instrumentés tournent avec
  les vraies données de l'appli : un test qui oublie ou remplace l'appareil
  commence par `assumeNoRealDevice` (sauté sur un téléphone appairé), et son
  `@After` n'efface rien si le test a été sauté.
- Nouvelle dépendance : licence compatible, version dans
  `gradle/libs.versions.toml`, justifiée dans le compte rendu. Polices et
  icônes tierces : licence dans `third_party/`.
- CI : `.github/workflows/android.yml` (Linux, filtré sur `apps/android/**`
  et `crates/**`), produit l'APK de debug.
- **Pas de commentaires** (note 0072, comme l'extension, note 0066) : le
  pourquoi va dans une note de `docs/decisions/`, le quoi dans les noms et
  les tests, un raccourci dans une issue. `NoCommentsTest` (JVM) échoue sur
  tout commentaire de Kotlin, de `.kts`, de XML, de TOML, de `.properties`
  ou de `.pro` sous `apps/android`, sauf dans les fichiers générés
  (`Tokens.kt`, `res/values*/tokens.xml`, à corriger dans
  `tools/design-tokens/gen.mjs`). Pour trier ce qui reviendrait :
  `/delete-comment`.
