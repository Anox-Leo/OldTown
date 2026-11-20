// Messages entre l'interface (popup, réglages) et le service worker, seul
// propriétaire de la connexion (note 0009). L'interface n'ouvre jamais de
// connexion et ne voit aucune clé.

export type Connection = 'offline' | 'connecting' | 'online' | 'auth_failed';

export interface DeviceView {
  id: string;
  name: string;
  serverUrl: string;
  /** Premier appareil d'une personne invitée, pas encore certifiée (étape 10). */
  invited?: boolean;
}

/** Appareil d'une personne de l'annuaire vérifié (étape 10), sans clé. */
export interface PersonDeviceView {
  device_id: string;
  name: string;
  /** Nature déclarée au relais ; absente : devinée au nom (note 0031). */
  kind?: 'phone' | 'laptop' | 'desktop' | 'browser' | 'cli' | 'unknown';
  reachable: boolean;
  default: boolean;
  /** Sait ouvrir `0x05` (`users-v1`) : sinon, ne recevra rien d'une autre personne. */
  up_to_date: boolean;
  online: boolean;
}

/** Personne de l'annuaire vérifié, moi compris. */
export interface PersonView {
  user: string;
  name: string;
  role: 'admin' | 'member' | 'service';
  /** Service que j'ai coupé. */
  muted: boolean;
  devices: PersonDeviceView[];
}

/** Alerte de l'annuaire, à montrer (étape 10). */
export type DirectoryAlert =
  | { kind: 'fork' }
  | { kind: 'rejected'; user: string; reason: string }
  | { kind: 'keys_changed'; user: string };

export interface MemberView {
  device_id: string;
  name: string;
  sign_fingerprint: string;
  exchange_fingerprint: string;
  added_epoch: number;
}

export interface GroupView {
  epoch: number;
  key_fingerprint: string;
  roster_version: number;
  members: MemberView[];
}

/** QR code : `size` × `size` modules, ligne par ligne, 1 = noir. */
export interface QrView {
  size: number;
  modules: number[];
}

export type PendingView =
  | { kind: 'offer'; qr: QrView; text: string; expiresAt: string }
  | { kind: 'join' }
  /** Invité (étape 10) : en attente de la certification par l'admin. */
  | { kind: 'invite' };

export interface StateView {
  version: string;
  /** `unconfigured` : rien ; `pairing` : appairage en cours ; `ready` : dans un groupe. */
  phase: 'unconfigured' | 'pairing' | 'ready';
  device?: DeviceView;
  connection: Connection;
  /** Dernière erreur de connexion, pour l'aide « vérifier Tailscale ». */
  connectionError?: string;
  pending?: PendingView;
  group?: GroupView;
  /** Dernier destinataire choisi ; `''` : tous les autres appareils. */
  recipient: string;
  /** Envois pas encore acceptés par le relais. */
  pendingSends: number;
  /** Fichiers en cours d'envoi ou de réception. */
  transfers: TransferView[];
  /**
   * Accès HTTP à l'origine du relais accordé (note 0027) : sans lui, le
   * navigateur bloque les fichiers (CORS). Toujours vrai hors de Chrome.
   */
  relayAccess: boolean;
  /** Annuaire vérifié (étape 10), moi compris ; vide sans annuaire. */
  people: PersonView[];
  /** Ma personne, une fois l'annuaire lu. */
  me?: string;
  /** Ma personne est l'admin. */
  admin: boolean;
  /**
   * Admin sans annuaire : le certificat racine reste à publier (« Publier
   * l'annuaire »), avec ce nom, celui que le relais donne à l'admin.
   */
  rootMissing?: { name: string };
  /** Alertes de l'annuaire depuis le démarrage (refus, fourche, clés changées). */
  alerts: DirectoryAlert[];
  /** Dernière ouverture de chaque conversation d'une personne (`user:<id>`), pour les non-lus. */
  opened: Record<string, string>;
  /** Invitation refusée ou échouée (invité). */
  inviteError?: string;
}

