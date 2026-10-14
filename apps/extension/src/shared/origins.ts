// Origines utilisées par les permissions d'hôte optionnelles (note 0027) :
// celle du relais (requêtes HTTP des fichiers, soumises à CORS sans
// permission) et celle d'une image envoyée depuis une page.

/** URL HTTP(S) du relais, d'après son URL WebSocket. */
export function httpBase(serverUrl: string): string {
  const raw = serverUrl.trim();
  const u = new URL(raw.includes('://') ? raw : `ws://${raw}`);
  const scheme = u.protocol === 'wss:' || u.protocol === 'https:' ? 'https:' : 'http:';
  return `${scheme}//${u.host}`;
}

/** Motif de permission d'une URL : `https://hôte/*` (tous les ports). */
export function originPattern(url: string): string {
  const u = new URL(url);
  return `${u.protocol}//${u.hostname}/*`;
}

/** Motif de permission de l'origine du relais. */
export function relayOrigin(serverUrl: string): string {
  return originPattern(httpBase(serverUrl));
}
