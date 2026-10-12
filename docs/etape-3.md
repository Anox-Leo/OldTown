# Plan détaillé — Étape 3 : Android, partage

L'étape 3 livre l'appli Android : partager un texte ou un lien depuis n'importe
quelle appli vers mes autres appareils, recevoir une notification avec
« Copier » et « Ouvrir », et appairer en scannant un QR code : le téléphone
rejoint le groupe, puis parraine les autres appareils.

**État (12 octobre 2026)** : code livré (T0 à T10) ; précisions et écarts
dans la note [0022](decisions/0022-etape-3-wss-qr-inverse-ffi.md). Validé en
local : tests Rust, `./gradlew lint test assembleDebug`, tests instrumentés
sur émulateur Android 16, et essai de bout en bout sur émulateur. Essai réel
sur le téléphone (T11) concluant pour tous les critères, dont la
consommation : 70 mAh en 17 h d'usage (1,5 % de la batterie consommée).
**Étape terminée.**

Références : jalon « 3. Android : partage » de la [conception](design.md),
[revue après l'étape 1](revue-etapes-0-1.md), note 0009 (un seul processus
tient la connexion), 0013 (QR dans les deux sens), 0015 (`wss`), 0016
(service, pas de tuile, zxing-cpp).

**Prérequis** : étape 2 terminée (chiffrement, roster, appairage par code,
trait `Keyring`).

## Objectif et périmètre

**Résultat attendu** : dans Chrome sur le téléphone, Partager → OldTown →
« Mac » ; le lien s'affiche sur le Mac (`ot listen`). Depuis le Mac,
`ot send "code 4821"` fait apparaître une notification sur le téléphone ; le
bouton « Copier » met le texte dans le presse-papiers. Téléphone verrouillé
une nuit entière : les messages arrivent toujours au réveil, sans vider la
batterie.

**Inclus**

- Appli Kotlin + Jetpack Compose dans `apps/android`, Android 13 minimum.
- Pont Rust : crate `ot-ffi` (UniFFI) au-dessus de `ot-core`, compilée pour
  Android avec `cargo-ndk`.
- Service de premier plan qui tient l'unique connexion de l'appareil.
- Cible de partage (texte, lien), raccourcis Direct Share par appareil.
- Notifications de réception avec actions, marquage des contenus sensibles.
- Relais en `wss`, certificat `tailscale cert` (note 0015).
- Appairage par QR dans les deux sens (note 0013) : le téléphone rejoint en
  scannant le code affiché par `ot pair new --qr`, puis parraine les autres
  appareils en scannant le QR qu'ils affichent (QR inversé).
- Clés protégées par le Keystore Android.

**Exclus**

- Fichiers et images partagés (étape 5) : l'appli affiche « bientôt ».
- Presse-papiers automatique (impossible en arrière-plan sur Android) et
  historique consultable (étape 8). L'appli garde seulement les 50 derniers
  éléments reçus.
- UnifiedPush (étape 8, ou plus tôt si la batterie le demande).
- Publication sur le Play Store : l'APK s'installe à la main.

## Architecture

```text
┌──────────────── Appli Android (un seul processus) ────────────────┐
│ ShareActivity ─┬──▶ RelayService (premier plan) ──▶ ot-ffi ──▶ ot-core ──▶ relais
│ MainActivity  ─┘        │                                          │
│                         └──▶ Notifications, presse-papiers         │
│ SecretStore (Kotlin, Keystore AES-GCM) ◀── appelé par ot-core      │
└────────────────────────────────────────────────────────────────────┘
```

- **Un seul propriétaire de la connexion** : `RelayService`. Les activités
  (partage, écran principal) lui parlent par un service lié ou un
  `Intent` ; elles n'ouvrent jamais de connexion (note 0009).
- **Tout le protocole et la crypto restent en Rust.** Kotlin ne voit que des
  types simples : `Content`, `Received`, `Device`, `Event`.
- **Clés** : `ot-core` reçoit un `SecretStore` (interface étrangère UniFFI,
  implémentée en Kotlin). Les secrets sont chiffrés par une clé AES-GCM non
  exportable du Keystore Android et rangés dans le stockage privé de l'appli.
  Le Keystore ne tient pas directement les clés Ed25519/X25519 : le cœur Rust
  doit pouvoir s'en servir.
- **Réseau** : `wss` vers le relais (note 0015), TLS fait par `rustls` dans
  le code Rust. La configuration réseau de l'appli interdit le clair pour tout
  le code Java.