/** Fichier annoncé (étape 5) : jamais de clé, seulement de quoi l'afficher. */
export interface FileView {
  blobId: string;
  name: string;
  mime: string;
  size: number;
  /**
   * Copie chiffrée complète et vérifiée dans l'OPFS (note 0029) : le fichier
   * s'ouvre sans le relais.
   */
  cached?: boolean;
  /** Dernier échec (expiré, réseau, accès au relais…). */
  error?: string;
}

export interface TransferView {
  /** `msg_id` de l'élément (réception) ou chemin de la source (envoi). */
  id: string;
  direction: 'up' | 'down';
  name: string;
  done: number;
  total: number;
  error?: string;
}

/** Code à usage unique (note 0045). */
export interface OtpView {
  /** Le code tel qu'écrit dans le mail (`G-482913`) ; le code seul est dans `body`. */
  display: string;
  senderName: string;
  senderDomain: string;
  /** Expéditeur authentifié (DMARC). */
  verified: boolean;
  /** Fin de validité, en millisecondes Unix. */
  expiresAt: number;
}

export interface Item {
  msgId: string;
  from: string;
  fromName: string;
  createdAt: string;
  kind: 'text' | 'link' | 'file' | 'otp';
  /** Texte, lien, nom du fichier, ou code à taper. */
  body: string;
  file?: FileView;
  otp?: OtpView;
  /** Envoyé par cet appareil (sinon reçu). */
  outgoing?: boolean;
  /** Destinataires ; vide : tous les appareils (canal « Tous »). */
  to?: string[];
  /** Envoyé, pas encore accepté par le relais. */
  pending?: boolean;
  /** Reçu d'une autre personne que la mienne (étape 10). */
  fromUser?: string;
  /** Reçu d'une personne de service. */
  fromService?: boolean;
  /** Envoi (ou copie de mes appareils, D43) à une autre personne. */
  toUser?: string;
  /** Envoi à un seul appareil de `toUser`. */
  toDevice?: string;
  /** Code (règle O2) : copié ou rempli d'office seulement si vrai. */
  autoFill?: boolean;
}

/** Derniers éléments gardés, reçus et envoyés (note 0030). */
export const HISTORY_LEN = 100;

/** Préfixe des canaux d'une autre personne (étape 10). */
export const USER_CHANNEL = 'user:';

/**
 * Canal d'un élément (notes 0030 et 0031, étape 10). Avec une autre
 * personne : `user:<id>`, ou `user:<id>/<appareil>` pour un envoi à un seul
 * de ses appareils ; reçu, d'après `fromUser`, envoyé ou copié d'après
 * `toUser`. Entre mes appareils, inchangé : `''` pour « Tous », sinon
 * l'autre appareil d'une conversation à deux (envoyé : son unique
 * destinataire ; reçu : son expéditeur s'il m'était adressé à moi seul).
 */
export function channelOf(item: Item, me: string | undefined): string {
  if (item.toUser) {
    return item.toDevice
      ? `${USER_CHANNEL}${item.toUser}/${item.toDevice}`
      : `${USER_CHANNEL}${item.toUser}`;
  }
  if (item.fromUser) return `${USER_CHANNEL}${item.fromUser}`;
  const to = item.to ?? [];
  if (item.outgoing) return to.length === 1 ? (to[0] ?? '') : '';
  return to.length === 1 && to[0] === me ? item.from : '';
}

/** Cible d'un canal : un de mes appareils (`''` : tous), ou une personne et peut-être un de ses appareils. */
export type Target = { user?: undefined; device: string } | { user: string; device?: string };

export function parseChannel(channel: string): Target {
  if (!channel.startsWith(USER_CHANNEL)) return { device: channel };
  const [user = '', device] = channel.slice(USER_CHANNEL.length).split('/');
  return device ? { user, device } : { user };
}

