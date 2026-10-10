# 0019 — Presse-papiers : envoi manuel, écriture à la réception

**Statut : acceptée** (10 octobre 2026). Tranche D24 de
`decisions-a-prendre.md`.

**Contexte.** L'étape 6 prévoyait une appli desktop qui surveille le
presse-papiers de l'OS. Il n'y a plus d'appli desktop (note 0017). Une
extension Chrome ne peut lire le presse-papiers qu'au moment d'un geste, ou en
sondant depuis un document offscreen, sans voir les marqueurs posés par les
gestionnaires de mots de passe.

**Décision.**

- Pas de surveillance automatique du presse-papiers, nulle part : un contenu
  ne part que sur un geste (bouton ou raccourci de l'extension, menu Partager
  d'Android).
- À la réception, un contenu peut être écrit directement dans le
  presse-papiers (réglage par appareil, désactivé par défaut) ; sinon il
  arrive en notification avec « Copier ».
- L'étape 6 devient une petite étape : modèle `clip`, images collées,
  écriture automatique à la réception, marquage sensible, et éventuellement
  la tuile Android (note 0016).

**Conséquences.**

- Aucun risque d'envoyer un mot de passe copié sans le vouloir, y compris sur
  un ordinateur professionnel.
- Pas de « copier ici, coller là-bas » sans geste : il faut cliquer ou
  utiliser le raccourci avant de changer d'appareil.
- Les filtres par OS et l'anti-boucle du plan initial disparaissent : un
  contenu écrit à la réception n'est jamais renvoyé automatiquement.
