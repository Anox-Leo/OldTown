# 0018 — Fichiers reçus, historique local, révocation

**Statut : acceptée** (10 octobre 2026). Tranche D20, D22 et D23 de
`decisions-a-prendre.md`.

**Décision.**

- **D20 — Téléchargement des fichiers reçus** automatique sous 20 Mo (sur
  Wi-Fi seulement pour le téléphone), à la demande au-delà ; blobs gardés
  48 h sur le relais.
- **D22 — Historique local en SQLite simple** (IndexedDB dans l'extension),
  avec les contenus chiffrés un par un par `ot-crypto` ; pas de SQLCipher. La
  recherche se fait en déchiffrant l'historique en mémoire, suffisant pour
  quelques milliers d'éléments (90 jours de rétention).
- **D23 — Révocation sans SSH par tout membre**, journalisée et notifiée aux
  autres appareils ; `ot-server device revoke` par SSH reste le recours.

**Conséquences.**

- Aucune dépendance C pour l'historique, et la règle « RustCrypto
  uniquement » tient.
- Les métadonnées de l'historique (date, type, expéditeur) restent en clair
  dans la base locale, sauf décision contraire au démarrage de 8a.
- Risque accepté : un voleur avec un appareil déverrouillé peut révoquer les
  autres appareils ; la reprise passe par SSH sur le laptop.
