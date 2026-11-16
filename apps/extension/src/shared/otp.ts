// Codes à usage unique (note 0045) : à quelle page un code peut aller d'office,
// et la fonction injectée dans la page pour le placer dans son champ.

import { getDomain } from 'tldts';

/**
 * Le site de l'onglet est-il celui qui a envoyé le code ? Vrai si les deux
 * noms ont le même domaine enregistrable, d'après la liste des suffixes
 * publics, suffixes privés compris (`mail.accounts.riotgames.com` ↔
 * `authenticate.riotgames.com`, mais jamais `a.github.io` ↔ `b.github.io`).
 * Ni adresse IP, ni nom sans domaine (`localhost`).
 */
export function sameSite(pageHost: string, senderDomain: string): boolean {
  const site = (h: string) =>
    getDomain(h.trim().toLowerCase().replace(/\.$/, ''), { allowPrivateDomains: true });
  const page = site(pageHost);
  return page !== null && page === site(senderDomain);
}

/** Code encore valable (`expiresAt` en millisecondes Unix). */
export function isLive(expiresAt: number, now = Date.now()): boolean {
  return expiresAt > now;
}

/** Résultat de la fonction injectée. */
export type FillResult = 'filled' | 'no-field';

/**
 * Place `code` dans le champ du code de la page. Injectée telle quelle par
 * `chrome.scripting.executeScript` : aucune référence hors de son corps.
 *
 * Ordre : champ `autocomplete="one-time-code"` ; une rangée de cases d'un
 * caractère, autant que de caractères du code ; un champ dont le nom, l'id,
 * le texte indicatif ou le libellé parle de code (pas d'un code postal ou
 * promotionnel). `focused` (raccourci) :
 * le champ qui a le focus passe avant les indices. Jamais un champ de mot de
 * passe, sauf marqué `one-time-code`.
 */
export function fillOtp(code: string, focused: boolean): FillResult {
  const visible = (el: HTMLElement) => el.getClientRects().length > 0 || el.offsetParent !== null;
  const typable = (el: HTMLInputElement) =>
    !el.disabled && !el.readOnly && ['', 'text', 'tel', 'number', 'password'].includes(el.type);
  const inputs = [...document.querySelectorAll('input')].filter((el) => typable(el) && visible(el));
  const isOtp = (el: HTMLInputElement) => el.autocomplete === 'one-time-code';
  const usable = inputs.filter((el) => el.type !== 'password' || isOtp(el));

  const setValue = (el: HTMLInputElement, value: string) => {
    // Le setter natif : les pages React ou Vue ne voient pas une simple affectation.
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    el.focus();
    if (setter) setter.call(el, value);
    else el.value = value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  };

  const active = document.activeElement;
  const activeInput =
    active instanceof HTMLInputElement && usable.includes(active) ? active : undefined;

  const single = usable.find(isOtp);
  if (single && !(single.maxLength === 1 && code.length > 1)) {
    setValue(single, code);
    return 'filled';
  }

  // Cases d'un caractère : toutes dans le même formulaire (ou le document).
  const boxes = usable.filter((el) => el.maxLength === 1);
  const scope = activeInput?.form ?? boxes[0]?.form ?? null;
  const row = boxes.filter((el) => el.form === scope);
  if (row.length === code.length) {
    row.forEach((el, i) => setValue(el, code.charAt(i)));
    return 'filled';
  }

  if (focused && activeInput && activeInput.maxLength !== 1) {
    setValue(activeInput, code);
    return 'filled';
  }

  const hint = /one.?time|otp|totp|2fa|mfa|code|token|verif|pin|passcode|security|auth/i;
  const notOtp =
    /postal|zip|promo|coupon|discount|gift|voucher|country|phone|author|search|captcha/i;
  const label = (el: HTMLInputElement) =>
    [
      el.name,
      el.id,
      el.placeholder,
      el.getAttribute('aria-label') ?? '',
      ...[...(el.labels ?? [])].map((l) => l.textContent ?? ''),
    ].join(' ');
  const guessed = usable.find(
    (el) =>
      el.type !== 'password' &&
      el.maxLength !== 1 &&
      hint.test(label(el)) &&
      !notOtp.test(label(el)),
  );
  if (guessed) {
    setValue(guessed, code);
    return 'filled';
  }
  return 'no-field';
}
