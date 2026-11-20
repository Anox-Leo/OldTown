import {
  type Item,
  type MemberView,
  type PersonDeviceView,
  type PersonView,
  channelOf,
  inChannel,
  parseChannel,
  userChannel,
} from '../shared/api';
import type { AdminUser } from '../background/frames';
import { gridPairs, linkBetween } from './admin';
import {
  conversations,
  deviceKind,
  initials,
  personConversations,
  railPeople,
  unread,
} from './devices';

const at = (n: number) => new Date(Date.UTC(2026, 10, 20, 10, n)).toISOString();

const ME = 'dev_me';
const BOB = 'usr_bob';
const CAROL = 'usr_carol';

function item(over: Partial<Item>): Item {
  return {
    msgId: over.msgId ?? 'm',
    from: 'dev_x',
    fromName: 'x',
    createdAt: at(1),
    kind: 'text',
    body: 'b',
    ...over,
  };
}

function device(id: string, name: string, over: Partial<PersonDeviceView> = {}): PersonDeviceView {
  return {
    device_id: id,
    name,
    reachable: true,
    default: false,
    up_to_date: true,
    online: false,
    ...over,
  };
}

function person(
  user: string,
  name: string,
  devices: PersonDeviceView[],
  over: Partial<PersonView> = {},
): PersonView {
  return { user, name, role: 'member', muted: false, devices, ...over };
}

describe('channelOf (étape 10)', () => {
  it('reçu d’une autre personne : sa conversation', () => {
    expect(channelOf(item({ from: 'dev_bob', fromUser: BOB, to: [ME] }), ME)).toBe('user:usr_bob');
  });

  it('envoyé à une personne, ou à un de ses appareils', () => {
    const sent = item({ outgoing: true, from: ME, to: ['dev_bob', 'dev_tel'], toUser: BOB });
    expect(channelOf(sent, ME)).toBe('user:usr_bob');
    expect(channelOf({ ...sent, toDevice: 'dev_bob' }, ME)).toBe('user:usr_bob/dev_bob');
  });

  it('copie d’un envoi d’un de mes appareils (D43) : la conversation de la personne', () => {
    const copy = item({ from: 'dev_tel', to: ['dev_bob', ME], toUser: BOB });
    expect(channelOf(copy, ME)).toBe('user:usr_bob');
  });

  it('entre mes appareils, rien ne change (notes 0030, 0031)', () => {
    expect(channelOf(item({ from: 'dev_tel', to: [ME] }), ME)).toBe('dev_tel');
    expect(channelOf(item({ from: 'dev_tel', to: [] }), ME)).toBe('');
    expect(channelOf(item({ outgoing: true, from: ME, to: ['dev_tel'] }), ME)).toBe('dev_tel');
  });

  it('se relit en cible', () => {
    expect(parseChannel('')).toEqual({ device: '' });
    expect(parseChannel('dev_tel')).toEqual({ device: 'dev_tel' });
    expect(parseChannel(userChannel(BOB))).toEqual({ user: BOB });
    expect(parseChannel(userChannel(BOB, 'dev_bob'))).toEqual({ user: BOB, device: 'dev_bob' });
  });
});

describe('inChannel', () => {
  const fromPhone = item({ msgId: '1', from: 'dev_bob_tel', fromUser: BOB });
  const fromPc = item({ msgId: '2', from: 'dev_bob_pc', fromUser: BOB });
  const toAll = item({ msgId: '3', outgoing: true, toUser: BOB, to: ['dev_bob_tel'] });
  const toPc = item({ msgId: '4', outgoing: true, toUser: BOB, toDevice: 'dev_bob_pc' });

  it('la conversation de la personne montre tout, quel que soit l’appareil', () => {
    const c = userChannel(BOB);
    expect([fromPhone, fromPc, toAll, toPc].filter((i) => inChannel(i, c, ME))).toHaveLength(4);
  });

  it('celle d’un de ses appareils est le même fil filtré', () => {
    const c = userChannel(BOB, 'dev_bob_pc');
    expect(
      [fromPhone, fromPc, toAll, toPc].filter((i) => inChannel(i, c, ME)).map((i) => i.msgId),
    ).toEqual(['2', '4']);
  });

  it('une autre personne n’y entre pas', () => {
    const carol = item({ fromUser: CAROL, from: 'dev_carol' });
    expect(inChannel(carol, userChannel(BOB), ME)).toBe(false);
    expect(inChannel(carol, '', ME)).toBe(false);
  });
});

