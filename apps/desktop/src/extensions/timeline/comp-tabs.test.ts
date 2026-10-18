// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import type { PowermoveAPI } from 'powermove';

import { mountCompTabs } from './comp-tabs';

function setup() {
  const comps = [{ id: 'A', name: 'Main' }, { id: 'B', name: 'Precomp' }];
  let open = 'A';
  let tabs = ['A', 'B'];
  const changes = new Set<() => void>();
  const projectListeners = new Set<(payload: { kind: string }) => void>();
  const model = {
    openTabs: () => tabs,
    get: (id: string) => comps.find((c) => c.id === id) ?? null,
    isOpen: (id: string) => id === open,
    open: (id: string) => { open = id; changes.forEach((fn) => fn()); return true; },
    close: (id: string) => { tabs = tabs.filter((t) => t !== id); return true; },
    onChange: (fn: () => void) => { changes.add(fn); return () => changes.delete(fn); },
  };
  const api = {
    services: { get: () => model },
    events: { on: (_: string, fn: (payload: { kind: string }) => void) => { projectListeners.add(fn); return { dispose: () => projectListeners.delete(fn) }; } },
    ui: { menu: () => undefined },
    commands: { run: () => undefined },
  } as unknown as PowermoveAPI;
  const host = document.createElement('div');
  document.body.append(host);
  let scrolls = 0;
  const original = Element.prototype.scrollIntoView;
  Element.prototype.scrollIntoView = function () { scrolls++; };
  const dispose = mountCompTabs(api, host);
  const changed = (kind = 'layers') => projectListeners.forEach((fn) => fn({ kind }));
  return {
    host, comps, changed, model,
    scrolls: () => scrolls,
    restore: () => { Element.prototype.scrollIntoView = original; dispose(); host.remove(); },
  };
}

describe('composition tabs', () => {
  it('keep the strip as it is when a project change leaves the tabs unchanged', () => {
    const t = setup();
    try {
      const first = [...t.host.children];
      expect(first.map((e) => (e as HTMLElement).dataset.compId)).toEqual(['A', 'B']);
      expect(t.scrolls()).toBe(1);
      // An edit announces history, layers, and project changes: nothing to redraw.
      t.changed('history');
      t.changed('layers');
      t.changed('project');
      expect([...t.host.children]).toEqual(first);
      expect(t.scrolls()).toBe(1);
    } finally {
      t.restore();
    }
  });

  it('rebuild when a name, the open composition, or the set of tabs changes', () => {
    const t = setup();
    try {
      t.comps[1]!.name = 'Renamed';
      t.changed('project');
      expect((t.host.children[1] as HTMLElement).title).toBe('Renamed');
      t.model.open('B');
      expect(t.host.children[1]!.getAttribute('aria-selected')).toBe('true');
      expect(t.scrolls()).toBe(3);
      t.model.close('A');
      t.changed('project');
      expect([...t.host.children].map((e) => (e as HTMLElement).dataset.compId)).toEqual(['B']);
      // A single tab has no close button.
      expect(t.host.querySelector('button')).toBeNull();
    } finally {
      t.restore();
    }
  });
});
