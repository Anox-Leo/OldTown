import {
  type ClientFrame,
  type DeviceInfo,
  type ErrorCode,
  REPLIES,
  type ServerFrame,
  answers,
  callReference,
  decode,
  encode,
  wsUrl,
} from './frames';

export class RelayError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'RelayError';
  }
}

interface Origin {
  from_user?: string;
  from_service?: boolean;
  to_user?: string;
}

export type CoreEvent =
  | ({
      type: 'message';
      seq: number;
      msg_id: string;
      from: string;
      created_at: string;
      kind: 'text' | 'link';
      body: string;
      to?: string[];
    } & Origin)
  | { type: 'rejected'; msg_id: string; from: string; reason: string }
  | { type: 'group_changed'; epoch: number }
  | { type: 'pairing_completed'; device_id: string; name: string }
  | { type: 'keyring_error'; message: string }
  | ({
      type: 'file';
      seq: number;
      msg_id: string;
      from: string;
      created_at: string;
      blob_id: string;
      name: string;
      mime: string;
      size: number;
      to?: string[];
    } & Origin)
  | ({
      type: 'otp';
      seq: number;
      msg_id: string;
      from: string;
      created_at: string;
      code: string;
      display: string;
      sender_name: string;
      sender_domain: string;
      sender_verified: boolean;
      expires_at: number;
      to?: string[];
      auto_fill: boolean;
    } & Origin)
  | { type: 'directory_changed' }
  | { type: 'directory_rejected'; user: string; reason: string }
  | { type: 'directory_fork' }
  | { type: 'keys_changed'; user: string }
  | { type: 'person_joined'; user: string; name: string; device_name: string; fingerprint: string }
  | { type: 'joined'; user: string }
  | { type: 'invite_failed'; reason: string };

export type ClientEvent =
  | CoreEvent
  | { type: 'connected'; pending: number }
  | { type: 'disconnected'; reason: string }
  | { type: 'auth_failed'; message: string }
  | { type: 'accepted'; msg_id: string; seq: number }
  | { type: 'send_failed'; msg_id: string; code: ErrorCode; message: string }
  | { type: 'relay_directory_changed' };

interface CoreOutput {
  events: CoreEvent[];
  sends: unknown[];
}

export interface CoreApi {
  process(seq: number, envelope: string): string;
}

export interface AuthApi {
  authFrame(nonce: string, deviceId: string): string;
}

export interface ClientStorage {
  loadOutbox(): Promise<string[]>;
  saveOutbox(envelopes: string[]): Promise<void>;
  loadSeen(): Promise<string[]>;
  saveSeen(ids: string[]): Promise<void>;
}

export interface Timing {
  pingEvery: number;
  idleTimeout: number;
  handshakeTimeout: number;
  backoffInitial: number;
  backoffMax: number;
}

export const DEFAULT_TIMING: Timing = {
  pingEvery: 20_000,
  idleTimeout: 75_000,
  handshakeTimeout: 10_000,
  backoffInitial: 500,
  backoffMax: 25_000,
};

export const SEEN_CAPACITY = 1000;

export interface ClientDeps {
  serverUrl: string;
  deviceId: string;
  auth: AuthApi;
  core: CoreApi;
  decodeFrame(text: string): string;
  persistVault(): Promise<void>;
  storage: ClientStorage;
  onEvent(event: ClientEvent): void | Promise<void>;
  afterDeliver?(): void;
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

interface PendingCall extends PendingRequest<ServerFrame> {
  frame: ClientFrame;
  reference?: string;
}

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
  private calls: PendingCall[] = [];
  private loaded?: Promise<void>;
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

  get pendingSends(): number {
    return this.outbox.length;
  }

  start(): Promise<void> {
    if (this.status !== 'stopped') return this.load();
    this.status = 'waiting';
    return this.load().then(() => this.connect());
  }

  stop(): void {
    this.status = 'stopped';
    this.clearTimers();
    clearTimeout(this.timers.retry);
    this.failRequests(new Error('client arrêté'));
    const ws = this.ws;
    this.ws = undefined;
    ws?.close(1000);
  }

  reconnectNow(): void {
    if (this.status === 'stopped' || this.status === 'online' || this.status === 'connecting') {
      return;
    }
    clearTimeout(this.timers.retry);
    this.attempt = 0;
    this.connect();
  }

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

  listDevices(): Promise<DeviceInfo[]> {
    if (!this.online) return Promise.reject(new Error('hors ligne'));
    return new Promise((resolve, reject) => {
      this.deviceRequests.push({ resolve, reject });
      this.sendRaw(encode.listDevices());
    });
  }

  call(frame: ClientFrame | string): Promise<ServerFrame> {
    const f = typeof frame === 'string' ? (JSON.parse(frame) as ClientFrame) : frame;
    if (!this.online) return Promise.reject(new Error('hors ligne'));
    return new Promise((resolve, reject) => {
      this.calls.push({ frame: f, reference: callReference(f), resolve, reject });
      this.sendRaw(encode.request(f));
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
    for (const c of this.calls.splice(0)) c.reject(error);
  }

  private takeCall(pred: (c: PendingCall) => boolean): PendingCall | undefined {
    const i = this.calls.findIndex(pred);
    return i >= 0 ? this.calls.splice(i, 1)[0] : undefined;
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
      case 'directory_changed':
        await this.emit({ type: 'relay_directory_changed' });
        return;
      case 'error': {
        if (frame.code === 'auth_failed') {
          this.authFailed = true;
          ws.close();
          return;
        }
        const i = frame.ref ? this.outbox.findIndex((o) => o.msgId === frame.ref) : -1;
        const call =
          i >= 0
            ? undefined
            : frame.ref
              ? this.takeCall((c) => c.reference === frame.ref)
              : this.takeCall((c) => c.reference === undefined);
        if (call) {
          call.reject(new RelayError(frame.code, frame.message));
          return;
        }
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
        if (REPLIES.has(frame.t)) {
          const call = this.takeCall((c) => answers(c.frame, frame));
          if (call) call.resolve(frame);
          else this.log(`réponse ${frame.t} sans demande ignorée`);
          return;
        }
        this.log(`trame ${frame.t} inattendue ignorée`);
    }
  }

  private async onDeliver(ws: WebSocket, seq: number, env: unknown): Promise<void> {
    const msgId = (env as { msg_id: string }).msg_id;
    if (!this.seenSet.has(msgId)) {
      const out = JSON.parse(this.deps.core.process(seq, JSON.stringify(env))) as CoreOutput;
      await this.deps.persistVault();
      for (const sent of out.sends) {
        const text = JSON.stringify(sent);
        this.outbox.push({ msgId: (sent as { msg_id: string }).msg_id, env: text });
      }
      if (out.sends.length > 0) await this.saveOutbox();
      const to = (env as { to?: string[] }).to ?? [];
      for (const event of out.events) {
        await this.emit(
          event.type === 'message' || event.type === 'file' || event.type === 'otp'
            ? { ...event, to }
            : event,
        );
      }
      this.remember(msgId);
      await this.deps.storage.saveSeen(this.seen);
      for (const sent of out.sends) this.sendRaw(encode.send(JSON.stringify(sent)));
    } else {
      this.log(`doublon ${seq} ignoré`);
    }
    if (this.ws === ws) ws.send(encode.ack(seq));
    this.deps.afterDeliver?.();
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
