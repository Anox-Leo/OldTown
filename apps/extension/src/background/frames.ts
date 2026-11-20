// Trames du protocole (crates/ot-proto). Les trames reçues sont validées par
// `ot-proto` compilé en WebAssembly (decodeFrame) ; les trames simples sont
// écrites ici, celles qui portent une signature par le WebAssembly. Le format
// est figé par crates/ot-proto/tests/golden/, rejoué par les tests.

export const PROTOCOL_VERSION = 1;
export const WS_PATH = '/v1/ws';

/** Enveloppe telle que la voit le relais : `payload` reste opaque. */
export interface Envelope {
  msg_id: string;
  from: string;
  to: string[];
  created_at: string;
  payload: { enc: string; data: string };
}

export interface DeviceInfo {
  id: string;
  name: string;
  online: boolean;
}

export type ErrorCode =
  | 'bad_frame'
  | 'unsupported_version'
  | 'unknown_type'
  | 'unexpected_frame'
  | 'auth_failed'
  | 'envelope_too_large'
  | 'sender_mismatch'
  | 'unknown_recipient'
  | 'msg_id_conflict'
  | 'pairing_invalid'
  | 'name_taken'
  | 'internal'
  // Étape 10.
  | 'forbidden'
  | 'not_allowed'
  | 'quota_exceeded'
  | 'user_revoked'
  | 'conflict'
  | 'unknown';

/** Objet signé (certificat, entrée de journal), opaque ici. */
export interface SignedBlob {
  data: string;
  sig: string;
}

export type Role = 'admin' | 'member' | 'service';
/** Nature d'un appareil (étape 10) ; `cli` : jamais joignable ni par défaut. */
export type DeviceKindName = 'phone' | 'laptop' | 'desktop' | 'browser' | 'cli' | 'unknown';

/** Une personne vue par l'admin (`users`), tous ses appareils actifs. */
export interface AdminUser {
  id: string;
  name: string;
  role: Role;
  state: 'pending' | 'active' | 'revoked';
  /** Millisecondes Unix. */
  created_at: number;
  devices: AdminDevice[];
  /** Personnes vers qui elle peut écrire. */
  links: string[];
  /** Services qu'elle a coupés. */
  muted?: string[];
}

export interface AdminDevice {
  id: string;
  name: string;
  kind?: DeviceKindName;
  reachable: boolean;
  default: boolean;
  features: string[];
  online: boolean;
  /** Dernière fin de session, en millisecondes Unix. */
  last_seen_at?: number;
}

/** Personne d'une page de l'annuaire, telle que le relais la sert (non vérifiée). */
export interface DirectoryUser {
  id: string;
  name: string;
  role: Role;
  state: 'pending' | 'active' | 'revoked';
  cert?: SignedBlob;
}

/** Réglages d'un appareil (`device_update`) ; absent : inchangé. */
export interface DeviceSettings {
  kind?: DeviceKindName;
  reachable?: boolean;
  default?: boolean;
}

/** Trame reçue du relais. */
export type ServerFrame =
  | { t: 'challenge'; nonce: string }
  | { t: 'welcome'; device_id: string; pending: number }
  | { t: 'accepted'; msg_id: string; seq: number }
  | { t: 'deliver'; seq: number; env: Envelope }
  | { t: 'devices'; devices: DeviceInfo[] }
  | { t: 'pair_created'; token: string; expires_at: string }
  | { t: 'pair_redeemed'; device_id: string; sponsor: string }
  | { t: 'pair_registered'; device_id: string }
  | { t: 'pong' }
  | { t: 'error'; code: ErrorCode; message: string; ref?: string }
  // Étape 10 : annuaire, journal, administration.
  | { t: 'directory'; me: string; admin: string; users: DirectoryUser[]; next?: string }
  | { t: 'directory_changed' }
  | { t: 'roster_published'; user: string; seq: number }
  | { t: 'device_updated'; device: string }
  | { t: 'device_revoked'; device: string }
  | { t: 'service_muted'; service: string; muted: boolean }
  | { t: 'admin_initialized'; user: string }
  | { t: 'user_invited'; user: string; token: string; expires_at: string }
  | { t: 'user_certified'; user: string }
  | { t: 'user_revoked'; user: string }
  | { t: 'user_renamed'; user: string }
  | { t: 'link_updated'; from: string; to: string; allowed: boolean }
  | { t: 'users'; users: AdminUser[] }
  | { t: 'invite_redeemed'; user: string; device: string; inviter: string }
  // Trames de client renvoyées par erreur : décodées, puis ignorées.
  | {
      t:
        | 'auth'
        | 'send'
        | 'ack'
        | 'list_devices'
        | 'pair_create'
        | 'pair_redeem'
        | 'pair_register'
        | 'ping'
        | 'directory_get'
        | 'roster_publish'
        | 'device_update'
        | 'device_revoke'
        | 'service_mute'
        | 'admin_init'
        | 'user_invite'
        | 'user_reinvite'
        | 'user_certify'
        | 'service_create'
        | 'service_device'
        | 'user_revoke'
        | 'user_rename'
        | 'link_set'
        | 'users_get'
        | 'invite_redeem';
    };

/** Trame de demande, écrite ici ou par le WebAssembly (`t` en tête de champs). */
export interface ClientFrame {
  t: string;
  [field: string]: unknown;
}

/** Décode et valide une trame avec `ot-proto` (WebAssembly). */
export function decode(text: string, decodeFrame: (text: string) => string): ServerFrame {
  return JSON.parse(decodeFrame(text)) as ServerFrame;
}

function frame(t: string, fields: Record<string, unknown> = {}): string {
  return JSON.stringify({ v: PROTOCOL_VERSION, t, ...fields });
}

