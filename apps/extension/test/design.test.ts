// Identité visuelle (note 0059) : couleurs prises dans les jetons, signe et
// icônes repris de `branding/`.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

import { MARK_PATH, MARK_SMALL_PATH, WORDMARK_PATH } from '../src/app/brand';

const app = path.resolve(__dirname, '..');
const branding = path.resolve(app, '../../branding');

/** Fichiers sous `src/`, chemins relatifs à l'extension. */
function sources(dir = path.join(app, 'src')): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return [path.relative(app, full)];
  });
}

describe('couleurs', () => {
  // Générés depuis `design/tokens.json` ; le QR reste noir sur blanc ; liaisons
  // WebAssembly générées.
  const allowed = new Set(['src/tokens.css', 'src/shared/tokens.ts', 'src/app/qr.ts']);

  it('aucune couleur en dur dans src/ hors des jetons générés et du QR', () => {
    const found: string[] = [];
    for (const file of sources()) {
      if (allowed.has(file) || file.startsWith('src/wasm/')) continue;
      if (!/\.(ts|css|html)$/.test(file)) continue;
      readFileSync(path.join(app, file), 'utf8')
        .split('\n')
        .forEach((line, i) => {
          // Couleurs hexadécimales (pas les entités HTML `&#…;`) et fonctions de couleur.
          const m = /(?<![&\w])#[0-9a-fA-F]{3,8}\b|\b(rgba?|hsla?)\(/.exec(line);
          if (m) found.push(`${file}:${i + 1}: ${m[0]}`);
        });
    }
    expect(found).toEqual([]);
  });
});

describe('signe et logotype', () => {
  const paths = (file: string) =>
    [...readFileSync(path.join(branding, file), 'utf8').matchAll(/ d="([^"]*)"/g)].map((m) => m[1]);

  it('mêmes tracés que branding/', () => {
    expect(paths('archivolte-signe.svg')).toEqual([MARK_PATH]);
    expect(paths('archivolte-petit.svg')).toEqual([MARK_SMALL_PATH]);
    expect(paths('logotype-mono.svg')).toEqual([MARK_PATH, WORDMARK_PATH]);
  });
});

describe('icônes', () => {
  /** Largeur et hauteur d'un PNG (en-tête IHDR). */
  function size(file: string): [number, number] {
    const png = readFileSync(path.join(app, 'public', file));
    expect(png.subarray(1, 4).toString()).toBe('PNG');
    return [png.readUInt32BE(16), png.readUInt32BE(20)];
  }

  it('le manifeste pointe des PNG aux bonnes tailles', () => {
    const manifest = JSON.parse(readFileSync(path.join(app, 'manifest.json'), 'utf8'));
    for (const icons of [manifest.icons, manifest.action.default_icon]) {
      for (const [n, file] of Object.entries<string>(icons)) {
        expect(size(file)).toEqual([Number(n), Number(n)]);
      }
    }
  });
});
