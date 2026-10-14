# 0026 — Relecture du plan de l'étape 5 contre le code

**Statut : acceptée** (13 octobre 2026). Relecture du plan de cadrage de
l'étape 5 (note 0012) avant T1 ; écarts et précisions. Les choix laissés à
trancher, D25 et D26, l'ont été par la note
[0027](0027-image-par-domaine-transferts-android.md).

**Contexte.** Le plan date d'avant les étapes 2 à 4. Depuis : RustCrypto est
passé à `aead` 0.6 (`chacha20poly1305` 0.11), le cœur client est partagé
entre Rust natif, UniFFI et WebAssembly (note 0023), les clés ne doivent
jamais sortir du Rust (ni vers Kotlin, ni vers le JavaScript), et
l'extension n'a aucune permission d'hôte. Plusieurs points du plan ne
tiennent plus tels quels.

**Décision.**

- **STREAM vient de la crate `aead-stream`** (écart) : le module
  `aead::stream` n'existe plus dans `aead` 0.6. `aead-stream` 0.6.0
  (RustCrypto, dépôt `AEADs`, MIT OR Apache-2.0, sans dépendance hors `aead`)
  en est la suite. Nouvelle dépendance de `ot-crypto`, justifiée ici.
- **Variante `StreamBE32<XChaCha20Poly1305>`** : nonce de 24 octets =
  préfixe aléatoire de 19 octets ‖ compteur 32 bits big-endian ‖ drapeau
  « dernier » (1 octet). Le préfixe est tiré par fichier et voyage dans le
  message (champ `nonce`, absent du plan). Données associées constantes
  `"ot-file-v1"` (séparation de domaine ; la clé est déjà propre au fichier).
- **Accès direct aux morceaux** (précision) : chiffrement et déchiffrement par
  `StreamPrimitive::{encrypt,decrypt}_in_place(position, dernier)`, que
  RustCrypto déclare sans état pour permettre le parallélisme, et non par
  l'`Encryptor` séquentiel. C'est ce qui rend possibles la reprise au morceau
  près et le parallélisme de 4. Le destinataire déchiffre le morceau `n` avec
  `dernier = (n == chunks - 1)` : un morceau manquant, déplacé, altéré ou un
  faux dernier morceau échouent. Fichier vide : un seul morceau, vide, marqué
  dernier.
- **Corps `file`** (schéma `0x01`, signé comme les autres) : `blob_id`,
  `name`, `mime`, `size` (clair), `chunk_size`, `chunks`, `key` (32 o),
  `nonce` (19 o), `sha256` (du clair). Les clients produisent des morceaux de
  1 Mio ; à la réception, `chunk_size` est accepté jusqu'à 4 Mio et `chunks`
  doit valoir `max(1, ⌈size / chunk_size⌉)`. Le nom est nettoyé à la
  réception (séparateurs de chemin, caractères de contrôle, point initial,
  noms réservés de Windows, 255 octets au plus) : il vient d'un membre mais
  finit dans un système de fichiers.
- **Les clés de fichier restent dans le Rust** (précision, comme les clés de
  groupe) : à la réception d'un `file`, `ot-core` range l'offre (clé
  comprise) dans le `Keyring` **avant** l'acquittement (`files.json`, effacé
  après le téléchargement ou 7 jours), puis émet un événement sans clé
  (`blob_id`, nom, type, taille, expéditeur). L'appli demande ensuite le
  téléchargement par `blob_id`. Dans l'extension, le `Core` WebAssembly garde
  la clé et expose « ouvrir le morceau `n` du blob `b` ». `Content` de
  `ot-proto` reste texte ou lien.
