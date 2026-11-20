import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { Item, RelayDevice, StateView } from '../shared/api';
import { App } from './app';
import { BackgroundService } from './background.service';

class FakeBackground {
  readonly state = signal<StateView | undefined>(undefined);
  readonly items = signal<Item[]>([]);
  readonly error = signal<string | undefined>(undefined);
  readonly openChat = signal<string | undefined>(undefined);
  readonly connect = (): void => undefined;
  devices: RelayDevice[] = [];
  run(): Promise<undefined> {
    return Promise.resolve(undefined);
  }
  request<T>(): Promise<T> {
    return Promise.resolve(this.devices as T);
  }
}

function openChat(fixture: { detectChanges(): void; nativeElement: HTMLElement }, name: string) {
  const row = [...fixture.nativeElement.querySelectorAll<HTMLButtonElement>('.chats button')].find(
    (b) => b.querySelector('.name')?.textContent?.trim() === name,
  );
  if (!row) throw new Error(`pas de conversation ${name}`);
  row.click();
  fixture.detectChanges();
}

const rows = (el: HTMLElement) =>
  [...el.querySelectorAll('.chats .name')].map((n) => n.textContent?.trim());

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
      people: [],
      admin: false,
      alerts: [],
      opened: {},
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

  it('dans le groupe : accueil, liste des conversations', () => {
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
    expect(el.querySelector('h1 app-logo [aria-label="OldTown"]')).toBeTruthy();
    expect(el.querySelector('[aria-label="Réglages"]')).toBeTruthy();
    expect(el.textContent).not.toContain('Connecté');
    expect(rows(el)).toEqual(['Tous mes appareils', 'Téléphone']);
    expect(el.querySelector('.chats app-avatar')).toBeTruthy();
    const avatars = el.querySelectorAll('.chats app-avatar');
    expect(avatars[0]?.querySelector('app-mark')).toBeTruthy();
    expect(avatars[1]?.querySelector('app-mark')).toBeNull();
    expect(avatars[1]?.querySelector('app-icon')).toBeTruthy();
    expect(el.textContent).toContain('Aucun message');
    expect(el.querySelector('textarea')).toBeNull();
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
      people: [],
      admin: false,
      alerts: [],
      opened: {},
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: { epoch: 1, key_fingerprint: '', roster_version: 1, members },
    });
    const at = (n: number) => new Date(Date.UTC(2026, 9, 15, 10, n)).toISOString();
    bg.state.set(state(''));
    bg.items.set([
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
    expect(el.querySelector('.chats li:nth-child(1) .last')?.textContent).toContain(
      'Vous : salut à tous',
    );
    expect(el.querySelector('.chats li:nth-child(2) .last')?.textContent).toContain(
      'rien que pour toi',
    );
    openChat(fixture, 'Tous mes appareils');
    const bubbles = () =>
      [...el.querySelectorAll('.messages li.msg')].map((li) => [
        li.classList.contains('out') ? 'moi' : 'reçu',
        li.querySelector('.text')?.textContent?.trim(),
      ]);
    expect(bubbles()).toEqual([
      ['reçu', 'bonjour'],
      ['moi', 'salut à tous'],
    ]);
    expect(el.textContent).toContain('envoi…');
    el.querySelector<HTMLButtonElement>('[aria-label="Retour"]')?.click();
    fixture.detectChanges();
    expect(el.querySelector('.messages')).toBeNull();
    openChat(fixture, 'Téléphone');
    expect(el.querySelector('header .name')?.textContent?.trim()).toBe('Téléphone');
    expect(el.querySelector('.channels')).toBeNull();
    expect(bubbles()).toEqual([['reçu', 'rien que pour toi']]);
    expect(el.querySelector('textarea')?.getAttribute('placeholder')).toBe('Message à Téléphone');
  });

  it('conversations de la plus récente à la plus ancienne, « Tous » en premier', () => {
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
      people: [],
      admin: false,
      alerts: [],
      opened: {},
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
    expect(rows(el)).toEqual(['Tous mes appareils', 'Bravo', 'Charlie', 'Alpha']);
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
      people: [],
      admin: false,
      alerts: [],
      opened: {},
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
    openChat(fixture, 'Tous mes appareils');
    expect(el.querySelector('.messages li.msg.out')?.textContent).toContain('video.mp4');
    expect(el.textContent).toContain('25 %');
    expect(el.textContent).toContain('1,5 Mo');
    const downloads = [...el.querySelectorAll('.messages a.side[aria-label="Télécharger"]')];
    expect(downloads.length).toBe(2);
    for (const a of downloads) {
      expect(a.getAttribute('target')).toBe('_blank');
      expect(a.hasAttribute('download')).toBe(false);
    }
    expect(downloads[0]?.getAttribute('href')).toBe('f/blob_photo.jpg/photo.jpg?dl=1');
    expect(el.querySelector('[aria-label="Envoyer un fichier"]')).toBeTruthy();
    expect(el.textContent).toContain("autoriser l'accès au relais");
  });

  it('présence : en ligne sous le nom, d’après le relais', async () => {
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
      people: [],
      admin: false,
      alerts: [],
      opened: {},
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: {
        epoch: 1,
        key_fingerprint: '',
        roster_version: 1,
        members: [
          member('dev_me', 'Chrome Mac'),
          member('dev_tel', 'Téléphone'),
          member('dev_pc', 'PC'),
        ],
      },
    });
    bg.devices = [
      { id: 'dev_tel', name: 'Téléphone', online: true, member: true },
      { id: 'dev_pc', name: 'PC', online: false, member: true },
    ];
    TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelectorAll('.chats .dot').length).toBe(1);
    openChat(fixture, 'Téléphone');
    expect(el.querySelector('header .presence')?.textContent?.trim()).toBe('en ligne');
    el.querySelector<HTMLButtonElement>('[aria-label="Retour"]')?.click();
    fixture.detectChanges();
    openChat(fixture, 'PC');
    expect(el.querySelector('header .presence')?.textContent?.trim()).toBe('hors ligne');
    el.querySelector<HTMLButtonElement>('[aria-label="Retour"]')?.click();
    fixture.detectChanges();
    openChat(fixture, 'Tous mes appareils');
    expect(el.querySelector('header .presence')?.textContent?.trim()).toBe('1 en ligne');
  });

  it('ouverte pour un élément reçu : directement sa conversation', () => {
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
      connection: 'offline',
      recipient: '',
      pendingSends: 0,
      transfers: [],
      relayAccess: true,
      people: [],
      admin: false,
      alerts: [],
      opened: {},
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: {
        epoch: 1,
        key_fingerprint: '',
        roster_version: 1,
        members: [member('dev_me', 'Chrome Mac'), member('dev_tel', 'Téléphone')],
      },
    });
    bg.openChat.set('dev_tel');
    TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('header .name')?.textContent?.trim()).toBe('Téléphone');
    expect(bg.openChat()).toBeUndefined();
    el.querySelector<HTMLButtonElement>('[aria-label="Retour"]')?.click();
    fixture.detectChanges();
    expect(el.querySelector('.chats')).toBeTruthy();
    bg.openChat.set('dev_parti');
    fixture.detectChanges();
    expect(el.querySelector('header .name')?.textContent?.trim()).toBe('Tous mes appareils');
  });

  it('barre latérale : mes appareils à l’ouverture (I1), puis une personne et ses appareils (I2)', () => {
    const bg = new FakeBackground();
    const members = ['dev_me', 'dev_tel'].map((id) => ({
      device_id: id,
      name: id === 'dev_me' ? 'Chrome Mac' : 'Téléphone',
      sign_fingerprint: '',
      exchange_fingerprint: '',
      added_epoch: 1,
    }));
    const dev = (device_id: string, name: string, more = {}) => ({
      device_id,
      name,
      reachable: true,
      default: false,
      up_to_date: true,
      online: false,
      ...more,
    });
    bg.state.set({
      version: '0.1.0',
      phase: 'ready',
      connection: 'online',
      recipient: '',
      pendingSends: 0,
      transfers: [],
      relayAccess: true,
      people: [
        {
          user: 'usr_me',
          name: 'Alice',
          role: 'admin',
          muted: false,
          devices: [dev('dev_me', 'Chrome Mac'), dev('dev_tel', 'Téléphone')],
        },
        {
          user: 'usr_bob',
          name: 'Bob',
          role: 'member',
          muted: false,
          devices: [dev('dev_bob', 'bob-phone', { default: true, online: true })],
        },
      ],
      me: 'usr_me',
      admin: true,
      alerts: [],
      opened: {},
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: { epoch: 1, key_fingerprint: '', roster_version: 1, members },
    });
    bg.items.set([
      {
        msgId: 'r',
        from: 'dev_bob',
        fromName: 'bob-phone',
        createdAt: new Date().toISOString(),
        kind: 'text',
        body: 'coucou',
        to: ['dev_me'],
        fromUser: 'usr_bob',
      },
    ]);
    TestBed.configureTestingModule({ providers: [{ provide: BackgroundService, useValue: bg }] });
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const el = fixture.nativeElement as HTMLElement;
    expect(rows(el)).toEqual(['Tous mes appareils', 'Téléphone']);
    const rail = [...el.querySelectorAll<HTMLButtonElement>('app-rail button')];
    expect(rail.map((b) => b.getAttribute('aria-label'))).toEqual([
      'Mes appareils',
      'Bob, 1 non lu',
      'Inviter une personne',
    ]);
    rail[1]?.click();
    fixture.detectChanges();
    expect(el.querySelector('h1')?.textContent?.trim()).toBe('Bob');
    expect(rows(el)).toEqual(['Bob', 'bob-phone']);
    expect(el.querySelector('.chats li:nth-child(1) .last')?.textContent).toContain('coucou');
    openChat(fixture, 'Bob');
    expect(el.querySelector('.messages .msg.in .time')?.textContent).toContain('bob-phone');
    expect(el.querySelector('textarea')?.getAttribute('placeholder')).toBe('Message à Bob');
  });

  it('sans autre personne et sans être admin : pas de barre latérale', () => {
    const el = render({
      phase: 'ready',
      connection: 'online',
      device: { id: 'dev_me', name: 'Chrome Mac', serverUrl: 'wss://relais' },
      group: { epoch: 1, key_fingerprint: '', roster_version: 1, members: [] },
      people: [{ user: 'usr_me', name: 'Léa', role: 'member', muted: false, devices: [] }],
      me: 'usr_me',
    });
    expect(el.querySelector('app-rail')).toBeNull();
  });
});
