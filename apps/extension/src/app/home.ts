import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';

import { type Item, type RelayDevice, type StateView, parseChannel } from '../shared/api';
import { AvatarComponent } from './avatar';
import { BackgroundService } from './background.service';
import { LogoComponent } from './brand';
import { ChatComponent } from './chat';
import {
  type Conversation,
  conversations,
  initials,
  personConversations,
  preview,
} from './devices';
import { IconComponent } from './icon';
import { RailComponent } from './rail';
import { connectionStatus, shortTime } from './status';

const PRESENCE_EVERY_MS = 15_000;

@Component({
  selector: 'app-home',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AvatarComponent, ChatComponent, IconComponent, LogoComponent, RailComponent],
  template: `
    @let s = state();
    @if (showRail()) {
      <app-rail
        [people]="s.people"
        [items]="bg.items()"
        [me]="s.me"
        [opened]="s.opened"
        [admin]="s.admin"
        [selected]="person()?.user"
        (choose)="choosePerson($event)"
        (invite)="openAdmin()"
      />
    }
    <div class="main">
      @if (open(); as id) {
        <app-chat [channel]="id.channel" [presence]="presence()" (back)="open.set(undefined)" />
      } @else {
        <header class="row">
          @if (person(); as p) {
            <h1 class="grow person-title">{{ p.name }}</h1>
          } @else {
            <h1 class="grow"><app-logo [size]="26" /></h1>
          }
          <button
            class="icon ghost"
            (click)="openSettings()"
            title="Réglages"
            aria-label="Réglages"
          >
            <app-icon name="settings" />
          </button>
        </header>
        @if (status().help; as help) {
          <div class="banner warning">{{ help }}</div>
        }
        @if (s.alerts.length) {
          <div class="banner danger" role="alert">
            Annuaire : {{ s.alerts.length }} alerte{{ s.alerts.length > 1 ? 's' : '' }}, voir les
            réglages.
          </div>
        }
        @if (bg.error(); as error) {
          <div class="banner danger" role="alert">{{ error }}</div>
        }
        <ul class="chats" aria-label="Conversations">
          @for (c of list(); track c.id) {
            <li>
              <button class="chat" (click)="select(c)">
                <span class="avatar">
                  <app-avatar [kind]="c.kind" [color]="c.color" [initials]="initialsOf(c.name)" />
                  @if (isOnline(c)) {
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
                  @if (c.note; as note) {
                    <span class="note" [class.warn]="c.outdated">{{ note }}</span>
                  }
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
    </div>
  `,
  styles: `
    :host {
      display: flex;
      height: 580px;
    }
    .main {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
    }
    .person-title {
      font-size: var(--text-title);
      font-weight: 600;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .note {
      font-size: 11px;
      color: var(--muted);
    }
    .note.warn {
      color: var(--warning-fg);
    }
    header {
      flex: none;
      padding: 10px 12px 6px 16px;
    }
    h1 {
      margin: 0;
      display: flex;
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
  protected readonly personId = signal<string | undefined>(undefined);
  protected readonly person = computed(() => {
    const id = this.personId();
    return id ? this.state().people.find((p) => p.user === id) : undefined;
  });
  protected readonly showRail = computed(() => {
    const s = this.state();
    return !!s.me && (s.admin || s.people.some((p) => p.user !== s.me));
  });
  protected readonly list = computed(() => {
    const s = this.state();
    const p = this.person();
    if (p) return personConversations(p, this.bg.items(), s.device?.id);
    const mine = s.people.find((x) => x.user === s.me)?.devices ?? [];
    return conversations(s.group?.members ?? [], this.bg.items(), s.device?.id, mine);
  });
  protected readonly open = signal<{ channel: string } | undefined>(undefined);
  protected readonly presence = signal<ReadonlyMap<string, boolean> | undefined>(undefined);

  constructor() {
    effect(() => {
      const channel = this.bg.openChat();
      if (channel === undefined) return;
      untracked(() => {
        this.personId.set(parseChannel(channel).user);
        const list = this.list();
        const c = list.find((x) => x.id === channel) ?? list[0];
        if (c) this.select(c);
        this.bg.openChat.set(undefined);
      });
    });
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
      this.presence.set(undefined);
    }
  }

  protected select(c: Conversation): void {
    this.open.set({ channel: c.id });
    void this.bg.run({ type: 'set-recipient', to: c.id });
  }

  protected choosePerson(user: string | undefined): void {
    this.open.set(undefined);
    this.personId.set(user);
  }

  protected initialsOf(name: string): string {
    return initials(name);
  }

  protected isOnline(c: Conversation): boolean {
    return c.online ?? this.presence()?.get(c.id) ?? false;
  }

  protected openAdmin(): Promise<chrome.tabs.Tab> {
    return chrome.tabs.create({ url: 'admin.html' });
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
