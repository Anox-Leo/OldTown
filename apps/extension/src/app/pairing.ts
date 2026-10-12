import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  OnDestroy,
  signal,
} from '@angular/core';

import type { StateView } from '../shared/api';
import { BackgroundService } from './background.service';
import { QrComponent } from './qr';
import { connectionStatus } from './status';

/** Appairage en cours : QR inversé à scanner, ou attente du parrain. */
@Component({
  selector: 'app-pairing',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [QrComponent],
  template: `
    @let s = state();
    @if (s.pending?.kind === 'offer') {
      @let offer = $any(s.pending);
      <h1>Scanner avec le téléphone</h1>
      <p class="muted small">
        Sur le téléphone : Groupe → « Ajouter un appareil », puis visez ce QR. Il contient un secret
        : ne le montrez qu'à votre téléphone.
      </p>
      <div class="qr"><app-qr [qr]="offer.qr" label="QR d'appairage" /></div>
      <p class="small muted center">
        {{ s.device?.name }} · valable encore {{ remaining(offer.expiresAt) }}
      </p>
    } @else if (s.pending?.kind === 'join') {
      <h1>En attente du parrain</h1>
      <p class="muted">
        Demande envoyée. Le groupe arrive dès que l'appareil qui a donné le code est en ligne.
      </p>
    } @else {
      <h1>QR expiré</h1>
      <p class="muted">
        L'appairage n'a pas abouti à temps. Recommencer pour afficher un nouveau QR.
      </p>
    }
    <p class="small" [class]="status().tone">{{ status().text }}</p>
    @if (s.connection === 'offline' && status().help; as help) {
      <div class="banner warning">{{ help }}</div>
    }
    @if (bg.error(); as error) {
      <div class="banner danger" role="alert">{{ error }}</div>
    }
    <button (click)="cancel()">{{ s.pending ? 'Annuler' : 'Recommencer' }}</button>
  `,
  styles: `
    :host {
      display: block;
      padding: 16px;
    }
    h1 {
      font-size: 18px;
      margin: 0 0 6px;
    }
    .qr {
      width: 232px;
      margin: 8px auto;
    }
    .center {
      text-align: center;
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
export class PairingComponent implements OnDestroy {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly status = computed(() => connectionStatus(this.state()));
  private readonly now = signal(Date.now());
  private readonly tick = setInterval(() => this.now.set(Date.now()), 1000);

  protected remaining(expiresAt: string): string {
    const s = Math.max(0, Math.round((Date.parse(expiresAt) - this.now()) / 1000));
    return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s`;
  }

  protected cancel(): Promise<unknown> {
    return this.bg.run({ type: 'cancel-pairing' });
  }

  ngOnDestroy(): void {
    clearInterval(this.tick);
  }
}
