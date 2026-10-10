# ot-server

Relais : authentifie les appareils par défi Ed25519, stocke les enveloppes
dans SQLite, les livre et purge ce qui est acquitté ou expiré ; enregistre un
nouvel appareil sur jeton d'appairage (note 0020). Binaire `ot-server`
(`serve`, `device add|list|revoke`, `messages purge [--all]`).

- Ne dépend que de `ot-proto`. **Ne lit jamais `payload`**, pas même
  `payload.enc`, et ne dépendra jamais de `ot-crypto`.
- Une session par appareil (registre `registry.rs`, note 0009). Les
  transferts de fichiers HTTP (étape 5) s'authentifient sans y entrer.
- SQLite : migrations versionnées dans `migrations/`, jamais modifiées une fois
  livrées (on en ajoute une) ; écritures en `BEGIN IMMEDIATE` (note 0007) ;
  requêtes `sqlx::query` non vérifiées à la compilation (note 0003).
- Compteurs Prometheus écrits à la main dans `metrics.rs` (note 0004).
- Logs structurés : identifiants d'appareil, `seq`, tailles ; jamais de
  contenu, de jeton (ni son hash) ni de clé.
- Tests : `tests/auth.rs`, `routing.rs`, `store.rs`, `pairing.rs`,
  `logs.rs` (seul dans son binaire), `e2e.rs` et `e2e_crypto.rs` (relais en
  processus et clients `ot-core`, relais malveillant simulé par `proxy`) ;
  helpers dans `tests/common/`. Chaque attente est bornée dans le temps.
- Déploiement : `deploy/` (unité systemd durcie sous `ot-relay`, sauvegarde
  des seuls appareils, note 0006). Toute nouvelle option se documente dans le
  README, section « Serveur ».
