import type { Item, MemberView } from '../shared/api';
import { ALL_NAME, avatarColor, conversations, deviceKind, preview } from './devices';

const member = (id: string, name: string): MemberView => ({
  device_id: id,
  name,
  sign_fingerprint: '',
  exchange_fingerprint: '',
  added_epoch: 1,
});

const at = (n: number) => new Date(Date.UTC(2026, 9, 15, 10, n)).toISOString();

describe('devices', () => {
  it('devine le type d’appareil à son nom', () => {
    expect(deviceKind('Chrome Mac')).toBe('browser');
    expect(deviceKind('Brave Windows')).toBe('browser');
    expect(deviceKind('Téléphone')).toBe('phone');
    expect(deviceKind('Pixel 9')).toBe('phone');
    expect(deviceKind('PC Windows')).toBe('desktop');
    expect(deviceKind('Mac mini')).toBe('desktop');
    expect(deviceKind('MacBook Air')).toBe('laptop');
    expect(deviceKind('relais')).toBe('laptop');
  });

  it('une couleur stable par appareil', () => {
    expect(avatarColor('dev_a')).toBe(avatarColor('dev_a'));
    expect(avatarColor('dev_a')).toMatch(/^#[0-9a-f]{6}$/);
  });

  it('« Tous » en premier, puis du plus récemment actif au moins récent', () => {
    const sent = (id: string, to: string[], n: number): Item => ({
      msgId: id,
      from: 'dev_me',
      fromName: 'Moi',
      createdAt: at(n),
      kind: 'text',
      body: id,
      to,
      outgoing: true,
    });
    const list = conversations(
      [
        member('dev_me', 'Moi'),
        member('dev_a', 'Alpha'),
        member('dev_b', 'Bravo'),
        member('dev_c', 'Charlie'),
      ],
      [sent('1', ['dev_c'], 1), sent('2', ['dev_b'], 2), sent('3', [], 3)],
      'dev_me',
    );
    expect(list.map((c) => c.name)).toEqual([ALL_NAME, 'Bravo', 'Charlie', 'Alpha']);
    expect(list.map((c) => c.last?.msgId)).toEqual(['3', '2', '1', undefined]);
    expect(list[0]?.kind).toBe('group');
  });

  it('aperçu sur une ligne, avec l’auteur', () => {
    const item: Item = {
      msgId: 'a',
      from: 'dev_tel',
      fromName: 'Téléphone',
      createdAt: at(1),
      kind: 'text',
      body: 'ligne 1\nligne 2',
    };
    expect(preview(item, 'dev_tel')).toBe('ligne 1 ligne 2');
    expect(preview(item, '')).toBe('Téléphone : ligne 1 ligne 2');
    expect(preview({ ...item, outgoing: true }, '')).toBe('Vous : ligne 1 ligne 2');
  });
});
