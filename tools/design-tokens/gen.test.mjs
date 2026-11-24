import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  ANDROID_RES_ROLES,
  androidColor,
  argb,
  avatars,
  contrast,
  contrasts,
  DEFAULT_ROOT,
  load,
  OUTPUTS,
  render,
  SOURCE,
  stale,
} from './gen.mjs';

const GEN = fileURLToPath(new URL('./gen.mjs', import.meta.url));

function sandbox() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'design-tokens-'));
  for (const rel of [SOURCE, ...Object.values(OUTPUTS)]) {
    fs.mkdirSync(path.dirname(path.join(root, rel)), { recursive: true });
    fs.copyFileSync(path.join(DEFAULT_ROOT, rel), path.join(root, rel));
  }
  return root;
}

function run(...args) {
  try {
    execFileSync(process.execPath, [GEN, ...args], { stdio: 'pipe' });
    return { code: 0, stderr: '' };
  } catch (e) {
    return { code: e.status, stderr: e.stderr.toString() };
  }
}

test('les sorties du dépôt sont à jour', () => {
  assert.deepEqual(stale(DEFAULT_ROOT, render(load())), []);
  assert.equal(run('--check').code, 0);
});

test('--check échoue sur une sortie périmée, sans rien écrire', () => {
  const root = sandbox();
  const css = path.join(root, OUTPUTS.css);
  fs.writeFileSync(css, fs.readFileSync(css, 'utf8').replace('--screen: #0f1120', '--screen: #000000'));
  const before = fs.readFileSync(css, 'utf8');
  const r = run('--check', '--root', root);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /périmé : apps\/extension\/src\/tokens\.css/);
  assert.equal(fs.readFileSync(css, 'utf8'), before);
  assert.equal(run('--root', root).code, 0);
  assert.equal(run('--check', '--root', root).code, 0);
});

test('--check échoue sur une sortie absente ou un jeton changé', () => {
  const root = sandbox();
  fs.rmSync(path.join(root, OUTPUTS.kotlin));
  assert.equal(run('--check', '--root', root).code, 1);

  const root2 = sandbox();
  const src = path.join(root2, SOURCE);
  const tokens = JSON.parse(fs.readFileSync(src, 'utf8'));
  tokens.radius.card.$value.value = 16;
  fs.writeFileSync(src, JSON.stringify(tokens));
  const r = run('--check', '--root', root2);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /tokens\.css/);
  assert.match(r.stderr, /Tokens\.kt/);
});

test('contraste WCAG : valeurs de la charte', () => {
  assert.equal(contrast('#ffffff', '#000000').toFixed(1), '21.0');
  assert.equal(contrast('#ffffff', '#b4552a').toFixed(1), '4.9');
  assert.equal(contrast('#d27443', '#232841').toFixed(2), '4.35');
  assert.equal(contrast('#e39c75', '#0f1120').toFixed(1), '8.3');
  assert.equal(contrast('#000000', '#ffffff'), contrast('#ffffff', '#000000'));
  assert.equal(contrast('#ffffff00', '#336699').toFixed(2), '1.00');
  assert.throws(() => contrast('#ffffff', '#33669980'), /translucide/);
});

test('chaque paire listée passe, dans chaque thème', () => {
  const all = contrasts(load());
  assert.ok(all.length > 40);
  for (const p of all) {
    assert.ok(p.ok, `${p.theme} ${p.kind} ${p.fg} sur ${p.bg} : ${p.ratio.toFixed(2)}`);
  }
  assert.equal(all.filter((p) => p.fg === 'on-avatar').length, 2 * avatars(load()).length);
});

