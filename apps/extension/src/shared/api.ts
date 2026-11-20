export type Connection = 'offline' | 'connecting' | 'online' | 'auth_failed';

export interface DeviceView {
  id: string;
  name: string;
  serverUrl: string;
  invited?: boolean;
}

export interface PersonDeviceView {
  device_id: string;
  name: string;
  kind?: 'phone' | 'laptop' | 'desktop' | 'browser' | 'cli' | 'unknown';
  reachable: boolean;
  default: boolean;
  up_to_date: boolean;
  online: boolean;
}

export interface PersonView {
  user: string;
  name: string;
  role: 'admin' | 'member' | 'service';
  muted: boolean;
  devices: PersonDeviceView[];
}

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

export interface QrView {
  size: number;
  modules: number[];
}

export type PendingView =
  | { kind: 'offer'; qr: QrView; text: string; expiresAt: string }
  | { kind: 'join' }
  | { kind: 'invite' };

export interface StateView {
  version: string;
  phase: 'unconfigured' | 'pairing' | 'ready';
  device?: DeviceView;
  connection: Connection;
  connectionError?: string;
  pending?: PendingView;
  group?: GroupView;
  recipient: string;
  pendingSends: number;
  transfers: TransferView[];
  relayAccess: boolean;
  people: PersonView[];
  me?: string;
  admin: boolean;
  rootMissing?: { name: string };
  alerts: DirectoryAlert[];
  opened: Record<string, string>;
  inviteError?: string;
}

export interface FileView {
  blobId: string;
  name: string;
  mime: string;
  size: number;
  cached?: boolean;
  error?: string;
}

export interface TransferView {
  id: string;
  direction: 'up' | 'down';
  name: string;
  done: number;
  total: number;
  error?: string;
}

export interface OtpView {
  display: string;
  senderName: string;
  senderDomain: string;
  verified: boolean;
  expiresAt: number;
}

export interface Item {
  msgId: string;
  from: string;
  fromName: string;
  createdAt: string;
  kind: 'text' | 'link' | 'file' | 'otp';
  body: string;
  file?: FileView;
  otp?: OtpView;
  outgoing?: boolean;
  to?: string[];
  pending?: boolean;
  fromUser?: string;
  fromService?: boolean;
  toUser?: string;
  toDevice?: string;
  autoFill?: boolean;
}

export const HISTORY_LEN = 100;

export const USER_CHANNEL = 'user:';

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

export type Target = { user?: undefined; device: string } | { user: string; device?: string };

export function parseChannel(channel: string): Target {
  if (!channel.startsWith(USER_CHANNEL)) return { device: channel };
  const [user = '', device] = channel.slice(USER_CHANNEL.length).split('/');
  return device ? { user, device } : { user };
}

export function userChannel(user: string, device?: string): string {
  return device ? `${USER_CHANNEL}${user}/${device}` : `${USER_CHANNEL}${user}`;
}

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
  | { type: 'send-file'; path: string; name: string; mime: string; size: number; to: string }
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

export interface InvitationView {
  user: string;
  name: string;
  code: string;
  link: string;
  qr: QrView;
  expiresAt: string;
}

export type Reply<T = unknown> = { ok: true; value: T } | { ok: false; error: string };

export type Push =
  | { type: 'state'; state: StateView }
  | { type: 'item'; item: Item }
  | { type: 'open'; channel: string }
  | { type: 'person-joined'; user: string; name: string; deviceName: string; fingerprint: string };

export const POPUP_PORT = 'popup';

export type OffscreenMessage =
  { target: 'offscreen'; type: 'copy'; text: string } | { target: 'offscreen'; type: 'read' };
