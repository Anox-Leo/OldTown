# 0021 — Format chiffré, roster et cœur client de l'étape 2

**Statut : acceptée** (10 octobre 2026). Tâches T1 à T3 et T5 à T9 de
l'étape 2.

**Contexte.** Le plan de l'étape 2 fixe les trois schémas de `payload.data`,
les données associées et la signature. En codant, plusieurs points ont dû être
précisés, et quelques-uns s'écartent du plan.

**Décision.**

- **Format figé** par `crates/ot-crypto/tests/vectors/` (six messages et un
  code d'appairage), conforme au plan : en-têtes `0x01`/`0x02`/`0x03`,
  XChaCha20-Poly1305 à nonce aléatoire, HKDF-SHA256 sans sel, clair
  `signature ‖ corps JSON`. Les vecteurs ont été recoupés avec une
  implémentation indépendante (Python `cryptography`, HChaCha20 recodé).
  Une clé X25519 de faible ordre est refusée (secret non contributif).
- **Corps** (`ot_crypto::Body`, `kind` en tête, champs inconnus refusés) :
  `text` et `link` gardent le format de l'étape 1 ; `pair_request {name,
  sign_key, exchange_key}` ; `pair_welcome {keys: [{epoch, key}], roster}` ;
  `roster {roster}` ; `group_key {epoch, key, roster}`.
- **Roster versionné** (précision) : `{epoch, version, members}`, chaque
  membre avec `device_id`, `name`, `sign_key`, `exchange_key`,
  `added_epoch`. Un roster n'en remplace un autre que si `(epoch, version)`
  est plus grand, s'il vient d'un membre, s'il contient l'appareil et s'il ne
  change les clés d'aucun membre. Sans cette version, un `roster` en retard
  pourrait réintroduire un appareil exclu. Un `roster` rejoué est ignoré sans
  bruit.
- **Rotation concurrente** : la plus haute époque gagne ; à égalité, la clé
  tirée par le plus petit `device_id` (l'auteur est gardé avec chaque époque).
- **Quarantaine** : un seul fichier `quarantine.json` (`0600`) au lieu d'un
  dossier `quarantine/` (écart) ; au plus 100 messages, 7 jours.
- **API de `ot-core`** : `ClientConfig::new` prend un `Arc<dyn Keyring>` ;
  `Received.content` n'est plus optionnel ; nouveaux événements `Rejected`,
  `GroupChanged`, `PairingCompleted`, `KeyringError` ; nouvelles erreurs
  `NoGroup`, `NotMember`. `Client::send` chiffre toujours ; `send_envelope`
  envoie telle quelle une enveloppe déjà faite (tests).
- **Fermeture propre** (correctif trouvé à l'essai) : à l'arrêt, le client
  attend la réponse du relais à sa trame de fermeture (2 s au plus). Sinon un
  processus qui quitte juste après avoir acquitté perd l'acquittement, et le
  parrain recevait de nouveau la `pair_request`.
- **Vérification CI** (écart) : `cargo tree -i` ne sort jamais en erreur, et
  les tests de `ot-server` utilisent légitimement `ot-core`, donc `ot-crypto`.
  La CI vérifie que `ot-crypto` n'apparaît pas dans les dépendances normales
  et de build de `ot-server`.
- **CLI** : `ot pair join` crée toujours une identité neuve (et refuse si
  l'appareil est déjà enregistré, sauf `--force`) ; `ot init --force` oublie
  le groupe (nouvelle identité). `ot pair new` affiche les messages reçus
  pendant l'attente.

**Conséquences.**

- Le parrainage n'existe qu'en mémoire : si `ot pair new` s'arrête avant la
  demande, il faut un nouveau code. De même, si `ot pair join` s'arrête avant
  la réponse, la refaire avec `--force` et un nouveau code.
- Un relais qui altère la `pair_welcome` fait échouer l'appairage côté nouvel
  appareil, mais le parrain l'a déjà ajouté au roster. Il ne peut rien en
  faire (il n'a pas la clé de groupe) ; `ot group rotate --exclude <nom>` le
  retire.
- Tout changement de format chiffré casse un vecteur : il demande une note.
