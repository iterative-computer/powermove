// @vitest-environment happy-dom
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';

import type { ExtensionRecord, MenuContribution, PowermoveAPI } from 'powermove';

import ModsPanel from './ModsPanel.svelte';

type Health = ExtensionRecord['health'];

function record(id: string, over: Partial<ExtensionRecord> = {}): ExtensionRecord {
  return {
    id,
    scope: 'user',
    manifest: { id, name: id, version: '1.0.0', apiVersion: 1, description: `${id} description` },
    dir: `/tmp/${id}`,
    enabled: true,
    bundleUrl: null,
    bundleHash: null,
    health: { state: 'ok' } as Health,
    updatedAt: 0,
    ...over
  };
}

function fakeApi(initial: ExtensionRecord[]) {
  let list = [...initial];
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const menus: Array<{ anchor: unknown; items: MenuContribution[] }> = [];

  const calls = {
    setEnabled: vi.fn(async (_id: string, _enabled: boolean) => {}),
    remove: vi.fn(async (_id: string) => {}),
    reload: vi.fn(async (_id: string) => {}),
    reveal: vi.fn(async (_id: string) => {}),
    requestFix: vi.fn((_id: string) => {}),
    toast: vi.fn((_text: string) => {}),
    confirm: vi.fn(async (_title: string, _body?: string) => true),
    list: vi.fn(() => list.map((entry) => ({ ...entry })))
  };

  const api = {
    id: 'mods',
    apiVersion: 1,
    extensions: {
      list: calls.list,
      setEnabled: calls.setEnabled,
      remove: calls.remove,
      reload: calls.reload,
      reveal: calls.reveal,
      requestFix: calls.requestFix
    },
    events: {
      on(event: string, fn: (payload: unknown) => void) {
        let set = listeners.get(event);
        if (!set) listeners.set(event, (set = new Set()));
        set.add(fn);
        return { dispose: () => set!.delete(fn) };
      },
      emit(event: string, payload: unknown) {
        for (const fn of [...(listeners.get(event) ?? [])]) fn(payload);
      }
    },
    ui: {
      toast: calls.toast,
      confirm: calls.confirm,
      menu: (anchor: unknown, items: MenuContribution[]) => {
        menus.push({ anchor, items });
      }
    },
    log: vi.fn()
  };

  return {
    api: api as unknown as PowermoveAPI,
    calls,
    menus,
    emit: (event: string) => api.events.emit(event, { id: 'x' }),
    listenerCount: (event: string) => listeners.get(event)?.size ?? 0,
    setList: (next: ExtensionRecord[]) => {
      list = next;
    }
  };
}

function rows(target: HTMLElement): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>('[role="listitem"]')];
}

function rowFor(target: HTMLElement, name: string): HTMLElement {
  const row = rows(target).find((candidate) => candidate.getAttribute('aria-label') === name);
  if (!row) throw new Error(`Missing row: ${name}`);
  return row;
}

function buttonWithText(scope: HTMLElement, text: string): HTMLButtonElement {
  const button = [...scope.querySelectorAll<HTMLButtonElement>('button')].find(
    (candidate) => candidate.textContent?.trim() === text
  );
  if (!button) throw new Error(`Missing button: ${text}`);
  return button;
}

function labels(items: MenuContribution[]): string[] {
  return items.map((item) => (typeof item === 'string' ? item : 'label' in item ? item.label : ''));
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}

