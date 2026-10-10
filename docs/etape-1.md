# Plan détaillé — Étape 1

L'étape 1 livre un relais qui tourne en service sur le laptop Ubuntu et une CLI qui envoie un texte d'un terminal à un autre, via Tailscale, avec mise en attente pour les appareils hors ligne. Elle pose surtout le format d'enveloppe et le schéma de données que les étapes 2 à 8 réutiliseront sans migration douloureuse.

Référence : jalon « 1. Fondations » de [la conception](design.md).

## Objectif et périmètre

**Résultat attendu** : depuis le PC Windows, `ot send "bonjour"` s'affiche sur le terminal du Mac qui exécute `ot listen` ; si le Mac est éteint, le message arrive à sa reconnexion.

**Inclus**

- Préparation du laptop serveur et du tailnet (laptop, Mac, PC Windows).
- Workspace Rust : types du protocole, cœur client, serveur relais, CLI.
- Protocole WebSocket versionné : authentification d'appareil, envoi, livraison, accusé de réception.
- Stockage SQLite côté serveur : appareils, messages en attente, état de livraison, purge par durée de vie.
- Déploiement systemd, logs structurés, métriques, CI.

**Exclus (étapes suivantes)**

- Chiffrement du contenu et clé de groupe, appairage par QR (étape 2). Le format d'enveloppe prévoit déjà le champ chiffré.
- Applis Android et desktop (étapes 3 et 4), fichiers (5), presse-papiers (6), extension (7).
- Historique côté client, UnifiedPush, révocation complète (8).

## Composants

Quatre crates dans un seul workspace Cargo ; seul `ot-core` sera repris tel quel par les applis Android et desktop.

&#91;embedded content: composants de l'étape 1 · 4 crates, 1 serveur\]

| Crate | Type | Contenu | Dépendances clés |
| --- | --- | --- | --- |
| `ot-proto` | bibliothèque | Trames, enveloppe, codes d'erreur, version du protocole | serde, ulid |
| `ot-core` | bibliothèque | Client : connexion, auth, reconnexion, dédoublonnage, clés | tokio, tokio-tungstenite, ed25519-dalek |
| `ot-server` | binaire | Relais, stockage, purge, sous-commandes d'admin | axum, sqlx, tracing, metrics |
| `ot-cli` (binaire `ot`) | binaire | Outil de test et d'usage en terminal | clap, ot-core |

Le serveur n'importe jamais `ot-core` : il ne dépend que de `ot-proto`, ce qui garantit qu'il ne touchera pas aux clés de contenu à l'étape 2.

## Protocole et API

Une seule connexion WebSocket par appareil porte tout le trafic ; chaque trame est un objet JSON `{"v":1,"t":"<type>", ...}` et tout type inconnu est rejeté avec une erreur explicite.

