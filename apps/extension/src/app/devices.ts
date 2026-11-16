import { type Item, type MemberView, channelOf } from '../shared/api';

/** Type d'appareil, pour l'avatar ; `group` : la conversation « Tous ». */
export type DeviceKind = 'phone' | 'laptop' | 'desktop' | 'browser' | 'group';

/** Nom de la conversation « Tous » (note 0031). */
export const ALL_NAME = 'Tous mes appareils';

// Mots du nom, sans accents ni casse. Le roster ne dit pas le type de
// l'appareil (l'ajouter changerait le format) : il se devine au nom.
const BROWSER =
  /\b(chrome|chromium|brave|firefox|edge|safari|opera|vivaldi|navigateur|browser|extension)\b/;
const PHONE = /\b(telephone|phone|smartphone|mobile|android|pixel|iphone|galaxy|ipad|tablette)\b/;
const DESKTOP = /\b(pc|desktop|fixe|tour|tower|windows|imac|mini|studio)\b/;

/**
 * Type d'un appareil d'après son nom (note 0031) : navigateur d'abord
 * (« Chrome Mac » est l'extension), puis téléphone, ordinateur fixe ; un
 * portable sinon (Mac, Linux, ou un nom d'hôte).
 */
export function deviceKind(name: string): Exclude<DeviceKind, 'group'> {
  const n = name.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
  if (BROWSER.test(n)) return 'browser';
  if (PHONE.test(n)) return 'phone';
  if (DESKTOP.test(n)) return 'desktop';
  return 'laptop';
}

/** Couleurs d'avatar, lisibles sous un pictogramme blanc dans les deux thèmes. */
const COLORS = [
  '#e05d5d',
  '#e0803c',
  '#c99a1e',
  '#3fa66b',
  '#22988a',
  '#3b82c4',
  '#6d6cf0',
  '#a35cc8',
  '#d6567f',
  '#5f7d95',
];

/** Couleur d'un appareil : tirée de son identifiant, donc la même à chaque ouverture. */
export function avatarColor(id: string): string {
  // FNV-1a 32 bits.
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return COLORS[(h >>> 0) % COLORS.length] ?? '#5f7d95';
}

export interface Conversation {
  /** `''` pour « Tous », sinon l'identifiant de l'appareil. */
  id: string;
  name: string;
  kind: DeviceKind;
  color: string;
  /** Dernier élément reçu ou envoyé dans le canal. */
  last?: Item;
}

/**
 * Liste de l'accueil (note 0031) : « Tous » en premier, puis un appareil par
 * ligne, du plus récemment actif au moins récent, puis par nom.
 */
export function conversations(
  members: readonly MemberView[],
  items: readonly Item[],
  me: string | undefined,
): Conversation[] {
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
      kind: deviceKind(m.name),
      color: avatarColor(m.device_id),
      last: last.get(m.device_id),
    }))
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
