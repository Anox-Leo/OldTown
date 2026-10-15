# OldTown : conception et mise en place

**Recommandation : construire ton propre système autour d'un serveur relais sur le laptop Ubuntu, qui ne voit que des données chiffrées de bout en bout, et d'un cœur partagé en Rust (protocole et chiffrement) réutilisé par l'appli Android et l'extension Chrome (en WebAssembly).** Le laptop suffit largement comme serveur ; Tailscale le rend joignable de partout sans ouvrir de port, et un VPS n'est utile que plus tard.

Le vrai travail est côté clients : chaque OS a ses propres règles sur le presse-papiers et le partage. La feuille de route avance par jalons utilisables, du simple envoi de texte aux fichiers volumineux, puis vers un système autonome prêt à publier.

## Besoins

- **Serveur** : laptop Ubuntu, toujours allumé et connecté, à la maison.
- **Clients** : téléphone Android 13, Mac M3, PC Windows 10, plus un navigateur sur chacun.
- **Contenus** : texte, liens, fichiers (documents personnels, potentiellement volumineux).
- **Intégration** : menu Partager d'Android, extension navigateur, presse-papiers envoyé d'un geste (pas de synchronisation automatique, notes 0019 et 0034).
- **Portée** : à la maison et à distance (4G, autre Wi-Fi).
- **Sécurité** : chiffrement de bout en bout, le serveur ne voit jamais le contenu en clair.
- **Projet** : système maison, adaptable finement, développé sur la durée.

## Architecture

&#91;embedded content: architecture · 3 clients, 1 serveur relais\]

Le chiffrement se fait dans le cœur Rust de chaque client ; le laptop ne fait que relayer, mettre en attente et réveiller le téléphone.

## Composants et choix techniques

Un cœur Rust unique évite d'écrire trois fois le protocole et le chiffrement, là où les bugs coûtent le plus cher.

