// Compatibilité du format chiffré : les vecteurs de ot-crypto, ouverts par le
// cœur WebAssembly comme le ferait l'extension.
import { beforeAll, describe, expect, it } from 'vitest';

import { Core, Vault } from '../src/wasm/ot_wasm.js';
import { loadWasm, vector } from './support';

beforeAll(loadWasm);

const A = 'dev_01J9Z3K8V7Q2W4E6R8T0Y1V3H5';
const B = 'dev_01J9Z3M2N4P6Q8R0S2T4V6W8X0';
const enc = new TextEncoder();

/** Octets `start, start+1, …` en base64 (comme `bytes` des vecteurs). */
function seq(start: number, n: number): string {
  return btoa(String.fromCharCode(...Array.from({ length: n }, (_, i) => (start + i) & 0xff)));
}

interface Roster {
  epoch: number;
  version: number;
  members: unknown[];
}

/** Appareil B (« windows ») des vecteurs, membre du groupe à l'époque `epoch`. */
function deviceB(epoch: number): { vault: Vault; core: Core } {
  const roster = (vector('group_roster')['body'] as { roster: Roster }).roster;
  const vault = new Vault();
  vault.load('device.key', enc.encode(`${seq(2, 32)}\n`));
  vault.load('exchange.key', enc.encode(`${seq(102, 32)}\n`));
  vault.load(
    'group.json',
    enc.encode(JSON.stringify({ current: epoch, epochs: [{ epoch, key: seq(10, 32) }] })),
  );
  vault.load('roster.json', enc.encode(JSON.stringify({ ...roster, epoch, version: 1 })));
  return { vault, core: Core.open(vault, B) };
}

function open(core: Core, name: string): { events: Record<string, unknown>[] } {
  const env = vector(name)['envelope'];
  return JSON.parse(core.process(1, JSON.stringify(env))) as { events: Record<string, unknown>[] };
}

describe('vecteurs de ot-crypto', () => {
  it('group_text : texte du groupe', () => {
    const { core } = deviceB(1);
    expect(open(core, 'group_text').events).toEqual([
      expect.objectContaining({ type: 'message', from: A, kind: 'text', body: 'bonjour' }),
    ]);
  });

  it('group_link : lien à plusieurs destinataires, époque 7', () => {
    const { core } = deviceB(7);
    expect(open(core, 'group_link').events).toEqual([
      expect.objectContaining({ type: 'message', kind: 'link', body: 'https://example.org/' }),
    ]);
  });

  it('group_roster : nouveau roster adopté', () => {
    const { core } = deviceB(1);
    expect(open(core, 'group_roster').events).toEqual([{ type: 'group_changed', epoch: 1 }]);
    expect(JSON.parse(core.group() ?? '{}').roster_version).toBe(2);
  });

  it('un vecteur altéré est rejeté', () => {
    const { core } = deviceB(1);
    const env = structuredClone(vector('group_text')['envelope']) as { payload: { data: string } };
    const data = atob(env.payload.data);
    env.payload.data = btoa(
      data.slice(0, -1) + String.fromCharCode(data.charCodeAt(data.length - 1) ^ 1),
    );
    const out = JSON.parse(core.process(1, JSON.stringify(env))) as {
      events: Record<string, unknown>[];
    };
    expect(out.events).toEqual([
      expect.objectContaining({ type: 'rejected', reason: 'déchiffrement impossible' }),
    ]);
  });

  it('pairing_qr_welcome : groupe reçu par QR inversé', () => {
    const offer = vector('pairing_offer');
    const welcome = vector('pairing_qr_welcome');
    const vault = new Vault();
    vault.load('device.key', enc.encode(`${offer['signing_seed'] as string}\n`));
    vault.load('exchange.key', enc.encode(`${offer['exchange_secret'] as string}\n`));
    const expires = new Date(Date.now() + 60_000).toISOString();
    vault.load(
      'pending_offer.json',
      enc.encode(JSON.stringify({ qr: offer['qr'], secret: offer['secret'], expires_at: expires })),
    );
    const core = Core.open(vault, offer['device_id'] as string);
    expect(JSON.parse(core.pending() ?? 'null')).toMatchObject({ kind: 'offer' });
    const out = JSON.parse(core.process(1, JSON.stringify(welcome['envelope']))) as {
      events: Record<string, unknown>[];
    };
    expect(out.events).toContainEqual(expect.objectContaining({ type: 'group_changed' }));
    expect(core.pending()).toBeUndefined();
    expect(vault.has('pending_offer.json')).toBe(false);
  });
});
