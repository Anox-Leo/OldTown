# ot-core

Cœur client réutilisé par la CLI et l'appli Android (via `ot-ffi`, étape 3),
et, sans sa partie réseau, par l'extension Chrome (via `ot-wasm`, étape 4,
note 0023) : identité, connexion au relais, authentification, reconnexion
avec backoff, file d'envoi, dédoublonnage ; à partir de l'étape 2,
chiffrement, roster et appairage (en s'appuyant sur `ot-crypto`).

- **Fonction `net`** (par défaut) : `Client`, `redeem_pairing`, TLS, les
  transferts de fichiers (`transfer.rs`, `http.rs`, client `hyper`), et les
  dépendances tokio, tokio-tungstenite, rustls, hyper. Sans elle, le crate se
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
  groupe ; `send_envelope` n'envoie que ce que `Client::seal` a chiffré
  (pour connaître le `msg_id` avant l'envoi, note 0033), ou les enveloppes
  construites par les tests.
- Fichiers (étape 5, note 0026) : `files.rs` (sans réseau) range les offres
  reçues (`files.json`) et les dépôts en cours (`uploads.json`) dans le
  `Keyring`, clés comprises, et signe les requêtes HTTP ; `Secure` traite le
  corps `file` (offre rangée **avant** l'acquittement, puis `Event::File`
  sans clé). Toute écriture de cet état passe par la tâche du client
  (`Command::WithSecure`) : jamais d'accès concurrent. `transfer.rs` fait les
  transferts en parallèle (4), réessaie tant que le transfert progresse, et
  lit et écrit par `ChunkSource` et `ChunkSink`, fournis par l'appli et
  appelés hors du runtime (`spawn_blocking`). Un retrait n'est finalisé
  (`ChunkSink::finish`) qu'après la vérification du SHA-256.
- Annuaire (étape 10) : `directory.rs` (sans réseau) vérifie tout jusqu'à
  l'ancre (certificat racine, journal de l'admin, empreintes `certs`,
  certificats, journaux des personnes, profils filtrés sur le roster
  vérifié) et ne croit jamais le relais ; l'état vérifié est rangé dans le
  `Keyring` (`directory.json`). Le client le lit à chaque connexion et à
  chaque `directory_changed` (`DirectoryRead` : pages par `next`, tours tant
  qu'un journal a `more`), seulement s'il a une ancre. Les refus sans
  référence du relais (`pair_create`, `directory_get`, `admin_init`) sont
  rendus à la plus ancienne demande en attente (`Pending`) : le relais
  répond dans l'ordre.
- Envois entre personnes (étape 10, `0x05`) : `Secure::recipients` résout
  une personne d'après l'annuaire vérifié seulement (tous ses appareils
  joignables et à jour, le principal en tête, note 0079, plus la copie à mes
  appareils hors CLI) ; les clés viennent des rosters vérifiés, jamais
  des profils. `Client::send_to`/`seal_to` relisent l'annuaire s'il date de
  plus de 60 s (`Command::Fresh`). Un `0x05` d'un expéditeur inconnu va en
  quarantaine, réessayée après chaque lecture. `Origin` dit qui écrit et à
  qui ; `ReceivedOtp::auto_fill` porte la règle O2. Le client déclare
  `users-v1` (`device_update`) quand son profil ne l'a pas.
- Invitation et administration (étape 10) : l'état vit dans le `Keyring`
  (`invites.json` côté admin, `pending_invite.json` côté invité, avec le
  secret `S`) ; une demande d'invitation est vérifiée et rangée avant
  l'acquittement, la suite (certificat gardé, `user_certify`, entrée qui
  l'engage, bienvenue) est menée en tâche de fond par le client
  (`Flow`, `people.rs`) et reprend après une coupure. L'invité ne crée son
  groupe qu'une fois son certificat confirmé par l'annuaire (tête
  `admin_head` atteinte). Une seule voie publie mon journal :
  `Secure::next_entry(Publish)` (roster local plus récent, certificat engagé
  ou retiré), sous un verrou, en relisant sur `conflict` ; seul un membre du
  roster publié (ou le `genesis`, entrée 1) signe, et mon groupe adopte la
  version publiée. Le relais ne prévient pas la session qui change quelque
  chose : le client relit l'annuaire lui-même. Les trames d'administration
  passent par `Client::call` (réponse ou refus, `Pending::Call`), coupées par
  une perte de connexion, jamais rejouées.
- Services (étape 10, § 6.1 et 6.2) : `secure/service.rs` (sans réseau) et
  `services.rs`. L'admin ajoute un appareil de service par son QR inversé
  (`Client::add_service_device`) : `service_create` ou `service_device`,
  certificat engagé par son journal, entrée du journal du service signée par
  un appareil de l'admin, puis `service_welcome` en `0x04` sous le secret du
  QR ; chaque étape se reprend avec le même QR. L'appareil de service range
  la bienvenue en mémoire et ne se dit rangé (`Event::Joined`) qu'une fois
  l'annuaire lu jusqu'à `admin_head`, son certificat et son roster confirmés
  et la signature vérifiée par la clé de l'expéditeur dans le roster de
  l'admin. Après le retrait d'un appareil de l'admin, le client republie le
  journal de chaque service (`Flow::Republish`, D47).
- Jamais de contenu, de clé ni de jeton dans les logs `tracing`.
- Tests : unitaires dans le crate ; les scénarios avec un vrai relais vivent
  dans `crates/ot-server/tests/` (relais en processus).
