# Plan — Étape 5 : fichiers

L'étape 5 permet d'envoyer documents et photos, quelle que soit leur taille,
chiffrés de bout en bout, avec reprise après coupure. Le message chiffré
n'emporte que la référence du fichier et sa clé ; le relais stocke des
morceaux opaques pour une durée limitée.

**État (15 octobre 2026)** : **étape terminée.** Plan relu contre le code
avant T1 (note [0026](decisions/0026-relecture-etape-5.md)) ; D25 et D26
tranchées par la note
[0027](decisions/0027-image-par-domaine-transferts-android.md) ; écarts de
la réalisation dans la note [0028](decisions/0028-etape-5-realisation.md) ;
après le premier essai, réception dans l'extension refaite (T8, note
[0029](decisions/0029-extension-fichiers-ouverts-par-le-navigateur.md)) et
popup en conversation par canal (note
[0030](decisions/0030-extension-conversation-par-canal.md)). Validé en
local et sur un relais de dépannage (le Mac) : tests Rust, `npm run lint &&
npm test`, `./gradlew lint test assembleDebug`, et essai réel des envois du
téléphone vers l'extension. Non essayés explicitement : 500 Mo avec coupure
du Wi-Fi (couvert par les tests de bout en bout) et les tests instrumentés
Android avec l'APK corrigée (`docs/actions-manuelles.md`).

Références : jalon « 5. Fichiers » de la [conception](design.md), notes 0009
(un transfert n'ouvre pas de session), 0014 (RustCrypto), 0017 (extension
autonome), 0018 (téléchargement des fichiers reçus), 0023 (cœur en
WebAssembly, transport en TypeScript), 0024 (ouverture directe dans le
navigateur), 0026 (relecture) et 0027 (images domaine par domaine,
transferts Android).

**Niveau de détail** : plan de cadrage (note 0012), complété par la relecture
du 13 octobre 2026 ; la note 0026 fait foi sur les détails.

**Prérequis** : étapes 2 à 4 (chiffrement, appli Android, extension).

## Objectif et périmètre

**Résultat attendu** : partager une vidéo de 500 Mo depuis la galerie du
téléphone vers le Mac ; couper le Wi-Fi au milieu ; le transfert reprend où il
s'était arrêté et le fichier arrive intact dans le dossier Téléchargements.

**Inclus** : API HTTP de dépôt et de retrait par morceaux, chiffrement en flux,
reprise dans les deux sens, durée de vie et quotas sur le relais, envoi et
réception dans la CLI, Android (partage d'images et de fichiers) et
l'extension (choix d'un fichier, « envoyer l'image » au clic droit,
téléchargement des fichiers reçus, ouverture directe des images et PDF).

**Exclus** : synchronisation de dossiers (Syncthing le fait), aperçus côté
relais (il ne voit rien), miniatures dans les notifications, transfert direct
d'appareil à appareil en réseau local (piste pour plus tard), images collées
du presse-papiers (étape 6).

## Conception

**Chiffrement** (dans `ot-crypto`, crate `aead-stream`, note 0026) : une clé
aléatoire de 32 octets et un préfixe de nonce de 19 octets par fichier ;
morceaux de 1 Mio chiffrés par STREAM (`StreamBE32<XChaCha20Poly1305>`), en
accès direct (`position`, `dernier`) pour la reprise et le parallélisme. Un
fichier tronqué, réordonné ou altéré est rejeté. Le nom, le type MIME et le
SHA-256 du clair ne voyagent que dans le message chiffré.

**Message** : nouveau `kind: "file"` dans le corps chiffré : `blob_id`,
`name`, `mime`, `size`, `chunk_size`, `chunks`, `key`, `nonce`, `sha256`.
L'expéditeur dépose tout le fichier, puis envoie le message ; un destinataire
ne voit jamais de fichier incomplet. La clé reste dans le Rust : `ot-core` la
range avant d'acquitter et ne donne aux applis qu'une offre sans clé.

**API HTTP du relais** (même écoute que le WebSocket, `ws` ou `wss`) :

