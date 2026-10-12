// L'extension (moteur du service worker, WebAssembly compris) contre un vrai
// `ot-server`, avec la CLI `ot` comme parrain : QR inversé, code collé,
// messages dans les deux sens, redémarrage, file hors ligne, maintien.
import { IDBFactory } from 'fake-indexeddb';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import { Engine } from '../src/background/engine';
import { IndexedDbKeySource } from '../src/background/secrets';
import type { Item } from '../src/shared/api';
import { Cli, MemoryKV, Relay, eventually, loadWasm } from './support';

const FAST = { backoffInitial: 50, backoffMax: 300, pingEvery: 200, idleTimeout: 2_000 };

interface Profile {
  kv: MemoryKV;
  idb: IDBFactory;
}

function profile(): Profile {
  return { kv: new MemoryKV(), idb: new IDBFactory() };
}

function open(p: Profile): Promise<Engine> {
  return Engine.open({ kv: p.kv, keys: new IndexedDbKeySource(p.idb), timing: FAST });
}

describe('extension contre ot-server', () => {
  let relay: Relay;
  let phone: Cli;
  const chrome = profile();
  let engine: Engine;

  beforeAll(async () => {
    loadWasm();
    relay = await Relay.start();
    phone = new Cli(relay, 'telephone');
    await phone.setupGroup();
  });

  afterAll(async () => {
    engine?.close();
    await relay?.dispose();
  });

  it('rejoint le groupe par QR inversé', async () => {
    engine = await open(chrome);
    const state = await engine.setupOffer(relay.url, 'chrome');
    expect(state.phase).toBe('pairing');
    if (state.pending?.kind !== 'offer') throw new Error('offre attendue');
    expect(state.pending.text).toMatch(/^oldtown:\/\/sponsor\?o=/);
    expect(state.pending.qr.modules.length).toBe(state.pending.qr.size ** 2);

    // Le téléphone scanne le QR (ici : la CLI le parraine).
    await phone.run('pair', 'sponsor', state.pending.text);
    await eventually('groupe adopté', () => engine.state().phase === 'ready' || undefined);
    const group = engine.state().group;
    expect(group?.members.map((m) => m.name).sort()).toEqual(['chrome', 'telephone']);
    expect(engine.state().pending).toBeUndefined();
    // L'effacement du secret est rangé juste après (avant l'acquittement).
    await eventually('offre effacée', () =>
      chrome.kv.items.has('secret:pending_offer.json') ? undefined : true,
    );
  });

  it('envoie et reçoit', async () => {
    const listen = phone.listen();
    try {
      await eventually('connexion', () => engine.state().connection === 'online' || undefined);
      // Laisser `ot listen` ouvrir sa session avant d'envoyer.
      await new Promise((r) => setTimeout(r, 500));
      await engine.send('https://exemple.org/page', '');
      const got = await eventually('reçu par le téléphone', () => listen.messages[0]);
      expect(got).toMatchObject({
        kind: 'link',
        body: 'https://exemple.org/page',
        from_name: 'chrome',
      });
    } finally {
      listen.stop();
    }

    const items: Item[] = [];
    engine.onChange((p) => p.type === 'item' && items.push(p.item));
    await phone.run('send', 'bonjour depuis le téléphone');
    const item = await eventually('reçu par l’extension', () => items[0]);
    expect(item).toMatchObject({
      kind: 'text',
      body: 'bonjour depuis le téléphone',
      fromName: 'telephone',
    });
    expect((await engine.history())[0]?.body).toBe('bonjour depuis le téléphone');
  });

  it('rien de lisible dans le profil Chrome', async () => {
    const raw = JSON.stringify([...chrome.kv.items.entries()]);
    expect(raw).not.toContain('bonjour depuis le téléphone');
    expect(raw).not.toContain('exemple.org');
    for (const key of chrome.kv.items.keys()) {
      expect(
        ['device', 'recipient', 'outbox', 'seen'].includes(key) || key.startsWith('secret:'),
      ).toBe(true);
    }
  });

  it('reprend après un redémarrage du service worker, sans doublon', async () => {
    engine.close();
    await phone.run('send', 'pendant l’arrêt');
    engine = await open(chrome);
    expect(engine.state().phase).toBe('ready');
    await eventually(
      'message en attente reçu',
      async () => (await engine.history()).some((i) => i.body === 'pendant l’arrêt') || undefined,
    );
    const bodies = (await engine.history()).map((i) => i.body);
    expect(bodies.filter((b) => b === 'bonjour depuis le téléphone')).toHaveLength(1);
  });

  it('garde les envois hors ligne et les envoie au retour du relais', async () => {
    await relay.stop();
    await eventually('déconnexion', () => engine.state().connection !== 'online' || undefined);
    await engine.send('écrit hors ligne', '');
    expect(engine.state().pendingSends).toBe(1);
    await relay.restart();
    await eventually('envoi accepté', () => engine.state().pendingSends === 0 || undefined);
    const listen = phone.listen();
    try {
      const got = await eventually('reçu par le téléphone', () =>
        listen.messages.find((m) => m['body'] === 'écrit hors ligne'),
      );
      expect(got['from_name']).toBe('chrome');
    } finally {
      listen.stop();
    }
  });

  it('reste connecté grâce aux ping', async () => {
    await eventually('connexion', () => engine.state().connection === 'online' || undefined);
    const drops: string[] = [];
    engine.onChange((p) => {
      if (p.type === 'state' && p.state.connection !== 'online') drops.push(p.state.connection);
    });
    // Délai d'inactivité de 2 s : sans `pong`, la session tomberait.
    await new Promise((r) => setTimeout(r, 4_000));
    expect(drops).toEqual([]);
  });

  it('rejoint le groupe avec un code collé', async () => {
    const other = profile();
    const second = await open(other);
    try {
      const invitation = await phone.pairNew();
      await second.setupCode(invitation.code, relay.url, 'chrome-pro');
      expect(second.state().pending).toEqual({ kind: 'join' });
      await eventually('groupe adopté', () => second.state().phase === 'ready' || undefined);
      await invitation.done;
      expect(second.state().group?.members).toHaveLength(3);
      await eventually('demande effacée', () =>
        other.kv.items.has('secret:pending_join.json') ? undefined : true,
      );
    } finally {
      second.close();
    }
  });

  it('un code invalide est refusé sans rien garder', async () => {
    const other = profile();
    const third = await open(other);
    await expect(third.setupCode('ot1-pas-un-code', relay.url, 'x')).rejects.toThrow();
    expect(third.state().phase).toBe('unconfigured');
    third.close();
  });

  it('un QR inversé expiré n’est plus affiché ni retenté', async () => {
    const other = profile();
    const lonely = await open(other);
    try {
      await lonely.setupOffer(relay.url, 'sans-parrain');
      expect(lonely.state().pending?.kind).toBe('offer');
      vi.useFakeTimers({ toFake: ['Date'], now: Date.now() + 11 * 60_000 });
      const state = lonely.state();
      expect(state.phase).toBe('pairing');
      expect(state.pending).toBeUndefined();
    } finally {
      vi.useRealTimers();
      lonely.close();
    }
  });

  it('des secrets illisibles n’empêchent pas d’oublier l’appareil', async () => {
    const other = profile();
    const broken = await open(other);
    await broken.setupOffer(relay.url, 'cassé');
    broken.close();
    // Profil restauré sans sa clé de stockage (IndexedDB vidée).
    other.idb = new IDBFactory();
    const reopened = await open(other);
    expect(reopened.state().phase).toBe('unconfigured');
    expect(reopened.state().connectionError).toMatch(/illisibles/);
    const fresh = await reopened.forget();
    expect(fresh.phase).toBe('unconfigured');
    expect([...other.kv.items.keys()].filter((k) => k.startsWith('secret:'))).toEqual([]);
    reopened.close();
  });

  it('oublier l’appareil efface tout', async () => {
    await engine.forget();
    expect(engine.state().phase).toBe('unconfigured');
    expect([...chrome.kv.items.keys()].filter((k) => k !== 'recipient')).toEqual([]);
  });
});
