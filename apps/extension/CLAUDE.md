# apps/extension

Extension Chrome (étape 4, notes 0017 et 0023) : Manifest V3, autonome
(aucune appli à côté). Le service worker tient la connexion au relais ; la
crypto, le roster et l'appairage restent en Rust (`crates/ot-wasm`, compilé
en WebAssembly) ; le TypeScript ne fait que le transport, le stockage et
l'interface.

- **Arborescence** : `src/background/` (service worker : `main.ts` branche
  les API Chrome, `engine.ts` le cœur sans API Chrome, `client.ts` le
  protocole, `secrets.ts` le stockage chiffré, `frames.ts` les trames),
  `src/offscreen/` (copie dans le presse-papiers), `src/app/` (popup et
  réglages, Angular), `src/shared/api.ts` (messages interface ↔ service
  worker), `src/wasm/` (liaisons générées par `npm run wasm`, **jamais
  versionnées**), `test/` (tests du service worker sous Node),
  `manifest.json`, `scripts/` (build).
- **Build** : `npm run build` = WebAssembly (`cargo build --profile wasm`
  puis `wasm-bindgen --target web`), `ng build` (popup `index.html`,
  réglages `options.html`), puis esbuild pour `background.js` et
  `offscreen.js`. Résultat non empaqueté : `dist/extension`. Prérequis :
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
  l'a changé. L'historique (contenu en clair) est rangé de la même façon.
  En clair dans `chrome.storage.local` : seulement `device` (id, nom, URL),
  `recipient`, `outbox` (enveloppes chiffrées) et `seen` (`msg_id`).
- **Jamais de contenu, de clé, de code d'appairage ni de texte de QR dans
  `console`** : `log()` de `main.ts` n'écrit que des messages d'erreur.
- **Format** : les trames reçues sont validées par `ot-proto` (WebAssembly,
  `decodeFrame`) ; les trames écrites en TypeScript sont comparées aux golden
  (`test/frames.test.ts`) ; les vecteurs de `ot-crypto` sont rejoués
  (`test/vectors.test.ts`).
- **Permissions** (`manifest.json`) : `activeTab`, `alarms`,
  `clipboardRead` (bouton « presse-papiers » seulement), `contextMenus`,
  `notifications`, `offscreen`, `storage`. CSP : `script-src 'self'
  'wasm-unsafe-eval'`. Toute permission nouvelle se justifie dans une note.
- **Identifiant fixe** : `key` du manifeste (clé publique) ; la clé privée,
  qui ne sert qu'à empaqueter, est dans `private/extension-key.pem`.
- **Tests** : `npm test` = WebAssembly, puis Vitest sous Node
  (`vitest.worker.config.mts` : golden, vecteurs, stockage, et
  `test/relay.test.ts` contre un vrai `ot-server` avec la CLI `ot` comme
  parrain : binaires de `target/debug`, ou `OT_BIN_DIR`), puis `ng test`
  (interface). `npm run lint` = ESLint, Prettier et `tsc` du service worker
  et des tests.
- UI en français, thème repris de l'appli Android (DM Sans, OFL, licence
  dans `third_party/`).
