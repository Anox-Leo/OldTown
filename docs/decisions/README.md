# Décisions

Une note par décision qui s'écarte des plans de `docs/` ou qui les précise.
Le code fait foi ; les plans des étapes suivantes partent de ces notes.

Format : contexte, décision, conséquences. Une décision remplacée n'est pas
effacée : on la marque « Remplacée par NNNN ».

| # | Décision | Étape | Statut |
| --- | --- | --- | --- |
| [0001](0001-trames-liste-appareils.md) | Trames `list_devices` et `devices` | 1 | Acceptée |
| [0002](0002-echec-auth-ferme.md) | Un échec d'authentification ferme la connexion | 1 | Acceptée |
| [0003](0003-sqlx-requetes-dynamiques.md) | sqlx sans requêtes vérifiées à la compilation | 1 | Acceptée |
| [0004](0004-metriques-maison.md) | Compteurs Prometheus écrits à la main | 1 | Acceptée |
| [0005](0005-service-utilisateur-ot-relay.md) | Service sous `ot-relay` plutôt que `DynamicUser` | 1 | Acceptée |
| [0006](0006-sauvegarde-appareils.md) | La sauvegarde ne contient que les appareils | 1 | Acceptée |
| [0007](0007-sqlite-begin-immediate.md) | Écritures SQLite en `BEGIN IMMEDIATE` | 1 | Acceptée |
| [0008](0008-rust-1-88.md) | Rust 1.88 minimum | 1 | Acceptée |
| [0009](0009-une-session-par-appareil.md) | Une session par appareil : conséquences pour la CLI | 1 | Acceptée |
| [0010](0010-depot-public-mit.md) | Monorepo, licence MIT, publication après l'étape 2 | 1 | Publication remplacée par 0011 |
| [0011](0011-publication-fin-de-projet.md) | Publication à la fin du projet | 1 | Acceptée |
| [0012](0012-organisation-du-travail.md) | Organisation du travail et ordre des jalons (D1 à D4) | 2 | Ordre des jalons remplacé par 0017 |
| [0013](0013-appairage-code-puis-qr-inverse.md) | Appairage : code long à l'étape 2, QR inversé à l'étape 3 (D5) | 2 | Acceptée |
| [0014](0014-chiffrement-etape-2.md) | Chiffrement de l'étape 2 : bibliothèque, clé de groupe, bascule (D6 à D11) | 2 | Acceptée |
| [0015](0015-wss-tailscale-cert.md) | `wss` dès l'étape 3, avec un certificat `tailscale cert` (D13) | 3 | Acceptée |
| [0016](0016-android-service-tuile-qr.md) | Android : service `specialUse`, pas de tuile, zxing-cpp (D14, D15, D18) | 3 | Acceptée |
| [0017](0017-extension-chrome-sans-appli-desktop.md) | Pas d'appli desktop : extension Chrome autonome à l'étape 4 (D12, D16, D17, D19, D21) | 4 | Acceptée |
| [0018](0018-fichiers-historique-revocation.md) | Fichiers reçus, historique local, révocation (D20, D22, D23) | 5, 8 | Acceptée |
| [0019](0019-presse-papiers-manuel.md) | Presse-papiers : envoi manuel, écriture à la réception (D24) | 6 | Acceptée |
| [0020](0020-trames-appairage.md) | Trames d'appairage côté relais | 2 | Acceptée |
| [0021](0021-format-chiffre-et-roster.md) | Format chiffré, roster versionné et cœur client | 2 | Acceptée |
| [0022](0022-etape-3-wss-qr-inverse-ffi.md) | `wss`, QR inversé (trames, schéma `0x04`), pont UniFFI, appli Android | 3 | Acceptée |
| [0023](0023-extension-wasm-et-transport-ts.md) | Extension : cœur de groupe en WebAssembly, transport en TypeScript, `ping`/`pong` | 4 | Acceptée |
| [0024](0024-retours-essai-extension-brave.md) | Retours d'essai : Brave, ouverture directe des éléments reçus, raccourci presse-papiers, notifications Android | 4 | Acceptée |
| [0025](0025-diagnostic-du-relais.md) | Diagnostic du relais envoyé par un membre dédié `relais-diag` | Hors jalon | Acceptée |
| [0026](0026-relecture-etape-5.md) | Relecture de l'étape 5 : `aead-stream`, accès direct aux morceaux, clés de fichier en Rust, API HTTP, permissions de l'extension | 5 | Acceptée |
| [0027](0027-image-par-domaine-transferts-android.md) | Images des pages autorisées domaine par domaine ; transferts Android dans le service (D25, D26) | 5 | Acceptée |
| [0028](0028-etape-5-realisation.md) | Étape 5 : reprise du retrait limitée à l'appli ouverte, géométrie chiffrée, état des fichiers sérialisé, choix de la CLI, de l'extension et d'Android | 5 | Acceptée |
| [0029](0029-extension-fichiers-ouverts-par-le-navigateur.md) | Extension : fichiers reçus ouverts par le navigateur, téléchargement sur un geste, trombone qui ouvre le sélecteur (D27) | 5 | Acceptée |
| [0030](0030-extension-conversation-par-canal.md) | Extension : la popup devient une conversation par canal (« Tous », un par appareil) | 5 | Acceptée |
| [0031](0031-extension-accueil-liste-de-conversations.md) | Extension : accueil en liste de conversations (avatars, présence), conversation ouverte au clic | 5 | Acceptée |
| [0032](0032-popup-ouverte-sur-la-conversation.md) | Extension : la popup ouverte pour un élément reçu (d'office ou par sa notification) montre sa conversation | 5 | Acceptée |
| [0033](0033-android-conversations-partage-par-usage.md) | Android : conversations comme l'extension (envois dans l'historique, destinataires des reçus), partage trié par usage | Hors jalon | Acceptée |
| [0034](0034-etape-6-abandonnee.md) | Étape 6 (presse-papiers) abandonnée ; prochaine étape : la 8 | 6, 8 | Acceptée |
