# 0029 — Extension : fichiers reçus ouverts par le navigateur

**Statut : acceptée** (14 octobre 2026). Tranche D27 de
`decisions-a-prendre.md` ; valide le plan T8 de l'[étape 5](../etape-5.md),
qui remplace la réception de fichiers des notes 0026 et 0028 dans
l'extension. Essai T8a concluant dans Brave (14 octobre 2026, points 0 à 6) :
**voie A**.

**Contexte.** Premier essai réel : un fichier reçu était rangé par
`chrome.downloads`, et Brave demandait où l'enregistrer (son réglage
« Demander où enregistrer chaque fichier » l'emporte sur `saveAs: false`).
L'aperçu passait par une page maison. Je veux le comportement de l'extension
Pushbullet : le navigateur ouvre le fichier, je l'enregistre si je veux.

**Décision.**

- **Petit fichier (moins de 20 Mo) que le navigateur sait afficher** (image,
  PDF, texte, audio, vidéo) : ouvert dans son lecteur, sans rien enregistrer
  (règles de la note 0024 pour l'ouverture automatique). L'enregistrer passe
  par le clic droit du navigateur ou l'icône de téléchargement de la popup.
- **D27 — Fichier que le navigateur ne sait pas afficher** (`.zip`,
  `.docx`…), quelle que soit sa taille : rien n'arrive sur le disque sans
  geste. Une notification (ou l'élément dans la popup ouverte) ; le
  téléchargement se fait depuis l'extension.
- **Gros fichier** : la popup s'ouvre (à défaut, notification) ; on le
  télécharge depuis elle.
- **Popup** : un clic sur un fichier l'ouvre ; une icône de téléchargement à
  droite de l'élément, toujours présente, le télécharge (téléchargement natif
  du navigateur). Annuler ce téléchargement ne change rien à l'élément.
- **Envoi depuis la popup** : le trombone ouvre directement le sélecteur de
  fichiers de l'OS ; le destinataire est celui choisi dans la popup (plus de
  liste dans la fenêtre d'envoi).

- **Voie A** (essai T8a) : le service worker sert lui-même les adresses
  `chrome-extension://<id>/f/<blob_id>/<nom>` (événement `fetch`), en flux :
  morceaux lus dans une copie **chiffrée** de l'OPFS, ou retirés du relais
  et ajoutés à cette copie, déchiffrés dans le WebAssembly au fil de l'eau.
  `inline` pour l'ouvrir, `attachment` (`?dl=1`) pour un téléchargement
  natif ; requêtes `Range` pour les vidéos. Un fichier de moins de 20 Mo est
  copié (chiffré) dès sa réception, son SHA-256 vérifié, et le relais
  prévenu (`done`) ; un gros fichier l'est au fil de sa première lecture.
  La clé du fichier reste dans le coffre tant que la copie existe ; les deux
  partent quand l'élément quitte l'historique, ou après 7 jours.
- **Sécurité de l'affichage** : ces réponses sont servies sous l'origine de
  l'extension. Sauf pour le PDF (lu par le lecteur intégré, sans script),
  elles portent `Content-Security-Policy: sandbox` (origine opaque : aucun
  accès aux API de l'extension), et `X-Content-Type-Options: nosniff`. SVG et
  HTML ne s'affichent jamais : ils se téléchargent.
- **Téléchargement = navigation** (correctif après l'essai du 15 octobre) :
  un lien `download` (ou `<a download>`) est récupéré par le gestionnaire de
  téléchargements sans passer par le service worker, et échoue (« Vérifiez
  votre connexion Internet »). Le téléchargement se fait donc par un lien
  ouvert dans un nouvel onglet (`target="_blank"`), que le service worker
  sert en `attachment` et que le navigateur referme, comme dans l'essai
  T8a.
- Plus de permission `downloads`, de page `viewer.html`, de raison `BLOBS`
  ni de fichier déchiffré dans l'OPFS ; la page d'essai `probe.html` est
  retirée.

**Conséquences.**

- Plus de fichier reçu déchiffré dans le profil (voie A : copie chiffrée).
- Bug relevé à l'essai, corrigé par T8e : après un téléchargement annulé
  dans la fenêtre de Brave, l'élément montrait « Ouvrir » au lieu du bouton
  de téléchargement, sans fichier sur le disque (l'élément était déjà
  « enregistré » dans l'OPFS).
- Si la popup se ferme quand le sélecteur de fichiers de l'OS s'ouvre (à
  vérifier dans T8a), une page de l'extension prend le relais, sans liste de
  destinataires.
