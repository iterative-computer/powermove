// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import ColorField from './ColorField.svelte';
import { installPointerFocus } from './pointer-focus';
import { createKernel } from '../kernel/registries';
import type { PowermoveAPI } from '../kernel/api';
import { doc } from '../state/document.svelte';
import { makePM } from '../legacy/__tests__/make-pm';

const mounted: object[] = [];
const cleanup: (() => void)[] = [];
afterEach(async () => {
  for (const dispose of cleanup.splice(0)) dispose();
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  Reflect.deleteProperty(window, 'EyeDropper');
});

async function fixture(sample: () => Promise<{ sRGBHex: string }>) {
  vi.stubGlobal('matchMedia', () => ({ matches: true }));
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 2 }); PM.time = 0;
  expect(PM.Edit.apply({ type: 'add_layer', id: 'shape', layerType: 'shape', content: { color: '#FF6B1A' }, select: true }).ok).toBe(true);
  const sync = () => { doc.proj = PM.proj; doc.bump('values'); flushSync(); };
  const api = {
    edit: PM.Edit, history: PM.hist,
    transport: { invalidate: sync }, ui: { toast: vi.fn() }
  } as unknown as PowermoveAPI;
  const kernel = createKernel();
  kernel.bind('editor', { key: 'escape', command: 'deselect' });
  const keyListener = kernel.installKeyListener(() => { PM.selectLayers([]); sync(); });
  cleanup.push(() => keyListener.dispose(), installPointerFocus());
  Object.defineProperty(window, 'EyeDropper', { configurable: true, value: class { open = sample; } });
  sync();
  mounted.push(mount(ColorField, { target: document.body, props: {
    api, label: 'Fill', get: () => PM.L('shape').d.color,
    edit: { mode: 'command', label: 'Fill', command: value => ({ type: 'set_content', target: 'shape', patch: { color: String(value) } }) }
  } }));
  flushSync();
  const show = async () => { document.querySelector<HTMLButtonElement>('.color-field')!.click(); await tick(); };
  const picker = () => document.querySelector<HTMLElement>('.color-picker')!;
  const sampleClick = () => {
    const eyedropper = picker().querySelector<HTMLButtonElement>('[aria-label="Sample screen color"]')!;
    // Chromium focuses the pressed button before click; the global policy then
    // releases pointer focus so Space won't re-activate an action later.
    eyedropper.focus();
    eyedropper.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));
  };
  const close = async () => {
    document.querySelector<HTMLElement>('.color-picker-layer')!.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    await tick();
  };
  await show();
  return { PM, api, picker, show, sampleClick, close, history: PM.hist.list().length };
}

it.each(['sample', 'abort', 'error'])('keeps Escape in the color picker after a pointer-clicked eyedropper %s', async outcome => {
  const sample = vi.fn(async () => {
    if (outcome === 'abort') throw new DOMException('Canceled', 'AbortError');
    if (outcome === 'error') throw new Error('Native sampling failed');
    return { sRGBHex: '#0a84ff' };
  });
  const f = await fixture(sample);
  f.sampleClick();
  await tick();
  expect(sample).toHaveBeenCalledOnce();
  expect(f.PM.L('shape').d.color).toBe(outcome === 'sample' ? '#0A84FF' : '#FF6B1A');
  expect(f.api.ui.toast).toHaveBeenCalledTimes(outcome === 'error' ? 1 : 0);
  if (outcome === 'error') expect(f.api.ui.toast).toHaveBeenCalledWith('Could not sample that color');
  const focusStayedInPicker = f.picker().contains(document.activeElement);
  document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
  await tick();
  expect(f.PM.sel.layers).toEqual(['shape']);
  expect(focusStayedInPicker).toBe(true);
  expect(f.PM.L('shape').d.color).toBe('#FF6B1A');
  expect(f.PM.hist.list().length).toBe(f.history);
  expect(f.PM.hist.busy()).toBe(false);
  expect(document.querySelector('.color-picker')).toBeNull();
});

it.each(['closed', 'reopened', 'unmounted'])('ignores a pending sample from a %s picker without stealing focus', async state => {
  const resolve: ((color: { sRGBHex: string }) => void)[] = [];
  const sample = vi.fn(() => new Promise<{ sRGBHex: string }>(done => { resolve.push(done); }));
  const f = await fixture(sample);
  f.sampleClick();
  await tick();
  await f.close();
  if (state === 'reopened') {
    await f.show();
    f.sampleClick();
    expect(sample).toHaveBeenCalledTimes(2);
  }
  if (state === 'unmounted') await unmount(mounted.pop()!);
  const focus = state === 'reopened' ? f.picker().querySelector<HTMLElement>('.cp-plane')! : document.createElement('input');
  if (state !== 'reopened') document.body.append(focus);
  focus.focus();
  resolve[0]!({ sRGBHex: '#0a84ff' }); await tick();
  expect(document.activeElement).toBe(focus);
  expect(f.PM.L('shape').d.color).toBe('#FF6B1A');
  expect(f.PM.hist.list().length).toBe(f.history);
  expect(f.PM.hist.busy()).toBe(false);
  if (state === 'reopened') {
    f.sampleClick();
    // Finishing the older session must not clear the new session's busy flag.
    expect(sample).toHaveBeenCalledTimes(2);
    resolve[1]!({ sRGBHex: '#34c759' }); await tick();
    expect(f.PM.L('shape').d.color).toBe('#34C759');
    document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await tick();
    expect(f.PM.L('shape').d.color).toBe('#FF6B1A');
  }
});
