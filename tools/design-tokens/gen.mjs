#!/usr/bin/env node
// Générateur des jetons de design (note 0059, D50) : lit `design/tokens.json`
// (format DTCG) et écrit les jetons de l'extension, d'Android et le
// nuancier. Node 24, aucune dépendance.
//
//   node tools/design-tokens/gen.mjs            écrit les sorties
//   node tools/design-tokens/gen.mjs --check    échoue si une sortie est périmée
//   --root <dossier>                            racine du dépôt (tests)
//
// Il refuse aussi toute paire de couleurs listée dans `tokens.json`
// (`$extensions["dev.oldtown.contrast"]`) sous 4,5:1 pour du texte ou 3:1
// pour un élément graphique, dans chaque thème.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const DEFAULT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export const SOURCE = 'design/tokens.json';
export const OUTPUTS = {
  css: 'apps/extension/src/tokens.css',
  ts: 'apps/extension/src/shared/tokens.ts',
  kotlin: 'apps/android/app/src/main/kotlin/dev/oldtown/android/ui/theme/Tokens.kt',
  specimen: 'design/specimen.html',
};
export const THEMES = ['dark', 'light'];
export const MIN_RATIO = { text: 4.5, graphic: 3 };
const HEADER = `Fichier généré par tools/design-tokens/gen.mjs depuis ${SOURCE} : ne pas modifier.`;

// ---- Lecture des jetons ----

export function load(root = DEFAULT_ROOT) {
  return JSON.parse(fs.readFileSync(path.join(root, SOURCE), 'utf8'));
}

/** Nœud au chemin `a.b.c`, ou une erreur qui le nomme. */
function at(tokens, dotted) {
  let node = tokens;
  for (const key of dotted.split('.')) {
    node = node?.[key];
    if (node === undefined) throw new Error(`jeton inconnu : ${dotted}`);
  }
  return node;
}

/** Valeur d'un jeton, références `{a.b}` suivies (au plus 10 sauts). */
export function value(tokens, dotted) {
  let node = at(tokens, dotted);
  for (let hops = 0; hops < 10; hops++) {
    const v = node.$value;
    if (v === undefined) throw new Error(`pas de $value : ${dotted}`);
    const ref = typeof v === 'string' && /^\{([^}]+)\}$/.exec(v);
    if (!ref) return v;
    node = at(tokens, ref[1]);
  }
  throw new Error(`références en boucle : ${dotted}`);
}

/** Enfants d'un groupe (clés sans `$`), dans l'ordre du fichier. */
function children(group) {
  return Object.keys(group).filter((k) => !k.startsWith('$'));
}

/** Couleur `#RRGGBB` ou `#RRGGBBAA`, normalisée en minuscules. */
export function hex(tokens, dotted) {
  const v = value(tokens, dotted);
  if (typeof v !== 'string' || !/^#([0-9a-f]{6}|[0-9a-f]{8})$/i.test(v)) {
    throw new Error(`couleur attendue (#RRGGBB ou #RRGGBBAA) : ${dotted} = ${JSON.stringify(v)}`);
  }
  return v.toLowerCase();
}

function px(tokens, dotted) {
  const v = value(tokens, dotted);
  if (v?.unit !== 'px' || !Number.isFinite(v.value)) throw new Error(`dimension en px attendue : ${dotted}`);
  return v.value;
}

/** Rôles d'un thème : `[nom, couleur]`, dans l'ordre du fichier. */
export function roles(tokens, theme) {
  return children(tokens.role[theme]).map((name) => [name, hex(tokens, `role.${theme}.${name}`)]);
}

export function avatars(tokens) {
  return children(tokens.color.avatar).map((k) => hex(tokens, `color.avatar.${k}`));
}

function dims(tokens, group) {
  return children(at(tokens, group)).map((k) => [k, px(tokens, `${group}.${k}`)]);
}

function family(tokens, name) {
  return value(tokens, `type.family.${name}`);
}

// ---- Contrastes (WCAG 2.x) ----

function rgba(color) {
  const n = (i) => parseInt(color.slice(i, i + 2), 16) / 255;
  return [n(1), n(3), n(5), color.length === 9 ? n(7) : 1];
}