export const encode = {
  ping: (): string => frame('ping'),
  ack: (seq: number): string => frame('ack', { seq }),
  listDevices: (): string => frame('list_devices'),
  /** `env` : enveloppe JSON produite par le cœur, transmise telle quelle. */
  send: (env: string): string => `{"v":${PROTOCOL_VERSION},"t":"send","env":${env}}`,
  /** Trame d'une demande (`call`), dans l'ordre des champs de `ot-proto`. */
  request: (f: ClientFrame): string => JSON.stringify({ v: PROTOCOL_VERSION, ...f }),
};

/** Demandes de l'étape 10 écrites ici (les signées viennent du WebAssembly). */
export const request = {
  usersGet: (): ClientFrame => ({ t: 'users_get' }),
  userInvite: (name: string, links: string[]): ClientFrame => ({
    t: 'user_invite',
    name,
    role: 'member',
    links,
  }),
  userReinvite: (user: string): ClientFrame => ({ t: 'user_reinvite', user }),
  userCertify: (user: string, cert: SignedBlob): ClientFrame => ({ t: 'user_certify', user, cert }),
  userRevoke: (user: string): ClientFrame => ({ t: 'user_revoke', user }),
  userRename: (user: string, name: string): ClientFrame => ({ t: 'user_rename', user, name }),
  linkSet: (from: string, to: string, allowed: boolean): ClientFrame => ({
    t: 'link_set',
    from,
    to,
    allowed,
  }),
  deviceUpdate: (device: string, s: DeviceSettings): ClientFrame => {
    // Champs absents : inchangés (`skip_serializing_if`).
    const f: ClientFrame = { t: 'device_update', device };
    if (s.kind !== undefined) f['kind'] = s.kind;
    if (s.reachable !== undefined) f['reachable'] = s.reachable;
    if (s.default !== undefined) f['default'] = s.default;
    return f;
  },
  deviceRevoke: (device: string): ClientFrame => ({ t: 'device_revoke', device }),
  serviceMute: (service: string, muted: boolean): ClientFrame => ({
    t: 'service_mute',
    service,
    muted,
  }),
};

/**
 * Référence d'un refus de cette demande par le relais (comme
 * `call_reference` de `client.rs`) ; `undefined` : refus sans référence.
 */
export function callReference(f: ClientFrame): string | undefined {
  switch (f.t) {
    case 'user_reinvite':
    case 'user_certify':
    case 'user_revoke':
    case 'user_rename':
      return f['user'] as string;
    case 'roster_publish':
      return String(f['seq'] as number);
    case 'device_update':
    case 'device_revoke':
    case 'service_create':
    case 'service_device':
      return f['device'] as string;
    case 'service_mute':
      return f['service'] as string;
    default:
      return undefined;
  }
}

/** `reply` répond-elle à la demande `f` ? (comme `answered_by` de `client.rs`). */
export function answers(f: ClientFrame, reply: ServerFrame): boolean {
  const r = reply as unknown as Record<string, unknown>;
  switch (f.t) {
    case 'directory_get':
      return reply.t === 'directory';
    case 'admin_init':
      return reply.t === 'admin_initialized';
    case 'user_invite':
      return reply.t === 'user_invited';
    case 'users_get':
      return reply.t === 'users';
    case 'user_reinvite':
      return reply.t === 'user_invited' && r['user'] === f['user'];
    case 'user_certify':
      return reply.t === 'user_certified' && r['user'] === f['user'];
    case 'user_revoke':
      return reply.t === 'user_revoked' && r['user'] === f['user'];
    case 'user_rename':
      return reply.t === 'user_renamed' && r['user'] === f['user'];
    case 'roster_publish':
      return reply.t === 'roster_published' && r['user'] === f['user'] && r['seq'] === f['seq'];
    case 'device_update':
      return reply.t === 'device_updated' && r['device'] === f['device'];
    case 'device_revoke':
      return reply.t === 'device_revoked' && r['device'] === f['device'];
    case 'service_mute':
      return reply.t === 'service_muted' && r['service'] === f['service'];
    case 'link_set':
      return reply.t === 'link_updated' && r['from'] === f['from'] && r['to'] === f['to'];
    default:
      return false;
  }
}

/** Trames qui ne répondent qu'à une demande (`call`). */
export const REPLIES = new Set<ServerFrame['t']>([
  'directory',
  'roster_published',
  'device_updated',
  'device_revoked',
  'service_muted',
  'admin_initialized',
  'user_invited',
  'user_certified',
  'user_revoked',
  'user_renamed',
  'link_updated',
  'users',
]);

/** URL de l'endpoint WebSocket à partir de l'URL du relais (comme ot-core). */
export function wsUrl(serverUrl: string): string {
  let base = serverUrl.trim().replace(/\/+$/, '');
  if (base.startsWith('http://')) base = `ws://${base.slice('http://'.length)}`;
  else if (base.startsWith('https://')) base = `wss://${base.slice('https://'.length)}`;
  else if (!base.includes('://')) base = `ws://${base}`;
  return `${base}${WS_PATH}`;
}

/**
 * URL du relais acceptée : `wss://` (ou `ws://` pour les essais), avec un
 * hôte. Même règle que l'appli Android.
 */
export function checkServerUrl(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, '');
  const rest = trimmed.startsWith('wss://')
    ? trimmed.slice(6)
    : trimmed.startsWith('ws://')
      ? trimmed.slice(5)
      : null;
  if (rest === null) {
    throw new Error("l'URL du relais commence par wss:// (ex. wss://relais.exemple.ts.net:7878)");
  }
  if (rest === '' || /\s/.test(rest)) throw new Error('URL du relais invalide');
  return trimmed;
}
