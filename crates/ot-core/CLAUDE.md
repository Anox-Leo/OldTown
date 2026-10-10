# ot-core

Cœur client réutilisé par la CLI, l'appli Android (via `ot-ffi`, étape 3) et
l'appli desktop Tauri (étape 4) : identité, connexion au relais,
authentification, reconnexion avec backoff, file d'envoi, dédoublonnage ;
à partir de l'étape 2, chiffrement, roster et appairage (en s'appuyant sur
`ot-crypto`).

- API publique pensée pour UniFFI : un handle `Client` clonable, un flux
  d'`Event`, des types simples. Pas de génériques ni de traits complexes dans
  ce qui est exposé.
- Aucune dépendance à un OS ou à un stockage précis : les secrets passent par
  un trait (`SecretStore` / `Keyring`), chaque appli fournit son
  implémentation.
- Livraison au moins une fois : un message n'est acquitté qu'après avoir été
  remis à l'appli (ou rejeté définitivement), et `Dedup` écarte les doublons.
- Jamais de contenu, de clé ni de jeton dans les logs `tracing`.
- Tests : unitaires dans le crate ; les scénarios avec un vrai relais vivent
  dans `crates/ot-server/tests/` (relais en processus).
