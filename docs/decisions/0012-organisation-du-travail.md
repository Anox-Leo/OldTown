# 0012 — Organisation du travail et ordre des jalons

**Statut : acceptée** (10 octobre 2026). Tranche D1 à D4 de
`decisions-a-prendre.md`. L'ordre des jalons (D4) est remplacé par
[0017](0017-extension-chrome-sans-appli-desktop.md).

**Contexte.** La [revue après l'étape 1](../revue-etapes-0-1.md) a posé quatre
questions d'organisation avant de planifier les étapes 2 à 8 : où vit la
documentation, quel niveau de détail pour les plans, quel format pour
l'orchestrateur d'agents, et dans quel ordre livrer les jalons.

**Décision.**

- **D1 — Documentation** : dans le dépôt (`docs/`, `CLAUDE.md`), pas dans des
  fichiers à part.
- **D2 — Profondeur des plans** : même trame pour toutes les étapes ; plans
  détaillés pour les étapes 2 et 3, plans de cadrage pour les étapes 4 à 8,
  relus contre le code au démarrage de chaque étape.
- **D3 — Format pour les agents** : un `CLAUDE.md` à la racine et un par crate
  ou appli ; des tâches numérotées et autonomes dans `etape-N.md`. Pas de
  format propre à un orchestrateur.
- **D4 — Ordre des jalons** : inchangé (2 chiffrement, 3 Android, 4 desktop,
  5 fichiers, 6 presse-papiers, 7 extension, 8 confort).

**Conséquences.** Un agent trouve tout dans le dépôt. Les plans de cadrage
(étapes 4 à 8) imposent une relecture au démarrage, avec une note de décision
pour chaque écart.
