# OldTown design system

One source, generated outputs. `tokens.json` (W3C design tokens format,
`$value` / `$type`) holds every color, radius, spacing and type size used by
the browser extension and the Android app. Nothing else is edited by hand.

```sh
node tools/design-tokens/gen.mjs            # write the outputs
node tools/design-tokens/gen.mjs --check    # fail if an output is stale (CI)
node --test 'tools/design-tokens/*.test.mjs'
```

| Output | Used by |
| --- | --- |
| `apps/extension/src/tokens.css` | Popup, settings and pages: CSS variables, dark by default, light under `prefers-color-scheme: light` |
| `apps/extension/src/shared/tokens.ts` | Values read from TypeScript: avatar colors, toolbar badge |
| `apps/android/.../ui/theme/Tokens.kt` | `OtTokens`: same roles, avatars, radii, spacing and type sizes for Compose |
| `design/specimen.html` | Swatches, buttons, rows, bubbles and the contrast table, both themes (open it locally) |

Node 24, no dependency: a hundred tokens do not need a build tool.

## Tokens

- **Primitives** (`color.*`): three families. *Nuit*, blue-tinted darks;
  *parchemin*, warm lights; *pierre*, the brand color (the mark, the main
  action, unread counts). Plus white and ten avatar colors.
- **Roles** (`role.dark.*`, `role.light.*`): what the apps actually use
  (`screen`, `card`, `fg`, `muted`, `primary`…). Both themes define the
  same roles in the same order; a role points to a primitive whenever one
  fits. Success, warning and danger keep plain functional colors: the brand
  never imitates them, and they are never decoration.
- **Badge** (`badge.*`): the extension's toolbar badge, the same in both
  browser themes.
- **Radius**: 10 (controls), 14 (cards, bubbles), 20 (sheets), 999 (pills).
- **Space**: steps of 4 (4, 8, 12, 16, 24, 32).
- **Type**: DM Sans; sizes 11, 13, 15, 17, 20, 24, plus 14 for the
  extension popup body.

## Rules

- Use a role, never a primitive or a literal color, in app code. The
  extension has a test that rejects any hex color in `src/` outside the
  generated files and the QR code (always black on white, for scanners).
- Text pairs must reach 4.5:1, graphical elements 3:1 (WCAG 2.x). The pairs
  are listed in `tokens.json` (`$extensions["dev.oldtown.contrast"]`); the
  generator measures each one in both themes and refuses to write anything
  when one fails. Add a pair whenever a role is used on a new background.
- Avatar colors keep their order: a device's color is picked from its id by
  index, so reordering would repaint every device. Each one carries a white
  pictogram at 3:1 or more.
- The primary color differs per theme on purpose: white on `pierre-400`
  is only 3.3:1, so the light theme uses `pierre-500` behind white text.
- No gradient, shadow or glow; nothing animated except loading.

## Do / don't

| Do | Don't |
| --- | --- |
| `background: var(--card)` | `background: #161a2e` |
| `OtTokens.Dark.primary` (through the theme) | `Color(0xFFD27443)` in a screen |
| Add a role when a new meaning appears | Reuse `danger` or `primary` as decoration |
| Change `tokens.json`, regenerate, commit both | Edit a generated file |

The mark, logotype and icons are in [`branding/`](../branding/).
