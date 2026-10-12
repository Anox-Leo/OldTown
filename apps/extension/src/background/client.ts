// Client du protocole (T4, note 0023) : connexion au relais, authentification,
// file d'envoi persistée, acquittements, dédoublonnage persisté, reconnexion,
// maintien de la connexion. Même logique que `ot-core` (client.rs) ; le
// chiffrement, le roster et l'appairage restent dans le WebAssembly (`Core`).
//
// Un seul client par appareil, tenu par le service worker (note 0009).

import { decode, encode, type DeviceInfo, type ErrorCode, type ServerFrame, wsUrl } from './frames';

/** Événement du cœur (WebAssembly), voir crates/ot-wasm/src/core.rs. */
export type CoreEvent =
  | {
      type: 'message';
      seq: number;
      msg_id: string;
      from: string;
      created_at: string;
      kind: 'text' | 'link';
      body: string;
    }
  | { type: 'rejected'; msg_id: string; from: string; reason: string }
  | { type: 'group_changed'; epoch: number }
  | { type: 'pairing_completed'; device_id: string; name: string }
  | { type: 'keyring_error'; message: string };

export type ClientEvent =
  | CoreEvent
  | { type: 'connected'; pending: number }
  | { type: 'disconnected'; reason: string }
  | { type: 'auth_failed'; message: string }
  | { type: 'accepted'; msg_id: string; seq: number }
  | { type: 'send_failed'; msg_id: string; code: ErrorCode; message: string };

/** Sortie du cœur : événements, puis enveloppes à envoyer (JSON). */
interface CoreOutput {
  events: CoreEvent[];
  sends: unknown[];
}

/** Ce que le client demande au cœur WebAssembly. */
export interface CoreApi {
  process(seq: number, envelope: string): string;
}

/** Signature de la réponse au défi, par le coffre WebAssembly. */
export interface AuthApi {
  authFrame(nonce: string, deviceId: string): string;
}

/** État persistant du client : rien de secret (enveloppes déjà chiffrées). */
export interface ClientStorage {
  loadOutbox(): Promise<string[]>;
  saveOutbox(envelopes: string[]): Promise<void>;
  loadSeen(): Promise<string[]>;
  saveSeen(ids: string[]): Promise<void>;
}

export interface Timing {
  /** Intervalle des `ping` (20 s : Chrome arrête le service worker après 30 s). */
  pingEvery: number;
  /** Silence du relais au-delà duquel la connexion est morte. */
  idleTimeout: number;
  /** Délai maximal de la poignée de main et de l'authentification. */
  handshakeTimeout: number;
  backoffInitial: number;
  backoffMax: number;
}

export const DEFAULT_TIMING: Timing = {
  pingEvery: 20_000,
  idleTimeout: 75_000,
  handshakeTimeout: 10_000,
  backoffInitial: 500,
  // Plafond sous les 30 s de vie d'un service worker inactif ; une alarme le
  // réveille de toute façon s'il a été arrêté pendant l'attente.
  backoffMax: 25_000,
};

/** Nombre de `msg_id` gardés pour écarter les doublons (comme ot-core). */
export const SEEN_CAPACITY = 1000;

export interface ClientDeps {
  serverUrl: string;
  deviceId: string;
  auth: AuthApi;
  core: CoreApi;
  decodeFrame(text: string): string;
  /** Chiffre et range les changements du coffre ; appelé avant chaque `ack`. */
  persistVault(): Promise<void>;
  storage: ClientStorage;
  /** Traite un événement ; un message n'est acquitté qu'une fois traité. */
  onEvent(event: ClientEvent): void | Promise<void>;
  /**
   * Délai avant de réessayer après un refus d'authentification ; `null` :
   * abandonner. Par défaut, le plafond du backoff.
   */
  authRetryDelay?(): number | null;
  webSocket?(url: string): WebSocket;
  timing?: Partial<Timing>;
  log?(message: string, error?: unknown): void;
}