- **API HTTP** (précisions) :
  - En-têtes `OT-Device`, `OT-Time` (millisecondes), `OT-Nonce` (16 octets)
    et `OT-Signature`, en base64 standard comme le reste du protocole
    (précisé à T2 : ses caractères sont permis dans un en-tête HTTP). Message signé :
    `"ot-http-v1" ‖ 0x00 ‖ méthode ‖ 0x00 ‖ chemin ‖ 0x00 ‖ heure (u64 BE) ‖
    nonce ‖ SHA-256 du corps` : méthode et chemin ont une longueur variable,
    l'octet nul les sépare sans ambiguïté. Fonction
    `ot_proto::http_request_message`, à côté de `auth_message`.
  - Nonces mémorisés 2 minutes (fenêtre de ±60 s), en mémoire et bornés. Un
    redémarrage du relais les oublie : un rejeu dans la minute qui suit ne
    peut que recréer un blob vide (compté dans le quota, expiré en 48 h) ou
    refaire une opération idempotente. Accepté (Tailscale et `wss`).
  - `blob_id` = `blob_` + ULID, tiré par le relais. `POST /v1/blobs` reçoit
    `{size, chunk_size, chunks, to}` où `size` est la taille **chiffrée**
    totale ; le relais vérifie la longueur de chaque morceau (`chunk_size +
    16`, sauf le dernier) et réserve la taille dans le quota dès la création.
  - Un destinataire ne retire un morceau que d'un blob complet (`409
    incomplete` sinon). `GET /v1/blobs/{id}` renvoie `missing` (indices des
    morceaux absents) et `complete`.
  - **En plus du plan** : `DELETE /v1/blobs/{id}` par le propriétaire
    (annulation d'un envoi, libère le quota).
  - Erreurs en JSON `{"error": code, "message": …}` ; codes dans `ot-proto`
    (`unauthorized` 401, `forbidden` 403, `not_found` 404, `incomplete` 409,
    `too_large` 413, `quota_exceeded` 507, `bad_request` 400 ; à T2,
    `busy` 503 quand la mémoire des nonces est pleine, 100 000 au plus).
    Tailles de l'API toutes en **chiffré** (taille totale, morceau chiffré) :
    le relais vérifie la géométrie sans rien savoir du format ; la limite de
    taille porte sur le clair estimé (taille moins 16 octets par morceau).
    Un blob sans destinataire actif est refusé (400). Types dans un
    module `ot_proto::http`, golden `tests/golden/http_*.json`.
  - Un appareil révoqué est refusé (401) ; ses blobs expirent normalement.
- **Stockage du relais** : dossier `--blob-dir` (`OT_BLOB_DIR`), par défaut
  `blobs/` à côté de la base (donc `/var/lib/ot-relay/blobs`, couvert par le
  `StateDirectory` de l'unité, sans changement de `deploy/`). Un morceau =
  un fichier `<blob_id>/<n>`, écrit en `<n>.tmp` puis renommé. Au démarrage,
  les `.tmp` et les dossiers sans ligne en base sont supprimés. Limites
  `--max-file-size` (2 Gio), `--blob-quota` (20 Gio), `--blob-ttl` (48 h).
  Migration `0003_blobs.sql`. Corps HTTP limité à 4 Mio + 16 octets.
- **Client HTTP de `ot-core`** : `hyper` 1 (déjà dans l'arbre par axum) et
  `hyper-util`, avec la configuration rustls (`ring`, autorités de Mozilla) de
  `tls.rs`, sous la fonction `net`. Écarté : `reqwest`, qui tire `aws-lc-rs`
  par défaut (note 0022). À confirmer au début de T3.
- **Sources et destinations côté appli** : `ot-core` lit et écrit par deux
  traits, `ChunkSource` (taille, lecture à un décalage) et `ChunkSink`
  (écriture à un décalage, fin, abandon). La CLI les implémente sur des
  fichiers ; Android en Kotlin (trait étranger UniFFI) sur un
  `ContentResolver`, parce que le Rust ne sait pas ouvrir une URI
  `content://` ; l'extension refait la boucle en TypeScript (comme le
  transport, note 0023), le chiffrement restant dans le WebAssembly.
