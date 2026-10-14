import { describe, expect, it, vi } from 'vitest';
import { chordMatches, chordOfEvent, formatShortcut, isFieldTarget, normalizeChord, shortcutPlatform } from './keychord';

const event = (init: Partial<KeyboardEvent> & { key: string }): KeyboardEvent => ({ code: '', ...init }) as KeyboardEvent;

describe('normalizeChord', () => {
  const cases: Array<[string, string]> = [
    ['cmd+k', 'cmd+k'],
    ['K', 'k'],
    ['Meta+K', 'cmd+k'],
    ['Command+Shift+K', 'cmd+shift+k'],
    ['shift+cmd+k', 'cmd+shift+k'],
    ['ctrl+alt+cmd+shift+p', 'cmd+ctrl+alt+shift+p'],
    ['Option+ArrowLeft', 'alt+left'],
    ['option+ArrowRight', 'alt+right'],
    ['ArrowUp', 'up'],
    ['ArrowDown', 'down'],
    [' ', 'space'],
    ['Space', 'space'],
    ['Esc', 'escape'],
    ['Return', 'enter'],
    ['Del', 'delete'],
    ['shift+F9', 'shift+f9'],
    ['cmd+[', 'cmd+['],
    ['⌘⇧+k', 'k'],
    ['cmd++', 'cmd++'],
    ['+', '+'],
    ['', ''],
    ['   ', '']
  ];
  for (const [input, expected] of cases) {
    it(`normalises ${JSON.stringify(input)} → ${JSON.stringify(expected)}`, () => {
      expect(normalizeChord(input)).toBe(expected);
    });
  }

  it('is idempotent', () => {
    for (const [input] of cases) expect(normalizeChord(normalizeChord(input))).toBe(normalizeChord(input));
  });
});

describe('chordOfEvent', () => {
  it('reads letters, digits, and punctuation from event.key', () => {
    expect(chordOfEvent(event({ key: 'K', metaKey: true }))).toBe('cmd+k');
    expect(chordOfEvent(event({ key: '1' }))).toBe('1');
    expect(chordOfEvent(event({ key: '[', metaKey: true }))).toBe('cmd+[');
  });

  it('spells out multi-character key names', () => {
    expect(chordOfEvent(event({ key: 'ArrowRight', shiftKey: true }))).toBe('shift+right');
    expect(chordOfEvent(event({ key: ' ' }))).toBe('space');
    expect(chordOfEvent(event({ key: 'Escape' }))).toBe('escape');
    expect(chordOfEvent(event({ key: 'F9', metaKey: true }))).toBe('cmd+f9');
  });

  it('falls back to the physical digit when shift produces a symbol', () => {
    expect(chordOfEvent(event({ key: '!', code: 'Digit1', shiftKey: true }))).toBe('shift+1');
    // A letter under shift keeps its key, not its code.
    expect(chordOfEvent(event({ key: 'A', code: 'KeyA', shiftKey: true }))).toBe('shift+a');
  });

  it('falls back to physical brackets when shift produces braces', () => {
    expect(chordOfEvent(event({ key: '{', code: 'BracketLeft', metaKey: true, shiftKey: true }))).toBe('cmd+shift+[');
    expect(chordOfEvent(event({ key: '}', code: 'BracketRight', ctrlKey: true, shiftKey: true }))).toBe('ctrl+shift+]');
    // Unshifted bracket keys continue to use event.key directly.
    expect(chordOfEvent(event({ key: '[', code: 'BracketLeft', metaKey: true }))).toBe('cmd+[');
  });

  it('returns null for bare modifiers and empty keys', () => {
    expect(chordOfEvent(event({ key: 'Shift', shiftKey: true }))).toBeNull();
    expect(chordOfEvent(event({ key: 'Meta', metaKey: true }))).toBeNull();
    expect(chordOfEvent(event({ key: 'Control' }))).toBeNull();
    expect(chordOfEvent(event({ key: '' }))).toBeNull();
  });

  it('orders every modifier canonically', () => {
    expect(chordOfEvent(event({ key: 'p', shiftKey: true, altKey: true, ctrlKey: true, metaKey: true }))).toBe('cmd+ctrl+alt+shift+p');
  });
});

