# Revue des choix de planification après l'étape 1

Relecture, au 10 octobre 2026, des choix faits pendant la première session de
planification (conception, étapes 0 et 1) à la lumière de la mise en place
réelle. Elle sert de point de départ aux plans des étapes 2 à 8.

**En bref : l'architecture tient.** Relais Rust sur le laptop, Tailscale, JSON
sur WebSocket, identité Ed25519 et livraison au moins une fois ont marché du
premier coup entre Windows et Mac. Les ajustements portent sur l'enregistrement
des appareils (trop manuel), sur la règle « une session par appareil » qui
contraint toutes les applis, et sur quelques briques des étapes suivantes
(bibliothèque crypto, Android récent, ordre de certaines fonctions).

Chaque ajustement proposé ici est repris dans
[`decisions-a-prendre.md`](decisions-a-prendre.md) avec sa recommandation ; les
plans `etape-N.md` appliquent la recommandation tant que rien n'est tranché.

## Ce que le terrain a confirmé

| Choix initial | Constat | Verdict |
| --- | --- | --- |
| Relais sur le laptop Ubuntu, service systemd | Repart seul après reboot, `/healthz` répond ; consommation négligeable | Gardé |
| Tailscale, écoute sur l'IP du tailnet seulement | Aucun port ouvert ; deux pannes réseau côté client, toutes deux hors OldTown (voir plus bas) | Gardé |
| Serveur en Rust (axum + sqlx), même langage que le cœur | 2 700 lignes, 49 tests, CI sur trois OS ; aucun point bloquant | Gardé |
| JSON sur WebSocket, trames versionnées | Débogage facile ; golden tests efficaces pour figer le format | Gardé ; les fichiers passeront en HTTP binaire (étape 5) |
| Identité Ed25519 et défi signé dès l'étape 1 | Fonctionne ; le modèle de données n'aura pas à migrer | Gardé |
| Livraison au moins une fois, dédoublonnage client | Essai réel hors ligne puis reconnexion sans doublon | Gardé |
| Le serveur ne dépend que de `ot-proto` | Règle simple à vérifier, tenue | Gardé, étendue à `ot-crypto` (étape 2) |
| Une note de décision par écart | 11 notes en une étape ; c'est ce qui permet de reprendre le projet sans mémoire | Gardé, renforcé pour les agents |
| Durée de vie 7 jours (Q7) | Pas de problème constaté ; le contenu sera chiffré à l'étape 2 | Gardé |

## Ce que le terrain a appris

**1. L'enregistrement d'un appareil est trop manuel.** Il faut lancer `ot init`,
copier la clé publique, ouvrir une session SSH sur le laptop, lancer
`ot-server device add`, recopier l'identifiant, relancer `ot init`. Acceptable
pour deux ordinateurs, pas pour le téléphone. L'appairage de l'étape 2 doit
supprimer le passage par SSH, pas seulement transmettre la clé de groupe.

**2. « Une session par appareil » structure toutes les applis** (note 0009).
C'est le bon choix (pas de livraison en double), mais il impose un seul
processus propriétaire de la connexion sur chaque appareil :

- Android : le menu Partager, la tuile et les notifications passent par le
  service de premier plan, jamais par une connexion à eux.
- Desktop : l'appli Tauri tient la connexion ; la CLI installée sur la même
  machine doit être un appareil distinct (sinon elle coupe l'appli).
- Extension navigateur : le navigateur lance **un nouveau processus** de
  native messaging à chaque connexion. Ce processus ne doit donc pas parler au
  relais : il relaie vers l'appli desktop par une socket locale. Ce point
  n'apparaissait pas dans la conception.
- Fichiers (étape 5) : les transferts HTTP s'authentifient sans enregistrer de
  session.

**3. Les pannes réseau viennent des postes clients.** Un autre VPN sous
Windows captait la plage `100.64.0.0/10` ; le relais démarre parfois avant que
l'IP Tailscale soit montée (systemd réessaie 5 s plus tard). Les deux sont
documentés dans le README. Pour les applis, cela veut dire : un état de
connexion visible et un message d'erreur qui pointe vers Tailscale plutôt que
vers OldTown.

**4. Le laptop sert à la fois au développement et à la production.** Les
agents Claude Code tournent dessus sous l'utilisateur de dev, le relais sous
`ot-relay`. La séparation tient tant que les agents n'ont pas `sudo`. Après
l'étape 2, une fuite de la base du relais n'expose plus de contenu.

**5. La CI coûte des minutes.** Elle est déjà limitée (PR sous Linux, trois OS
sur `main`, rien pour la doc). Android et Tauri vont coûter bien plus cher :
chaque appli aura son propre workflow, filtré sur son dossier.

**6. L'environnement de travail a changé.** L'étape 0 visait T3 Code en SSH ;
le développement se fait désormais avec un orchestrateur d'agents qui n'a pas
accès à la mémoire des sessions de planification. Conséquence : tout ce qu'un
agent doit savoir est dans le dépôt (`CLAUDE.md`, plans, notes), et chaque
tâche d'un plan est autonome (fichiers concernés, tests attendus, critère de
fin).

