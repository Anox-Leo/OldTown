# 0010 — Monorepo, licence MIT, publication après l'étape 2

**Statut : la date de publication est remplacée par [0011](0011-publication-fin-de-projet.md)** ; le reste tient.

**Contexte.** Le projet commence dans un dépôt GitHub privé (Q6) et doit
devenir public. L'historique git ne s'efface pas : ce qui y entre sera publié.

**Décision.**
- Un seul dépôt pour le serveur, le cœur et les futures applis.
- Licence MIT.
- Aucune donnée propre à mon installation dans le dépôt (noms de machine, IP,
  réglages du laptop) : elles vivent dans `private/`, ignoré par git, et dans
  des surcharges locales (`systemctl edit`).
- Publication au plus tôt après l'étape 2 (chiffrement de bout en bout), une
  fois la liste de `SECURITY.md` cochée.

**Conséquences.** Les plans d'installation personnels de l'étape 0 ne sont pas
versionnés. Les exemples du dépôt utilisent le nom d'hôte `relais`.
