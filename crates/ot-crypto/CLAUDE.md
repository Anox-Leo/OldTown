# ot-crypto

Chiffrement de bout en bout : types de clés (`GroupKey`, `ExchangeKeyPair`,
`PairingSecret`), `seal`/`open` pour les cinq schémas de `payload.data`
(`0x04` : QR inversé, note 0022 ; `0x05` : plusieurs destinataires, entre
personnes, étape 10), données associées, signature, corps des messages
(`Body`, `Roster`), codes d'appairage et d'invitation (`PairingCode`,
`InviteCode`), QR inversé (`PairingOffer`), objets signés de l'annuaire
(`directory.rs` : `UserCert`, `RosterEntry`, ancre ; leurs règles de chaîne
sont dans `ot-core`) et, à l'étape 5, chiffrement des fichiers par morceaux
(`file.rs` : `FileKey`, `FileCipher`, `sanitize_file_name`, note 0026).

- Chaque clé n'admet que ses corps (`KeyKind::admits`), au scellement comme
  à l'ouverture : corps d'invitation sous la clé d'invitation seulement,
  `service_welcome` en `0x04` seulement, `0x05` seulement `text`, `link`,
  `file`, `otp`.
- Aucune E/S, aucun réseau, aucun async, aucun tokio : la crate doit pouvoir
  se compiler pour Android (UniFFI) et en WebAssembly (extension, note 0017).
- Primitives RustCrypto uniquement (`chacha20poly1305`, `aead-stream`,
  `x25519-dalek`, `hkdf`, `sha2`, `ed25519-dalek`) ; aucune construction
  maison sans note de décision.
- Toute clé secrète vit dans un type qui s'efface (`Zeroize`,
  `ZeroizeOnDrop`, `Zeroizing`) et dont le `Debug` n'affiche rien de secret.
- **Le format est figé par `tests/vectors/`.** Un vecteur qui casse signale un
  changement du format chiffré : s'il est voulu, régénérer
  (`OT_REGENERATE_VECTORS=1 cargo test -p ot-crypto --test vectors`), écrire
  une note dans `docs/decisions/` et prévenir que tous les clients doivent
  être mis à jour ensemble.
- Fichiers : STREAM (`StreamBE32<XChaCha20Poly1305>`) en accès direct
  (`index`, `last`), jamais l'`Encryptor` séquentiel : la reprise et le
  parallélisme en dépendent. Un corps `file` incohérent est refusé par
  `Body::from_json` ; son nom se nettoie avant toute écriture sur disque.
- Les erreurs (`CryptoError`) ne portent jamais de clé ni de contenu.
- `ot-server` ne dépend jamais de cette crate (vérifié en CI).
