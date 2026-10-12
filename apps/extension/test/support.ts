// Outils des tests : WebAssembly chargé sous Node, stockage en mémoire, relais
// `ot-server` et CLI `ot` lancés pour de vrai.

import { execFile, spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { initSync } from '../src/wasm/ot_wasm.js';
import type { ClientStorage } from '../src/background/client';
import type { KeyValue } from '../src/background/secrets';

export const ROOT = path.resolve(__dirname, '../../..');
const run = promisify(execFile);

let wasmReady = false;

/** Charge le WebAssembly construit par `npm run wasm`. */
export function loadWasm(): void {
  if (wasmReady) return;
  const file = path.join(__dirname, '../src/wasm/ot_wasm_bg.wasm');
  initSync({ module: readFileSync(file) });
  wasmReady = true;
}

export function golden(name: string): string {
  return readFileSync(
    path.join(ROOT, `crates/ot-proto/tests/golden/${name}.json`),
    'utf8',
  ).trimEnd();
}

export function vector(name: string): Record<string, unknown> {
  const file = path.join(ROOT, `crates/ot-crypto/tests/vectors/${name}.json`);
  return JSON.parse(readFileSync(file, 'utf8')) as Record<string, unknown>;
}

/** chrome.storage.local en mémoire. */
export class MemoryKV implements KeyValue {
  readonly items = new Map<string, unknown>();

  async get(keys: string | string[] | null): Promise<Record<string, unknown>> {
    const wanted = keys === null ? [...this.items.keys()] : Array.isArray(keys) ? keys : [keys];
    const out: Record<string, unknown> = {};
    for (const k of wanted) if (this.items.has(k)) out[k] = structuredClone(this.items.get(k));
    return out;
  }

  async set(items: Record<string, unknown>): Promise<void> {
    for (const [k, v] of Object.entries(items)) this.items.set(k, structuredClone(v));
  }

  async remove(keys: string | string[]): Promise<void> {
    for (const k of Array.isArray(keys) ? keys : [keys]) this.items.delete(k);
  }
}

export class MemoryClientStorage implements ClientStorage {
  outbox: string[] = [];
  seen: string[] = [];
  async loadOutbox(): Promise<string[]> {
    return [...this.outbox];
  }
  async saveOutbox(envelopes: string[]): Promise<void> {
    this.outbox = [...envelopes];
  }
  async loadSeen(): Promise<string[]> {
    return [...this.seen];
  }
  async saveSeen(ids: string[]): Promise<void> {
    this.seen = [...ids];
  }
}

function binary(name: string): string {
  const exe = process.platform === 'win32' ? `${name}.exe` : name;
  return process.env['OT_BIN_DIR']
    ? path.join(process.env['OT_BIN_DIR'], exe)
    : path.join(ROOT, 'target/debug', exe);
}

async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = createServer();
    srv.once('error', reject);
    srv.listen(0, '127.0.0.1', () => {
      const addr = srv.address();
      srv.close(() =>
        typeof addr === 'object' && addr ? resolve(addr.port) : reject(new Error('port')),
      );
    });
  });
}

