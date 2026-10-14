// Cœur du service worker, sans API Chrome (testé sous Node contre un vrai
// relais) : appareil, coffre et cœur WebAssembly, client du protocole,
// historique. `main.ts` y branche menus, notifications, alarmes et messages.

import {
  Core,
  Hasher,
  type QrCode,
  Vault,
  decodeFrame,
  newDeviceId,
  parseJoin,
  qrCode,
  version,
} from '../wasm/ot_wasm.js';
import { HISTORY_LEN } from '../shared/api';
import type {
  Connection,
  DeviceView,
  FileView,
  GroupView,
  Item,
  PendingView,
  Push,
  QrView,
  RelayDevice,
  StateView,
  TransferView,
} from '../shared/api';
import { AUTO_CACHE_MAX, isViewable } from '../shared/files';
import { type ClientEvent, type ClientStorage, RelayClient, type Timing } from './client';
import { checkServerUrl, decode, wsUrl } from './frames';
import { type KeySource, type KeyValue, SecretStore } from './secrets';
import {
  type ByteSource,
  ExpiredError,
  type Geometry,
  type TransferTiming,
  Transfers,
} from './transfer';

export { HISTORY_LEN };
/** Historique : contenu en clair, donc rangé chiffré comme les clés. */
const HISTORY = 'history.json';
const DEVICE = 'device';
const RECIPIENT = 'recipient';
const OUTBOX = 'outbox';
const SEEN = 'seen';
/** Envois de fichiers pas terminés, repris au réveil ; rangés chiffrés (noms). */
const OUTGOING = 'outgoing.json';
/** Délai maximal de l'enregistrement par code. */
const REDEEM_TIMEOUT = 10_000;
export { AUTO_CACHE_MAX };

/** Fichiers de l'appareil (OPFS dans l'extension, mémoire dans les tests). */
export interface FileStorage {
  /** Fichier à envoyer, déjà copié sous `path`. */
  source(path: string): Promise<ByteSource>;
  remove(path: string): Promise<void>;
  /** Copie **chiffrée** des fichiers reçus (note 0029) : un morceau, s'il y est. */
  getChunk(blobId: string, index: number): Promise<Uint8Array | undefined>;
  /** Range un morceau chiffré (entier ou absent). */
  putChunk(blobId: string, index: number, sealed: Uint8Array): Promise<void>;
  removeCache(blobId: string): Promise<void>;
  /** Fichiers reçus dont une copie existe. */
  listCache(): Promise<string[]>;
}

/** Plage demandée par `Range: bytes=…`, bornes comprises. */
function parseRange(
  header: string | null,
  size: number,
): { start: number; end: number } | undefined {
  const m = header ? /^bytes=(\d*)-(\d*)$/.exec(header.trim()) : null;
  if (!m || size === 0 || (m[1] === '' && m[2] === '')) return undefined;
  const start = m[1] === '' ? Math.max(size - Number(m[2]), 0) : Number(m[1]);
  const end = m[1] !== '' && m[2] !== '' ? Math.min(Number(m[2]), size - 1) : size - 1;
  return start <= end && start < size ? { start, end } : undefined;
}

export interface EngineDeps {
  kv: KeyValue;
  keys: KeySource;
  files?: FileStorage;
  webSocket?(url: string): WebSocket;
  fetch?: typeof fetch;
  timing?: Partial<Timing>;
  transferTiming?: Partial<TransferTiming>;
  /**
   * Accès HTTP à l'origine du relais (permission d'hôte, note 0027). Sans
   * lui, le navigateur bloque toute requête de fichier. Absent : accordé.
   */
  relayAllowed?(serverUrl: string): Promise<boolean>;
  log?(message: string, error?: unknown): void;
}

/** Message d'un fichier bloqué faute d'accès au relais. */
export const NO_RELAY_ACCESS = 'accès au relais pas encore autorisé : « Autoriser » dans la popup';

/** Envoi de fichier en cours, rangé pour reprendre après l'arrêt du service worker. */
interface Outgoing {
  path: string;
  name: string;
  mime: string;
  size: number;
  to: string;
}

type Pending = { kind: 'offer'; qr: string; expires_at: string } | { kind: 'join' };