describe('isFieldTarget', () => {
  it('matches text-entry surfaces only', () => {
    expect(isFieldTarget({ tagName: 'INPUT' })).toBe(true);
    expect(isFieldTarget({ tagName: 'textarea' })).toBe(true);
    expect(isFieldTarget({ tagName: 'SELECT' })).toBe(false);
    expect(isFieldTarget({ tagName: 'DIV', isContentEditable: true })).toBe(true);
    expect(isFieldTarget({ tagName: 'DIV' })).toBe(false);
    expect(isFieldTarget(null)).toBe(false);
    expect(isFieldTarget(undefined)).toBe(false);
  });

  it('matches nested content inside an editable surface', () => {
    const editor = { tagName: 'DIV', isContentEditable: true };
    let editable = true;
    const child = { tagName: 'SPAN', closest: () => editable ? editor : null };

    expect(isFieldTarget(editor)).toBe(true);
    expect(isFieldTarget(child)).toBe(true);
    editable = false;
    expect(isFieldTarget(child)).toBe(false);
  });
});

describe('chordMatches', () => {
  it('maps primary command bindings to Ctrl only off macOS', () => {
    for (const platform of ['Win32', 'Linux x86_64']) {
      expect(chordMatches('cmd+shift+s', 'ctrl+shift+s', false, platform)).toBe(true);
      expect(chordMatches('cmd+s', 'ctrl+shift+s', false, platform)).toBe(false);
      expect(chordMatches('cmd+s', 'cmd+s', false, platform)).toBe(true);
    }
    expect(chordMatches('cmd+s', 'cmd+s', false, 'MacIntel')).toBe(true);
    expect(chordMatches('cmd+s', 'ctrl+s', false, 'MacIntel')).toBe(false);
    expect(chordMatches('ctrl+left', 'cmd+left', false, 'MacIntel')).toBe(false);
    expect(chordMatches('ctrl+left', 'ctrl+left', false, 'MacIntel')).toBe(true);
  });
  it('supports legacy subset matching and cmd/ctrl equivalence only when requested', () => {
    expect(chordMatches('cmd+k', 'cmd+ctrl+alt+k', true)).toBe(true);
    expect(chordMatches('cmd+k', 'ctrl+alt+k', true)).toBe(true);
    expect(chordMatches('cmd+k', 'cmd+alt+k')).toBe(false);
    expect(chordMatches('cmd+shift+k', 'cmd+k', true)).toBe(false);
  });
});

describe('platform shortcut labels', () => {
  it.each([
    ['⌘N', 'Ctrl+N'], ['⌘⇧S', 'Ctrl+Shift+S'], ['⇧⌘C', 'Ctrl+Shift+C'],
    ['⌥⌘K', 'Ctrl+Alt+K'], ['⌥Drag', 'Alt+Drag'], ['⌃⇧Tab', 'Ctrl+Shift+Tab'],
    ['⌥', 'Alt'], ['⇧', 'Shift'], ['⌘⌃Z', 'Ctrl+Z'],
    ['⌫', 'Backspace'], ['⌦', 'Delete'], ['↩', 'Enter'], ['⌘↵', 'Ctrl+Enter'], ['⎋', 'Esc'],
    ['⌃→ / ⇧K', 'Ctrl+→ / Shift+K'], ['⇧/', 'Shift+/'],
    ['cmd+shift+s', 'Ctrl+Shift+S'], ['alt+ArrowLeft', 'Alt+←'], ['cmd++', 'Ctrl++'],
    ['alt+drag', 'Alt+Drag'], ['Ctrl Alt Numpad 0', 'Ctrl+Alt+Numpad 0'], ['Shift B', 'Shift+B'], ['←↑→↓', '←↑→↓'], ['PageUp', 'PageUp'], ['', '']
  ])('formats %s on Windows and Linux', (input, expected) => {
    expect(formatShortcut(input, 'Win32')).toBe(expected);
    expect(formatShortcut(input, 'Linux')).toBe(expected);
  });

  it('preserves Mac glyph labels and explicit Control', () => {
    for (const label of ['⌘⇧S', '⌥⌘N', '⇧⌘C', '⌃→ / ⇧K', '⌫', '⌘↵']) {
      expect(formatShortcut(label, 'MacIntel')).toBe(label);
    }
    expect(formatShortcut('cmd+shift+s', 'macOS')).toBe('⌘⇧S');
    expect(formatShortcut('ctrl+left', 'macOS')).toBe('⌃←');
  });

  it('prefers client hints platform and falls back to navigator.platform', () => {
    try {
      vi.stubGlobal('navigator', { platform: 'MacIntel', userAgentData: { platform: 'Windows' } });
      expect(shortcutPlatform()).toBe('Windows');
      expect(formatShortcut('⌘S')).toBe('Ctrl+S');
      vi.stubGlobal('navigator', { platform: 'MacIntel' });
      expect(shortcutPlatform()).toBe('MacIntel');
      expect(formatShortcut('⌘S')).toBe('⌘S');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
