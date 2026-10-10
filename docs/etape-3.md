# Plan détaillé — Étape 3 : Android, partage

L'étape 3 livre l'appli Android : partager un texte ou un lien depuis n'importe
quelle appli vers mes autres appareils, recevoir une notification avec
« Copier » et « Ouvrir », envoyer le presse-papiers depuis une tuile, et
appairer le téléphone en scannant un QR code.

Références : jalon « 3. Android : partage » de la [conception](design.md),
[revue après l'étape 1](revue-etapes-0-1.md), note 0009 (un seul processus
tient la connexion), décisions D13 à D18 de
[`decisions-a-prendre.md`](decisions-a-prendre.md) (ce plan applique leurs
recommandations).

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
- Tuile Quick Settings « Envoyer le presse-papiers » (D15).
- Appairage par QR (scanner un code affiché par `ot pair new --qr`, ou
  afficher un QR pour parrainer un autre appareil).
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
│ ShareActivity ─┐                                                   │
│ TileActivity  ─┼──▶ RelayService (premier plan) ──▶ ot-ffi ──▶ ot-core ──▶ relais
│ MainActivity  ─┘        │                                          │
│                         └──▶ Notifications, presse-papiers         │
│ SecretStore (Kotlin, Keystore AES-GCM) ◀── appelé par ot-core      │
└────────────────────────────────────────────────────────────────────┘
```

- **Un seul propriétaire de la connexion** : `RelayService`. Les activités
  (partage, tuile, écran principal) lui parlent par un service lié ou un
  `Intent` ; elles n'ouvrent jamais de connexion (note 0009).
- **Tout le protocole et la crypto restent en Rust.** Kotlin ne voit que des
  types simples : `Content`, `Received`, `Device`, `Event`.
- **Clés** : `ot-core` reçoit un `SecretStore` (interface étrangère UniFFI,
  implémentée en Kotlin). Les secrets sont chiffrés par une clé AES-GCM non
  exportable du Keystore Android et rangés dans le stockage privé de l'appli.
  Le Keystore ne tient pas directement les clés Ed25519/X25519 : le cœur Rust
  doit pouvoir s'en servir.
- **Réseau** : la connexion part du code Rust (sockets natives), donc la
  politique « trafic en clair interdit » d'Android, qui ne s'applique qu'aux
  piles réseau Java, ne bloque pas `ws://`. La configuration réseau de l'appli
  interdit quand même le clair pour tout le code Java (D13).

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
- `ot-ffi` ne sert qu'à Android (et à iOS un jour) : l'appli desktop Tauri
  appelle `ot-core` directement en Rust.

## Détails Android

| Sujet | Choix | Remarque |
| --- | --- | --- |
| Versions | `minSdk 33` (Android 13), `targetSdk` = dernière version stable | Un seul téléphone : pas de compatibilité ancienne à porter |
| Service | `foregroundServiceType="specialUse"` avec la propriété de justification (D14) | `dataSync` est limité à 6 h par jour à partir d'Android 15 ; à revérifier au démarrage de l'étape sur la doc Android |
| Démarrage | Au lancement de l'appli et au boot (`BOOT_COMPLETED`) | Vérifier que le type retenu peut démarrer depuis le boot sur la version cible |
| Batterie | Demander l'exemption d'optimisation (`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`) | Autorisé hors Play Store |
| Réseau | `ConnectivityManager.NetworkCallback` → `reconnect_now()` | Passage Wi-Fi ↔ 4G |
| Notification permanente | « Connecté », « Reconnexion… », « Tailscale injoignable ? » | Canal à faible importance |
| Réception | Canal « Messages » ; actions Copier, Ouvrir (lien), Partager | Copier se fait dans un `BroadcastReceiver` : l'écriture du presse-papiers est permise en arrière-plan |
| Contenu sensible | `ClipDescription.EXTRA_IS_SENSITIVE` quand le contenu est marqué sensible | Le marquage arrive vraiment avec l'étape 6 ; prévoir le champ |
| Partage | `ShareActivity` transparente pour `ACTION_SEND` `text/plain` ; choix du destinataire en feuille du bas | Les autres types MIME sont acceptés mais répondent « fichiers : étape 5 » |
| Direct Share | Un raccourci dynamique par appareil du roster (`ShortcutManagerCompat`) | Mis à jour à chaque `GroupChanged` |
| Tuile | `TileService` qui ouvre une activité transparente ; elle lit le presse-papiers quand elle a le focus, envoie, se ferme | Android 14+ : `startActivityAndCollapse(PendingIntent)` |
| QR | Lecture par zxing-cpp (Apache 2.0, sans Google Play Services) via CameraX ; saisie du code en secours (D18) | ML Kit écarté : dépendance Google |
| QR (contenu) | `oldtown://pair?s=<url du relais>&c=<code ot1-…>` | Même code que l'étape 2 |
| Dédoublonnage | Les `msg_id` vus sont persistés dans le stockage privé | Remplit `ClientConfig.seen` au démarrage |
| Signature de l'APK | Clé personnelle hors dépôt (`private/`), installation par `adb install` | Jamais de keystore versionné |

## Découpage en tâches

Chaque tâche finit par les validations de `CLAUDE.md` (Rust) et par
`./gradlew lint test assembleDebug` dans `apps/android` (Android).

