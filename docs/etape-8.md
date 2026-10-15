# Plan — Étape 8 : confort, durcissement et publication

L'étape 8 rend OldTown autonome et prêt à être publié : historique chiffré
consultable sur chaque appareil, réveil du téléphone sans connexion
permanente, révocation sans SSH, sauvegardes, et la liste de `SECURITY.md`
entièrement cochée avant de rendre le dépôt public (note 0011).

Références : jalon « 8. Confort et durcissement » de la
[conception](design.md), [`SECURITY.md`](../SECURITY.md), notes 0014
(rotation), 0015 (`wss`) et 0018 (historique, révocation).

**Niveau de détail** : plan de cadrage (note 0012). Les blocs ci-dessous sont
indépendants : chacun peut devenir une sous-étape (8a, 8b…) avec son propre
plan détaillé au moment de le lancer.

## 8a. Historique chiffré et recherche

- Base locale (note 0018) : SQLite simple sur Android (module `history` de
  `ot-core`), IndexedDB dans l'extension ; chaque contenu est chiffré un par
  un par `ot-crypto` (`ot-wasm` dans l'extension), sous une clé gardée comme
  les autres secrets. Recherche en déchiffrant l'historique en mémoire.
  Décider au démarrage si les métadonnées (date, type, expéditeur) sont
  chiffrées aussi.
- Rétention réglable (par défaut 90 jours), contenus `sensitive` jamais
  enregistrés (aucun marquage n'existe : l'étape 6 est abandonnée, note
  0034), effacement complet en un geste.
- Écrans « Historique » sur Android et dans l'extension, recherche, renvoi d'un élément.
- Critère de fin : retrouver un lien reçu il y a un mois en tapant un mot ;
  les contenus de la base copiée sur une autre machine sont illisibles.

## 8b. Réveil par UnifiedPush et ntfy

- ntfy auto-hébergé sur le laptop, joignable seulement par Tailscale.
- L'appli Android s'enregistre comme application UnifiedPush (distributeur :
  appli ntfy) et transmet son point de terminaison au relais (nouvelle trame
  authentifiée, golden et note de décision).
- Quand un message attend un appareil hors ligne, le relais pousse un réveil
  **sans contenu** ; l'appli se connecte, reçoit, acquitte, puis se
  déconnecte. Le service de premier plan devient optionnel.
- Critère de fin : téléphone sans service permanent, message reçu en moins de
  10 s, consommation de batterie mesurée et comparée à l'étape 3.

## 8c. Révocation sans SSH

- Trame authentifiée `revoke_device` ; tout membre peut révoquer, l'action
  est journalisée et notifiée aux autres appareils (note 0018).
- La révocation enchaîne la rotation de la clé de groupe (étape 2, T8).
- Critère de fin : depuis le téléphone, révoquer le PC ; le PC est refusé à sa
  prochaine connexion et ne lit plus rien.

## 8d. Sauvegardes et reprise

- Relais : la sauvegarde quotidienne des appareils (note 0006) part aussi hors
  du laptop, chiffrée (restic ou Borg), avec la configuration.
- Groupe : procédure documentée si tous les appareils sont perdus (recréer le
  groupe et réappairer) ; option d'une sauvegarde papier de la clé de groupe,
  à trancher.
- Exercice de restauration du relais sur une machine vierge, chronométré et
  documenté dans le README.

## 8e. Indépendance réseau (optionnel)

- Évaluer Headscale sur un petit VPS pour remplacer le serveur de
  coordination de Tailscale ; garder Tailscale si l'évaluation n'apporte
  rien de concret.
- Ou se passer de Tailscale : relais joignable par la box, en `wss` avec un
  nom de domaine et un certificat propres (note 0015), ce qui permet aussi
  l'extension sur un ordinateur sans Tailscale. Préalables : les limites du
  relais de 8f, et une revue de l'exposition à Internet.

## 8f. Avant publication

La liste de `SECURITY.md`, dans l'ordre :

- [ ] Limiter les connexions non authentifiées et freiner les échecs
      d'authentification et d'appairage sur le relais.
- [ ] Avertissement dans le README sur l'exposition du relais hors tailnet
      (`wss` est en place depuis l'étape 3).
- [ ] Relecture externe de `ot-crypto` et des formats chiffrés.
- [ ] Recherche de données personnelles dans tout l'historique git.
- [ ] Documentation traduite en anglais et rendue générique.
- [ ] Réglages GitHub : détection de secrets, Dependabot, protection de
      `main`, signalement privé de failles.
- [ ] Rendre le dépôt public, note de décision datée.

## Critères de fin de l'étape

Les critères de 8a à 8d sont tenus, 8e est évalué (fait ou écarté par une
note), toute la liste 8f est cochée.
