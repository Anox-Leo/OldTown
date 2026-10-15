# apps/extension

Extension Chrome (étape 4, notes 0017 et 0023) : Manifest V3, autonome
(aucune appli à côté). Le service worker tient la connexion au relais ; la
crypto, le roster et l'appairage restent en Rust (`crates/ot-wasm`, compilé
en WebAssembly) ; le TypeScript ne fait que le transport, le stockage et
l'interface.

- **Arborescence** : `src/background/` (service worker : `main.ts` branche
  les API Chrome, `engine.ts` le cœur sans API Chrome, `client.ts` le
  protocole, `secrets.ts` le stockage chiffré, `frames.ts` les trames,
  `transfer.ts` les fichiers, sans API Chrome), `src/offscreen/` (copie
  dans le presse-papiers), `src/pages/` (`send.html` : permissions sur un
  clic, et choix de fichiers en repli ; TypeScript simple, esbuild),
  `src/app/` (popup et réglages, Angular), `src/shared/` (`api.ts` messages
  interface ↔ service worker, `files.ts` adresses des fichiers reçus,
  `opfs.ts` fichiers, `origins.ts` permissions d'hôte), `src/wasm/` (liaisons générées par
  `npm run wasm`, **jamais versionnées**), `test/` (tests du service worker sous Node),
  `manifest.json`, `scripts/` (build).
- **Build** : `npm run build` = WebAssembly (`cargo build --profile wasm`
  puis `wasm-bindgen --target web`), `ng build` (popup `index.html`,
  réglages `options.html`), puis esbuild pour `background.js`,
  `offscreen.js` et `send.js`. Résultat non empaqueté : `dist/extension`.
  `npm run deploy -- <dossier>` (`scripts/deploy.sh`) construit et copie
  dans le dossier chargé par le navigateur (jamais de chemin personnel dans
  le dépôt : dossier en argument ou `OT_EXT_DIR`). Prérequis :
  cible `wasm32-unknown-unknown`, `wasm-bindgen-cli` de la version du
  `Cargo.lock`, Node 24.
- **Un seul propriétaire de la connexion** (note 0009) : le service worker.
  La popup et les réglages passent par `chrome.runtime.sendMessage` et un
  port (`POPUP_PORT`, état poussé ; pas de notification tant qu'il est
  ouvert). Tout message est refusé s'il ne vient pas d'une page
  `chrome-extension://<id>/` de l'extension. Pas de script de contenu, pas
  d'`externally_connectable`.
- **Service worker** : écouteurs Chrome enregistrés au premier niveau de
  `main.ts` ; pas d'`await` au premier niveau (interdit dans un service
  worker). `ping` toutes les 20 s, alarme de 30 s pour le réveiller
  (`minimum_chrome_version` 120). Ne rien garder d'important en mémoire
  seulement : tout état est relu au réveil.
- **Secrets** : uniquement dans le `Vault` du WebAssembly, et au repos dans
  `chrome.storage.local` sous `secret:<nom>`, chiffrés en AES-256-GCM par une
  clé WebCrypto non exportable (IndexedDB), nom en donnée associée. L'état
  du coffre est rangé (`persistVault`) avant l'acquittement du message qui
  l'a changé. L'historique (contenu en clair) et les envois de fichiers en
  cours (`outgoing.json`, des noms) sont rangés de la même façon.
  En clair dans `chrome.storage.local` : seulement `device` (id, nom, URL),
  `recipient`, `outbox` (enveloppes chiffrées) et `seen` (`msg_id`).
- **Popup = conversations** (notes 0030 et 0031) : accueil (`home.ts`) en
  liste, « Tous mes appareils » puis un appareil par ligne (plus récent
  d'abord), avatar (type deviné au nom, couleur tirée de l'id :
  `devices.ts`) et dernier message ; un clic ouvre la conversation
  (`chat.ts`, canal ouvert = `recipient`) : flèche de retour, présence
  d'après `devices` (toutes les 15 s), bulles envoyées à droite et reçues à
  gauche, zone d'envoi en bas. Rangement des éléments : `channelOf`.
  L'historique (100 éléments) garde aussi les envois, « en cours » jusqu'à
  `accepted` ; les destinataires d'un reçu viennent de l'enveloppe.
- **Élément reçu** (note 0024, `deliver`) : popup ouverte → liste seulement ;
  navigateur au premier plan et élément récent → lien ouvert dans un onglet,
  texte → popup (`action.openPopup`) ; sinon notification. Raccourcis
  `send-page` (`Alt+Shift+S`) et `send-clipboard` (`Alt+Shift+V`, lecture
  par le document offscreen). Navigateur cible : Brave (Chromium).
- **Fichiers envoyés** (étape 5, notes 0026, 0027, 0029) : le trombone de
  la popup ouvre le sélecteur de l'OS ; la popup copie le fichier dans
  l'OPFS (`outgoing/<id>`), puis le service worker l'envoie.
- **Fichiers reçus** (note 0029) : le service worker répond lui-même aux
  adresses `chrome-extension://<id>/f/<blob_id>/<nom>` (événement `fetch`,
  `fileResponse` du moteur), en flux, depuis une copie **chiffrée** dans
  l'OPFS (`cache/<blob_id>/<n>`) ou le relais ; `inline` pour un type que le
  navigateur affiche (`isViewable` : ni SVG ni HTML), `attachment` sinon ou
  avec `?dl=1` ; `Range` ; `Content-Security-Policy: sandbox` (sauf PDF) et
  `nosniff`. Petits fichiers copiés d'office, SHA-256 vérifié, puis `done`.
  La clé du fichier reste dans le coffre tant que la copie existe
  (`pruneCache`). Rien n'arrive sur le disque sans geste. Les requêtes HTTP
  vers le relais demandent la permission de son origine
  (`optional_host_permissions`) ; une image qu'un site ne laisse pas lire,
  celle de son seul domaine. Une demande de permission se fait toujours
  depuis une page sur un clic (`send.html`), jamais depuis la popup, qui
  peut se fermer.
- **Jamais de contenu, de clé, de code d'appairage ni de texte de QR dans
  `console`** : `log()` de `main.ts` n'écrit que des messages d'erreur.
- **Format** : les trames reçues sont validées par `ot-proto` (WebAssembly,
  `decodeFrame`) ; les trames écrites en TypeScript sont comparées aux golden
  (`test/frames.test.ts`) ; les vecteurs de `ot-crypto` sont rejoués
  (`test/vectors.test.ts`).
- **Permissions** (`manifest.json`) : `activeTab`, `alarms`,
  `clipboardRead` (bouton et raccourci « presse-papiers » seulement),
  `contextMenus`, `notifications`, `offscreen`, `storage` ;
  `optional_host_permissions` `<all_urls>`, demandée origine par origine.
  CSP : `script-src 'self' 'wasm-unsafe-eval'`. Toute permission nouvelle se justifie dans une note.
- **Identifiant fixe** : `key` du manifeste (clé publique) ; la clé privée,
  qui ne sert qu'à empaqueter, est dans `private/extension-key.pem`.
- **Tests** : `npm test` = WebAssembly, puis Vitest sous Node
  (`vitest.worker.config.mts` : golden, vecteurs, stockage, et
  `test/relay.test.ts` et `test/files.test.ts` contre un vrai `ot-server`
  avec la CLI `ot` comme parrain : binaires de `target/debug`, ou
  `OT_BIN_DIR`), puis `ng test`
  (interface). `npm run lint` = ESLint, Prettier et `tsc` du service worker
  et des tests.
- UI en français, thème repris de l'appli Android (DM Sans, OFL, licence
  dans `third_party/`).
