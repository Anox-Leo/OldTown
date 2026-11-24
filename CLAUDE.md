# OldTown — consignes pour les agents

Système maison d'échange de textes, liens, fichiers et codes à usage unique
entre mes appareils (Android, navigateur, terminal) et avec les personnes
que j'invite, via un relais auto-hébergé, joignable par Tailscale ou
publiquement derrière un frontal TLS. Le contenu est chiffré de bout en
bout : le relais ne voit jamais le clair.

Réponses et documentation interne en français ; `README.md`, `SECURITY.md`, identifiants, messages de commit et noms de
branches en anglais.

Le dépôt est public : `README.md` (simplifié) et le code en sont la
documentation. `docs/` reste **local, ignoré par git, jamais publié** ; il
n'y a pas d'autre mémoire. Si une information manque, la demander plutôt
que la deviner.

## Où regarder

| Fichier | Contenu | Quand le lire |
| --- | --- | --- |
| `docs/systeme/` | L'état actuel, un fichier par sujet (`README.md` : la liste) | Avant toute tâche : le sujet touché, et `vue-d-ensemble.md` |
| `docs/chantiers/` | Travail en cours : conception, tâches, un seul paragraphe d'état | Si ta tâche fait partie d'un chantier |
| `docs/feuille-de-route.md` | Ce qui est prévu, par version | Pour situer une tâche |
| `docs/decisions/` | Notes numérotées : décisions, écarts et précisions | Toujours, avant de coder (celles du sujet touché) |
| `docs/decisions-a-prendre.md` | Questions ouvertes (D1…), avec la recommandation appliquée par défaut | Si ta tâche touche un sujet listé |
| `docs/actions-manuelles.md` | Actions que je dois faire à la main sur mes machines, dans l'ordre | Dès qu'une tâche exige une action de ma part |
| `docs/archive/` | Plans d'étape et conception d'origine, figés | Seulement pour l'historique d'une note ; ne pas s'y fier pour l'état actuel |
| `SECURITY.md` | Modèle de menace, ce qui est en place, limites connues | Toute tâche qui touche auth, crypto, stockage, réseau |
| `crates/*/CLAUDE.md`, `apps/*/CLAUDE.md`, `tools/*/CLAUDE.md` | Règles propres à chaque composant | Avant de modifier ce composant |

**Ordre de priorité** en cas de contradiction : le code et
`docs/decisions/` font foi, puis `docs/systeme/`, puis le chantier, puis
l'archive.

## Prendre une tâche

1. Lire le sujet de `docs/systeme/` concerné, les notes de décision qu'il
   cite et le `CLAUDE.md` des composants touchés. Pour une tâche de
   chantier : sa section (**Fichiers**, **Faire**, **Tests**, **Fini
   quand**).
2. Relire contre le code ce qu'on s'apprête à changer ; un écart avec la
   documentation se note avant de coder (dans le chantier, ou dans une note
   s'il engage la suite).
3. Rester dans le périmètre de la tâche. Un problème découvert ailleurs se
   signale dans le compte rendu, il ne s'élargit pas en douce.
4. Écrire les tests demandés avec le code, pas après.
5. Valider (section plus bas). Mettre à jour le sujet de `docs/systeme/`
   si le comportement change, cocher la tâche du chantier et réécrire son
   paragraphe « État » (un seul, sans identifiant de commit). Rendre
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
  format chiffré par les vecteurs de `ot-crypto` : un changement de
  format est délibéré et accompagné d'une note de décision.
- Jamais de contenu de message, de clé ni de jeton dans les logs.
- Un seul processus tient la connexion au relais sur chaque appareil (note
  0009) ; les autres points d'entrée passent par lui.
- Crypto : primitives standard de RustCrypto uniquement, jamais de
  construction maison sans note de décision ; secrets effacés de la mémoire
  (`zeroize`).
- Ne pas capturer d'erreur sans la traiter ou la remonter ; pas de `unwrap()`
  hors tests : un invariant se dit dans le message d'un `expect`.
- **Pas de commentaires** dans le code (notes 0066, 0072 et 0077) : le
  pourquoi va dans une note de `docs/decisions/`, le quoi dans les noms et
  les tests, un raccourci dans une issue. Des tests-cliquets échouent sur
  tout commentaire : `crates/ot-proto/tests/no_comments.rs` (Rust, TOML,
  `deploy/`, scripts et unités d'`otp-relay`),
  `tools/otp-relay/tests/test_no_comments.py` (Python, docstrings
  comprises), `tools/design-tokens/no-comments.test.mjs`, la règle ESLint
  `local/no-comments` (extension) et `NoCommentsTest` (Android). Restent
  permis : les directives (`# shellcheck`, `@ts-expect-error`…), les
  migrations SQL (sqlx vérifie leur somme de contrôle : jamais retouchées)
  et le modèle `tools/otp-relay/contrib/config.example.toml`. L'aide des
  CLI s'écrit en attributs clap (`#[command(about = …)]`,
  `#[arg(help = …)]`), pas en `///`. Pour trier : `/delete-comment`.
- Nouvelle dépendance : justifiée, licence compatible avec `deny.toml`.
- Toute action que je dois faire moi-même sur mes machines (installation,
  commande sur le relais, réglage d'un appareil) va dans
  `docs/actions-manuelles.md`, étape par étape et dans l'ordre, avec les
  commandes exactes ; le compte rendu y renvoie au lieu de la détailler.

## Décisions

- Une décision qui engage la suite, ou un écart voulu à `docs/systeme/`,
  donne une note `docs/decisions/NNNN-titre-court.md` (contexte, décision,
  conséquences) et une ligne dans `docs/decisions/README.md`. Les écarts
  trouvés en codant et les relectures de PR vont dans le chantier, pas
  dans une note.
- **Ne jamais trancher seul une question de `docs/decisions-a-prendre.md`.**
  Appliquer la recommandation (★) et le dire dans le compte rendu. Quand je
  tranche, la question devient une note numérotée et quitte ce fichier.

## Valider avant de rendre la main

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

Plus, selon les dossiers touchés : `./gradlew lint test assembleDebug` dans
`apps/android`, `npm run lint && npm test` dans `apps/extension`,
`python3 -m unittest discover -s tests -t .` dans `tools/otp-relay`, `node tools/design-tokens/gen.mjs --check` et
`node --test 'tools/design-tokens/*.test.mjs'` si `design/` ou une couleur
change. Une commande qui ne passe pas se signale avec
sa sortie ; jamais de test désactivé pour passer.

## Compte rendu attendu

1. Résumé des changements (fichiers, raisons).
2. Validation exécutée et résultat.
3. Décisions appliquées par défaut, notes ajoutées, risques et suivi.

## Git

Commits au format Conventional Commits, en anglais, seulement sur demande.
Jamais de lien vers une conversation Claude dans un commit, une PR ou un
commentaire.