type Status = 'stopped' | 'waiting' | 'connecting' | 'online';

interface PendingRequest<T> {
  resolve(value: T): void;
  reject(error: Error): void;
}

/** Délai exponentiel plafonné, avec gigue (entre la moitié et la totalité). */
export function backoffDelay(attempt: number, initial: number, max: number): number {
  const base = Math.min(initial * 2 ** Math.min(attempt, 16), max);
  return base / 2 + Math.random() * (base / 2);
}

export class RelayClient {
  private readonly timing: Timing;
  private status: Status = 'stopped';
  private ws?: WebSocket;
  private attempt = 0;
  private authFailed = false;
  private outbox: { msgId: string; env: string }[] = [];
  private seen: string[] = [];
  private seenSet = new Set<string>();
  private deviceRequests: PendingRequest<DeviceInfo[]>[] = [];
  private loaded?: Promise<void>;
  /** Traitement en série des trames reçues et des commandes. */
  private chain: Promise<void> = Promise.resolve();
  private timers: {
    retry?: ReturnType<typeof setTimeout>;
    ping?: ReturnType<typeof setInterval>;
    idle?: ReturnType<typeof setTimeout>;
    handshake?: ReturnType<typeof setTimeout>;
  } = {};

  constructor(private readonly deps: ClientDeps) {
    this.timing = { ...DEFAULT_TIMING, ...deps.timing };
  }

  get online(): boolean {
    return this.status === 'online';
  }

  /** Envois pas encore acceptés par le relais. */
  get pendingSends(): number {
    return this.outbox.length;
  }

  /** Se connecte, puis se reconnecte seul jusqu'à `stop`. */
  start(): Promise<void> {
    if (this.status !== 'stopped') return this.load();
    this.status = 'waiting';
    return this.load().then(() => this.connect());
  }

  /** Ferme la connexion ; les demandes en attente échouent. */
  stop(): void {
    this.status = 'stopped';
    this.clearTimers();
    clearTimeout(this.timers.retry);
    this.failRequests(new Error('client arrêté'));
    const ws = this.ws;
    this.ws = undefined;
    ws?.close(1000);
  }

  /** Reconnexion immédiate (réveil du service worker, réseau revenu). */
  reconnectNow(): void {
    if (this.status === 'stopped' || this.status === 'online' || this.status === 'connecting') {
      return;
    }
    clearTimeout(this.timers.retry);
    this.attempt = 0;
    this.connect();
  }

  /**
   * Met une enveloppe chiffrée (JSON) en file : elle est rangée, puis envoyée
   * maintenant ou à la prochaine connexion. L'acceptation arrive par
   * l'événement `accepted`.
   */
  async queue(env: string): Promise<string> {
    await this.load();
    const msgId = (JSON.parse(env) as { msg_id: string }).msg_id;
    return this.serial(async () => {
      this.outbox.push({ msgId, env });
      await this.saveOutbox();
      this.sendRaw(encode.send(env));
      return msgId;
    });
  }

  /** Appareils connus du relais (connexion requise). */
  listDevices(): Promise<DeviceInfo[]> {
    if (!this.online) return Promise.reject(new Error('hors ligne'));
    return new Promise((resolve, reject) => {
      this.deviceRequests.push({ resolve, reject });
      this.sendRaw(encode.listDevices());
    });
  }

  private load(): Promise<void> {
    this.loaded ??= (async () => {
      const [outbox, seen] = await Promise.all([
        this.deps.storage.loadOutbox(),
        this.deps.storage.loadSeen(),
      ]);
      this.outbox = outbox.map((env) => ({
        msgId: (JSON.parse(env) as { msg_id: string }).msg_id,
        env,
      }));
      this.seen = seen.slice(-SEEN_CAPACITY);
      this.seenSet = new Set(this.seen);
    })();
    return this.loaded;
  }

