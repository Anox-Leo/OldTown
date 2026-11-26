# apps/extension

Extension Chrome (étape 4, notes 0017 et 0023) : Manifest V3, autonome
(aucune appli à côté). Le service worker tient la connexion au relais ; la
crypto, le roster et l'appairage restent en Rust (`crates/ot-wasm`, compilé
en WebAssembly) ; le TypeScript ne fait que le transport, le stockage et
l'interface.

- **Arborescence** : `src/background/` (service worker : `main.ts` branche
  les API Chrome, `engine.ts` le cœur sans API Chrome, `client.ts` le
  protocole, `people.ts` les personnes (annuaire, journal, invitation,
  administration ; étape 10), `secrets.ts` le stockage chiffré, `frames.ts` les trames,
  `transfer.ts` les fichiers, sans API Chrome), `src/offscreen/` (copie
  dans le presse-papiers), `src/pages/` (`send.html` : permissions sur un
  clic, et choix de fichiers en repli ; TypeScript simple, esbuild),
  `src/app/` (popup, réglages et administration, Angular), `src/shared/` (`api.ts` messages
  interface ↔ service worker, `files.ts` adresses des fichiers reçus,
  `opfs.ts` fichiers, `origins.ts` permissions d'hôte), `src/wasm/` (liaisons générées par
  `npm run wasm`, **jamais versionnées**), `test/` (tests du service worker sous Node),
  `manifest.json`, `scripts/` (build).
- **Build** : `npm run build` = WebAssembly (`cargo build --profile wasm`
  puis `wasm-bindgen --target web`), `ng build` (popup `index.html`,
  réglages `options.html`, administration `admin.html`), puis esbuild pour `background.js`,
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
  `chrome-extension://<id>/` de l'extension. Pas de script de contenu
  déclaré, pas d'`externally_connectable` ; seule `fillOtp` est injectée
  (codes, note 0045).
- **Service worker** : écouteurs Chrome enregistrés au premier niveau de
  `main.ts` ; pas d'`await` au premier niveau (interdit dans un service
  worker). `ping` toutes les 20 s, alarme de 30 s pour le réveiller
  (`minimum_chrome_version` 120). Ne rien garder d'important en mémoire
  seulement : tout état est relu au réveil.
- **Secrets** : uniquement dans le `Vault` du WebAssembly, et au repos dans
  `chrome.storage.local` sous `secret:<nom>`, chiffrés en AES-256-GCM par une
  clé WebCrypto non exportable (IndexedDB), nom en donnée associée. L'état
  du coffre est rangé (`persistVault`) avant l'acquittement du message qui
  l'a changé. L'historique (contenu en clair), les envois de fichiers en
  cours (`outgoing.json`, des noms) et la dernière ouverture des
  conversations des personnes (`opened.json`, non-lus), ainsi que le
  compte des éléments reçus perdus (`lost.json`), sont rangés de la même
  façon.
  En clair dans `chrome.storage.local` : seulement `device` (id, nom, URL),
  `recipient`, `outbox` (enveloppes chiffrées) et `seen` (`msg_id`).
  Dans le `localStorage` des pages : seulement `theme` (choix d'affichage,
  `src/shared/theme.ts`, lu avant le premier rendu).
