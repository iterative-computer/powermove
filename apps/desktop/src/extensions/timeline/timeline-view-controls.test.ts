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

it.each(['layers', 'tracks'])('keeps the Graph Editor switch in step in %s mode through snap, mode switches and head replacement', mode => {
  const value = fakePowermoveAPI(vi); harnesses.push(value);
  value.api.storage.set('mode', mode);
  activate(value.api);
  const body = document.createElement('div'); document.body.append(body);
  value.state.panel!.build!(body, { spec: {} });
  const toggle = () => body.querySelector<HTMLButtonElement>('.tl-graph-toggle')!;
  const assertShown = (shown: boolean) => {
    expect(body.querySelectorAll('.tl-graph-toggle')).toHaveLength(1);
    expect(toggle().classList.contains('on')).toBe(shown);
    expect(toggle().getAttribute('aria-pressed')).toBe(String(shown));
    expect((body.querySelector('[title="Graph options"]') as HTMLElement).hidden).toBe(!shown);
  };
  assertShown(false);
  for (const shown of [true, false]) {
    toggle().click(); assertShown(shown);
    expect((value.state.timeline! as any).graph).toBe(shown);
    body.querySelector<HTMLButtonElement>('[data-mode="tracks"]')!.click(); assertShown(shown);
    const snap = body.querySelector<HTMLButtonElement>('.tl-snap')!;
    expect(snap.hidden).toBe(false);
    for (const active of [false, true]) {
      snap.click(); assertShown(shown);
      expect(snap.getAttribute('aria-pressed')).toBe(String(active));
      expect(snap.classList.contains('on')).toBe(active);
    }
    body.querySelector<HTMLButtonElement>('[data-mode="layers"]')!.click(); assertShown(shown);
    expect(snap.hidden).toBe(true);
    body.querySelector<HTMLButtonElement>(`[data-mode="${mode}"]`)!.click();
    value.state.panel!.build!(body, { spec: {} }); assertShown(shown);
  }
});