**Endpoints HTTP** (écoute uniquement sur l'IP Tailscale du laptop)

| Méthode et chemin | Rôle |
| --- | --- |
| `GET /v1/ws` | Upgrade WebSocket, toute la messagerie |
| `GET /healthz` | Vivant et base accessible (pour systemd et la supervision) |
| `GET /metrics` | Métriques Prometheus : connexions, messages reçus, livrés, en attente, purgés |

L'administration (ajouter ou retirer un appareil) passe par une sous-commande locale du binaire serveur, `ot-server device add <nom>`, pas par l'API réseau.

**Trames**

| Sens | Type | Contenu | Effet |
| --- | --- | --- | --- |
| serveur → client | `challenge` | `nonce` (32 octets) | Envoyé à l'ouverture |
| client → serveur | `auth` | `device_id`, `sig` (signature du nonce) | Authentifie la connexion |
| serveur → client | `welcome` | `device_id`, `pending` | Ouvre la session, annonce les messages en attente |
| client → serveur | `send` | `env` (enveloppe) | Le serveur stocke puis route |
| serveur → client | `accepted` | `msg_id`, `seq` | L'expéditeur sait que c'est stocké |
| serveur → client | `deliver` | `seq`, `env` | Livraison d'un message |
| client → serveur | `ack` | `seq` | Marque livré pour cet appareil |
| les deux | `error` | `code`, `message`, `ref` | Erreur typée, ne coupe pas la session sauf `auth_failed` |

Le ping/pong natif de WebSocket (toutes les 30 s) détecte les connexions mortes.

**Enveloppe** (seule partie qui traversera le chiffrement de l'étape 2)

```json
{
  "msg_id": "01J9Z… (ULID généré par l'expéditeur)",
  "from": "dev_…",
  "to": ["dev_…"],
  "created_at": "2026-10-08T19:30:00Z",
  "payload": { "enc": "none", "data": "<base64>" }
}
```

- `to` vide signifie « tous mes autres appareils ».
- `payload.data` contient un objet `{"kind":"text"|"link","body":"…"}` sérialisé. À l'étape 2, `enc` passe à `xchacha20poly1305` et le serveur ne change pas : il ne lit jamais `payload`.
- Le serveur vérifie que `from` correspond à l'appareil authentifié et rejette les enveloppes au-delà de 64 Kio.

**Sémantique de livraison** : au moins une fois. Le serveur renvoie tout message non acquitté à chaque reconnexion, dans l'ordre de `seq` ; le client ignore un `msg_id` déjà vu. Un `send` rejoué avec le même `msg_id` est idempotent.

## Modèle de données

Le serveur tient trois tables SQLite (mode WAL) ; le contenu des messages y est un blob opaque, et une ligne de livraison par destinataire permet d'acquitter appareil par appareil.

| Table | Colonnes | Remarques |
| --- | --- | --- |
| `devices` | `id` TEXT PK (`dev_` + ULID), `name`, `public_key` BLOB (Ed25519, 32 octets), `created_at`, `last_seen_at`, `revoked_at` NULL | Un appareil révoqué est refusé au `auth` ; la ligne reste pour l'audit |
| `messages` | `seq` INTEGER PK auto, `msg_id` TEXT UNIQUE, `sender_id` FK, `created_at`, `expires_at`, `size`, `envelope` BLOB | `seq` donne l'ordre global ; `msg_id` assure l'idempotence |
| `deliveries` | `seq` FK, `device_id` FK, `delivered_at` NULL, `acked_at` NULL, PK (`seq`, `device_id`) | Créées à l'insertion du message, une par destinataire résolu |

- **Purge** : une tâche périodique (toutes les 10 min) supprime les messages dont toutes les livraisons sont acquittées, ou dont `expires_at` est dépassé. Durée de vie par défaut : 7 jours, configurable.
- **Migrations** versionnées dans le dépôt, appliquées au démarrage du serveur.
- **Index** : `deliveries(device_id, acked_at)` pour retrouver vite les messages en attente d'un appareil.

**Côté client (CLI)**, pas de base à cette étape : un fichier de configuration TOML (`server_url`, `device_id`, `device_name`), la clé privée dans un fichier en `0600`, et un petit fichier d'état avec les derniers `msg_id` vus pour dédoublonner. Le cache SQLite client arrive avec l'historique.

## Choix cryptographiques

Je propose d'avancer l'identité d'appareil Ed25519 dès l'étape 1 et de laisser le chiffrement du contenu à l'étape 2 : l'identifiant et la clé publique sont la clé primaire de tout le modèle, les changer plus tard coûterait une migration, alors que la signature d'un défi tient en une cinquantaine de lignes.

| Sujet | Étape 1 | Étape 2 |
| --- | --- | --- |
| Identité d'appareil | Paire Ed25519 générée par `ot init` ([ed25519-dalek](https://docs.rs/ed25519-dalek)) | Inchangée, plus une paire X25519 pour l'échange de clé |
| Enregistrement | `ot init` affiche la clé publique ; `ot-server device add <nom> <clé>` sur le laptop | Appairage par QR, jeton à usage unique |
| Authentification | Défi : nonce 32 octets aléatoire, signature sur `"ot-auth-v1" ‖ nonce ‖ device_id` | Inchangée |
| Transport | WireGuard via Tailscale, WebSocket en clair sur l'IP du tailnet | Option `wss` avec certificat `tailscale cert` |
| Contenu | `enc: "none"`, le serveur pourrait le lire | XChaCha20-Poly1305 sous la clé de groupe, signé par l'expéditeur |
| Stockage de la clé privée | Fichier `0600` (CLI uniquement) | Coffre de l'OS dans les applis |

- Le préfixe de domaine `ot-auth-v1` empêche de réutiliser une signature d'authentification pour autre chose ; le `device_id` dans le message signé lie la réponse à l'appareil.
- Le nonce est à usage unique et expire après 30 s ; trois échecs d'`auth` consécutifs ferment la connexion.
- **Limite assumée** : pendant l'étape 1, ne pas faire transiter de données sensibles. Tailscale protège le réseau, mais la base du laptop contient les textes en clair jusqu'à leur purge.

## Découpage en tâches

Dix tâches dans l'ordre des dépendances ; à partir de T4, chaque tâche se termine par un test qui passe en CI.

- [x] **T1. Laptop et tailnet.** Veille capot fermé désactivée, seuil de charge, Ethernet, mises à jour automatiques, SSH par clé. Tailscale sur laptop, Mac et PC, avec un nom MagicDNS stable pour le serveur.
- [x] **T2. Dépôt et CI.** Workspace Cargo, `rustfmt`, `clippy -D warnings`, tests, `cargo deny` pour les licences et failles connues, sur Linux, macOS et Windows.
- [x] **T3. Crate `ot-proto`.** Types des trames et de l'enveloppe, sérialisation serde, constantes de version. Tests « golden » sur des fichiers JSON de référence pour figer le format.
- [x] **T4. Stockage serveur.** Schéma, migrations, couche d'accès (insertion idempotente, messages en attente par appareil, acquittement, purge). Tests sur base en mémoire.
- [x] **T5. Authentification.** Génération et vérification du défi, sous-commande `ot-server device add|list|revoke`. Tests : bonne clé, mauvaise clé, nonce rejoué, appareil révoqué.
- [x] **T6. Routage.** Registre des connexions actives, `send` → stockage → `accepted` → `deliver` aux connectés, rejeu des en attente au `welcome`, `ack`. Une seule connexion active par appareil, la nouvelle remplace l'ancienne.
- [x] **T7. Crate `ot-core`.** Client : connexion, authentification, reconnexion avec backoff exponentiel et gigue, file d'envoi en mémoire, dédoublonnage, flux de messages reçus. API pensée pour UniFFI (étape 3) et Tauri (étape 4).
- [x] **T8. CLI `ot`.** `ot init --server <url> --name <nom>`, `ot send <texte> [--to <nom>]`, `ot listen`, `ot devices`.
- [x] **T9. Observabilité et déploiement.** Logs structurés `tracing` (jamais le contenu des messages), `/healthz`, `/metrics`, unité systemd durcie (`DynamicUser`, `ProtectSystem=strict`, `Restart=on-failure`), sauvegarde de la base.
- [x] **T10. Tests de bout en bout.** Serveur lancé en processus dans les tests, deux clients `ot-core` : livraison en ligne, livraison différée, coupure réseau, redémarrage du serveur, expiration. Puis essai réel Windows → Mac via le laptop.

## État au 10 octobre 2026 : étape terminée

- **Fait** : T1 à T10. 49 tests automatisés verts en CI sur Linux, macOS et Windows ; relais installé en service sur le laptop ; essai réel Windows → Mac réussi (en ligne, lien ciblé, hors ligne, reconnexion sans doublon) ; après un reboot du laptop, le service repart seul et `/healthz` répond.
- **Constaté pendant l'essai réel** : un autre VPN actif sur le PC empêchait le trafic vers le tailnet ; le service tente parfois une première fois de démarrer avant que l'IP Tailscale soit montée, puis repart 5 s plus tard. Les deux cas sont dans la section « Dépannage » du README.
- **Écarts** : voir [`decisions/`](decisions/README.md), notes 0001 à 0009. En bref : trames `list_devices`/`devices` ajoutées, premier `auth_failed` fermant, requêtes sqlx non vérifiées à la compilation, compteurs maison, service sous `ot-relay`, sauvegarde des seuls appareils, écritures en `BEGIN IMMEDIATE`, Rust 1.88. Question ouverte : une session par appareil et la CLI (0009).

## Critères de fin

L'étape est terminée quand le scénario Windows → Mac marche en vrai et que chaque cas ci-dessous a un test automatisé vert en CI.

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Les deux appareils en ligne | Le texte s'affiche sur le Mac en moins d'une seconde | Test E2E + essai réel |
| Destinataire hors ligne | Livré à la reconnexion, dans l'ordre d'envoi | Test E2E |
| Coupure pendant la livraison | Livré de nouveau, affiché une seule fois | Test E2E |
| `send` rejoué avec le même `msg_id` | Un seul message stocké | Test de stockage |
| Redémarrage du serveur | Les messages en attente survivent | Test E2E |
| Clé inconnue, signature fausse, appareil révoqué | Connexion refusée avec `auth_failed` | Test d'auth |
| Enveloppe > 64 Kio ou `from` usurpé | Rejetée avec une erreur typée | Test de routage |
| Durée de vie dépassée | Message purgé, compteur `/metrics` incrémenté | Test de purge |
| Reboot du laptop | Le service redémarre seul, `/healthz` répond | Essai manuel (10 octobre 2026) |

En plus : CI verte sur les trois OS, `clippy` sans avertissement, aucun contenu de message dans les logs, README avec l'installation du serveur et de la CLI.

## Questions ouvertes

Le plan suit déjà la recommandation de chaque ligne ; une réponse différente ne change que les sections indiquées.

| # | Question | Recommandation | Alternative | Impacte |
| --- | --- | --- | --- | --- |
| Q1 | Authentification à l'étape 1 | Clé Ed25519 et défi signé dès maintenant | Jeton statique par appareil, remplacé à l'étape 2 | Crypto, T5 |
| Q2 | Langage du serveur | Rust (axum + tokio), même langage que le cœur | Kotlin + Quarkus, que tu connais déjà ; Go | Composants, T4 à T6 |
| Q3 | Format sur le fil | JSON, lisible et facile à déboguer | CBOR ou Protobuf, plus compact pour les fichiers | Protocole, T3 |
| Q4 | Accès SQLite | `sqlx` (async, requêtes vérifiées à la compilation) | `rusqlite` dans un pool bloquant, plus simple | T4 |
| Q5 | TLS en plus de Tailscale | Non à l'étape 1, WireGuard suffit | `wss` avec `tailscale cert` dès maintenant | Crypto, T9 |
| Q6 | Dépôt | Un monorepo GitHub privé pour tout (serveur, cœur, applis) | Un dépôt par composant | T2 |
| Q7 | Durée de vie des messages | 7 jours | Plus court (24 h) pour limiter ce qui reste sur le laptop | Modèle de données |

Q1 et Q2 sont tranchées le 8 octobre : clé Ed25519 dès l'étape 1 et serveur en Rust. Les autres se changent facilement plus tard.
