// Dessin final du signe « archivolte » et de ses déclinaisons.
import fs from 'node:fs';
import * as fontkit from 'fontkit';
const [OUT, FONT] = process.argv.slice(2); // dossier de sortie, chemin de dm-sans.ttf
const NIGHT = '#232841', STONE = '#D27443';
const r2 = n => +n.toFixed(2);

// ---- Géométrie maîtresse (grille de 128, unités entières pour les droites) ----
// Ogive : flancs x = 18 et 110, naissance y = 80, arcs de rayon 60 (centres x = 50 et 78).
// Archivolte 8, joint 6, noyau à 14 ; murs 8, meneau 6, appui 8 ; lancettes de 21.
const SPRING = 80, BASE = 118;
const ogive = d => { const r = 60 - d, ay = SPRING - Math.sqrt(r * r - 196);
  return `M${18 + d} ${BASE}V${SPRING}A${r} ${r} 0 0 1 64 ${r2(ay)}A${r} ${r} 0 0 1 ${110 - d} ${SPRING}V${BASE}z`; };
const lancet = (x0, w, spring, bottom) => `M${x0} ${bottom}V${spring}A${w} ${w} 0 0 1 ${r2(x0 + w / 2)} ${r2(spring - Math.sqrt(3) / 2 * w)}A${w} ${w} 0 0 1 ${x0 + w} ${spring}V${bottom}z`;
const MARK = [ogive(0), ogive(8), ogive(14), lancet(40, 21, 86, 110), lancet(67, 21, 86, 110)].join('');
// Taille optique : archivolte 14, joint 8, noyau plein.
const SMALL = [ogive(0), ogive(14), ogive(22)].join('');
const BOX = { x: 18, y: SPRING - Math.sqrt(3600 - 196), w: 92, h: BASE - (SPRING - Math.sqrt(3600 - 196)) };

// Place le signe (boîte BOX) à la hauteur h, centré en (cx, cy).
const place = (h, cx, cy) => { const k = h / BOX.h; return `translate(${r2(cx - k * BOX.w / 2)} ${r2(cy - k * BOX.h / 2)}) scale(${r2(k * 1e4) / 1e4}) translate(${-BOX.x} ${r2(-BOX.y)})`; };

// ---- Proportions reprises de l'icône Android actuelle ----
// ic_launcher_foreground : flèches de 41 × 37 dp sur 108, dans la zone visible de 72.
// Le signe y prend 42 dp de haut (58 % de la zone visible) ; remonté de 1 dp (pointe légère en haut).
const ANDROID_H = 42, ANDROID_LIFT = 1;
const TILE_H = r2(128 * ANDROID_H / 72), TILE_LIFT = r2(128 * ANDROID_LIFT / 72);

const svg = (vb, body, comment) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${vb}" role="img" aria-labelledby="t">\n  <title id="t">OldTown</title>\n  <!-- ${comment} -->\n${body}</svg>\n`;
const markPath = (d, fill, tr) => `  <path fill="${fill}" fill-rule="evenodd"${tr ? ` transform="${tr}"` : ''} d="${d}"/>\n`;
const tight = `${BOX.x} ${r2(BOX.y)} ${BOX.w} ${r2(BOX.h)}`;

