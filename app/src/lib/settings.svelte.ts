// The user's settings, as the UI uses them. They are stored by the backend
// (which also acts on some of them: timeouts, keep-alive, history, zoom);
// this module keeps the copy the UI reads and puts the theme into effect.

import { backend } from './backend';
import type { Settings } from './types';

export const defaultSettings: Settings = {
  theme: 'system',
  zoom: 100,
  editorTabSize: 2,
  editorWordWrap: false,
  uppercaseKeywords: true,
  rememberTabs: true,
  defaultRowLimit: 10_000,
  queryTimeoutSecs: 0,
  connectTimeoutSecs: 15,
  keepAliveSecs: 0,
  keepHistory: true,
  resultMemoryMb: 512,
};

export const themes: { value: string; label: string; note: string }[] = [
  { value: 'system', label: 'Follow the system', note: 'Light or dark, whichever the operating system is set to' },
  { value: 'light', label: 'Light', note: '' },
  { value: 'dark', label: 'Dark', note: '' },
  { value: 'midnight', label: 'Midnight', note: 'A darker, bluer dark theme' },
  { value: 'sepia', label: 'Sepia', note: 'A warm, low-glare light theme' },
  { value: 'contrast', label: 'High contrast', note: 'Black background, strong borders' },
];

/** Where the last theme is remembered, so the window opens in it instead of flashing the default first. */
const THEME_CACHE = 'sqltool.theme';

/** Told to everything that paints its own colours (the result grid) when the theme changes. */
export const APPEARANCE_EVENT = 'sqltool:appearance';

export function applyTheme(theme: string): void {
  const root = document.documentElement;
  if (theme === 'system' || !themes.some((t) => t.value === theme)) delete root.dataset.theme;
  else root.dataset.theme = theme;
  try {
    localStorage.setItem(THEME_CACHE, theme);
  } catch {
    // Remembering it is only to avoid a flash at start.
  }
  window.dispatchEvent(new Event(APPEARANCE_EVENT));
}

/** Called before the app is drawn, ahead of the settings arriving from the backend. */
export function applyRememberedTheme(): void {
  try {
    const theme = localStorage.getItem(THEME_CACHE);
    if (theme) applyTheme(theme);
  } catch {
    // Starts in the system theme.
  }
}

class SettingsState {
  current = $state<Settings>({ ...defaultSettings });

  async load(): Promise<void> {
    this.current = await backend.settingsGet();
    applyTheme(this.current.theme);
  }

  /** Saves the settings. What comes back is what was stored: out-of-range values are corrected. */
  async save(next: Settings): Promise<void> {
    this.current = await backend.settingsSet(next);
    applyTheme(this.current.theme);
  }
}

export const settings = new SettingsState();
