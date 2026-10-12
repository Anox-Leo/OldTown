// Messages entre l'interface (popup, réglages) et le service worker, seul
// propriétaire de la connexion (note 0009). L'interface n'ouvre jamais de
// connexion et ne voit aucune clé.

export type Connection = 'offline' | 'connecting' | 'online' | 'auth_failed';

export interface DeviceView {
  id: string;
  name: string;
  serverUrl: string;
}

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
  { kind: 'offer'; qr: QrView; text: string; expiresAt: string } | { kind: 'join' };

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
}

export interface Item {
  msgId: string;
  from: string;
  fromName: string;
  createdAt: string;
  kind: 'text' | 'link';
  body: string;
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
  | { type: 'set-recipient'; to: string }
  | { type: 'history' }
  | { type: 'clear-history' }
  | { type: 'devices' }
  | { type: 'rotate'; exclude: string[] }
  | { type: 'set-server-url'; url: string }
  | { type: 'forget' };

export type Reply<T = unknown> = { ok: true; value: T } | { ok: false; error: string };

/** Messages poussés par le service worker vers la popup ouverte (port). */
export type Push = { type: 'state'; state: StateView } | { type: 'item'; item: Item };

/** Nom du port ouvert par la popup : tant qu'il est ouvert, pas de notification. */
export const POPUP_PORT = 'popup';

/** Messages du service worker vers le document offscreen. */
export type OffscreenMessage =
  { target: 'offscreen'; type: 'copy'; text: string } | { target: 'offscreen'; type: 'read' };
