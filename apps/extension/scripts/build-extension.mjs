import { build } from 'esbuild';
import { copyFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const dist = path.join(app, 'dist/extension');

await build({
  entryPoints: {
    background: path.join(app, 'src/background/main.ts'),
    offscreen: path.join(app, 'src/offscreen/offscreen.ts'),
    send: path.join(app, 'src/pages/send.ts'),
  },
  outdir: dist,
  bundle: true,
  format: 'esm',
  target: 'chrome120',
  minify: true,
  sourcemap: false,
  legalComments: 'none',
  logLevel: 'warning',
});

mkdirSync(path.join(dist, 'wasm'), { recursive: true });
copyFileSync(path.join(app, 'src/wasm/ot_wasm_bg.wasm'), path.join(dist, 'wasm/ot_wasm_bg.wasm'));
copyFileSync(path.join(app, 'src/offscreen/offscreen.html'), path.join(dist, 'offscreen.html'));
for (const page of ['send.html', 'page.css']) {
  copyFileSync(path.join(app, 'src/pages', page), path.join(dist, page));
}
copyFileSync(path.join(app, 'src/tokens.css'), path.join(dist, 'tokens.css'));
copyFileSync(path.join(app, 'manifest.json'), path.join(dist, 'manifest.json'));
copyFileSync(path.join(dist, 'index.html'), path.join(dist, 'options.html'));
copyFileSync(path.join(dist, 'index.html'), path.join(dist, 'admin.html'));
console.log(`extension prête : ${path.relative(process.cwd(), dist)}`);
