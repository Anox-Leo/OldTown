import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';

import type { Item, RelayDevice, StateView } from '../shared/api';
import { AvatarComponent } from './avatar';
import { BackgroundService } from './background.service';
import { ChatComponent } from './chat';
import { type Conversation, conversations, preview } from './devices';
import { IconComponent } from './icon';
import { connectionStatus, shortTime } from './status';

/** Présence relue auprès du relais tant que la popup est ouverte. */
const PRESENCE_EVERY_MS = 15_000;

/**
 * Accueil de la popup (note 0031) : la liste des conversations, « Tous »
 * puis une ligne par appareil, avec le dernier message. Un clic ouvre la
 * conversation, qui devient aussi le destinataire des raccourcis et menus.
 */
@Component({
  selector: 'app-home',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AvatarComponent, ChatComponent, IconComponent],
  template: `
    @if (open(); as id) {
      <app-chat [channel]="id.channel" [presence]="presence()" (back)="open.set(undefined)" />
    } @else {
      <header class="row">
        <h1 class="grow">OldTown</h1>
        <button class="icon ghost" (click)="openSettings()" title="Réglages" aria-label="Réglages">
          <app-icon name="settings" />
        </button>
      </header>
      @if (status().help; as help) {
        <div class="banner warning">{{ help }}</div>
      }
      @if (bg.error(); as error) {
        <div class="banner danger" role="alert">{{ error }}</div>
      }
      <ul class="chats" aria-label="Conversations">
        @for (c of list(); track c.id) {
          <li>
            <button class="chat" (click)="select(c)">
              <span class="avatar">
                <app-avatar [kind]="c.kind" [color]="c.color" />
                @if (presence()?.get(c.id)) {
                  <span class="dot" title="En ligne"></span>
                }
              </span>
              <span class="body">
                <span class="top">
                  <span class="name">{{ c.name }}</span>
                  @if (c.last; as last) {
                    <span class="time">{{ time(last) }}</span>
                  }
                </span>
                <span class="last">
                  @if (c.last; as last) {
                    @if (last.file) {
                      <app-icon name="paperclip" [size]="13" />
                    }
                    <span class="text">{{ previewOf(last, c.id) }}</span>
                  } @else {
                    <span class="text">Aucun message</span>
                  }
                </span>
              </span>
            </button>
          </li>
        }
      </ul>
    }
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      height: 580px;
    }
    header {
      flex: none;
      padding: 10px 12px 6px 16px;
    }
    h1 {
      margin: 0;
      font-size: 20px;
      font-weight: 700;
    }
    .ghost {
      display: inline-flex;
      border: none;
      background: none;
      color: var(--muted);
      padding: 6px;
    }
    .ghost:hover {
      color: var(--fg);
    }
    .banner {
      margin: 4px 12px;
    }
    .chats {
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      list-style: none;
      margin: 0;
      padding: 4px 0;
    }
    .chat {
      width: 100%;
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 0 16px;
      border: none;
      border-radius: 0;
      background: none;
      text-align: left;
    }
    .chat:hover:not(:disabled) {
      background: var(--hover);
    }
    .avatar {
      position: relative;
      display: inline-flex;
    }
    .dot {
      position: absolute;
      right: 0;
      bottom: 0;
      width: 12px;
      height: 12px;
      border-radius: 50%;
      background: var(--success);
      border: 2px solid var(--screen);
    }
    .body {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
      gap: 2px;
      padding: 12px 0;
    }
    .top,
    .last {
      display: flex;
      align-items: center;
      gap: 6px;
      min-width: 0;
    }
    .name {
      flex: 1;
      min-width: 0;
      font-weight: 600;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .time {
      flex: none;
      font-size: 11px;
      color: var(--muted);
    }
    .last {
      color: var(--muted);
      font-size: 13px;
    }
    .last .text {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    /* Séparateur à la WhatsApp : sous le texte, pas sous l'avatar. */
    li + li .body {
      border-top: 1px solid var(--border);
    }
  `,
})
export class HomeComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly status = computed(() => connectionStatus(this.state()));
  private readonly online = computed(() => this.state().connection === 'online');
  protected readonly list = computed(() =>
    conversations(this.state().group?.members ?? [], this.bg.items(), this.state().device?.id),
  );
  /** Conversation ouverte ; `undefined` : la liste. */
  protected readonly open = signal<{ channel: string } | undefined>(undefined);
  /** Appareils en ligne d'après le relais ; `undefined` tant qu'on ne sait pas. */
  protected readonly presence = signal<ReadonlyMap<string, boolean> | undefined>(undefined);

  constructor() {
    // Présence : à la connexion, puis régulièrement ; inconnue hors ligne.
    effect((onCleanup) => {
      if (!this.online()) {
        this.presence.set(undefined);
        return;
      }
      void this.loadPresence();
      const timer = setInterval(() => void this.loadPresence(), PRESENCE_EVERY_MS);
      onCleanup(() => clearInterval(timer));
    });
  }

  private async loadPresence(): Promise<void> {
    try {
      const devices = await this.bg.request<RelayDevice[]>({ type: 'devices' });
      this.presence.set(new Map(devices.map((d) => [d.id, d.online])));
    } catch {
      // Relais injoignable entre deux : l'état de la connexion le dira.
      this.presence.set(undefined);
    }
  }

  /** Ouvre la conversation ; son canal devient le destinataire des raccourcis et des menus. */
  protected select(c: Conversation): void {
    this.open.set({ channel: c.id });
    void this.bg.run({ type: 'set-recipient', to: c.id });
  }

  protected time(item: Item): string {
    return shortTime(item.createdAt);
  }

  protected previewOf(item: Item, channel: string): string {
    return preview(item, channel);
  }

  protected openSettings(): Promise<void> {
    return chrome.runtime.openOptionsPage();
  }
}
