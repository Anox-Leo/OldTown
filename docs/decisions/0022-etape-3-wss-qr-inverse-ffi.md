# 0022 — Étape 3 : `wss`, QR inversé, pont UniFFI et appli Android

**Statut : acceptée** (11 octobre 2026). Tâches T1 à T10 de l'étape 3.

**Contexte.** Le plan de l'étape 3 et les notes 0013, 0015 et 0016 laissent à
figer le format du QR inversé, la construction de l'envoi authentifié par `S`,
la trame d'enregistrement par le parrain, et plusieurs choix techniques
(fournisseur TLS, licences, type de service Android). Le codage a aussi fait
apparaître un défaut de l'étape 2 et quelques écarts au plan.

**Décision.**

- **Service Android** (vérification demandée par 0016, doc Android du
  11 octobre 2026) : `specialUse` et `remoteMessaging` n'ont ni limite de
  durée (seuls `dataSync` et `mediaProcessing` sont limités à 6 h par 24 h
  pour une appli qui cible Android 15 ou plus) ni interdiction de démarrer
  depuis `BOOT_COMPLETED` (liste interdite : `dataSync`, `camera`,
  `mediaPlayback`, `phoneCall`, `mediaProjection`, `microphone`). On garde
  `specialUse` ; `remoteMessaging` reste possible en ne changeant que le
  manifeste. Android 13 ne connaît pas `specialUse` : le service y démarre
  sans type.
- **TLS** : rustls 0.23 avec le fournisseur `ring` (pas `aws-lc-rs`, qui
  demande cmake et complique la compilation pour Android). Client :
  autorités de Mozilla (`webpki-roots`), plus des racines en option
  (`TlsRoots::with_extra`, tests). Relais : `--tls-cert`/`--tls-key`
  (`OT_TLS_CERT`/`OT_TLS_KEY`), PEM ; certificat relu si sa date de
  modification change (vérification toutes les 10 min ; l'ancien reste en
  service si la lecture échoue) ; poignées de main hors de la boucle
  d'accueil (10 s au plus) ; ALPN `http/1.1`. `ws://` reste possible sans
  certificat. Renouvellement : `deploy/ot-tls-renew.{service,timer}`, nom du
  serveur dans `/etc/oldtown/tls.env`, jamais dans le dépôt.
- **Licences** (`deny.toml`) : ISC ajoutée à la liste (ring, rustls-webpki,
  untrusted ; équivalente à MIT) ; exceptions pour `webpki-roots`
  (CDLA-Permissive-2.0, données) et les crates d'UniFFI (MPL-2.0, utilisées
  sans modification, copyleft limité à leurs fichiers).
- **QR inversé, contenu** : `oldtown://sponsor?o=` puis base64url sans
  remplissage de `version (1) ‖ ULID de l'identifiant (16) ‖ clé Ed25519 (32)
  ‖ clé X25519 (32) ‖ S (16) ‖ signature (64) ‖ nom (UTF-8)`. Le nouvel
  appareil **choisit son `device_id`** : il doit le connaître pour
  s'authentifier dès qu'il est enregistré. La signature Ed25519 porte sur
  `"ot-pair-reg-v1" ‖ device_id ‖ clé publique ‖ nom`
  (`ot_proto::pair_register_message`) ; le parrain la vérifie, puis le relais :
  il n'enregistre que des clés dont le possesseur a demandé l'enregistrement,
  sous ce nom et cet identifiant. `S` n'est pas signé : le canal optique
  l'authentifie. Figé par `crates/ot-crypto/tests/vectors/pairing_offer.json`.
- **Trames** (protocole toujours `v: 1`) : `pair_register {device_id, name,
  public_key, sig}`, parrain → relais, session authentifiée ;
  `pair_registered {device_id}`. Une erreur porte `ref` = l'identifiant
  demandé et **laisse la session ouverte** (contrairement à `pair_redeem`) :
  `pairing_invalid` (signature fausse, clé ou identifiant déjà pris),
  `name_taken`, `bad_frame` (nom invalide, ou entouré d'espaces : la
  signature porte sur le nom exact). Pas de migration : la table `devices`
  suffit, le parrain est seulement journalisé. Métrique
  `ot_pairings_total{result="registered"}`.
