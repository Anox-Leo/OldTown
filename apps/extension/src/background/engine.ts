import {
  Core,
  Hasher,
  Vault,
  decodeFrame,
  newDeviceId,
  parseInvite,
  parseJoin,
  version,
} from '../wasm/ot_wasm.js';
import { HISTORY_LEN, parseChannel } from '../shared/api';
import type {
  Connection,
  DeviceView,
  DirectoryAlert,
  FileView,
  GroupView,
  InvitationView,
  Item,
  PendingView,
  PersonDeviceView,
  PersonView,
  Push,
  RelayDevice,
  StateView,
  TransferView,
} from '../shared/api';
import { AUTO_CACHE_MAX, isViewable } from '../shared/files';
import {
  type ClientEvent,
  type ClientStorage,
  RelayClient,
  RelayError,
  type Timing,
} from './client';
import { type AdminUser, type ServerFrame, checkServerUrl, decode, wsUrl } from './frames';
import { type CoreOutput, People, qrView } from './people';
import { type KeySource, type KeyValue, SecretStore } from './secrets';
import {
  type ByteSource,
  ExpiredError,
  type Geometry,
  type TransferTiming,
  Transfers,
} from './transfer';

export { HISTORY_LEN };
const HISTORY = 'history.json';
const DEVICE = 'device';
const RECIPIENT = 'recipient';
const OUTBOX = 'outbox';
const SEEN = 'seen';
const OUTGOING = 'outgoing.json';
const OPENED = 'opened.json';
const MAX_ALERTS = 20;
const REDEEM_TIMEOUT = 10_000;
export { AUTO_CACHE_MAX };

export interface FileStorage {
  source(path: string): Promise<ByteSource>;
  remove(path: string): Promise<void>;
  getChunk(blobId: string, index: number): Promise<Uint8Array | undefined>;
  putChunk(blobId: string, index: number, sealed: Uint8Array): Promise<void>;
  removeCache(blobId: string): Promise<void>;
  listCache(): Promise<string[]>;
}

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
  relayAllowed?(serverUrl: string): Promise<boolean>;
  log?(message: string, error?: unknown): void;
}

export const NO_RELAY_ACCESS = 'accès au relais pas encore autorisé : « Autoriser » dans la popup';

interface Outgoing {
  path: string;
  name: string;
  mime: string;
  size: number;
  to: string;
}

