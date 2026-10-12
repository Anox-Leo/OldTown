# ot-core

Cœur client réutilisé par la CLI et l'appli Android (via `ot-ffi`, étape 3),
et, sans sa partie réseau, par l'extension Chrome (via `ot-wasm`, étape 4,
note 0023) : identité, connexion au relais, authentification, reconnexion
avec backoff, file d'envoi, dédoublonnage ; à partir de l'étape 2,
chiffrement, roster et appairage (en s'appuyant sur `ot-crypto`).

- **Fonction `net`** (par défaut) : `Client`, `redeem_pairing`, TLS, et les
  dépendances tokio, tokio-tungstenite, rustls. Sans elle, le crate se
  compile en WebAssembly : rien de ce qui reste ne doit dépendre du réseau,
  d'un runtime ou de l'OS. Les types communs vivent dans `types.rs`.
  `Secure` (et `Effect`) est public pour `ot-wasm`, qui refait en TypeScript
  la boucle de `client.rs` (même règles : acquitter après avoir rangé l'état).
- API publique pensée pour UniFFI : un handle `Client` clonable, un flux
  d'`Event`, des types simples. Pas de génériques ni de traits complexes dans
  ce qui est exposé.
- Aucune dépendance à un OS ou à un stockage précis : chaque appli fournit
  un `SecretStore` (octets nommés : fichiers `0600` pour la CLI, Keystore
  pour Android) ; `StoreKeyring` en tire le `Keyring` (`keyring.rs`), au
  format des fichiers de l'étape 2.
- Connexion en `ws://` ou `wss://` (`tls.rs`, rustls avec `ring`, autorités
  de Mozilla, `TlsRoots::with_extra` pour les tests). Tout message reçu, ping
  compris, entretient la session (`idle_timeout`). Le client Rust n'envoie
  pas de trame `ping` (les pings WebSocket du relais suffisent) ; il ignore
  `pong`.
- Appairage : code (`redeem_pairing`, `pair_join`), et QR inversé (note
  0022) : `pair_offer` côté nouvel appareil, `sponsor` côté parrain (trame
  `pair_register`, réponse en schéma `0x04`). Formats texte des QR et
  classement texte/lien dans `uri.rs`.
- Livraison au moins une fois : un message n'est acquitté qu'après avoir été
  remis à l'appli (ou rejeté définitivement, ou mis en quarantaine dans le
  `Keyring`), et `Dedup` écarte les doublons.
- Chiffrement dans `secure.rs` (sceller, ouvrir, roster, quarantaine,
  appairage, rotation), règles du roster dans `roster.rs`, stockage par le
  trait `Keyring` (`keyring.rs`, `MemoryKeyring` pour les tests). Le format
  vient de `ot-crypto` ; aucune primitive crypto n'est appelée directement ici.
- Aucun envoi en clair : `Client::send` chiffre toujours et refuse sans
  groupe ; `send_envelope` ne sert qu'aux tests.
- Jamais de contenu, de clé ni de jeton dans les logs `tracing`.
- Tests : unitaires dans le crate ; les scénarios avec un vrai relais vivent
  dans `crates/ot-server/tests/` (relais en processus).
