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
- **Partage** : `share/ShareActivity` (feuille du bas, `text/plain` ;
  fichiers refusés jusqu'à l'étape 5) et un raccourci Direct Share par membre
  (`share/Shortcuts`).
- **UI** : thème de T3 Code (`ui/theme`, sombre et clair), composants
  maison dans `ui/components` (pas de Material Icons : icônes au trait
  d'après Lucide), textes en français. Navigation par une pile d'écrans
  dans `MainActivity`.
- **Tests** : unitaires JVM dans `app/src/test` (logique pure : actions,
  partage) ; instrumentés dans `app/src/androidTest` (Keystore, service,
  feuille de partage). Ce qui touche au protocole se teste en Rust.
- Nouvelle dépendance : licence compatible, version dans
  `gradle/libs.versions.toml`, justifiée dans le compte rendu. Polices et
  icônes tierces : licence dans `third_party/`.
- CI : `.github/workflows/android.yml` (Linux, filtré sur `apps/android/**`
  et `crates/**`), produit l'APK de debug.