test('une paire sous le minimum est refusée', () => {
  const root = sandbox();
  const src = path.join(root, SOURCE);
  const tokens = JSON.parse(fs.readFileSync(src, 'utf8'));
  tokens.role.light.primary = { $value: '{color.pierre.400}' };
  fs.writeFileSync(src, JSON.stringify(tokens));
  const r = run('--root', root);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /contraste insuffisant \(light, text\) : on-primary sur primary = 3\.\d\d:1/);
  assert.equal(
    fs.readFileSync(path.join(root, OUTPUTS.css), 'utf8'),
    fs.readFileSync(path.join(DEFAULT_ROOT, OUTPUTS.css), 'utf8'),
  );
});

test('mêmes avatars, dans le même ordre, dans toutes les sorties', () => {
  const expected = avatars(load());
  assert.equal(expected.length, 10);
  const files = render(load());
  const ts = [...files[OUTPUTS.ts].matchAll(/^ {2}'(#[0-9a-f]{6})',$/gm)].map((m) => m[1]);
  assert.deepEqual(ts, expected);
  const kt = files[OUTPUTS.kotlin].split('avatarColors = longArrayOf(')[1].split(')')[0];
  assert.deepEqual(
    [...kt.matchAll(/0xFF([0-9A-F]{6})/g)].map((m) => `#${m[1].toLowerCase()}`),
    expected,
  );
  for (const c of expected) assert.ok(files[OUTPUTS.specimen].includes(c));
});

test('ARGB de Compose et des ressources Android', () => {
  assert.equal(argb('#0f1120'), '0xFF0F1120');
  assert.equal(argb('#fe9a0052'), '0x52FE9A00');
  assert.equal(androidColor('#fe9a0052'), '#52FE9A00');
});

test('ressources Android : mêmes couleurs que Tokens.kt, clair dans values/, sombre dans values-night/', () => {
  const files = render(load());
  const colors = (rel) =>
    Object.fromEntries([...files[rel].matchAll(/<color name="ot_(\w+)">(#[0-9A-F]{8})<\/color>/g)].map((m) => [m[1], m[2]]));
  const light = colors(OUTPUTS.androidLight);
  const dark = colors(OUTPUTS.androidDark);
  assert.deepEqual(Object.keys(light), ANDROID_RES_ROLES.map((n) => n.replace(/-/g, '_')));
  assert.deepEqual(Object.keys(dark), Object.keys(light));
  const kt = files[OUTPUTS.kotlin];
  for (const [theme, res] of [['Light', light], ['Dark', dark]]) {
    const block = kt.split(`object ${theme} {`)[1].split('}')[0];
    for (const [name, color] of Object.entries(res)) {
      const camelName = name.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
      assert.match(block, new RegExp(`val ${camelName} = Color\\(0x${color.slice(1)}\\)`));
    }
  }
  assert.notEqual(light.screen, dark.screen);
});

test('mêmes rôles dans les deux thèmes, référence inconnue refusée', () => {
  const tokens = load();
  assert.deepEqual(Object.keys(tokens.role.dark), Object.keys(tokens.role.light));
  tokens.role.dark.fg = { $value: '{color.nuit.123}' };
  assert.throws(() => render(tokens), /jeton inconnu : color\.nuit\.123/);
});

test('thème clair : celui du système, ou choisi par data-theme', () => {
  const css = render(load())[OUTPUTS.css];
  assert.match(css, /@media \(prefers-color-scheme: light\) \{\n {2}:root:not\(\[data-theme='dark'\]\) \{/);
  assert.match(css, /^:root\[data-theme='light'\] \{$/m);
  const block = (start) => css.slice(css.indexOf(start)).split('}')[0].split('\n').slice(1).map((l) => l.trim());
  assert.deepEqual(block(":root:not([data-theme='dark'])"), block(":root[data-theme='light']"));
});

test('aucun commentaire dans les sorties de l\'extension et d\'Android', () => {
  const files = render(load());
  for (const key of ['css', 'ts', 'kotlin']) {
    assert.doesNotMatch(files[OUTPUTS[key]], /\/\/|\/\*/);
  }
  for (const key of ['androidLight', 'androidDark']) {
    assert.doesNotMatch(files[OUTPUTS[key]], /<!--/);
  }
});
