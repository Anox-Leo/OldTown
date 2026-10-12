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
  | 'unknown';

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
        | 'ping';
    };

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
};

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