| Méthode et chemin | Qui | Rôle |
| --- | --- | --- |
| `POST /v1/blobs` | Tout appareil | Crée un blob : taille chiffrée, morceaux, destinataires ; renvoie `blob_id` |
| `PUT /v1/blobs/{id}/chunks/{n}` | Propriétaire | Dépose le morceau `n` (idempotent) |
| `GET /v1/blobs/{id}` | Propriétaire, destinataires | État : morceaux manquants, complet ou non |
| `GET /v1/blobs/{id}/chunks/{n}` | Destinataires | Retire le morceau `n` (blob complet seulement) |
| `POST /v1/blobs/{id}/done` | Destinataires | Tout reçu ; le relais supprime quand tous l'ont dit |
| `DELETE /v1/blobs/{id}` | Propriétaire | Annule l'envoi (ajout de la note 0026) |

- **Authentification sans session** (note 0009) : en-têtes `OT-Device`,
  `OT-Time`, `OT-Nonce`, `OT-Signature` ; signature Ed25519 de
  `"ot-http-v1" ‖ 0 ‖ méthode ‖ 0 ‖ chemin ‖ 0 ‖ heure ‖ nonce ‖ SHA-256 du
  corps`. Fenêtre de ±60 s, nonces mémorisés. Aucune entrée dans le registre
  des sessions.
- **Droits** : seul le créateur dépose et annule ; seuls les destinataires
  retirent (`to` vide = tous les autres appareils actifs à la création).
- **Stockage** : un fichier par morceau sous `--blob-dir` (par défaut
  `blobs/` à côté de la base), écrit puis renommé ; tables `blobs` et
  `blob_recipients` (migration `0003`).
- **Limites** (options) : taille maximale d'un fichier (2 Gio), quota total
  (20 Gio, réservé à la création), durée de vie (48 h). Dépassement : `413`
  ou `507` avec un code d'erreur.
- **Format** figé par des golden JSON (`ot_proto::http`), comme les trames.

**Clients** : état des transferts persisté (reprise après redémarrage) ;
téléchargement automatique sous 20 Mo, à la demande au-delà, et sur réseau
non facturé seulement pour le téléphone (note 0018) ; Android télécharge dans
`Download/OldTown` (MediaStore), l'extension dans Téléchargements, la CLI dans
`--download-dir`.

## Découpage en tâches

Chaque tâche finit par les validations de `CLAUDE.md`, plus
`./gradlew lint test assembleDebug` (T5) ou `npm run lint && npm test` (T6).

- [x] **T1. Chiffrement en flux dans `ot-crypto`.** *(`BlobId` ajouté à
  `ot-proto` dès T1 : le corps `file` le porte.)*
  Fichiers : `crates/ot-crypto/src/file.rs`, `body.rs`, `lib.rs`,
  `Cargo.toml` et `Cargo.toml` du workspace (`aead-stream`),
  `tests/vectors/file_chunks.json`, `tests/vectors.rs`, `CLAUDE.md`.
  Faire : `FileKey` (32 o + préfixe de 19 o, `ZeroizeOnDrop`, `Debug` muet),
  `seal_chunk(n, dernier, clair)` et `open_chunk(n, dernier, chiffré)` sur
  `StreamPrimitive` ; `Body::File` avec contrôle de cohérence (`chunks`,
  `chunk_size` ≤ 4 Mio) ; `sanitize_file_name`. Vérifier `cargo deny` (licence
  MIT OR Apache-2.0) et la compilation `wasm32-unknown-unknown`.
  Tests : aller-retour ; troncature (dernier morceau absent, faux dernier),
  réordonnancement, morceau altéré, mauvaise clé, fichier vide ; noms
  dangereux (`../x`, `CON`, contrôles, 300 octets) ; vecteur figé de 3
  morceaux (dernier court), recoupé avec une implémentation indépendante.
  Fini quand : vecteurs et tests passent en natif, la crate compile en
  WebAssembly.

