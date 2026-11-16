// Transferts de fichiers (étape 5, notes 0026 et 0027) : dépôt et retrait par
// morceaux chiffrés, en parallèle, avec reprise. Même logique que
// `ot-core` (transfer.rs) ; clés, chiffrement et signatures restent dans le
// WebAssembly (`Core`, `Vault`), le TypeScript ne fait que le transport.
//
// Sans API Chrome : la source est fournie par l'appelant (OPFS dans
// l'extension, mémoire dans les tests). À la réception, le moteur retire les
// morceaux un par un (`chunk`) pour servir le fichier en flux (note 0029).

import { httpBase } from '../shared/origins';
import { Hasher, fileGeometry } from '../wasm/ot_wasm.js';

/** Morceaux en clair de 1 Mio, comme les clients Rust. */
export const CHUNK_SIZE = 1 << 20;
const BLOBS_PATH = '/v1/blobs';

/** Fichier à envoyer, lu par morceaux. */
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
}

/** Ce que le transfert demande au cœur WebAssembly (`Core`). */
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
  /** Sans aucun progrès pendant ce délai, le transfert échoue. */
  stallTimeout: number;
  /** Durée maximale d'une requête. */
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
  /** Signature des requêtes (`Vault.signHttp`) : en-têtes en JSON. */
  sign(method: string, path: string, body: Uint8Array, nowMs: number): string;
  core: TransferCore;
  /** Range le coffre (clé d'un nouveau dépôt, offre oubliée). */
  persistVault(): Promise<void>;
  fetch?: typeof fetch;
  timing?: Partial<TransferTiming>;
}

export type Progress = (done: number, total: number) => void;

/** Corps de `POST /v1/blobs` (format figé par le golden `http_create_blob`). */
export function createBlobBody(g: Geometry, to: string[]): Record<string, unknown> {
  return { size: g.sealed_size, chunk_size: g.sealed_chunk_size, chunks: g.chunks, to };
}

/** Le fichier n'est plus sur le relais (expiré, annulé). */
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

/** Dernier progrès d'un transfert, partagé par ses tâches. */
class Stall {
  private last = Date.now();
  touch(): void {
    this.last = Date.now();
  }
  since(): number {
    return Date.now() - this.last;
  }
}

/** Progression en octets de clair, appelée au plus tous les 250 ms (et à la fin). */
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

  /**
   * Requête signée. Les erreurs passagères (réseau, 5xx) sont réessayées
   * tant que le transfert a progressé depuis moins de `stallTimeout`.
   */
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
        let message = `erreur HTTP ${res.status}`;
        try {
          message = (JSON.parse(new TextDecoder().decode(bytes)) as { message: string }).message;
        } catch {
          // Corps non JSON : le code HTTP suffit.
        }
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

  /**
   * Dépose le fichier (en reprenant un dépôt interrompu de la même source)
   * et renvoie l'annonce chiffrée à envoyer, avec l'identifiant du fichier.
   * Appeler `uploadSent` une fois l'annonce acceptée par le relais.
   */
  async upload(
    file: { sourceId: string; name: string; mime: string; to: string[] },
    source: ByteSource,
    onProgress?: Progress,
  ): Promise<{ env: string; blobId: string }> {
    const core = this.deps.core;
    let up: Upload | undefined;
    const existing = core.upload(file.sourceId, source.size);
    let missing: number[] | undefined;
    if (existing) {
      up = JSON.parse(existing) as Upload;
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

  /** L'annonce est acceptée : oublier le dépôt. */
  async uploadSent(sourceId: string): Promise<void> {
    this.deps.core.forgetUpload(sourceId);
    await this.deps.persistVault();
  }

  /** Abandonne un dépôt : libère la place sur le relais. */
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

  /**
   * Retire le morceau chiffré `index` d'un fichier annoncé (nouvel essai des
   * erreurs passagères). `ExpiredError` s'il n'est plus sur le relais.
   */
  async chunk(blobId: string, index: number): Promise<Uint8Array> {
    const path = `${BLOBS_PATH}/${blobId}/chunks/${index}`;
    return this.call('GET', path, new Uint8Array(), new Stall());
  }

  /** Tout est retiré et vérifié : le relais peut effacer le fichier. */
  async done(blobId: string): Promise<void> {
    try {
      await this.call('POST', `${BLOBS_PATH}/${blobId}/done`, new Uint8Array(), new Stall());
    } catch (e) {
      // Déjà effacé : rien à faire ; sinon il expirera de lui-même.
      if (!(e instanceof ExpiredError)) throw e;
    }
  }
}

/** SHA-256 d'une source, lue dans l'ordre. */
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

/** Attend toutes les tâches ; la première erreur l'emporte. */
async function allOrFirstError(tasks: Promise<void>[]): Promise<void> {
  const results = await Promise.allSettled(tasks);
  const failed = results.find((r): r is PromiseRejectedResult => r.status === 'rejected');
  if (failed) throw failed.reason;
}
