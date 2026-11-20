# tools/design-tokens

Générateur des jetons de design (note 0059, D50). Source unique :
`design/tokens.json` (format DTCG). Sorties, **jamais modifiées à la main**
(en-tête « fichier généré ») :

- `apps/extension/src/tokens.css` : rôles en variables CSS (`:root` sombre,
  `@media (prefers-color-scheme: light)` clair), arrondis, espacements,
  tailles, polices ;
- `apps/extension/src/shared/tokens.ts` : ce que le TypeScript lit en
  valeur (`AVATAR_COLORS`, `BADGE`) ;
- `apps/android/.../ui/theme/Tokens.kt` : `object OtTokens` (`Dark`,
  `Light`, `avatarColors`, `Radius`, `Space`, `TextSize`) ;
- `design/specimen.html` : nuancier des deux thèmes et table des contrastes.

Règles :

- **Node 24, aucune dépendance** (ni Style Dictionary ni autre) ; module
  ES, fonctions exportées pour les tests.
- Un jeton changé : modifier `tokens.json`, lancer `gen.mjs`, versionner
  source et sorties ensemble.
- **Contrastes** : chaque paire de `$extensions["dev.oldtown.contrast"]`
  est mesurée dans les deux thèmes (texte 4,5:1, graphique 3:1) ; une seule
  en dessous et rien n'est écrit. Un rôle posé sur un nouveau fond ajoute sa
  paire.
- Les deux thèmes ont les mêmes rôles, dans le même ordre ; l'ordre des
  avatars ne change jamais (la couleur d'un appareil en dépend, extension et
  Android).
- Noms Kotlin : rôles en camelCase (`input-border` → `inputBorder`).

Valider :

```sh
node tools/design-tokens/gen.mjs --check
node --test 'tools/design-tokens/*.test.mjs'
```
