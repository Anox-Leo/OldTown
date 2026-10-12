# 0023 — Étape 4 : cœur de groupe en WebAssembly, transport en TypeScript

**Statut : acceptée** (12 octobre 2026). Relecture du plan de l'étape 4
contre le code, avant T1 ; écarts au plan et à la note 0017.

**Contexte.** Le plan prévoit que `ot-wasm` n'expose que `ot-crypto` et que
le client TypeScript reprenne toute la logique de `ot-core` : connexion,
file, acquittements, mais aussi roster, époques, quarantaine et appairage.
Cette seconde moitié représente environ 1 700 lignes de Rust (`secure.rs`,
`roster.rs`, `keyring.rs`, `dedup.rs`) sans aucune entrée-sortie, et la
réécrire en TypeScript aurait fait passer les clés de groupe dans le
JavaScript, ce que le plan interdit par ailleurs (« le JavaScript ne
manipule que des poignées »). Les règles de Manifest V3 ont aussi été
vérifiées (documentation Chrome du 12 octobre 2026).

**Décision.**

- **Partage Rust / TypeScript.** `ot-core` reçoit une fonction `net`, active
  par défaut, qui porte la partie réseau (`Client`, `redeem_pairing`, TLS ;
  dépendances tokio, tokio-tungstenite, rustls). Sans elle, `ot-core` se
  compile en WebAssembly : identité, `Secure` (sceller, ouvrir, roster,
  quarantaine, appairage, rotation), `Keyring`, `Dedup`, formats des QR.
  `ot-cli` et `ot-ffi` gardent la fonction par défaut : leur code est
  inchangé. Les types communs (`Event`, `ClientError`, `Received`,
  `Accepted`, `PairingInvitation`) passent dans un module sans réseau,
  réexportés sous les mêmes noms. `Secure` et ses effets deviennent publics
  (`ot_core::Secure`, `ot_core::Effect`) pour `ot-wasm`.
- **`ot-wasm`** expose ce cœur par `wasm-bindgen` : un coffre en mémoire
  (`Vault`) qui sert de `SecretStore`, l'identité, les signatures
  d'authentification et d'appairage, le cœur de groupe (`Core`), le
  décodage des trames (`ot-proto`) et les QR. Les enveloppes circulent en
  JSON (chiffrées) ; les clés ne sortent pas du WebAssembly, sauf pour être
  enregistrées (point suivant).
- **Le TypeScript** ne garde que le transport : WebSocket, authentification,
  file d'envoi persistée, acquittements, dédoublonnage persisté,
  reconnexion (backoff plafonné à 25 s, sous la durée de vie du service
  worker), stockage, interface. Les trames entrantes sont validées par
  `ot-proto` en WebAssembly (même code que le relais et les autres
  clients) ; les trames qui portent une signature (`auth`, `pair_redeem`)
  sont produites par le WebAssembly ; les autres (`ping`, `ack`,
  `list_devices`, `send`) sont écrites en TypeScript et comparées aux golden.
- **Stockage.** Les API du navigateur sont asynchrones, le `Keyring` de
  `ot-core` est synchrone : le `Vault` garde les valeurs en mémoire et note ce
  qui change ; après chaque opération, le service worker chiffre les
  changements (AES-256-GCM, clé WebCrypto non exportable dans IndexedDB, nom
  en donnée associée, même principe que le Keystore Android) et les range
  dans `chrome.storage.local`, **avant** d'acquitter le message qui les a
  causés (livraison au moins une fois, comme `ot-core`). Limite assumée : les
  octets en clair passent un instant par le JavaScript pour aller à
  WebCrypto, qui n'accepte que des tampons JavaScript. Le service worker et
  le WebAssembly partagent de toute façon le même processus.
