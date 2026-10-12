# 0024 — Retours de l'essai de l'étape 4 : Brave, ouverture directe, raccourcis

**Statut : acceptée** (12 octobre 2026). Retours après l'essai réel de
l'extension (Mac) et du téléphone ; précise les notes 0017 et 0023.

**Contexte.** J'utilise Brave sur tous mes ordinateurs, pas Chrome. L'essai
a montré ce qui manque ou gêne : notifications reçues alors que je suis déjà
dans le navigateur, clic sur une notification de texte qui n'ouvre pas
l'extension, pas de raccourci pour le presse-papiers, écran « Groupe » trop
technique, notifications du téléphone qui s'accumulent ou arrivent appli
ouverte. Par la suite (étape 5), photos et PDF reçus devront eux aussi
s'ouvrir directement dans le navigateur s'il est au premier plan.

**Décision.**

- **Navigateur cible : Brave** (Chromium), Chrome restant possible : même
  extension, mêmes API. La documentation et le nom proposé à l'appairage
  (« Brave Mac », d'après `navigator.brave`) en tiennent compte. Chromium 120
  au moins ; l'ouverture de la popup par l'extension demande Chromium 127.
- **Élément reçu, selon la situation** (`deliver` dans `main.ts`) :
  - popup ouverte : seulement dans la liste (inchangé) ;
  - navigateur au premier plan (fenêtre qui a le focus de l'OS,
    `windows.getLastFocused`) et élément de moins de 2 minutes : un lien
    `http(s)` s'ouvre dans un nouvel onglet, un texte ouvre la popup
    (`action.openPopup`), sans notification ; si la popup ne peut pas
    s'ouvrir, notification ;
  - sinon, notification. Un élément plus ancien (messages en attente à la
    réouverture du navigateur) donne toujours une notification : pas de
    rafale d'onglets.
- **Clic sur une notification** : un lien s'ouvre (inchangé) ; un texte
  ouvre la popup, ou à défaut la même page dans une petite fenêtre.
- **Raccourci `Alt+Maj+V`** : envoie le presse-papiers au dernier
  destinataire. Le service worker le lit par le document offscreen
  (`execCommand('paste')`, permission `clipboardRead` déjà présente), au
  raccourci seulement. Une coche brève sur l'icône confirme l'envoi des deux
  raccourcis.
- **Popup** : *Entrée* envoie, *Maj+Entrée* va à la ligne ; icônes au trait
  (réglages, presse-papiers « coller et envoyer » à côté du bouton d'envoi) ;
  plus d'onglet « Groupe ».
- **Groupe dans les réglages**, réduit aux noms des membres et à la nouvelle
  clé de groupe : plus d'empreintes ni d'époque à l'écran. Les empreintes
  servaient à vérifier les clés à la main ; la confiance vient de
  l'appairage (QR ou code), et `ot group show` les affiche toujours.
- **Android** : au plus 3 notifications de messages, la plus ancienne
  retirée pour la suivante ; à l'ouverture de l'écran principal, les
  notifications de messages déjà affichées sont retirées (les éléments sont
  dans la liste). La règle « appli au premier plan → pas de notification »
  (T8b) échouait encore sur le téléphone, sans pouvoir être reproduite sur
  émulateur Android 16 : elle s'appuie désormais aussi sur l'importance du
  processus donnée par Android (`IMPORTANCE_FOREGROUND` quand un écran de
  l'appli est visible), en plus du drapeau tenu par `MainActivity`. À
  revérifier sur le téléphone.

**Conséquences.**

- Un membre du groupe peut ouvrir un onglet `http(s)` dans mon navigateur
  quand il est au premier plan, sans clic de ma part. Accepté : seuls mes
  appareils sont membres, et seuls les liens web s'ouvrent (comme
  « Ouvrir » sur Android). L'étape 5 suivra la même règle pour les fichiers.
- `action.openPopup` n'existe pas avant Chromium 127 ou peut être refusé :
  la notification (ou la petite fenêtre) prend le relais.