export async function eventually<T>(
  what: string,
  check: () => T | undefined | Promise<T | undefined>,
  timeout = 15_000,
): Promise<T> {
  const until = Date.now() + timeout;
  for (;;) {
    const v = await check();
    if (v !== undefined && v !== false) return v;
    if (Date.now() > until) throw new Error(`délai dépassé : ${what}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

/** Relais `ot-server` dans un dossier temporaire. */
export class Relay {
  private constructor(
    readonly dir: string,
    readonly url: string,
    private proc: ChildProcess,
    readonly port: number,
  ) {}

  static async start(): Promise<Relay> {
    const dir = mkdtempSync(path.join(tmpdir(), 'ot-ext-'));
    const port = await freePort();
    const proc = spawn(
      binary('ot-server'),
      ['serve', '--db', path.join(dir, 'relay.db'), '--listen', `127.0.0.1:${port}`],
      { stdio: ['ignore', 'ignore', 'inherit'] },
    );
    const relay = new Relay(dir, `ws://127.0.0.1:${port}`, proc, port);
    await eventually('démarrage du relais', async () => {
      try {
        const res = await fetch(`http://127.0.0.1:${port}/healthz`);
        return res.ok || undefined;
      } catch {
        return undefined;
      }
    });
    return relay;
  }

  async addDevice(name: string, publicKey: string): Promise<string> {
    const { stdout } = await run(binary('ot-server'), [
      '--db',
      path.join(this.dir, 'relay.db'),
      'device',
      'add',
      name,
      publicKey,
    ]);
    const id = stdout.split('\n')[0]?.trim();
    if (!id?.startsWith('dev_')) throw new Error(`device add : ${stdout}`);
    return id;
  }

  /** Arrête le relais (la base reste, pour un redémarrage). */
  async stop(): Promise<void> {
    if (this.proc.exitCode !== null) return;
    const exited = new Promise((r) => this.proc.once('exit', r));
    this.proc.kill();
    await exited;
  }

  /** Relance le relais sur le même port, avec la même base. */
  async restart(): Promise<void> {
    await this.stop();
    this.proc = spawn(
      binary('ot-server'),
      ['serve', '--db', path.join(this.dir, 'relay.db'), '--listen', `127.0.0.1:${this.port}`],
      { stdio: ['ignore', 'ignore', 'inherit'] },
    );
    await eventually('redémarrage du relais', async () => {
      try {
        return (await fetch(`http://127.0.0.1:${this.port}/healthz`)).ok || undefined;
      } catch {
        return undefined;
      }
    });
  }

  async dispose(): Promise<void> {
    await this.stop();
    rmSync(this.dir, { recursive: true, force: true });
  }
}

/** CLI `ot`, premier appareil du groupe (le « téléphone » des tests). */
export class Cli {
  readonly home: string;

  constructor(
    private readonly relay: Relay,
    readonly name: string,
  ) {
    this.home = path.join(relay.dir, name);
  }

  run(...args: string[]): Promise<{ stdout: string; stderr: string }> {
    return run(binary('ot'), ['--home', this.home, ...args], { timeout: 60_000 });
  }

  /** Crée l'appareil, l'enregistre sur le relais et crée le groupe. */
  async setupGroup(): Promise<string> {
    const { stdout } = await this.run('init', '--server', this.relay.url, '--name', this.name);
    const pk = /Clé publique\s*:\s*(\S+)/.exec(stdout)?.[1];
    if (!pk) throw new Error(`ot init : ${stdout}`);
    const id = await this.relay.addDevice(this.name, pk);
    await this.run('init', '--device-id', id);
    await this.run('group', 'init');
    return id;
  }

  /** `ot listen --json` en tâche de fond ; renvoie les messages au fil de l'eau. */
  listen(): { messages: Record<string, unknown>[]; stop(): void } {
    const proc = spawn(binary('ot'), ['--home', this.home, 'listen', '--json'], {
      stdio: ['ignore', 'pipe', 'inherit'],
    });
    const messages: Record<string, unknown>[] = [];
    let buf = '';
    proc.stdout.on('data', (chunk: Buffer) => {
      buf += chunk.toString('utf8');
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).trim();
        buf = buf.slice(i + 1);
        if (line) messages.push(JSON.parse(line) as Record<string, unknown>);
      }
    });
    return { messages, stop: () => proc.kill() };
  }

  /** `ot pair new` en tâche de fond : renvoie le code dès qu'il est affiché. */
  async pairNew(): Promise<{ code: string; done: Promise<string> }> {
    const proc = spawn(binary('ot'), ['--home', this.home, 'pair', 'new'], {
      stdio: ['ignore', 'pipe', 'inherit'],
    });
    let out = '';
    const done = new Promise<string>((resolve, reject) => {
      proc.on('exit', (code) =>
        code === 0 ? resolve(out) : reject(new Error(`ot pair new : ${out}`)),
      );
    });
    proc.stdout.on('data', (c: Buffer) => (out += c.toString('utf8')));
    const code = await eventually('code d’appairage', () => /\b(ot1-\S+)/.exec(out)?.[1]);
    return { code, done };
  }
}
