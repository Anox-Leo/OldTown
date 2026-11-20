// @vitest-environment jsdom
// Thème choisi dans les réglages : rangé, appliqué par `data-theme`.
import { afterEach, describe, expect, it } from 'vitest';

import { THEME_KEY, applyTheme, readTheme, saveTheme } from '../src/shared/theme';

describe('thème', () => {
  afterEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
  });

  it('suit le système par défaut, ou si la valeur rangée est inconnue', () => {
    expect(readTheme()).toBe('system');
    localStorage.setItem(THEME_KEY, 'sepia');
    expect(readTheme()).toBe('system');
  });

  it('range le choix et pose data-theme', () => {
    saveTheme('light');
    expect(readTheme()).toBe('light');
    expect(document.documentElement.dataset['theme']).toBe('light');
    saveTheme('dark');
    expect(document.documentElement.dataset['theme']).toBe('dark');
    // Retour au système : plus rien de rangé ni d'attribut.
    saveTheme('system');
    expect(localStorage.getItem(THEME_KEY)).toBeNull();
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('applyTheme sans rien ranger', () => {
    applyTheme('dark');
    expect(document.documentElement.dataset['theme']).toBe('dark');
    expect(localStorage.getItem(THEME_KEY)).toBeNull();
  });
});
