# Plan — Étape 5 : fichiers

L'étape 5 permet d'envoyer documents et photos, quelle que soit leur taille,
chiffrés de bout en bout, avec reprise après coupure. Le message chiffré
n'emporte que la référence du fichier et sa clé ; le relais stocke des
morceaux opaques pour une durée limitée.

Références : jalon « 5. Fichiers » de la [conception](design.md), note 0009
(un transfert n'ouvre pas de session), décisions D6 et D20 de
[`decisions-a-prendre.md`](decisions-a-prendre.md).

**Niveau de détail** : plan de cadrage (D2). Au démarrage, relire contre le
code, compléter les tâches et noter les écarts dans `decisions/`.

**Prérequis** : étapes 2 à 4 (chiffrement, applis Android et desktop).

## Objectif et périmètre

**Résultat attendu** : partager une vidéo de 500 Mo depuis la galerie du
téléphone vers le Mac ; couper le Wi-Fi au milieu ; le transfert reprend où il
s'était arrêté et le fichier arrive intact dans le dossier Téléchargements.

**Inclus** : API HTTP de dépôt et de retrait par morceaux, chiffrement en flux,
reprise dans les deux sens, durée de vie et quotas sur le relais, envoi et
réception dans la CLI, Android (partage d'images et de fichiers) et desktop
(glisser-déposer de fichiers).

**Exclus** : synchronisation de dossiers (Syncthing le fait), aperçus côté
relais (il ne voit rien), transfert direct d'appareil à appareil en réseau
local (piste pour plus tard).

## Conception

**Chiffrement** (dans `ot-crypto`) : une clé aléatoire de 32 octets par
fichier ; découpage en morceaux de 1 Mio chiffrés avec la construction STREAM
de RustCrypto (`aead::stream`, XChaCha20-Poly1305), qui authentifie l'ordre des
morceaux et marque le dernier : un fichier tronqué ou réordonné est rejeté.
Le nom, le type MIME et l'empreinte SHA-256 du clair ne voyagent que dans le
message chiffré.

**Message** : nouveau `kind: "file"` dans le corps chiffré :
`blob_id`, `name`, `mime`, `size`, `chunk_size`, `chunks`, `key`,
`sha256`. L'expéditeur dépose tout le fichier, puis envoie le message ; un
destinataire ne voit jamais de fichier incomplet.

**API HTTP du relais** (même écoute que le WebSocket) :

| Méthode et chemin | Rôle |
| --- | --- |
| `POST /v1/blobs` | Crée un blob : taille, nombre de morceaux, destinataires ; renvoie `blob_id` |
| `PUT /v1/blobs/{id}/chunks/{n}` | Dépose le morceau `n` (idempotent) |
| `GET /v1/blobs/{id}` | État : morceaux reçus, complet ou non (sert à la reprise) |
| `GET /v1/blobs/{id}/chunks/{n}` | Retire le morceau `n` |
| `POST /v1/blobs/{id}/done` | Le destinataire a tout reçu ; le relais supprime quand tous l'ont dit |

- **Authentification sans session** (règle de 0009) : chaque requête porte
  `device_id`, un horodatage, un nonce et une signature Ed25519 sur
  `"ot-http-v1" ‖ méthode ‖ chemin ‖ horodatage ‖ nonce ‖ SHA-256 du corps`.
  Fenêtre de 60 s, nonces mémorisés pendant la fenêtre. Aucune entrée dans le
  registre des sessions.
- **Droits** : seul le créateur dépose ; seuls les destinataires retirent
  (`to` vide = tous les autres appareils au moment de la création).
- **Stockage** : morceaux sur disque sous `/var/lib/ot-relay/blobs/`, table
  `blobs` (id, propriétaire, taille, morceaux, création, expiration, fin) et
  `blob_recipients` (blob, appareil, `done_at`). Écriture dans un fichier
  temporaire puis renommage, pour qu'un morceau soit entier ou absent.
- **Limites** (configurables) : taille maximale d'un fichier (2 Gio par
  défaut), quota disque total (20 Gio), durée de vie d'un blob (48 h, plus
  courte que les messages). Dépassement : `413` ou `507` avec un code d'erreur.
- **Le format de l'API** est figé par des golden JSON, comme les trames.

**Clients** : état des transferts persisté (reprise après redémarrage de
l'appli) ; téléchargement automatique sous un seuil, à la demande au-delà
(D20) ; sur Android, téléchargement dans `Download/OldTown` via MediaStore,
avec WorkManager pour les longs transferts.

## Découpage en tâches

- [ ] **T1. Chiffrement en flux** dans `ot-crypto` ; vecteurs figés ; tests de
  troncature, réordonnancement, morceau altéré.
- [ ] **T2. API blobs du relais** : migration, stockage disque, routes,
  authentification signée, quotas, purge ; golden JSON ; note de décision.
- [ ] **T3. Transferts dans `ot-core`** : dépôt et retrait avec reprise,
  parallélisme borné (4 morceaux), progression en événements.
- [ ] **T4. CLI** : `ot send --file <chemin>`, `ot listen --download-dir`.
- [ ] **T5. Android** : partage d'images et de fichiers (`SEND`,
  `SEND_MULTIPLE`), notification de progression, téléchargement.
- [ ] **T6. Desktop** : glisser-déposer de fichiers, progression, ouverture du
  dossier.
- [ ] **T7. Tests de bout en bout et essai réel** : gros fichier, coupure,
  redémarrage du relais pendant le dépôt, quota plein, expiration.

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| 500 Mo téléphone → Mac avec coupure | Reprise, fichier identique (SHA-256) | Essai réel |
| Morceau altéré ou fichier tronqué | Rejeté à la réception | Test `ot-crypto` |
| Transfert pendant que l'appli reçoit | La session WebSocket n'est pas coupée | Test E2E |
| Quota plein, fichier trop gros | Erreur claire côté expéditeur | Test relais |
| Tous les destinataires ont fini, ou 48 h passées | Morceaux supprimés du disque | Test relais |
| Disque du relais | Aucun nom de fichier ni contenu en clair | Test E2E |
