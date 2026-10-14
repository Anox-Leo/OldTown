# 0030 — Extension : la popup devient une conversation par canal

**Statut : acceptée** (15 octobre 2026). Demande après l'essai de l'étape 5 ;
remplace la mise en page de la popup (notes 0024 et 0029 pour le reste).

**Contexte.** La popup montrait une zone d'envoi en haut et la liste des
seuls éléments reçus. Je veux une conversation : mes messages et ceux des
autres appareils, par canal, la zone d'envoi en bas.

**Décision.**

- **Canaux** : « Tous », puis un canal par autre appareil du groupe. Un
  élément est rangé ainsi (`channelOf`, `shared/api.ts`) :
  - envoyé d'ici : le canal de son unique destinataire, sinon « Tous » ;
  - reçu : le canal de son expéditeur s'il m'était adressé à moi seul,
    sinon « Tous » (envoi à tous, ou à plusieurs appareils).
- **Destinataires d'un message reçu** : lus dans l'enveloppe (`to`, en clair
  pour que le relais la livre), ajoutés à l'événement par le client
  TypeScript. Aucun changement de format ni du Rust.
- **Messages envoyés dans l'historique** (chiffré, comme les reçus) :
  rangés avant la mise en file, « en cours » jusqu'à l'accusé du relais
  (`accepted`). Un fichier envoyé apparaît une fois déposé ; pendant le
  dépôt, une bulle de progression. Historique porté à 100 éléments.
- **Ordre des canaux** : « Tous » toujours en premier ; puis les appareils
  du plus utilisé au moins utilisé (nombre d'éléments reçus et envoyés dans
  le canal, dans l'historique), à égalité le plus récemment actif, puis par
  nom.
- **Canal ouvert = destinataire** : choisir un canal règle aussi le
  destinataire des raccourcis et des menus (le « dernier destinataire »).
- **Bulles** : envoyées à droite, reçues à gauche (nom de l'expéditeur dans
  « Tous ») ; clic pour ouvrir un fichier ou un lien, ou déplier un texte ;
  icône à droite pour télécharger un fichier reçu ou copier un texte.

**Conséquences.**

- Un élément envoyé n'est ni notifié ni ouvert (`deliverItem`).
- Un fichier envoyé d'ici ne s'ouvre pas depuis la popup : l'extension n'en
  garde pas de copie.
- Les messages envoyés depuis un autre appareil (CLI, téléphone) au groupe
  apparaissent comme reçus, dans « Tous ».
