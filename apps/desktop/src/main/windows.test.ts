import { describe, expect, it } from 'vitest';

import { EditorWindows, restorableSessions, TAB_STRIP_BAND, windowUnderTabDrop } from './windows';

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

    // Showing nothing keeps the tab; only closing it frees the document.
    expect(editors.claim(first, null)).toBe(true);
    expect(editors.claim(second, 'hero')).toBe(false);
    expect(editors.release(first, 'hero')).toBe(true);
    expect(editors.claim(second, 'hero')).toBe(true);
    expect(editors.openProjectIds()).toEqual(['titles', 'hero']);
  });

  it('keeps a window\'s tabs in strip order and shows one of them', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const window = fakeWindow('one');
    editors.add(window, 'b', ['a', 'b', 'a', '']);
    expect(editors.tabsOf(window)).toEqual(['a', 'b']);
    expect(editors.projectOf(window)).toBe('b');

    // A new document is appended; switching to an existing tab leaves the strip alone.
    editors.claim(window, 'c');
    editors.claim(window, 'a');
    expect(editors.tabsOf(window)).toEqual(['a', 'b', 'c']);
    expect(editors.projectOf(window)).toBe('a');

    // Closing the tab on screen leaves the window showing nothing until it picks.
    expect(editors.release(window, 'a')).toBe(true);
    expect(editors.release(window, 'a')).toBe(false);
    expect(editors.projectOf(window)).toBeNull();
    expect(editors.tabsOf(window)).toEqual(['b', 'c']);
  });

  it('reorders only the tabs a window has', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const window = fakeWindow('one');
    editors.add(window, 'a', ['a', 'b', 'c']);
    editors.reorder(window, ['c', 'a', 'stranger', 'c']);
    // Unknown ids are ignored and a tab left out keeps its place at the end.
    expect(editors.tabsOf(window)).toEqual(['c', 'a', 'b']);
  });

  it('remembers every window\'s tabs for the next launch', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>();
    const first = fakeWindow('first');
    const empty = fakeWindow('empty');
    const second = fakeWindow('second');
    editors.add(first, 'b', ['a', 'b']);
    editors.add(empty);
    editors.add(second, 'c');
    expect(editors.openSessions()).toEqual([
      { tabs: ['a', 'b'], active: 'b' },
      { tabs: ['c'], active: 'c' }
    ]);
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

  it('never shows or focuses a window when quiet, as in background tests', () => {
    const editors = new EditorWindows<ReturnType<typeof fakeWindow>>({ quiet: true });
    const window = fakeWindow('hidden');
    window.minimized = true;
    editors.add(window, 'a');
    expect(editors.reveal(window)).toBe(true);
    expect([window.minimized, window.shown, window.focused]).toEqual([true, 0, 0]);
  });
});

describe('restorableSessions', () => {
  const known = (id: string) => id !== 'gone';

  it('puts each window back with its own tabs', () => {
    expect(restorableSessions([{ tabs: ['a', 'b'], active: 'a' }, { tabs: ['c'], active: 'c' }], ['x'], ['y'], known)).toEqual([
      { tabs: ['a', 'b'], active: 'a' },
      { tabs: ['c'], active: 'c' }
    ]);
  });

  it('falls back to one window per project, then to a single-window tab strip', () => {
    expect(restorableSessions(undefined, ['a', 'b'], ['x'], known)).toEqual([
      { tabs: ['a'], active: 'a' },
      { tabs: ['b'], active: 'b' }
    ]);
    expect(restorableSessions([], [], ['x', 'y'], known)).toEqual([{ tabs: ['x', 'y'], active: 'y' }]);
    expect(restorableSessions(undefined, undefined, undefined, known)).toEqual([]);
  });

  it('never asks for one document twice, or for one that is no longer there', () => {
    expect(restorableSessions([
      { tabs: ['a', 'a', 'gone', null, 7, ''], active: 'gone' },
      { tabs: ['a'], active: 'a' },
      'junk'
    ], null, null, known)).toEqual([{ tabs: ['a'], active: 'a' }]);
    expect(restorableSessions(null, ['a', 'a', 'gone', null, 7, ''], null, known)).toEqual([{ tabs: ['a'], active: 'a' }]);
  });

  it('caps how many windows and tabs a launch may open', () => {
    const many = Array.from({ length: 40 }, (_, index) => `p${index}`);
    expect(restorableSessions(null, many, null, known)).toHaveLength(12);
    expect(restorableSessions(null, many, null, known, { windows: 3 }).map(session => session.active)).toEqual(['p0', 'p1', 'p2']);
    expect(restorableSessions([{ tabs: many, active: 'p39' }], null, null, known, { tabs: 5 })).toEqual([
      { tabs: ['p0', 'p1', 'p2', 'p3', 'p4'], active: 'p4' }
    ]);
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

describe('windowUnderTabDrop', () => {
  const bounds: Record<string, { x: number; y: number; width: number; height: number } | null> = {
    from: { x: 0, y: 0, width: 800, height: 600 },
    front: { x: 600, y: 100, width: 800, height: 600 },
    back: { x: 500, y: 50, width: 800, height: 600 },
    minimized: null
  };
  const drop = (order: string[], x: number, y: number) =>
    windowUnderTabDrop(order, 'from', { x, y }, (window) => bounds[window] ?? null);

  it('lands on the strip of the window under the point', () => {
    expect(drop(['front', 'from'], 700, 110)).toBe('front');
    expect(drop(['front', 'from'], 700, 100 + TAB_STRIP_BAND + 1)).toBeNull();
    // Where the source window is in front, the drop stays with it.
    expect(drop(['from', 'front'], 700, 110)).toBeNull();
  });

  it('lets the front-most window take the drop, and its body block the strip behind', () => {
    // The point is on both strips' height, but the front window is on top.
    expect(drop(['front', 'back'], 650, 105)).toBe('front');
    // Over the front window's body: the back window's strip there is hidden.
    expect(drop(['front', 'back'], 700, 300)).toBeNull();
    expect(drop(['back', 'front'], 700, 60)).toBe('back');
  });

  it('never drops onto the window the tab came from, or one that is minimized', () => {
    expect(drop(['from', 'front'], 100, 10)).toBeNull();
    expect(drop(['minimized'], 10, 10)).toBeNull();
    expect(drop(['front'], 5000, 5000)).toBeNull();
  });
});