describe('ModsPanel', () => {
  let target: HTMLDivElement;
  let instance: ReturnType<typeof mount> | undefined;

  function render(harness: ReturnType<typeof fakeApi>): HTMLDivElement {
    instance = mount(ModsPanel, { target, props: { api: harness.api } });
    flushSync();
    return target;
  }

  beforeEach(() => {
    target = document.createElement('div');
    document.body.appendChild(target);
  });

  afterEach(() => {
    if (instance) unmount(instance);
    instance = undefined;
    target.remove();
  });

  it('groups what you added before what shipped, and describes each mod', () => {
    const harness = fakeApi([
      record('builtin-one', { scope: 'builtin', manifest: { id: 'builtin-one', name: 'Timeline', version: '1.0.0', apiVersion: 1 } }),
      record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1, description: 'Adds a bounce.' } })
    ]);
    render(harness);

    const headings = [...target.querySelectorAll('h2')].map((h) => h.textContent);
    expect(headings).toEqual(['Added by you', 'Built in']);
    expect(rows(target).map((row) => row.getAttribute('aria-label'))).toEqual(['Bounce', 'Timeline']);
    expect(rowFor(target, 'Bounce').textContent).toContain('Adds a bounce.');
    // Missing description falls back to the id rather than an empty line.
    expect(rowFor(target, 'Timeline').textContent).toContain('builtin-one');
  });

  it('counts the mods and how many need attention', () => {
    const harness = fakeApi([
      record('a'),
      record('b', { health: { state: 'build-error', error: 'boom' } }),
      record('c', { scope: 'builtin', enabled: false, health: { state: 'disabled' } })
    ]);
    render(harness);
    expect(target.querySelector('.summary')?.textContent).toBe('3 mods · 1 needs attention');
  });

  it('invites the user to ask when nothing has been added', () => {
    const harness = fakeApi([record('builtin-one', { scope: 'builtin' })]);
    render(harness);
    expect(target.querySelector('.empty')?.textContent).toBe(
      'Ask the agent to change anything about Powermove — it shows up here.'
    );
  });

  it('turns a mod off from its switch and re-reads the list', async () => {
    const harness = fakeApi([record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } })]);
    render(harness);
    harness.calls.list.mockClear();

    const toggle = rowFor(target, 'Bounce').querySelector<HTMLElement>('.onoff');
    expect(toggle?.getAttribute('data-enabled')).toBe('true');
    toggle?.querySelector<HTMLButtonElement>('button[aria-label^="Turn off"]')?.click();
    flushSync();

    expect(harness.calls.setEnabled).toHaveBeenCalledWith('mine', false);
    // Optimistic: the switch flips before the promise settles.
    expect(rowFor(target, 'Bounce').querySelector('.onoff')?.getAttribute('data-enabled')).toBe('false');
    await settle();
    expect(harness.calls.list).toHaveBeenCalled();
  });

  it('toggles from the keyboard with Space', () => {
    const harness = fakeApi([record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } })]);
    render(harness);

    const row = rowFor(target, 'Bounce');
    expect(row.getAttribute('tabindex')).toBe('0');
    row.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));
    flushSync();
    expect(harness.calls.setEnabled).toHaveBeenCalledWith('mine', false);
  });

  it('shows a one-line error and Turn off without a manual Fix it button', () => {
    const long = `${'x'.repeat(200)}\nsecond line`;
    const harness = fakeApi([
      record('broken', {
        manifest: { id: 'broken', name: 'Broken', version: '1.0.0', apiVersion: 1 },
        health: { state: 'activation-error', error: long }
      })
    ]);
    render(harness);

    const row = rowFor(target, 'Broken');
    const error = row.querySelector<HTMLElement>('.error');
    expect(error?.textContent?.length).toBeLessThanOrEqual(120);
    expect(error?.textContent).not.toContain('second line');
    expect(error?.getAttribute('title')).toBe(long);

    expect(row.textContent).not.toContain('Fix it');
    expect(harness.calls.requestFix).not.toHaveBeenCalled();

    buttonWithText(row, 'Turn off').click();
    flushSync();
    expect(harness.calls.setEnabled).toHaveBeenCalledWith('broken', false);
  });

  it('notes when a mod replaces or was forked from a built-in, and when it was replaced', () => {
    const harness = fakeApi([
      record('fork', {
        manifest: {
          id: 'fork',
          name: 'My timeline',
          version: '2.0.0',
          apiVersion: 1,
          replaces: ['timeline'],
          forkedFrom: 'timeline@1.4.0'
        }
      }),
      record('timeline', {
        scope: 'builtin',
        manifest: { id: 'timeline', name: 'Timeline', version: '1.4.0', apiVersion: 1 },
        health: { state: 'replaced', by: 'fork' }
      })
    ]);
    render(harness);

    const notes = [...rowFor(target, 'My timeline').querySelectorAll('.note')].map((n) => n.textContent);
    expect(notes).toEqual(['Replaces built-in timeline', 'Forked from timeline v1.4.0']);
    expect(rowFor(target, 'Timeline').querySelector('.note')?.textContent).toBe('Replaced by fork');
    // "replaced" is a settled state, not a problem to fix.
    expect(target.querySelector('.summary')?.textContent).toBe('2 mods');
  });

  it('offers remove and reveal only for mods you added', () => {
    const harness = fakeApi([
      record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } }),
      record('timeline', { scope: 'builtin', manifest: { id: 'timeline', name: 'Timeline', version: '1.0.0', apiVersion: 1 } })
    ]);
    render(harness);

    rowFor(target, 'Bounce').querySelector<HTMLButtonElement>('.more')?.click();
    rowFor(target, 'Timeline').querySelector<HTMLButtonElement>('.more')?.click();
    flushSync();

    expect(labels(harness.menus[0]!.items)).toEqual(['Reload', 'Show in Finder', '-', 'Remove…']);
    expect(labels(harness.menus[1]!.items)).toEqual(['Reload']);
    expect(harness.menus[0]!.anchor).toBe(rowFor(target, 'Bounce').querySelector('.more'));
  });

  it('removes only after the user confirms', async () => {
    const harness = fakeApi([record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } })]);
    render(harness);
    rowFor(target, 'Bounce').querySelector<HTMLButtonElement>('.more')?.click();
    flushSync();

    const remove = harness.menus[0]!.items.find((item) => typeof item !== 'string' && 'label' in item && item.label === 'Remove…');
    if (typeof remove === 'string' || !remove || !('run' in remove)) throw new Error('Missing remove item');

    harness.calls.confirm.mockResolvedValueOnce(false);
    remove.run?.();
    await settle();
    expect(harness.calls.remove).not.toHaveBeenCalled();

    remove.run?.();
    await settle();
    expect(harness.calls.confirm).toHaveBeenCalledTimes(2);
    expect(harness.calls.remove).toHaveBeenCalledWith('mine');
  });

  it('reloads through the overflow menu and reports failure as a toast', async () => {
    const harness = fakeApi([record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } })]);
    render(harness);
    rowFor(target, 'Bounce').querySelector<HTMLButtonElement>('.more')?.click();
    flushSync();

    harness.calls.reload.mockRejectedValueOnce(new Error('no such folder'));
    const reload = harness.menus[0]!.items.find((item) => typeof item !== 'string' && 'label' in item && item.label === 'Reload');
    if (typeof reload === 'string' || !reload || !('run' in reload)) throw new Error('Missing reload item');
    reload.run?.();
    await settle();

    expect(harness.calls.reload).toHaveBeenCalledWith('mine');
    expect(harness.calls.toast).toHaveBeenCalledWith('Could not reload Bounce. no such folder');
  });

  it('re-reads the list when mods load or unload, and stops listening when closed', () => {
    const harness = fakeApi([record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } })]);
    render(harness);
    expect(harness.listenerCount('extension:loaded')).toBe(1);

    harness.setList([
      record('mine', { manifest: { id: 'mine', name: 'Bounce', version: '1.0.0', apiVersion: 1 } }),
      record('later', { manifest: { id: 'later', name: 'Later', version: '1.0.0', apiVersion: 1 } })
    ]);
    harness.emit('extension:loaded');
    flushSync();
    expect(rows(target).map((row) => row.getAttribute('aria-label'))).toEqual(['Bounce', 'Later']);

    unmount(instance!);
    instance = undefined;
    flushSync();
    expect(harness.listenerCount('extension:loaded')).toBe(0);
    expect(harness.listenerCount('extension:unloaded')).toBe(0);
  });
});
