import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import type { RelayDevice, StateView } from '../shared/api';
import { BackgroundService } from './background.service';
import { connectionStatus } from './status';

/** Page de réglages : appareil, relais, état, raccourci, oubli. */
@Component({
  selector: 'app-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule],
  template: `
    <h1>OldTown · réglages</h1>
    @let s = state();
    @if (s.device; as d) {
      <section class="card">
        <h2>Cet appareil</h2>
        <p>
          <strong>{{ d.name }}</strong
          ><span class="muted mono id">{{ d.id }}</span>
        </p>
        <p class="small" [class]="status().tone">● {{ status().text }}</p>
        @if (s.connectionError && s.connection !== 'online') {
          <p class="small muted">Dernière erreur : {{ s.connectionError }}</p>
        }
        @if (status().help; as help) {
          <div class="banner warning">{{ help }}</div>
        }
        <label for="url">URL du relais</label>
        <div class="row">
          <input
            id="url"
            class="grow"
            [(ngModel)]="serverUrl"
            spellcheck="false"
            autocomplete="off"
          />
          <button (click)="saveUrl()" [disabled]="serverUrl === d.serverUrl">Enregistrer</button>
        </div>
      </section>

      <section class="card">
        <h2>Appareils sur le relais</h2>
        @for (dev of devices(); track dev.id) {
          <p class="row small">
            <span class="grow">{{ dev.name }}</span>
            <span class="muted">{{ dev.online ? 'en ligne' : 'hors ligne' }}</span>
            @if (!dev.member) {
              <span class="danger">hors du groupe</span>
            }
          </p>
        } @empty {
          <p class="small muted">Disponible une fois connecté.</p>
        }
        <button (click)="loadDevices()">Actualiser</button>
      </section>

      <section class="card">
        <h2>Raccourci clavier</h2>
        <p class="small muted">
          Alt+Maj+S envoie l'adresse de l'onglet au dernier destinataire choisi. La sélection
          s'envoie par le clic droit.
        </p>
        <button (click)="shortcuts()">Modifier les raccourcis</button>
      </section>

      <section class="card">
        <h2>Données</h2>
        <div class="row">
          <button (click)="clearHistory()">Effacer les éléments reçus</button>
          @if (!confirmForget()) {
            <button class="danger" (click)="confirmForget.set(true)">Oublier cet appareil…</button>
          } @else {
            <button class="danger" (click)="forget()">Confirmer l'oubli</button>
          }
        </div>
        @if (confirmForget()) {
          <p class="small muted">
            Efface les clés, le groupe et l'historique de cette extension. Il faudra l'appairer à
            nouveau ; pensez à l'exclure du groupe depuis le téléphone.
          </p>
        }
      </section>
    } @else {
      <p class="muted">Pas encore appairé : ouvrez la popup d'OldTown pour commencer.</p>
    }
    @if (bg.error(); as error) {
      <div class="banner danger" role="alert">{{ error }}</div>
    }
    <p class="small muted">Version {{ s.version }}</p>
  `,
  styles: `
    :host {
      display: block;
      max-width: 640px;
      margin: 24px auto;
      padding: 0 16px;
    }
    h1 {
      font-size: 22px;
    }
    h2 {
      font-size: 15px;
      margin: 0 0 8px;
    }
    .id {
      margin-left: 8px;
    }
    section.card {
      padding: 14px 16px;
      margin-bottom: 12px;
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
export class SettingsComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly status = computed(() => connectionStatus(this.state()));
  protected readonly devices = signal<RelayDevice[]>([]);
  protected readonly confirmForget = signal(false);
  protected serverUrl = '';

  constructor() {
    effect(() => {
      this.serverUrl = this.state().device?.serverUrl ?? '';
    });
    void this.loadDevices();
  }

  protected async loadDevices(): Promise<void> {
    try {
      this.devices.set(await this.bg.request<RelayDevice[]>({ type: 'devices' }));
    } catch {
      this.devices.set([]);
    }
  }

  protected saveUrl(): Promise<unknown> {
    return this.bg.run({ type: 'set-server-url', url: this.serverUrl });
  }

  protected async clearHistory(): Promise<void> {
    await this.bg.run({ type: 'clear-history' });
    await this.bg.refreshHistory();
  }

  protected async forget(): Promise<void> {
    await this.bg.run({ type: 'forget' });
    this.confirmForget.set(false);
  }

  protected shortcuts(): Promise<chrome.tabs.Tab> {
    return chrome.tabs.create({ url: 'chrome://extensions/shortcuts' });
  }
}