## Pont Rust : crate `ot-ffi`

| Exposé à Kotlin | Forme | Rôle |
| --- | --- | --- |
| `OtClient::start(config, secret_store)` | constructeur | Démarre le client en tâche de fond (runtime tokio interne) |
| `send(content, to)` | `suspend` | Chiffre et envoie ; attend `accepted` |
| `next_event()` | `suspend` | Prochain `Event` (message, connexion, groupe changé) |
| `devices()`, `roster()` | `suspend` | Liste du relais et membres du groupe |
| `pair_new()`, `pair_join(code)` | `suspend` | Appairage (étape 2) |
| `reconnect_now()` | simple | Appelé quand le réseau revient, pour ne pas attendre le backoff |
| `shutdown()` | `suspend` | Arrêt propre |
| `SecretStore` | interface étrangère | `get(name)`, `put(name, bytes)`, `delete(name)` |

- UniFFI en mode macros (`#[uniffi::export]`), sans fichier UDL.
- Cibles : `aarch64-linux-android` (téléphone), `x86_64-linux-android`
  (émulateur). `cargo-ndk` est appelé par une tâche Gradle ; les liaisons
  Kotlin sont générées par `uniffi-bindgen` au build, jamais versionnées.
- `ot-ffi` ne sert qu'à Android (et à iOS un jour) : l'extension Chrome
  (étape 4) passe par `ot-wasm`.

## Détails Android

| Sujet | Choix | Remarque |
| --- | --- | --- |
| Versions | `minSdk 33` (Android 13), `targetSdk` = dernière version stable | Un seul téléphone : pas de compatibilité ancienne à porter |
| Service | `foregroundServiceType="specialUse"` avec la propriété de justification (note 0016) | `dataSync` est limité à 6 h par jour à partir d'Android 15. Au démarrage de l'étape, vérifier sur la doc Android si `remoteMessaging` conviendrait (ni limite de durée, ni restriction au boot) et noter le résultat |
| Démarrage | Au lancement de l'appli et au boot (`BOOT_COMPLETED`) | Vérifier que le type retenu peut démarrer depuis le boot sur la version cible |
| Batterie | Demander l'exemption d'optimisation (`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`) | Autorisé hors Play Store |
| Réseau | `ConnectivityManager.NetworkCallback` → `reconnect_now()` | Passage Wi-Fi ↔ 4G |
| Notification permanente | « Connecté », « Reconnexion… », « Tailscale injoignable ? » | Canal à faible importance |
| Réception | Canal « Messages » ; actions Copier, Ouvrir (lien), Partager | Copier se fait dans un `BroadcastReceiver` : l'écriture du presse-papiers est permise en arrière-plan |
| Contenu sensible | `ClipDescription.EXTRA_IS_SENSITIVE` quand le contenu est marqué sensible | Le marquage arrive vraiment avec l'étape 6 ; prévoir le champ |
| Partage | `ShareActivity` transparente pour `ACTION_SEND` `text/plain` ; choix du destinataire en feuille du bas | Les autres types MIME sont acceptés mais répondent « fichiers : étape 5 » |
| Direct Share | Un raccourci dynamique par appareil du roster (`ShortcutManagerCompat`) | Mis à jour à chaque `GroupChanged` |
| QR | Lecture par zxing-cpp (Apache 2.0, sans Google Play Services) via CameraX ; saisie du code en secours (note 0016) | ML Kit écarté : dépendance Google |
| QR (contenu) | Rejoindre : `oldtown://pair?s=<url du relais>&c=<code ot1-…>` ; parrainer : QR inversé affiché par le nouvel appareil (nom, clés publiques, secret, signature) | Même code que l'étape 2 ; format du QR inversé figé par T9 (note 0013) |
| Dédoublonnage | Les `msg_id` vus sont persistés dans le stockage privé | Remplit `ClientConfig.seen` au démarrage |
| Signature de l'APK | Clé personnelle hors dépôt (`private/`), installation par `adb install` | Jamais de keystore versionné |

## Découpage en tâches

Chaque tâche finit par les validations de `CLAUDE.md` (Rust) et par
`./gradlew lint test assembleDebug` dans `apps/android` (Android).

