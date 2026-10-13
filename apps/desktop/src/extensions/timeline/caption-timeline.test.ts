// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { createTimelineRuntime } from './timeline';
import { fakePowermoveAPI } from './fake-api.test-helper';
import { install as installUIState } from '../../renderer/src/legacy/core/ui-state';

const disposers: Array<() => void> = [];
afterEach(() => {
  disposers.splice(0).forEach(dispose => dispose());
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function harness(mode: 'layers' | 'tracks') {
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
  vi.spyOn(performance, 'now').mockReturnValue(0);
  const value = fakePowermoveAPI(vi);
  const cues = Array.from({ length: 3000 }, (_, i) => ({ id: `c${i}`, start: i * 1.5, end: i * 1.5 + 1.2, text: `Line ${i}` }));
  value.state.project.dur = 4600;
  value.state.project.layers = [
    { id: 'cap', name: 'English', type: 'captions', collapsed: true, from: 0, dur: 4600, on: true, d: { cues, style: {} }, fx: [], masks: [], p: {} },
    { id: 'clip', name: 'Interview', type: 'video', collapsed: true, from: 0, dur: 4600, on: true, d: {}, fx: [], masks: [], p: {} },
  ];
  value.api.storage.get = vi.fn((key: string) => key === 'mode' ? mode : undefined) as any;
  const PM: any = { proj: value.state.project, bus: { on() {} } };
  installUIState(PM);
  Object.assign(value.api.uiState, PM.UIState);
  const text = vi.fn();
  const context = new Proxy({ fillText: text, measureText: () => ({ width: 10 }), createLinearGradient: () => ({ addColorStop() {} }) }, {
    get(target, key) { return key in target ? target[key as keyof typeof target] : () => {}; },
    set(target, key, next) { (target as any)[key] = next; return true; },
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as any);
  const timeline = createTimelineRuntime(value.api);
  timeline.cv = document.createElement('canvas');
  timeline.ctx = context;
  timeline.w = 1200; timeline.hgt = 320;
  timeline.cv.width = 1200; timeline.cv.height = 320;
  disposers.push(() => timeline.disposeRuntime());
  return { ...value, timeline, text };
}

it.each(['layers', 'tracks'] as const)('draws only the visible cues of a 3000-cue captions layer in the %s view', mode => {
  const value = harness(mode);
  value.timeline.pps = 60;
  value.timeline.scrollT = 2000;
  value.text.mockClear();
  value.emit('invalidate', 'timeline');
  const lines = value.text.mock.calls.map(([label]) => String(label)).filter(label => label.startsWith('Line '));
  // ~1000px of lane at 60px/s is ~16s, ~11 cues.
  expect(lines.length).toBeGreaterThan(5);
  expect(lines.length).toBeLessThan(20);
  expect(lines).toContain('Line 1340');
  if (mode === 'tracks') expect(value.text.mock.calls.some(([label]) => label === 'C1')).toBe(true);
  const started = Date.now();
  for (let frame = 0; frame < 60; frame++) { value.timeline.scrollT += .05; value.emit('invalidate', 'timeline'); }
  expect(Date.now() - started).toBeLessThan(1500);
});