function luminance([r, g, b]) {
  const lin = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** Rapport de contraste ; un premier plan translucide est composé sur le fond. */
export function contrast(fg, bg) {
  const b = rgba(bg);
  if (b[3] !== 1) throw new Error(`fond translucide : ${bg}`);
  const f = rgba(fg);
  const mixed = [0, 1, 2].map((i) => f[i] * f[3] + b[i] * (1 - f[3]));
  const [l1, l2] = [luminance(mixed), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

/** Couleurs désignées par un nom de paire dans un thème : un rôle, ou un chemin (`*` final). */
function colorsOf(tokens, ref, theme) {
  if (!ref.includes('.') && theme === 'tous') throw new Error(`rôle sans thème : ${ref}`);
  if (!ref.includes('.')) return [[ref, hex(tokens, `role.${theme}.${ref}`)]];
  if (ref.endsWith('.*')) {
    const group = ref.slice(0, -2);
    return children(at(tokens, group)).map((k) => [`${group}.${k}`, hex(tokens, `${group}.${k}`)]);
  }
  return [[ref, hex(tokens, ref)]];
}

/** Toutes les paires listées, mesurées dans chaque thème. */
export function contrasts(tokens) {
  const spec = tokens.$extensions?.['dev.oldtown.contrast'];
  if (!spec) throw new Error('paires de contraste absentes ($extensions["dev.oldtown.contrast"])');
  const out = [];
  for (const kind of ['text', 'graphic']) {
    for (const [fgRef, bgRef] of spec[kind] ?? []) {
      // Une paire sans rôle ne dépend pas du thème : mesurée une fois.
      const themes = fgRef.includes('.') && bgRef.includes('.') ? ['tous'] : THEMES;
      for (const theme of themes) {
        for (const [fgName, fg] of colorsOf(tokens, fgRef, theme)) {
          for (const [bgName, bg] of colorsOf(tokens, bgRef, theme)) {
            const ratio = contrast(fg, bg);
            out.push({ kind, theme, fg: fgName, bg: bgName, ratio, ok: ratio >= MIN_RATIO[kind] });
          }
        }
      }
    }
  }
  return out;
}

// ---- Sorties ----

const camel = (name) => name.replace(/-([a-z0-9])/g, (_, c) => c.toUpperCase());

/** `#rrggbb[aa]` → `0xAARRGGBB` (Compose). */
export function argb(color) {
  const c = color.slice(1).toUpperCase();
  return `0x${c.length === 8 ? c.slice(6) : 'FF'}${c.slice(0, 6)}`;
}

const cssFamily = (list) => list.map((f) => (f.includes(' ') ? `'${f}'` : f)).join(', ');

function css(tokens) {
  const lines = [`/* ${HEADER} */`, ':root {'];
  for (const [n, c] of roles(tokens, 'dark')) lines.push(`  --${n}: ${c};`);
  for (const [n, v] of dims(tokens, 'radius')) lines.push(`  --radius-${n}: ${v}px;`);
  for (const [n, v] of dims(tokens, 'space')) lines.push(`  --space-${n}: ${v}px;`);
  for (const [n, v] of dims(tokens, 'type.size')) lines.push(`  --text-${n}: ${v}px;`);
  for (const n of children(tokens.type.family)) lines.push(`  --font-${n}: ${cssFamily(family(tokens, n))};`);
  lines.push('  color-scheme: dark;', '}', '');
  // Clair : celui du système, sauf choix contraire (`data-theme` sur
  // `<html>`), ou choisi explicitement.
  const light = roles(tokens, 'light');
  lines.push('@media (prefers-color-scheme: light) {', "  :root:not([data-theme='dark']) {");
  for (const [n, c] of light) lines.push(`    --${n}: ${c};`);
  lines.push('    color-scheme: light;', '  }', '}', '', ":root[data-theme='light'] {");
  for (const [n, c] of light) lines.push(`  --${n}: ${c};`);
  lines.push('  color-scheme: light;', '}', '');
  return lines.join('\n');
}

function ts(tokens) {
  const badge = children(tokens.badge).map((k) => `  ${k}: '${hex(tokens, `badge.${k}`)}',`);
  return [
    `// ${HEADER}`,
    '',
    '/** Couleurs des avatars, dans l\'ordre : la couleur d\'un appareil en dépend (`devices.ts`). */',
    'export const AVATAR_COLORS = [',
    ...avatars(tokens).map((c) => `  '${c}',`),
    '] as const;',
    '',
    '/** Badge de l\'icône (`setBadgeBackgroundColor`) : le même dans les deux thèmes du navigateur. */',
    'export const BADGE = {',
    ...badge,
    '} as const;',
    '',
  ].join('\n');
}

function kotlin(tokens) {
  const theme = (name, doc, t) => [
    `    /** ${doc} */`,
    `    object ${name} {`,
    ...roles(tokens, t).map(([n, c]) => `        val ${camel(n)} = Color(${argb(c)})`),
    '    }',
  ];
  const dimObject = (name, doc, group, unit) => [
    `    /** ${doc} */`,
    `    object ${name} {`,
    ...dims(tokens, group).map(([n, v]) => `        val ${camel(n)} = ${v}.${unit}`),
    '    }',
  ];
  return [
    `// ${HEADER}`,
    'package dev.oldtown.android.ui.theme',
    '',
    'import androidx.compose.ui.graphics.Color',
    'import androidx.compose.ui.unit.dp',
    'import androidx.compose.ui.unit.sp',
    '',
    '/**',
    ' * Jetons du système de design (`design/tokens.json`), mêmes valeurs que',
    ' * l\'extension. `Color.kt` et `Theme.kt` les liront à la place de leurs',
    ' * valeurs (lot G, PR D2).',
    ' */',
    'object OtTokens {',
    ...theme('Dark', 'Rôles du thème sombre « nuit ».', 'dark'),
    '',
    ...theme('Light', 'Rôles du thème clair « parchemin ».', 'light'),
    '',
    '    /** Couleurs des avatars (ARGB), dans l\'ordre : les mêmes que l\'extension. */',
    '    val avatarColors = longArrayOf(',
    ...avatars(tokens).map((c) => `        ${argb(c)},`),
    '    )',
    '',
    ...dimObject('Radius', 'Arrondis : contrôles, cartes, feuilles, pastilles.', 'radius', 'dp'),
    '',
    ...dimObject('Space', 'Espacements, au pas de 4.', 'space', 'dp'),
    '',
    ...dimObject('TextSize', 'Échelle typographique.', 'type.size', 'sp'),
    '}',
    '',
  ].join('\n');
}

const escape = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function specimen(tokens) {
  const vars = (t) => roles(tokens, t).map(([n, c]) => `--${n}: ${c};`).join(' ');
  const radius = dims(tokens, 'radius').map(([n, v]) => `--radius-${n}: ${v}px;`).join(' ');
  const swatches = (t) =>
    roles(tokens, t)
      .map(([n, c]) => `<li><span class="sw" style="background: var(--${n})"></span><b>${n}</b><code>${c}</code></li>`)
      .join('\n        ');
  const avatarRow = avatars(tokens)
    .map((c, i) => `<span class="av" style="background: ${c}" title="avatar ${i} ${c}">${i}</span>`)
    .join('');
  const pairs = contrasts(tokens)
    .map(
      (p) =>
        `<tr><td>${p.theme}</td><td>${p.kind === 'text' ? 'texte' : 'graphique'}</td><td>${escape(p.fg)}</td><td>${escape(p.bg)}</td><td>${p.ratio.toFixed(2)}</td></tr>`,
    )
    .join('\n      ');
  const panel = (t, title, logo) => `
    <section class="theme" style="${vars(t)}">
      <h2><img src="../branding/${logo}" alt="OldTown" height="28"> <span>${title}</span></h2>
      <div class="demo">
        <div class="row card"><span class="av mark"><img src="../branding/archivolte-signe.svg" alt="" width="22"></span><span class="grow"><b>Tous mes appareils</b><br><span class="muted">Lien : article</span></span><span class="pill">2</span></div>
        <div class="row card"><span class="av" style="background: ${avatars(tokens)[5]}"></span><span class="grow"><b>Téléphone</b><br><span class="muted">Code 482 913</span></span></div>
        <p class="bubble out">Le code arrive dans la page.</p>
        <p class="bubble in">Bien reçu, merci <a href="#">lien</a></p>
        <p class="row"><button class="primary">Envoyer</button><button>Annuler</button><button class="danger">Oublier</button></p>
        <p class="banner warning">Avertissement</p>
        <p class="banner danger">Erreur</p>
      </div>
      <ul class="swatches">
        ${swatches(t)}
      </ul>
    </section>`;
  return `<!doctype html>
<!-- ${HEADER} -->
<html lang="fr">
<head>
<meta charset="utf-8">
<title>OldTown · nuancier</title>
<style>
  @font-face { font-family: 'DM Sans'; src: url('../apps/extension/public/fonts/dm-sans.ttf') format('truetype'); font-weight: 100 1000; }
  body { margin: 0; font: 14px/1.45 ${cssFamily(family(tokens, 'sans'))}; background: #888; }
  main { display: grid; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr)); }
  .theme { ${radius} background: var(--screen); color: var(--fg); padding: 24px; }
  h2 { display: flex; align-items: center; gap: 12px; margin: 0 0 16px; font-size: 16px; font-weight: 500; color: var(--muted); }
  .demo { display: grid; gap: 8px; margin-bottom: 24px; }
  .row { display: flex; gap: 12px; align-items: center; margin: 0; }
  .grow { flex: 1; }
  .card { background: var(--card); border: 1px solid var(--border); border-radius: var(--radius-card); padding: 10px 12px; }
  .muted { color: var(--muted); }
  .av { width: 40px; height: 40px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; color: var(--on-avatar); font-size: 12px; }
  .av.mark { background: var(--tile); }
  .pill { background: var(--primary); color: var(--on-primary); border-radius: var(--radius-pill); padding: 0 7px; font-size: 12px; }
  .bubble { margin: 0; padding: 8px 12px; border-radius: var(--radius-card); max-width: 70%; }
  .bubble.out { justify-self: end; background: var(--bubble-out); }
  .bubble.in { background: var(--grouped); border: 1px solid var(--border); }
  a { color: var(--primary-text); }
  button { font: inherit; color: var(--fg); background: var(--card); border: 1px solid var(--input-border); border-radius: var(--radius-control); padding: 7px 12px; }
  button.primary { background: var(--primary); border-color: var(--primary); color: var(--on-primary); }
  button.danger { color: var(--danger-fg); border-color: var(--danger-border); }
  .banner { margin: 0; padding: 8px 10px; border-radius: var(--radius-control); }
  .banner.warning { background: var(--warning); border: 1px solid var(--warning-border); color: var(--warning-fg); }
  .banner.danger { background: var(--danger); border: 1px solid var(--danger-border); color: var(--danger-fg); }
  .swatches { list-style: none; padding: 0; margin: 0; display: grid; gap: 4px; }
  .swatches li { display: grid; grid-template-columns: 28px 1fr auto; gap: 8px; align-items: center; }
  .sw { width: 28px; height: 20px; border-radius: 6px; border: 1px solid var(--border); }
  code { font-family: ${cssFamily(family(tokens, 'mono'))}; font-size: 12px; color: var(--muted); }
  .shared { background: #fff; color: #222; padding: 24px; }
  .shared table { border-collapse: collapse; font-size: 12px; }
  .shared td { padding: 2px 10px 2px 0; }
</style>
</head>
<body>
<main>${panel('dark', 'Sombre « nuit »', 'logotype-sombre.svg')}${panel('light', 'Clair « parchemin »', 'logotype-clair.svg')}
</main>
<section class="shared">
  <h2>Avatars</h2>
  <p>${avatarRow}</p>
  <h2>Contrastes vérifiés</h2>
  <table>
    <tbody>
      ${pairs}
    </tbody>
  </table>
</section>
</body>
</html>
`;
}

/** Contenu attendu de chaque sortie, par chemin relatif à la racine. */
export function render(tokens) {
  const [dark, light] = THEMES.map((t) => roles(tokens, t).map(([n]) => n).join());
  if (dark !== light) throw new Error('les deux thèmes doivent avoir les mêmes rôles, dans le même ordre');
  return {
    [OUTPUTS.css]: css(tokens),
    [OUTPUTS.ts]: ts(tokens),
    [OUTPUTS.kotlin]: kotlin(tokens),
    [OUTPUTS.specimen]: specimen(tokens),
  };
}

/** Sorties absentes ou différentes de ce que donnent les jetons. */
export function stale(root, files) {
  return Object.entries(files)
    .filter(([rel, content]) => {
      try {
        return fs.readFileSync(path.join(root, rel), 'utf8') !== content;
      } catch (e) {
        if (e.code === 'ENOENT') return true;
        throw e;
      }
    })
    .map(([rel]) => rel);
}

function main(argv) {
  let root = DEFAULT_ROOT;
  let check = false;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--check') check = true;
    else if (argv[i] === '--root') root = path.resolve(argv[++i] ?? '');
    else {
      console.error(`option inconnue : ${argv[i]}\nusage : gen.mjs [--check] [--root <dossier>]`);
      return 2;
    }
  }
  const tokens = load(root);
  const failed = contrasts(tokens).filter((p) => !p.ok);
  for (const p of failed) {
    console.error(
      `contraste insuffisant (${p.theme}, ${p.kind}) : ${p.fg} sur ${p.bg} = ${p.ratio.toFixed(2)}:1, minimum ${MIN_RATIO[p.kind]}:1`,
    );
  }
  if (failed.length) return 1;
  const files = render(tokens);
  const old = stale(root, files);
  if (check) {
    for (const rel of old) console.error(`périmé : ${rel} (lancer node tools/design-tokens/gen.mjs)`);
    return old.length ? 1 : 0;
  }
  for (const rel of old) {
    fs.mkdirSync(path.dirname(path.join(root, rel)), { recursive: true });
    fs.writeFileSync(path.join(root, rel), files[rel]);
    console.log(`écrit : ${rel}`);
  }
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}
