# 0028 — Étape 5 : écarts et précisions de la réalisation

**Statut : acceptée** (13 octobre 2026). Tâches T1 à T6 de l'étape 5 ;
complète les notes [0026](0026-relecture-etape-5.md) et
[0027](0027-image-par-domaine-transferts-android.md).

**Contexte.** La relecture (0026) a fixé la conception. En codant, quelques
points ont été précisés, et deux s'écartent du plan.

**Décision.**

- **Reprise du retrait** (écart au plan, « état persisté » dans les deux
  sens) : seul le **dépôt** reprend après un redémarrage (état
  `uploads.json`, avec la clé, dans le `Keyring`). Un retrait interrompu
  par une coupure reprend tout seul tant que l'appli tourne (nouvel essai de
  chaque requête tant que le transfert a progressé depuis moins de
  10 minutes) ; après un redémarrage, il repart du premier morceau. Raison :
  le SHA-256 est calculé dans l'ordre d'écriture ; reprendre demanderait de
  relire le fichier partiel, que MediaStore et l'OPFS ne garantissent pas.
  Le fichier reste 48 h sur le relais : rien n'est perdu, seulement du
  temps.
- **`BlobId` dans `ot-proto` dès T1** (précision) : le corps `file` le porte.
- **Géométrie du chiffré** (précision) : l'API ne parle que de tailles
  chiffrées (taille totale, morceau chiffré = clair + 16 octets) ; le
  relais en déduit le nombre de morceaux et la longueur de chacun, sans
  rien savoir du format. La limite de taille porte sur le clair estimé.
- **Sérialisation de l'état des fichiers** (précision) : offres reçues et
  dépôts en cours s'écrivent dans le `Keyring` uniquement depuis la tâche du
  client (`Command::WithSecure`) ; les transferts eux-mêmes tournent à
  côté, en parallèle, sans toucher à la session WebSocket.
- **CLI** : `ot download <blob_id>` (et non `<msg_id>`), et `ot files` pour
  lister les fichiers annoncés ; ils ne listent que ce qu'`ot listen` a déjà
  reçu, comme pour les messages.
- **Extension** :
  - `outgoing.json` (envois en cours, avec les noms de fichiers) est rangé
    chiffré, comme l'historique : rien de plus en clair dans le profil.
  - Un fichier reçu est toujours rangé dans Téléchargements ; image ou PDF
    récent, navigateur au premier plan, il s'ouvre aussi dans l'aperçu
    (`viewer.html`). Images et PDF restent dans l'OPFS pour « Ouvrir » ;
    les autres en sont effacés une fois rangés, et l'OPFS est nettoyé au
    démarrage de ce qui n'est plus dans l'historique.
  - Un fichier de plus de 20 Mo donne une notification à l'annonce ; un
    petit n'en donne qu'une, une fois enregistré.
  - Les demandes de permission (origine du relais, domaine d'une image) se
    font dans la petite fenêtre `send.html`, sur un clic : une demande depuis
    la popup peut la fermer et perdre la réponse.
  - **Correctif après le premier essai** (14 octobre) : un fichier reçu
    déclenchait un téléchargement d'office alors que l'accès à l'origine du
    relais n'avait jamais été demandé (il ne l'était qu'au premier envoi) ;
    le navigateur bloquait les requêtes (CORS, le relais répond `405` à la
    requête préalable) et le transfert restait à 0 % en réessayant. Le
    moteur vérifie désormais la permission avant tout transfert : sans
    elle, le fichier est noté « accès au relais pas encore autorisé » ; la
    popup affiche un bandeau *Autoriser*, et l'accord relance seul ce qui
    attendait.
- **Android** : en plus du menu *Partager*, un bouton trombone dans la zone
  d'envoi ouvre le sélecteur de fichiers d'Android (demande du 14 octobre) ;
  l'accès aux fichiers choisis est gardé (`takePersistableUriPermission`)
  jusqu'à la fin de l'envoi, si bien que les rechoisir après un redémarrage
  reprend l'envoi. Le partage d'un fichier ferme la feuille aussitôt confié
  au service.
- **Tests** : les tests de bout en bout des fichiers utilisent une base
  SQLite sur disque. Une requête annulée par une coupure peut faire jeter sa
  connexion par sqlx ; une base en mémoire repartirait alors vide (« no such
  table »), ce qui n'arrive pas en production.

**Conséquences.**

- Un gros fichier reçu sur le téléphone, interrompu par un redémarrage, se
  retélécharge en entier.
- `ot files` et `ot download` complètent `ot listen` ; le README les
  documente.
- Les tests instrumentés Android (partage d'une image, écriture MediaStore)
  sont écrits mais n'ont pas tourné faute d'émulateur dans l'environnement de
  développement : à lancer avec le téléphone (`connectedDebugAndroidTest`).
