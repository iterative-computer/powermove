import { systemPreferences, type WebContents } from 'electron';

/** `--system-accent` from AppKit's "RRGGBBAA" accent colour; null when it isn't one. */
export function systemAccentCss(rgba: unknown): string | null {
  if (typeof rgba !== 'string' || !/^[0-9a-f]{6}([0-9a-f]{2})?$/i.test(rgba)) return null;
  return `:root{--system-accent:#${rgba.slice(0, 6).toUpperCase()}}`;
}

/**
 * Menus highlight their rows in the accent colour chosen in System Settings,
 * as AppKit's do. The colour is injected as a CSS variable on every load and
 * replaced when the person changes it.
 */
export function installSystemAccent(contents: WebContents): void {
  if (process.platform !== 'darwin') return;
  let inserted: string | undefined;
  const apply = async (): Promise<void> => {
    if (contents.isDestroyed()) return;
    const css = systemAccentCss(systemPreferences.getAccentColor());
    if (!css) return;
    const previous = inserted;
    inserted = await contents.insertCSS(css).catch(() => undefined);
    if (previous) await contents.removeInsertedCSS(previous).catch(() => undefined);
  };
  contents.on('did-finish-load', () => { inserted = undefined; void apply(); });
  const subscription = systemPreferences.subscribeNotification('AppleColorPreferencesChangedNotification', () => { void apply(); });
  contents.once('destroyed', () => systemPreferences.unsubscribeNotification(subscription));
}