function qrView(text: string): QrView {
  const qr: QrCode = qrCode(text);
  try {
    return { size: qr.size, modules: Array.from(qr.modules) };
  } finally {
    qr.free();
  }
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

class KvClientStorage implements ClientStorage {
  constructor(private readonly kv: KeyValue) {}
  async loadOutbox(): Promise<string[]> {
    const v = (await this.kv.get(OUTBOX))[OUTBOX];
    return Array.isArray(v) ? (v as string[]) : [];
  }
  saveOutbox(envelopes: string[]): Promise<void> {
    return this.kv.set({ [OUTBOX]: envelopes });
  }
  async loadSeen(): Promise<string[]> {
    const v = (await this.kv.get(SEEN))[SEEN];
    return Array.isArray(v) ? (v as string[]) : [];
  }
  saveSeen(ids: string[]): Promise<void> {
    return this.kv.set({ [SEEN]: ids });
  }
}

export class Engine {
  private readonly secrets: SecretStore;
  private vault = new Vault();
  private core?: Core;
  private client?: RelayClient;
  private device?: DeviceView;
  private recipient = '';
  private connection: Connection = 'offline';
  private connectionError?: string;
  private readonly listeners = new Set<(push: Push) => void>();
  private transfers?: Transfers;
  private readonly progress = new Map<string, TransferView>();
  /** Transferts lancés dans cette vie du service worker. */
  private readonly running = new Set<string>();
  private relayAccess = true;
  /** Écritures du coffre et de l'historique, dans l'ordre. */
  private writes: Promise<void> = Promise.resolve();

  private constructor(private readonly deps: EngineDeps) {
    this.secrets = new SecretStore(deps.kv, deps.keys);
  }

  /** Relit l'appareil et ses secrets, et se connecte s'il est configuré. */
  static async open(deps: EngineDeps): Promise<Engine> {
    const engine = new Engine(deps);
    await engine.load();
    return engine;
  }

  private log(msg: string, error?: unknown): void {
    this.deps.log?.(msg, error);
  }

  private async load(): Promise<void> {
    const items = await this.deps.kv.get([DEVICE, RECIPIENT]);
    const device = items[DEVICE] as DeviceView | undefined;
    this.recipient = typeof items[RECIPIENT] === 'string' ? items[RECIPIENT] : '';
    try {
      for (const [name, value] of await this.secrets.loadAll()) {
        if (name !== HISTORY && name !== OUTGOING) this.vault.load(name, value);
        value.fill(0);
      }
      if (device) this.openCore(device.id);
    } catch (e) {
      // Clé de stockage perdue (profil restauré sans IndexedDB…) ou état
      // corrompu : l'appareil est inutilisable, mais l'extension doit rester
      // ouvrable pour l'oublier et l'appairer à nouveau.
      this.log('secrets illisibles', e);
      this.core?.free();
      this.core = undefined;
      this.vault.free();
      this.vault = new Vault();
      this.connectionError = `Clés de cet appareil illisibles (${message(e)}) : l'appairer à nouveau.`;
      return;
    }
    this.device = device;
    if (device) {
      this.startClient();
      void this.pruneCache().catch((e: unknown) => this.log('nettoyage des fichiers', e));
    }
  }

  private openCore(deviceId: string): void {
    this.core?.free();
    this.core = Core.open(this.vault, deviceId);
    const startup = JSON.parse(this.core.startupOutput()) as { events: ClientEvent[] };
    for (const e of startup.events) void this.onClientEvent(e);
  }

  // --- Écoute (popup ouverte, menus, notifications) ---

  onChange(listener: (push: Push) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private push(p: Push): void {
    for (const l of this.listeners) {
      try {
        l(p);
      } catch (e) {
        this.log('écouteur en échec', e);
      }
    }
  }

  private changed(): void {
    this.push({ type: 'state', state: this.state() });
  }

  // --- État ---

  group(): GroupView | undefined {
    const json = this.core?.group();
    return json ? (JSON.parse(json) as GroupView) : undefined;
  }

  private pending(): Pending | undefined {
    const json = this.core?.pending();
    return json ? (JSON.parse(json) as Pending) : undefined;
  }

  state(): StateView {
    const group = this.group();
    const pending = this.pending();
    let pendingView: PendingView | undefined;
    if (pending?.kind === 'offer' && !this.offerExpired()) {
      pendingView = {
        kind: 'offer',
        qr: qrView(pending.qr),
        text: pending.qr,
        expiresAt: pending.expires_at,
      };
    } else if (pending?.kind === 'join') {
      pendingView = { kind: 'join' };
    }
    return {
      version: version(),
      phase: !this.device ? 'unconfigured' : group ? 'ready' : 'pairing',
      device: this.device,
      connection: this.connection,
      connectionError: this.connectionError,
      pending: pendingView,
      group,
      recipient: this.recipient,
      pendingSends: this.client?.pendingSends ?? 0,
      transfers: [...this.progress.values()],
      relayAccess: this.relayAccess,
    };
  }

  /** Nom d'un membre, ou son identifiant s'il n'est pas (plus) au roster. */
  nameOf(deviceId: string): string {
    return this.group()?.members.find((m) => m.device_id === deviceId)?.name ?? deviceId;
  }

  // --- Stockage ---

  private serialWrite(task: () => Promise<void>): Promise<void> {
    const run = this.writes.then(task);
    this.writes = run.catch(() => undefined);
    return run;
  }

  /** Chiffre et range ce qui a changé dans le coffre. */
  persistVault(): Promise<void> {
    return this.serialWrite(async () => {
      for (const change of this.vault.takeChanges()) {
        const value = change.value;
        try {
          if (value) {
            await this.secrets.put(change.name, value);
            value.fill(0);
          } else {
            await this.secrets.delete(change.name);
          }
        } finally {
          change.free();
        }
      }
    });
  }

  async history(): Promise<Item[]> {
    await this.writes;
    return this.readHistory();
  }

  private async readHistory(): Promise<Item[]> {
    const bytes = await this.secrets.get(HISTORY);
    if (!bytes) return [];
    try {
      return JSON.parse(new TextDecoder().decode(bytes)) as Item[];
    } finally {
      bytes.fill(0);
    }
  }

  private addToHistory(item: Item): Promise<void> {
    return this.serialWrite(async () => {
      const items = (await this.readHistory()).filter((i) => i.msgId !== item.msgId);
      items.unshift(item);
      const bytes = new TextEncoder().encode(JSON.stringify(items.slice(0, HISTORY_LEN)));
      await this.secrets.put(HISTORY, bytes);
      bytes.fill(0);
    });
  }

  clearHistory(): Promise<void> {
    return this.serialWrite(() => this.secrets.delete(HISTORY));
  }

  async item(msgId: string): Promise<Item | undefined> {
    return (await this.history()).find((i) => i.msgId === msgId);
  }

  async setRecipient(to: string): Promise<void> {
    this.recipient = to;
    await this.deps.kv.set({ [RECIPIENT]: to });
    this.changed();
  }

  // --- Connexion ---

  private startClient(): void {
    const device = this.device;
    const core = this.core;
    if (!device || !core) return;
    this.client?.stop();
    const client = new RelayClient({
      serverUrl: device.serverUrl,
      deviceId: device.id,
      auth: this.vault,
      core,
      decodeFrame,
      persistVault: () => this.persistVault(),
      storage: new KvClientStorage(this.deps.kv),
      onEvent: (e) => this.onClientEvent(e),
      authRetryDelay: () => this.authRetryDelay(),
      webSocket: this.deps.webSocket,
      timing: this.deps.timing,
      log: this.deps.log,
    });
    this.client = client;
    this.transfers = new Transfers({
      serverUrl: device.serverUrl,
      deviceId: device.id,
      sign: (method, path, body, now) => this.vault.signHttp(device.id, method, path, body, now),
      core,
      persistVault: () => this.persistVault(),
      fetch: this.deps.fetch,
      timing: this.deps.transferTiming,
    });
    this.connection = 'connecting';
    void client.start().catch((e: unknown) => this.log('démarrage du client', e));
    void this.refreshRelayAccess().then(() => this.resumeOutgoing());
  }

  /**
   * Relit la permission d'accès au relais ; si elle vient d'être accordée,
   * relance les envois et les téléchargements qu'elle bloquait.
   */
  async refreshRelayAccess(): Promise<boolean> {
    const url = this.device?.serverUrl;
    const allowed = !url || !this.deps.relayAllowed || (await this.deps.relayAllowed(url));
    const granted = allowed && !this.relayAccess;
    this.relayAccess = allowed;
    this.changed();
    if (granted) {
      void this.resumeOutgoing();
      for (const item of await this.history()) {
        const f = item.file;
        if (f && !f.cached && f.error === NO_RELAY_ACCESS && f.size <= AUTO_CACHE_MAX) {
          void this.cacheFile(item.msgId);
        }
      }
    }
    return allowed;
  }

  /**
   * Après un refus : en attendant que le parrain enregistre l'appareil (QR
   * inversé), réessayer toutes les 1 à 3 s jusqu'à l'expiration de l'offre ;
   * sinon (appareil révoqué…), toutes les 25 s.
   */
  private authRetryDelay(): number | null {
    const pending = this.pending();
    if (pending?.kind === 'offer') {
      return Date.parse(pending.expires_at) > Date.now() ? 1000 + Math.random() * 2000 : null;
    }
    return 25_000;
  }

  /**
   * Réveil (alarme, démarrage de Chrome) : se reconnecter s'il le faut. Pas
   * après l'expiration d'un QR inversé : l'appareil n'existe pas sur le relais.
   */
  ensureConnected(): void {
    const client = this.client;
    if (!client || this.offerExpired()) return;
    void client.start().then(() => client.reconnectNow());
  }

  /** Appairage par QR inversé commencé, mais ni groupe ni offre valable. */
  private offerExpired(): boolean {
    if (!this.device || this.group()) return false;
    const pending = this.pending();
    return !pending || (pending.kind === 'offer' && Date.parse(pending.expires_at) <= Date.now());
  }

  private async onClientEvent(e: ClientEvent): Promise<void> {
    switch (e.type) {
      case 'connected':
        this.connection = 'online';
        this.connectionError = undefined;
        break;
      case 'disconnected':
        this.connection = 'offline';
        this.connectionError = e.reason;
        break;
      case 'auth_failed':
        this.connection = 'auth_failed';
        this.connectionError = e.message;
        break;
      case 'message': {
        const item: Item = {
          msgId: e.msg_id,
          from: e.from,
          fromName: this.nameOf(e.from),
          createdAt: e.created_at,
          kind: e.kind,
          body: e.body,
          to: e.to ?? [],
        };
        // Rangé avant l'acquittement : le client attend la fin de ce
        // traitement.
        await this.addToHistory(item);
        this.push({ type: 'item', item });
        return;
      }
      case 'file': {
        const item: Item = {
          msgId: e.msg_id,
          from: e.from,
          fromName: this.nameOf(e.from),
          createdAt: e.created_at,
          kind: 'file',
          body: e.name,
          file: { blobId: e.blob_id, name: e.name, mime: e.mime, size: e.size },
          to: e.to ?? [],
        };
        // L'offre (clé comprise) est déjà dans le coffre, rangé par le client
        // avant l'acquittement ; l'élément aussi, ici.
        await this.addToHistory(item);
        this.push({ type: 'item', item });
        void this.pruneCache().catch((err: unknown) => this.log('nettoyage des fichiers', err));
        // Petit fichier : copié (chiffré) d'office, sans retarder
        // l'acquittement ; sans accès au relais, noté en attente.
        if (e.size <= AUTO_CACHE_MAX && this.deps.files && !this.relayAccess) {
          await this.updateFile(e.msg_id, { error: NO_RELAY_ACCESS });
        } else if (e.size <= AUTO_CACHE_MAX && this.deps.files) {
          void this.cacheFile(e.msg_id);
        }
        return;
      }
      case 'rejected':
        this.log(`message de ${this.nameOf(e.from)} rejeté : ${e.reason}`);
        return;
      case 'keyring_error':
        this.log(`stockage des clés : ${e.message}`);
        return;
      case 'send_failed':
        this.log(`envoi refusé par le relais : ${e.code}`);
        break;
      case 'accepted':
        // Un message envoyé depuis cette extension n'est plus « en cours ».
        await this.updateItem(e.msg_id, { pending: false }, (i) => i.pending === true);
        break;
      case 'group_changed':
      case 'pairing_completed':
        break;
    }
    this.changed();
  }

  // --- Appairage ---

  private async resetDevice(): Promise<void> {
    this.client?.stop();
    this.client = undefined;
    this.core?.free();
    this.core = undefined;
    this.vault.clear();
    await this.persistVault();
    await this.secrets.clear();
    await this.deps.kv.remove([DEVICE, OUTBOX, SEEN]);
    this.progress.clear();
    this.transfers = undefined;
    this.device = undefined;
    this.connection = 'offline';
    this.connectionError = undefined;
  }

  private ensureNotInGroup(): void {
    if (this.group())
      throw new Error('cette extension est déjà dans un groupe : l’oublier d’abord');
  }

  /**
   * Nouvel appareil, QR inversé : crée l'identité et affiche le QR que le
   * téléphone scanne. L'extension réessaie de s'authentifier jusqu'à ce que
   * le parrain l'ait fait enregistrer (10 minutes au plus).
   */
  async setupOffer(serverUrl: string, name: string): Promise<StateView> {
    this.ensureNotInGroup();
    const url = checkServerUrl(serverUrl);
    await this.resetDevice();
    const id = newDeviceId();
    this.vault.ensureIdentity();
    this.openCore(id);
    try {
      (this.core as Core).offer(name.trim());
    } catch (e) {
      await this.resetDevice();
      throw e;
    }
    await this.persistVault();
    this.device = { id, name: name.trim(), serverUrl: url };
    await this.deps.kv.set({ [DEVICE]: this.device });
    this.startClient();
    this.changed();
    return this.state();
  }

  /**
   * Nouvel appareil, code collé (`ot1-…` ou QR « rejoindre ») : s'enregistre
   * auprès du relais avec le jeton, puis envoie la demande au parrain.
   */
  async setupCode(text: string, serverUrl: string, name: string): Promise<StateView> {
    this.ensureNotInGroup();
    const join = JSON.parse(parseJoin(text)) as { server_url: string | null; code: string };
    const url = checkServerUrl(join.server_url ?? serverUrl);
    const deviceName = name.trim();
    await this.resetDevice();
    this.vault.ensureIdentity();
    await this.persistVault();
    let redeemed: { device_id: string; sponsor: string };
    try {
      redeemed = await this.redeem(url, join.code, deviceName);
    } catch (e) {
      await this.resetDevice();
      throw e;
    }
    this.openCore(redeemed.device_id);
    const env = (this.core as Core).startJoin(join.code, redeemed.sponsor, deviceName);
    await this.persistVault();
    this.device = { id: redeemed.device_id, name: deviceName, serverUrl: url };
    await this.deps.kv.set({ [DEVICE]: this.device });
    this.startClient();
    await this.client?.queue(env);
    this.changed();
    return this.state();
  }

  /** `pair_redeem` sur une connexion à part, que le relais ferme ensuite. */
  private redeem(
    serverUrl: string,
    code: string,
    name: string,
  ): Promise<{ device_id: string; sponsor: string }> {
    return new Promise((resolve, reject) => {
      const url = wsUrl(serverUrl);
      const ws = this.deps.webSocket ? this.deps.webSocket(url) : new WebSocket(url);
      let settled = false;
      const finish = (error: Error | null, value?: { device_id: string; sponsor: string }) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        ws.close();
        if (error) reject(error);
        else if (value) resolve(value);
      };
      const timer = setTimeout(
        () => finish(new Error('pas de réponse du relais : vérifier Tailscale et l’URL')),
        REDEEM_TIMEOUT,
      );
      ws.onmessage = (ev: MessageEvent) => {
        if (typeof ev.data !== 'string') return;
        try {
          const frame = decode(ev.data, decodeFrame);
          if (frame.t === 'challenge') {
            ws.send(this.vault.pairRedeemFrame(frame.nonce, code, name));
          } else if (frame.t === 'pair_redeemed') {
            finish(null, { device_id: frame.device_id, sponsor: frame.sponsor });
          } else if (frame.t === 'error') {
            finish(new Error(`appairage refusé par le relais : ${frame.message}`));
          }
        } catch (e) {
          finish(new Error(message(e)));
        }
      };
      ws.onclose = () =>
        finish(new Error('connexion au relais impossible : vérifier Tailscale et l’URL'));
    });
  }

  /** Abandonne un appairage en cours, ou oublie l'appareil. */
  async forget(): Promise<StateView> {
    await this.resetDevice();
    await this.clearHistory();
    this.changed();
    return this.state();
  }

  async setServerUrl(url: string): Promise<StateView> {
    if (!this.device) throw new Error('appareil pas encore appairé');
    this.device = { ...this.device, serverUrl: checkServerUrl(url) };
    await this.deps.kv.set({ [DEVICE]: this.device });
    this.startClient();
    this.changed();
    return this.state();
  }

  // --- Envoi ---

  /** Chiffre et envoie un texte à `to` (un membre) ou à tous (`''`). */
  async send(text: string, to: string): Promise<string> {
    const core = this.core;
    const client = this.client;
    if (!core || !client || !this.group()) {
      throw new Error("cette extension n'est pas encore dans un groupe");
    }
    const env = core.sealText(text, to ? [to] : []);
    const trimmed = text.trim();
    // Comme `ot_core::classify` : un lien s'il n'est fait que d'une URL web.
    const kind = /^https?:\/\/\S+$/i.test(trimmed) ? 'link' : 'text';
    await this.recordSent(env, { kind, body: kind === 'link' ? trimmed : text }, to);
    const msgId = await client.queue(env);
    this.changed();
    return msgId;
  }

  /**
   * Range un élément envoyé dans l'historique (note 0030), « en cours »
   * jusqu'à l'accusé du relais. Rangé **avant** la mise en file : l'accusé
   * pourrait sinon arriver avant l'élément.
   */
  private async recordSent(
    env: string,
    content: Pick<Item, 'kind' | 'body' | 'file'>,
    to: string,
  ): Promise<void> {
    const device = this.device;
    if (!device) return;
    const parsed = JSON.parse(env) as { msg_id: string; created_at: string };
    const item: Item = {
      msgId: parsed.msg_id,
      from: device.id,
      fromName: device.name,
      createdAt: parsed.created_at,
      ...content,
      outgoing: true,
      to: to ? [to] : [],
      pending: true,
    };
    await this.addToHistory(item);
    this.push({ type: 'item', item });
  }

  // --- Fichiers (étape 5) ---

  private setProgress(view: TransferView | undefined, id: string): void {
    if (view) this.progress.set(id, view);
    else this.progress.delete(id);
    this.changed();
  }

  private ready(): { client: RelayClient; transfers: Transfers; files: FileStorage } {
    const { client, transfers } = this;
    const files = this.deps.files;
    if (!client || !transfers || !this.group() || !files) {
      throw new Error("cette extension n'est pas encore dans un groupe");
    }
    return { client, transfers, files };
  }

  /**
   * Envoie un fichier déjà copié sous `path` à `to` (un membre) ou à tous
   * (`''`). Rangé d'abord dans `outgoing` : un arrêt du service worker
   * reprend l'envoi au réveil. Renvoie le `msg_id` de l'annonce.
   */
  async sendFile(file: Outgoing): Promise<string> {
    this.ready();
    if (!(await this.refreshRelayAccess())) throw new Error(NO_RELAY_ACCESS);
    await this.editOutgoing((list) =>
      list.some((o) => o.path === file.path) ? list : [...list, file],
    );
    return this.runUpload(file);
  }

  private async outgoing(): Promise<Outgoing[]> {
    await this.writes;
    return this.readOutgoing();
  }

  private async readOutgoing(): Promise<Outgoing[]> {
    const bytes = await this.secrets.get(OUTGOING);
    if (!bytes) return [];
    try {
      return JSON.parse(new TextDecoder().decode(bytes)) as Outgoing[];
    } finally {
      bytes.fill(0);
    }
  }

  private editOutgoing(edit: (list: Outgoing[]) => Outgoing[]): Promise<void> {
    return this.serialWrite(async () => {
      const list = edit(await this.readOutgoing());
      if (list.length === 0) {
        await this.secrets.delete(OUTGOING);
        return;
      }
      const bytes = new TextEncoder().encode(JSON.stringify(list));
      await this.secrets.put(OUTGOING, bytes);
      bytes.fill(0);
    });
  }

  private async runUpload(file: Outgoing): Promise<string> {
    const { client, transfers, files } = this.ready();
    if (this.running.has(file.path)) throw new Error('envoi déjà en cours');
    this.running.add(file.path);
    const view = (done: number, error?: string): TransferView => ({
      id: file.path,
      direction: 'up',
      name: file.name,
      done,
      total: file.size,
      error,
    });
    try {
      const source = await files.source(file.path);
      const { env, blobId } = await transfers.upload(
        { sourceId: file.path, name: file.name, mime: file.mime, to: file.to ? [file.to] : [] },
        source,
        (done) => this.setProgress(view(done), file.path),
      );
      const fileView = { blobId, name: file.name, mime: file.mime, size: file.size };
      await this.recordSent(env, { kind: 'file', body: file.name, file: fileView }, file.to);
      const msgId = await client.queue(env);
      await transfers.uploadSent(file.path);
      await this.editOutgoing((list) => list.filter((o) => o.path !== file.path));
      await files.remove(file.path);
      this.setProgress(undefined, file.path);
      return msgId;
    } catch (e) {
      this.setProgress(view(0, message(e)), file.path);
      throw e;
    } finally {
      this.running.delete(file.path);
    }
  }

  /** Reprend les envois interrompus par l'arrêt du service worker. */
  private async resumeOutgoing(): Promise<void> {
    if (!this.deps.files || !this.group() || !this.relayAccess) return;
    for (const file of await this.outgoing()) {
      if (this.running.has(file.path)) continue;
      void this.runUpload(file).catch((e: unknown) => this.log('reprise d’un envoi', e));
    }
  }

  /** Abandonne un envoi (libère la place sur le relais). */
  async cancelUpload(path: string): Promise<void> {
    const { transfers, files } = this.ready();
    const file = (await this.outgoing()).find((o) => o.path === path);
    if (file) await transfers.cancelUpload(path, file.size);
    await this.editOutgoing((list) => list.filter((o) => o.path !== path));
    await files.remove(path);
    this.setProgress(undefined, path);
  }

  /** Met à jour le fichier d'un élément de l'historique (copie, erreur…). */
  updateFile(msgId: string, change: Partial<FileView>): Promise<Item | undefined> {
    return this.updateItem(
      msgId,
      {},
      (i) => !!i.file,
      (i) => {
        i.file = { ...(i.file as FileView), ...change };
      },
    );
  }

  /**
   * Met à jour un élément de l'historique, s'il existe et vérifie `when`, et
   * le pousse à la popup.
   */
  private async updateItem(
    msgId: string,
    change: Partial<Item>,
    when: (item: Item) => boolean = () => true,
    edit?: (item: Item) => void,
  ): Promise<Item | undefined> {
    let updated: Item | undefined;
    await this.serialWrite(async () => {
      const items = await this.readHistory();
      const item = items.find((i) => i.msgId === msgId);
      if (!item || !when(item)) return;
      Object.assign(item, change);
      edit?.(item);
      updated = item;
      const bytes = new TextEncoder().encode(JSON.stringify(items));
      await this.secrets.put(HISTORY, bytes);
      bytes.fill(0);
    });
    if (updated) this.push({ type: 'item', item: updated });
    return updated;
  }

  // --- Fichiers reçus, servis par le service worker (note 0029) ---

  private geometry(blobId: string): Geometry {
    const core = this.core;
    if (!core) throw new Error("cette extension n'est pas encore dans un groupe");
    return JSON.parse(core.fileGeometry(blobId)) as Geometry;
  }

  /**
   * Morceau chiffré `index` : la copie de l'OPFS, sinon le relais (et il
   * rejoint la copie).
   */
  private async sealedChunk(blobId: string, index: number): Promise<Uint8Array> {
    const { transfers, files } = this.ready();
    const cached = await files.getChunk(blobId, index);
    if (cached) return cached;
    if (!this.relayAccess && !(await this.refreshRelayAccess())) throw new Error(NO_RELAY_ACCESS);
    const sealed = await transfers.chunk(blobId, index);
    await files.putChunk(blobId, index, sealed);
    return sealed;
  }

  /**
   * Morceaux déchiffrés `from` à `to` (compris), dans l'ordre, quatre retirés
   * d'avance. Chaque morceau est authentifié (STREAM) par le WebAssembly.
   */
  private async *plainChunks(blobId: string, from: number, to: number): AsyncGenerator<Uint8Array> {
    const core = this.core;
    if (!core) throw new Error("cette extension n'est pas encore dans un groupe");
    const pending: Promise<Uint8Array>[] = [];
    let next = from;
    const fill = () => {
      while (next <= to && pending.length < 4) {
        const p = this.sealedChunk(blobId, next++);
        // Une erreur sera relevée à son tour ; pas de rejet non traité.
        p.catch(() => undefined);
        pending.push(p);
      }
    };
    fill();
    for (let i = from; i <= to; i++) {
      const sealed = await (pending.shift() as Promise<Uint8Array>);
      fill();
      yield core.openChunk(blobId, i, sealed);
    }
  }

  /**
   * Fichier lu en entier : son SHA-256 est vérifié ; s'il concorde, le relais
   * peut l'effacer et la copie est complète. Sinon la copie est effacée.
   */
  private async verified(blobId: string, msgId: string, sha256: Uint8Array): Promise<void> {
    const { transfers, files } = this.ready();
    if (!this.core?.checkFile(blobId, sha256)) {
      await files.removeCache(blobId);
      throw new Error('fichier altéré : son empreinte SHA-256 ne correspond pas');
    }
    try {
      await transfers.done(blobId);
    } catch (e) {
      this.log('fin du retrait non signalée au relais', e);
    }
    await this.updateFile(msgId, { cached: true, error: undefined });
  }

  /**
   * Copie chiffrée d'un fichier reçu, morceau par morceau, puis vérification
   * (petits fichiers, à la réception). Sans effet si elle existe déjà.
   */
  async cacheFile(msgId: string): Promise<void> {
    const item = await this.item(msgId);
    const file = item?.file;
    if (!file || file.cached || this.running.has(msgId)) return;
    if (!(await this.refreshRelayAccess())) {
      await this.updateFile(msgId, { error: NO_RELAY_ACCESS });
      return;
    }
    this.running.add(msgId);
    const hasher = new Hasher();
    let done = 0;
    try {
      const g = this.geometry(file.blobId);
      for await (const plain of this.plainChunks(file.blobId, 0, g.chunks - 1)) {
        hasher.update(plain);
        done += plain.length;
        plain.fill(0);
        this.setProgress(
          { id: msgId, direction: 'down', name: file.name, done, total: file.size },
          msgId,
        );
      }
      await this.verified(file.blobId, msgId, hasher.finish());
    } catch (e) {
      const error =
        e instanceof ExpiredError || (e instanceof Error && e.message === NO_RELAY_ACCESS)
          ? e.message
          : `réception impossible : ${message(e)}`;
      this.log('copie d’un fichier reçu', e);
      await this.updateFile(msgId, { error });
    } finally {
      hasher.free();
      this.running.delete(msgId);
      this.setProgress(undefined, msgId);
    }
  }

  /**
   * Réponse à l'adresse `/f/<blob_id>/…` (note 0029) : le fichier en clair,
   * en flux, depuis la copie chiffrée ou le relais. `inline` pour un type que
   * le navigateur affiche, sinon (ou `download`) `attachment`. `Range` pour
   * les vidéos. Une lecture complète vérifie le SHA-256 et complète la copie.
   */
  async fileResponse(
    blobId: string,
    opts: { download: boolean; range: string | null },
  ): Promise<Response> {
    const notFound = () =>
      new Response('Fichier inconnu ou expiré : il n’est plus sur cet appareil.', {
        status: 404,
        headers: { 'Content-Type': 'text/plain; charset=utf-8' },
      });
    const item = (await this.history()).find((i) => i.file?.blobId === blobId);
    const file = item?.file;
    if (!item || !file) return notFound();
    let g: Geometry;
    try {
      g = this.geometry(blobId);
    } catch {
      return notFound();
    }
    const range = parseRange(opts.range, g.size);
    const start = range?.start ?? 0;
    const end = range?.end ?? g.size - 1;
    const first = Math.floor(start / g.chunk_size);
    const last = g.size === 0 ? 0 : Math.floor(end / g.chunk_size);
    const chunks = this.plainChunks(blobId, first, last);
    // Seule une lecture complète permet de vérifier l'empreinte.
    let hasher: Hasher | undefined = range ? undefined : new Hasher();
    const release = () => {
      hasher?.free();
      hasher = undefined;
    };
    let index = first;
    const body = new ReadableStream<Uint8Array>({
      pull: async (controller) => {
        try {
          const next = await chunks.next();
          if (next.done) {
            const digest = hasher?.finish();
            release();
            if (digest) await this.verified(blobId, item.msgId, digest);
            controller.close();
            return;
          }
          const plain = next.value;
          hasher?.update(plain);
          const offset = index * g.chunk_size;
          index++;
          controller.enqueue(
            plain.slice(Math.max(start - offset, 0), Math.min(end - offset + 1, plain.length)),
          );
          plain.fill(0);
        } catch (e) {
          release();
          this.log('lecture d’un fichier reçu', e);
          controller.error(e);
        }
      },
      cancel: async () => {
        release();
        await chunks.return(undefined);
      },
    });
    const mime = file.mime || 'application/octet-stream';
    const inline = !opts.download && isViewable(mime);
    const headers = new Headers({
      'Content-Type': mime,
      'Content-Length': String(Math.max(end - start + 1, 0)),
      'Accept-Ranges': 'bytes',
      'Content-Disposition': `${inline ? 'inline' : 'attachment'}; filename*=UTF-8''${encodeURIComponent(file.name)}`,
      'X-Content-Type-Options': 'nosniff',
      'Cache-Control': 'no-store',
    });
    // Servi sous l'origine de l'extension : origine opaque, sans script ni
    // accès aux API (sauf le PDF, lu par le lecteur intégré du navigateur).
    if (mime !== 'application/pdf') headers.set('Content-Security-Policy', 'sandbox');
    if (range) headers.set('Content-Range', `bytes ${start}-${end}/${g.size}`);
    return new Response(body, { status: range ? 206 : 200, headers });
  }

  /**
   * Efface les copies et les clés des fichiers sortis de l'historique (50
   * derniers) ou expirés (7 jours) : les deux partent ensemble (note 0029).
   */
  async pruneCache(): Promise<void> {
    const files = this.deps.files;
    const core = this.core;
    if (!files || !core) return;
    const kept = new Set(
      (await this.history()).map((i) => i.file?.blobId).filter((b): b is string => !!b),
    );
    const offers = JSON.parse(core.files()) as { blob_id: string }[];
    let changed = false;
    for (const o of offers) {
      if (!kept.has(o.blob_id)) {
        core.forgetFile(o.blob_id);
        changed = true;
      }
    }
    const known = new Set(offers.filter((o) => kept.has(o.blob_id)).map((o) => o.blob_id));
    for (const blobId of await files.listCache()) {
      if (!known.has(blobId)) await files.removeCache(blobId);
    }
    if (changed) await this.persistVault();
  }

  async devices(): Promise<RelayDevice[]> {
    if (!this.client) throw new Error('appareil pas encore appairé');
    const group = this.group();
    const list = await this.client.listDevices();
    return list.map((d) => ({
      ...d,
      member: group?.members.some((m) => m.device_id === d.id) ?? false,
    }));
  }

  /** Nouvelle clé de groupe sans les membres `exclude`. */
  async rotate(exclude: string[]): Promise<number> {
    const core = this.core;
    const client = this.client;
    if (!core || !client) throw new Error("cette extension n'est pas encore dans un groupe");
    const out = JSON.parse(core.rotate(exclude)) as { epoch: number; sends: unknown[] };
    await this.persistVault();
    for (const env of out.sends) await client.queue(JSON.stringify(env));
    this.changed();
    return out.epoch;
  }

  /** Pour les tests : arrête la connexion sans rien effacer. */
  close(): void {
    this.client?.stop();
    this.client = undefined;
    this.transfers = undefined;
    this.core?.free();
    this.core = undefined;
  }
}
