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

/**
 * Type d'appareil, pour l'avatar ; `group` : la conversation « Tous » ;
 * `person` : une autre personne (initiales).
 */
export type DeviceKind = 'phone' | 'laptop' | 'desktop' | 'browser' | 'cli' | 'group' | 'person';

/** Nom de la conversation « Tous » (note 0031). */
export const ALL_NAME = 'Tous mes appareils';

// Mots du nom, sans accents ni casse. Le roster ne dit pas le type de
// l'appareil (l'ajouter changerait le format) : il se devine au nom.
const BROWSER =
  /\b(chrome|chromium|brave|firefox|edge|safari|opera|vivaldi|navigateur|browser|extension)\b/;
const PHONE = /\b(telephone|phone|smartphone|mobile|android|pixel|iphone|galaxy|ipad|tablette)\b/;
const DESKTOP = /\b(pc|desktop|fixe|tour|tower|windows|imac|mini|studio)\b/;

/**
 * Type d'un appareil : la nature déclarée au relais (étape 10), sinon
 * d'après son nom (note 0031) : navigateur d'abord (« Chrome Mac » est
 * l'extension), puis téléphone, ordinateur fixe ; un portable sinon (Mac,
 * Linux, ou un nom d'hôte). Une nature inconnue compte comme un portable.
 */
export function deviceKind(
  name: string,
  kind?: PersonDeviceView['kind'],
): Exclude<DeviceKind, 'group' | 'person'> {
  if (kind === 'unknown') return 'laptop';
  if (kind) return kind;
  const n = name.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
  if (BROWSER.test(n)) return 'browser';
  if (PHONE.test(n)) return 'phone';
  if (DESKTOP.test(n)) return 'desktop';
  return 'laptop';
}

/**
 * Couleur d'un appareil : tirée de son identifiant, donc la même à chaque
 * ouverture. Les dix teintes viennent des jetons de design (`AVATAR_COLORS`,
 * lisibles sous un pictogramme blanc dans les deux thèmes) : leur ordre ne
 * change pas, pour qu'un appareil garde sa famille de couleur.
 */
export function avatarColor(id: string): string {
  // FNV-1a 32 bits.
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return AVATAR_COLORS[(h >>> 0) % AVATAR_COLORS.length] ?? AVATAR_COLORS[0];
}

export interface Conversation {
  /**
   * Canal : `''` pour « Tous », l'identifiant d'un de mes appareils, ou
   * `user:<id>[/<appareil>]` pour une autre personne.
   */
  id: string;
  name: string;
  kind: DeviceKind;
  color: string;
  /** Dernier élément reçu ou envoyé dans le canal. */
  last?: Item;
  /** Sous le nom (« par défaut », « pas à jour, ne recevra rien »). */
  note?: string;
  /** Appareil qui ne sait pas ouvrir `0x05` : rien ne lui parviendra. */
  outdated?: boolean;
  online?: boolean;
}

/** Initiales d'une personne (deux au plus), pour son avatar. */
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

/** Le plus récent de ces éléments. */
function latest(items: Iterable<Item>): Item | undefined {
  let last: Item | undefined;
  for (const i of items) {
    if (!last || Date.parse(i.createdAt) > Date.parse(last.createdAt)) last = i;
  }
  return last;
}

/**
 * Liste de l'accueil (note 0031) : « Tous » en premier, puis un appareil par
 * ligne, du plus récemment actif au moins récent, puis par nom. Les
 * appareils en ligne de commande (`cli`, A4) n'y sont pas : ils restent dans
 * « Tous » et dans les réglages.
 */
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
      kind: deviceKind(m.name, profile.get(m.device_id)?.kind),
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

/**
 * Aperçu du dernier élément, sur une ligne : « Vous : » devant un envoi, le
 * nom de l'expéditeur devant un reçu de « Tous ».
 */
export function preview(item: Item, channel: string): string {
  const body = item.body.replace(/\s+/g, ' ').trim().slice(0, 200);
  if (item.outgoing) return `Vous : ${body}`;
  return channel === '' ? `${item.fromName} : ${body}` : body;
}

/**
 * Liste d'une autre personne (I2) : la personne (ses appareils par défaut)
 * en premier, puis chacun de ses appareils joignables, par nom ; un
 * appareil pas à jour est signalé, il ne recevra rien.
 */
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
        kind: deviceKind(d.name, d.kind),
        color: avatarColor(d.device_id),
        last: latest(mine.filter((i) => inChannel(i, id, me))),
        note: notes.filter(Boolean).join(' · ') || undefined,
        outdated: !d.up_to_date,
        online: d.online,
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name, 'fr'));
  // Un service n'a pas de conversation par appareil : il écrit, on ne lui répond pas.
  return person.role === 'service' ? [head] : [head, ...devices];
}

/** Personne d'un élément échangé avec une autre personne, ou `undefined`. */
function personOf(item: Item): string | undefined {
  return item.toUser ?? item.fromUser;
}

/**
 * Barre latérale (I2) : les autres personnes, de la plus récemment active à
 * la moins, puis par nom ; moi n'y figure pas (il est en tête, à part).
 */
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

/** Éléments reçus d'une personne depuis la dernière ouverture de sa conversation. */
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