describe('listes de l’accueil', () => {
  const member = (id: string, name: string): MemberView => ({
    device_id: id,
    name,
    sign_fingerprint: '',
    exchange_fingerprint: '',
    added_epoch: 1,
  });

  it('la nature déclarée passe avant le nom ; une nature inconnue est un portable', () => {
    expect(deviceKind('Chrome Mac', 'phone')).toBe('phone');
    expect(deviceKind('pc', 'cli')).toBe('cli');
    expect(deviceKind('pc', 'unknown')).toBe('laptop');
    expect(deviceKind('Chrome Mac')).toBe('browser');
  });

  it('les appareils en ligne de commande ne sont pas des conversations (A4)', () => {
    const list = conversations(
      [member(ME, 'moi'), member('dev_tel', 'tel'), member('dev_cli', 'serveur')],
      [],
      ME,
      [device('dev_cli', 'serveur', { kind: 'cli' })],
    );
    expect(list.map((c) => c.id)).toEqual(['', 'dev_tel']);
  });

  it('une personne : elle d’abord, puis ses appareils joignables par nom', () => {
    const bob = person(BOB, 'Bob', [
      device('dev_b2', 'bob-pc'),
      device('dev_b1', 'bob-phone', { default: true, online: true }),
      device('dev_b3', 'bob-vieux', { up_to_date: false }),
      device('dev_b4', 'bob-cache', { reachable: false }),
    ]);
    const last = item({ msgId: 'x', from: 'dev_b2', fromUser: BOB, createdAt: at(5) });
    const list = personConversations(bob, [last], ME);
    expect(list.map((c) => c.id)).toEqual([
      'user:usr_bob',
      'user:usr_bob/dev_b2',
      'user:usr_bob/dev_b1',
      'user:usr_bob/dev_b3',
    ]);
    expect(list[0]).toMatchObject({ name: 'Bob', kind: 'person', online: true });
    expect(list[0]?.last?.msgId).toBe('x');
    expect(list[1]?.last?.msgId).toBe('x');
    expect(list[2]?.last).toBeUndefined();
    expect(list[2]?.note).toBe('par défaut');
    expect(list[3]).toMatchObject({ outdated: true, note: 'pas à jour, ne recevra rien' });
  });

  it('un service n’a que sa conversation (on ne lui répond pas)', () => {
    const svc = person('usr_svc', 'Notifier', [device('dev_n', 'notifier-1')], { role: 'service' });
    expect(personConversations(svc, [], ME).map((c) => c.id)).toEqual(['user:usr_svc']);
  });
});

describe('barre latérale', () => {
  const people = [
    person('usr_me', 'Alice', []),
    person(BOB, 'Bob', []),
    person(CAROL, 'Carol', []),
    person('usr_dan', 'Dan', []),
  ];

  it('les autres personnes, de la plus récemment active à la moins, puis par nom', () => {
    const items = [
      item({ msgId: '1', fromUser: BOB, createdAt: at(1) }),
      item({ msgId: '2', outgoing: true, toUser: 'usr_dan', createdAt: at(2) }),
    ];
    expect(railPeople(people, items, 'usr_me').map((p) => p.name)).toEqual(['Dan', 'Bob', 'Carol']);
  });

  it('non-lus : reçus de la personne depuis la dernière ouverture de sa conversation', () => {
    const items = [
      item({ msgId: '1', fromUser: BOB, createdAt: at(1) }),
      item({ msgId: '2', fromUser: BOB, createdAt: at(3) }),
      item({ msgId: '3', outgoing: true, toUser: BOB, createdAt: at(4) }),
      item({ msgId: '4', fromUser: CAROL, createdAt: at(4) }),
    ];
    expect(unread(items, BOB, {})).toBe(2);
    expect(unread(items, BOB, { [BOB]: at(2) })).toBe(1);
    expect(unread(items, BOB, { [BOB]: at(9) })).toBe(0);
  });

  it('initiales', () => {
    expect(initials('Alice')).toBe('AL');
    expect(initials('Jean-Paul')).toBe('JP');
    expect(initials('Marie Claire Dupont')).toBe('MC');
    expect(initials('élise')).toBe('ÉL');
  });
});

describe('grille du réseau (D41)', () => {
  const user = (id: string, links: string[]): AdminUser => ({
    id,
    name: id,
    role: 'member',
    state: 'active',
    created_at: 0,
    devices: [],
    links,
  });

  it('une case par paire, liens symétriques ou à sens unique', () => {
    const users = [user('a', ['b']), user('b', ['a', 'c']), user('c', [])];
    expect(linkBetween(users, 'a', 'b')).toBe('both');
    expect(linkBetween(users, 'b', 'a')).toBe('both');
    expect(linkBetween(users, 'b', 'c')).toBe('one-way');
    expect(linkBetween(users, 'a', 'c')).toBe('none');
    expect(gridPairs(['a', 'b', 'c'])).toEqual([
      ['a', 'b'],
      ['a', 'c'],
      ['b', 'c'],
    ]);
  });
});
