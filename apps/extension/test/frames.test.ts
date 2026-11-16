// Compatibilité du format : les golden de ot-proto, rejoués côté TypeScript.
import { readdirSync } from 'node:fs';
import path from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

import { checkServerUrl, decode, encode, wsUrl } from '../src/background/frames';
import { type Geometry, createBlobBody } from '../src/background/transfer';
import { Vault, decodeFrame, fileGeometry } from '../src/wasm/ot_wasm.js';
import { ROOT, golden, loadWasm } from './support';

beforeAll(loadWasm);

describe('golden de ot-proto', () => {
  it('toutes les trames sont acceptées et relues à l’identique', () => {
    const dir = path.join(ROOT, 'crates/ot-proto/tests/golden');
    // Les `http_*` sont des corps de l'API des fichiers, pas des trames.
    const names = readdirSync(dir)
      .map((f) => f.replace(/\.json$/, ''))
      .filter((n) => !n.startsWith('http_'));
    expect(names).toContain('ping');
    expect(names).toContain('pong');
    for (const name of names) {
      const text = golden(name);
      expect(decodeFrame(text), name).toBe(text);
      expect(decode(text, decodeFrame).t).toBe(name);
    }
  });

  it('les trames écrites en TypeScript sont celles des golden', () => {
    expect(encode.ping()).toBe(golden('ping'));
    expect(encode.ack(42)).toBe(golden('ack'));
    expect(encode.listDevices()).toBe(golden('list_devices'));
    const env = (JSON.parse(golden('send')) as { env: unknown }).env;
    expect(encode.send(JSON.stringify(env))).toBe(golden('send'));
  });

  it('le corps de création d’un fichier est celui du golden', () => {
    const g = JSON.parse(fileGeometry(2_500_000, 1 << 20)) as Geometry;
    const body = createBlobBody(g, ['dev_01J9Z3M2N4P6Q8R0S2T4V6W8X0']);
    expect(JSON.stringify(body)).toBe(golden('http_create_blob'));
  });

  it('les trames signées par le WebAssembly ont la forme des golden', () => {
    const vault = new Vault();
    vault.ensureIdentity();
    const challenge = (JSON.parse(golden('challenge')) as { nonce: string }).nonce;
    const auth = JSON.parse(vault.authFrame(challenge, 'dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5'));
    expect(Object.keys(auth)).toEqual(Object.keys(JSON.parse(golden('auth'))));
    expect(decodeFrame(JSON.stringify(auth))).toBeTruthy();
    vault.free();
  });

  it('une trame invalide est refusée', () => {
    expect(() => decodeFrame('{"v":2,"t":"ack","seq":1}')).toThrow(/version/);
    expect(() => decodeFrame('{"v":1,"t":"ack","seq":1,"extra":true}')).toThrow();
    expect(() => decodeFrame('{"v":1,"t":"inconnu"}')).toThrow(/inconnu/);
  });
});

describe('URL du relais', () => {
  it('endpoint WebSocket comme ot-core', () => {
    expect(wsUrl('ws://relais:7878')).toBe('ws://relais:7878/v1/ws');
    expect(wsUrl('http://relais:7878/')).toBe('ws://relais:7878/v1/ws');
    expect(wsUrl('https://relais:7878')).toBe('wss://relais:7878/v1/ws');
    expect(wsUrl('relais:7878')).toBe('ws://relais:7878/v1/ws');
  });

  it('seuls wss:// et ws:// sont acceptés', () => {
    expect(checkServerUrl(' wss://relais:7878/ ')).toBe('wss://relais:7878');
    expect(checkServerUrl('ws://127.0.0.1:7878')).toBe('ws://127.0.0.1:7878');
    for (const bad of ['relais:7878', 'https://relais', 'wss://', 'wss://a b']) {
      expect(() => checkServerUrl(bad), bad).toThrow();
    }
  });
});
