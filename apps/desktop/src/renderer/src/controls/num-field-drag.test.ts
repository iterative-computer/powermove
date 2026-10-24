// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import NumField from './NumField.svelte';
import type { EditBinding } from './gesture';
import type { PowermoveAPI } from '../kernel/api';
import { doc } from '../state/document.svelte';
import { makePM } from '../legacy/__tests__/make-pm';

const mounted: object[] = [];
const restoreBoundaries: (() => void)[] = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  for (const restore of restoreBoundaries.splice(0)) restore();
  document.body.replaceChildren(); vi.restoreAllMocks();
});

function fixture() {
  const PM = makePM('core/util', 'core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 2 }); PM.time = 0;
  expect(PM.Edit.apply({ type: 'add_layer', id: 'target', layerType: 'solid', content: { w: 100, h: 100 } }).ok).toBe(true);
  const value = () => PM.L('target').p['position.x'].v;
  const sync = () => { doc.proj = PM.proj; doc.bump('values'); flushSync(); };
  const api = {
    edit: {
      begin: (...args: any[]) => PM.Edit.begin(...args),
      dispatch: (command: unknown) => { const result = PM.Edit.dispatch(command); sync(); return result; },
      commit: (...args: any[]) => PM.Edit.commit(...args),
      cancel: () => { PM.Edit.cancel(); sync(); },
      apply: (...args: any[]) => { const result = PM.Edit.apply(...args); sync(); return result; }
    },
    history: PM.hist,
    ui: { drag: (...args: any[]) => PM.drag(...args) },
    util: { round: PM.round }
  } as unknown as PowermoveAPI;
  const edit: EditBinding = { mode: 'command', label: 'Position X', command: next => ({ type: 'set_property', target: 'target', path: 'position.x', value: Number(next) }) };
  const host = document.createElement('div'); document.body.append(host);
  // The inspector selects the property after the field starts its gesture.
  host.onpointerdown = () => { PM.sel.chan = 'position.x'; doc.bump('values'); };
  sync();
  mounted.push(mount(NumField, { target: host, props: { api, get: value, edit, label: 'Position X', unit: 'px' } }));
  flushSync();
  const input = host.querySelector<HTMLInputElement>('input')!;
  let locked: Element | null = null;
  const boundary = (object: object, name: string, descriptor: PropertyDescriptor) => {
    const previous = Object.getOwnPropertyDescriptor(object, name);
    Object.defineProperty(object, name, { configurable: true, ...descriptor });
    restoreBoundaries.push(() => { if (previous) Object.defineProperty(object, name, previous); else Reflect.deleteProperty(object, name); });
  };
  boundary(document, 'pointerLockElement', { get: () => locked });
  const request = vi.fn(async () => { locked = document.body; document.dispatchEvent(new Event('pointerlockchange')); });
  boundary(document.body, 'requestPointerLock', { value: request });
  const exit = vi.fn(() => { locked = null; document.dispatchEvent(new Event('pointerlockchange')); });
  boundary(document, 'exitPointerLock', { value: exit });
  const down = () => input.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, cancelable: true, button: 0, pointerType: 'mouse', pointerId: 1, clientX: 100, clientY: 50 }));
  const move = (x: number) => window.dispatchEvent(new PointerEvent('pointermove', { button: -1, buttons: 1, pointerId: 1, clientX: x, clientY: 50 }));
  const relative = (movementX: number) => window.dispatchEvent(new MouseEvent('mousemove', { buttons: 1, movementX }));
  const up = () => window.dispatchEvent(new PointerEvent('pointerup', { button: 0, pointerId: 1, clientX: 110, clientY: 50 }));
  return { PM, input, value, sync, down, move, relative, up, request, exit, start: value() };
}

it('scrubs through the real component and drag helper, locks after the threshold, and undoes one edit', () => {
  const f = fixture(); const history = f.PM.hist.list().length;
  f.down(); f.move(102);
  expect(f.request).not.toHaveBeenCalled(); expect(f.value()).toBe(f.start);
  f.move(110);
  expect(f.request).toHaveBeenCalledTimes(1); expect(document.pointerLockElement).toBe(document.body);
  f.relative(10000); expect(f.value()).toBeGreaterThan(f.start + 1000);
  f.relative(-20000); expect(f.value()).toBeLessThan(f.start - 1000);
  f.up();
  expect(f.exit).toHaveBeenCalledTimes(1); expect(document.pointerLockElement).toBeNull();
  expect(f.PM.hist.list().length).toBe(history + 1);
  expect(f.PM.hist.undo()).toBe(true); f.sync();
  expect(f.value()).toBe(f.start); expect(f.input.value).toBe(`${f.start}px`);
});

it('cancels the whole scrub when pointer lock is lost, and a click still opens text editing', async () => {
  const f = fixture(); const history = f.PM.hist.list().length;
  f.down(); f.move(110); f.relative(5000);
  expect(f.value()).toBeGreaterThan(f.start + 1000);
  document.exitPointerLock(); f.up();
  expect(f.value()).toBe(f.start); expect(f.PM.hist.list().length).toBe(history);
  f.request.mockClear(); f.down(); f.up(); await tick();
  expect(f.input.readOnly).toBe(false); expect(f.request).not.toHaveBeenCalled();
  f.input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); flushSync();
  expect(f.input.readOnly).toBe(true); expect(f.value()).toBe(f.start);
});
