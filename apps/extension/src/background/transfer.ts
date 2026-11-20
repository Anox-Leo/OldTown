import { httpBase } from '../shared/origins';
import { Hasher, fileGeometry } from '../wasm/ot_wasm.js';

export const CHUNK_SIZE = 1 << 20;
const BLOBS_PATH = '/v1/blobs';

export interface ByteSource {
  readonly size: number;
  read(offset: number, length: number): Promise<Uint8Array>;
}

export interface Geometry {
  size: number;
  chunk_size: number;
  chunks: number;
  sealed_size: number;
  sealed_chunk_size: number;
}

interface Upload extends Geometry {
  blob_id: string;
  expires_at: string;
  to: string[];
  to_user?: string;
}

function sameTarget(up: Upload, to: string[], toUser: string | undefined): boolean {
  const a = [...up.to].sort();
  const b = [...to].sort();
  return up.to_user === toUser && a.length === b.length && a.every((x, i) => x === b[i]);
}

export interface TransferCore {
  upload(sourceId: string, size: number): string | undefined;
  startUpload(
    sourceId: string,
    name: string,
    mime: string,
    size: number,
    chunkSize: number,
    sha256: Uint8Array,
    to: string[],
    blobId: string,
    expiresAt: string,
    toUser?: string,
  ): string;
  sealChunk(sourceId: string, index: number, plain: Uint8Array): Uint8Array;
  sealFile(sourceId: string): string;
  forgetUpload(sourceId: string): void;
}

export interface TransferTiming {
  parallel: number;
  chunkSize: number;
  retryInitial: number;
  retryMax: number;
  stallTimeout: number;
  requestTimeout: number;
}

const DEFAULT_TIMING: TransferTiming = {
  parallel: 4,
  chunkSize: CHUNK_SIZE,
  retryInitial: 500,
  retryMax: 30_000,
  stallTimeout: 10 * 60_000,
  requestTimeout: 120_000,
};

export interface TransferDeps {
  serverUrl: string;
  deviceId: string;
  sign(method: string, path: string, body: Uint8Array, nowMs: number): string;
  core: TransferCore;
  persistVault(): Promise<void>;
  fetch?: typeof fetch;
  timing?: Partial<TransferTiming>;
}

export type Progress = (done: number, total: number) => void;

export function createBlobBody(g: Geometry, to: string[]): Record<string, unknown> {
  return { size: g.sealed_size, chunk_size: g.sealed_chunk_size, chunks: g.chunks, to };
}

export class ExpiredError extends Error {
  constructor() {
    super('fichier expiré : il n’est plus sur le relais');
  }
}

