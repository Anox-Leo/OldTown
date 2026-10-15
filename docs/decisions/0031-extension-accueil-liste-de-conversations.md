# 0031 — Extension : accueil en liste de conversations

**Statut : acceptée** (15 octobre 2026). Demande après l'essai de la note
0030 ; remplace la barre de canaux et l'ordre des canaux de cette note (le
rangement des éléments, `channelOf`, ne change pas).

**Contexte.** La conversation par canal convient, mais les canaux en
pastilles au-dessus des messages se lisent mal. Je veux un accueil à la
WhatsApp : la liste des conversations, puis la conversation ouverte.

**Décision.**

- **Accueil** (`home.ts`) : en-tête « OldTown » et bouton des réglages
  (plus d'état de connexion ; le bandeau d'aide reste quand le relais est
  injoignable). Puis une ligne par conversation : avatar, nom en
  demi-gras, heure et aperçu du dernier élément sur une ligne (« Vous : »
  devant un envoi, nom de l'expéditeur dans « Tous », trombone devant un
  fichier).
- **« Tous mes appareils »** reste une conversation, toujours en première
  ligne (les envois à tous et les reçus à plusieurs n'ont pas d'autre
  place).
- **Ordre** : les appareils du plus récemment actif au moins récent, puis
  par nom (au lieu du plus utilisé, note 0030).
- **Avatar** : pictogramme du type d'appareil (téléphone, portable,
  ordinateur fixe, navigateur ; groupe pour « Tous ») sur une couleur tirée
  de l'identifiant de l'appareil, donc stable. Le roster ne porte pas le
  type d'appareil (l'y ajouter changerait le format) : il se devine au nom
  (`deviceKind`, `devices.ts`), navigateur d'abord (« Chrome Mac » est
  l'extension), portable par défaut.
- **Conversation** (`chat.ts`) : en-tête flèche de retour, avatar, nom, et
  dessous « en ligne » ou « hors ligne » (pour « Tous » : combien sont en
  ligne). Plus de barre de canaux.
- **Présence** : la liste `devices` du relais (trame existante), demandée à
  la connexion puis toutes les 15 s tant que la popup est ouverte ; un
  point vert sur l'avatar à l'accueil. Inconnue quand cet appareil est
  hors ligne : rien n'est affiché.
- **Ouvrir une conversation** règle le destinataire (`set-recipient`) comme
  avant. La popup s'ouvre toujours sur l'accueil.

**Conséquences.**

- Aucun changement de format, de service worker ni de permission.
- Un nom d'appareil trompeur donne un mauvais pictogramme ; sans
  conséquence autre que visuelle.
- Popup ouverte, une requête `devices` toutes les 15 s vers le relais.
