import {
  type Item,
  type MemberView,
  type PersonDeviceView,
  type PersonView,
  channelOf,
  inChannel,
  userChannel,
} from '../shared/api';
import { AVATAR_COLORS } from '../shared/tokens';

export type DeviceKind =
  'phone' | 'laptop' | 'desktop' | 'browser' | 'cli' | 'unknown' | 'group' | 'person';

export const ALL_NAME = 'Tous mes appareils';

export function deviceKind(
  kind?: PersonDeviceView['kind'],
): Exclude<DeviceKind, 'group' | 'person'> {
  return kind ?? 'unknown';
}

export function avatarColor(id: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return AVATAR_COLORS[(h >>> 0) % AVATAR_COLORS.length] ?? AVATAR_COLORS[0];
}

export interface Conversation {
  id: string;
  name: string;
  kind: DeviceKind;
  color: string;
  last?: Item;
  note?: string;
  outdated?: boolean;
  online?: boolean;
}

export function initials(name: string): string {
  const words = name
    .trim()
    .split(/[\s\-_.]+/u)
    .filter(Boolean);
  const letters = (words.length > 1 ? [words[0], words[1]] : [name.trim()]).map((w) =>
    [...(w ?? '')].slice(0, words.length > 1 ? 1 : 2).join(''),
  );
  return letters.join('').toLocaleUpperCase('fr');
}

function latest(items: Iterable<Item>): Item | undefined {
  let last: Item | undefined;
  for (const i of items) {
    if (!last || Date.parse(i.createdAt) > Date.parse(last.createdAt)) last = i;
  }
  return last;
}

export function conversations(
  members: readonly MemberView[],
  items: readonly Item[],
  me: string | undefined,
  profiles: readonly PersonDeviceView[] = [],
): Conversation[] {
  const profile = new Map(profiles.map((p) => [p.device_id, p]));
  const last = new Map<string, Item>();
  for (const item of items) {
    const c = channelOf(item, me);
    const prev = last.get(c);
    if (!prev || Date.parse(item.createdAt) > Date.parse(prev.createdAt)) last.set(c, item);
  }
  const at = (c: Conversation) => (c.last ? Date.parse(c.last.createdAt) || 0 : 0);
  const devices = members
    .filter((m) => m.device_id !== me)
    .map((m): Conversation => ({
      id: m.device_id,
      name: m.name,
      kind: deviceKind(profile.get(m.device_id)?.kind),
      color: avatarColor(m.device_id),
      last: last.get(m.device_id),
    }))
    .filter((c) => c.kind !== 'cli')
    .sort((a, b) => at(b) - at(a) || a.name.localeCompare(b.name, 'fr'));
  const all: Conversation = {
    id: '',
    name: ALL_NAME,
    kind: 'group',
    color: avatarColor(''),
    last: last.get(''),
  };
  return [all, ...devices];
}

export function preview(item: Item, channel: string): string {
  const body = item.body.replace(/\s+/g, ' ').trim().slice(0, 200);
  if (item.outgoing) return `Vous : ${body}`;
  return channel === '' ? `${item.fromName} : ${body}` : body;
}

export function personConversations(
  person: PersonView,
  items: readonly Item[],
  me: string | undefined,
): Conversation[] {
  const channel = userChannel(person.user);
  const mine = items.filter((i) => inChannel(i, channel, me));
  const head: Conversation = {
    id: channel,
    name: person.name,
    kind: 'person',
    color: avatarColor(person.user),
    last: latest(mine),
    note: person.role === 'service' ? 'service' : 'ses appareils par défaut',
    online: person.devices.some((d) => d.online),
  };
  const devices = person.devices
    .filter((d) => d.reachable)
    .map((d): Conversation => {
      const id = userChannel(person.user, d.device_id);
      const notes = [
        d.default ? 'par défaut' : '',
        d.up_to_date ? '' : 'pas à jour, ne recevra rien',
      ];
      return {
        id,
        name: d.name,
        kind: deviceKind(d.kind),
        color: avatarColor(d.device_id),
        last: latest(mine.filter((i) => inChannel(i, id, me))),
        note: notes.filter(Boolean).join(' · ') || undefined,
        outdated: !d.up_to_date,
        online: d.online,
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name, 'fr'));
  return person.role === 'service' ? [head] : [head, ...devices];
}

function personOf(item: Item): string | undefined {
  return item.toUser ?? item.fromUser;
}

export function railPeople(
  people: readonly PersonView[],
  items: readonly Item[],
  meUser: string | undefined,
): PersonView[] {
  const last = new Map<string, number>();
  for (const i of items) {
    const p = personOf(i);
    const t = Date.parse(i.createdAt) || 0;
    if (p && t > (last.get(p) ?? 0)) last.set(p, t);
  }
  return people
    .filter((p) => p.user !== meUser)
    .sort(
      (a, b) =>
        (last.get(b.user) ?? 0) - (last.get(a.user) ?? 0) || a.name.localeCompare(b.name, 'fr'),
    );
}

export function unread(
  items: readonly Item[],
  user: string,
  opened: Readonly<Record<string, string>>,
): number {
  const since = Date.parse(opened[user] ?? '') || 0;
  return items.filter(
    (i) => !i.outgoing && i.fromUser === user && (Date.parse(i.createdAt) || 0) > since,
  ).length;
}
