import type { StateView } from '../shared/api';
import { connectionStatus, shortTime, suggestedName } from './status';

function state(over: Partial<StateView>): StateView {
  return {
    version: '0.1.0',
    phase: 'ready',
    connection: 'online',
    recipient: '',
    pendingSends: 0,
    ...over,
  };
}

describe('connectionStatus', () => {
  it('connecté', () => {
    expect(connectionStatus(state({}))).toEqual({ text: 'Connecté', tone: 'ok' });
  });

  it('hors ligne : piste Tailscale', () => {
    const s = connectionStatus(state({ connection: 'offline' }));
    expect(s.tone).toBe('danger');
    expect(s.help).toContain('Tailscale');
  });

  it('refus du relais pendant un QR inversé : on attend le téléphone, sans alarme', () => {
    expect(connectionStatus(state({ phase: 'pairing', connection: 'auth_failed' }))).toEqual({
      text: 'En attente du téléphone',
      tone: 'warning',
    });
    expect(connectionStatus(state({ connection: 'auth_failed' })).tone).toBe('danger');
  });
});

describe('suggestedName', () => {
  it('d’après la plateforme', () => {
    expect(suggestedName('MacIntel')).toBe('Chrome Mac');
    expect(suggestedName('Win32')).toBe('Chrome Windows');
    expect(suggestedName('Linux x86_64')).toBe('Chrome Linux');
    expect(suggestedName('')).toBe('Chrome');
  });
});

describe('shortTime', () => {
  it('heure le jour même, date sinon', () => {
    const now = new Date('2026-10-12T15:00:00');
    expect(shortTime(new Date('2026-10-12T09:05:00').toISOString(), now)).toBe('09:05');
    expect(shortTime(new Date('2026-10-01T09:05:00').toISOString(), now)).toMatch(/1 oct/);
  });
});
