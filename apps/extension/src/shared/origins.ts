export const OTP_ORIGINS = ['https://*/*'];

export function httpBase(serverUrl: string): string {
  const raw = serverUrl.trim();
  const u = new URL(raw.includes('://') ? raw : `ws://${raw}`);
  const scheme = u.protocol === 'wss:' || u.protocol === 'https:' ? 'https:' : 'http:';
  return `${scheme}//${u.host}`;
}

export function originPattern(url: string): string {
  const u = new URL(url);
  return `${u.protocol}//${u.hostname}/*`;
}

export function relayOrigin(serverUrl: string): string {
  return originPattern(httpBase(serverUrl));
}