function delay(attempt: number, t: TransferTiming): number {
  const base = Math.min(t.retryInitial * 2 ** Math.min(attempt, 16), t.retryMax);
  return base / 2 + Math.random() * (base / 2);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

class Stall {
  private last = Date.now();
  touch(): void {
    this.last = Date.now();
  }
  since(): number {
    return Date.now() - this.last;
  }
}

class Reporter {
  private lastAt = 0;
  constructor(
    private readonly total: number,
    private done: number,
    private readonly onProgress?: Progress,
  ) {}
  add(n: number): void {
    this.done += n;
    const now = Date.now();
    if (this.done >= this.total || now - this.lastAt >= 250) {
      this.lastAt = now;
      this.onProgress?.(this.done, this.total);
    }
  }
  start(): void {
    this.onProgress?.(this.done, this.total);
  }
}

export class Transfers {
  private readonly t: TransferTiming;
  private readonly base: string;

  constructor(private readonly deps: TransferDeps) {
    this.t = { ...DEFAULT_TIMING, ...deps.timing };
    this.base = httpBase(deps.serverUrl);
  }

  private async call(
    method: string,
    path: string,
    body: Uint8Array,
    stall: Stall,
  ): Promise<Uint8Array> {
    const doFetch = this.deps.fetch ?? fetch;
    for (let attempt = 0; ; attempt++) {
      const headers = JSON.parse(this.deps.sign(method, path, body, Date.now())) as Record<
        string,
        string
      >;
      let failure: string;
      try {
        const res = await doFetch(this.base + path, {
          method,
          headers,
          body: method === 'GET' || method === 'DELETE' ? undefined : (body as BodyInit),
          credentials: 'omit',
          signal: AbortSignal.timeout(this.t.requestTimeout),
        });
        const bytes = new Uint8Array(await res.arrayBuffer());
        if (res.ok) {
          stall.touch();
          return bytes;
        }
        if (res.status === 404) throw new ExpiredError();
        const message = relayMessage(bytes) ?? `erreur HTTP ${res.status}`;
        if (res.status < 500) throw new Error(`refusé par le relais : ${message}`);
        failure = message;
      } catch (e) {
        if (e instanceof ExpiredError || (e instanceof Error && e.message.startsWith('refusé')))
          throw e;
        failure = e instanceof Error ? e.message : String(e);
      }
      if (stall.since() >= this.t.stallTimeout) {
        throw new Error(`relais injoignable : ${failure}`);
      }
      await sleep(delay(attempt, this.t));
    }
  }

  private async json<T>(method: string, path: string, body: unknown, stall: Stall): Promise<T> {
    const bytes =
      body === undefined ? new Uint8Array() : new TextEncoder().encode(JSON.stringify(body));
    const res = await this.call(method, path, bytes, stall);
    return JSON.parse(new TextDecoder().decode(res)) as T;
  }

  async upload(
    file: { sourceId: string; name: string; mime: string; to: string[]; toUser?: string },
    source: ByteSource,
    onProgress?: Progress,
  ): Promise<{ env: string; blobId: string }> {
    const core = this.deps.core;
    let up: Upload | undefined;
    const existing = core.upload(file.sourceId, source.size);
    let missing: number[] | undefined;
    const previous = existing ? (JSON.parse(existing) as Upload) : undefined;
    if (previous && sameTarget(previous, file.to, file.toUser)) {
      up = previous;
      try {
        const st = await this.json<{ missing: number[] }>(
          'GET',
          `${BLOBS_PATH}/${up.blob_id}`,
          undefined,
          new Stall(),
        );
        missing = st.missing;
      } catch (e) {
        if (!(e instanceof ExpiredError)) throw e;
        up = undefined;
      }
    }
    if (!up) {
      const sha = await hashSource(source, this.t.chunkSize);
      const g = JSON.parse(fileGeometry(source.size, this.t.chunkSize)) as Geometry;
      const created = await this.json<{ blob_id: string; expires_at: string }>(
        'POST',
        BLOBS_PATH,
        createBlobBody(g, file.to),
        new Stall(),
      );
      up = JSON.parse(
        core.startUpload(
          file.sourceId,
          file.name,
          file.mime,
          source.size,
          this.t.chunkSize,
          sha,
          file.to,
          created.blob_id,
          created.expires_at,
          file.toUser,
        ),
      ) as Upload;
      await this.deps.persistVault();
    }
    const geo: Upload = up;
    const todo = missing ?? [...Array(geo.chunks).keys()];
    const len = (i: number) => Math.min(geo.chunk_size, geo.size - i * geo.chunk_size);
    const already = geo.size - todo.reduce((n, i) => n + len(i), 0);
    const reporter = new Reporter(geo.size, already, onProgress);
    reporter.start();
    const queue = [...todo];
    const stall = new Stall();
    const worker = async () => {
      for (let i = queue.shift(); i !== undefined; i = queue.shift()) {
        const plain = await source.read(i * geo.chunk_size, len(i));
        if (plain.length !== len(i)) throw new Error('le fichier a changé pendant l’envoi');
        const sealed = core.sealChunk(file.sourceId, i, plain);
        plain.fill(0);
        await this.call('PUT', `${BLOBS_PATH}/${geo.blob_id}/chunks/${i}`, sealed, stall);
        reporter.add(len(i));
      }
    };
    await allOrFirstError(Array.from({ length: this.t.parallel }, worker));
    return { env: core.sealFile(file.sourceId), blobId: geo.blob_id };
  }

  async uploadSent(sourceId: string): Promise<void> {
    this.deps.core.forgetUpload(sourceId);
    await this.deps.persistVault();
  }

  async cancelUpload(sourceId: string, size: number): Promise<void> {
    const existing = this.deps.core.upload(sourceId, size);
    if (existing) {
      const up = JSON.parse(existing) as Upload;
      try {
        await this.call('DELETE', `${BLOBS_PATH}/${up.blob_id}`, new Uint8Array(), new Stall());
      } catch (e) {
        if (!(e instanceof ExpiredError)) throw e;
      }
    }
    await this.uploadSent(sourceId);
  }

  async chunk(blobId: string, index: number): Promise<Uint8Array> {
    const path = `${BLOBS_PATH}/${blobId}/chunks/${index}`;
    return this.call('GET', path, new Uint8Array(), new Stall());
  }

  async done(blobId: string): Promise<void> {
    try {
      await this.call('POST', `${BLOBS_PATH}/${blobId}/done`, new Uint8Array(), new Stall());
    } catch (e) {
      if (!(e instanceof ExpiredError)) throw e;
    }
  }
}

async function hashSource(source: ByteSource, chunkSize: number): Promise<Uint8Array> {
  const hasher = new Hasher();
  try {
    for (let offset = 0; offset < source.size; offset += chunkSize) {
      const len = Math.min(chunkSize, source.size - offset);
      const data = await source.read(offset, len);
      if (data.length !== len) throw new Error('fichier plus court que prévu');
      hasher.update(data);
      data.fill(0);
    }
    return hasher.finish();
  } finally {
    hasher.free();
  }
}

async function allOrFirstError(tasks: Promise<void>[]): Promise<void> {
  const results = await Promise.allSettled(tasks);
  const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
  if (failed) throw failed.reason;
}

function relayMessage(bytes: Uint8Array): string | undefined {
  try {
    return (JSON.parse(new TextDecoder().decode(bytes)) as { message?: string }).message;
  } catch {
    return undefined;
  }
}
