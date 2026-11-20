import { getDomain } from 'tldts';

export function sameSite(pageHost: string, senderDomain: string): boolean {
  const site = (h: string) =>
    getDomain(h.trim().toLowerCase().replace(/\.$/, ''), { allowPrivateDomains: true });
  const page = site(pageHost);
  return page !== null && page === site(senderDomain);
}

export function autoFillAllowed(item: { autoFill?: boolean }): boolean {
  return item.autoFill !== false;
}

export function isLive(expiresAt: number, now = Date.now()): boolean {
  return expiresAt > now;
}

export type FillResult = 'filled' | 'no-field';

export function fillOtp(code: string, focused: boolean): FillResult {
  const visible = (el: HTMLElement) => el.getClientRects().length > 0 || el.offsetParent !== null;
  const typable = (el: HTMLInputElement) =>
    !el.disabled && !el.readOnly && ['', 'text', 'tel', 'number', 'password'].includes(el.type);
  const inputs = [...document.querySelectorAll('input')].filter((el) => typable(el) && visible(el));
  const isOtp = (el: HTMLInputElement) => el.autocomplete === 'one-time-code';
  const usable = inputs.filter((el) => el.type !== 'password' || isOtp(el));

  const setValue = (el: HTMLInputElement, value: string) => {
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
