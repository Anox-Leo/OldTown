---
name: delete-comment
description: Trie les commentaires d'un composant (tautologie, périmé, excuse, décision), les supprime, ouvre une issue par excuse, recueille les décisions dans une note de docs/decisions, puis resserre le cliquet du lint. À lancer sur apps/extension (règle local/no-comments) ou sur un autre composant.
---

# /delete-comment

Argument : un dossier ou des fichiers (par défaut `apps/extension`). Le
travail se fait dans un worktree à part, jamais dans la copie principale.

## 1. Inventaire

- Extension : la liste à jour, cliquet compris.

  ```sh
  cd apps/extension
  f=$(mktemp --suffix=.json); echo '{}' > "$f"
  npx eslint . --suppressions-location "$f" --pass-on-unpruned-suppressions
  rm -f "$f"
  ```

  Sans `--suppressions-location`, `npx eslint .` ne montre que les
  commentaires **nouveaux** (hors `eslint-suppressions.json`).
- Ailleurs (Kotlin, Rust, CSS, shell, JSON) : chercher `//`, `/* */`,
  `<!-- -->` et `#` avec un vrai analyseur quand il y en a un. Les
  expressions régulières se trompent sur les chaînes et les URL.
- Fusionner les lignes `//` qui se suivent en un seul bloc et numéroter les
  blocs (`C001`…). Compter les blocs et les lignes.

## 2. Classer chaque bloc

Lire le code autour avant de trancher. Un bloc, une catégorie (celle qui
domine) :

| Catégorie | Reconnaître | Action |
| --- | --- | --- |
| Tautologie | Redit le nom, la signature ou le corps (« Ferme la connexion » sur `close()`), séparateur `// --- État ---` | Supprimer |
| Périmé | Cite une étape, une tâche, une PR, un essai daté ou un fichier qui n'a plus cours, et n'apprend rien d'autre | Supprimer |
| Excuse | Justifie un raccourci : erreur avalée, attente fixe, type écrit à la main, heuristique « faute de mieux », `as unknown as` | Supprimer, et ouvrir une issue |
| Décision | Un choix non évident : pourquoi tel ordre, telle limite, telle permission, la parité avec `ot-core` ou Android, une contrainte de plateforme | Recueillir dans une note, puis supprimer |

Ne jamais toucher :

- les directives (`@ts-check`, `@ts-expect-error`, `eslint-disable…`,
  `@vitest-environment`, `/// <reference>`, `prettier-ignore`) ;
- les fichiers générés (`src/shared/tokens.ts`, `src/tokens.css`,
  `src/wasm/`) : on corrige leur générateur, dans une autre PR.

Écrire le classement dans un fichier de travail (scratchpad), pas dans le
dépôt.

## 3. Supprimer

- Retirer chaque ligne entière du bloc, puis `npx prettier --write` sur les
  fichiers touchés.
- Un `catch {}` ou une fonction vidée échoue au lint (`no-empty`,
  `no-empty-function`). Réécrire le code pour qu'il dise la même chose : une
  fonction qui rend `undefined`, un gestionnaire qui ne sert à rien retiré.
  Pas de commentaire de remplacement, pas d'`eslint-disable`.
- Ne pas changer le comportement. Un raccourci à corriger va dans une issue,
  pas dans cette PR.

## 4. Excuses → issues

Une issue par sujet (`gh issue create -l bug|enhancement`), en anglais :
fichier et symbole, ce que le commentaire excusait, le risque, le correctif
proposé. Jamais de nom de machine, de chemin personnel ni de lien vers une
conversation. Une issue est publique : relire le texte avant de l'envoyer.

## 5. Décisions → note

- Une seule note `docs/decisions/NNNN-titre.md` (prochain numéro libre),
  plus sa ligne dans `docs/decisions/README.md` : contexte, décision (le tri
  et ses comptes, les issues), conséquences, puis les décisions recueillies,
  groupées par fichier, chacune avec le symbole qui suivait le commentaire
  et le commit d'origine.
- Garder le texte d'origine. Corriger seulement une référence périmée
  (fichier renommé…).
- Une décision déjà écrite dans une note existante : renvoyer à cette note
  au lieu de la recopier.
- `docs/` est local et jamais publié : rien de ce qu'on y écrit ne doit être
  recopié dans le dépôt public.

## 6. Resserrer le cliquet

```sh
cd apps/extension
npm run lint:prune   # retire de eslint-suppressions.json ce qui a disparu
npm run lint
```

Le fichier `eslint-suppressions.json` ne fait que rétrécir. Ne jamais
lancer `--suppress-all` ni `--suppress-rule` pour faire passer un
commentaire nouveau. Ces options ne servent qu'à poser le cliquet d'un
nouveau composant.

Pour couvrir un autre composant : y brancher la même règle
(`apps/extension/eslint-rules/no-comments.js`) ou son équivalent (detekt
pour Kotlin, un test pour Rust), poser le cliquet, puis trier.

## 7. Valider et rendre compte

- `npm run lint && npm test && npm run build` dans `apps/extension`, plus
  la validation propre à chaque autre composant touché.
- Compte rendu : blocs et lignes par catégorie, lignes supprimées, issues
  ouvertes (liens), numéro de la note, ce qui reste dans le cliquet et
  pourquoi, et le code réécrit pour passer le lint.
