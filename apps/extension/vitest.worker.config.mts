// Tests du service worker (client du protocole, stockage, WebAssembly), sous
// Node. Les tests de l'interface Angular passent par `ng test`.
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['test/**/*.test.ts'],
    testTimeout: 30_000,
    hookTimeout: 60_000,
  },
});
