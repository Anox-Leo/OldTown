export const PROTOCOL_VERSION = 1;
export const WS_PATH = '/v1/ws';

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
  | 'forbidden'
  | 'not_allowed'
  | 'quota_exceeded'
  | 'user_revoked'
  | 'conflict'
  | 'unknown';

export interface SignedBlob {
  data: string;
  sig: string;
}

export type Role = 'admin' | 'member' | 'service';
export type DeviceKindName = 'phone' | 'laptop' | 'desktop' | 'browser' | 'cli' | 'unknown';

export interface AdminUser {
  id: string;
  name: string;
  role: Role;
  state: 'pending' | 'active' | 'revoked';
  created_at: number;
  devices: AdminDevice[];
  links: string[];
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
  last_seen_at?: number;
}

export interface DirectoryUser {
  id: string;
  name: string;
  role: Role;
  state: 'pending' | 'active' | 'revoked';
  cert?: SignedBlob;
}

export interface DeviceSettings {
  kind?: DeviceKindName;
  reachable?: boolean;
  default?: boolean;
}

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

export interface ClientFrame {
  t: string;
  [field: string]: unknown;
}

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
  send: (env: string): string => `{"v":${PROTOCOL_VERSION},"t":"send","env":${env}}`,
  request: (f: ClientFrame): string => JSON.stringify({ v: PROTOCOL_VERSION, ...f }),
};

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

export function wsUrl(serverUrl: string): string {
  let base = serverUrl.trim().replace(/\/+$/, '');
  if (base.startsWith('http://')) base = `ws://${base.slice('http://'.length)}`;
  else if (base.startsWith('https://')) base = `wss://${base.slice('https://'.length)}`;
  else if (!base.includes('://')) base = `ws://${base}`;
  return `${base}${WS_PATH}`;
}

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
