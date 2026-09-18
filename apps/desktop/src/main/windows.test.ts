import { describe, expect, it } from 'vitest';

import { EditorWindows, restorableProjects } from './windows';

function fakeWindow(name: string) {
  return {
    name,
    destroyed: false,
    minimized: false,
    shown: 0,
    focused: 0,
    isDestroyed() { return this.destroyed; },
    isMinimized() { return this.minimized; },
    restore() { this.minimized = false; },
    show() { this.shown++; },
    focus() { this.focused++; }
  };
}

describe('editor window registry', () => {
  it('keeps a document in one window and raises the window that has it', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const first = fakeWindow('first');
    const second = fakeWindow('second');
    editors.add(first, 'hero');
    editors.add(second);

    // The window holding the document is the one a second claim is sent to.
    expect(editors.claim(second, 'hero')).toBe(false);
    expect(editors.windowForProject('hero')).toBe(first);
    expect(editors.projectOf(second)).toBeNull();

    // Claiming something free succeeds, and re-claiming your own is a no-op.
    expect(editors.claim(second, 'titles')).toBe(true);
    expect(editors.claim(second, 'titles')).toBe(true);
    expect(editors.openProjectIds()).toEqual(['hero', 'titles']);

    // Letting a document go frees it for another window.
    expect(editors.claim(first, null)).toBe(true);
    expect(editors.claim(second, 'hero')).toBe(true);
    expect(editors.openProjectIds()).toEqual(['hero']);
  });

  it('restores the list in window order and forgets closed windows', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const first = fakeWindow('first');
    const second = fakeWindow('second');
    const third = fakeWindow('third');
    editors.add(first, 'a');
    editors.add(second, 'b');
    editors.add(third, 'c');
    expect(editors.openProjectIds()).toEqual(['a', 'b', 'c']);

    editors.remove(second);
    expect(editors.openProjectIds()).toEqual(['a', 'c']);
    expect(editors.size).toBe(2);

    // A destroyed window that never fired its 'closed' event still drops out.
    third.destroyed = true;
    expect(editors.openProjectIds()).toEqual(['a']);
  });

  it('answers with the window the user was last in', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const first = fakeWindow('first');
    const second = fakeWindow('second');
    editors.add(first, 'a');
    editors.add(second, 'b');
    expect(editors.mostRecent()).toBe(second);

    editors.touch(first);
    expect(editors.mostRecent()).toBe(first);

    // A window that is not ours never becomes the answer.
    const stray = fakeWindow('stray');
    editors.touch(stray);
    expect(editors.mostRecent()).toBe(first);

    first.destroyed = true;
    expect(editors.mostRecent()).toBe(second);
  });

  it('un-minimizes and fronts a window it reveals, and ignores a dead one', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const window = fakeWindow('one');
    window.minimized = true;
    editors.add(window, 'a');

    expect(editors.reveal(window)).toBe(true);
    expect(window.minimized).toBe(false);
    expect(window.shown).toBe(1);
    expect(window.focused).toBe(1);

    window.destroyed = true;
    expect(editors.reveal(window)).toBe(false);
    expect(editors.reveal(null)).toBe(false);
  });
});

describe('restorableProjects', () => {
  const known = (id: string) => id !== 'gone';

  it('prefers the window-era key and falls back to a pre-window profile', () => {
    expect(restorableProjects(['a', 'b'], ['x'], known)).toEqual(['a', 'b']);
    expect(restorableProjects([], ['x', 'y'], known)).toEqual(['x', 'y']);
    expect(restorableProjects(undefined, undefined, known)).toEqual([]);
  });

  it('never asks for one document twice, or for one that is no longer there', () => {
    expect(restorableProjects(['a', 'a', 'gone', null, 7, ''], null, known)).toEqual(['a']);
  });

  it('caps how many windows a launch may open', () => {
    const many = Array.from({ length: 40 }, (_, index) => `p${index}`);
    expect(restorableProjects(many, null, known)).toHaveLength(12);
    expect(restorableProjects(many, null, known, 3)).toEqual(['p0', 'p1', 'p2']);
  });
});

describe('window bookkeeping is not a source of surprise', () => {
  it('ignores a claim from a window it does not know', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const stray = fakeWindow('stray');
    expect(editors.claim(stray, 'a')).toBe(false);
    expect(editors.has(stray)).toBe(false);
    expect(editors.openProjectIds()).toEqual([]);
  });

  it('adds a window once, however many times it is announced', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const window = fakeWindow('one');
    editors.add(window, 'a');
    editors.add(window, 'b');
    expect(editors.size).toBe(1);
    expect(editors.projectOf(window)).toBe('a');
  });
});
