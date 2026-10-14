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
  /** Fichiers en cours d'envoi ou de réception. */
  transfers: TransferView[];
  /**
   * Accès HTTP à l'origine du relais accordé (note 0027) : sans lui, le
   * navigateur bloque les fichiers (CORS). Toujours vrai hors de Chrome.
   */
  relayAccess: boolean;
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

export interface Item {
  msgId: string;
  from: string;
  fromName: string;
  createdAt: string;
  kind: 'text' | 'link' | 'file';
  /** Texte, lien, ou nom du fichier. */
  body: string;
  file?: FileView;
  /** Envoyé par cet appareil (sinon reçu). */
  outgoing?: boolean;
  /** Destinataires ; vide : tous les appareils (canal « Tous »). */
  to?: string[];
  /** Envoyé, pas encore accepté par le relais. */
  pending?: boolean;
}

/** Derniers éléments gardés, reçus et envoyés (note 0030). */
export const HISTORY_LEN = 100;

/**
 * Canal d'un élément (note 0030) : `''` pour « Tous », sinon l'autre appareil
 * d'une conversation à deux. Envoyé : son unique destinataire. Reçu : son
 * expéditeur s'il m'était adressé à moi seul. Le reste va dans « Tous ».
 */
export function channelOf(item: Item, me: string | undefined): string {
  const to = item.to ?? [];
  if (item.outgoing) return to.length === 1 ? (to[0] ?? '') : '';
  return to.length === 1 && to[0] === me ? item.from : '';
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
  | { type: 'forget' }
  /** Fichier déjà copié dans l'OPFS (`path`) par la page d'envoi. */
  | { type: 'send-file'; path: string; name: string; mime: string; size: number; to: string };

export type Reply<T = unknown> = { ok: true; value: T } | { ok: false; error: string };

/** Messages poussés par le service worker vers la popup ouverte (port). */
export type Push = { type: 'state'; state: StateView } | { type: 'item'; item: Item };

/** Nom du port ouvert par la popup : tant qu'il est ouvert, pas de notification. */
export const POPUP_PORT = 'popup';

/** Messages du service worker vers le document offscreen. */
export type OffscreenMessage =
  { target: 'offscreen'; type: 'copy'; text: string } | { target: 'offscreen'; type: 'read' };
