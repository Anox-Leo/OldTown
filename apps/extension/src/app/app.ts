import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';

import { BackgroundService } from './background.service';
import { HomeComponent } from './home';
import { PairingComponent } from './pairing';
import { SettingsComponent } from './settings';
import { SetupComponent } from './setup';

/**
 * Racine : la même appli sert la popup (`index.html`) et les réglages
 * (`options.html`).
 */
@Component({
  selector: 'app-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HomeComponent, PairingComponent, SettingsComponent, SetupComponent],
  template: `
    @if (options) {
      @if (bg.state()) {
        <app-settings />
      }
    } @else {
      @switch (phase()) {
        @case ('unconfigured') {
          <app-setup />
        }
        @case ('pairing') {
          <app-pairing />
        }
        @case ('ready') {
          <app-home />
        }
        @default {
          <p class="loading muted">Chargement…</p>
        }
      }
    }
  `,
  styles: `
    .loading {
      padding: 16px;
    }
  `,
})
export class App {
  protected readonly bg = inject(BackgroundService);
  protected readonly options = location.pathname.endsWith('options.html');
  protected readonly phase = computed(() => this.bg.state()?.phase);

  constructor() {
    if (!this.options) document.body.classList.add('popup');
    this.bg.connect();
  }
}
