// Cœur du service worker, sans API Chrome (testé sous Node contre un vrai
// relais) : appareil, coffre et cœur WebAssembly, client du protocole,
// historique. `main.ts` y branche menus, notifications, alarmes et messages.

import {
  Core,
  type QrCode,
  Vault,
  decodeFrame,
  newDeviceId,
  parseJoin,
  qrCode,
  version,
} from '../wasm/ot_wasm.js';
import type {
  Connection,
  DeviceView,
  GroupView,
  Item,
  PendingView,
  Push,
  QrView,
  RelayDevice,
  StateView,
} from '../shared/api';
import { type ClientEvent, type ClientStorage, RelayClient, type Timing } from './client';
import { checkServerUrl, decode, wsUrl } from './frames';
import { type KeySource, type KeyValue, SecretStore } from './secrets';

/** Derniers éléments reçus gardés (plan de l'étape 4). */
export const HISTORY_LEN = 50;
/** Historique : contenu en clair, donc rangé chiffré comme les clés. */
const HISTORY = 'history.json';
const DEVICE = 'device';
const RECIPIENT = 'recipient';
const OUTBOX = 'outbox';
const SEEN = 'seen';
/** Délai maximal de l'enregistrement par code. */
const REDEEM_TIMEOUT = 10_000;

export interface EngineDeps {
  kv: KeyValue;
  keys: KeySource;
  webSocket?(url: string): WebSocket;
  timing?: Partial<Timing>;
  log?(message: string, error?: unknown): void;
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
        if (name !== HISTORY) this.vault.load(name, value);
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
    if (device) this.startClient();
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
    this.connection = 'connecting';
    void client.start().catch((e: unknown) => this.log('démarrage du client', e));
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
        };
        // Rangé avant l'acquittement : le client attend la fin de ce
        // traitement.
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
        break;
      case 'group_changed':
      case 'pairing_completed':
      case 'accepted':
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
    const msgId = await client.queue(env);
    this.changed();
    return msgId;
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
    this.core?.free();
    this.core = undefined;
  }
}
