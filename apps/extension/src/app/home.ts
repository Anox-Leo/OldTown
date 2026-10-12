import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import type { Item, StateView } from '../shared/api';
import { BackgroundService } from './background.service';
import { IconComponent } from './icon';
import { connectionStatus, shortTime } from './status';

/** Écran principal de la popup : envoi et derniers éléments reçus. */
@Component({
  selector: 'app-home',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, IconComponent],
  template: `
    @let s = state();
    <header class="row">
      <strong class="grow">{{ s.device?.name }}</strong>
      <span class="small" [class]="status().tone">● {{ status().text }}</span>
      <button class="icon" (click)="openSettings()" title="Réglages" aria-label="Réglages">
        <app-icon name="settings" />
      </button>
    </header>
    @if (status().help; as help) {
      <div class="banner warning">{{ help }}</div>
    }

    <section class="send card">
      <textarea
        [(ngModel)]="text"
        placeholder="Texte ou lien à envoyer"
        aria-label="Texte à envoyer"
        (keydown)="onKey($event)"
      ></textarea>
      <div class="row">
        <select
          class="grow"
          [ngModel]="s.recipient"
          (ngModelChange)="setRecipient($event)"
          aria-label="Destinataire"
        >
          <option value="">Tous mes appareils</option>
          @for (m of others(); track m.device_id) {
            <option [value]="m.device_id">{{ m.name }}</option>
          }
        </select>

        <button
          class="icon"
          (click)="sendClipboard()"
          [disabled]="busy()"
          title="Coller et envoyer le presse-papiers"
          aria-label="Coller et envoyer le presse-papiers"
        >
          <app-icon name="clipboard" />
        </button>
        <button
          class="primary icon"
          (click)="send()"
          [disabled]="busy() || !text.trim()"
          title="Envoyer (Entrée)"
          aria-label="Envoyer"
        >
          <app-icon name="send" />
        </button>
      </div>
      @if (notice(); as n) {
        <p class="small muted notice">{{ n }}</p>
      }
      @if (bg.error(); as error) {
        <div class="banner danger" role="alert">{{ error }}</div>
      }
    </section>

    <h2 class="small muted title">Reçus</h2>
    <ul class="items">
      @for (item of bg.items(); track item.msgId) {
        <li class="card">
          <div class="row small muted">
            <span class="grow">{{ item.fromName }}</span>
            <span>{{ time(item.createdAt) }}</span>
          </div>
          @if (isLink(item)) {
            <a class="body" [href]="item.body" target="_blank" rel="noopener noreferrer">{{
              item.body
            }}</a>
          } @else {
            <p class="body">{{ item.body }}</p>
          }
          <div class="row actions">
            <button class="link small" (click)="copy(item)">
              {{ copied() === item.msgId ? 'Copié' : 'Copier' }}
            </button>
          </div>
        </li>
      } @empty {
        <li class="muted empty">Rien reçu pour l'instant.</li>
      }
    </ul>
  `,
  styles: `
    :host {
      display: block;
      padding: 12px;
    }
    header {
      margin-bottom: 8px;
    }
    .icon {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      padding: 7px 10px;
    }
    header .icon {
      border: none;
      background: none;
      padding: 4px;
      color: var(--muted);
    }
    .title {
      margin: 12px 2px 6px;
      font-weight: 500;
    }
    .send {
      padding: 8px;
      display: grid;
      gap: 8px;
    }
    .notice {
      margin: 0;
    }
    .items {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      gap: 6px;
    }
    .items li.card {
      padding: 8px 10px;
    }
    .body {
      display: block;
      margin: 4px 0;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      max-height: 7.5em;
      overflow: hidden;
    }
    a.body {
      color: var(--primary-text);
    }
    .actions {
      justify-content: flex-end;
    }
    .empty {
      text-align: center;
      padding: 24px 0;
    }
    .ok {
      color: var(--success);
    }
    .warning {
      color: var(--warning-fg);
    }
    .danger {
      color: var(--danger-fg);
    }
  `,
})
export class HomeComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly status = computed(() => connectionStatus(this.state()));
  protected readonly others = computed(
    () => this.state().group?.members.filter((m) => m.device_id !== this.state().device?.id) ?? [],
  );
  protected readonly busy = signal(false);
  protected readonly notice = signal<string | undefined>(undefined);
  protected readonly copied = signal<string | undefined>(undefined);
  protected text = '';

  protected time(iso: string): string {
    return shortTime(iso);
  }

  protected isLink(item: Item): boolean {
    return item.kind === 'link' && /^https?:\/\//i.test(item.body);
  }

  private recipientName(to: string): string {
    return to ? (this.others().find((m) => m.device_id === to)?.name ?? to) : 'tous vos appareils';
  }

  protected setRecipient(to: string): Promise<unknown> {
    return this.bg.run({ type: 'set-recipient', to });
  }

  /** Entrée envoie, Maj+Entrée va à la ligne (et rien pendant une composition IME). */
  protected onKey(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void this.send();
    }
  }

  protected async send(): Promise<void> {
    if (!this.text.trim()) return;
    const to = this.state().recipient;
    this.busy.set(true);
    const sent = await this.bg.run<string>({ type: 'send', text: this.text, to });
    this.busy.set(false);
    if (sent) {
      this.text = '';
      this.notice.set(this.sentNotice(to));
    }
  }

  protected async sendClipboard(): Promise<void> {
    this.bg.error.set(undefined);
    let text: string;
    try {
      // Lecture seulement à ce clic (permission clipboardRead, note 0023).
      text = await navigator.clipboard.readText();
    } catch {
      this.bg.error.set('Presse-papiers illisible.');
      return;
    }
    if (!text.trim()) {
      this.bg.error.set('Le presse-papiers ne contient pas de texte.');
      return;
    }
    const to = this.state().recipient;
    this.busy.set(true);
    const sent = await this.bg.run<string>({ type: 'send-clipboard', text, to });
    this.busy.set(false);
    if (sent) this.notice.set(this.sentNotice(to));
  }

  private sentNotice(to: string): string {
    const name = this.recipientName(to);
    return this.state().connection === 'online'
      ? `Envoyé à ${name}.`
      : `Hors ligne : envoi à ${name} au retour de la connexion.`;
  }

  protected async copy(item: Item): Promise<void> {
    try {
      await navigator.clipboard.writeText(item.body);
      this.copied.set(item.msgId);
    } catch {
      this.bg.error.set('Copie impossible.');
    }
  }

  protected openSettings(): Promise<void> {
    return chrome.runtime.openOptionsPage();
  }
}
