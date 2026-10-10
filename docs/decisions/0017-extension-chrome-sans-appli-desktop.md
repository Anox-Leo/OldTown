# 0017 — Pas d'appli desktop : une extension Chrome autonome à l'étape 4

**Statut : acceptée** (10 octobre 2026). Tranche D12, D16, D17, D19 et D21 de
`decisions-a-prendre.md`, et remplace le point D4 de la note
[0012](0012-organisation-du-travail.md) (ordre des jalons).

**Contexte.** La feuille de route prévoyait une appli Tauri sur chaque
ordinateur (étape 4), propriétaire de la connexion au relais, et une
extension navigateur qui passait par elle (étape 7). Je n'ai pas besoin d'une
appli desktop : tous mes environnements de travail ont Chrome, et une
extension suffit. Surtout, je pourrai peut-être l'ajouter un jour au
navigateur d'un ordinateur professionnel, où installer une appli ou un
binaire ne sera sans doute pas permis : c'est l'esprit de Pushbullet, une
extension qui se suffit à elle-même.

**Décision.**

- **D16 — Pas d'appli desktop**, ni sur Ubuntu ni ailleurs. Sur un
  ordinateur, le client est l'extension Chrome ; la CLI reste l'outil de test
  et d'administration.
- **Extension autonome** : elle tient elle-même la connexion au relais, sans
  binaire à installer.
  - La connexion appartient au service worker (note 0009) ; popup, menus et
    raccourci lui parlent par messages.
  - Crypto et formats en Rust : `ot-crypto` compilée en WebAssembly (nouvelle
    crate `ot-wasm`). Le client du protocole (connexion, authentification,
    file, acquittements, dédoublonnage, roster) est réécrit en TypeScript ;
    sa compatibilité est vérifiée en rejouant les golden de `ot-proto` et les
    vecteurs de `ot-crypto` dans ses tests.
  - Clés générées dans le WebAssembly, stockées dans le profil Chrome,
    chiffrées par une clé WebCrypto AES-GCM non exportable (même principe que
    le Keystore Android, étape 3).
- **D21 — Chrome seulement**, extension non empaquetée en mode développeur,
  identifiant fixé par la clé du manifeste (clé privée dans `private/`).
- **D12 — CLI** : l'extension est un appareil à part entière ; une CLI
  lancée sur le même ordinateur est un autre appareil, appairé séparément.
  Aucun conflit de session.
- **D19 — Interface de l'extension** (popup, page de réglages) en Angular,
  que je maîtrise et relis facilement.
- **D17 — Monorepo** : `crates/` (Rust, dont `ot-wasm`), `apps/android`,
  `apps/extension` ; un workflow CI par dossier. Plus de `apps/desktop`.
- **Jalons** : 1 fondations, 2 chiffrement, 3 Android, **4 extension
  Chrome**, 5 fichiers, 6 presse-papiers (note 0019), 8 confort. Le
  numéro 7 n'est pas réattribué, pour ne pas casser les références.

**Conséquences.**

- Réception seulement quand Chrome tourne ; le relais garde les messages en
  attente 7 jours, rien n'est perdu.
- Chrome arrête un service worker inactif après 30 s : une trame applicative
  doit passer au moins toutes les 30 s (nouvelle trame de maintien dans
  `ot-proto`, golden et note à l'étape 4).
- Deux implémentations du protocole (Rust et TypeScript) : tout changement de
  format doit passer les golden des deux côtés.
- Les clés dans le profil Chrome sont moins bien protégées que dans un coffre
  de l'OS ; les secrets restent autant que possible dans la mémoire du
  WebAssembly (`zeroize`), le JavaScript ne manipule pas de clé en clair.
- Sur un ordinateur professionnel, deux obstacles possibles : une politique
  qui interdit le mode développeur (il faudrait alors une publication non
  répertoriée sur le Chrome Web Store, à décider le moment venu), et
  l'absence de Tailscale (le relais devra alors être joignable par la box,
  note 0015). L'extension ne doit rien supposer d'autre que Chrome et
  l'accès réseau au relais.
- Le presse-papiers live reposait sur l'appli desktop : l'étape 6 est
  recadrée par la note 0019. Les fichiers (étape 5) passent par l'extension.
- `etape-4.md` devient le plan de l'extension ; `etape-7.md` ne garde qu'un
  renvoi.