- [x] **T0. Fins de ligne des fichiers figés (CI Windows rouge depuis
  l'étape 2 ; à faire en premier).**
  Fichiers : `.gitattributes` (nouveau).
  Constat : `frozen_vectors` (`crates/ot-crypto/tests/vectors.rs`) échoue sur
  `windows-latest` : le checkout convertit `tests/vectors/*.json` en CRLF
  (`core.autocrlf`, pas de `.gitattributes`) alors que le test compare octet
  par octet avec une sortie en LF. Le format chiffré n'a pas changé.
  Faire : `crates/*/tests/**/*.json text eol=lf` dans `.gitattributes` ; ne
  pas normaliser les fins de ligne dans le test (le contrôle du format figé
  resterait affaibli).
  Fini quand : le job `Tests (windows-latest)` passe, et les jobs qui en
  dépendent (vérification `ot-server` sans `ot-crypto`, binaire `ot`) tournent.

- [x] **T1. `SecretStore` dans `ot-core`.**
  Fichiers : `crates/ot-core/src/keyring.rs`, `crates/ot-cli/src/files.rs`.
  Faire : faire reposer le `Keyring` de l'étape 2 sur un trait `SecretStore`
  minimal (`get`/`put`/`delete` d'octets nommés) ; la CLI l'implémente sur ses
  fichiers `0600`.
  Tests : les tests de l'étape 2 passent sans changement de comportement.

- [x] **T2. Crate `ot-ffi`.**
  Fichiers : `crates/ot-ffi/` (nouvelle), `Cargo.toml`, `deny.toml`.
  Faire : l'API du tableau ci-dessus avec UniFFI ; runtime tokio interne ;
  erreurs converties en une énumération simple (`Offline`, `AuthFailed`,
  `NotInGroup`, `Pairing`, `Internal`).
  Tests : test Rust qui appelle l'API exportée contre un relais en processus ;
  `uniffi-bindgen generate` produit du Kotlin sans erreur (en CI).
  Fini quand : `ot-ffi` compile pour `aarch64-linux-android` avec `cargo ndk`.

- [x] **T3. Squelette de l'appli et build.**
  Fichiers : `apps/android/` (Gradle Kotlin DSL, catalogue de versions),
  `apps/android/CLAUDE.md`, `.github/workflows/android.yml`.
  Faire : projet Compose vide, tâche Gradle qui appelle `cargo ndk` puis
  `uniffi-bindgen`, chargement de la bibliothèque native ; workflow CI filtré
  sur `apps/android/**` et `crates/**`, Linux seulement, qui produit un APK de
  debug.
  Fini quand : l'APK de debug s'installe et affiche la version du cœur Rust.

- [x] **T4. `SecretStore` Keystore.**
  Fichiers : `apps/android/app/src/main/java/.../secrets/`.
  Faire : clé AES-256-GCM non exportable dans le Keystore, secrets chiffrés
  dans le stockage privé (`noBackupFilesDir`, exclus des sauvegardes Android).
  Tests : test instrumenté aller-retour ; un secret effacé disparaît.

- [x] **T5. `RelayService`.** *(Code et tests instrumentés faits ; l'essai
  d'une nuit fait partie de T11.)*
  Faire : service de premier plan `specialUse`, notification permanente avec
  l'état, démarrage au boot, `NetworkCallback`, boucle `next_event()`,
  persistance des `msg_id` vus, demande d'exemption batterie.
  Tests : test instrumenté de démarrage et d'arrêt ; essai manuel d'une nuit.

- [x] **T6. Notifications de réception.**
  Faire : canal « Messages », actions Copier / Ouvrir / Partager, groupe de
  notifications, liste des 50 derniers éléments dans l'écran principal.
  Tests : test unitaire du choix des actions selon le `kind`.

- [x] **T7. Cible de partage et Direct Share.**
  Faire : `ShareActivity`, feuille de choix du destinataire (tous ou un
  membre), envoi par le service, confirmation brève ; raccourcis par appareil.
  Tests : test instrumenté d'un `Intent` `ACTION_SEND`.

- [x] **T8. `wss` sur le relais et dans `ot-core`** (note 0015 ; à faire
  avant T5).
  Fichiers : `crates/ot-server/src/main.rs`, `src/lib.rs`,
  `crates/ot-core/src/client.rs`, `deploy/` (minuterie de renouvellement),
  `README.md`, `SECURITY.md`.
  Faire : TLS optionnel côté relais (`rustls`, chemins du certificat et de la
  clé en configuration, rechargement du certificat renouvelé sans
  redémarrage) ; `wss://` côté `ot-core` et CLI, vérification par les
  autorités de Mozilla (`webpki-roots`) ; minuterie systemd qui lance
  `tailscale cert` (installée par moi avec `sudo`). Aucun nom de machine
  dans le dépôt : exemples en `relais`.
  Tests : connexion `wss` avec un certificat de test ; certificat invalide
  refusé ; rechargement du certificat ; `ws://` toujours possible sans
  configuration TLS.

- [x] **T9. Appairage par QR, dans les deux sens** (note 0013).
  Fichiers : `crates/ot-proto/src/frame.rs`, `tests/golden/`,
  `crates/ot-server/src/session.rs`, `src/store.rs`, `crates/ot-crypto/`
  (vecteurs), `crates/ot-core/src/pairing.rs`, `crates/ot-cli/src/main.rs`
  (options `--qr` de `ot pair new` et `ot pair join`, QR en terminal),
  `apps/android/.../pairing/`, note de décision sur le format.
  Faire : rejoindre en scannant `oldtown://pair` (ou en collant le code) ;
  parrainer par QR inversé : le nouvel appareil affiche nom, clés publiques,
  secret `S` et signature ; le téléphone scanne, fait enregistrer l'appareil
  par sa session authentifiée (nouvelle trame), puis lui envoie clé de groupe
  et roster authentifiés par `S` et scellés pour sa clé X25519 ; le nouvel
  appareil réessaie `auth` jusqu'à être enregistré (10 min au plus). Écran
  « Groupe » (membres, empreintes, rotation).
  Tests : analyse de l'URI `oldtown://pair` et du QR inversé (cas valides et
  invalides) ; E2E : le téléphone simulé parraine une CLI par QR inversé ; un
  relais qui altère l'envoi ou enregistre une autre clé fait échouer
  l'appairage ; vecteurs figés pour le nouvel envoi.
  Fini quand : un PC rejoint le groupe en affichant un QR scanné par le
  téléphone, sans rien taper.

- [x] **T10. Réglages et état.** *(Le nom de l'appareil est affiché mais
  fixé à l'appairage : le roster ne prévoit pas de renommage, note 0022.)*
  Faire : nom de l'appareil, URL du relais, état de connexion avec aide
  « vérifier Tailscale », version.

- [x] **T11. Essai réel et documentation.**
  Fichiers : `README.md`, `docs/etape-3.md` (état), notes de décision.
  Faire : installer, appairer depuis le Mac, dérouler le tableau des critères.

## Critères de fin

| Scénario | Attendu | Vérifié par | Résultat (12 oct. 2026) |
| --- | --- | --- | --- |
| Partager un lien depuis Chrome | Arrive sur le Mac en moins de 2 s | Essai réel | OK, aussi par le raccourci Direct Share |
| Texte reçu, téléphone verrouillé | Notification ; Copier remplit le presse-papiers | Essai réel | OK |
| `wss` | Le téléphone se connecte en TLS ; un certificat invalide est refusé | Test + essai réel | OK (connexion en `wss` ; refus couvert par les tests) |
| Appairage par QR | Le téléphone rejoint le groupe sans SSH ni saisie | Essai réel | OK |
| QR inversé | Un PC rejoint en affichant un QR scanné par le téléphone ; relais qui triche → échec | E2E + essai réel | OK (« Groupe rejoint » ; relais tricheur couvert par l'E2E) |
| Nuit entière en veille | Aucun message perdu ; consommation acceptable (noter le pourcentage) | Essai réel, résultat noté ici | OK : 70 mAh en 17 h d'usage actif et passif (plus de 10 messages reçus et envoyés), 1,5 % de la consommation totale de la batterie ; aucune perte signalée |
| Reboot du téléphone | Le service repart seul | Essai réel | OK |
| Wi-Fi → 4G | Reconnexion en moins de 10 s | Essai réel | OK |
| Clés | Absentes des sauvegardes Android et des logs | Revue + test | OK (`allowBackup=false`, `noBackupFilesDir` à T4 ; logs Android limités aux erreurs, sans contenu) |
| CI | APK de debug construit, tests Kotlin et Rust verts | CI | OK (jobs Android et CI verts sur `main`) |

## Risques

- **Batterie et Doze** : si la connexion permanente coûte trop, avancer
  UnifiedPush (étape 8) plutôt que d'affaiblir la livraison.
- **Évolution des règles Android** sur les services de premier plan : relire
  la doc officielle des types de service au démarrage de l'étape et noter
  l'écart dans `decisions/`.
- **Taille de l'APK** : la bibliothèque Rust pèse quelques Mo ; compiler en
  `release` avec `lto` et `strip`.
