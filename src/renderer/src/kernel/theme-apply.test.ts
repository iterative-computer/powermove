// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { installKernel } from './install';
import { createKernel } from './registries';
import { installThemeApply, THEME_STYLE_ATTR, themeScheme, themeTokens } from './theme-apply';

const WIN98 = {
  id: 'win98',
  name: 'Windows 98',
  scheme: 'light' as const,
  tokens: { '--accent': '#000080', '--bg-window': '#c0c0c0' },
  css: '.panel { border-style: outset; }',
  rootAttributes: { 'data-win98': '' }
};

function root(): HTMLElement {
  return document.documentElement;
}

afterEach(() => {
  root().removeAttribute('style');
  root().removeAttribute('data-win98');
  root().removeAttribute('data-solar');
  document.querySelectorAll(`style[${THEME_STYLE_ATTR}]`).forEach((node) => node.remove());
  vi.unstubAllGlobals();
});

describe('theme token resolution', () => {
  it('layers darkTokens over tokens only for the dark scheme', () => {
    const def = { id: 't', name: 'T', scheme: 'auto' as const, tokens: { '--tx': '#111' }, darkTokens: { '--tx': '#eee' } };
    expect(themeTokens(def, 'light')).toEqual({ '--tx': '#111' });
    expect(themeTokens(def, 'dark')).toEqual({ '--tx': '#eee' });
    expect(themeTokens(undefined, 'dark')).toEqual({});
  });

  it('lets an explicit theme scheme win over the user preference', () => {
    expect(themeScheme(WIN98, 'dark')).toBe('light');
    expect(themeScheme({ id: 'a', name: 'A', scheme: 'auto' }, 'dark')).toBe('dark');
    expect(themeScheme({ id: 'a', name: 'A', scheme: 'auto' }, 'light')).toBe('light');
  });
});

describe('installThemeApply', () => {
  let kernel: ReturnType<typeof createKernel>;
  let handle: ReturnType<typeof installThemeApply>;

  beforeEach(() => {
    kernel = createKernel();
    handle = installThemeApply(kernel);
  });

  afterEach(() => handle.dispose());

  it('writes tokens, root attributes and css when a theme activates', () => {
    kernel.themes.register('ext', WIN98);
    kernel.activateTheme('win98');

    expect(root().style.getPropertyValue('--accent')).toBe('#000080');
    expect(root().getAttribute('data-win98')).toBe('');
    expect(document.querySelector(`style[${THEME_STYLE_ATTR}]`)?.textContent).toBe(WIN98.css);
  });

  it('removes the previous theme entirely when another activates', () => {
    kernel.themes.register('ext', WIN98);
    kernel.themes.register('ext', { id: 'solar', name: 'Solar', scheme: 'light', tokens: { '--tx': '#586e75' }, rootAttributes: { 'data-solar': 'on' } });
    kernel.activateTheme('win98');
    kernel.activateTheme('solar');

    expect(root().style.getPropertyValue('--accent')).toBe('');
    expect(root().hasAttribute('data-win98')).toBe(false);
    expect(root().style.getPropertyValue('--tx')).toBe('#586e75');
    expect(root().getAttribute('data-solar')).toBe('on');
    expect(document.querySelector(`style[${THEME_STYLE_ATTR}]`)).toBeNull();
  });

  it('repaints when the scheme preference changes', () => {
    kernel.themes.register('ext', { id: 'duo', name: 'Duo', scheme: 'auto', tokens: { '--tx': '#111' }, darkTokens: { '--tx': '#eee' } });
    kernel.activateTheme('duo');
    expect(root().style.getPropertyValue('--tx')).toBe('#111');

    kernel.setScheme('dark');
    expect(root().style.getPropertyValue('--tx')).toBe('#eee');
  });

  it('repaints when the active theme is re-registered (override)', () => {
    kernel.themes.register('ext', WIN98);
    kernel.activateTheme('win98');
    kernel.themes.register('ext:fork', { ...WIN98, tokens: { '--accent': '#ff6b1a' }, css: '' });

    expect(root().style.getPropertyValue('--accent')).toBe('#ff6b1a');
    expect(document.querySelector(`style[${THEME_STYLE_ATTR}]`)).toBeNull();
  });

  it('emits theme:changed when the active definition is replaced, restored, or removed', () => {
    const base = kernel.themes.register('ext', WIN98);
    kernel.activateTheme('win98');
    const changed = vi.fn();
    const subscription = kernel.events.on('theme:changed', changed);

    const override = kernel.themes.register('ext:fork', { ...WIN98, tokens: { '--accent': '#ff6b1a' } });
    override.dispose();
    base.dispose();

    expect(changed).toHaveBeenCalledTimes(3);
    expect(changed.mock.calls.map(([event]) => event)).toEqual([
      { id: 'win98', scheme: 'light' },
      { id: 'win98', scheme: 'light' },
      { id: 'win98', scheme: 'light' }
    ]);
    subscription.dispose();
  });

  it('repaints automatic darkTokens on system appearance changes and removes its listener on dispose', () => {
    let dark = false;
    const listeners = new Set<() => void>();
    const media = {
      get matches() { return dark; },
      addEventListener: vi.fn((_event: string, listener: () => void) => listeners.add(listener)),
      removeEventListener: vi.fn((_event: string, listener: () => void) => listeners.delete(listener))
    };
    vi.stubGlobal('matchMedia', vi.fn(() => media));
    handle.dispose();
    handle = installThemeApply(kernel);
    kernel.themes.register('ext', {
      id: 'automatic',
      name: 'Automatic',
      scheme: 'auto',
      tokens: { '--probe': 'light' },
      darkTokens: { '--probe': 'dark' }
    });
    kernel.activateTheme('automatic');

    expect(root().style.getPropertyValue('--probe')).toBe('light');
    expect(listeners.size).toBe(1);
    dark = true;
    for (const listener of [...listeners]) listener();
    expect(root().style.getPropertyValue('--probe')).toBe('dark');

    handle.dispose();
    expect(listeners.size).toBe(0);
    expect(media.removeEventListener).toHaveBeenCalledWith('change', expect.any(Function));
  });

  it('drops everything it applied on dispose', () => {
    kernel.themes.register('ext', WIN98);
    kernel.activateTheme('win98');
    handle.dispose();

    expect(root().style.getPropertyValue('--accent')).toBe('');
    expect(root().hasAttribute('data-win98')).toBe(false);
    expect(document.querySelector(`style[${THEME_STYLE_ATTR}]`)).toBeNull();
  });

  it('ignores tokens that are not custom properties', () => {
    kernel.themes.register('ext', { id: 'bad', name: 'Bad', scheme: 'light', tokens: { color: 'red' } as Record<string, string> });
    kernel.activateTheme('bad');

    expect(root().style.getPropertyValue('color')).toBe('');
  });
});

