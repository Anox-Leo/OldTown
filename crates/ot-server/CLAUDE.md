# ot-server

Relais : authentifie les appareils par défi Ed25519, stocke les enveloppes
dans SQLite, les livre et purge ce qui est acquitté ou expiré ; enregistre un
nouvel appareil sur jeton d'appairage (note 0020) ou à la demande d'un
parrain authentifié (`pair_register`, QR inversé, note 0022) ; stocke les
fichiers chiffrés déposés par morceaux (API HTTP `/v1/blobs`, étape 5, note
0026). Binaire `ot-server` (`serve [--admin-listen --trusted-proxy --tls-cert
--tls-key --blob-dir --max-file-size --blob-quota --blob-ttl]`, `device add|list|revoke`,
`messages purge [--all]`).

- Ne dépend que de `ot-proto`. **Ne lit jamais `payload`**, pas même
  `payload.enc`, et ne dépendra jamais de `ot-crypto`.
- Une session par appareil (registre `registry.rs`, note 0009). `ping`
  (session authentifiée) reçoit `pong` : maintien de l'extension Chrome,
  dont le service worker s'arrête après 30 s sans message (note 0023). Les
  transferts de fichiers HTTP (étape 5) s'authentifient sans y entrer.
- Fichiers (`blobs.rs`) : chaque requête signée (`auth::verify_http`, nonces
  en mémoire dans `NonceCache`) ; le relais ne connaît que la géométrie du
  chiffré, jamais nom ni type. Un morceau = un fichier `0600` sous
  `<blob_dir>/<blob_id>/`, écrit sous un nom temporaire, synchronisé puis
  renommé, et noté en base seulement après. Au démarrage, les `.tmp` et les
  dossiers inconnus de la base sont effacés (seulement les noms
  `blob_<ULID>`). Suppression quand tous les destinataires actifs ont dit
  `done`, à l'annulation, ou à l'expiration (purge).
- SQLite : migrations versionnées dans `migrations/`, jamais modifiées une fois
  livrées (on en ajoute une) ; écritures en `BEGIN IMMEDIATE` (note 0007) ;
  requêtes `sqlx::query` non vérifiées à la compilation (note 0003).
- Compteurs Prometheus écrits à la main dans `metrics.rs` (note 0004),
  servis par `admin_router` sur une adresse à part (`OT_ADMIN_LISTEN`),
  jamais sur le port public (note 0036).
- Limites (`limits.rs`, note 0036) : connexions non authentifiées bornées
  par IP et au total (place rendue à la fin de l'auth), échecs freinés par
  IP (seau à jetons : auth refusée, jeton d'appairage invalide, requête
  HTTP en `401`), refus en `503` `busy` avant l'upgrade. IPv6 comptée par
  /64. `X-Forwarded-For` lu seulement d'un frontal de `OT_TRUSTED_PROXY`.
  Tout en mémoire, aucune IP sur le disque.
- Logs structurés : identifiants d'appareil, `seq`, tailles ; jamais de
  contenu, de jeton (ni son hash) ni de clé.
- TLS dans `tls.rs` : certificat PEM rechargé à chaud, poignées de main hors
  de la boucle d'accueil ; `serve_tls` à côté de `serve`.
- Tests : `tests/auth.rs`, `limits.rs`, `routing.rs`, `store.rs`, `pairing.rs`,
  `blobs.rs` (API des fichiers, client HTTP signé de `tests/common/`),
  `logs.rs` (seul dans son binaire), `e2e.rs`, `e2e_crypto.rs` et
  `e2e_qr.rs` (relais en processus et clients `ot-core`, relais malveillant
  simulé par `proxy`), `tls.rs` (autorité de test générée par `rcgen`) ;
  helpers dans `tests/common/`. Chaque attente est bornée dans le temps.
- Déploiement : `deploy/` (unité systemd durcie sous `ot-relay`, sauvegarde
  des seuls appareils, note 0006, renouvellement du certificat par
  `tailscale cert`, note 0015) ; relais public derrière Caddy
  (`deploy/Caddyfile`, `deploy/docker/`, note 0038), vérifié par
  `deploy/tests/compose-test.sh`. Toute nouvelle option se documente dans le
  README (anglais) et dans `docs/readme-fr.md` (local).
