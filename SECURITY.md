# Sécurité

OldTown est un projet personnel en cours de construction. **Ne pas l'utiliser
pour des données sensibles avant la bascule de l'étape 2** (README, « Passage
au chiffrement »), ni exposer le relais sur Internet. La partie
cryptographique n'a pas encore été relue par quelqu'un d'autre.

## Modèle de menace

| Acteur | Ce qu'il peut faire aujourd'hui (étape 2) | Cible |
| --- | --- | --- |
| Réseau entre appareil et relais | Rien : le relais n'écoute que sur Tailscale (WireGuard), et le contenu est chiffré de bout en bout | `wss` à l'étape 3 (note 0015) |
| Relais ou qui lit sa base | Voir les métadonnées : qui écrit à qui, quand, la taille des messages, les noms d'appareils. Ni lire, ni modifier, ni rejouer sous un autre `msg_id`, ni rediriger un contenu (données associées), ni fabriquer un message (signature d'un membre du roster) | Inchangé : les métadonnées restent visibles |
| Relais malveillant pendant un appairage | Refuser l'appairage. Pas l'usurper : sans le secret du code (128 bits), il ne lit pas la demande et ne fabrique pas de réponse | QR inversé à l'étape 3 (note 0013) |
| Relais qui ajoute un appareil à lui | Le faire apparaître dans `ot devices` (signalé « hors du groupe ») ; il ne reçoit rien de lisible et ses messages sont rejetés | Inchangé |
| Appareil du tailnet non enregistré | Ouvrir des connexions, échouer à l'authentification, essayer des jetons d'appairage (80 bits, 10 minutes, usage unique) | Limites de connexions et de débit |
| Membre du groupe | Lire et écrire comme les autres ; ajouter un appareil par appairage ; tirer une nouvelle clé de groupe | Inchangé : un groupe sert une seule personne |
| Appareil volé | Tout ce que fait un membre, jusqu'à `ot group rotate --exclude <nom>` (il ne lit plus la suite) et `ot-server device revoke` | Étape 8 : révocation depuis un appareil, sans SSH |

Ce qui est déjà en place :
- Chiffrement de bout en bout (étape 2, `ot-crypto`) : XChaCha20-Poly1305
  sous une clé de groupe par époque ; transport de clé par X25519 + HKDF-SHA256 ;
  signature Ed25519 de chaque message ; données associées couvrant en-tête,
  `msg_id`, `from`, `to` et `created_at`. Primitives RustCrypto uniquement,
  format figé par des vecteurs recoupés avec une implémentation indépendante.
- Les clients refusent le clair, les expéditeurs hors roster et les signatures
  fausses ; la confiance vient de l'appairage, jamais de la liste du relais.
- Rotation de la clé de groupe en excluant un appareil ; anciennes clés
  gardées 7 jours puis oubliées.
- Clés secrètes effacées de la mémoire (`zeroize`) et jamais affichées.
- Jetons d'appairage stockés par leur SHA-256 seulement, jamais journalisés.
- Authentification par défi Ed25519 signé, avec un préfixe de domaine
  (`ot-auth-v1`) et un nonce à usage unique.
- Taille des trames et des enveloppes bornée.
- Aucun contenu de message dans les logs.
- Service systemd sous un utilisateur dédié, durci.
- `cargo deny` en CI (failles connues, licences, sources).
- Clés privées et état du groupe de la CLI dans des fichiers `0600`.

## Avant de rendre le dépôt public

Le dépôt reste privé jusqu'à la fin du projet (décision 0011).

- [ ] Étape 2 livrée : contenu chiffré de bout en bout, relais aveugle.
      *(Code livré ; à cocher après la bascule réelle et l'essai Mac → Windows.)*
- [ ] Limiter les connexions non authentifiées (nombre simultané, délai) et
      freiner les échecs d'authentification.
- [ ] Option TLS (`wss`) pour un relais hors Tailscale, et avertissement clair
      dans le README.
- [ ] Relire la partie cryptographique (idéalement par quelqu'un d'autre).
- [ ] Vérifier que ni le dépôt ni son historique ne contiennent de données
      personnelles : `git log -p | grep -iE '<ip|nom de machine|tailnet>'`.
- [ ] Traduire la documentation en anglais et la rendre générique (sans
      référence à mon installation).
- [ ] Activer sur GitHub : détection de secrets avec blocage des pushs,
      Dependabot, protection de `main`, signalement privé de failles.

## Signaler une faille

Une fois le dépôt public : par le signalement privé de GitHub (onglet
*Security* → *Report a vulnerability*), pas par une issue publique.
