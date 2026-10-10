# ot-crypto

Chiffrement de bout en bout : types de clés (`GroupKey`, `ExchangeKeyPair`,
`PairingSecret`), `seal`/`open` pour les trois schémas de `payload.data`,
données associées, signature, corps des messages (`Body`, `Roster`) et code
d'appairage (`PairingCode`).

- Aucune E/S, aucun réseau, aucun async, aucun tokio : la crate doit pouvoir
  se compiler pour Android (UniFFI) et en WebAssembly (extension, note 0017).
- Primitives RustCrypto uniquement (`chacha20poly1305`, `x25519-dalek`,
  `hkdf`, `sha2`, `ed25519-dalek`) ; aucune construction maison sans note de
  décision.
- Toute clé secrète vit dans un type qui s'efface (`Zeroize`,
  `ZeroizeOnDrop`, `Zeroizing`) et dont le `Debug` n'affiche rien de secret.
- **Le format est figé par `tests/vectors/`.** Un vecteur qui casse signale un
  changement du format chiffré : s'il est voulu, régénérer
  (`OT_REGENERATE_VECTORS=1 cargo test -p ot-crypto --test vectors`), écrire
  une note dans `docs/decisions/` et prévenir que tous les clients doivent
  être mis à jour ensemble.
- Les erreurs (`CryptoError`) ne portent jamais de clé ni de contenu.
- `ot-server` ne dépend jamais de cette crate (vérifié en CI).