- **Envoi au nouvel appareil : schéma `0x04`**, le schéma `0x03` (note 0021)
  avec `S` ajouté au matériau d'entrée de HKDF :
  `HKDF-SHA256(DH(éph, X_B) ‖ S, sel vide, "ot-pair-qr-v1" ‖ éph ‖ X_B)`,
  à la manière du mode PSK de HPKE (RFC 9180). Il faut `S` (sinon le relais
  pourrait fabriquer un faux groupe) **et** la clé X25519 secrète de B (une
  photo du QR ne suffit pas à lire la clé de groupe). Le corps reste
  `pair_welcome`, signé par le parrain. Figé par
  `pairing_qr_welcome.json`, recoupé avec une implémentation indépendante
  (Python `cryptography`, HChaCha20 recodé).
- **Offre côté nouvel appareil** : `S` ne vit que dans la mémoire du client,
  comme le code de l'étape 2. En attendant d'être enregistré, l'appareil
  réessaie `auth` toutes les 1 à 3 s, 10 min au plus (CLI).
- **QR « rejoindre »** : `oldtown://pair?s=<URL du relais>&c=<code ot1-…>`,
  URL encodée en pourcentage hors caractères non réservés et `:` `/` ;
  paramètres inconnus ignorés (compatibilité).
- **CLI** : `ot pair new --qr`, `ot pair join --qr`, `ot pair sponsor
  '<texte du QR>'` (parrainer sans téléphone). QR dessiné en demi-blocs
  Unicode pour un terminal sombre (lu grâce à `tryInvert` de zxing-cpp).
- **`SecretStore` (T1)** : trait `get`/`put`/`delete` dans `ot-core` ;
  `StoreKeyring` garde exactement le format des fichiers de l'étape 2 (aucune
  migration). La CLI l'implémente sur ses fichiers (`FileStore`, noms
  validés).
- **`ot-ffi` (T2)** : UniFFI 0.32 en mode macros. Le code généré par les
  macros contient de l'`unsafe` : la crate n'hérite pas des lints du
  workspace (`unsafe_code = "forbid"`) ; le code écrit à la main n'en a pas.
  Runtime tokio interne de 2 threads, où les fonctions `async` exportées
  lancent leur travail. Erreurs : celles du plan plus `Invalid` (saisie
  invalide). Événements en plus : `JoinFailed`, `StorageError`. Le pont range
  aussi dans le `SecretStore` la configuration, l'identité, les messages vus,
  les 50 derniers éléments reçus et l'appairage par code en cours
  (`pending_join.json`, chiffré par le Keystore) : la demande au parrain
  repart si le service redémarre avant la réponse (mieux que la CLI).
- **Build Android** : profil cargo `android` (`lto`, `strip`) plutôt que de
  modifier `release` (relais et CLI inchangés). Les liaisons Kotlin sont
  générées depuis la bibliothèque de l'hôte : la bibliothèque Android
  dépouillée n'a plus les métadonnées UniFFI. `compileSdk` et `targetSdk` 37
  (dernière stable, exigée par Compose 2026.09), `minSdk` 33.
- **Correctif de l'étape 2** : le client ne comptait pas les pings du relais
  comme de l'activité ; une session calme tombait et se reconnectait toutes
  les 75 s (coûteux sur téléphone). Tout message reçu remet désormais le
  délai à zéro (`ClientConfig::idle_timeout`, test
  `quiet_session_stays_open_thanks_to_pings`).
- **`Client::reconnect_now`** : ferme la connexion en cours et se reconnecte
  aussitôt (changement de réseau Android).
- **Appli** : thème repris de T3 Code (palette, DM Sans en OFL), icônes au
  trait d'après Lucide (ISC) ; licences dans `apps/android/third_party/`.
  Nom de l'appareil fixé à l'appairage (le roster n'a pas de renommage).

**Conséquences.**

- Relais et clients de l'étape 3 vont ensemble pour le QR inversé (un relais
  plus ancien répond `unknown_type`) ; le reste du protocole est inchangé.
- En `wss`, l'URL doit porter le nom complet du certificat (`*.ts.net`), pas
  l'IP Tailscale.
- Un relais qui enregistre une autre clé sous l'identifiant du nouvel
  appareil ne gagne rien : l'imposteur ne lit pas la réponse et le roster du
  parrain porte la vraie clé ; le nouvel appareil, lui, n'arrive pas à
  s'authentifier. Nettoyage : `ot group rotate --exclude` et révocation.
- Le parrain ne sait pas si le nouvel appareil a bien reçu le groupe : on
  vérifie sur ce dernier (`ot group show`).
- Tout changement des formats `0x04` ou du QR inversé casse un vecteur.
