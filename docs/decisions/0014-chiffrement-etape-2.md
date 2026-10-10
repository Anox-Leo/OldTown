# 0014 — Chiffrement de l'étape 2 : bibliothèque, clé de groupe, bascule

**Statut : acceptée** (10 octobre 2026). Tranche D6 à D11 de
`decisions-a-prendre.md` : les recommandations du plan de l'étape 2 sont
retenues telles quelles.

**Contexte.** Le plan de l'étape 2 appliquait par défaut six choix ouverts par
la [revue après l'étape 1](../revue-etapes-0-1.md). Leurs avantages et
inconvénients ont été comparés avant de trancher.

**Décision.**

- **D6 — Bibliothèque** : RustCrypto (`chacha20poly1305`, `x25519-dalek`,
  `hkdf`, `sha2`), pas libsodium. Raison : pur Rust, compilation croisée sans
  chaîne C (Android, et WebAssembly pour l'extension, note 0017), dépendances
  visibles par `cargo deny`.
- **D7 — Clé de groupe** partagée par les membres, pas de chiffrement par
  destinataire.
- **D8 — Rotation** de la clé de groupe dès l'étape 2
  (`ot group rotate --exclude <appareil>`).
- **D9 — Crate `ot-crypto`**, sans réseau ni async ; `ot-server` n'en dépend
  jamais (vérifié en CI).
- **D10 — Ce sont les clients qui refusent le clair** ; le relais ne lit
  jamais `payload`, pas même `enc`.
- **D11 — Purge complète** des messages à la bascule
  (`ot-server messages purge --all`), suivie d'un `VACUUM` pour que les pages
  libérées par SQLite ne gardent pas de clair.

**Conséquences.** Le plan de l'étape 2 ne change pas, sauf l'ajout du `VACUUM`
à la purge (T10). Un message ciblé reste lisible par tout membre qui
obtiendrait son chiffré : c'est accepté.
