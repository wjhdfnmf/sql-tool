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
  reformatStyle: 'standard',
  reformatKeywords: 'upper',
  sliceThreads: 1,
  organizeConnections: true,
  autoHideOutput: false,
};

export interface Theme {
  value: string;
  label: string;
  note: string;
  group: 'Automatic' | 'Light' | 'Dark' | 'Bold';
}

// Every colour pair in every theme is checked for contrast (text, each
// syntax colour on the background, on a selection and on the current line,
// selections, pending edits), so the editor reads clearly in all of them.
export const themes: Theme[] = [
  { value: 'system', label: 'Follow the system', note: 'Light or dark, whichever the operating system is set to', group: 'Automatic' },
  { value: 'light', label: 'Light', note: 'White with blue', group: 'Light' },
  { value: 'mist', label: 'Mist', note: 'Soft cool greys and blue', group: 'Light' },
  { value: 'sage', label: 'Sage', note: 'Soft greens', group: 'Light' },
  { value: 'sepia', label: 'Sepia', note: 'Warm and low-glare', group: 'Light' },
  { value: 'rose', label: 'Rose', note: 'Soft pinks', group: 'Light' },
  { value: 'lavender', label: 'Lavender', note: 'Soft purples', group: 'Light' },
  { value: 'dark', label: 'Dark', note: 'Charcoal with blue', group: 'Dark' },
  { value: 'midnight', label: 'Midnight', note: 'Blue-black', group: 'Dark' },
  { value: 'graphite', label: 'Graphite', note: 'Charcoal and steel with teal', group: 'Dark' },
  { value: 'forest', label: 'Forest', note: 'Deep greens', group: 'Dark' },
  { value: 'orchid', label: 'Orchid', note: 'Plum with pink', group: 'Dark' },
  { value: 'ember', label: 'Ember', note: 'Near-black with orange', group: 'Bold' },
  { value: 'neon', label: 'Neon', note: 'Violet-black with vivid pink and cyan', group: 'Bold' },
  { value: 'cobalt', label: 'Cobalt', note: 'Deep blue with yellow', group: 'Bold' },
  { value: 'contrast', label: 'High contrast', note: 'Black with strong borders, for the most legibility', group: 'Bold' },
];

export const themeGroups: Theme['group'][] = ['Automatic', 'Light', 'Dark', 'Bold'];

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