describe('kernel theme installed against PM', () => {
  function makePM(stored: Record<string, unknown> = {}) {
    const store = { ...stored };
    return {
      store: { get: (key: string, fallback: unknown) => (key in store ? store[key] : fallback), set: (key: string, value: unknown) => void (store[key] = value) },
      bus: { on: () => () => {}, emit() {} },
      raw: store
    } as any;
  }

  it('leaves the default definition to the built-in extension', () => {
    const PM = makePM();
    const kernel = installKernel(PM);

    expect(kernel.theme.activeId).toBe('default');
    expect(kernel.themes.get('default')).toBeUndefined();
    kernel.uninstall();
  });

  it('persists the active theme id and restores it', () => {
    const PM = makePM();
    const kernel = installKernel(PM);
    kernel.themes.register('ext', WIN98);

    kernel.activateTheme('win98');
    expect(PM.raw.activeTheme).toBe('win98');

    kernel.activateTheme('default');
    kernel.restoreTheme();
    expect(kernel.theme.activeId).toBe('default');

    PM.raw.activeTheme = 'win98';
    kernel.restoreTheme();
    expect(kernel.theme.activeId).toBe('win98');
    kernel.uninstall();
  });

  it('falls back to default when the persisted theme is not registered', () => {
    const PM = makePM({ activeTheme: 'gone' });
    const kernel = installKernel(PM);

    kernel.restoreTheme();

    expect(kernel.theme.activeId).toBe('default');
    kernel.uninstall();
  });

  it('adopts the stored appearance preference at install time', () => {
    const PM = makePM({ themeMode: 'dark' });
    const kernel = installKernel(PM);

    expect(kernel.theme.scheme).toBe('dark');
    kernel.uninstall();
  });

  it('routes setScheme through the legacy PM.theme so persistence stays single-writer', () => {
    const PM = makePM();
    const kernel = installKernel(PM);
    /* The real PM.theme.apply (legacy/app.ts) persists, writes the scheme back
       into the kernel and emits — so setScheme must delegate, not duplicate. */
    const apply = vi.fn((mode: string) => {
      PM.raw.themeMode = mode;
      kernel.theme.scheme = mode as 'light';
      kernel.events.emit('theme:changed', { id: kernel.theme.activeId, scheme: 'light' });
    });
    PM.theme = { apply };

    kernel.setScheme('light');

    expect(apply).toHaveBeenCalledWith('light');
    expect(kernel.theme.scheme).toBe('light');
    expect(PM.raw.themeMode).toBe('light');
    expect(apply).toHaveBeenCalledOnce();
    kernel.uninstall();
  });
});
