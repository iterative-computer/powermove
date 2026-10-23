// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import activate from './index';
import { fakePowermoveAPI, type FakeAPIHarness } from './fake-api.test-helper';

const harnesses: FakeAPIHarness[] = [];
beforeEach(() => {
  vi.spyOn(window, 'requestAnimationFrame').mockReturnValue(1);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as never);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
});
afterEach(() => {
  harnesses.splice(0).reverse().forEach(value => value.dispose());
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it.each(['layers', 'tracks'])('keeps exactly the selected view highlighted in %s mode through snap, mode switches and head replacement', mode => {
  const value = fakePowermoveAPI(vi); harnesses.push(value);
  value.api.storage.set('mode', mode);
  activate(value.api);
  const body = document.createElement('div'); document.body.append(body);
  value.state.panel!.build!(body, { spec: {} });
  const views = () => [...body.querySelectorAll<HTMLButtonElement>('.tl-view-button')];
  const assertSelected = (index: number) => {
    expect(views().map(button => button.classList.contains('on'))).toEqual([0, 1, 2].map(i => i === index));
    expect(views().map(button => button.getAttribute('aria-pressed'))).toEqual([0, 1, 2].map(i => String(i === index)));
    expect((body.querySelector('[title="Graph options"]') as HTMLElement).hidden).toBe(index === 0);
  };
  assertSelected(0);
  for (const index of [1, 2, 0]) {
    views()[index]!.click(); assertSelected(index);
    const timeline = value.state.timeline! as any;
    expect(timeline.graph).toBe(index !== 0);
    if (index !== 0) expect(timeline.graphCombined).toBe(index === 1);
    body.querySelector<HTMLButtonElement>('[data-mode="tracks"]')!.click(); assertSelected(index);
    const snap = body.querySelector<HTMLButtonElement>('.tl-snap')!;
    expect(snap.hidden).toBe(false);
    for (const active of [false, true]) {
      snap.click(); assertSelected(index);
      expect(snap.getAttribute('aria-pressed')).toBe(String(active));
      expect(snap.classList.contains('on')).toBe(active);
    }
    body.querySelector<HTMLButtonElement>('[data-mode="layers"]')!.click(); assertSelected(index);
    expect(snap.hidden).toBe(true);
    body.querySelector<HTMLButtonElement>(`[data-mode="${mode}"]`)!.click();
    value.state.panel!.build!(body, { spec: {} }); assertSelected(index);
  }
});