export function userChannel(user: string, device?: string): string {
  return device ? `${USER_CHANNEL}${user}/${device}` : `${USER_CHANNEL}${user}`;
}

/**
 * L'élément est-il dans la conversation `channel` ? Celle d'une personne
 * montre tout ce qui est échangé avec elle, quel que soit l'appareil ;
 * celle d'un de ses appareils est le même fil filtré : envoyé à lui seul,
 * ou reçu de lui.
 */
export function inChannel(item: Item, channel: string, me: string | undefined): boolean {
  const c = channelOf(item, me);
  const t = parseChannel(channel);
  if (t.user === undefined) return c === channel;
  if (t.device === undefined) return c === channel || c.startsWith(`${channel}/`);
  return c === channel || (!item.outgoing && item.fromUser === t.user && item.from === t.device);
}

export interface RelayDevice {
  id: string;
  name: string;
  online: boolean;
  member: boolean;
}

/** Requêtes de l'interface ; la réponse est `Reply<T>`. */
export type Request =
  | { type: 'state' }
  | { type: 'setup-offer'; serverUrl: string; name: string }
  | { type: 'setup-code'; text: string; serverUrl: string; name: string }
  | { type: 'cancel-pairing' }
  | { type: 'send'; text: string; to: string }
  | { type: 'send-clipboard'; text: string; to: string }
  | { type: 'setup-invite'; text: string; name: string }
  | { type: 'set-recipient'; to: string }
  | { type: 'history' }
  | { type: 'clear-history' }
  | { type: 'devices' }
  | { type: 'rotate'; exclude: string[] }
  | { type: 'set-server-url'; url: string }
  | { type: 'forget' }
  /** Fichier déjà copié dans l'OPFS (`path`) par la page d'envoi. */
  | { type: 'send-file'; path: string; name: string; mime: string; size: number; to: string }
  // Étape 10 : `to` d'un envoi peut aussi être un canal `user:<id>[/<appareil>]`.
  | { type: 'mark-read'; channel: string }
  | { type: 'refresh-directory' }
  | { type: 'admin-init' }
  | { type: 'users' }
  | { type: 'invite'; name: string; links: string[] }
  | { type: 'reinvite'; user: string }
  | { type: 'revoke-user'; user: string }
  | { type: 'rename-user'; user: string; name: string }
  | { type: 'link-set'; from: string; to: string; allowed: boolean }
  | {
      type: 'update-device';
      device: string;
      settings: { kind?: PersonDeviceView['kind']; reachable?: boolean; default?: boolean };
    }
  | { type: 'remove-device'; device: string }
  | { type: 'mute-service'; service: string; muted: boolean };

/** Invitation créée par l'admin : à transmettre à la personne seule (le code porte le secret). */
export interface InvitationView {
  user: string;
  name: string;
  /** Code `ot2-…`. */
  code: string;
  /** `oldtown://invite?s=…&c=…`, qui porte aussi l'URL du relais. */
  link: string;
  qr: QrView;
  expiresAt: string;
}

export type Reply<T = unknown> = { ok: true; value: T } | { ok: false; error: string };

/**
 * Messages poussés par le service worker vers la popup ouverte (port).
 * `open` : la popup vient de s'ouvrir pour un élément reçu (popup ouverte
 * d'office ou clic sur sa notification), elle montre sa conversation.
 */
export type Push =
  | { type: 'state'; state: StateView }
  | { type: 'item'; item: Item }
  | { type: 'open'; channel: string }
  /** Admin : une personne invitée par cet appareil est certifiée. */
  | { type: 'person-joined'; user: string; name: string; deviceName: string; fingerprint: string };

/** Nom du port ouvert par la popup : tant qu'il est ouvert, pas de notification. */
export const POPUP_PORT = 'popup';

/** Messages du service worker vers le document offscreen. */
export type OffscreenMessage =
  { target: 'offscreen'; type: 'copy'; text: string } | { target: 'offscreen'; type: 'read' };