- **Appairage en attente persisté** (réponse 2a de la relecture) : l'offre du
  QR inversé (son secret `S`) et le code collé sont rangés chiffrés
  (`pending_offer.json`, `pending_join.json`, comme `pending_join.json` sur
  Android) et effacés dès que le groupe est adopté, ou à l'expiration (10
  minutes pour l'offre). Raison : Chrome arrête un service worker inactif,
  et un secret gardé en mémoire seulement rendrait le QR affiché inutilisable
  sans le dire. `Secure` gagne `restore_offer` et `restore_join`.
- **Maintien de la connexion** (vérifié dans la documentation Chrome) :
  depuis Chrome 116, une WebSocket active prolonge la vie du service worker
  si un message passe au moins toutes les 30 s ; la documentation recommande
  20 s et ne dit rien des trames de contrôle, qui ne sont de toute façon pas
  visibles du JavaScript. Nouvelles trames `ping` (client → relais, session
  authentifiée) et `pong` (réponse) : l'extension envoie `ping` toutes les
  20 s et considère la connexion morte sans aucune trame pendant 75 s.
  `ot-core` décode `pong` mais n'envoie pas de `ping` : les pings WebSocket du
  relais lui suffisent, et un message toutes les 20 s coûterait de la
  batterie au téléphone. Une alarme (`chrome.alarms`, 30 s, le minimum
  depuis Chrome 120) réveille le service worker s'il a été arrêté pendant une
  attente de reconnexion. `minimum_chrome_version` : 120.
- **Raccourci clavier** (`Alt+Shift+S`) : envoie l'URL de l'onglet actif au
  dernier destinataire choisi. Il n'envoie pas la sélection, contrairement à
  ce qui avait été proposé : la lire demanderait d'injecter un script dans la
  page (permission `scripting`), ce que le plan exclut. La sélection passe
  par le menu contextuel.
- **Permission en plus : `clipboardRead`**, pour « envoyer le
  presse-papiers ». Sans elle, la lecture depuis la popup dépend d'une
  invite de Chrome peu fiable dans une popup. Elle n'est utilisée qu'au clic
  sur le bouton. `alarms` est aussi ajoutée (point précédent). Pas de
  `host_permissions` : une WebSocket n'en demande pas.
- **Popup et notifications** : la popup ouvre un port vers le service
  worker ; tant qu'il est ouvert, un message reçu n'affiche pas de
  notification (demande ajoutée au plan). Le service worker n'accepte de
  messages que de sa propre extension (pas d'`externally_connectable`, pas de
  script de contenu, expéditeur vérifié).
- **Identifiant fixe** : la clé publique est dans le manifeste (`key`),
  l'identifiant vaut `fiblpidpjopjpinfgonlkblkonllpgfb` ; la clé privée,
  utile seulement pour empaqueter, reste dans `private/`.
- **Copier** : depuis la popup, `navigator.clipboard` directement (page au
  premier plan) ; depuis une notification, par le document offscreen
  (`execCommand('copy')`, seule méthode qui marche hors premier plan).
- **Outillage** : Angular 22 sans zone.js, à base de signals, tests Vitest ;
  service worker et document offscreen construits par esbuild ; WebAssembly
  construit par `cargo build --target wasm32-unknown-unknown` puis
  `wasm-bindgen` (cible `web`). `getrandom` y utilise `crypto.getRandomValues`
  (fonction `wasm_js`), `time` l'horloge JavaScript (fonction
  `wasm-bindgen`) ; seul `ot-wasm` active ces fonctions.

**Conséquences.**

- Une seule implémentation du roster, des époques et de l'appairage : les
  tests Rust existants couvrent aussi l'extension. Le TypeScript garde une
  seconde implémentation du transport, vérifiée par les golden et par un
  test contre un vrai `ot-server` en CI.
- `ping`/`pong` : relais et extension vont ensemble ; un relais plus ancien
  répond `unknown_type` au `ping` et l'extension se reconnecte toutes les
  75 s. Les clients Rust ne changent pas de comportement.
- Une page web ou une autre extension ne peut pas parler au service worker ;
  à vérifier en relisant le manifeste.
