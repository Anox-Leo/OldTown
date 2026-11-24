import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const DIR = path.dirname(fileURLToPath(import.meta.url));
const REGEX_BEFORE = new Set([...'(,=:[!&|?{};+-*%<>~^']);
const REGEX_KEYWORDS = new Set([
  'return',
  'typeof',
  'case',
  'in',
  'of',
  'void',
  'throw',
  'delete',
  'new',
]);

export function comments(src) {
  const found = [];
  const stack = [];
  let line = 1;
  let prev = '';
  let i = 0;
  const skipString = (quote) => {
    i++;
    while (i < src.length && src[i] !== quote) {
      if (src[i] === '\\') i++;
      if (src[i] === '\n') line++;
      i++;
    }
    i++;
  };
  const skipTemplate = () => {
    i++;
    while (i < src.length && src[i] !== '`') {
      if (src[i] === '\\') i++;
      else if (src[i] === '$' && src[i + 1] === '{') {
        stack.push('template');
        i += 2;
        return;
      }
      if (src[i] === '\n') line++;
      i++;
    }
    i++;
  };
  while (i < src.length) {
    const c = src[i];
    if (c === '\n') {
      line++;
      i++;
    } else if (/\s/.test(c)) {
      i++;
    } else if (src.startsWith('//', i)) {
      found.push(line);
      while (i < src.length && src[i] !== '\n') i++;
    } else if (src.startsWith('/*', i)) {
      found.push(line);
      const end = src.indexOf('*/', i + 2);
      const stop = end < 0 ? src.length : end + 2;
      for (; i < stop; i++) if (src[i] === '\n') line++;
    } else if (c === '"' || c === "'") {
      skipString(c);
      prev = 'value';
    } else if (c === '`') {
      skipTemplate();
      prev = 'value';
    } else if (c === '/' && (prev === '' || REGEX_BEFORE.has(prev) || REGEX_KEYWORDS.has(prev))) {
      let inClass = false;
      i++;
      while (i < src.length && (src[i] !== '/' || inClass)) {
        if (src[i] === '\\') i++;
        else if (src[i] === '[') inClass = true;
        else if (src[i] === ']') inClass = false;
        i++;
      }
      i++;
      prev = 'value';
    } else if (c === '{') {
      stack.push('brace');
      prev = c;
      i++;
    } else if (c === '}') {
      if (stack.pop() === 'template') {
        skipTemplate();
        prev = 'value';
      } else {
        prev = c;
        i++;
      }
    } else if (/[\w$]/.test(c)) {
      const start = i;
      while (i < src.length && /[\w$]/.test(src[i])) i++;
      prev = src.slice(start, i);
      if (!REGEX_KEYWORDS.has(prev)) prev = 'value';
    } else {
      prev = c;
      i++;
    }
  }
  return found;
}

test('le lexer distingue commentaires, chaînes, gabarits et expressions régulières', () => {
  const src = [
    "const a = '// pas'; // oui",
    'const b = `/* pas ${"//"} ${`${1} // pas`}`;',
    'const c = x.replace(/\\/\\//g, "") / 2; /* oui */',
    'const d = /[/]/.test(e);',
    'return /a/;',
    '/* oui',
    '*/',
  ].join('\n');
  assert.deepEqual(comments(src), [1, 3, 6]);
});

test('aucun commentaire dans les scripts du générateur', () => {
  const files = fs.readdirSync(DIR).filter((f) => f.endsWith('.mjs'));
  assert.ok(files.length >= 3);
  const offenders = files.flatMap((f) =>
    comments(fs.readFileSync(path.join(DIR, f), 'utf8')).map((l) => `${f}:${l}`),
  );
  assert.deepEqual(offenders, []);
});