- [ ] **T1. `SecretStore` dans `ot-core`.**
  Fichiers : `crates/ot-core/src/keyring.rs`, `crates/ot-cli/src/files.rs`.
  Faire : faire reposer le `Keyring` de l'étape 2 sur un trait `SecretStore`
  minimal (`get`/`put`/`delete` d'octets nommés) ; la CLI l'implémente sur ses
  fichiers `0600`.
  Tests : les tests de l'étape 2 passent sans changement de comportement.

- [ ] **T2. Crate `ot-ffi`.**
  Fichiers : `crates/ot-ffi/` (nouvelle), `Cargo.toml`, `deny.toml`.
  Faire : l'API du tableau ci-dessus avec UniFFI ; runtime tokio interne ;
  erreurs converties en une énumération simple (`Offline`, `AuthFailed`,
  `NotInGroup`, `Pairing`, `Internal`).
  Tests : test Rust qui appelle l'API exportée contre un relais en processus ;
  `uniffi-bindgen generate` produit du Kotlin sans erreur (en CI).
  Fini quand : `ot-ffi` compile pour `aarch64-linux-android` avec `cargo ndk`.

- [ ] **T3. Squelette de l'appli et build.**
  Fichiers : `apps/android/` (Gradle Kotlin DSL, catalogue de versions),
  `apps/android/CLAUDE.md`, `.github/workflows/android.yml`.
  Faire : projet Compose vide, tâche Gradle qui appelle `cargo ndk` puis
  `uniffi-bindgen`, chargement de la bibliothèque native ; workflow CI filtré
  sur `apps/android/**` et `crates/**`, Linux seulement, qui produit un APK de
  debug.
  Fini quand : l'APK de debug s'installe et affiche la version du cœur Rust.

- [ ] **T4. `SecretStore` Keystore.**
  Fichiers : `apps/android/app/src/main/java/.../secrets/`.
  Faire : clé AES-256-GCM non exportable dans le Keystore, secrets chiffrés
  dans le stockage privé (`noBackupFilesDir`, exclus des sauvegardes Android).
  Tests : test instrumenté aller-retour ; un secret effacé disparaît.

- [ ] **T5. `RelayService`.**
  Faire : service de premier plan `specialUse`, notification permanente avec
  l'état, démarrage au boot, `NetworkCallback`, boucle `next_event()`,
  persistance des `msg_id` vus, demande d'exemption batterie.
  Tests : test instrumenté de démarrage et d'arrêt ; essai manuel d'une nuit.

- [ ] **T6. Notifications de réception.**
  Faire : canal « Messages », actions Copier / Ouvrir / Partager, groupe de
  notifications, liste des 50 derniers éléments dans l'écran principal.
  Tests : test unitaire du choix des actions selon le `kind`.

- [ ] **T7. Cible de partage et Direct Share.**
  Faire : `ShareActivity`, feuille de choix du destinataire (tous ou un
  membre), envoi par le service, confirmation brève ; raccourcis par appareil.
  Tests : test instrumenté d'un `Intent` `ACTION_SEND`.

- [ ] **T8. Tuile « Envoyer le presse-papiers ».**
  Faire : `TileService` + activité transparente ; si le presse-papiers est
  vide ou non textuel, message clair.
  Tests : essai manuel sur le téléphone (Android 13) et l'émulateur récent.

- [ ] **T9. Appairage par QR.**
  Fichiers : `crates/ot-cli/src/main.rs` (option `--qr` qui affiche le QR en
  terminal), `apps/android/.../pairing/`.
  Faire : écran d'accueil « Rejoindre » (scanner ou coller le code) ; écran
  « Ajouter un appareil » qui affiche un QR et attend la demande ; écran
  « Groupe » (membres, empreintes, rotation).
  Tests : analyse de l'URI `oldtown://pair` (cas valides et invalides).

- [ ] **T10. Réglages et état.**
  Faire : nom de l'appareil, URL du relais, état de connexion avec aide
  « vérifier Tailscale », version.

- [ ] **T11. Essai réel et documentation.**
  Fichiers : `README.md`, `docs/etape-3.md` (état), notes de décision.
  Faire : installer, appairer depuis le Mac, dérouler le tableau des critères.

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Partager un lien depuis Chrome | Arrive sur le Mac en moins de 2 s | Essai réel |
| Texte reçu, téléphone verrouillé | Notification ; Copier remplit le presse-papiers | Essai réel |
| Tuile | Le presse-papiers du téléphone part vers tous les autres | Essai réel |
| Appairage par QR | Le téléphone rejoint le groupe sans SSH ni saisie | Essai réel |
| Nuit entière en veille | Aucun message perdu ; consommation acceptable (noter le pourcentage) | Essai réel, résultat noté ici |
| Reboot du téléphone | Le service repart seul | Essai réel |
| Wi-Fi → 4G | Reconnexion en moins de 10 s | Essai réel |
| Clés | Absentes des sauvegardes Android et des logs | Revue + test |
| CI | APK de debug construit, tests Kotlin et Rust verts | CI |

## Risques

- **Batterie et Doze** : si la connexion permanente coûte trop, avancer
  UnifiedPush (étape 8) plutôt que d'affaiblir la livraison.
- **Évolution des règles Android** sur les services de premier plan : relire
  la doc officielle des types de service au démarrage de l'étape et noter
  l'écart dans `decisions/`.
- **Taille de l'APK** : la bibliothèque Rust pèse quelques Mo ; compiler en
  `release` avec `lto` et `strip`.