const files = {
  'archivolte-signe.svg': svg(tight, markPath(MARK, STONE), `Signe seul, pierre ${STONE}, boîte serrée. Fonds clairs et sombres.`),
  'archivolte-mono.svg': svg(tight, markPath(MARK, 'currentColor'), 'Signe en une couleur (currentColor).'),
  'archivolte-petit.svg': svg(tight, markPath(SMALL, STONE), 'Taille optique, 24 px et moins : archivolte 14, joint 8, noyau plein.'),
  'archivolte-petit-mono.svg': svg(tight, markPath(SMALL, 'currentColor'), 'Taille optique en une couleur (notification Android).'),
  'archivolte-icone.svg': svg('0 0 128 128', `  <rect width="128" height="128" rx="28" fill="${NIGHT}"/>\n` + markPath(MARK, STONE, place(TILE_H, 64, 64 - TILE_LIFT)),
    `Icône : tuile nuit ${NIGHT}, signe de ${TILE_H} de haut (proportions de l'icône Android).`),
  'archivolte-petit-icone.svg': svg('0 0 128 128', `  <rect width="128" height="128" rx="28" fill="${NIGHT}"/>\n` + markPath(SMALL, STONE, place(88, 64, 64 - TILE_LIFT)),
    'Icône en taille optique (16 et 32 px) : signe agrandi à 88 pour gagner des pixels.'),
  'android-foreground.svg': svg('0 0 108 108', markPath(MARK, STONE, place(ANDROID_H, 54, 54 - ANDROID_LIFT)),
    `Avant-plan de l'icône adaptative (108 dp, fond ${NIGHT}) : signe de ${ANDROID_H} dp.`),
  'android-monochrome.svg': svg('0 0 108 108', markPath(MARK, '#FFFFFF', place(ANDROID_H, 54, 54 - ANDROID_LIFT)), 'Icône thématique Android 13.'),
  'android-notification.svg': svg('0 0 24 24', markPath(SMALL, '#FFFFFF', place(20, 12, 12)), 'Petite icône de notification (24 dp, blanc).'),
};

// ---- Logotype : DM Sans, wght 600, opsz 40, vectorisé ----
const font = fontkit.openSync(FONT).getVariation({ wght: 600, opsz: 40 });
const word = (text, size, x0, baseline, track) => {
  const run = font.layout(text); const k = size / font.unitsPerEm; let x = x0, d = '';
  run.glyphs.forEach((g, i) => {
    const p = g.path.scale(k, -k).translate(x, baseline);
    d += p.toSVG(); x += run.positions[i].xAdvance * k + track * size;
  });
  return { d, width: x - x0 - track * size };
};
// Hauteur de capitale = 46 % du signe ; capitales centrées sur le noyau ; écart = 1/4 du signe.
const H = BOX.h, CAP = 0.46 * H, SIZE = CAP / (font.capHeight / font.unitsPerEm), GAP = 0.25 * H, TRACK = -0.01;
const capMid = BOX.y + H * 0.56, base = r2(capMid + CAP / 2);
const w = word('OldTown', SIZE, BOX.x + BOX.w + GAP, base, TRACK);
const lockW = r2(BOX.w + GAP + w.width);
for (const [n, fg, text] of [['logotype-clair.svg', STONE, NIGHT], ['logotype-sombre.svg', STONE, '#EEECF3'], ['logotype-mono.svg', 'currentColor', 'currentColor']]) {
  files[n] = svg(`${BOX.x} ${r2(BOX.y)} ${lockW} ${r2(BOX.h)}`, markPath(MARK, fg) + `  <path fill="${text}" d="${w.d}"/>\n`,
    `Logotype : signe + « OldTown » en DM Sans demi-gras (opsz 40), vectorisé. Texte ${text}.`);
}
// Aperçu du dépôt (réseaux, 1280 × 640) : logotype sombre centré sur la tuile, 160 de haut.
{
  const k = 160 / BOX.h, tr = `translate(${r2(640 - k * lockW / 2)} ${r2(320 - k * BOX.h / 2)}) scale(${+k.toFixed(4)}) translate(${-BOX.x} ${r2(-BOX.y)})`;
  files['apercu-github.svg'] = svg('0 0 1280 640', `  <rect width="1280" height="640" fill="${NIGHT}"/>\n  <g transform="${tr}">\n    ${markPath(MARK, STONE).trim()}\n    <path fill="#EEECF3" d="${w.d}"/>\n  </g>\n`,
    `Aperçu du dépôt (1280 × 640) : logotype sombre sur la tuile nuit ${NIGHT}.`);
}
fs.mkdirSync(OUT, { recursive: true });
for (const [n, s] of Object.entries(files)) fs.writeFileSync(`${OUT}/${n}`, s);
console.log('tuile', TILE_H, 'android', ANDROID_H, 'logotype', lockW, 'x', r2(BOX.h));
