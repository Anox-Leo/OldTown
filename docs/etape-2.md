# Plan détaillé — Étape 2 : chiffrement et appairage

L'étape 2 rend le relais aveugle : tout contenu est chiffré de bout en bout
sous une clé de groupe que seuls mes appareils détiennent, et un nouvel
appareil rejoint le groupe par un code d'appairage, sans session SSH sur le
laptop.

**État (10 octobre 2026)** : code livré (T1 à T9, T11), précisions et écarts
dans les notes [0020](decisions/0020-trames-appairage.md) et
[0021](decisions/0021-format-chiffre-et-roster.md). Reste T10 : la bascule de
l'installation réelle et l'essai Mac → Windows.

Références : jalon « 2. Chiffrement et appairage » de la
[conception](design.md), [revue après l'étape 1](revue-etapes-0-1.md),
notes 0013 (code d'appairage) et 0014 (choix crypto et bascule).

## Objectif et périmètre

**Résultat attendu** : sur le Mac, `ot pair new` affiche un code ; sur le PC
Windows, `ot pair join --server ws://relais:7878 --name windows <code>`
enregistre l'appareil et reçoit la clé de groupe. Ensuite `ot send "bonjour"`
arrive en clair sur le Mac, alors que la base du relais ne contient que des
octets chiffrés. `ot group rotate --exclude windows` coupe l'accès de Windows
aux messages suivants.

**Inclus**

- Crate `ot-crypto` : primitives, formats chiffrés, vecteurs de test figés.
- Clé d'échange X25519 par appareil, clé de groupe par époque, liste des
  membres (« roster ») tenue par les appareils eux-mêmes.
- Chiffrement et signature de chaque message dans `ot-core`, refus du clair.
- Appairage par code : jeton à usage unique sur le relais, secret jamais vu
  par le relais.
- Rotation de la clé de groupe en excluant un appareil.
- Bascule de l'installation réelle (purge, réappairage).

**Exclus**

- QR code et coffres de clés de l'OS (étapes 3 et 4 : ce sont les applis qui
  les apportent). La CLI garde ses clés dans des fichiers `0600`.
- `wss` (étape 3, note 0015), révocation côté relais sans SSH (étape 8).
- Fichiers (étape 5) : leur clé voyagera dans un message chiffré de cette étape.

## Principes

- **Le relais ne change presque pas.** Il ne lit jamais `payload` ; il gagne
  seulement l'appairage (une table, quatre trames). Il ne dépend jamais de
  `ot-crypto` (vérifié en CI, tâche T1).
- **Le format JSON de l'enveloppe ne change pas.** `payload.enc` vaut
  `xchacha20poly1305` (valeur déjà prévue dans `ot-proto`), et la structure
  interne de `payload.data` est définie et figée par `ot-crypto`.
- **La confiance vient de l'appairage, pas du relais.** Un appareil ne croit
  que les clés reçues chiffrées sous la clé d'appairage ou sous la clé de
  groupe. La liste d'appareils du relais sert au routage, jamais à la confiance.

## Modèle cryptographique

| Élément | Où il vit | Rôle |
| --- | --- | --- |
| Paire Ed25519 (existante) | Chaque appareil, `device.key` | Authentification au relais, signature des messages |
| Paire X25519 (nouvelle) | Chaque appareil, `exchange.key` | Recevoir une clé de groupe scellée (appairage, rotation) |
| Clé de groupe `K_n`, 32 octets, époque `n` | Chaque membre, `group.json` | Chiffrer le contenu de tous les messages |
| Roster | Chaque membre, `roster.json` | Pour chaque membre : `device_id`, nom, clés Ed25519 et X25519 publiques, époque d'ajout |
| Secret d'appairage `S`, 16 octets | Code d'appairage, mémoire du parrain | Authentifier et chiffrer l'échange d'appairage |

Bibliothèques (RustCrypto, note 0014) : `chacha20poly1305` (XChaCha20-Poly1305),
`x25519-dalek`, `hkdf` + `sha2`, `ed25519-dalek` (déjà là). Aléa par
`getrandom`. Toutes les clés secrètes sont dans des types qui s'effacent à la
destruction (`zeroize`).

### Format de `payload.data`

Premier octet : le schéma. Tous les schémas chiffrent en XChaCha20-Poly1305
avec un nonce aléatoire de 24 octets.

| Schéma | Octets | Clé utilisée | Usage |
| --- | --- | --- | --- |
| `0x01` groupe | `01 ‖ époque (u32 BE) ‖ nonce ‖ chiffré` | `K_époque` | Tous les messages courants |
| `0x02` appairage | `02 ‖ id du jeton (8 o) ‖ nonce ‖ chiffré` | `HKDF-SHA256(S, info="ot-pair-v1")` | Demande et réponse d'appairage |
| `0x03` appareil | `03 ‖ clé X25519 éphémère (32 o) ‖ nonce ‖ chiffré` | `HKDF-SHA256(X25519(éph, destinataire), info="ot-device-v1" ‖ éph ‖ destinataire)` | Nouvelle clé de groupe pour un seul appareil |

**Données associées** (authentifiées, non chiffrées) : `"ot-env-v1"` ‖ en-tête
du schéma ‖ `msg_id` ‖ `from` ‖ `to` (dans l'ordre de l'enveloppe, séparés par
`,`) ‖ `created_at` (RFC 3339). Le relais ne peut donc ni changer
l'expéditeur, ni rediriger, ni rejouer un contenu sous un autre `msg_id`.

**Clair** : `signature Ed25519 (64 o) ‖ corps JSON`. La signature porte sur
`"ot-sig-v1" ‖ données associées ‖ corps`. Le corps est l'un de :

| `kind` | Contenu | Schéma |
| --- | --- | --- |
| `text`, `link` | `body` (inchangé depuis l'étape 1) | `0x01` |
| `pair_request` | nom, clés publiques Ed25519 et X25519 du nouvel appareil | `0x02` |
| `pair_welcome` | époque et clés de groupe encore utiles, roster complet | `0x02` |
| `roster` | roster complet, envoyé à tous après un ajout | `0x01` |
| `group_key` | nouvelle époque, nouvelle clé, roster sans les exclus | `0x03`, un message par destinataire |

Un appareil rejette (et acquitte, pour ne pas bloquer la file) tout message
dont l'expéditeur n'est pas dans son roster, dont la signature est fausse, ou
qui arrive en clair (`enc: "none"`). Un message dont l'époque est inconnue est
mis de côté (au plus 100) et réessayé après chaque `group_key` reçu.

Les anciennes clés de groupe sont gardées 7 jours (durée de vie des messages)
après une rotation, pour lire les messages en retard.

## Appairage

```text
Parrain A (membre)                 Relais                     Nouvel appareil B
ot pair new
  pair_create ───────────────────▶ stocke sha256(T), 10 min
  ◀────────────── pair_created {T}
  tire S, affiche le code T‖S
                                                    ot pair join <code>
                                                      génère Ed25519 + X25519
                                   ◀── challenge / pair_redeem {T, nom, clé, sig}
                                   crée l'appareil, consomme T
                                   pair_redeemed {device_id, parrain} ──▶
                                   ◀────────────── auth normale (reconnexion)
                                   ◀── send: pair_request (schéma 02, clé de S)
  ◀────────── deliver
  vérifie, ajoute B au roster
  send: pair_welcome (schéma 02) ─▶ ────────── deliver ─────────────────────▶
  send: roster (schéma 01) à tous                     B stocke K et roster
```

- **Code** : `ot1-` puis base32 sans remplissage de `T (10 o) ‖ S (16 o)`,
  soit 46 caractères. Le relais ne voit que `T` ; `S` ne quitte jamais le code.
  L'id du jeton dans l'en-tête `0x02` est `sha256(T)[..8]`.
- **Garanties** : le relais peut refuser l'appairage, pas l'usurper. Sans `S`,
  il ne peut ni lire la demande, ni fabriquer une réponse, ni retrouver `S`
  (128 bits).
- **Délais** : le jeton expire après 10 minutes et ne sert qu'une fois ;
  `ot pair new` attend la demande pendant ce temps puis abandonne.
- `pair_redeem` porte une signature de la nouvelle clé sur
  `"ot-pair-v1" ‖ nonce ‖ sha256(T)` : le relais n'enregistre que des clés
  dont le possesseur est présent.
- Après `pair_redeemed`, le relais ferme la connexion ; B se reconnecte et
  s'authentifie comme n'importe quel appareil. Le code de session du relais
  ne change pas.

**Amorçage** : le tout premier appareil est enregistré comme à l'étape 1
(`ot init` puis `ot-server device add` sur le laptop), puis `ot group init`
crée l'époque 1 et un roster qui ne contient que lui. Les autres appareils de
l'étape 1 sont révoqués et réappairés (deux ou trois machines : plus simple
qu'un chemin de migration).

## Protocole : ce qui change dans `ot-proto`

Un changement de format délibéré, avec une note de décision et de nouveaux
fichiers dans `tests/golden/`. Rien ne change dans les trames existantes.

| Sens | Type | Contenu | Quand |
| --- | --- | --- | --- |
| client → serveur | `pair_create` | — | Session authentifiée |
| serveur → client | `pair_created` | `token` (base64, 10 o), `expires_at` | Réponse |
| client → serveur | `pair_redeem` | `token`, `name`, `public_key`, `sig` | À la place de `auth`, après `challenge` |
| serveur → client | `pair_redeemed` | `device_id`, `sponsor` | Puis fermeture |

Nouveaux codes d'erreur : `pairing_invalid` (inconnu, expiré ou déjà utilisé ;
ferme la connexion comme `auth_failed`), `name_taken`.

## Modèle de données

**Relais** : migration `0002_pairings.sql`.

| Table | Colonnes | Remarques |
| --- | --- | --- |
| `pairings` | `token_hash` BLOB PK (SHA-256), `sponsor_id` FK, `created_at`, `expires_at`, `used_at` NULL, `device_id` NULL FK | Purgée avec les messages ; jamais le jeton en clair |

Nouvelle sous-commande d'administration : `ot-server messages purge --all`
(bascule, note 0014). Nouveaux compteurs : `pairings_created`, `pairings_redeemed`,
`pairings_rejected`.

**Client (CLI)**, dans le dossier `--home` : `exchange.key` (`0600`),
`group.json` (`0600`, clés par époque avec date de retrait), `roster.json`,
`quarantine/` (messages d'époque inconnue). `ot-core` accède aux clés par un
trait `Keyring` : la CLI l'implémente avec ces fichiers, les applis le feront
avec le coffre de l'OS.

## Découpage en tâches

Chaque tâche est autonome : elle dit quoi toucher, quoi tester et quand elle
est finie. Ordre conseillé : T1 → T2 → T3 en série, T4 en parallèle de T2-T3,
puis T5 à T11. Chaque tâche finit par les quatre commandes de validation de
`CLAUDE.md`, toutes vertes.

- [x] **T1. Crate `ot-crypto`.**
  Fichiers : `crates/ot-crypto/`, `Cargo.toml` (workspace), `deny.toml` si une
  licence manque, `.github/workflows/ci.yml`.
  Faire : types de clés (`GroupKey`, `ExchangeKeyPair`, `PairingSecret`) avec
  `zeroize` ; `seal`/`open` pour les trois schémas ; construction des données
  associées à partir d'une `ot_proto::Envelope` ; signature et vérification ;
  encodage du code d'appairage. Pas de tokio, pas de réseau, pas d'E/S.
  Ajouter en CI une vérification que `ot-server` ne dépend pas de `ot-crypto`
  (dépendances normales et de build : ses tests y ont accès par `ot-core`,
  note 0021).
  Tests : aller-retour par schéma ; chaque altération (en-tête, nonce, chiffré,
  `msg_id`, `from`, `to`, `created_at`) est rejetée ; mauvaise clé rejetée ;
  vecteurs figés dans `crates/ot-crypto/tests/vectors/` (clés et nonces fixés),
  sur le modèle des golden de `ot-proto`.
  Fini quand : les vecteurs passent et la vérification CI est en place.

- [x] **T2. Clés, roster et `Keyring` dans `ot-core`.**
  Fichiers : `crates/ot-core/src/keyring.rs`, `roster.rs`, `lib.rs`.
  Faire : trait `Keyring` (lire et écrire clé d'échange, clés de groupe,
  roster) ; implémentation en mémoire pour les tests ; règles du roster
  (ajout, remplacement complet signé par un membre, retrait à la rotation).
  Tests : roster refusé s'il vient d'un non-membre ; époques conservées puis
  retirées après 7 jours.
  Fini quand : l'API est documentée (`///`) et pensée pour UniFFI (types
  simples, pas de génériques exposés).

- [x] **T3. Chiffrer et signer chaque message.**
  Fichiers : `crates/ot-core/src/client.rs`, nouveau `crates/ot-core/src/secure.rs`.
  Faire : `Client::send` chiffre (schéma `0x01`, époque courante) et signe ;
  à la réception, déchiffre, vérifie l'expéditeur dans le roster et la
  signature, puis émet `Event::Message`. Les messages système
  (`roster`, `group_key`, `pair_*`) sont traités par le cœur et donnent un
  `Event::GroupChanged`. Quarantaine des époques inconnues.
  Tests : deux clients en mémoire échangent ; message en clair rejeté ;
  expéditeur hors roster rejeté ; signature fausse rejetée ; quarantaine
  vidée à l'arrivée de la clé.
  Fini quand : `Received.content` n'est plus jamais rempli depuis un payload
  en clair.

- [x] **T4. Appairage côté relais.**
  Fichiers : `crates/ot-proto/src/frame.rs`, `tests/golden/pair_*.json`,
  `crates/ot-server/migrations/0002_pairings.sql`, `src/store.rs`,
  `src/session.rs`, `src/metrics.rs`, note de décision sur les trames
  d'appairage (`docs/decisions/NNNN-trames-appairage.md`, prochain numéro libre).
  Faire : les quatre trames et deux codes d'erreur ; table `pairings` ;
  `pair_create` en session authentifiée ; `pair_redeem` dans la phase
  d'authentification (vérifie signature, jeton, unicité du nom), crée
  l'appareil puis ferme ; purge des jetons expirés ; compteurs.
  Tests : jeton valide ; expiré ; déjà utilisé ; inconnu ; signature fausse ;
  nom déjà pris ; `pair_create` avant `auth` refusé ; aucun jeton en clair en
  base ni dans les logs.
  Fini quand : la note sur les trames existe et les golden sont à jour.

- [x] **T5. Commandes de groupe et d'appairage dans la CLI.**
  Fichiers : `crates/ot-cli/src/main.rs`, `files.rs`.
  Faire : `Keyring` sur fichiers `0600` ; `ot group init` ;
  `ot pair new` (affiche le code, attend 10 min) ;
  `ot pair join --server <url> --name <nom> <code>` ;
  `ot group show` (époque, membres, empreintes courtes des clés).
  Tests : permissions des fichiers sous Unix ; `ot group init` refuse si un
  groupe existe déjà (sauf `--force`).
  Fini quand : le README décrit ces commandes.

- [x] **T6. Échange d'appairage.**
  Fichiers : `crates/ot-core/src/pairing.rs`, `client.rs`.
  Faire : côté nouveau, `pair_redeem` puis reconnexion, envoi de
  `pair_request` (schéma `0x02`) au parrain ; côté parrain, vérification,
  ajout au roster, `pair_welcome` au nouveau, `roster` à tous.
  Tests (E2E, relais en processus) : A parraine B, puis B écrit à A et à C ;
  un relais qui modifie `pair_request` ou `pair_welcome` fait échouer
  l'appairage ; un code faux échoue côté relais.
  Fini quand : un troisième appareil apprend B par le message `roster`.

- [x] **T7. Avertir d'un appareil inconnu.**
  Fichiers : `crates/ot-cli/src/main.rs`, `crates/ot-core/src/client.rs`.
  Faire : `ot devices` croise la liste du relais et le roster, et signale un
  appareil présent sur le relais mais absent du roster (relais compromis ou
  appairage inachevé). Les envois ciblés (`--to`) ne visent que des membres
  du roster.
  Tests : appareil ajouté par `ot-server device add` sans appairage → signalé.

- [x] **T8. Rotation de la clé de groupe.**
  Fichiers : `crates/ot-core/src/secure.rs`, `crates/ot-cli/src/main.rs`.
  Faire : `ot group rotate [--exclude <nom>]...` tire `K_{n+1}`, envoie un
  `group_key` (schéma `0x03`) à chaque membre restant, puis chiffre sous la
  nouvelle époque. Rappeler dans la sortie de lancer
  `ot-server device revoke <nom>` sur le laptop.
  Tests (E2E) : après rotation excluant C, C ne lit plus les nouveaux
  messages ; A et B lisent encore les anciens en retard.

- [x] **T9. Plus aucun clair.**
  Fichiers : `crates/ot-core`, `crates/ot-cli`, `crates/ot-proto/src/envelope.rs`.
  Faire : supprimer l'envoi en clair des clients (`Envelope::new_plain` ne
  sert plus qu'aux tests) ; un client sans groupe refuse d'envoyer et dit
  pourquoi. Le relais reste inchangé (note 0014).
  Tests : un test E2E lit la base du relais et vérifie qu'aucun `body` envoyé
  n'y apparaît.

- [ ] **T10. Bascule de l'installation réelle.** *(Documentation prête :
  README « Passage au chiffrement » et `SECURITY.md` ; reste l'opération sur
  le laptop et les appareils, puis l'essai réel.)*
  Fichiers : `README.md`, `SECURITY.md`, `docs/etape-2.md` (état).
  Faire : mettre à jour le relais ; `ot-server messages purge --all` (suivi d'un
  `VACUUM`, note 0014) ;
  `ot group init` sur le premier appareil ; révoquer et réappairer les autres.
  Mettre à jour le modèle de menace de `SECURITY.md` et cocher sa première
  ligne. Retirer l'avertissement « contenu en clair » du README.
  Fini quand : l'essai réel ci-dessous passe.

- [x] **T11. Tests de bout en bout.**
  Fichiers : `crates/ot-server/tests/e2e.rs` ou nouveau `e2e_crypto.rs`.
  Faire : les scénarios du tableau ci-dessous, avec un relais en processus.

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Appairage par code | B rejoint, reçoit la clé, échange avec A | E2E + essai réel Mac → Windows |
| Relais qui altère l'appairage | Échec, B n'est pas ajouté au roster | E2E |
| Code faux, expiré, déjà utilisé | `pairing_invalid`, connexion fermée | Test relais |
| Base du relais | Aucun texte envoyé en clair | E2E |
| Message en clair, expéditeur hors roster, signature fausse | Rejeté, acquitté, signalé | Test `ot-core` |
| Rotation excluant un appareil | Il ne lit plus les nouveaux messages ; les autres lisent les anciens | E2E |
| Message d'époque inconnue | Mis de côté puis lu à l'arrivée de la clé | Test `ot-core` |
| Format chiffré | Vecteurs figés inchangés | `ot-crypto/tests/vectors` |
| Dépendances | `ot-server` ne dépend pas de `ot-crypto` | CI |

En plus : CI verte, aucune clé ni contenu dans les logs, README et SECURITY à
jour, notes de décision pour chaque écart.

## Risques

- **Erreur de construction crypto.** Mitigation : primitives standard
  uniquement, données associées couvrant tout le routage, vecteurs figés, et
  la relecture externe prévue dans `SECURITY.md` avant publication.
- **Perte de la clé de groupe** si tous les appareils sont perdus : les
  messages en attente deviennent illisibles, ce qui est voulu. On recrée un
  groupe.
- **Rotation concurrente** par deux appareils : la plus haute époque gagne ;
  à égalité, le plus petit `device_id`. Cas improbable, à tester quand même.
