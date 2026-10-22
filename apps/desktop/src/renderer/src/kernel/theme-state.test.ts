import { describe, expect, it } from 'vitest';
import { createKernel } from './registries';
import { ownThemeState, reconcileThemeState, type ThemeStateBackend } from './theme-state';

function harness() {
  const kernel = createKernel();
  for (const id of ['default', 'previous', 'first', 'second']) kernel.themes.register(id, { id, name: id, scheme: 'auto' });
  kernel.activateTheme('previous'); kernel.setScheme('light');
  return kernel;
}

describe('owned theme state', () => {
  it.each([true, false])('restores scheme and selection after restart (enabled: %s)', enabled => {
    let saved: unknown;
    const backend: ThemeStateBackend = { load: () => saved, save: value => { saved = JSON.parse(JSON.stringify(value)); } };
    const original = harness(), mod = ownThemeState(original, 'mod', backend);
    mod.activate('first'); mod.setScheme('dark');
    const restarted = harness();
    restarted.activateTheme('default'); restarted.setScheme('dark');
    reconcileThemeState(restarted, new Set(enabled ? ['mod'] : []), backend);
    if (enabled) {
      const next = ownThemeState(restarted, 'mod', backend);
      next.activate('first'); next.setScheme('dark'); next.dispose();
    }
    expect(restarted.theme).toEqual({ activeId: 'previous', scheme: 'light' });
  });

  it('restores a prior theme that registers later during boot', () => {
    let saved: unknown;
    const backend: ThemeStateBackend = { load: () => saved, save: value => { saved = JSON.parse(JSON.stringify(value)); } };
    const original = harness(); ownThemeState(original, 'mod', backend).activate('first');
    const restarted = createKernel();
    reconcileThemeState(restarted, new Set(), backend);
    expect(restarted.theme.activeId).toBe('default');
    restarted.themes.register('prior-owner', { id: 'previous', name: 'Previous', scheme: 'auto' });
    expect(restarted.theme.activeId).toBe('previous');
  });

  it('restores prior theme selection and scheme and ignores calls after unload', () => {
    const kernel = harness(), mod = ownThemeState(kernel, 'mod');
    mod.activate('first'); mod.setScheme('dark');
    mod.dispose(); mod.activate('first'); mod.setScheme('dark');
    expect(kernel.theme).toEqual({ activeId: 'previous', scheme: 'light' });
  });

  it.each([true, false])('unwinds overlapping theme selections in either order (%s)', topFirst => {
    const kernel = harness(), first = ownThemeState(kernel, 'first'), second = ownThemeState(kernel, 'second');
    first.activate('first'); first.setScheme('dark');
    second.activate('second'); second.setScheme('system');
    if (topFirst) { second.dispose(); expect(kernel.theme.activeId).toBe('first'); first.dispose(); }
    else { first.dispose(); expect(kernel.theme.activeId).toBe('second'); second.dispose(); }
    expect(kernel.theme).toEqual({ activeId: 'previous', scheme: 'light' });
  });

  it('preserves later user choices', () => {
    const kernel = harness(), mod = ownThemeState(kernel, 'mod');
    mod.activate('first'); mod.setScheme('dark');
    kernel.activateTheme('second'); kernel.setScheme('system');
    mod.dispose();
    expect(kernel.theme).toEqual({ activeId: 'second', scheme: 'system' });
  });

  it('falls back to the default if the prior theme has been removed', () => {
    const kernel = harness(), mod = ownThemeState(kernel, 'mod');
    mod.activate('first'); kernel.themes.disposeOwner('previous'); mod.dispose();
    expect(kernel.theme.activeId).toBe('default');
  });
});