- [x] **T2. API blobs du relais.** *(Purge des fichiers dans la boucle
  existante ; nettoyage des orphelins au démarrage seulement, note 0026.)*
  Fichiers : `crates/ot-proto/src/http.rs`, `lib.rs`
  (`http_request_message`), `tests/golden/http_*.json` ;
  `crates/ot-server/migrations/0003_blobs.sql`, `src/blobs.rs` (routes et
  stockage), `src/auth.rs`, `src/store.rs`, `src/lib.rs` (`Config`, purge,
  métriques), `src/main.rs` (options), `tests/blobs.rs` ; README section
  « Serveur ».
  Faire : routes ci-dessus avec authentification signée et cache de nonces
  borné ; contrôle des droits, des longueurs de morceau et du quota ;
  écriture `tmp` + renommage ; purge (expirés, tous `done`, annulés) dans la
  boucle existante ; nettoyage des orphelins au démarrage ; options
  `--blob-dir`, `--max-file-size`, `--blob-quota`, `--blob-ttl` ;
  métriques (octets stockés, blobs créés, refus par code). `ot-server` ne
  dépend toujours que de `ot-proto`.
  Tests : golden aller-retour ; signature fausse, heure hors fenêtre, nonce
  rejoué, appareil révoqué → 401 ; non-propriétaire qui dépose, non-
  destinataire qui retire → 403 ; retrait d'un blob incomplet → 409 ; fichier
  trop gros → 413, quota plein → 507 ; morceau redéposé (idempotent) ;
  suppression après tous les `done`, après 48 h, après `DELETE` ; un `.tmp`
  laissé par un arrêt brutal est nettoyé ; une session WebSocket ouverte du
  même appareil n'est pas coupée par ses requêtes HTTP.
  Fini quand : tests verts, logs sans en-tête de signature ni nom de fichier.

- [x] **T3. Transferts dans `ot-core`, `ot-ffi` et `ot-wasm`.** *(Retrait
  repris dans la même exécution seulement, note 0028.)*
  Fichiers : `crates/ot-core/src/transfer.rs` (fonction `net`),
  `src/http.rs` (client `hyper`), `src/secure.rs` (offres `file` rangées
  avant l'acquittement), `src/types.rs` (événements), `src/keyring.rs`
  (`files.json`) ; `crates/ot-ffi/src/` (traits étrangers `ChunkSource`,
  `ChunkSink`, `send_file`, `download`, `cancel`) ; `crates/ot-wasm/src/`
  (signature des requêtes, `sealChunk`, `openChunk`, offres) ;
  `crates/ot-server/tests/e2e_files.rs`.
  Faire : dépôt (création, morceaux manquants d'après `GET`, 4 en parallèle,
  nouvel essai avec backoff, message `file` à la fin) ; retrait (morceaux
  vérifiés, SHA-256, `done`, écriture finale seulement si tout concorde) ;
  état persisté par transfert ; événements de progression (4 par seconde au
  plus), de fin et d'échec (`expired` sur 404) ; annulation.
  Tests : contre un relais en processus : aller-retour de 5 Mio, coupure du
  proxy au milieu puis reprise, relais redémarré pendant le dépôt, blob
  expiré, mauvais SHA-256 → aucun fichier écrit ; la session WebSocket reste
  ouverte pendant un transfert.
  Fini quand : les tests passent et aucune clé de fichier n'apparaît dans
  l'API de `ot-ffi` ni de `ot-wasm`.

- [x] **T4. CLI.** *(`ot download <blob_id>` et `ot files`, note 0028.)*
  Fichiers : `crates/ot-cli/src/main.rs`, `files.rs`, README section « CLI ».
  Faire : `ot send --file <chemin>` (barre de progression, reprise si
  relancé), `ot listen --download-dir <dossier>` (tout télécharger ; sans
  l'option, afficher l'offre), `ot download <msg_id> [--to <dossier>]`. Nom
  déjà pris → suffixe ` (2)`.
  Tests : test d'intégration CLI ↔ relais en processus (fichier de 3 Mio,
  SHA-256 identique) ; collision de noms.
  Fini quand : un fichier passe d'un terminal à l'autre et le README
  documente les commandes.

- [x] **T5. Android.** *(Tests instrumentés écrits, pas encore lancés :
  pas d'émulateur ; voir T7.)*
  Fichiers : `apps/android/app/src/main/AndroidManifest.xml`,
  `share/ShareActivity`, `relay/RelayService`, `relay/RelayHub`, nouveau
  paquet `files/` (source, destination MediaStore, réglage du seuil),
  `notify/`, écrans de la liste ; `apps/android/CLAUDE.md`.
  Faire : `SEND` et `SEND_MULTIPLE` d'images, vidéos et fichiers (un message
  par fichier) ; transferts dans `RelayService` (note 0027) avec
  notification de progression (canal « Transferts ») ; téléchargement automatique sous 20 Mo
  sur réseau non facturé, bouton « Télécharger » sinon ; MediaStore
  `Download/OldTown` avec `IS_PENDING` ; « Ouvrir » par `ACTION_VIEW` ;
  fichiers dans la liste (nom, taille, état).
  Tests : unitaires JVM (règle de téléchargement automatique, choix des
  actions) ; instrumentés : partage d'une image par `SEND`, écriture
  MediaStore.
  Fini quand : `./gradlew lint test assembleDebug` passe et une photo
  partagée depuis la galerie arrive dans `ot listen --download-dir`.