  private serial<T>(task: () => Promise<T>): Promise<T> {
    const run = this.chain.then(task);
    this.chain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private log(message: string, error?: unknown): void {
    this.deps.log?.(message, error);
  }

  private async emit(event: ClientEvent): Promise<void> {
    try {
      await this.deps.onEvent(event);
    } catch (e) {
      // Un échec de l'appli (stockage de l'historique…) ne doit pas bloquer
      // la session : il est signalé, et le message reste vu.
      this.log(`événement ${event.type} non traité`, e);
    }
  }

  private connect(): void {
    if (this.status === 'stopped') return;
    this.status = 'connecting';
    this.authFailed = false;
    const url = wsUrl(this.deps.serverUrl);
    let ws: WebSocket;
    try {
      ws = this.deps.webSocket ? this.deps.webSocket(url) : new WebSocket(url);
    } catch (e) {
      this.lost(`connexion à ${url} : ${String(e)}`);
      return;
    }
    this.ws = ws;
    this.timers.handshake = setTimeout(() => {
      if (this.ws === ws && this.status === 'connecting') {
        ws.close();
        this.lost('authentification : délai dépassé');
      }
    }, this.timing.handshakeTimeout);
    ws.onmessage = (ev: MessageEvent) => {
      if (this.ws !== ws || typeof ev.data !== 'string') return;
      const text = ev.data;
      this.touch();
      void this.serial(() => this.onText(ws, text));
    };
    ws.onclose = (ev: CloseEvent) => {
      if (this.ws !== ws) return;
      const reason = ev.reason ? `fermée par le serveur : ${ev.reason}` : 'connexion fermée';
      this.lost(this.authFailed ? 'authentification refusée' : reason);
    };
    ws.onerror = () => {
      // Suivi d'un `close` : rien à faire ici. Le message d'erreur du
      // navigateur ne dit rien d'utile (et ne doit rien dire).
    };
  }

  private touch(): void {
    clearTimeout(this.timers.idle);
    if (this.status !== 'online') return;
    this.timers.idle = setTimeout(() => {
      this.ws?.close();
      this.lost('plus de nouvelles du serveur');
    }, this.timing.idleTimeout);
  }

  private clearTimers(): void {
    clearTimeout(this.timers.handshake);
    clearTimeout(this.timers.idle);
    clearInterval(this.timers.ping);
  }

  private failRequests(error: Error): void {
    for (const r of this.deviceRequests.splice(0)) r.reject(error);
  }

  private lost(reason: string): void {
    if (this.status === 'stopped') return;
    const wasOnline = this.status === 'online';
    this.ws = undefined;
    this.clearTimers();
    this.failRequests(new Error('connexion perdue'));
    this.status = 'waiting';
    let delay: number | null;
    if (this.authFailed) {
      delay = this.deps.authRetryDelay ? this.deps.authRetryDelay() : this.timing.backoffMax;
    } else {
      delay = backoffDelay(this.attempt, this.timing.backoffInitial, this.timing.backoffMax);
      this.attempt += 1;
    }
    if (!this.authFailed || wasOnline) void this.emit({ type: 'disconnected', reason });
    if (delay === null) {
      this.status = 'stopped';
      return;
    }
    clearTimeout(this.timers.retry);
    this.timers.retry = setTimeout(() => this.connect(), delay);
  }

  private sendRaw(text: string): void {
    if (this.status === 'online' && this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(text);
    }
  }

  private async onText(ws: WebSocket, text: string): Promise<void> {
    if (this.ws !== ws) return;
    let frame: ServerFrame;
    try {
      frame = decode(text, this.deps.decodeFrame);
    } catch (e) {
      this.log('trame illisible ignorée', e);
      return;
    }
    if (this.status === 'connecting') {
      await this.onHandshake(ws, frame);
    } else if (this.status === 'online') {
      await this.onFrame(ws, frame);
    }
  }

  private async onHandshake(ws: WebSocket, frame: ServerFrame): Promise<void> {
    switch (frame.t) {
      case 'challenge':
        ws.send(this.deps.auth.authFrame(frame.nonce, this.deps.deviceId));
        return;
      case 'welcome': {
        clearTimeout(this.timers.handshake);
        this.status = 'online';
        this.attempt = 0;
        this.touch();
        this.timers.ping = setInterval(() => this.sendRaw(encode.ping()), this.timing.pingEvery);
        await this.emit({ type: 'connected', pending: frame.pending });
        // Reprise : ce qui n'a pas été accepté repart, dans l'ordre.
        for (const out of this.outbox) this.sendRaw(encode.send(out.env));
        return;
      }
      case 'error':
        if (frame.code === 'auth_failed') {
          this.authFailed = true;
          await this.emit({ type: 'auth_failed', message: frame.message });
        } else {
          this.log(`erreur pendant l'authentification : ${frame.code}`);
        }
        return;
      default:
        this.log(`trame ${frame.t} ignorée avant welcome`);
    }
  }

  private async onFrame(ws: WebSocket, frame: ServerFrame): Promise<void> {
    switch (frame.t) {
      case 'deliver':
        await this.onDeliver(ws, frame.seq, frame.env);
        return;
      case 'accepted': {
        const i = this.outbox.findIndex((o) => o.msgId === frame.msg_id);
        if (i >= 0) {
          this.outbox.splice(i, 1);
          await this.saveOutbox();
          await this.emit({ type: 'accepted', msg_id: frame.msg_id, seq: frame.seq });
        }
        return;
      }
      case 'devices':
        this.deviceRequests.shift()?.resolve(frame.devices);
        return;
      case 'pong':
        return;
      case 'error': {
        if (frame.code === 'auth_failed') {
          this.authFailed = true;
          ws.close();
          return;
        }
        const i = frame.ref ? this.outbox.findIndex((o) => o.msgId === frame.ref) : -1;
        if (i >= 0) {
          const [out] = this.outbox.splice(i, 1);
          await this.saveOutbox();
          await this.emit({
            type: 'send_failed',
            msg_id: out.msgId,
            code: frame.code,
            message: frame.message,
          });
        } else {
          this.log(`erreur du serveur : ${frame.code} ${frame.message}`);
        }
        return;
      }
      default:
        this.log(`trame ${frame.t} inattendue ignorée`);
    }
  }

  private async onDeliver(ws: WebSocket, seq: number, env: unknown): Promise<void> {
    const msgId = (env as { msg_id: string }).msg_id;
    if (!this.seenSet.has(msgId)) {
      // Déchiffré et remis, mis de côté ou rejeté : dans tous les cas
      // acquitté ensuite, une fois l'état rangé (livraison au moins une fois).
      const out = JSON.parse(this.deps.core.process(seq, JSON.stringify(env))) as CoreOutput;
      await this.deps.persistVault();
      for (const sent of out.sends) {
        const text = JSON.stringify(sent);
        this.outbox.push({ msgId: (sent as { msg_id: string }).msg_id, env: text });
      }
      if (out.sends.length > 0) await this.saveOutbox();
      for (const event of out.events) await this.emit(event);
      this.remember(msgId);
      await this.deps.storage.saveSeen(this.seen);
      for (const sent of out.sends) this.sendRaw(encode.send(JSON.stringify(sent)));
    } else {
      this.log(`doublon ${seq} ignoré`);
    }
    if (this.ws === ws) ws.send(encode.ack(seq));
  }

  private remember(msgId: string): void {
    this.seen.push(msgId);
    this.seenSet.add(msgId);
    while (this.seen.length > SEEN_CAPACITY) {
      const old = this.seen.shift();
      if (old !== undefined) this.seenSet.delete(old);
    }
  }

  private saveOutbox(): Promise<void> {
    return this.deps.storage.saveOutbox(this.outbox.map((o) => o.env));
  }
}
