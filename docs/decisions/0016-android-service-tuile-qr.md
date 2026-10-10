# 0016 — Android : service `specialUse`, pas de tuile, zxing-cpp

**Statut : acceptée** (10 octobre 2026). Tranche D14, D15 et D18 de
`decisions-a-prendre.md`.

**Décision.**

- **D14 — Service de premier plan `specialUse`**, le plus simple aujourd'hui.
  `remoteMessaging` reste une option intéressante (type prévu pour ce
  cas d'usage) : au démarrage de l'étape 3, vérifier sur la doc Android
  qu'il n'a ni limite de durée ni restriction au démarrage du téléphone, et
  noter le résultat ; ne basculer que par une nouvelle note.
- **D15 — Pas de tuile « envoyer le presse-papiers » à l'étape 3.** Le menu
  Partager, ou un copier-coller dans l'appli ou dans l'extension, suffit pour
  l'instant. La tuile reste une piste de l'étape 6.
- **D18 — Lecture du QR par zxing-cpp** (Apache 2.0) avec CameraX, saisie du
  code en secours ; pas de ML Kit.

**Conséquences.** Le plan de l'étape 3 perd la tuile (activité, tâche et
critère de fin) ; le scanner sert à rejoindre le groupe et à parrainer par QR
inversé (note 0013).
