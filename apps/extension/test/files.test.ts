// Étape 5, T6 et T8 : fichiers entre l'extension (moteur du service worker,
// WebAssembly compris) et la CLI `ot`, contre un vrai `ot-server`. Le
// stockage des fichiers (OPFS dans le navigateur) est ici en mémoire ; les
// fichiers reçus sont servis par `fileResponse` (note 0029).
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { IDBFactory } from 'fake-indexeddb';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { Engine, type FileStorage, NO_RELAY_ACCESS } from '../src/background/engine';
import { IndexedDbKeySource } from '../src/background/secrets';
import type { ByteSource } from '../src/background/transfer';
import { Cli, MemoryKV, Relay, eventually, loadWasm } from './support';

const FAST = { backoffInitial: 50, backoffMax: 300, pingEvery: 200, idleTimeout: 2_000 };
/** Morceaux de 64 Kio : quelques Mio en font des dizaines. */
const TRANSFER = { chunkSize: 64 * 1024, retryInitial: 20, retryMax: 200 };

/** OPFS en mémoire ; `failAfter` simule un arrêt en plein envoi. */
class MemFiles implements FileStorage {
  readonly files = new Map<string, Uint8Array>();
  reads = 0;
  failAfter = Infinity;

  async source(p: string): Promise<ByteSource> {
    const data = this.files.get(p);
    if (!data) throw new Error(`absent : ${p}`);
    return {
      size: data.length,
      read: async (offset, length) => {
        if (++this.reads > this.failAfter) throw new Error('service worker arrêté');
        return data.slice(offset, offset + length);
      },
    };
  }

  async getChunk(blobId: string, index: number): Promise<Uint8Array | undefined> {
    return this.files.get(`cache/${blobId}/${index}`);
  }

  async putChunk(blobId: string, index: number, sealed: Uint8Array): Promise<void> {
    this.files.set(`cache/${blobId}/${index}`, sealed);
  }

  async removeCache(blobId: string): Promise<void> {
    for (const k of [...this.files.keys()]) {
      if (k.startsWith(`cache/${blobId}/`)) this.files.delete(k);
    }
  }

  async listCache(): Promise<string[]> {
    const ids = [...this.files.keys()]
      .filter((k) => k.startsWith('cache/'))
      .map((k) => k.split('/')[1] ?? '');
    return [...new Set(ids)];
  }

  async remove(p: string): Promise<void> {
    this.files.delete(p);
  }
}

function sample(len: number): Uint8Array {
  const out = new Uint8Array(len);
  let x = 0x12345678;
  for (let i = 0; i < len; i++) {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    out[i] = x & 0xff;
  }
  return out;
}

