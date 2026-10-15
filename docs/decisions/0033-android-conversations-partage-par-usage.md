# 0033 — Android : conversations comme l'extension, partage trié par usage

**Statut : acceptée** (15 octobre 2026). Réalise le
[plan des conversations Android](../plan-android-conversations.md) (A1 à
A6), qui reprend les notes 0030 et 0031 de l'extension, et ajoute le tri
des destinataires du partage, demandé en même temps.

**Contexte.** L'extension montre une liste de conversations puis la
conversation ouverte ; l'appli Android gardait un compositeur et la liste
des seuls éléments reçus. Je veux la même chose sur le téléphone (en
gardant les écrans Groupe et Réglages), et, dans la feuille de partage, les
appareils les plus utilisés en premier.

**Décision.**

- **Destinataires des reçus** (A1) : `ot_core::Received` et
  `ot_core::ReceivedFile` portent `to`, lu dans l'enveloppe (en clair pour
  que le relais la livre). Aucun changement de format, de golden ni de
  vecteur.
- **Éléments de l'historique** (`ot_ffi::Received`) : nouveaux champs `to`
  (vide : tous), `outgoing` et `pending`, tous `#[serde(default)]` : un
  ancien `history.json` se relit.
- **Envois dans l'historique** (A2), comme l'extension (note 0030) :
  - `send_text` chiffre d'abord (`Client::seal`, nouveau) pour connaître le
    `msg_id`, range l'élément « en cours », envoie l'enveloppe
    (`send_envelope`), puis le marque accepté à l'accusé du relais, ou le
    retire si l'envoi échoue (erreur rendue à l'appelant) ;
  - `send_file` range l'élément une fois le fichier déposé et annoncé
    (`Client::send_file_info`, nouveau, décrit le fichier sans sa clé) ;
    pendant le dépôt, seule la progression s'affiche ;
  - chaque étape est signalée par `Event::Sent { item }` (rangé ou mis à
    jour) ou `Event::Unsent { msg_id }` ; `RelayService` met `RelayHub` à
    jour sans relire l'historique et ne notifie jamais un envoi ;
  - historique porté à **100 éléments** (Rust et `RelayHub`).
- **Mêmes règles que l'extension** (A3, `ui/Conversations.kt`) :
  `channelOf`, `deviceKind`, palette et FNV-1a de `avatarColor` (même
  couleur des deux côtés, vérifiée par des valeurs calculées par
  l'extension), ordre des conversations, aperçu, heure courte.
- **Accueil et conversation** (A4, A5) : comme la note 0031. Présence lue à
  l'affichage puis toutes les 15 s, tant que l'écran est au premier plan et
  connecté. Toucher une bulle ouvre la feuille de l'élément (`ItemSheet`,
  sortie dans son propre fichier).
- **Notification → conversation** (A6) : l'`Intent` porte le canal
  (`EXTRA_CHANNEL`), jamais le contenu.
- **Feuille de partage triée par usage** : « Tous mes appareils » reste en
  tête (ce n'est pas un appareil) ; puis les appareils du plus utilisé au
  moins utilisé, c'est-à-dire le nombre d'éléments reçus et envoyés dans
  leur conversation, dans l'historique (`byUsage`, la règle d'ordre de la
  note 0030) ; à égalité, le plus récemment actif, puis par nom. Les
  raccourcis Direct Share sont rangés de la même façon (au démarrage du
  service et à chaque changement du groupe).

**Écarts et précisions par rapport au plan.**

- Le canal ouvert ne devient pas le destinataire par défaut du partage : le
  tri par usage le remplace.
- Pas d'`imePadding` dans la conversation : la racine de `MainActivity` a
  déjà `safeDrawingPadding`, qui comprend le clavier ; l'ajouter le
  compterait deux fois.
- Les bulles de progression d'un envoi de fichier ne s'affichent que dans
  la conversation de ses destinataires (`Transfer.to`) ; l'extension les
  montre partout.
- Une bulle de fichier envoyé d'ici ne s'ouvre pas (le téléphone n'en garde
  pas de copie, comme l'extension) ; la feuille d'un envoi est signée
  « Vous ».
- L'heure d'un fichier envoyé est celle de l'accusé, prise sur le téléphone
  (le cœur scelle l'annonce dans sa tâche, sans rendre l'enveloppe).
- Test instrumenté sur un état fictif (ordre de l'accueil, bulle à droite
  dans le canal d'un appareil) ; l'envoi réel avec `to == [id]` est testé en
  Rust contre un relais en processus.

**Conséquences.**

- `send_envelope` sert hors des tests, mais seulement pour une enveloppe
  chiffrée par `Client::seal` : toujours aucun envoi en clair.
- L'ordre du partage ne voit que les 100 derniers éléments, et repart de
  zéro quand l'historique est effacé (Réglages).
- Les messages envoyés depuis un autre appareil au groupe apparaissent comme
  reçus, dans « Tous » (comme l'extension).
- Hors périmètre, comme prévu : non-lus et badges, recherche, suppression,
  épinglage.