| Brique | Choix proposé | Rôle | Alternative |
| --- | --- | --- | --- |
| Cœur partagé | Rust : crypto (RustCrypto, crate `ot-crypto`), protocole, file d'envoi, cache local SQLite | Tout ce qui ne dépend pas de l'OS | Kotlin Multiplatform |
| Serveur relais | Rust (axum + tokio) ou Go, SQLite, blobs sur disque | Authentifier les appareils, router les messages, stocker les fichiers chiffrés avec durée de vie | — |
| Appli Android | Kotlin + Jetpack Compose, cœur via UniFFI | Cible de partage, tuile, notifications, réception | — |
| Extension Chrome | Manifest V3, interface Angular, crypto `ot-crypto` en WebAssembly, protocole en TypeScript | Client des ordinateurs : tient sa propre connexion, envoie la page, un lien, la sélection ou le presse-papiers, reçoit en notification | Appli desktop Tauri, écartée (note 0017) |
| Notifications Android | WebSocket dans un service de premier plan, puis [UnifiedPush](https://unifiedpush.org/users/distributors/) avec ntfy auto-hébergé | Réveiller le téléphone sans passer par Google | Firebase (FCM), à éviter ici |

Il n'y a pas d'appli desktop (note 0017) : l'extension Chrome est un appareil à part entière, autonome, installable sur n'importe quel ordinateur qui a Chrome, y compris un ordinateur professionnel, dans l'esprit de Pushbullet. Son service worker est l'unique propriétaire de la connexion (note 0009) ; ses clés sont chiffrées dans le profil Chrome.

## Contraintes par plateforme

Android est la seule plateforme où le presse-papiers ne peut pas être lu automatiquement ; tout le reste se fait proprement.

| Plateforme | Envoyer | Recevoir | Presse-papiers | Piège à anticiper |
| --- | --- | --- | --- | --- |
| Android 13 | Cible de partage (`ACTION_SEND`, `SEND_MULTIPLE`), raccourcis Direct Share par appareil | Notification avec actions Copier et Ouvrir, écriture dans le presse-papiers | Lecture interdite en arrière-plan depuis Android 10 : tuile Quick Settings qui ouvre une activité transparente le temps de lire | Marquer les contenus sensibles (`EXTRA_IS_SENSITIVE`, Android 13) pour masquer l'aperçu ; optimisation batterie à désactiver pour l'appli |
| macOS (M3) | Extension de partage, menu Services, glisser sur l'icône de barre de menu | Notification, écriture dans `NSPasteboard` | Pas d'événement de changement : sonder `changeCount` toutes les 0,5 s environ | Ignorer les types `org.nspasteboard.ConcealedType` (mots de passe) ; vérifier les nouvelles alertes de confidentialité du presse-papiers sur ta version de macOS |
| Windows 10 | Entrée « Envoyer vers » ou menu contextuel, glisser sur l'icône | Notification toast, écriture dans le presse-papiers | Événement natif `AddClipboardFormatListener` | Ignorer les formats `ExcludeClipboardContentFromMonitorProcessing` posés par les gestionnaires de mots de passe |
| Ordinateurs (extension Chrome) | Menu contextuel (page, lien, sélection, image), popup, raccourci | Notification Chrome, popup avec les derniers éléments reçus | Envoi sur un geste, écriture à la réception (note 0019) | Réception seulement quand Chrome tourne ; service worker arrêté après 30 s d'inactivité |
| Ubuntu (si le laptop sert aussi de client) | GSConnect montre la voie : extension GNOME Shell | Notification | Sous GNOME Wayland, une appli ordinaire ne peut pas surveiller le presse-papiers en arrière-plan (à confirmer sur ta version) : passer par une extension GNOME ou une session X11 | Le serveur seul n'en a pas besoin |

Sans appli desktop (note 0017) ni étape presse-papiers (note 0034), les lignes macOS, Windows et Ubuntu restent une référence pour une suite éventuelle ; seules les lignes Android et extension Chrome sont construites.

Pour éviter les boucles, chaque élément porte un identifiant : un client qui écrit un contenu reçu ne le renvoie pas quand il le détecte.

## Protocole et chiffrement

Le serveur authentifie des appareils et transporte des enveloppes opaques ; seuls tes appareils détiennent les clés qui les ouvrent.

1. **Identité d'appareil** : chaque appareil génère une paire de clés Ed25519 (signature) et X25519 (échange), stockées dans le coffre de l'OS (Android Keystore, Trousseau macOS, DPAPI Windows).
2. **Appairage par code, puis QR** : un appareil déjà membre obtient du relais un jeton à usage unique et affiche un code (jeton + secret que le relais ne voit jamais) ; le nouvel appareil s'enregistre avec le jeton, puis reçoit la clé de groupe dans un échange chiffré sous le secret. Code copié-collé entre CLI à l'étape 2 ; à partir de l'étape 3, QR dans les deux sens : le téléphone scanne le code pour rejoindre, puis scanne le QR affiché par chaque nouvel appareil pour le parrainer (QR inversé, note 0013 ; détail : [étape 2](etape-2.md), [étape 3](etape-3.md)).
3. **Authentification au serveur** : défi signé avec la clé Ed25519, pas de mot de passe ; connexion WebSocket persistante en TLS par-dessus le VPN.
4. **Messages** : texte et liens chiffrés avec XChaCha20-Poly1305 sous la clé de groupe, signés par l'expéditeur ; le serveur ne voit que destinataire, taille et date.
5. **Fichiers** : découpés en morceaux chiffrés en flux (construction STREAM de RustCrypto), envoyés en HTTP avec reprise ; le message n'emporte que la référence et la clé du fichier.
6. **File d'attente** : le serveur garde les enveloppes pour les appareils hors ligne, puis les supprime après accusé de réception ou au bout d'une durée que tu fixes.
7. **Révocation** : retirer un appareil fait tourner la clé de groupe, redistribuée aux appareils restants (rotation disponible dès l'étape 2).

Une clé de groupe unique suffit pour quatre appareils à toi. Si tu veux plus tard des propriétés plus fortes (confidentialité persistante par message), MLS via la bibliothèque OpenMLS est l'évolution naturelle.

## Le laptop Ubuntu en serveur

Oui, c'est possible et c'est suffisant : un serveur relais en Rust ou Go consomme quelques dizaines de Mo de RAM, et la batterie fait office d'onduleur en cas de coupure.

**Réglages du laptop**

- Ne pas se mettre en veille capot fermé : `HandleLidSwitch=ignore` dans `/etc/systemd/logind.conf`, et désactiver la mise en veille automatique.
- Limiter la charge de la batterie à environ 80 % pour éviter qu'elle gonfle en restant branchée en permanence (`charge_control_end_threshold` ou TLP, selon ce que ton modèle accepte).
- Préférer un câble Ethernet au Wi-Fi, et prévoir de la place disque pour les fichiers en transit.
- Lancer le serveur comme service systemd qui redémarre tout seul ; démarrage automatique après coupure de courant si le BIOS le propose.

**Réseau**

- **Tailscale** sur le laptop et les trois clients : gratuit en usage personnel ([tarifs](https://tailscale.com/pricing)), trafic WireGuard chiffré de bout en bout, aucun port ouvert sur ta box. Le serveur n'écoute que sur son IP Tailscale.
- Plus tard, pour ne dépendre de personne : Headscale (serveur de coordination libre) sur un petit VPS, ou WireGuard brut avec un port ouvert et un nom de domaine dynamique.
- Un VPS (Hetzner CX23, 5,49 € HT par mois depuis juin 2026, [source](https://byteiota.com/hetzner-june-2026-price-shock/)) ne sert que de relais ; il ne stocke jamais rien en clair.

**Sécurité du serveur**

- Chiffrer au moins le volume de données. Avec un disque entier chiffré (LUKS), un redémarrage demande la phrase secrète : prévoir un déverrouillage à distance par SSH au démarrage (dropbear-initramfs) ou par la puce TPM.
- Mises à jour de sécurité automatiques (unattended-upgrades), SSH par clé uniquement et seulement via Tailscale.
- Double authentification sur le compte Tailscale ; retirer immédiatement un appareil perdu.
- Sauvegarde chiffrée (restic ou Borg) de la configuration et des clés du serveur, pas des messages.

## Feuille de route

Chaque jalon donne un outil utilisable au quotidien, ce qui te permet d'ajuster le suivant à l'usage réel. Chaque jalon a son plan détaillé (`etape-N.md`) ; les ajustements faits après l'étape 1 sont expliqués dans la [revue](revue-etapes-0-1.md), et les choix encore ouverts dans [`decisions-a-prendre.md`](decisions-a-prendre.md).

| Jalon | À construire | Ce que tu sais faire à la fin |
| --- | --- | --- |
| [1. Fondations](etape-1.md) (terminé) | Tailscale, serveur relais minimal (WebSocket + SQLite), CLI de test, cœur Rust avec tests | Envoyer un texte d'un terminal à un autre |
| [2. Chiffrement et appairage](etape-2.md) (terminé) | Clés d'échange, clé de groupe et rotation, appairage par code, relais aveugle | Le serveur ne voit plus rien en clair |
| [3. Android : partage](etape-3.md) (terminé) | Appli Kotlin, cible de partage, notifications, service de premier plan, appairage QR dans les deux sens, `wss` | Partager un lien ou un texte depuis n'importe quelle appli du téléphone |
| [4. Extension Chrome](etape-4.md) (terminé) | Extension autonome : crypto en WebAssembly, protocole en TypeScript, menus contextuels, popup, notifications | Échanger avec le téléphone depuis n'importe quel ordinateur qui a Chrome |
| [5. Fichiers](etape-5.md) (terminé) | Envoi par morceaux chiffrés, reprise, durée de vie sur le serveur | Envoyer des documents et des photos volumineux |
| [8. Confort et durcissement](etape-8.md) | Historique chiffré et recherche, UnifiedPush avec ntfy, révocation sans SSH, sauvegardes, Headscale, publication | Un système autonome, sans dépendance externe |

Deux jalons ont été supprimés, sans réattribuer leur numéro : le 6, presse-papiers ([note 0034](decisions/0034-etape-6-abandonnee.md) : l'envoi du presse-papiers d'un geste existe déjà dans l'extension), et le 7, extension navigateur, devenu le jalon 4 ([note 0017](decisions/0017-extension-chrome-sans-appli-desktop.md)). Entre les jalons 5 et 8, l'appli Android a repris l'accueil en conversations de l'extension ([plan](plan-android-conversations.md), [note 0033](decisions/0033-android-conversations-partage-par-usage.md)). Le jalon 8 conclut le projet.

Dès les jalons 1 à 3, OldTown remplace déjà l'usage principal de Pushbullet.

## Annexe : solutions existantes dont s'inspirer

Ces projets open source résolvent déjà une partie du problème ; leur code est une bonne référence pour les cas difficiles (KDE Connect pour Android, ClipCascade pour le presse-papiers chiffré).

| Solution | Partage Android | Presse-papiers live | Fichiers | Hors réseau local | Chiffrement | Serveur à héberger |
| --- | --- | --- | --- | --- | --- | --- |
| [KDE Connect](https://kdeconnect.kde.org/) / GSConnect | Oui, natif | PC vers téléphone auto, téléphone vers PC en un geste | Oui | Oui, via Tailscale ou WireGuard | TLS entre appareils appairés | Aucun |
| [ClipCascade](https://github.com/Sathvik-Rao/ClipCascade) | Via l'app | Oui, tous les OS | Oui (1 Mio par défaut, plus en mode P2P) | Oui | Bout en bout | Oui, Docker ou Java 21 |
| [Syncthing](https://syncthing.net/) | Partage vers un dossier | Non | Oui, dossiers synchronisés | Oui | TLS, nœud « non fiable » chiffré possible | Optionnel (nœud toujours allumé) |
| [PairDrop](https://github.com/schlagmichdoch/PairDrop) | Oui (appli web) | Non, envoi manuel de texte | Oui | Oui, avec serveur TURN | WebRTC (DTLS) | Oui, Docker |
| [LocalSend](https://localsend.org/) | Oui | Non | Oui | Non, réseau local seulement | HTTPS bout en bout | Aucun |
| [ntfy](https://ntfy.sh/) | Oui | Non | Oui, en pièce jointe | Oui | TLS seulement, le serveur voit le contenu | Oui, Docker |
| [Nextcloud](https://nextcloud.com/) | Oui | Non | Oui, stockage complet | Oui | Serveur voit le contenu (E2EE limité) | Oui, plus lourd (2 Go de RAM et plus) |

## Annexe : détail par solution

**KDE Connect / GSConnect.** C'est le plus intégré : il apparaît dans le menu Partager d'Android, envoie fichiers et liens, synchronise les notifications, et existe sur Android, Linux (GSConnect sur Ubuntu GNOME), Windows, macOS et iOS ([téléchargements](https://kdeconnect.kde.org/download.html)). Pas de serveur : les appareils se parlent directement. Limite principale : depuis Android 10, une appli ne peut plus lire le presse-papiers en arrière-plan, donc ce que tu copies sur le téléphone part via la tuile « Envoyer le presse-papiers » ([discussion KDE](https://discuss.kde.org/t/can-kde-connect-synchronise-the-clipboard-both-ways/33959)). Hors du Wi-Fi maison, il faut un VPN et ajouter les appareils par leur IP. La version macOS reste moins aboutie.

**ClipCascade.** Outil open source fait exactement pour le presse-papiers multi-appareils : texte, images et fichiers, applis Windows, macOS, Linux et Android, serveur auto-hébergé en Docker, chiffrement de bout en bout et mode pair-à-pair où le serveur ne fait que la mise en relation ([dépôt](https://github.com/Sathvik-Rao/ClipCascade), v3.2.0). Limites : projet porté par un seul développeur, taille de message limitée à 1 Mio par défaut, et l'Android reste soumis à la même restriction sur la lecture du presse-papiers. Pense à changer le compte admin par défaut dès l'installation.

**Syncthing.** Idéal pour les documents : un dossier « Échanges » identique sur toutes les machines, synchronisé dès qu'elles se voient. Un serveur toujours allumé sert de relais quand le téléphone et le PC ne sont pas en ligne en même temps, et il peut être déclaré « non fiable » pour ne stocker que du chiffré. Pas de presse-papiers.

**PairDrop.** Équivalent web d'AirDrop, auto-hébergeable, installable comme appli sur Android. Pratique depuis un ordinateur qui n'est pas le tien. Les deux appareils doivent être en ligne en même temps.

**LocalSend.** Très simple et chiffré, sur tous les OS, mais uniquement sur le même réseau local ([site](https://localsend.org/)). À garder comme dépannage pour les gros fichiers à la maison.

**ntfy et Nextcloud.** ntfy pousse texte et pièces jointes vers le téléphone mais le serveur lit tout ; Nextcloud est un cloud complet (fichiers, photos, agenda) plus lourd à maintenir. À garder si tu veux aussi un stockage personnel, pas pour l'échange instantané.
