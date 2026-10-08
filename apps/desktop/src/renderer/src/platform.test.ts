// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { isWindows, shortcutLabel } from './platform';
import { chordMatches } from './kernel/keychord';
afterEach(() => vi.unstubAllGlobals());
describe('Windows keyboard controls', () => {
  it('renders familiar modifier labels without changing Mac labels', () => {
    expect(shortcutLabel('⌘⇧K', 'Win32')).toBe('Ctrl+Shift+K');
    expect(shortcutLabel('Compile ⌘↵', 'Win32')).toBe('Compile Ctrl+Enter');
    expect(shortcutLabel('⌥⌘N', 'MacIntel')).toBe('⌥⌘N');
    expect(isWindows('Win32')).toBe(true);
  });
  it('lets Windows users run extension Command shortcuts with Control, keeping exact modifiers', () => {
    vi.stubGlobal('navigator', { platform: 'Win32' });
    expect(chordMatches('cmd+shift+k', 'ctrl+shift+k')).toBe(true);
    expect(chordMatches('cmd+shift+k', 'ctrl+k')).toBe(false);
    expect(chordMatches('cmd+k', 'ctrl+alt+k')).toBe(false);
    expect(chordMatches('cmd+k', 'ctrl+alt+k', true)).toBe(true);
    expect(chordMatches('cmd+k', 'cmd+k')).toBe(false);
  });
});
