/*
 * Applying the active kernel theme to the document.
 *
 * A theme is three things, applied together and removed together:
 *   1. `rootAttributes` on <html> (`data-win98=""` and friends)
 *   2. `tokens` (+ `darkTokens` when the resolved scheme is dark) written as
 *      inline custom properties on <html>, so they beat the stylesheet cascade
 *      but still lose to anything set later with higher specificity — which is
 *      what lets the workspace theme in layout/install.ts layer on top.
 *   3. `css`, injected as ONE <style data-powermove-theme> element.
 *
 * Everything the previous theme set is removed first, tracked by key, so
 * switching themes can never leave a token behind.
 */
import type { Disposable, ThemeDefinition } from './api';
import type { Kernel, SchemePreference } from './registries';

export const THEME_STYLE_ATTR = 'data-powermove-theme';

export interface ThemeApplyHandle extends Disposable {
  /** Re-read the active theme and reapply it. */
  apply(): void;
  /** Tokens currently written by the kernel theme (test seam). */
  appliedTokens(): Record<string, string>;
}

export function resolveScheme(mode: SchemePreference): 'light' | 'dark' {
  if (mode === 'light' || mode === 'dark') return mode;
  const query = typeof globalThis.matchMedia === 'function' ? globalThis.matchMedia('(prefers-color-scheme: dark)') : null;
  return query?.matches ? 'dark' : 'light';
}

/** Tokens a theme contributes for a resolved scheme (dark overrides layered last). */
export function themeTokens(def: ThemeDefinition | undefined, scheme: 'light' | 'dark'): Record<string, string> {
  if (!def) return {};
  const base = { ...(def.tokens ?? {}) };
  if (scheme === 'dark') Object.assign(base, def.darkTokens ?? {});
  return base;
}

/** The scheme a theme resolves to given the kernel's user preference. */
export function themeScheme(def: ThemeDefinition | undefined, preference: SchemePreference): 'light' | 'dark' {
  if (def?.scheme === 'light' || def?.scheme === 'dark') return def.scheme;
  return resolveScheme(preference);
}

/** Identity used when the persisted theme is absent; its definition is a built-in extension. */
export const DEFAULT_THEME: ThemeDefinition = { id: 'default', name: 'Default', scheme: 'auto' };

export function installThemeApply(kernel: Kernel): ThemeApplyHandle {
  const root = typeof document === 'undefined' ? null : document.documentElement;
  let tokenKeys: string[] = [];
  let attributeKeys: string[] = [];
  let styleElement: HTMLStyleElement | null = null;
  let applied: Record<string, string> = {};
  let media: MediaQueryList | null = null;
  let listeningToMedia = false;
  let broadcasting = false;
  let resolved: 'light' | 'dark' = 'light';

  const onMediaChange = (): void => repaintAndNotify();

  const setMediaListening = (enabled: boolean): void => {
    if (enabled && !media && typeof globalThis.matchMedia === 'function') {
      media = globalThis.matchMedia('(prefers-color-scheme: dark)');
    }
    if (enabled === listeningToMedia || !media) return;
    if (enabled) {
      media.addEventListener?.('change', onMediaChange);
      if (!media.addEventListener) media.addListener?.(onMediaChange);
    } else {
      media.removeEventListener?.('change', onMediaChange);
      if (!media.removeEventListener) media.removeListener?.(onMediaChange);
    }
    listeningToMedia = enabled;
  };

  const clear = (): void => {
    if (!root) return;
    for (const key of tokenKeys) root.style.removeProperty(key);
    for (const key of attributeKeys) root.removeAttribute(key);
    tokenKeys = [];
    attributeKeys = [];
  };

  const repaint = (): void => {
    const def = kernel.themes.get(kernel.theme.activeId);
    const followsSystem = def?.scheme !== 'light' && def?.scheme !== 'dark' && kernel.theme.scheme === 'system';
    setMediaListening(followsSystem);
    resolved = def?.scheme === 'light' || def?.scheme === 'dark'
      ? def.scheme
      : kernel.theme.scheme === 'light' || kernel.theme.scheme === 'dark'
        ? kernel.theme.scheme
        : media?.matches ? 'dark' : 'light';
    const tokens = themeTokens(def, resolved);
    applied = tokens;
    if (!root) return;
    clear();
    for (const [key, value] of Object.entries(tokens)) {
      if (!key.startsWith('--')) continue;
      root.style.setProperty(key, String(value));
      tokenKeys.push(key);
    }
    for (const [key, value] of Object.entries(def?.rootAttributes ?? {})) {
      root.setAttribute(key, String(value));
      attributeKeys.push(key);
    }
    const css = typeof def?.css === 'string' ? def.css : '';
    if (css) {
      if (!styleElement || !styleElement.isConnected) {
        styleElement = document.querySelector<HTMLStyleElement>(`style[${THEME_STYLE_ATTR}]`);
        if (!styleElement) {
          styleElement = document.createElement('style');
          styleElement.setAttribute(THEME_STYLE_ATTR, '');
          document.head.appendChild(styleElement);
        }
      }
      styleElement.textContent = css;
    } else if (styleElement) {
      styleElement.remove();
      styleElement = null;
    }
  };

  function repaintAndNotify(): void {
    repaint();
    broadcasting = true;
    try {
      kernel.events.emit('theme:changed', { id: kernel.theme.activeId, scheme: resolved });
    } finally {
      broadcasting = false;
    }
  }

  const subs: Disposable[] = [
    kernel.events.on('theme:changed', () => {
      if (!broadcasting) repaint();
    }),
    /* Re-registering the active id (an override, or the extension that owns it
       reloading) must repaint without a second activate() call. */
    kernel.themes.onChange((change) => {
      if (change.id === kernel.theme.activeId) repaintAndNotify();
    })
  ];

  repaint();

  return {
    apply: repaintAndNotify,
    appliedTokens: () => ({ ...applied }),
    dispose: () => {
      for (const sub of subs) sub.dispose();
      setMediaListening(false);
      clear();
      styleElement?.remove();
      styleElement = null;
    }
  };
}