- [x] **T6. Extension.** *(Choix du fichier dans `send.html` d'office ; pas
  encore essayé dans Brave.)*
  Fichiers : `apps/extension/manifest.json` (`downloads`,
  `optional_host_permissions`), `src/background/transfer.ts`, `engine.ts`,
  `main.ts`, `src/offscreen/` (raison `BLOBS`), `src/app/` (bouton
  « Envoyer un fichier », fenêtre `send.html`, page `viewer.html`, fichiers
  dans la liste, réglage de l'origine du relais), `test/transfer.test.ts`,
  `apps/extension/CLAUDE.md`, README.
  Faire : vérifier d'abord sur Brave Mac si la popup survit au sélecteur de
  fichiers (sinon fenêtre `send.html`, note 0026) ; permission de l'origine
  du relais ; copie dans l'OPFS puis dépôt par morceaux (chiffrement et
  signature dans le WebAssembly) ; « envoyer l'image » au clic droit
  (note 0027 : origine de l'image demandée au premier échec, requête sans
  cookies) ; retrait vers l'OPFS, puis Téléchargements par `chrome.downloads` via le document offscreen ;
  images et PDF ouverts dans `viewer.html` si le navigateur est au premier
  plan (note 0024), notification sinon ; progression dans la popup.
  Tests : sous Node contre un vrai `ot-server` (dépôt et retrait, reprise
  après coupure) ; trames et requêtes signées comparées aux golden ; vecteur
  `file_chunks.json` rejoué ; tests Angular de la liste.
  Fini quand : `npm run lint && npm test` passe et un PDF envoyé depuis la
  CLI s'ouvre dans Brave.

- [x] **T7. Tests de bout en bout et essai réel.** *(Tests de bout en bout,
  `SECURITY.md`, README et actions manuelles faits ; reste l'essai réel.)*
  Fichiers : `crates/ot-server/tests/e2e_files.rs` (compléments),
  `SECURITY.md`, `docs/actions-manuelles.md`, `README.md`, ce plan.
  Faire : vérifier que le disque du relais ne contient ni nom de fichier ni
  clair ; compléter `SECURITY.md` (authentification HTTP, blobs, noms
  nettoyés, permissions de l'extension) ; écrire les actions manuelles
  (déploiement du relais, rechargement de l'extension et permissions,
  nouvelle APK) ; essai réel selon les critères ci-dessous.
  Fini quand : tous les critères de fin sont vérifiés.

- [x] **T8. Extension : fichiers reçus ouverts par le navigateur** (demande
  du 14 octobre, après le premier essai ; note
  [0029](decisions/0029-extension-fichiers-ouverts-par-le-navigateur.md)).
  Plan ci-dessous ; voie A. *(Code de T8b à T8f livré : `npm run lint && npm
  test` (32 tests sous Node dont la réception servie par son adresse, 11 de
  la popup) et `npm run build` passent ; reste l'essai réel, lot 2 des
  actions manuelles.)*

### T8. Refonte de la réception de fichiers dans l'extension

**Ce qui gêne aujourd'hui.** Un fichier reçu est déchiffré dans l'OPFS puis
rangé par `chrome.downloads` ; Brave demande alors où l'enregistrer (son
réglage « Demander où enregistrer chaque fichier » l'emporte sur
`saveAs: false`, une extension ne peut pas le contourner). L'aperçu passe par
une page maison (`viewer.html`), et la popup montre « Télécharger » /
« Ouvrir » sous chaque fichier.

**Ce qui est voulu** (comme l'extension Pushbullet) : un petit fichier
(moins de 20 Mo) s'ouvre dans le lecteur du navigateur (image, PDF, vidéo,
texte), sans rien enregistrer ; on l'enregistre si on veut, par le clic droit
du navigateur ou depuis la popup. Un gros fichier ouvre la popup, d'où on le
télécharge. Dans la popup : un clic sur un fichier l'ouvre, une vraie icône
de téléchargement à droite de l'élément le télécharge.

**Faisabilité.** Le relais ne sert que du chiffré, avec des requêtes signées :
le navigateur ne peut pas l'ouvrir lui-même. Mais le service worker de
l'extension peut répondre aux requêtes vers ses propres adresses (événement
`fetch`). Une adresse comme `chrome-extension://<id>/f/<blob_id>/<nom>`,
ouverte dans un onglet, serait servie par le service worker : il retire les
morceaux, les déchiffre dans le WebAssembly et les renvoie au fil de l'eau
avec le bon type (`Content-Type`, `Content-Disposition: inline;
filename=<nom>`). Le navigateur l'affiche avec son propre lecteur, et son clic
droit *Enregistrer sous* propose le bon nom. Avec `attachment` au lieu
d'`inline`, la même adresse devient un téléchargement natif de Brave (barre
de progression du navigateur, sans passer par l'OPFS).

