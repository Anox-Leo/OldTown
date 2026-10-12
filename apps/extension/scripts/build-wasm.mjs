// Construit `ot-wasm` (crates/ot-wasm) et ses liaisons JavaScript dans
// src/wasm/ (généré, jamais versionné ; note 0023).
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const out = path.resolve(root, 'apps/extension/src/wasm');
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: 'inherit' });

run('cargo', [
  'build',
  '--locked',
  '-p',
  'ot-wasm',
  '--target',
  'wasm32-unknown-unknown',
  '--profile',
  'wasm',
]);
run('wasm-bindgen', [
  '--target',
  'web',
  '--out-dir',
  out,
  '--out-name',
  'ot_wasm',
  path.join(root, 'target/wasm32-unknown-unknown/wasm/ot_wasm.wasm'),
]);
