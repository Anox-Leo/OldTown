# Plan — Étape 4 : extension Chrome

L'étape 4 livre le client des ordinateurs : une extension Chrome autonome, qui
tient elle-même la connexion au relais. Envoyer la page, un lien ou une
sélection d'un clic droit ; recevoir en notification ; une popup avec les
derniers éléments et une zone d'envoi. Aucune appli à installer à côté
(note 0017).

**État (12 octobre 2026)** : code livré (T1 à T8b) ; écarts et précisions
dans la note [0023](decisions/0023-extension-wasm-et-transport-ts.md) (cœur
de groupe en WebAssembly, transport en TypeScript, `ping`/`pong`). Validé en
local : tests Rust, `npm run lint && npm test` (dont un vrai `ot-server`
avec la CLI comme parrain), `./gradlew lint test assembleDebug`, et un essai
de l'extension construite dans Chrome for Testing 155 contre un relais local
(QR inversé, envoi, réception avec et sans popup ouverte, stockage). Reste
T9 : l'essai réel sur le Mac et le PC Windows
([actions manuelles](actions-manuelles.md)).

Références : jalon « 4. Extension Chrome » de la [conception](design.md),
notes 0009 (un seul propriétaire de la connexion), 0013 (QR inversé), 0015
(`wss`) et 0017 (extension autonome).

**Niveau de détail** : plan de cadrage (note 0012). Au démarrage de l'étape,
un agent relit ce plan contre l'état du code et des étapes 2-3, vérifie les
règles Manifest V3 du moment (durée de vie du service worker, documents
offscreen, WebAssembly), complète les tâches et note les écarts dans
`decisions/` avant de coder.

**Prérequis** : étapes 2 et 3 terminées (chiffrement, roster, appairage par QR
inversé, relais en `wss`).

## Objectif et périmètre

**Résultat attendu** : sur le PC Windows, clic droit sur un lien → « Envoyer à
OldTown » → « Téléphone » ; la notification arrive sur le téléphone. Depuis le
téléphone, Partager → OldTown → « Mac » : une notification Chrome s'affiche
sur le Mac, un clic ouvre le lien dans un onglet.

**Inclus**

- Extension Manifest V3 pour Chrome dans `apps/extension`, interface en
  Angular (popup, page de réglages).
- Crate `ot-wasm` : `ot-crypto` compilée en WebAssembly.
- Client du protocole en TypeScript dans le service worker.
- Clés dans le profil Chrome, chiffrées par une clé WebCrypto non exportable.
- Menus contextuels (page, lien, sélection), raccourci clavier.
- Popup : derniers éléments reçus (50), zone d'envoi, bouton « envoyer le
  presse-papiers », choix du destinataire, état de connexion.
- Notifications : clic → ouvrir le lien ; bouton « Copier ».
- Appairage : la popup affiche un QR inversé que le téléphone scanne, ou
  accepte un code collé ; écran « Groupe » (membres, empreintes, rotation).

**Exclus** : fichiers et images (étape 5), presse-papiers automatique
(jamais, note 0019), historique (étape 8), autres navigateurs (note 0017),
publication sur le Chrome Web Store.

## Architecture

```text
┌──────────────────────── Extension Chrome ─────────────────────────┐
│ Popup, menus, raccourci ──▶ service worker ──▶ WebSocket wss ──────┼──▶ relais
│                               ├─ ot-wasm (chiffrement, signature)  │
│                               ├─ clés : profil Chrome, chiffrées   │
│                               └─ document offscreen (presse-papiers)
└────────────────────────────────────────────────────────────────────┘
```

- **Un seul propriétaire de la connexion** : le service worker. La popup, les
  menus et le raccourci lui parlent par messages ; ils n'ouvrent jamais de
  connexion (note 0009).
- **Durée de vie** : Chrome arrête un service worker inactif après 30 s ; une
  WebSocket le garde en vie si un message passe au moins toutes les 30 s.
  `ot-proto` n'a pas de trame de maintien : en ajouter une (golden et note),
  envoyée toutes les 20 s. Chrome fermé : rien n'est reçu, le relais garde les
  messages en attente.
- **Crypto en Rust, protocole en TypeScript** : `ot-wasm` expose `ot-crypto`
  (sceller, ouvrir, signer, vérifier, données associées, code d'appairage, QR
  inversé). Le client TypeScript reprend la logique de `ot-core` (connexion,
  authentification, file d'envoi, acquittements, dédoublonnage, roster,
  quarantaine des époques inconnues). Ses tests rejouent les golden de
  `ot-proto` et les vecteurs de `ot-crypto`.
- **Clés** : générées dans le WebAssembly ; rangées dans
  `chrome.storage.local`, chiffrées en AES-GCM par une clé WebCrypto non
  exportable gardée dans IndexedDB. Les secrets restent dans la mémoire du
  WebAssembly (`zeroize`) ; le JavaScript ne manipule que des poignées.
