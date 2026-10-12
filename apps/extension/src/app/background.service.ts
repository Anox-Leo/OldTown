import { Injectable, signal } from '@angular/core';

import {
  type Item,
  POPUP_PORT,
  type Push,
  type Reply,
  type Request,
  type StateView,
} from '../shared/api';

/**
 * Accès au service worker, seul propriétaire de la connexion (note 0009) :
 * requêtes par message, et état poussé sur un port tant que la page est
 * ouverte (le service worker ne notifie pas pendant ce temps).
 */
@Injectable({ providedIn: 'root' })
export class BackgroundService {
  readonly state = signal<StateView | undefined>(undefined);
  readonly items = signal<Item[]>([]);
  readonly error = signal<string | undefined>(undefined);

  private port?: chrome.runtime.Port;

  /** Ouvre le port ; `popup` : la page est la popup (pas de notification). */
  connect(): void {
    if (this.port) return;
    const port = chrome.runtime.connect({ name: POPUP_PORT });
    this.port = port;
    port.onMessage.addListener((push: Push) => {
      if (push.type === 'state') this.state.set(push.state);
      else
        this.items.update((items) =>
          [push.item, ...items.filter((i) => i.msgId !== push.item.msgId)].slice(0, 50),
        );
    });
    port.onDisconnect.addListener(() => {
      // Le service worker a redémarré : on se reconnecte.
      this.port = undefined;
      setTimeout(() => this.connect(), 200);
    });
    void this.refreshHistory();
  }

  async request<T>(req: Request): Promise<T> {
    const reply = (await chrome.runtime.sendMessage(req)) as Reply<T> | undefined;
    if (!reply) throw new Error('le service worker ne répond pas');
    if (!reply.ok) throw new Error(reply.error);
    return reply.value;
  }

  /** Comme `request`, l'erreur en plus dans `error` ; `undefined` en cas d'échec. */
  async run<T>(req: Request): Promise<T | undefined> {
    this.error.set(undefined);
    try {
      const value = await this.request<T>(req);
      if (value && typeof value === 'object' && 'phase' in value)
        this.state.set(value as unknown as StateView);
      return value;
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
      return undefined;
    }
  }

  async refreshHistory(): Promise<void> {
    try {
      this.items.set(await this.request<Item[]>({ type: 'history' }));
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    }
  }
}
