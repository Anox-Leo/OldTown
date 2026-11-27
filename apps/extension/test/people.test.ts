import { IDBFactory } from 'fake-indexeddb';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { Engine } from '../src/background/engine';
import { IndexedDbKeySource } from '../src/background/secrets';
import { type Item, type Push, channelOf, inChannel, userChannel } from '../src/shared/api';
import { Cli, MemoryKV, Relay, eventually, loadWasm } from './support';

const FAST = { backoffInitial: 50, backoffMax: 300, pingEvery: 200, idleTimeout: 2_000 };

function open(): Promise<Engine> {
  return Engine.open({
    kv: new MemoryKV(),
    keys: new IndexedDbKeySource(new IDBFactory()),
    timing: FAST,
    log: process.env['OT_TEST_LOG'] ? (m, e) => console.warn(`[engine] ${m}`, e ?? '') : undefined,
  });
}

async function pairWith(phone: Cli, engine: Engine, relay: Relay, name: string): Promise<void> {
  const state = await engine.setupOffer(relay.url, name);
  if (state.pending?.kind !== 'offer') throw new Error('offre attendue');
  await phone.run('pair', 'sponsor', state.pending.text);
  await eventually('groupe adopté', () => engine.state().phase === 'ready' || undefined);
}

async function received(
  cli: Cli,
  pred: (m: Record<string, unknown>) => boolean,
): Promise<Record<string, unknown>> {
  const listen = cli.listen();
  try {
    return await eventually('message reçu par la CLI', () => listen.messages.find(pred), 30_000);
  } finally {
    listen.stop();
  }
}

describe('extension, appareil de l’admin', () => {
  let relay: Relay;
  let phone: Cli;
  let phoneId: string;
  let engine: Engine;
  let bob: Cli;
  let bobUser = '';
  const pushes: Push[] = [];

  beforeAll(async () => {
    loadWasm();
    relay = await Relay.start();
    phone = new Cli(relay, 'alice-phone');
    phoneId = await phone.setupGroup();
    await relay.renameUser('owner', 'Alice');
    await phone.run('admin', 'init');
    engine = await open();
    engine.onChange((p) => pushes.push(p));
  }, 60_000);

  afterAll(async () => {
    engine?.close();
    await relay?.dispose();
  });

  it('adopte la racine publiée par le téléphone et lit l’annuaire', async () => {
    await pairWith(phone, engine, relay, 'alice-chrome');
    const state = await eventually('annuaire lu', () => {
      const s = engine.state();
      const mine = s.people[0]?.devices.find((d) => d.device_id === s.device?.id);
      return s.me && s.people[0]?.devices.length === 2 && mine?.up_to_date ? s : undefined;
    });
    expect(state.admin).toBe(true);
    expect(state.rootMissing).toBeUndefined();
    expect(state.people.map((p) => p.name)).toEqual(['Alice']);
    await expect(engine.adminInit()).rejects.toThrow(/déjà publié/);
  }, 60_000);

  it('invite Bob, qui rejoint par la CLI ; l’extension le certifie', async () => {
    const me = engine.state().me as string;
    const invitation = await engine.invite('Bob', [me]);
    expect(invitation.link).toMatch(/^oldtown:\/\/invite\?s=/);
    expect(invitation.code).toMatch(/^ot2-/);
    expect(invitation.qr.modules.length).toBe(invitation.qr.size ** 2);
    bob = new Cli(relay, 'bob-cli');
    await bob.run('invite', 'join', invitation.link, '--name', 'bob-cli');
    const joined = pushes.find((p) => p.type === 'person-joined');
    expect(joined).toMatchObject({ type: 'person-joined', name: 'Bob', deviceName: 'bob-cli' });
    bobUser = invitation.user;
    await eventually(
      'Bob dans l’annuaire',
      () =>
        engine.state().people.some((p) => p.user === bobUser && p.devices.length > 0) || undefined,
    );
    const users = await engine.users();
    expect(users.find((u) => u.id === bobUser)?.links).toContain(me);
  }, 90_000);

  it('écrit à Bob et reçoit sa réponse dans sa conversation', async () => {
    const listen = bob.listen();
    try {
      await new Promise((r) => setTimeout(r, 500));
      await engine.send('bonjour Bob', userChannel(bobUser));
      const got = await eventually('reçu par Bob', () =>
        listen.messages.find((m) => m['body'] === 'bonjour Bob'),
      );
      expect(got['from_user_name']).toBe('Alice');
    } finally {
      listen.stop();
    }
    const sent = (await engine.history()).find((i) => i.body === 'bonjour Bob') as Item;
    expect(sent.toUser).toBe(bobUser);
    expect(channelOf(sent, engine.state().device?.id)).toBe(userChannel(bobUser));

    await bob.run('send', '--to-user', 'Alice', 'merci');
    const reply = await eventually('réponse de Bob', async () =>
      (await engine.history()).find((i) => i.body === 'merci'),
    );
    expect(reply.fromUser).toBe(bobUser);
    expect(reply.fromName).toBe('bob-cli');
    expect(inChannel(reply, userChannel(bobUser), engine.state().device?.id)).toBe(true);
    const copy = await received(phone, (m) => m['body'] === 'bonjour Bob');
    expect(copy['from_name']).toBe('alice-chrome');
  }, 90_000);

  it('règle le réseau, renomme et coupe un lien', async () => {
    const me = engine.state().me as string;
    await engine.renameUser(bobUser, 'Robert');
    await eventually(
      'nom re-signé',
      () => engine.state().people.find((p) => p.user === bobUser)?.name === 'Robert' || undefined,
    );
    await engine.linkSet(bobUser, me, false);
    const users = await engine.users();
    expect(users.find((u) => u.id === bobUser)?.links).not.toContain(me);
    await engine.linkSet(bobUser, me, true);
  }, 60_000);

  it('règle un de ses appareils, puis retire le téléphone (rotation, re-certification)', async () => {
    await engine.updateDevice(phoneId, { kind: 'phone' });
    await eventually(
      'nature relue',
      () =>
        engine
          .state()
          .people.find((p) => p.user === engine.state().me)
          ?.devices.find((d) => d.device_id === phoneId)?.kind === 'phone' || undefined,
    );
    const epoch = engine.state().group?.epoch ?? 0;
    await engine.removeDevice(phoneId);
    expect(engine.state().group?.epoch).toBeGreaterThan(epoch);
    expect(engine.state().group?.members.map((m) => m.name)).toEqual(['alice-chrome']);
    await eventually('téléphone hors de l’annuaire', () => {
      const alice = engine.state().people.find((p) => p.user === engine.state().me);
      return alice && !alice.devices.some((d) => d.device_id === phoneId) ? true : undefined;
    });
    expect(engine.state().alerts.filter((a) => a.kind !== 'keys_changed')).toEqual([]);
    const seen = await bob.run('users');
    expect(seen.stdout).toContain('Alice (admin)');
    expect(seen.stdout).toContain('alice-chrome');
    expect(seen.stdout).not.toContain('alice-phone');
  }, 90_000);

  it('révoque Robert', async () => {
    await engine.revokeUser(bobUser);
    await eventually('Robert hors de l’annuaire', () =>
      engine.state().people.some((p) => p.user === bobUser) ? undefined : true,
    );
  }, 60_000);
});