- **Presse-papiers** : écriture par un document offscreen (raison
  `CLIPBOARD`) ; lecture seulement au clic sur « envoyer le presse-papiers ».
- **Permissions minimales** : `contextMenus`, `notifications`, `storage`,
  `offscreen`, `activeTab`. Aucun script injecté dans les pages. CSP stricte,
  avec seulement `wasm-unsafe-eval` pour le WebAssembly.
- **Distribution** : extension non empaquetée en mode développeur,
  identifiant fixé par la clé du manifeste (clé privée dans `private/`).
- **Réseau** : `wss` vers le relais (note 0015) ; l'URL du relais est un
  réglage. Message d'erreur qui pointe vers Tailscale quand le relais est
  injoignable.

## Découpage en tâches

- [x] **T1. Crate `ot-wasm`.** `wasm-bindgen` au-dessus de `ot-core` sans
  réseau (fonction `net`, note 0023) et de `ot-crypto`, build
  `wasm32-unknown-unknown`, tests ; vérifier que `ot-crypto` compile sans
  `getrandom` natif (fonction `js`). Licences dans `deny.toml`.
- [x] **T2. Squelette et CI.** `apps/extension/` (Angular, MV3),
  `apps/extension/CLAUDE.md`, `.github/workflows/extension.yml` filtré sur
  `apps/extension/**` et `crates/**` : build du WebAssembly, lint, tests,
  archive. Fini quand : l'extension se charge et affiche la version de
  `ot-wasm`.
- [x] **T3. Trame de maintien.** Dans `ot-proto` et `ot-server`, golden, note
  de décision ; `ot-core` l'utilise aussi. *(`ping`/`pong` ; `ot-core` décode
  `pong` sans envoyer de `ping`, note 0023.)*
- [x] **T4. Client du protocole en TypeScript.** Connexion, authentification,
  envoi, réception, acquittements, dédoublonnage persisté, reconnexion avec
  backoff. Tests : golden de `ot-proto`, vecteurs de `ot-crypto`, et en CI un
  test contre un vrai `ot-server`.
- [x] **T5. Stockage des clés.** Clé WebCrypto non exportable, secrets
  chiffrés, effacement. Tests : aller-retour, aucune clé en clair dans
  `chrome.storage`.
- [x] **T6. Appairage et groupe.** QR inversé dans la popup (le téléphone
  scanne), code collé, écran « Groupe ».
- [x] **T7. Envoi.** Menus contextuels, raccourci, zone d'envoi, bouton
  « envoyer le presse-papiers », choix du destinataire.
- [x] **T8. Réception.** Notifications, ouverture du lien, « Copier » par le
  document offscreen, 50 derniers éléments. Pas de notification quand la
  popup est ouverte : l'élément apparaît seulement dans la liste.
- [x] **T8b. Android : pas de notification appli ouverte.** Demande venue
  après l'étape 3 : quand l'appli est au premier plan, un message reçu
  s'ajoute à l'historique sans notification (la notification permanente du
  service ne change pas). Tests : test unitaire de la règle « au premier plan
  → pas de notification ».
- [ ] **T9. Essai réel et documentation.** Windows et Mac, README
  (installation en mode développeur), état de l'étape, notes de décision.
  *(Documentation faite ; essai réel à dérouler, résultats à reporter dans
  le tableau.)*

Chaque tâche finit par les validations Rust de `CLAUDE.md`, plus
`npm run lint && npm test` dans `apps/extension`.

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Clic droit sur un lien → téléphone | Notification sur le téléphone | Essai réel |
| Téléphone → Mac, lien | Notification Chrome ; clic ouvre le lien | Essai réel |
| Message reçu, popup ou appli Android ouverte | Pas de notification ; l'élément apparaît dans la liste | Essai réel |
| Sélection de texte → Windows | Arrive sur le PC | Essai réel |
| Appairage | Le téléphone scanne le QR de la popup ; l'extension rejoint le groupe | Essai réel |
| Chrome ouvert sans activité pendant une heure | Toujours connecté, messages reçus | Essai réel |
| Chrome fermé puis rouvert | Les messages en attente arrivent | Essai réel |
| Compatibilité | Golden et vecteurs passent côté TypeScript | CI |
| Clés | Jamais en clair dans le profil Chrome ni dans les logs | Revue + test |
| Une page web ou une autre extension | Ne peut pas parler au service worker | Revue du manifeste |

## Risques

- **Divergence entre le client TypeScript et `ot-core`** : golden et vecteurs
  partagés, test contre le vrai relais en CI.
- **Évolution de Manifest V3** (durée de vie du service worker, offscreen) :
  vérifier au démarrage et noter les écarts.
- **Ordinateur professionnel** : politique qui bloque le mode développeur ou
  Tailscale (voir les conséquences de la note 0017).
