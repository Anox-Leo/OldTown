# 0032 — Extension : la popup s'ouvre sur la conversation de l'élément reçu

**Statut : acceptée** (15 octobre 2026). Précise la note 0031 (« la popup
s'ouvre toujours sur l'accueil ») pour les ouvertures déclenchées par un
élément reçu.

**Contexte.** Un message reçu de « Téléphone » ouvre la popup (note 0024)
sur l'accueil : il faut encore toucher la bonne conversation. Même chose
après un clic sur la notification.

**Décision.**

- Quand le service worker ouvre la popup pour un élément reçu (texte, ou
  gros fichier, navigateur au premier plan), ou après un clic sur sa
  notification (texte, ou fichier à voir dans la popup), il retient le
  canal de l'élément (`channelOf`) puis ouvre la popup (ou sa fenêtre de
  repli).
- À la connexion de la popup (`index.html` seulement, pas les réglages), il
  lui pousse `{ type: 'open', channel }` juste après l'état ; la popup ouvre
  cette conversation, qui devient le destinataire comme un clic sur sa
  ligne. Un appareil sorti du groupe donne « Tous mes appareils ».
- Le canal retenu vit en mémoire 10 s au plus et ne sert qu'une fois ; une
  ouverture manuelle de la popup reste sur l'accueil passé ce délai.

**Conséquences.**

- Si l'ouverture automatique échoue (notification à la place) et que
  j'ouvre la popup moi-même dans les 10 s, elle montre la conversation de
  ce message : voulu.
- Un clic sur une notification pendant que la popup est déjà ouverte ne
  change pas la conversation affichée.
- Le plan Android (`docs/plan-android-conversations.md`, A6) prévoit la
  même chose pour les notifications du téléphone. Sa note de décision
  prendra le prochain numéro libre.
