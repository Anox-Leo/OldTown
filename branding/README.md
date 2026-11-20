# OldTown branding

The mark is a pointed arch in stone (`#D27443`) pierced by two lancet
windows and ringed by an archivolt, on a night tile (`#232841`). Two colors,
flat, nothing else: no gradient, shadow or glow. The logotype sets
"OldTown" in DM Sans semibold, outlined, so no font has to be loaded.

## Files

| File | Use |
| --- | --- |
| `archivolte-icone.svg` | Icon on its tile: extension 48 and 128 px, README |
| `archivolte-petit-icone.svg` | Icon on its tile, optical size: extension 16 and 32 px |
| `archivolte-signe.svg` | Mark alone, tight box, stone |
| `archivolte-mono.svg` | Mark in one color (`currentColor`) |
| `archivolte-petit.svg`, `archivolte-petit-mono.svg` | Optical size without tile |
| `android-foreground.svg` | Adaptive icon foreground (108 dp; background `#232841`) |
| `android-monochrome.svg` | Android 13+ themed icon |
| `android-notification.svg` | Notification icon (24 dp, white) |
| `logotype-clair.svg`, `logotype-sombre.svg`, `logotype-mono.svg` | Mark and name, for light and dark backgrounds, or one color |
| `apercu-github.svg`, `apercu-github.png` | Repository social preview, 1280 × 640 |
| `charte.mjs` | Draws every SVG above from the master geometry |

The extension icons (`apps/extension/public/icons/icon-{16,32,48,128}.png`)
and `apercu-github.png` are exported once from these SVGs and committed:
there is no export step in any build.

## Sizes

- **48 px and up**: full mark.
- **32 px and below**: optical size (thicker archivolt, solid core). Browsers
  show the 32 px image in the toolbar on high-density screens.
- **Without a tile** (toolbars, headers): the mark alone, in stone on light
  and dark backgrounds alike; optical size under 24 px.
- **Minimum**: 16 px for the icon, 12 px for the bare mark, 20 px high for
  the logotype (below that, use the mark alone).

## Clear space

Keep at least a quarter of the mark's height free around it, and around the
logotype (the gap between mark and name is the same quarter). On Android,
the mark is 42 dp high on the 108 dp layer, inside the 66 dp safe zone, so
it survives circle, squircle and rounded-square masks; on the 128 tile it is
74.67 high, lifted by 1.78.

## Colors

The mark is always stone. The name is `#232841` on light backgrounds and
`#EEECF3` on dark ones; the mono variants take the surrounding text color.
Stone on the night tile is 4.35:1, above the 3:1 required for graphics.
Every value comes from [`design/tokens.json`](../design/tokens.json).

## Redrawing

`charte.mjs` needs `fontkit` (pinned here, not part of any app build) and
the DM Sans font shipped with the extension:

```sh
cd branding
npm ci
npm run draw        # node charte.mjs . ../apps/extension/public/fonts/dm-sans.ttf
```

It rewrites the SVGs in place. Export the PNGs again with any SVG
rasterizer (for instance `@resvg/resvg-js`), from `archivolte-petit-icone.svg`
for 16 and 32 px and `archivolte-icone.svg` for 48 and 128 px, then run the
extension tests: `test/design.test.ts` checks the icon sizes and that the
paths in `src/app/brand.ts` match these files.