describe('extension invitée', () => {
  let relay: Relay;
  let alice: Cli;
  let engine: Engine;

  beforeAll(async () => {
    loadWasm();
    relay = await Relay.start();
    alice = new Cli(relay, 'alice-laptop');
    await alice.setupGroup();
    await relay.renameUser('owner', 'Alice');
    await alice.run('admin', 'init');
    engine = await open();
  }, 60_000);

  afterAll(async () => {
    engine?.close();
    await relay?.dispose();
  });

  it('rejoint par le lien, est certifiée et écrit à l’admin', async () => {
    const invite = alice.spawn('admin', 'invite', 'Carol', '--link', 'Alice');
    const link = await eventually(
      'lien d’invitation',
      () => /(oldtown:\/\/invite\?\S+)/.exec(invite.out())?.[1],
    );
    const state = await engine.setupInvite(link, 'carol-chrome');
    expect(state.phase).toBe('pairing');
    expect(state.pending).toEqual({ kind: 'invite' });
    await invite.done;
    const ready = await eventually(
      'certifiée',
      () => {
        const s = engine.state();
        return s.phase === 'ready' && s.me && s.people.length === 2 ? s : undefined;
      },
      60_000,
    );
    expect(ready.admin).toBe(false);
    expect(ready.device?.invited).toBeUndefined();
    const aliceUser = ready.people.find((p) => p.role === 'admin')?.user as string;
    expect(ready.people.find((p) => p.user === ready.me)?.name).toBe('Carol');

    const listen = alice.listen();
    try {
      await new Promise((r) => setTimeout(r, 500));
      await engine.send('bonjour Alice', userChannel(aliceUser));
      const got = await eventually('reçu par Alice', () =>
        listen.messages.find((m) => m['body'] === 'bonjour Alice'),
      );
      expect(got['from_user_name']).toBe('Carol');
    } finally {
      listen.stop();
    }
  }, 120_000);
});
