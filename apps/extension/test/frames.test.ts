import { readdirSync } from 'node:fs';
import path from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

import {
  type ClientFrame,
  type ServerFrame,
  answers,
  callReference,
  checkServerUrl,
  decode,
  encode,
  request,
  wsUrl,
} from '../src/background/frames';
import { type Geometry, createBlobBody } from '../src/background/transfer';
import { Vault, decodeFrame, fileGeometry } from '../src/wasm/ot_wasm.js';
import { ROOT, golden, loadWasm } from './support';

beforeAll(loadWasm);

describe('golden de ot-proto', () => {
  it('toutes les trames sont acceptées et relues à l’identique', () => {
    const dir = path.join(ROOT, 'crates/ot-proto/tests/golden');
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

  it('les demandes de l’étape 10 écrites en TypeScript sont celles des golden', () => {
    const g = (name: string) => JSON.parse(golden(name)) as Record<string, unknown>;
    const BOB = 'usr_01J9Z4B1C2D3E4F5G6H7J8K9M0';
    expect(encode.request(request.usersGet())).toBe(golden('users_get'));
    expect(encode.request(request.userInvite('Bob', ['usr_01J9Z4A1B2C3D4E5F6G7H8J9K0']))).toBe(
      golden('user_invite'),
    );
    expect(encode.request(request.userReinvite(BOB))).toBe(golden('user_reinvite'));
    const cert = g('user_certify')['cert'] as { data: string; sig: string };
    expect(encode.request(request.userCertify(BOB, cert))).toBe(golden('user_certify'));
    expect(encode.request(request.userRevoke(BOB))).toBe(golden('user_revoke'));
    expect(encode.request(request.userRename(BOB, 'Bobby'))).toBe(golden('user_rename'));
    const link = g('link_set');
    expect(
      encode.request(request.linkSet(link['from'] as string, link['to'] as string, true)),
    ).toBe(golden('link_set'));
    expect(encode.request(request.deviceRevoke('dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5'))).toBe(
      golden('device_revoke'),
    );
    const mute = g('service_mute');
    expect(encode.request(request.serviceMute(mute['service'] as string, true))).toBe(
      golden('service_mute'),
    );
    const update = g('device_update');
    delete update['features'];
    expect(
      encode.request(
        request.deviceUpdate(update['device'] as string, {
          kind: 'cli',
          reachable: false,
          default: false,
        }),
      ),
    ).toBe(JSON.stringify(update));
    expect(decodeFrame(JSON.stringify(update))).toBe(JSON.stringify(update));
    expect(request.deviceUpdate('dev_x', { default: true })).toEqual({
      t: 'device_update',
      device: 'dev_x',
      default: true,
    });
  });

  it('chaque réponse va à sa demande, chaque refus à sa référence', () => {
    const reply = (name: string) => decode(golden(name), decodeFrame);
    const publish: ClientFrame = {
      t: 'roster_publish',
      user: 'usr_01J9Z4A1B2C3D4E5F6G7H8J9K0',
      seq: 21,
      entry: {},
    };
    expect(answers(publish, reply('roster_published'))).toBe(true);
    expect(answers({ ...publish, seq: 20 }, reply('roster_published'))).toBe(false);
    expect(callReference(publish)).toBe('21');
    expect(answers({ t: 'directory_get' }, reply('directory'))).toBe(true);
    expect(answers({ t: 'users_get' }, reply('users'))).toBe(true);
    expect(answers({ t: 'users_get' }, reply('directory'))).toBe(false);
    const invited = reply('user_invited') as Extract<ServerFrame, { t: 'user_invited' }>;
    expect(answers(request.userInvite('Bob', []), invited)).toBe(true);
    expect(answers(request.userReinvite(invited.user), invited)).toBe(true);
    expect(answers(request.userReinvite('usr_autre'), invited)).toBe(false);
    expect(callReference(request.userInvite('Bob', []))).toBeUndefined();
    expect(callReference(request.linkSet('a', 'b', true))).toBeUndefined();
    expect(callReference(request.userRevoke('usr_b'))).toBe('usr_b');
    expect(callReference(request.deviceUpdate('dev_x', {}))).toBe('dev_x');
    expect(callReference(request.serviceMute('usr_s', true))).toBe('usr_s');
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
