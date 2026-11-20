import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { BackgroundService } from './background.service';
import { LogoComponent } from './brand';
import { suggestedName } from './status';

/**
 * Premier lancement : « Rejoindre un appareil à moi » (URL du relais, nom,
 * puis QR inversé ou code collé), ou « J'ai reçu une invitation » (étape
 * 10 : le lien ou le code reçu de l'admin, qui porte déjà le relais).
 */
@Component({
  selector: 'app-setup',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, LogoComponent],
  template: `
    <h1><app-logo [size]="36" /></h1>
    <p class="muted">
      Textes et liens entre vos appareils, chiffrés de bout en bout. Le relais ne voit jamais le
      contenu.
    </p>

    <div class="modes" role="tablist">
      <button
        role="tab"
        [attr.aria-selected]="mode() !== 'invite'"
        [class.selected]="mode() !== 'invite'"
        (click)="mode.set('qr')"
      >
        Rejoindre un appareil à moi
      </button>
      <button
        role="tab"
        [attr.aria-selected]="mode() === 'invite'"
        [class.selected]="mode() === 'invite'"
        (click)="mode.set('invite')"
      >
        J'ai reçu une invitation
      </button>
    </div>

    @if (mode() !== 'invite') {
      <label for="url">URL du relais</label>
      <input
        id="url"
        [(ngModel)]="serverUrl"
        placeholder="wss://relais.exemple.ts.net:7878"
        autocomplete="off"
        spellcheck="false"
      />
    }
    <label for="name">Nom de cet appareil</label>
    <input id="name" [(ngModel)]="name" maxlength="64" autocomplete="off" />

    @if (mode() === 'invite') {
      <label for="invite">Lien d'invitation</label>
      <textarea
        id="invite"
        [(ngModel)]="invite"
        placeholder="oldtown://invite?…"
        spellcheck="false"
      ></textarea>
      <button class="primary wide" [disabled]="busy() || !invite.trim()" (click)="acceptInvite()">
        Rejoindre
      </button>
      <p class="small muted">
        Le lien reçu de la personne qui vous invite. Il contient un secret : ne le partagez pas.
      </p>
    } @else if (mode() === 'qr') {
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
      display: flex;
      margin: 4px 0 10px;
    }
    .wide {
      width: 100%;
      margin: 14px 0 6px;
    }
    .modes {
      display: flex;
      gap: 6px;
      margin: 8px 0 4px;
    }
    .modes button {
      flex: 1;
    }
    .modes button.selected {
      border-color: var(--primary);
      color: var(--primary-text);
    }
  `,
})
export class SetupComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly mode = signal<'qr' | 'code' | 'invite'>('qr');
  protected readonly busy = signal(false);
  protected serverUrl = 'wss://';
  protected name = suggestedName(navigator.platform, 'brave' in navigator ? 'Brave' : 'Chrome');
  protected code = '';
  protected invite = '';

  protected async acceptInvite(): Promise<void> {
    this.busy.set(true);
    await this.bg.run({ type: 'setup-invite', text: this.invite, name: this.name });
    this.busy.set(false);
  }

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
