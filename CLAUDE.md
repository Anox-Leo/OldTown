# OldTown — consignes pour les agents

Système maison d'échange de textes, liens, fichiers et presse-papiers entre mes
appareils (Android, Mac, Windows, Ubuntu, navigateur), via un relais
auto-hébergé sur un laptop Ubuntu, joignable seulement par Tailscale. Le relais
ne voit jamais le contenu en clair (à partir de l'étape 2).

Réponses, documentation interne et commentaires de code en français ;
`README.md`, `SECURITY.md`, identifiants, messages de commit et noms de
branches en anglais.

Le dépôt est public : `README.md` (simplifié) et le code en sont la
documentation. `docs/` (conception, plans, notes de décision, actions
manuelles, README détaillé en français `docs/readme-fr.md`) reste **local,
ignoré par git, jamais publié** ; il n'y a pas d'autre mémoire. Si une
information manque, la demander plutôt que la deviner.

## Où regarder

| Fichier | Contenu | Quand le lire |
| --- | --- | --- |
| `docs/design.md` | Conception, architecture, feuille de route (jalons 1 à 8) | Pour comprendre le pourquoi |
| `docs/etape-N.md` | Plan de l'étape N : périmètre, conception, tâches, critères de fin | Avant toute tâche de l'étape N |
| `docs/decisions/` | Notes numérotées : écarts et précisions par rapport aux plans | Toujours, avant de coder |
| `docs/decisions-a-prendre.md` | Questions ouvertes (D1…), avec la recommandation appliquée par défaut | Si ta tâche touche un sujet listé |
| `docs/actions-manuelles.md` | Actions que je dois faire à la main sur mes machines, dans l'ordre | Dès qu'une tâche exige une action de ma part |
| `docs/revue-etapes-0-1.md` | Ce que l'étape 1 a appris, et les choix revus pour la suite | Au démarrage d'une étape |
| `SECURITY.md` | Modèle de menace, liste avant publication | Toute tâche qui touche auth, crypto, stockage, réseau |
| `crates/*/CLAUDE.md`, `apps/*/CLAUDE.md` | Règles propres à chaque composant | Avant de modifier ce composant |

**Ordre de priorité** en cas de contradiction : le code et `docs/decisions/`
font foi, puis le plan de l'étape, puis `design.md`.

## Prendre une tâche

1. Lire la tâche dans `docs/etape-N.md` (champs **Fichiers**, **Faire**,
   **Tests**, **Fini quand**), les notes de décision récentes et le
   `CLAUDE.md` des composants concernés.
2. Si le plan est un plan de cadrage (étapes 4 à 8), commencer par le relire
   contre le code : compléter la tâche, et noter tout écart dans
   `docs/decisions/` avant de coder.
3. Rester dans le périmètre de la tâche. Un problème découvert ailleurs se
   signale dans le compte rendu, il ne s'élargit pas en douce.
4. Écrire les tests demandés avec le code, pas après.
5. Valider (section suivante), cocher la tâche dans `etape-N.md`, rendre
   compte.

## Règles

- Le dépôt est public : jamais de nom de machine, IP, nom de tailnet, nom de
  domaine personnel, adresse e-mail, chemin personnel ou secret dans les
  fichiers versionnés ni dans les commits. Exemple de
  nom d'hôte : `relais`. Les réglages personnels vont dans `private/` (ignoré
  par git).
- `ot-server` ne dépend que de `ot-proto` et ne lit jamais `payload`. Il ne
  dépendra jamais de `ot-crypto`.
- Le format du protocole est figé par `crates/ot-proto/tests/golden/`, le
  format chiffré par les vecteurs de `ot-crypto` (étape 2) : un changement de
  format est délibéré et accompagné d'une note de décision.
- Jamais de contenu de message, de clé ni de jeton dans les logs.
- Un seul processus tient la connexion au relais sur chaque appareil (note
  0009) ; les autres points d'entrée passent par lui.
- Crypto : primitives standard de RustCrypto uniquement, jamais de
  construction maison sans note de décision ; secrets effacés de la mémoire
  (`zeroize`).
- Ne pas capturer d'erreur sans la traiter ou la remonter ; pas de `unwrap()`
  hors tests et invariants commentés.
- Nouvelle dépendance : justifiée, licence compatible avec `deny.toml`.
- Toute action que je dois faire moi-même sur mes machines (installation,
  commande sur le relais, réglage d'un appareil) va dans
  `docs/actions-manuelles.md`, étape par étape et dans l'ordre, avec les
  commandes exactes ; le compte rendu y renvoie au lieu de la détailler.

## Décisions

- Tout écart par rapport à un plan, ou toute précision qui engage la suite,
  donne une note `docs/decisions/NNNN-titre-court.md` (contexte, décision,
  conséquences) et une ligne dans `docs/decisions/README.md`.
- **Ne jamais trancher seul une ligne de `docs/decisions-a-prendre.md`.**
  Appliquer la recommandation (★) et le dire dans le compte rendu. Quand je
  tranche, la ligne devient une note numérotée et quitte ce fichier.

## Valider avant de rendre la main

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

Plus, selon les dossiers touchés : `./gradlew lint test assembleDebug` dans
`apps/android` (étape 3), `npm run lint && npm test` dans `apps/extension`
(étape 4). Une commande qui ne passe pas se signale avec
sa sortie ; jamais de test désactivé pour passer.

## Compte rendu attendu

1. Résumé des changements (fichiers, raisons).
2. Validation exécutée et résultat.
3. Décisions appliquées par défaut, notes ajoutées, risques et suivi.

## Git

Commits au format Conventional Commits, en anglais, seulement sur demande.
Jamais de lien vers une conversation Claude dans un commit, une PR ou un
commentaire.