describe('fichiers entre l’extension et la CLI', () => {
  let relay: Relay;
  let phone: Cli;
  let engine: Engine;
  const files = new MemFiles();
  const kv = new MemoryKV();
  const idb = new IDBFactory();
  /** Permission d'accès au relais, comme `chrome.permissions` (note 0027). */
  let relayAllowed = true;
  const open = () =>
    Engine.open({
      kv,
      keys: new IndexedDbKeySource(idb),
      files,
      timing: FAST,
      transferTiming: TRANSFER,
      relayAllowed: async () => relayAllowed,
    });

  beforeAll(async () => {
    loadWasm();
    relay = await Relay.start();
    phone = new Cli(relay, 'telephone');
    await phone.setupGroup();
    engine = await open();
    const state = await engine.setupOffer(relay.url, 'chrome');
    if (state.pending?.kind !== 'offer') throw new Error('offre attendue');
    await phone.run('pair', 'sponsor', state.pending.text);
    await eventually('groupe adopté', () => engine.state().phase === 'ready' || undefined);
  });

  afterAll(async () => {
    engine?.close();
    await relay?.dispose();
  });

  it('envoie un fichier au téléphone', async () => {
    const data = sample(700_000);
    files.files.set('outgoing/a', data);
    const listen = phone.listen();
    try {
      const msgId = await engine.sendFile({
        path: 'outgoing/a',
        name: 'rapport.pdf',
        mime: 'application/pdf',
        size: data.length,
        to: '',
      });
      expect(msgId).toMatch(/^[0-9A-Z]{26}$/);
      // Source effacée, plus d'envoi en cours.
      expect(files.files.has('outgoing/a')).toBe(false);
      expect(engine.state().transfers).toEqual([]);
      const announced = await eventually('annoncé au téléphone', () =>
        listen.messages.find((m) => m['kind'] === 'file'),
      );
      expect(announced).toMatchObject({
        name: 'rapport.pdf',
        mime: 'application/pdf',
        size: 700_000,
      });
      const dir = mkdtempSync(path.join(tmpdir(), 'ot-ext-dl-'));
      await phone.run('download', String(announced['blob_id']), '--to', dir);
      expect(Buffer.compare(readFileSync(path.join(dir, 'rapport.pdf')), data)).toBe(0);
    } finally {
      listen.stop();
    }
  });

  it('reçoit un fichier : copie chiffrée d’office, servi en clair par son adresse', async () => {
    const data = sample(300_000);
    const dir = mkdtempSync(path.join(tmpdir(), 'ot-ext-up-'));
    const src = path.join(dir, 'photo été.jpg');
    writeFileSync(src, data);
    await phone.run('send', '--file', src);
    const item = await eventually('copie complète et vérifiée', async () =>
      (await engine.history()).find((i) => i.body === 'photo été.jpg' && i.file?.cached),
    );
    expect(item).toMatchObject({ kind: 'file', fromName: 'telephone' });
    expect(item.file).toMatchObject({ mime: 'image/jpeg', size: 300_000 });
    const blobId = item.file?.blobId ?? '';

    // Copie chiffrée : aucun morceau ne contient le clair.
    const cached = [...files.files.entries()].filter(([k]) => k.startsWith(`cache/${blobId}/`));
    expect(cached.length).toBeGreaterThan(0);
    const probe = Buffer.from(data.slice(1000, 1064));
    for (const [, c] of cached) expect(Buffer.from(c).includes(probe)).toBe(false);

    // Servi en entier, ouvert par le navigateur (image), en origine opaque.
    const res = await engine.fileResponse(blobId, { download: false, range: null });
    expect(res.status).toBe(200);
    expect(res.headers.get('content-type')).toBe('image/jpeg');
    expect(res.headers.get('content-disposition')).toMatch(/^inline; filename\*=UTF-8''photo%20/);
    expect(res.headers.get('content-security-policy')).toBe('sandbox');
    expect(Buffer.compare(Buffer.from(await res.arrayBuffer()), data)).toBe(0);

    // Par plage (vidéos), en téléchargement.
    const part = await engine.fileResponse(blobId, { download: true, range: 'bytes=1000-1999' });
    expect(part.status).toBe(206);
    expect(part.headers.get('content-range')).toBe('bytes 1000-1999/300000');
    expect(part.headers.get('content-disposition')).toMatch(/^attachment;/);
    expect(Buffer.compare(Buffer.from(await part.arrayBuffer()), data.slice(1000, 2000))).toBe(0);

    // Inconnu : 404. Et aucune clé dans l'historique.
    const unknown = await engine.fileResponse('blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V', {
      download: false,
      range: null,
    });
    expect(unknown.status).toBe(404);
    expect(JSON.stringify(await engine.history())).not.toMatch(/"key"|nonce/);
  });

  it('reprend un envoi interrompu sans tout relire', async () => {
    const data = sample(1_000_000);
    const chunks = Math.ceil(data.length / TRANSFER.chunkSize);
    files.files.set('outgoing/b', data);
    files.reads = 0;
    // Le hachage relit tout, puis l'envoi s'arrête à mi-chemin.
    files.failAfter = chunks + Math.floor(chunks / 2);
    const file = { path: 'outgoing/b', name: 'b.bin', mime: '', size: data.length, to: '' };
    await expect(engine.sendFile(file)).rejects.toThrow('service worker arrêté');
    expect(engine.state().transfers[0]?.error).toContain('arrêté');

    // Le service worker redémarre : l'envoi rangé reprend tout seul.
    engine.close();
    files.reads = 0;
    files.failAfter = Infinity;
    engine = await open();
    await eventually('envoi repris et terminé', () => !files.files.has('outgoing/b') || undefined);
    expect(files.reads).toBeGreaterThan(0);
    expect(files.reads).toBeLessThan(chunks);
    const listen = phone.listen();
    try {
      await eventually('annoncé au téléphone', () =>
        listen.messages.find((m) => m['name'] === 'b.bin'),
      );
    } finally {
      listen.stop();
    }
  });

  it('sans accès au relais, un fichier reçu attend l’autorisation puis se copie', async () => {
    relayAllowed = false;
    await engine.refreshRelayAccess();
    expect(engine.state().relayAccess).toBe(false);
    const data = sample(100_000);
    const dir = mkdtempSync(path.join(tmpdir(), 'ot-ext-perm-'));
    const src = path.join(dir, 'attente.png');
    writeFileSync(src, data);
    await phone.run('send', '--file', src);
    const waiting = await eventually('fichier en attente d’accès', async () =>
      (await engine.history()).find((i) => i.body === 'attente.png' && i.file?.error),
    );
    expect(waiting.file?.error).toBe(NO_RELAY_ACCESS);
    // Un envoi est refusé tout de suite, avec le même message.
    files.files.set('outgoing/c', sample(10));
    const blocked = { path: 'outgoing/c', name: 'c', mime: '', size: 10, to: '' };
    await expect(engine.sendFile(blocked)).rejects.toThrow(NO_RELAY_ACCESS);
    files.files.delete('outgoing/c');

    // L'accès est accordé : la copie repart seule.
    relayAllowed = true;
    await engine.refreshRelayAccess();
    const done = await eventually('copié après l’autorisation', async () =>
      (await engine.history()).find((i) => i.body === 'attente.png' && i.file?.cached),
    );
    const res = await engine.fileResponse(done.file?.blobId ?? '', {
      download: false,
      range: null,
    });
    expect(Buffer.compare(Buffer.from(await res.arrayBuffer()), data)).toBe(0);
  });

  it('ni nom de fichier ni contenu en clair dans le profil', async () => {
    const raw = JSON.stringify([...kv.items.entries()]);
    for (const needle of ['rapport.pdf', 'photo été', 'b.bin', 'attente.png']) {
      expect(raw).not.toContain(needle);
    }
    for (const key of kv.items.keys()) {
      expect(
        ['device', 'recipient', 'outbox', 'seen'].includes(key) || key.startsWith('secret:'),
      ).toBe(true);
    }
  });
});