type Pending = { kind: 'offer'; qr: string; expires_at: string } | { kind: 'join' };

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
  private readonly running = new Set<string>();
  private relayAccess = true;
  private writes: Promise<void> = Promise.resolve();
  private readonly people: People;
  private alerts: DirectoryAlert[] = [];
  private opened: Record<string, string> = {};
  private inviteError?: string;
  private readonly waiters = new Map<string, { resolve(): void; reject(e: Error): void }>();

  private constructor(private readonly deps: EngineDeps) {
    this.secrets = new SecretStore(deps.kv, deps.keys);
    this.people = new People({
      core: () => this.core,
      client: () => this.client,
      device: () => this.device,
      group: () => this.group(),
      persistVault: () => this.persistVault(),
      apply: (out) => this.applyOutput(out),
      sendAndWait: (env) => this.sendAndWait(env),
      rotate: (exclude) => this.rotate(exclude),
      changed: () => this.changed(),
      log: (m, e) => this.log(m, e),
    });
  }

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
        if (name === OPENED) this.opened = parseOpened(value);
        else if (name !== HISTORY && name !== OUTGOING) this.vault.load(name, value);
        value.fill(0);
      }
      if (device) this.openCore(device.id);
    } catch (e) {
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
    } else if (!group && this.device?.invited) {
      pendingView = { kind: 'invite' };
    }
    const people = this.people.people();
    const me = this.people.me();
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
      people,
      me: me?.user,
      admin: me?.role === 'admin',
      rootMissing: this.people.rootMissing,
      alerts: this.alerts,
      opened: this.opened,
      inviteError: this.inviteError,
    };
  }

  nameOf(deviceId: string): string {
    const member = this.group()?.members.find((m) => m.device_id === deviceId);
    if (member) return member.name;
    for (const p of this.people.people()) {
      const d = p.devices.find((x: PersonDeviceView) => x.device_id === deviceId);
      if (d) return d.name;
    }
    return deviceId;
  }

  person(user: string): PersonView | undefined {
    return this.people.person(user);
  }

  private serialWrite(task: () => Promise<void>): Promise<void> {
    const run = this.writes.then(task);
    this.writes = run.catch(() => undefined);
    return run;
  }

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

  markRead(channel: string): Promise<void> {
    const user = parseChannel(channel).user;
    if (!user) return Promise.resolve();
    this.opened = { ...this.opened, [user]: new Date().toISOString() };
    this.changed();
    const bytes = new TextEncoder().encode(JSON.stringify(this.opened));
    return this.serialWrite(async () => {
      await this.secrets.put(OPENED, bytes);
      bytes.fill(0);
    });
  }

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
      afterDeliver: () => this.people.afterDeliver(),
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

  private authRetryDelay(): number | null {
    const pending = this.pending();
    if (pending?.kind === 'offer') {
      return Date.parse(pending.expires_at) > Date.now() ? 1000 + Math.random() * 2000 : null;
    }
    return 25_000;
  }

  ensureConnected(): void {
    const client = this.client;
    if (!client || this.offerExpired()) return;
    void client.start().then(() => client.reconnectNow());
  }

  private offerExpired(): boolean {
    if (!this.device || this.group() || this.device.invited) return false;
    const pending = this.pending();
    return !pending || (pending.kind === 'offer' && Date.parse(pending.expires_at) <= Date.now());
  }

  private async onClientEvent(e: ClientEvent): Promise<void> {
    switch (e.type) {
      case 'connected':
        this.connection = 'online';
        this.connectionError = undefined;
        this.people.onConnected();
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
          ...origin(e),
        };
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
          ...origin(e),
        };
        await this.addToHistory(item);
        this.push({ type: 'item', item });
        void this.pruneCache().catch((err: unknown) => this.log('nettoyage des fichiers', err));
        if (e.size <= AUTO_CACHE_MAX && this.deps.files && !this.relayAccess) {
          await this.updateFile(e.msg_id, { error: NO_RELAY_ACCESS });
        } else if (e.size <= AUTO_CACHE_MAX && this.deps.files) {
          void this.cacheFile(e.msg_id);
        }
        return;
      }
      case 'otp': {
        const item: Item = {
          msgId: e.msg_id,
          from: e.from,
          fromName: this.nameOf(e.from),
          createdAt: e.created_at,
          kind: 'otp',
          body: e.code,
          otp: {
            display: e.display || e.code,
            senderName: e.sender_name,
            senderDomain: e.sender_domain,
            verified: e.sender_verified,
            expiresAt: e.expires_at * 1000,
          },
          to: e.to ?? [],
          ...origin(e),
          autoFill: e.auto_fill,
        };
        await this.addToHistory(item);
        this.push({ type: 'item', item });
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
        this.waiters.get(e.msg_id)?.reject(new RelayError(e.code, e.message));
        this.waiters.delete(e.msg_id);
        break;
      case 'accepted':
        this.waiters.get(e.msg_id)?.resolve();
        this.waiters.delete(e.msg_id);
        await this.updateItem(e.msg_id, { pending: false }, (i) => i.pending === true);
        break;
      case 'group_changed':
        this.people.onGroupChanged();
        break;
      case 'pairing_completed':
        break;
      case 'relay_directory_changed':
        this.people.onRelayChanged();
        return;
      case 'directory_changed':
        break;
      case 'directory_rejected':
        this.log(`objet de l'annuaire refusé : ${e.reason}`);
        this.alert({ kind: 'rejected', user: e.user, reason: e.reason });
        break;
      case 'directory_fork':
        this.log('le relais publie un autre groupe que le mien');
        this.alert({ kind: 'fork' });
        break;
      case 'keys_changed':
        this.alert({ kind: 'keys_changed', user: e.user });
        break;
      case 'person_joined':
        this.push({
          type: 'person-joined',
          user: e.user,
          name: e.name,
          deviceName: e.device_name,
          fingerprint: e.fingerprint,
        });
        break;
      case 'joined':
        if (this.device?.invited) {
          this.device = {
            id: this.device.id,
            name: this.device.name,
            serverUrl: this.device.serverUrl,
          };
          await this.deps.kv.set({ [DEVICE]: this.device });
        }
        this.inviteError = undefined;
        break;
      case 'invite_failed':
        this.inviteError = e.reason;
        break;
    }
    this.changed();
  }

  private alert(a: DirectoryAlert): void {
    const key = JSON.stringify(a);
    if (this.alerts.some((x) => JSON.stringify(x) === key)) return;
    this.alerts = [...this.alerts, a].slice(-MAX_ALERTS);
  }

  private async applyOutput(out: CoreOutput): Promise<void> {
    for (const e of out.events) await this.onClientEvent(e);
    for (const env of out.sends) await this.client?.queue(JSON.stringify(env));
  }

  private sendAndWait(env: string): Promise<void> {
    const client = this.client;
    if (!client) return Promise.reject(new Error('appareil pas encore appairé'));
    const msgId = (JSON.parse(env) as { msg_id: string }).msg_id;
    const accepted = new Promise<void>((resolve, reject) =>
      this.waiters.set(msgId, { resolve, reject }),
    );
    return client.queue(env).then(() => accepted);
  }

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
    this.people.stop();
    this.alerts = [];
    this.opened = {};
    this.inviteError = undefined;
  }

  private ensureNotInGroup(): void {
    if (this.group())
      throw new Error('cette extension est déjà dans un groupe : l’oublier d’abord');
  }

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
      redeemed = await this.redeem(
        url,
        (nonce) => this.vault.pairRedeemFrame(nonce, join.code, deviceName),
        (f) =>
          f.t === 'pair_redeemed' ? { device_id: f.device_id, sponsor: f.sponsor } : undefined,
      );
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

  async setupInvite(text: string, name: string): Promise<StateView> {
    this.ensureNotInGroup();
    const invite = JSON.parse(parseInvite(text.trim())) as { server_url: string; code: string };
    const url = checkServerUrl(invite.server_url);
    const deviceName = name.trim();
    if (!deviceName) throw new Error('nom de l’appareil vide');
    await this.resetDevice();
    this.vault.ensureIdentity();
    await this.persistVault();
    let redeemed: { user: string; device: string; inviter: string };
    try {
      redeemed = await this.redeem(
        url,
        (nonce) => this.vault.inviteRedeemFrame(nonce, invite.code, deviceName),
        (f) =>
          f.t === 'invite_redeemed'
            ? { user: f.user, device: f.device, inviter: f.inviter }
            : undefined,
      );
      this.vault.savePendingInvite(
        invite.code,
        redeemed.user,
        redeemed.device,
        redeemed.inviter,
        deviceName,
      );
    } catch (e) {
      await this.resetDevice();
      throw e;
    }
    await this.persistVault();
    this.openCore(redeemed.device);
    this.device = { id: redeemed.device, name: deviceName, serverUrl: url, invited: true };
    await this.deps.kv.set({ [DEVICE]: this.device });
    this.startClient();
    this.changed();
    return this.state();
  }

  private redeem<T>(
    serverUrl: string,
    frameFor: (nonce: string) => string,
    done: (frame: ServerFrame) => T | undefined,
  ): Promise<T> {
    return new Promise((resolve, reject) => {
      const url = wsUrl(serverUrl);
      const ws = this.deps.webSocket ? this.deps.webSocket(url) : new WebSocket(url);
      let settled = false;
      const finish = (error: Error | null, value?: T) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        ws.close();
        if (error) reject(error);
        else if (value !== undefined) resolve(value);
      };
      const timer = setTimeout(
        () => finish(new Error('pas de réponse du relais : vérifier Tailscale et l’URL')),
        REDEEM_TIMEOUT,
      );
      ws.onmessage = (ev: MessageEvent) => {
        if (typeof ev.data !== 'string') return;
        try {
          const frame = decode(ev.data, decodeFrame);
          const value = frame.t === 'challenge' ? undefined : done(frame);
          if (frame.t === 'challenge') {
            ws.send(frameFor(frame.nonce));
          } else if (value !== undefined) {
            finish(null, value);
          } else if (frame.t === 'error') {
            finish(new Error(`refusé par le relais : ${frame.message}`));
          }
        } catch (e) {
          finish(new Error(message(e)));
        }
      };
      ws.onclose = () =>
        finish(new Error('connexion au relais impossible : vérifier Tailscale et l’URL'));
    });
  }

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

  async send(text: string, to: string): Promise<string> {
    const core = this.core;
    const client = this.client;
    if (!core || !client || !this.group()) {
      throw new Error("cette extension n'est pas encore dans un groupe");
    }
    const target = parseChannel(to);
    let env: string;
    if (target.user !== undefined) {
      await this.people.fresh().catch((e: unknown) => this.log('relecture de l’annuaire', e));
      env = core.sealTextTo(text, target.user, target.device);
    } else {
      env = core.sealText(text, target.device ? [target.device] : []);
    }
    const trimmed = text.trim();
    const kind = /^https?:\/\/\S+$/i.test(trimmed) ? 'link' : 'text';
    await this.recordSent(env, { kind, body: kind === 'link' ? trimmed : text }, to);
    const msgId = await client.queue(env);
    this.changed();
    return msgId;
  }

  private async recordSent(
    env: string,
    content: Pick<Item, 'kind' | 'body' | 'file'>,
    to: string,
  ): Promise<void> {
    const device = this.device;
    if (!device) return;
    const parsed = JSON.parse(env) as { msg_id: string; created_at: string; to: string[] };
    const target = parseChannel(to);
    const item: Item = {
      msgId: parsed.msg_id,
      from: device.id,
      fromName: device.name,
      createdAt: parsed.created_at,
      ...content,
      outgoing: true,
      to: target.user !== undefined ? parsed.to : target.device ? [target.device] : [],
      pending: true,
    };
    if (target.user !== undefined) {
      item.toUser = target.user;
      if (target.device) item.toDevice = target.device;
    }
    await this.addToHistory(item);
    this.push({ type: 'item', item });
  }

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
      const target = parseChannel(file.to);
      let to: string[] = target.device ? [target.device] : [];
      if (target.user !== undefined) {
        await this.people.fresh().catch((e: unknown) => this.log('relecture de l’annuaire', e));
        to = JSON.parse((this.core as Core).recipients(target.user, target.device)) as string[];
      }
      const { env, blobId } = await transfers.upload(
        { sourceId: file.path, name: file.name, mime: file.mime, to, toUser: target.user },
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

  private async resumeOutgoing(): Promise<void> {
    if (!this.deps.files || !this.group() || !this.relayAccess) return;
    for (const file of await this.outgoing()) {
      if (this.running.has(file.path)) continue;
      void this.runUpload(file).catch((e: unknown) => this.log('reprise d’un envoi', e));
    }
  }

  async cancelUpload(path: string): Promise<void> {
    const { transfers, files } = this.ready();
    const file = (await this.outgoing()).find((o) => o.path === path);
    if (file) await transfers.cancelUpload(path, file.size);
    await this.editOutgoing((list) => list.filter((o) => o.path !== path));
    await files.remove(path);
    this.setProgress(undefined, path);
  }

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

  private geometry(blobId: string): Geometry {
    const core = this.core;
    if (!core) throw new Error("cette extension n'est pas encore dans un groupe");
    return JSON.parse(core.fileGeometry(blobId)) as Geometry;
  }

  private async sealedChunk(blobId: string, index: number): Promise<Uint8Array> {
    const { transfers, files } = this.ready();
    const cached = await files.getChunk(blobId, index);
    if (cached) return cached;
    if (!this.relayAccess && !(await this.refreshRelayAccess())) throw new Error(NO_RELAY_ACCESS);
    const sealed = await transfers.chunk(blobId, index);
    await files.putChunk(blobId, index, sealed);
    return sealed;
  }

  private async *plainChunks(blobId: string, from: number, to: number): AsyncGenerator<Uint8Array> {
    const core = this.core;
    if (!core) throw new Error("cette extension n'est pas encore dans un groupe");
    const pending: Promise<Uint8Array>[] = [];
    let next = from;
    const fill = () => {
      while (next <= to && pending.length < 4) {
        const p = this.sealedChunk(blobId, next++);
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
    if (mime !== 'application/pdf') headers.set('Content-Security-Policy', 'sandbox');
    if (range) headers.set('Content-Range', `bytes ${start}-${end}/${g.size}`);
    return new Response(body, { status: range ? 206 : 200, headers });
  }

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

  refreshDirectory(): Promise<void> {
    return this.people.read();
  }

  adminInit(): Promise<void> {
    return this.people.adoptRoot(true);
  }

  users(): Promise<AdminUser[]> {
    return this.people.users();
  }

  invite(name: string, links: string[]): Promise<InvitationView> {
    return this.people.invite(name, links);
  }

  reinvite(user: string): Promise<InvitationView> {
    return this.people.reinvite(user);
  }

  revokeUser(user: string): Promise<void> {
    return this.people.revokeUser(user);
  }

  renameUser(user: string, name: string): Promise<void> {
    return this.people.renameUser(user, name);
  }

  linkSet(from: string, to: string, allowed: boolean): Promise<void> {
    return this.people.linkSet(from, to, allowed);
  }

  updateDevice(
    device: string,
    settings: { kind?: PersonDeviceView['kind']; reachable?: boolean; default?: boolean },
  ): Promise<void> {
    return this.people.updateDevice(device, settings);
  }

  removeDevice(device: string): Promise<void> {
    return this.people.removeDevice(device);
  }

  muteService(service: string, muted: boolean): Promise<void> {
    return this.people.muteService(service, muted);
  }

  close(): void {
    this.people.stop();
    this.client?.stop();
    this.client = undefined;
    this.transfers = undefined;
    this.core?.free();
    this.core = undefined;
  }
}

function origin(e: {
  from_user?: string;
  from_service?: boolean;
  to_user?: string;
}): Pick<Item, 'fromUser' | 'fromService' | 'toUser'> {
  const o: Pick<Item, 'fromUser' | 'fromService' | 'toUser'> = {};
  if (e.from_user) o.fromUser = e.from_user;
  if (e.from_service) o.fromService = true;
  if (e.to_user) o.toUser = e.to_user;
  return o;
}

function parseOpened(bytes: Uint8Array): Record<string, string> {
  try {
    const v = JSON.parse(new TextDecoder().decode(bytes)) as unknown;
    return v && typeof v === 'object' ? (v as Record<string, string>) : {};
  } catch {
    return {};
  }
}