- **Voie A, recommandée** : le service worker sert les fichiers (ci-dessus),
  en lisant le relais à la demande et en gardant une copie **chiffrée** des
  morceaux dans l'OPFS. Rien de déchiffré au repos dans le profil, l'adresse
  reste valable après le retrait du relais, et le relais est prévenu (`done`)
  une fois tout retiré.
- **Voie B, repli** si Brave ne transmet pas ces requêtes au service worker
  (à vérifier, T8a) : la demande de secours que tu as décrite. Un petit
  fichier est téléchargé d'office dans `Téléchargements/OldTown/` (chemin
  relatif accepté par `chrome.downloads`), puis ouvert par le lecteur de
  Brave (`chrome.downloads.open`, qui demande la permission
  `downloads.open` et un geste de l'utilisateur, donc seulement au clic) ; un
  gros fichier ouvre la popup. La question de l'emplacement de Brave reste
  posée tant que son réglage est actif : il faudra le décocher (action
  manuelle).

Sous-tâches :

- [x] **T8a. Essai de faisabilité dans Brave** (une demi-journée au plus).
  *(Fait le 14 octobre : points 0 à 6 concluants, voie A ; page d'essai
  retirée.)* Une page de test dans
  l'extension, sans toucher au reste : (1) un onglet
  ouvert sur `chrome-extension://<id>/f/test.jpg` déclenche-t-il l'événement
  `fetch` du service worker ? (2) une réponse en flux (`ReadableStream`)
  s'affiche-t-elle dans le lecteur d'images, de PDF et de vidéo de Brave ?
  (3) *Enregistrer sous* propose-t-il le nom de `Content-Disposition` ?
  (4) `attachment` donne-t-il un téléchargement natif, sans demande
  d'emplacement quand le réglage est décoché ? (5) le service worker
  reste-t-il en vie pendant une réponse de 20 Mo, et de 500 Mo ? (6) la
  popup survit-elle à l'ouverture du sélecteur de fichiers de macOS, et à
  la copie dans l'OPFS du fichier choisi (pour T8e) ? Résultat : complément
  de la note 0029 (voie A ou B).
- [x] **T8b. Service worker : adresses de fichiers** (voie A). Route
  `/f/<blob_id>/<nom>` dans l'événement `fetch` ; réponse en flux depuis la
  copie chiffrée de l'OPFS, ou depuis le relais en la complétant ;
  `inline` par défaut, `attachment` avec `?dl=1` ; requêtes `Range` pour les
  vidéos (lecture et avance rapide) ; refus de toute requête qui ne vient pas
  d'une page de l'extension ou d'une navigation de premier niveau.
- [x] **T8c. Clé gardée tant que la copie existe.** Aujourd'hui l'offre (et sa
  clé) est oubliée après le téléchargement ; elle reste désormais dans le
  coffre tant que la copie chiffrée est dans l'OPFS, et les deux partent
  ensemble (élément sorti de l'historique, ou 7 jours). `ot-wasm` : déchiffrer
  un morceau d'un fichier dont l'offre est gardée (déjà le cas avec
  `openChunk`) ; `forgetFile` appelé au nettoyage, plus au téléchargement.
- [x] **T8d. Réception.** Petit fichier, navigateur au premier plan, élément
  récent (règle de la note 0024) : onglet ouvert sur son adresse, si le
  navigateur sait l'afficher (image, PDF, texte, audio et vidéo courants) ;
  autre type, quelle que soit sa taille : notification seulement (D27, note
  0029), rien sur le disque sans geste. Navigateur en arrière-plan : notification, un clic
  ouvre l'adresse. Gros fichier : la popup s'ouvre (`action.openPopup`), à
  défaut notification. Plus de `chrome.downloads` d'office, plus de
  `viewer.html` ni de copie déchiffrée dans l'OPFS.
- [x] **T8e. Popup.** Un clic sur un élément de fichier l'ouvre (son adresse
  dans un onglet) ; un type que le navigateur ne sait pas afficher se
  télécharge au clic. Icône de téléchargement à droite de l'élément
  (`?dl=1`), **toujours présente** : un téléchargement annulé (fenêtre
  d'enregistrement de Brave fermée) ne change rien à l'élément (corrige le
  bug de l'essai : bouton disparu, fichier absent). Barre de progression
  fine dans l'élément pendant un retrait ; plus de boutons texte sous les
  fichiers. Pour un texte ou un lien, même logique : clic pour ouvrir
  (lien) ou déplier (texte), icône « copier » à droite. **Trombone** : ouvre
  directement le sélecteur de fichiers de l'OS depuis la popup, destinataire
  = celui de la popup ; la fenêtre `send.html` ne garde que les demandes de
  permission (et sert de repli si T8a montre que la popup se ferme).
- [x] **T8f. Nettoyage et documentation.** Retirer `viewer.html`, la raison
  `BLOBS` du document offscreen et la permission `downloads` si la voie A ne
  s'en sert plus ; mettre à jour `apps/extension/CLAUDE.md`, le README,
  `SECURITY.md` (plus de fichier reçu en clair dans le profil) et les notes.

Tests : sous Node, la réponse en flux d'une adresse de fichier (corps égal
au clair, en-têtes, `Range`, refus d'une origine étrangère) contre un vrai
relais ; copie chiffrée dans l'OPFS (aucun octet du clair) ; clé oubliée
avec la copie. Angular : icône de téléchargement présente quel que soit
l'état de l'élément, trombone sans fenêtre. Essai réel : photo et PDF
ouverts par Brave, vidéo lue et avancée, *Enregistrer sous*, icône de
téléchargement (puis annulation : l'icône reste), `.zip` reçu (notification
seulement), envoi par le trombone, gros fichier.

Fini quand : un PDF reçu s'ouvre dans le lecteur de Brave sans demande
d'emplacement ni fichier enregistré, et l'icône de la popup le télécharge
avec la barre de téléchargement du navigateur.

## Critères de fin

| Scénario | Attendu | Vérifié par | Résultat (15 oct. 2026) |
| --- | --- | --- | --- |
| 500 Mo téléphone → Mac avec coupure | Reprise, fichier identique (SHA-256) | Essai réel | Non essayé ; coupure simulée OK (`transfers_resume_after_network_cuts`) |
| Photo Mac → téléphone | Dans `Download/OldTown`, « Ouvrir » l'affiche | Essai réel | Non confirmé |
| Image ou PDF reçu, navigateur au premier plan | S'ouvre dans le lecteur du navigateur (note 0029) | Essai réel | OK |
| Photos téléphone → extension | Déposées, annoncées, ouvertes | Essai réel | OK (après l'autorisation d'accès au relais) |
| Téléchargement depuis l'extension | Téléchargement natif du navigateur | Essai réel | Corrigé après l'essai (navigation, note 0029) |
| Morceau altéré ou fichier tronqué | Rejeté à la réception | Test `ot-crypto` | OK |
| Transfert pendant que l'appli reçoit | La session WebSocket n'est pas coupée | Test E2E | OK |
| Relais redémarré pendant un dépôt | Le dépôt reprend | Test E2E | OK |
| Quota plein, fichier trop gros | Erreur claire côté expéditeur | Test relais | OK |
| Tous les destinataires ont fini, ou 48 h passées | Morceaux supprimés du disque | Test relais | OK |
| Disque du relais | Aucun nom de fichier ni contenu en clair | Test E2E | OK |
| Clés de fichier | Jamais dans Kotlin, le JavaScript ou les logs | Revue + test | OK (tests `ot-ffi`, `ot-wasm`, extension, logs du relais) |

## Risques

- **Mémoire de l'extension** : un fichier de 2 Gio ne doit jamais tenir en
  mémoire ; tout passe par l'OPFS morceau par morceau. À mesurer à T6.
- **Comportement du sélecteur de fichiers dans la popup** selon l'OS : à
  vérifier avant de coder l'interface (T6).
- **Disque du relais** : le quota de 20 Gio doit tenir sur le laptop ;
  vérifier l'espace libre avant le déploiement (action manuelle).
- **Deux implémentations du transfert** (Rust et TypeScript) : mêmes golden,
  même vecteur, test contre le vrai relais en CI.