- **Extension : permissions** (écart à la note 0023, « pas de
  `host_permissions` ») :
  - `fetch` vers le relais est soumis à CORS sans permission d'hôte :
    l'origine du relais devient une permission optionnelle
    (`optional_host_permissions`), demandée par la page de réglages quand
    l'URL du relais est enregistrée, et au premier envoi de fichier pour une
    installation existante. Écarté : des en-têtes CORS dans le relais (il
    faudrait lui donner l'identifiant de l'extension).
  - `downloads`, pour ranger les fichiers reçus.
  - L'image d'une page (« envoyer l'image ») : origine par origine, note 0027.
- **Extension : pas d'URL de blob dans le service worker**
  (`URL.createObjectURL` n'y existe pas) : les morceaux reçus sont écrits dans
  l'OPFS de l'extension ; le document offscreen (raison `BLOBS`) en fait une
  URL pour `chrome.downloads.download`. Le fichier à envoyer est copié dans
  l'OPFS avant le dépôt : le service worker peut être arrêté, la popup se
  ferme, et un `File` ne passe pas par `runtime.sendMessage`.
- **Extension : choix du fichier dans une petite fenêtre** (`send.html`,
  ouverte par un bouton de la popup) plutôt que dans la popup elle-même : la
  popup peut se fermer quand le sélecteur de fichiers de l'OS s'ouvre, et la
  copie dans l'OPFS doit aller à son terme. À vérifier sur Brave Mac au
  début de T6 ; si la popup tient, la fenêtre devient inutile.
- **Extension : ouverture directe** (note 0024) : navigateur au premier plan,
  une image ou un PDF reçu s'ouvre dans une page de l'extension
  (`viewer.html`, lue depuis l'OPFS, bouton « Enregistrer ») ; les autres
  types vont dans Téléchargements, avec une notification.
- **Extension : maintien en vie** : rien de nouveau. Le `ping` toutes les
  20 s (note 0023) garde le service worker actif pendant un transfert, et
  chaque requête porte au plus un morceau (bien sous les 30 s de Chrome).
- **Android** : le service actuel (`RelayService`, `specialUse`) fait les
  transferts plutôt que WorkManager (note 0027). `ShareActivity` lui passe
  les URI avec `FLAG_GRANT_READ_URI_PERMISSION`. Une URI partagée n'est pas
  persistable : si le processus meurt pendant un dépôt, la reprise se fait
  tant que l'URI reste lisible, sinon le transfert échoue avec un message
  « partager de nouveau ». Pas de copie préalable du fichier (500 Mo de
  plus, et du temps). Téléchargement dans `Download/OldTown` par MediaStore
  (`IS_PENDING` jusqu'à la vérification du SHA-256), aucune permission de
  stockage. Wi-Fi = réseau non facturé à l'usage
  (`NET_CAPABILITY_NOT_METERED`).
- **CLI** : `ot listen --download-dir` télécharge tout, sans seuil (le
  dossier est donné exprès) ; sans l'option, il affiche le fichier reçu et sa
  commande de téléchargement (`ot download <msg_id>`). `ot send --file`
  relancé sur le même fichier (chemin, taille, date de modification) reprend
  le dépôt en cours.

**Conséquences.**

- Le format chiffré des fichiers est figé par un nouveau vecteur
  (`file_chunks.json`), recoupé avec une implémentation indépendante comme
  les autres ; le golden de `ot-proto` couvre l'API HTTP.
- Le relais voit la taille des fichiers (arrondie au morceau près), le nombre
  de destinataires et les dates, comme pour les messages ; ni le nom ni le
  type.
- Un message `file` peut survivre à son blob (7 jours contre 48 h) : le
  client affiche « fichier expiré » sur un `404`, il ne réessaie pas.
- L'extension demande trois permissions de plus (origine du relais,
  `downloads`, et les domaines d'images validés un par un) : à recharger et accepter à
  la main (`docs/actions-manuelles.md` à la livraison).
- `SECURITY.md` est à compléter à la livraison (authentification HTTP,
  stockage des blobs, nettoyage des noms, nouvelles permissions).
