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
  Share par membre (`share/Shortcuts`). Destinataires : « Tous » en tête,
  puis les appareils du plus utilisé au moins utilisé (`byUsage`, note
  0033), raccourcis rangés de même.
- **Conversations** (note 0033, comme l'extension) : l'accueil
  (`ui/HomeScreen`) liste « Tous mes appareils » puis un appareil par
  ligne, du plus récemment actif au moins récent, avec présence (relue
  toutes les 15 s au premier plan) ; `ui/ConversationScreen` montre les
  bulles du canal (envois à droite) et envoie dans ce canal ; une
  notification ouvre la conversation de son canal (`EXTRA_CHANNEL`). Règles
  pures dans `ui/Conversations.kt` (`channelOf`, `deviceKind`,
  `avatarColor`, `conversations`, `byUsage`), recopiées de l'extension et
  testées sous JVM : les garder identiques à `devices.ts` et `channelOf`.
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
- **UI** : thème de T3 Code (`ui/theme`, sombre et clair), composants
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
