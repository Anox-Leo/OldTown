import { type Core, type QrCode, inviteUri, qrCode } from '../wasm/ot_wasm.js';
import type { DeviceView, GroupView, InvitationView, PersonView, QrView } from '../shared/api';
import { type ClientEvent, RelayError, type RelayClient } from './client';
import {
  type AdminUser,
  type ClientFrame,
  type DeviceSettings,
  type ServerFrame,
  type SignedBlob,
  request,
} from './frames';

export const FRESH_MS = 60_000;
export const CHANGED_DEBOUNCE_MS = 2_000;
const PUBLISH_ATTEMPTS = 4;
const FLOW_RETRY_MS = 60_000;

export function qrView(text: string): QrView {
  const qr: QrCode = qrCode(text);
  try {
    return { size: qr.size, modules: Array.from(qr.modules) };
  } finally {
    qr.free();
  }
}

export interface CoreOutput {
  events: ClientEvent[];
  sends: unknown[];
}

type LogHead = Record<string, unknown>;

export type PublishWhat =
  | { kind: 'if_behind' }
  | { kind: 'certify'; user: string; cert: SignedBlob }
  | { kind: 'revoke'; user: string };

interface PublishOut {
  frame?: ClientFrame;
  head?: LogHead;
}

type DirectoryPage = Extract<ServerFrame, { t: 'directory' }>;

export interface PeopleHost {
  core(): Core | undefined;
  client(): RelayClient | undefined;
  device(): DeviceView | undefined;
  group(): GroupView | undefined;
  persistVault(): Promise<void>;
  apply(out: CoreOutput): Promise<void>;
  sendAndWait(env: string): Promise<void>;
  rotate(exclude: string[]): Promise<number>;
  changed(): void;
  log(message: string, error?: unknown): void;
}

export class People {
  private reading?: Promise<void>;
  private again = false;
  private readAt = 0;
  private changedTimer?: ReturnType<typeof setTimeout>;
  private readonly flows = new Set<string>();
  private readonly failed = new Map<string, number>();
  private lock: Promise<unknown> = Promise.resolve();
  private missingRoot?: { name: string };

  constructor(private readonly host: PeopleHost) {}

  people(): PersonView[] {
    const json = this.host.core()?.people();
    return json ? (JSON.parse(json) as PersonView[]) : [];
  }

  me(): PersonView | undefined {
    const id = this.host.device()?.id;
    return id ? this.people().find((p) => p.devices.some((d) => d.device_id === id)) : undefined;
  }

  person(user: string): PersonView | undefined {
    return this.people().find((p) => p.user === user);
  }

  get rootMissing(): { name: string } | undefined {
    return this.missingRoot;
  }

  onConnected(): void {
    this.sendInviteRequest();
    const core = this.host.core();
    if (!core || !this.host.group()) {
      if (core?.awaitsDirectory()) {
        void this.read().catch((e: unknown) => this.host.log('lecture de l’annuaire', e));
      }
      return;
    }
    void this.adoptRoot(false)
      .then(() => this.read())
      .catch((e: unknown) => this.host.log('annuaire', e));
  }

  onRelayChanged(): void {
    if (this.changedTimer) return;
    this.changedTimer = setTimeout(() => {
      this.changedTimer = undefined;
      void this.read().catch((e: unknown) => this.host.log('relecture de l’annuaire', e));
    }, CHANGED_DEBOUNCE_MS);
  }

  afterDeliver(): void {
    const core = this.host.core();
    if (core?.awaitsDirectory()) {
      void this.read().catch((e: unknown) => this.host.log('lecture de l’annuaire', e));
    }
    this.drive();
  }

  onGroupChanged(): void {
    if (this.people().length === 0 && this.host.client()?.online) {
      void this.adoptRoot(false)
        .then(() => this.read())
        .catch((e: unknown) => this.host.log('annuaire', e));
      return;
    }
    this.drive();
  }

  stop(): void {
    clearTimeout(this.changedTimer);
    this.changedTimer = undefined;
  }

  private sendInviteRequest(): void {
    const core = this.host.core();
    const client = this.host.client();
    if (!core || !client) return;
    try {
      const env = core.inviteRequest();
      if (env) void client.queue(env);
    } catch (e) {
      this.host.log('demande d’invitation', e);
    }
  }

  read(): Promise<void> {
    if (this.reading) {
      this.again = true;
      return this.reading;
    }
    const run = (async () => {
      try {
        do {
          this.again = false;
          await this.readOnce();
        } while (this.again);
      } finally {
        this.reading = undefined;
      }
    })();
    this.reading = run;
    void run.then(
      () => this.drive(),
      () => undefined,
    );
    return run;
  }

  async fresh(): Promise<void> {
    if (Date.now() - this.readAt > FRESH_MS) await this.read();
  }

