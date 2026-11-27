# ot-wasm

`ot-core` sans réseau (fonction `net` désactivée) et `ot-proto`, compilés en
WebAssembly pour l'extension Chrome (note 0023). Exposé par `wasm-bindgen` :
`Vault` (coffre en mémoire, `SecretStore` du cœur, avec relevé des
changements, signature des requêtes HTTP `signHttp`), `Core`
(`ot_core::Secure` : sceller, ouvrir, roster, quarantaine, appairage,
rotation ; fichiers : `upload`, `startUpload`, `sealChunk`, `sealFile`,
`openChunk`, `checkFile`…, `startUpload` vers une personne avec `toUser` ;
étape 10 : annuaire, envoi `0x05`, journal, invitation, et pour l'admin
`adminInitStart`/`adminRoot`/`adoptAdminRoot`, `certifiedBy`/`recertify`,
`servicesToRepublish`/`nextServiceEntry`/`serviceEntryPublished` ; lu
partout, note 0079 : `sealRead`, événement `read`, capacités passées à
`declarationFrame`), `Hasher`
(SHA-256 en flux), `fileGeometry`, `decodeFrame`, `parseJoin`,
`parseInvite`, `qrCode`.

- Les déroulés en plusieurs trames (publication, `admin_init`) gardent
  leur objet signé en mémoire jusqu'à la réponse du relais : perdu à
  l'arrêt du service worker, il se refait à la lecture suivante (un
  certificat racine déjà publié est adopté).

- Aucune clé ne sort d'ici, sauf par `Vault::take_changes` pour être
  chiffrée et rangée aussitôt par le service worker.
- Entrées et sorties complexes en JSON (enveloppes, événements, groupe) ;
  erreurs en `Error` JavaScript au message lisible, sans secret.
- Appairages en attente rangés dans le coffre (`pending_offer.json`,
  `pending_join.json`) et repris à l'ouverture : le service worker peut être
  arrêté à tout moment. Effacés dès que le groupe est adopté.
- Seule crate qui active `getrandom/wasm_js` et `time/wasm-bindgen`.
- Build : `cargo build -p ot-wasm --target wasm32-unknown-unknown --profile
  wasm`, puis `wasm-bindgen` (fait par `npm run wasm` dans
  `apps/extension`). Les tests Rust (`cargo test -p ot-wasm`) tournent en
  natif ; ceux du WebAssembly lui-même sont dans `apps/extension/test/`.
