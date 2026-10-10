# Décisions à prendre

Questions ouvertes par la [revue](revue-etapes-0-1.md) et par les plans des
étapes 2 à 8. Pour chacune, les plans appliquent déjà la recommandation (★).
Une réponse différente ne change que les sections indiquées.

Quand une décision est tranchée, elle devient une note numérotée dans
[`decisions/`](decisions/README.md) et sa ligne est retirée d'ici.

**Pour un agent** : ne tranche jamais une ligne de ce fichier toi-même. Si ta
tâche dépend d'une décision encore ouverte, applique la recommandation et
signale-le dans ton compte rendu.

## Organisation du travail

| # | Question | ★ Recommandation | Alternative | Impacte |
| --- | --- | --- | --- | --- |
| D1 | Où vit la documentation | Dans le dépôt (`docs/`, `CLAUDE.md`) | Fichiers à part | Tout |
| D2 | Profondeur des plans 2 à 8 | Même trame partout ; très détaillée pour 2 et 3, plus légère pour 4 à 8 avec revue obligatoire au démarrage | Tout au niveau de détail de l'étape 1 dès maintenant | `etape-4.md` à `etape-8.md` |
| D3 | Format pour l'orchestrateur d'agents | `CLAUDE.md` racine + un par crate ou appli ; tâches numérotées et autonomes dans `etape-N.md` | Format propre à l'orchestrateur (`AGENTS.md`, fichiers de tâches) | `CLAUDE.md`, plans |
| D4 | Ordre des jalons | Inchangé (2 chiffrement, 3 Android, 4 desktop, 5 fichiers, 6 presse-papiers, 7 extension, 8 confort) | Fichiers (5) avant desktop (4) | Feuille de route |

## Étape 2 : chiffrement et appairage

| # | Question | ★ Recommandation | Alternative | Impacte |
| --- | --- | --- | --- | --- |
| D5 | Forme du code d'appairage | Code de 46 caractères (jeton de 80 bits + secret de 128 bits), copié-collé ; le QR de l'étape 3 porte le même contenu | Code court à taper (`7-guitare-orbite`) avec un échange SPAKE2, comme magic-wormhole | `etape-2.md` T4 à T6 |
| D6 | Bibliothèque crypto | RustCrypto (`chacha20poly1305`, `x25519-dalek`, `hkdf`, `sha2`) | libsodium (`libsodium-sys`) | `etape-2.md`, `etape-5.md` |
| D7 | Clé de groupe ou chiffrement par destinataire | Clé de groupe, comme prévu | Clé de contenu scellée pour chaque destinataire | `etape-2.md` |
| D8 | Rotation de la clé de groupe | Dès l'étape 2 (`ot group rotate --exclude <appareil>`) | À l'étape 8 avec la révocation | `etape-2.md` T8, `etape-8.md` |
| D9 | Où vit le code crypto | Nouvelle crate `ot-crypto` | Module de `ot-core` | `etape-2.md` T1 |
| D10 | Qui refuse le contenu en clair | Les clients (le relais ne lit jamais `payload`) | Le relais, sur `payload.enc` | `etape-2.md` T9 |
| D11 | Messages en clair restés sur le relais à la bascule | Purge complète des messages au passage à l'étape 2 | Attendre l'expiration (7 jours) | `etape-2.md` T10 |
| D12 | CLI sur une machine qui a aussi l'appli | La CLI est un appareil distinct (`ot --home`) | Session « envoi seul » (`receive: false` à l'`auth`, note 0009) ; ou passage par la socket locale de l'appli, construite de toute façon pour l'extension (étape 4) | `etape-4.md` |

## Applis

| # | Question | ★ Recommandation | Alternative | Impacte |
| --- | --- | --- | --- | --- |
| D13 | `ws://` ou `wss://` pour Android | `ws://` sur Tailscale : la connexion part du code Rust, que la politique « pas de clair » d'Android (piles Java) ne concerne pas ; le clair reste interdit au code Java | `wss` via `tailscale cert` dès l'étape 3 (le nom de machine part dans les journaux publics de certificats) | `etape-3.md` |
| D14 | Type du service Android | `specialUse` (appli installée hors Play Store) | `dataSync` (limité à 6 h par jour à partir d'Android 15) ; `remoteMessaging` | `etape-3.md` |
| D15 | Tuile « envoyer le presse-papiers » | Avancée à l'étape 3 | Étape 6 comme prévu | `etape-3.md`, `etape-6.md` |
| D16 | Client Ubuntu | L'appli Tauri compile pour Linux, sans presse-papiers live sous Wayland | Pas de client Ubuntu | `etape-4.md`, `etape-6.md` |
| D17 | Organisation du monorepo | `crates/` (Rust), `apps/android`, `apps/desktop`, `apps/extension`, un workflow CI par dossier | Un dépôt par appli | `etape-3.md`, `etape-4.md`, `etape-7.md` |
| D18 | Lecture du QR sur Android | zxing-cpp (Apache 2.0) avec CameraX, saisie du code en secours | ML Kit (dépendance Google) | `etape-3.md` T9 |
| D19 | Interface de l'appli desktop | Angular, déjà maîtrisé | Svelte ou TypeScript sans framework, plus léger | `etape-4.md` |

## Étapes 5 à 8

| # | Question | ★ Recommandation | Alternative | Impacte |
| --- | --- | --- | --- | --- |
| D20 | Téléchargement des fichiers reçus | Automatique sous 20 Mo (et sur Wi-Fi seulement pour le téléphone), à la demande au-delà ; blobs gardés 48 h sur le relais | Toujours automatique ; ou toujours à la demande | `etape-5.md` |
| D21 | Navigateurs visés par l'extension | Chrome (mode développeur) et Firefox (signature Mozilla « unlisted ») | Un seul navigateur ; Firefox Developer Edition sans signature | `etape-7.md` |
| D22 | Base de l'historique local | SQLCipher (base entière chiffrée, recherche FTS5) | SQLite simple avec contenus chiffrés un par un (pas de recherche plein texte) | `etape-8.md` 8a |
| D23 | Qui peut révoquer un appareil sans SSH | Tout membre, journalisé et notifié aux autres | Seulement des appareils marqués « administrateurs » ; ou confirmation par un second appareil | `etape-8.md` 8c |