## Choix revus pour les étapes suivantes

| Sujet | Plan initial | Proposition | Pourquoi |
| --- | --- | --- | --- |
| Appairage (étape 2) | QR code dès l'étape 2 | Code texte copié-collé entre CLI à l'étape 2, même code en QR à l'étape 3 | Il n'y a pas encore d'appli avec caméra ; le contenu du code est le même |
| Enregistrement | `ot-server device add` par SSH | Le relais accepte un jeton d'appairage à usage unique créé par un appareil déjà enregistré ; la commande d'admin reste pour amorcer le premier appareil | Supprime le passage par SSH |
| Bibliothèque crypto | libsodium ou RustCrypto | RustCrypto (pur Rust) | Compilation croisée Android, macOS, Windows sans bibliothèque C ; `cargo deny` voit tout |
| Fichiers chiffrés (étape 5) | `secretstream` de libsodium | Construction STREAM de RustCrypto (`aead::stream`) | Conséquence du choix précédent ; mêmes garanties (ordre, troncature détectée) |
| Clé de groupe | Une clé partagée, rotation à la révocation (étape 8) | Gardée, mais la rotation arrive dès l'étape 2 | Le transport de clé scellée vers un appareil est déjà écrit pour l'appairage ; le téléphone (étape 3) est l'appareil le plus exposé au vol |
| Alternative écartée | — | Chiffrer pour chaque destinataire, sans clé de groupe | Exigerait un annuaire de clés authentifié ; avec la clé de groupe, le relais ne peut pas ajouter de faux appareil |
| Code crypto | Dans `ot-core` | Nouvelle crate `ot-crypto`, sans réseau ni async | Testable seule, réutilisable par UniFFI et Tauri, et le serveur ne doit jamais en dépendre (vérifiable en CI) |
| Tuile Android « envoyer le presse-papiers » | Étape 6 | Étape 3 | Quelques dizaines de lignes, et c'est le geste le plus utile sur le téléphone |
| Service Android | Service de premier plan | Type `specialUse` (appli installée hors Play Store), pas `dataSync` | `dataSync` est limité à 6 h par jour à partir d'Android 15 (à revérifier au démarrage de l'étape 3) |
| `ws://` sur Android | Non traité | Exception réseau limitée au nom du relais ; `wss` reste dans la liste avant publication | Android refuse le trafic en clair par défaut ; WireGuard chiffre déjà le lien |
| Client Ubuntu | Serveur seulement | L'appli Tauri compile aussi pour Linux, sans presse-papiers live sous Wayland | Coût quasi nul avec Tauri ; le laptop est aussi un poste de travail |
| Extension navigateur | Native messaging vers l'appli | Native messaging vers un petit relais local qui passe par l'appli (socket locale) | Conséquence de la note 0009 |
| Organisation du dépôt | Monorepo | Monorepo : `crates/` pour Rust, `apps/android`, `apps/desktop`, `apps/extension` | Un workflow CI par dossier |

## Ce qui ne change pas

- La feuille de route en huit jalons et leur ordre.
- Le relais aveugle : il ne lit jamais `payload`, même pour vérifier le type de
  chiffrement. Le refus du contenu en clair se fait côté clients.
- Le dépôt privé jusqu'à la fin du projet (note 0011) et la liste de
  `SECURITY.md` avant publication.
- La documentation en français, traduite avant publication.
