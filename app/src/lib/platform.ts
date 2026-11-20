// What differs between the systems the app runs on, as far as the interface
// needs to know.

/** True on macOS, where the modifier for shortcuts is Cmd and Alt is called Option. */
export const isMac =
  typeof navigator !== 'undefined' && /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent || '');

/**
 * Spells a shortcut the way this system does. Write `Mod` for the key that
 * is Ctrl on Windows and Linux and Cmd on a Mac; `Ctrl` stays Ctrl
 * everywhere, for the few shortcuts that use the Control key on a Mac too.
 */
export function keys(shortcut: string): string {
  return isMac ? shortcut.replace(/\bMod\b/g, 'Cmd').replace(/\bAlt\b/g, 'Option') : shortcut.replace(/\bMod\b/g, 'Ctrl');
}