- **Popup = conversations** (notes 0030 et 0031) : accueil (`home.ts`) en
  liste, un appareil par ligne (plus récent d'abord ; plus de « Tous mes
  appareils », note 0078 : un ancien envoi à tous n'entre dans aucune
  conversation, et `send` refuse un destinataire vide), avatar (type déclaré au relais, note 0068 : l'extension
  déclare `browser` ; sans type, pictogramme neutre ; couleur tirée de l'id :
  `devices.ts`) et dernier message ; un clic ouvre la conversation
  (`chat.ts`, canal ouvert = `recipient`) : flèche de retour, présence
  d'après `devices` (toutes les 15 s), bulles envoyées à droite et reçues à
  gauche, zone d'envoi en bas. Rangement des éléments : `channelOf`.
  Popup ouverte pour un élément reçu (d'office ou par sa notification) :
  sa conversation directement (push `open`, note 0032). Ouvrir une
  conversation (`mark-read`) efface les notifications de ses éléments
  (`notificationsToClear`), sur cet appareil seulement.
  L'historique (100 éléments) garde aussi les envois, « en cours » jusqu'à
  `accepted` ; les destinataires d'un reçu viennent de l'enveloppe.
- **Personnes** (étape 10, `people.ts`, déroulés de `client.rs` refaits
  au-dessus de `ot-wasm`) : annuaire relu à chaque connexion, à chaque
  `directory_changed` (2 s au plus) et après tout changement fait d'ici
  (le relais ne prévient pas la session qui change) ; relu avant un envoi
  s'il date de plus de 60 s. Demandes au relais par `RelayClient.call`,
  refus rangés par référence ou, sans référence, à la plus ancienne
  demande qui n'en a pas (comme `client.rs`). Racine de l'annuaire
  adoptée d'office si un autre appareil du groupe l'a publiée, publiée
  seulement sur un clic (« Publier l'annuaire », réglages). Barre latérale
  (`rail.ts`) : moi, puis les personnes par activité ; une personne montre
  elle (ses appareils par défaut) puis ses appareils joignables. Canaux
  `user:<id>` et `user:<id>/<appareil>` (`channelOf`, `inChannel` dans
  `shared/api.ts`), destinataires des envois, menus et raccourcis compris.
  Appareils `cli` hors des conversations (« Ligne de commande » repliée
  dans les réglages). Page `admin.html` (admin seulement) : inviter,
  personnes, grille du réseau (une case par paire, les deux sens), services.
- **Élément reçu perdu** (note 0067) : il est acquitté quoi qu'il arrive,
  comme dans `ot-core`. Un échec de son traitement (stockage) est réessayé
  trois fois, puis compté (`lostItems`) et signalé dans la popup jusqu'à
  « OK ».
- **Élément reçu** (note 0024, `deliver`) : popup ouverte → liste seulement ;
  navigateur au premier plan et élément récent → lien ouvert dans un onglet,
  texte → popup (`action.openPopup`) ; sinon notification. Un fichier reçu
  que le navigateur affiche s'ouvre dans un onglet pour l'admin seulement ;
  pour un non-admin, la popup s'ouvre sur sa conversation. Raccourcis
  `send-page` (`Alt+Shift+S`), `send-clipboard` (`Alt+Shift+V`, lecture
  par le document offscreen), vers la dernière conversation ouverte, sinon
  mon appareil principal, ou mon autre appareil le plus récemment actif si
  c'est celui-ci (`shortcutChannel`) et `fill-otp` (`Alt+Shift+O`). Navigateur cible : Brave (Chromium).
- **Codes à usage unique** (note 0045, `src/shared/otp.ts`) : élément
  `kind: 'otp'` (`body` = le code, `otp` = affichage, expéditeur, DMARC,
  expiration). Code valable reçu (`deliverOtp`), d'un de mes appareils ou
  d'un service seulement (règle O2, `autoFillAllowed`) : copié ; rempli d'office
  (`chrome.scripting`, cadre principal) seulement si expéditeur authentifié,
  onglet actif en HTTPS sur son site (`sameSite` : même domaine
  enregistrable, `tldts`) et accès aux pages HTTPS accordé (`OTP_ORIGINS`) ;
  puis notification. Sinon raccourci `fill-otp` (`Alt+Shift+O`,
  `activeTab`). Accès : bouton de la notification (`send.html?grant=otp`)
  ou section des réglages. Code expiré : historique seulement.
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
  `clipboardRead` (raccourci « presse-papiers » seulement),
  `contextMenus`, `notifications`, `offscreen`, `scripting` (remplissage
  des codes, note 0045), `storage` ;
  `optional_host_permissions` `<all_urls>`, demandée origine par origine,
  ou `https://*/*` pour les codes (note 0045).
  CSP : `script-src 'self' 'wasm-unsafe-eval'`. Toute permission nouvelle se justifie dans une note.
- **Identifiant fixe** : `key` du manifeste (clé publique) ; la clé privée,
  qui ne sert qu'à empaqueter, est dans `private/extension-key.pem`.
- **Tests** : `npm test` = WebAssembly, puis Vitest sous Node
  (`vitest.worker.config.mts` : golden, vecteurs, stockage, et
  `test/otp.test.ts` sous jsdom, `test/relay.test.ts` et
  `test/design.test.ts` (couleurs, signe, icônes),
  `test/files.test.ts` et `test/people.test.ts` (étape 10 : admin,
  invitée) contre un vrai `ot-server`
  avec la CLI `ot` comme parrain : binaires de `target/debug`, ou
  `OT_BIN_DIR`), puis `ng test`
  (interface). `npm run lint` = ESLint, Prettier et `tsc` du service worker
  et des tests.
- **Pas de commentaires** (note 0066) : le pourquoi va dans une note de
  `docs/decisions/`, le quoi dans les noms et les tests, un raccourci dans
  une issue. La règle `local/no-comments` (`eslint-rules/no-comments.js`)
  ne laisse passer que les directives (`@ts-check`, `eslint-disable`,
  `@vitest-environment`…). Cliquet : `eslint-suppressions.json` liste ce
  qui reste ; un commentaire nouveau fait échouer `npm run lint`, et un
  commentaire retiré se reporte avec `npm run lint:prune`. Pour trier ce
  qui reste : `/delete-comment`.
- UI en français. **Identité visuelle** (note 0059) : couleurs, arrondis,
  tailles et polices viennent de `src/tokens.css` et `src/shared/tokens.ts`
  (avatars, badge), **générés** depuis `design/tokens.json` par
  `tools/design-tokens/gen.mjs` : jamais modifiés à la main, et aucune
  couleur en dur ailleurs dans `src/` hors `qr.ts` (QR noir sur blanc ;
  `test/design.test.ts` le vérifie). Signe et logotype : `src/app/brand.ts`
  (`app-mark`, `app-logo`), tracés repris de `branding/`. Thème du système, ou
  « nuit » ou « parchemin » choisi dans les réglages (`data-theme` sur
  `<html>`, lu par `tokens.css`). Icônes
  `public/icons/` exportées depuis `branding/` (16 et 32 en taille
  optique). DM Sans, OFL, licence dans `third_party/`.
