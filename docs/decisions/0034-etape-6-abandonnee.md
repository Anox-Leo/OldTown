# 0034 — Étape 6 (presse-papiers) abandonnée

**Statut : acceptée** (15 octobre 2026). Décision de ma part ; remplace le
jalon « 6. Presse-papiers » de la [conception](../design.md) et le plan
[`etape-6.md`](../etape-6.md).

**Contexte.** Après la note 0019, l'étape 6 n'était plus qu'une petite étape :
contenu `clip`, images collées, écriture automatique à la réception, marquage
sensible, et peut-être la tuile Android. L'envoi du texte du presse-papiers
d'un geste existe déjà (bouton et raccourci de l'extension, note 0024), et la
réception propose « Copier » partout. Je n'en ai pas besoin de plus avant de
conclure le projet avec l'étape 8.

**Décision.**

- L'étape 6 est supprimée. `etape-6.md` ne garde qu'un renvoi vers cette
  note ; le numéro 6 n'est pas réattribué (comme le 7, note 0017).
- Après l'étape 5 et les améliorations de l'appli Android (note 0033), la
  prochaine étape est l'étape 8.
- La note 0019 reste valable pour ce qui existe : pas de surveillance
  automatique du presse-papiers, un contenu ne part que sur un geste.

**Conséquences.**

- Pas de contenu `clip` ni d'images collées : une image passe par l'envoi de
  fichiers (étape 5).
- Pas d'écriture automatique à la réception : « Copier » reste le geste.
- Pas de marquage sensible : le champ `sensitive` des éléments reste toujours
  faux, et la règle de l'étape 8 « contenus `sensitive` jamais enregistrés »
  ne s'applique à rien tant qu'aucun marquage n'existe.
- Pas de tuile Android « envoyer le presse-papiers » (note 0016).
- Les lignes macOS, Windows et Ubuntu de la table « Contraintes par
  plateforme » de la conception ne servent plus qu'à une suite éventuelle.
