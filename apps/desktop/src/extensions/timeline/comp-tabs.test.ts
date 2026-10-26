// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { mountCompTabs } from './comp-tabs';

afterEach(() => { document.body.replaceChildren(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

function harness() {
  let active = 'main', next = 0;
  let changed!: () => void, event!: (value: { kind: string }) => void;
  const tabs = [{ id: 'main', name: 'Main' }, { id: 'child', name: 'Nested composition' }];
  const frames = new Map<number, FrameRequestCallback>();
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++next, callback); return next; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  const model = {
    openTabs: () => tabs.map(tab => tab.id), get: (id: string) => tabs.find(tab => tab.id === id),
    isOpen: (id: string) => id === active,
    open: vi.fn((id: string) => { active = id; changed(); return true; }), close: vi.fn(),
    onChange: (listener: () => void) => { changed = listener; return vi.fn(); },
  };
  const dispose = vi.fn();
  const api = { ui: { icon: () => '<svg></svg>' }, services: { get: () => model }, events: { on: (_name: string, listener: typeof event) => { event = listener; return { dispose }; } } };
  const host = document.createElement('div'); document.body.append(host);
  const unmount = mountCompTabs(api as any, host);
  const flush = () => { const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach(callback => callback(0)); };
  flush();
  return { host, model, tabs, changed, event, frames, flush, unmount, dispose };
}

it('coalesces project-switch notifications and retains unchanged tab DOM and focus', () => {
  const h = harness(), replace = vi.spyOn(h.host, 'replaceChildren');
  const first = h.host.firstElementChild as HTMLElement; first.focus();
  for (let n = 0; n < 8; n++) { h.event({ kind: 'structure' }); h.changed(); }
  expect(h.frames.size).toBe(1); h.flush();
  expect(replace).not.toHaveBeenCalled(); expect(document.activeElement).toBe(first);
  h.tabs[0]!.name = 'Renamed'; h.changed(); h.changed(); h.flush();
  expect(replace).toHaveBeenCalledOnce(); expect(h.host.textContent).toContain('Renamed');
  h.unmount();
});

it('switches nested composition tabs by pointer and keyboard without scrolling parent panels', () => {
  const h = harness();
  const scroll = vi.spyOn(HTMLElement.prototype, 'scrollIntoView').mockImplementation(() => {});
  h.host.querySelector('[data-comp-id="child"]')!.dispatchEvent(new PointerEvent('pointerdown', { button: 0 }));
  h.flush(); h.flush();
  expect(h.host.querySelector('[aria-selected="true"]')?.getAttribute('data-comp-id')).toBe('child');
  h.host.querySelector('[data-comp-id="main"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
  h.flush(); h.flush();
  expect(h.host.querySelector('[aria-selected="true"]')?.getAttribute('data-comp-id')).toBe('main');
  expect(scroll).not.toHaveBeenCalled(); h.unmount();
});

it('cancels deferred work and unsubscribes on disposal', () => {
  const h = harness(); h.changed(); expect(h.frames.size).toBe(1);
  h.unmount(); expect(h.frames.size).toBe(0); expect(h.dispose).toHaveBeenCalledOnce();
});
