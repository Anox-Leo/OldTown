import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { BackgroundService } from './background.service';
import { suggestedName } from './status';

/** Premier lancement : URL du relais, nom, puis QR inversé ou code collé. */
@Component({
  selector: 'app-setup',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule],
  template: `
    <h1>OldTown</h1>
    <p class="muted">
      Textes et liens entre vos appareils, chiffrés de bout en bout. Le relais ne voit jamais le
      contenu.
    </p>

    <label for="url">URL du relais</label>
    <input
      id="url"
      [(ngModel)]="serverUrl"
      placeholder="wss://relais.exemple.ts.net:7878"
      autocomplete="off"
      spellcheck="false"
    />
    <label for="name">Nom de cet appareil</label>
    <input id="name" [(ngModel)]="name" maxlength="64" autocomplete="off" />

    @if (mode() === 'qr') {
      <button class="primary wide" [disabled]="busy()" (click)="offer()">
        Afficher un QR à scanner
      </button>
      <p class="small muted">
        Le téléphone, déjà dans le groupe, scanne ce QR (Groupe → « Ajouter un appareil »).
      </p>
      <button class="link" (click)="mode.set('code')">J'ai un code d'appairage</button>
    } @else {
      <label for="code">Code d'appairage ou texte du QR « rejoindre »</label>
      <textarea id="code" [(ngModel)]="code" placeholder="ot1-…" spellcheck="false"></textarea>
      <button class="primary wide" [disabled]="busy() || !code.trim()" (click)="join()">
        Rejoindre
      </button>
      <p class="small muted">Code affiché par « ot pair new » sur un autre appareil du groupe.</p>
      <button class="link" (click)="mode.set('qr')">Afficher plutôt un QR</button>
    }

    @if (bg.state()?.connectionError; as problem) {
      <div class="banner warning">{{ problem }}</div>
    }
    @if (bg.error(); as error) {
      <div class="banner danger" role="alert">{{ error }}</div>
    }
  `,
  styles: `
    :host {
      display: block;
      padding: 16px;
    }
    h1 {
      font-size: 20px;
      margin: 0 0 4px;
    }
    .wide {
      width: 100%;
      margin: 14px 0 6px;
    }
  `,
})
export class SetupComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly mode = signal<'qr' | 'code'>('qr');
  protected readonly busy = signal(false);
  protected serverUrl = 'wss://';
  protected name = suggestedName(navigator.platform);
  protected code = '';

  protected async offer(): Promise<void> {
    this.busy.set(true);
    await this.bg.run({ type: 'setup-offer', serverUrl: this.serverUrl, name: this.name });
    this.busy.set(false);
  }

  protected async join(): Promise<void> {
    this.busy.set(true);
    await this.bg.run({
      type: 'setup-code',
      text: this.code,
      serverUrl: this.serverUrl,
      name: this.name,
    });
    this.busy.set(false);
  }
}
