# Sécurité

OldTown est un projet personnel en cours de construction. Le contenu est
chiffré de bout en bout depuis l'étape 2, mais la partie cryptographique n'a
pas encore été relue par quelqu'un d'autre : **ne pas exposer le relais sur
Internet**, et rester prudent avec les données sensibles.

## Modèle de menace

| Acteur | Ce qu'il peut faire aujourd'hui (étape 3) | Cible |
| --- | --- | --- |
| Réseau entre appareil et relais | Rien : le relais n'écoute que sur Tailscale (WireGuard), en `wss` (certificat `tailscale cert`, autorités de Mozilla côté clients), et le contenu est chiffré de bout en bout | Inchangé ; limites de connexions avant de sortir de Tailscale |
| Relais ou qui lit sa base | Voir les métadonnées : qui écrit à qui, quand, la taille des messages, les noms d'appareils. Ni lire, ni modifier, ni rejouer sous un autre `msg_id`, ni rediriger un contenu (données associées), ni fabriquer un message (signature d'un membre du roster) | Inchangé : les métadonnées restent visibles |
| Relais malveillant pendant un appairage | Refuser l'appairage. Pas l'usurper : sans le secret du code ou du QR inversé (128 bits), il ne lit pas la demande et ne fabrique pas de réponse. Au QR inversé, enregistrer une autre clé sous l'identifiant du nouvel appareil ne lui donne rien : la réponse est scellée pour la clé X25519 lue dans le QR, et le roster porte la vraie clé | Inchangé |
| Relais qui ajoute un appareil à lui | Le faire apparaître dans `ot devices` (signalé « hors du groupe ») ; il ne reçoit rien de lisible et ses messages sont rejetés | Inchangé |
| Appareil du tailnet non enregistré | Ouvrir des connexions, échouer à l'authentification, essayer des jetons d'appairage (80 bits, 10 minutes, usage unique) | Limites de connexions et de débit |
| Membre du groupe | Lire et écrire comme les autres ; ajouter un appareil par appairage ; tirer une nouvelle clé de groupe | Inchangé : un groupe sert une seule personne |
| Appareil volé | Tout ce que fait un membre, jusqu'à `ot group rotate --exclude <nom>` (ou *Groupe* → exclure, depuis le téléphone ou l'extension) et `ot-server device revoke`. Sur Android, les clés sont chiffrées par une clé du Keystore, hors sauvegardes, mais utilisables téléphone verrouillé (réception la nuit). Dans l'extension Chrome, elles sont chiffrées par une clé WebCrypto non exportable du même profil : quiconque ouvre la session de l'ordinateur (ou copie tout le profil Chrome, IndexedDB compris) peut s'en servir | Étape 8 : révocation depuis un appareil, sans SSH |

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
- `wss` (étape 3) : rustls avec `ring`, certificat relu à chaud, poignées de
  main bornées à 10 s ; aucun trafic en clair depuis le code Java de l'appli
  Android (configuration réseau).
- Android : secrets chiffrés en AES-256-GCM par une clé non exportable du
  Keystore (nom du fichier en donnée associée), dans `noBackupFilesDir`,
  sauvegardes Android désactivées ; « Ouvrir » limité aux liens `http(s)` ;
  « Copier » relit le texte dans l'appli au lieu de le faire passer par un
  `Intent`.
- Extension Chrome (étape 4, note 0023) : clés générées et utilisées dans
  le WebAssembly ; au repos dans `chrome.storage.local`, chiffrées en
  AES-256-GCM par une clé WebCrypto non exportable gardée dans IndexedDB (nom
  en donnée associée) ; l'historique reçu aussi. Le service worker n'accepte
  de messages que des pages de l'extension (pas d'`externally_connectable`,
  pas de script de contenu) ; CSP `script-src 'self' 'wasm-unsafe-eval'` ;
  « Ouvrir » limité aux liens `http(s)`. Navigateur au premier plan, un lien
  reçu d'un membre s'ouvre seul dans un onglet (`http(s)` seulement, note
  0024). Limite : un instant en clair dans le JavaScript pour passer à
  WebCrypto.
- Appairage par QR inversé (note 0022) : enregistrement par le parrain
  seulement depuis une session authentifiée, signature du nouvel appareil
  vérifiée par le parrain et par le relais, réponse authentifiée par `S` et
  scellée pour la clé X25519 du nouvel appareil (schéma `0x04`, vecteurs
  recoupés).

## Avant de rendre le dépôt public

Le dépôt reste privé jusqu'à la fin du projet (décision 0011).

- [x] Étape 2 livrée : contenu chiffré de bout en bout, relais aveugle.
- [ ] Limiter les connexions non authentifiées (nombre simultané, délai) et
      freiner les échecs d'authentification.
- [x] Option TLS (`wss`) pour un relais hors Tailscale (étape 3, note 0022) ;
      le README déconseille toujours d'exposer le relais.
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
