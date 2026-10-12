import type { StateView } from '../shared/api';

export interface Status {
  text: string;
  tone: 'ok' | 'warning' | 'danger' | 'muted';
  /** Piste de correction, quand il y en a une. */
  help?: string;
}

const TAILSCALE_HELP =
  "Relais injoignable. Vérifier que Tailscale est connecté sur cet ordinateur, et que l'URL du relais est la bonne (nom complet en .ts.net).";

/** État de la connexion, pour l'en-tête et les réglages. */
export function connectionStatus(state: StateView): Status {
  if (state.phase === 'unconfigured') return { text: 'Pas encore appairé', tone: 'muted' };
  const waitingSponsor = state.phase === 'pairing';
  switch (state.connection) {
    case 'online':
      return waitingSponsor
        ? { text: 'En attente du parrain', tone: 'warning' }
        : { text: 'Connecté', tone: 'ok' };
    case 'connecting':
      return { text: 'Connexion…', tone: 'muted' };
    case 'auth_failed':
      return waitingSponsor
        ? { text: 'En attente du téléphone', tone: 'warning' }
        : {
            text: 'Refusé par le relais',
            tone: 'danger',
            help: "Le relais ne reconnaît plus cet appareil (révoqué ?). L'oublier dans les réglages, puis l'appairer à nouveau.",
          };
    case 'offline':
      return { text: 'Hors ligne', tone: 'danger', help: TAILSCALE_HELP };
  }
}

/** Nom proposé pour l'appareil, d'après le système et le navigateur (Brave, Chrome). */
export function suggestedName(platform: string, browser = 'Chrome'): string {
  const p = platform.toLowerCase();
  if (p.includes('mac')) return `${browser} Mac`;
  if (p.includes('win')) return `${browser} Windows`;
  if (p.includes('linux') || p.includes('x11')) return `${browser} Linux`;
  if (p.includes('cros')) return 'Chromebook';
  return browser;
}

/** Heure ou date courte d'un élément reçu. */
export function shortTime(iso: string, now = new Date()): string {
  const d = new Date(iso);
  const sameDay = d.toDateString() === now.toDateString();
  return sameDay
    ? d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' });
}
