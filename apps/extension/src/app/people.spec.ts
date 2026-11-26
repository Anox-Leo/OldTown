import {
  type Item,
  type MemberView,
  type PersonDeviceView,
  type PersonView,
  channelOf,
  inChannel,
  notificationsToClear,
  parseChannel,
  shortcutChannel,
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

  it('une nature déclarée est gardée ; absente ou inconnue, l’appareil reste neutre', () => {
    expect(deviceKind('phone')).toBe('phone');
    expect(deviceKind('cli')).toBe('cli');
    expect(deviceKind('unknown')).toBe('unknown');
    expect(deviceKind()).toBe('unknown');
  });

  it('les appareils en ligne de commande ne sont pas des conversations (A4)', () => {
    const list = conversations(
      [member(ME, 'moi'), member('dev_tel', 'tel'), member('dev_cli', 'serveur')],
      [],
      ME,
      [device('dev_cli', 'serveur', { kind: 'cli' })],
    );
    expect(list.map((c) => c.id)).toEqual(['dev_tel']);
  });

  it('plus de « Tous mes appareils » : un ancien envoi à tous n’entre dans aucune conversation (note 0078)', () => {
    const toAll = item({ msgId: 'tous', from: 'dev_tel', to: [], createdAt: at(9) });
    const list = conversations([member(ME, 'moi'), member('dev_tel', 'tel')], [toAll], ME);
    expect(list.map((c) => c.id)).toEqual(['dev_tel']);
    expect(list[0]?.last).toBeUndefined();
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

describe('raccourcis Alt+Shift+S et V (note 0078)', () => {
  const member = (id: string, name: string): MemberView => ({
    device_id: id,
    name,
    sign_fingerprint: '',
    exchange_fingerprint: '',
    added_epoch: 1,
  });
  const ALICE = 'usr_alice';
  const members = [
    member(ME, 'moi'),
    member('dev_tel', 'tel'),
    member('dev_pc', 'pc'),
    member('dev_cli', 'serveur'),
  ];
  const state = (recipient: string, principal?: string, extra: PersonView[] = []) => ({
    device: { id: ME, name: 'moi', serverUrl: 'wss://relais' },
    group: { epoch: 1, key_fingerprint: '', roster_version: 1, members },
    me: ALICE,
    recipient,
    people: [
      person(ALICE, 'Alice', [
        device(ME, 'moi', { default: principal === ME }),
        device('dev_tel', 'tel', { default: principal === 'dev_tel', kind: 'phone' }),
        device('dev_pc', 'pc', { default: principal === 'dev_pc', kind: 'laptop' }),
        device('dev_cli', 'serveur', { kind: 'cli', reachable: false }),
      ]),
      person(BOB, 'Bob', [device('dev_bob', 'bob-phone')]),
      ...extra,
    ],
  });
  const sent = (to: string, n: number) =>
    item({ msgId: `${to}-${n}`, outgoing: true, from: ME, to: [to], createdAt: at(n) });

  it('conversation ouverte : elle', () => {
    expect(shortcutChannel(state('dev_pc', 'dev_tel'), [])).toBe('dev_pc');
    expect(shortcutChannel(state(userChannel(BOB), 'dev_tel'), [])).toBe(userChannel(BOB));
  });

  it('sans conversation ouverte : mon appareil principal', () => {
    expect(shortcutChannel(state('', 'dev_tel'), [sent('dev_pc', 5)])).toBe('dev_tel');
  });

  it('principal = cet appareil : mon autre appareil le plus récemment actif', () => {
    expect(shortcutChannel(state('', ME), [sent('dev_tel', 1), sent('dev_pc', 2)])).toBe('dev_pc');
    expect(shortcutChannel(state('', ME), [sent('dev_pc', 1), sent('dev_tel', 2)])).toBe('dev_tel');
  });

  it('sans activité, ni principal : par nom ; jamais un appareil en ligne de commande', () => {
    expect(shortcutChannel(state('', undefined), [sent('dev_cli', 9)])).toBe('dev_pc');
  });

  it('une conversation qui n’existe plus compte comme aucune', () => {
    expect(shortcutChannel(state('dev_parti', 'dev_tel'), [])).toBe('dev_tel');
    expect(shortcutChannel(state('dev_cli', 'dev_tel'), [])).toBe('dev_tel');
    expect(shortcutChannel(state(userChannel('usr_parti'), 'dev_tel'), [])).toBe('dev_tel');
    expect(shortcutChannel(state(userChannel(ALICE), 'dev_tel'), [])).toBe('dev_tel');
    const svc = person('usr_svc', 'Notifier', [device('dev_n', 'n')], { role: 'service' });
    expect(shortcutChannel(state(userChannel('usr_svc'), 'dev_tel', [svc]), [])).toBe('dev_tel');
  });

  it('aucun autre appareil : aucune cible', () => {
    const alone = { ...state('', ME), group: { ...state('').group, members: [member(ME, 'moi')] } };
    expect(shortcutChannel(alone, [])).toBeUndefined();
  });
});

describe('notifications effacées à la lecture (A3)', () => {
  const fromTel = (msgId: string, n: number) =>
    item({ msgId, from: 'dev_tel', to: [ME], createdAt: at(n) });
  const fromBob = (msgId: string, from: string, n: number) =>
    item({ msgId, from, fromUser: BOB, to: [ME], createdAt: at(n) });
  const items = [
    fromTel('t1', 1),
    fromTel('t2', 2),
    item({ msgId: 't3', outgoing: true, from: ME, to: ['dev_tel'], createdAt: at(3) }),
    fromBob('b1', 'dev_bob_tel', 4),
    fromBob('b2', 'dev_bob_pc', 5),
    item({ msgId: 'c1', from: 'dev_carol', fromUser: CAROL, to: [ME], createdAt: at(6) }),
  ];
  const shown = ['t1', 't2', 't3', 'b1', 'b2', 'c1', 'joined:usr_dan'];

  it('ouvrir une conversation efface ses notifications, et seulement les siennes', () => {
    expect(notificationsToClear(shown, items, 'dev_tel', ME)).toEqual(['t1', 't2']);
    expect(notificationsToClear(shown, items, userChannel(CAROL), ME)).toEqual(['c1']);
  });

  it('celle d’une personne efface celles de tous ses appareils', () => {
    expect(notificationsToClear(shown, items, userChannel(BOB), ME)).toEqual(['b1', 'b2']);
  });

  it('une notification déjà fermée, ou sans élément, n’est pas touchée', () => {
    expect(notificationsToClear(['t2', 'joined:usr_dan'], items, 'dev_tel', ME)).toEqual(['t2']);
    expect(notificationsToClear([], items, 'dev_tel', ME)).toEqual([]);
  });
});
