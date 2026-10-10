# 0020 — Trames d'appairage côté relais

**Statut : acceptée** (10 octobre 2026). Tâche T4 de l'étape 2.

**Contexte.** Le plan de l'étape 2 fixe quatre trames et deux codes d'erreur
pour l'appairage par code, sans préciser plusieurs détails : version du
protocole, sort du jeton en cas d'erreur, validation du nom, compteurs.

**Décision.**

- **Trames** (protocole toujours en `v: 1`, l'ajout est compatible : un relais
  de l'étape 1 répond `unknown_type`) :
  `pair_create` (client authentifié → relais, sans champ) ;
  `pair_created {token, expires_at}` (jeton de 10 octets en base64,
  RFC 3339) ; `pair_redeem {token, name, public_key, sig}` (à la place
  d'`auth`, après `challenge`) ; `pair_redeemed {device_id, sponsor}`, puis
  fermeture. Golden : `crates/ot-proto/tests/golden/pair_*.json`.
- **Signature** de `pair_redeem` : `"ot-pair-v1" ‖ nonce ‖ sha256(jeton)`,
  construite par `ot_proto::pair_redeem_message`. Le relais calcule le SHA-256
  avec `sha2` (pas `ot-crypto`).
- **Erreurs**, qui toutes ferment la connexion : `pairing_invalid` (jeton
  inconnu, expiré, déjà utilisé, parrain révoqué, signature fausse ou clé déjà
  enregistrée), `name_taken`, `bad_frame` pour un nom vide, de plus de 64
  caractères ou avec un caractère de contrôle. Un nom pris ou une signature
  fausse **ne consomment pas** le jeton : on peut réessayer avec le même code.
- **Stockage** : table `pairings` (migration `0002_pairings.sql`), clé
  primaire `sha256(jeton)`, jamais le jeton. La consommation du jeton et la
  création de l'appareil se font dans une seule transaction. La purge
  périodique supprime les jetons expirés, utilisés ou non.
- **Durée** : 10 minutes (`Config::pairing_ttl`, non exposée en option).
- **Compteurs** : une seule métrique `ot_pairings_total` avec l'étiquette
  `result` (`created`, `redeemed`, `rejected`) plutôt que trois noms, comme
  `ot_messages_purged_total`.
- **Administration** : `ot-server messages purge` (purge ponctuelle) et
  `ot-server messages purge --all` (tous les messages, puis `VACUUM`, note
  0014).

**Conséquences.** Un relais et des clients de l'étape 2 doivent être déployés
ensemble pour l'appairage ; le reste du protocole est inchangé. Les journaux
du relais ne contiennent ni le jeton ni son hash (test `tests/logs.rs`).
