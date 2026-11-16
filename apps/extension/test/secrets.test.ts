// Stockage des clés (T5) : chiffré par une clé WebCrypto non exportable,
// jamais en clair dans chrome.storage.
import 'fake-indexeddb/auto';
import { IDBFactory } from 'fake-indexeddb';
import { describe, expect, it } from 'vitest';

import { IndexedDbKeySource, SecretStore } from '../src/background/secrets';
import { MemoryKV } from './support';

const enc = new TextEncoder();
const dec = new TextDecoder();

function store(kv = new MemoryKV(), idb = new IDBFactory()) {
  return {
    kv,
    keys: new IndexedDbKeySource(idb),
    store: new SecretStore(kv, new IndexedDbKeySource(idb)),
  };
}

describe('SecretStore', () => {
  it('aller-retour, et rien en clair dans le stockage', async () => {
    const { kv, store: s } = store();
    const secret = 'SECRET-de-test-0123456789';
    await s.put('device.key', enc.encode(secret));
    expect(dec.decode(await s.get('device.key'))).toBe(secret);
    const raw = JSON.stringify([...kv.items.entries()]);
    expect(raw).not.toContain(secret);
    expect(raw).not.toContain(btoa(secret));
    expect([...kv.items.keys()]).toEqual(['secret:device.key']);
    const all = await s.loadAll();
    expect([...all.keys()]).toEqual(['device.key']);
  });

  it('la clé de stockage est non exportable et réutilisée', async () => {
    const idb = new IDBFactory();
    const keys = new IndexedDbKeySource(idb);
    const key = await keys.key();
    expect(key.extractable).toBe(false);
    await expect(crypto.subtle.exportKey('raw', key)).rejects.toThrow();
    // Un autre démarrage du service worker relit la même clé.
    const again = await new IndexedDbKeySource(idb).key();
    const kv = new MemoryKV();
    await new SecretStore(kv, keys).put('a', enc.encode('x'));
    const sameKey = { key: async () => again, forget: () => Promise.resolve() };
    expect(dec.decode(await new SecretStore(kv, sameKey).get('a'))).toBe('x');
  });

  it('une valeur déplacée sous un autre nom ne se déchiffre pas', async () => {
    const { kv, store: s } = store();
    await s.put('group.json', enc.encode('{}'));
    kv.items.set('secret:roster.json', kv.items.get('secret:group.json'));
    await expect(s.get('roster.json')).rejects.toThrow(/illisible/);
  });

  it('clear efface les secrets et la clé', async () => {
    const kv = new MemoryKV();
    const idb = new IDBFactory();
    const s = new SecretStore(kv, new IndexedDbKeySource(idb));
    await s.put('a', enc.encode('1'));
    await kv.set({ device: { id: 'x' } });
    const before = kv.items.get('secret:a');
    await s.clear();
    expect([...kv.items.keys()]).toEqual(['device']);
    // L'ancienne valeur, même restaurée, n'est plus lisible : la clé est neuve.
    kv.items.set('secret:a', before);
    await expect(new SecretStore(kv, new IndexedDbKeySource(idb)).get('a')).rejects.toThrow(
      /illisible/,
    );
  });
});
