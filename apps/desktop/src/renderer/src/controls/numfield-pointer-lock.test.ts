// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import type { PMRegistry } from '../legacy/registry';
import { install } from '../legacy/core/util';
import NumField from './NumField.svelte';
import type { EditBinding } from './gesture';
import type { PowermoveAPI } from '../kernel/api';

const mounted: object[] = [];
afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  Reflect.deleteProperty(document.body, 'requestPointerLock');
  Reflect.deleteProperty(document, 'pointerLockElement');
  Reflect.deleteProperty(document, 'exitPointerLock');
});

/* NumField scrubbing through the real legacy drag helper, with pointer lock as
   Chromium (WebView2) grants it: lock arrives after a 3px drag, then motion is
   reported only as movementX/movementY. */
function scrubField(initial: number) {
  const PM: PMRegistry = {};
  install(PM);
  let current = initial;
  const Edit = {
    begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(),
    dispatch: vi.fn((command: { value: number }) => { current = command.value; }),
    apply: vi.fn(() => ({ ok: true }))
  };
  const api = {
    edit: Edit,
    ui: { drag: (event: PointerEvent, options: unknown) => PM.drag(event, options) },
    util: { round: (value: number, precision: number) => Number(value.toFixed(precision)) }
  } as unknown as PowermoveAPI;
  let locked: Element | null = null;
  Object.defineProperty(document, 'pointerLockElement', { configurable: true, get: () => locked });
  Object.defineProperty(document, 'exitPointerLock', { configurable: true, value: () => { locked = null; document.dispatchEvent(new Event('pointerlockchange')); } });
  Object.defineProperty(document.body, 'requestPointerLock', { configurable: true, value: () => {
    locked = document.body;
    document.dispatchEvent(new Event('pointerlockchange'));
    return Promise.resolve();
  } });
  const target = document.createElement('div');
  document.body.append(target);
  mounted.push(mount(NumField, { target, props: {
    api, get: () => current, min: 0, max: 100, step: 1, label: 'Opacity',
    edit: { mode: 'command', label: 'Opacity', command: (value: unknown) => ({ type: 'set_property', target: 'L1', path: 'opacity', value: value as number }) } satisfies EditBinding
  } }));
  flushSync();
  const input = target.querySelector('input')!;
  const lockedMove = (movementX: number, movementY = 0) => {
    const event = new MouseEvent('mousemove', { bubbles: true });
    Object.defineProperties(event, { movementX: { value: movementX }, movementY: { value: movementY } });
    window.dispatchEvent(event);
  };
  const press = () => {
    input.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, pointerId: 1, clientX: 200, clientY: 20 }));
    window.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, pointerId: 1, clientX: 204, clientY: 20 }));
  };
  const release = () => window.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
  return { Edit, value: () => current, press, lockedMove, release };
}

it('ignores Chromium pointer-lock movement spikes instead of scrubbing to the limit', () => {
  const field = scrubField(50);
  field.press();
  expect(field.value()).toBe(52);
  // Chromium on Windows reports spurious movementX near half the window width
  // right after locking and with high-polling mice (crbug.com/781182).
  field.lockedMove(445);
  expect(field.value()).toBe(52);
  field.lockedMove(-445);
  expect(field.value()).toBe(52);
  field.lockedMove(2);
  expect(field.value()).toBe(53);
  field.release();
  expect(field.Edit.commit).toHaveBeenCalledTimes(1);
  expect(field.value()).toBe(53);
});

it('keeps unlimited travel for real locked drags', () => {
  const field = scrubField(50);
  field.press();
  for (let i = 0; i < 60; i++) field.lockedMove(3);
  expect(field.value()).toBe(100);
  for (let i = 0; i < 300; i++) field.lockedMove(-3);
  expect(field.value()).toBe(0);
  field.release();
  expect(field.Edit.commit).toHaveBeenCalledTimes(1);
});
