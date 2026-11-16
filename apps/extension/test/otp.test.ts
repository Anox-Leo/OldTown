// @vitest-environment jsdom
// Codes à usage unique (note 0045) : site de l'expéditeur, champ à remplir.
import { beforeAll, describe, expect, it } from 'vitest';
import { fillOtp, isLive, sameSite } from '../src/shared/otp';

describe('sameSite', () => {
  it('compare les domaines enregistrables', () => {
    expect(sameSite('authenticate.riotgames.com', 'mail.accounts.riotgames.com')).toBe(true);
    expect(sameSite('accounts.google.com', 'google.com')).toBe(true);
    expect(sameSite('www.notion.so', 'mail.notion.so')).toBe(true);
    expect(sameSite('GitHub.com.', 'github.com')).toBe(true);
    expect(sameSite('login.service.co.uk', 'mail.service.co.uk')).toBe(true);
  });

  it('refuse les autres sites', () => {
    expect(sameSite('login.live.com', 'microsoft.com')).toBe(false);
    expect(sameSite('google.com.evil.example', 'google.com')).toBe(false);
    expect(sameSite('evilgoogle.com', 'google.com')).toBe(false);
    // Suffixes publics et privés : deux sites distincts.
    expect(sameSite('a.co.uk', 'b.co.uk')).toBe(false);
    expect(sameSite('evil.github.io', 'victim.github.io')).toBe(false);
    expect(sameSite('github.io', 'github.io')).toBe(false);
    expect(sameSite('localhost', 'localhost')).toBe(false);
    expect(sameSite('192.168.1.2', '192.168.1.2')).toBe(false);
  });
});

describe('isLive', () => {
  it('compare à la fin de validité', () => {
    expect(isLive(2_000, 1_000)).toBe(true);
    expect(isLive(1_000, 1_000)).toBe(false);
  });
});

describe('fillOtp', () => {
  beforeAll(() => {
    // jsdom ne calcule pas de mise en page : tout champ est visible, sauf `hidden`.
    HTMLElement.prototype.getClientRects = function (this: HTMLElement) {
      const rects = this.hidden ? [] : [new DOMRect(0, 0, 10, 10)];
      return rects as unknown as DOMRectList;
    };
  });

  const page = (html: string) => {
    document.body.innerHTML = html;
  };
  const value = (sel: string) => document.querySelector<HTMLInputElement>(sel)?.value;

  it('remplit le champ marqué one-time-code et prévient la page', () => {
    page('<input id="u" name="code-promo"><input id="o" autocomplete="one-time-code">');
    let seen = '';
    document.querySelector('#o')?.addEventListener('input', (e) => {
      seen = (e.target as HTMLInputElement).value;
    });
    expect(fillOtp('482913', false)).toBe('filled');
    expect(value('#o')).toBe('482913');
    expect(seen).toBe('482913');
    expect(value('#u')).toBe('');
  });

  it('répartit le code dans une rangée de cases', () => {
    page(
      '<form>' +
        [0, 1, 2, 3, 4, 5].map((i) => `<input id="c${i}" maxlength="1">`).join('') +
        '</form>',
    );
    expect(fillOtp('482913', false)).toBe('filled');
    expect([0, 1, 2, 3, 4, 5].map((i) => value(`#c${i}`)).join('')).toBe('482913');
  });

  it('devine le champ à son nom ou à son libellé', () => {
    page('<input id="e" type="email"><label>Code de vérification <input id="v"></label>');
    expect(fillOtp('482913', false)).toBe('filled');
    expect(value('#v')).toBe('482913');
  });

  it('ne touche ni aux mots de passe, ni aux codes postaux, ni aux champs cachés', () => {
    page(
      '<input id="p" type="password" name="pin">' +
        '<input id="z" name="postal_code">' +
        '<input id="h" name="otp" hidden>' +
        '<input id="d" name="otp" disabled>',
    );
    expect(fillOtp('482913', false)).toBe('no-field');
    for (const id of ['#p', '#z', '#h', '#d']) expect(value(id)).toBe('');
  });

  it('au raccourci, prend le champ qui a le focus', () => {
    page('<input id="a" name="reponse"><input id="b">');
    document.querySelector<HTMLInputElement>('#b')?.focus();
    expect(fillOtp('482913', false)).toBe('no-field');
    expect(fillOtp('482913', true)).toBe('filled');
    expect(value('#b')).toBe('482913');
  });
});
