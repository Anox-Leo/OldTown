// Thème de l'interface : celui du système, ou « nuit » (sombre) ou
// « parchemin » (clair) choisi dans les réglages. `tokens.css` lit
// `data-theme` sur `<html>`. Le choix est rangé dans le `localStorage` de
// l'extension (une préférence d'affichage, rien de secret) : lu de façon
// synchrone avant le premier rendu, et partagé par la popup, les réglages et
// `send.html`, qui ont la même origine.

export type Theme = 'system' | 'dark' | 'light';

export const THEME_KEY = 'theme';

export function readTheme(storage: Pick<Storage, 'getItem'> = localStorage): Theme {
  const t = storage.getItem(THEME_KEY);
  return t === 'dark' || t === 'light' ? t : 'system';
}

export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  if (theme === 'system') root.removeAttribute('data-theme');
  else root.dataset['theme'] = theme;
}

/** Range le choix et l'applique à cette page ; les autres le suivent (`followTheme`). */
export function saveTheme(theme: Theme, storage: Storage = localStorage): void {
  if (theme === 'system') storage.removeItem(THEME_KEY);
  else storage.setItem(THEME_KEY, theme);
  applyTheme(theme);
}

/** Applique le thème rangé, puis le suit quand une autre page le change. */
export function followTheme(): void {
  applyTheme(readTheme());
  addEventListener('storage', (e) => {
    if (e.key === THEME_KEY || e.key === null) applyTheme(readTheme());
  });
}
