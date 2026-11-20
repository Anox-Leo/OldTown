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

export function saveTheme(theme: Theme, storage: Storage = localStorage): void {
  if (theme === 'system') storage.removeItem(THEME_KEY);
  else storage.setItem(THEME_KEY, theme);
  applyTheme(theme);
}

export function followTheme(): void {
  applyTheme(readTheme());
  addEventListener('storage', (e) => {
    if (e.key === THEME_KEY || e.key === null) applyTheme(readTheme());
  });
}
