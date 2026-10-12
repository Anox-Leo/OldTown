import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { Item, StateView } from '../shared/api';
import { App } from './app';
import { BackgroundService } from './background.service';

class FakeBackground {
  readonly state = signal<StateView | undefined>(undefined);
  readonly items = signal<Item[]>([]);
  readonly error = signal<string | undefined>(undefined);
  connect(): void {
    // Pas de service worker sous jsdom.
  }
  run(): Promise<undefined> {
    return Promise.resolve(undefined);
  }
}

function render(state?: Partial<StateView>) {
  const bg = new FakeBackground();
  if (state) {
    bg.state.set({
      version: '0.1.0',
      phase: 'unconfigured',
      connection: 'offline',
      recipient: '',
      pendingSends: 0,
      ...state,
    });
  }
  TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
  const fixture = TestBed.createComponent(App);
  fixture.detectChanges();
  return fixture.nativeElement as HTMLElement;
}

describe('App (popup)', () => {
  it('attend l’état du service worker', () => {
    expect(render().textContent).toContain('Chargement');
  });

  it('premier lancement : réglage du relais', () => {
    const el = render({ phase: 'unconfigured' });
    expect(el.querySelector('#url')).toBeTruthy();
    expect(el.textContent).toContain('Afficher un QR');
  });

  it('QR inversé affiché pendant l’appairage', () => {
    const el = render({
      phase: 'pairing',
      device: { id: 'dev_x', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      pending: {
        kind: 'offer',
        qr: { size: 1, modules: [1] },
        text: 'oldtown://sponsor?o=x',
        expiresAt: new Date(Date.now() + 60_000).toISOString(),
      },
    });
    expect(el.querySelector('app-qr svg')).toBeTruthy();
    expect(el.textContent).toContain('Ajouter un appareil');
  });

  it('dans le groupe : envoi et éléments reçus', () => {
    const el = render({
      phase: 'ready',
      connection: 'online',
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: {
        epoch: 1,
        key_fingerprint: 'ab',
        roster_version: 2,
        members: [
          {
            device_id: 'dev_me',
            name: 'Chrome Mac',
            sign_fingerprint: '',
            exchange_fingerprint: '',
            added_epoch: 1,
          },
          {
            device_id: 'dev_tel',
            name: 'Téléphone',
            sign_fingerprint: '',
            exchange_fingerprint: '',
            added_epoch: 1,
          },
        ],
      },
    });
    expect(el.textContent).toContain('Connecté');
    const options = [...el.querySelectorAll('select option')].map((o) => o.textContent?.trim());
    expect(options).toEqual(['Tous mes appareils', 'Téléphone']);
    expect(el.textContent).toContain('Rien reçu');
  });
});