  private async readOnce(): Promise<void> {
    const core = this.host.core();
    const client = this.host.client();
    if (!core || !client?.online) return;
    let frame: string | ClientFrame | undefined = core.directoryStart();
    if (!frame) return;
    for (;;) {
      const page = await client.call(frame);
      const step = JSON.parse(core.directoryPage(JSON.stringify(page))) as CoreOutput & {
        frame?: ClientFrame;
        done: boolean;
      };
      await this.host.persistVault();
      await this.host.apply(step);
      if (!step.frame || step.done) break;
      frame = step.frame;
    }
    this.readAt = Date.now();
    const device = this.host.device();
    const features = device ? core.declarationFrame('browser') : undefined;
    if (features) {
      try {
        await client.call(features);
        this.again = true;
      } catch (e) {
        this.host.log('capacités de l’appareil non déclarées', e);
      }
    }
    this.host.changed();
  }

  private drive(): void {
    const core = this.host.core();
    if (!core || !this.host.client()?.online || !this.host.group()) return;
    if (this.people().length === 0) return;
    const due: [string, () => Promise<unknown>][] = [];
    for (const user of JSON.parse(core.invitesToCertify()) as string[]) {
      due.push([`certify:${user}`, () => this.certifyInvite(user)]);
    }
    due.push(['publish', () => this.publish({ kind: 'if_behind' })]);
    for (const user of JSON.parse(core.servicesToRepublish()) as string[]) {
      due.push([`republish:${user}`, () => this.republish(user)]);
    }
    const now = Date.now();
    for (const [name, run] of due) {
      if (this.flows.has(name) || now - (this.failed.get(name) ?? 0) < FLOW_RETRY_MS) continue;
      this.flows.add(name);
      void run().then(
        () => {
          this.flows.delete(name);
          this.failed.delete(name);
        },
        (e: unknown) => {
          this.flows.delete(name);
          this.failed.set(name, Date.now());
          this.host.log(`déroulé ${name.split(':')[0]} en échec, repris plus tard`, e);
        },
      );
    }
  }

  private locked<T>(task: () => Promise<T>): Promise<T> {
    const run = this.lock.then(task);
    this.lock = run.catch(() => undefined);
    return run;
  }

  private ready(): { core: Core; client: RelayClient } {
    const core = this.host.core();
    const client = this.host.client();
    if (!core || !client) throw new Error("cette extension n'est pas encore dans un groupe");
    return { core, client };
  }

  publish(what: PublishWhat): Promise<LogHead | undefined> {
    return this.locked(async () => {
      for (let attempt = 0; attempt < PUBLISH_ATTEMPTS; attempt++) {
        if (attempt === 0) await this.fresh();
        else await this.read();
        const { core, client } = this.ready();
        const out = JSON.parse(core.nextEntry(JSON.stringify(what))) as PublishOut;
        if (!out.frame) return out.head;
        try {
          await client.call(out.frame);
        } catch (e) {
          if (e instanceof RelayError && e.code === 'conflict') continue;
          throw e;
        }
        const fx = JSON.parse(core.entryPublished()) as CoreOutput;
        await this.host.persistVault();
        await this.host.apply(fx);
        void this.read().catch((e: unknown) => this.host.log('relecture de l’annuaire', e));
        return out.head;
      }
      throw new Error('journal publié en conflit : réessayer plus tard');
    });
  }

  private republish(service: string): Promise<void> {
    return this.locked(async () => {
      for (let attempt = 0; attempt < PUBLISH_ATTEMPTS; attempt++) {
        await this.read();
        const { core, client } = this.ready();
        const out = JSON.parse(core.nextServiceEntry(service)) as PublishOut;
        if (!out.frame) return;
        let reply: ServerFrame;
        try {
          reply = await client.call(out.frame);
        } catch (e) {
          if (e instanceof RelayError && e.code === 'conflict') continue;
          throw e;
        }
        if (reply.t !== 'roster_published') throw new Error(`réponse ${reply.t} inattendue`);
        await this.host.apply(
          JSON.parse(core.serviceEntryPublished(reply.user, reply.seq)) as CoreOutput,
        );
        await this.host.persistVault();
        void this.read().catch((e: unknown) => this.host.log('relecture de l’annuaire', e));
        return;
      }
      throw new Error('journal du service en conflit : réessayer plus tard');
    });
  }

  private async certifyInvite(user: string): Promise<void> {
    const { core, client } = this.ready();
    const { cert, certified } = JSON.parse(core.inviteCert(user)) as {
      cert: SignedBlob;
      certified: boolean;
    };
    await this.host.persistVault();
    if (!certified) {
      await client.call(request.userCertify(user, cert));
      core.inviteCertified(user);
      await this.host.persistVault();
    }
    const head = await this.publish({ kind: 'certify', user, cert });
    if (!head) throw new Error("journal de l'admin introuvable");
    await this.host.sendAndWait(core.inviteWelcome(user, JSON.stringify(head)));
    const out = JSON.parse(core.inviteDone(user)) as CoreOutput;
    await this.host.persistVault();
    await this.host.apply(out);
  }

