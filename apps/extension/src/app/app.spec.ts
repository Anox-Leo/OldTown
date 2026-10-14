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
      transfers: [],
      relayAccess: true,
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
    // Canaux (note 0030) : « Tous », puis un par appareil.
    const channels = [...el.querySelectorAll('.channels button')].map((b) => b.textContent?.trim());
    expect(channels).toEqual(['Tous', 'Téléphone']);
    expect(el.textContent).toContain('Aucun message pour tous vos appareils');
    // La zone d'envoi est en bas, sans liste de destinataires.
    expect(el.querySelector('footer.composer textarea')).toBeTruthy();
    expect(el.querySelector('select')).toBeNull();
  });

  it('conversation : envoyés à droite, reçus à gauche, un canal par appareil', () => {
    const bg = new FakeBackground();
    const members = ['dev_me', 'dev_tel'].map((id) => ({
      device_id: id,
      name: id === 'dev_me' ? 'Chrome Mac' : 'Téléphone',
      sign_fingerprint: '',
      exchange_fingerprint: '',
      added_epoch: 1,
    }));
    const state = (recipient: string): StateView => ({
      version: '0.1.0',
      phase: 'ready',
      connection: 'online',
      recipient,
      pendingSends: 0,
      transfers: [],
      relayAccess: true,
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: { epoch: 1, key_fingerprint: '', roster_version: 1, members },
    });
    const at = (n: number) => new Date(Date.UTC(2026, 9, 15, 10, n)).toISOString();
    bg.state.set(state(''));
    bg.items.set([
      // Du plus récent au plus ancien, comme l'historique.
      {
        msgId: 'c',
        from: 'dev_tel',
        fromName: 'Téléphone',
        createdAt: at(3),
        kind: 'text',
        body: 'rien que pour toi',
        to: ['dev_me'],
      },
      {
        msgId: 'b',
        from: 'dev_me',
        fromName: 'Chrome Mac',
        createdAt: at(2),
        kind: 'text',
        body: 'salut à tous',
        to: [],
        outgoing: true,
        pending: true,
      },
      {
        msgId: 'a',
        from: 'dev_tel',
        fromName: 'Téléphone',
        createdAt: at(1),
        kind: 'text',
        body: 'bonjour',
        to: [],
      },
    ]);
    TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const bubbles = () =>
      [...el.querySelectorAll('.messages li.msg')].map((li) => [
        li.classList.contains('out') ? 'moi' : 'reçu',
        li.querySelector('.text')?.textContent?.trim(),
      ]);
    // « Tous » : dans l'ordre, sans le message privé ; l'envoi en cours est signalé.
    expect(bubbles()).toEqual([
      ['reçu', 'bonjour'],
      ['moi', 'salut à tous'],
    ]);
    expect(el.textContent).toContain('envoi…');
    // Canal du téléphone : seulement le message qui m'était adressé.
    bg.state.set(state('dev_tel'));
    fixture.detectChanges();
    expect(bubbles()).toEqual([['reçu', 'rien que pour toi']]);
    expect(el.querySelector('textarea')?.getAttribute('placeholder')).toBe('Message à Téléphone');
  });

  it('canaux triés du plus utilisé au moins utilisé, « Tous » en premier', () => {
    const bg = new FakeBackground();
    const member = (id: string, name: string) => ({
      device_id: id,
      name,
      sign_fingerprint: '',
      exchange_fingerprint: '',
      added_epoch: 1,
    });
    bg.state.set({
      version: '0.1.0',
      phase: 'ready',
      connection: 'online',
      recipient: '',
      pendingSends: 0,
      transfers: [],
      relayAccess: true,
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: {
        epoch: 1,
        key_fingerprint: '',
        roster_version: 1,
        members: [
          member('dev_me', 'Chrome Mac'),
          member('dev_a', 'Alpha'),
          member('dev_b', 'Bravo'),
          member('dev_c', 'Charlie'),
        ],
      },
    });
    const at = (n: number) => new Date(Date.UTC(2026, 9, 15, 10, n)).toISOString();
    const sent = (id: string, to: string, n: number) => ({
      msgId: id,
      from: 'dev_me',
      fromName: 'Chrome Mac',
      createdAt: at(n),
      kind: 'text' as const,
      body: id,
      to: [to],
      outgoing: true,
    });
    const received = (id: string, from: string, n: number) => ({
      msgId: id,
      from,
      fromName: from,
      createdAt: at(n),
      kind: 'text' as const,
      body: id,
      to: ['dev_me'],
    });
    // Charlie : 3 messages ; Bravo : 1 ; Alpha : aucun. Les messages à tous
    // ne comptent pour aucun appareil.
    bg.items.set([
      sent('1', 'dev_c', 1),
      received('2', 'dev_c', 2),
      sent('3', 'dev_c', 3),
      received('4', 'dev_b', 4),
      { ...sent('5', 'dev_a', 5), to: [] },
    ]);
    TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    const channels = [...el.querySelectorAll('.channels button')].map((b) => b.textContent?.trim());
    expect(channels).toEqual(['Tous', 'Charlie', 'Bravo', 'Alpha']);
  });

  it('fichiers reçus et transferts en cours', () => {
    const bg = new FakeBackground();
    bg.state.set({
      version: '0.1.0',
      phase: 'ready',
      connection: 'online',
      recipient: '',
      pendingSends: 0,
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: { epoch: 1, key_fingerprint: '', roster_version: 1, members: [] },
      transfers: [{ id: 'outgoing/x', direction: 'up', name: 'video.mp4', done: 50, total: 200 }],
      relayAccess: false,
    });
    const file = (name: string, cached?: boolean) => ({
      msgId: name,
      from: 'dev_tel',
      fromName: 'Téléphone',
      createdAt: new Date().toISOString(),
      kind: 'file' as const,
      body: name,
      file: { blobId: `blob_${name}`, name, mime: 'image/jpeg', size: 1_572_864, cached },
      to: [],
    });
    bg.items.set([file('photo.jpg', true), file('gros.mov')]);
    TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    // Envoi en cours : une bulle à droite, avec sa progression.
    expect(el.querySelector('.messages li.msg.out')?.textContent).toContain('video.mp4');
    expect(el.textContent).toContain('25 %');
    expect(el.textContent).toContain('1,5 Mo');
    // Icône de téléchargement à droite de chaque fichier, copié ou non
    // (note 0029) ; plus de boutons texte sous les éléments.
    const downloads = [...el.querySelectorAll('.messages a.side[aria-label="Télécharger"]')];
    expect(downloads.length).toBe(2);
    // Une navigation, servie par le service worker : jamais un lien `download`,
    // que le navigateur irait chercher sans lui (essai du 15 octobre).
    for (const a of downloads) {
      expect(a.getAttribute('target')).toBe('_blank');
      expect(a.hasAttribute('download')).toBe(false);
    }
    expect(downloads[0]?.getAttribute('href')).toBe('f/blob_photo.jpg/photo.jpg?dl=1');
    expect(el.querySelector('[aria-label="Envoyer un fichier"]')).toBeTruthy();
    // Sans accès au relais, le bandeau propose de l'autoriser.
    expect(el.textContent).toContain("autoriser l'accès au relais");
  });
});