  async adoptRoot(create: boolean): Promise<void> {
    const { core, client } = this.ready();
    let start: string;
    try {
      start = core.adminInitStart();
    } catch {
      this.missingRoot = undefined;
      if (create) throw new Error("l'annuaire est déjà publié");
      return;
    }
    const page = (await client.call(start)) as DirectoryPage;
    const admin = page.users.find((u) => u.id === page.admin);
    if (page.me !== page.admin || !admin) {
      this.missingRoot = undefined;
      if (create) throw new Error("cet appareil n'est pas à l'admin");
      return;
    }
    if (!admin.cert && !create) {
      this.missingRoot = { name: admin.name };
      this.host.changed();
      return;
    }
    const res = JSON.parse(core.adminRoot(JSON.stringify(page))) as CoreOutput & {
      frame?: ClientFrame;
      adopted: boolean;
    };
    await this.host.persistVault();
    await this.host.apply(res);
    if (res.frame) {
      const reply = await client.call(res.frame);
      if (reply.t !== 'admin_initialized') throw new Error(`réponse ${reply.t} inattendue`);
      const out = JSON.parse(core.adoptAdminRoot(reply.user)) as CoreOutput;
      await this.host.persistVault();
      await this.host.apply(out);
    }
    this.missingRoot = undefined;
    this.host.changed();
    await this.read();
  }

  async users(): Promise<AdminUser[]> {
    const reply = await this.ready().client.call(request.usersGet());
    if (reply.t !== 'users') throw new Error(`réponse ${reply.t} inattendue`);
    return reply.users;
  }

  async invite(name: string, links: string[]): Promise<InvitationView> {
    const reply = await this.ready().client.call(request.userInvite(name.trim(), links));
    if (reply.t !== 'user_invited') throw new Error(`réponse ${reply.t} inattendue`);
    return this.invitation(reply, name.trim());
  }

  async reinvite(user: string): Promise<InvitationView> {
    const known = (await this.users()).find((u) => u.id === user);
    if (!known) throw new Error('personne inconnue du relais');
    const reply = await this.ready().client.call(request.userReinvite(user));
    if (reply.t !== 'user_invited') throw new Error(`réponse ${reply.t} inattendue`);
    return this.invitation(reply, known.name);
  }

  private async invitation(
    reply: Extract<ServerFrame, { t: 'user_invited' }>,
    name: string,
  ): Promise<InvitationView> {
    const { core } = this.ready();
    const device = this.host.device();
    if (!device) throw new Error('appareil pas encore appairé');
    const code = core.createInvite(reply.user, name, reply.token, reply.expires_at);
    await this.host.persistVault();
    const link = inviteUri(device.serverUrl, code);
    return { user: reply.user, name, code, link, qr: qrView(link), expiresAt: reply.expires_at };
  }

  async revokeUser(user: string): Promise<void> {
    await this.ready().client.call(request.userRevoke(user));
    await this.publish({ kind: 'revoke', user });
    await this.read();
  }

  async renameUser(user: string, name: string): Promise<void> {
    await this.ready().client.call(request.userRename(user, name.trim()));
    await this.fresh();
    const known = this.person(user);
    if (known && known.role !== 'admin') {
      await this.certify(user, this.recertify(user, name.trim()));
    }
    await this.read();
  }

  private recertify(user: string, name?: string): SignedBlob {
    return JSON.parse(this.ready().core.recertify(user, name)) as SignedBlob;
  }

  private async certify(user: string, cert: SignedBlob): Promise<void> {
    await this.ready().client.call(request.userCertify(user, cert));
    await this.publish({ kind: 'certify', user, cert });
  }

  async linkSet(from: string, to: string, allowed: boolean): Promise<void> {
    await this.ready().client.call(request.linkSet(from, to, allowed));
    await this.read();
  }

  async updateDevice(device: string, settings: DeviceSettings): Promise<void> {
    await this.ready().client.call(request.deviceUpdate(device, settings));
    await this.read();
  }

  async removeDevice(device: string): Promise<void> {
    const { client } = this.ready();
    const mine = this.host.group()?.members.some((m) => m.device_id === device) ?? false;
    if (device === this.host.device()?.id)
      throw new Error('cet appareil ne se retire pas lui-même');
    if (mine) {
      await this.host.rotate([device]);
      await this.publish({ kind: 'if_behind' });
    }
    await client.call(request.deviceRevoke(device));
    if (mine) {
      const { core } = this.ready();
      for (const user of JSON.parse(core.certifiedBy(device)) as string[]) {
        await this.certify(user, this.recertify(user));
      }
    }
    await this.read();
  }

  async muteService(service: string, muted: boolean): Promise<void> {
    await this.ready().client.call(request.serviceMute(service, muted));
    await this.read();
  }
}
