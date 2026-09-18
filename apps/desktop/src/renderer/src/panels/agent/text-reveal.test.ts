// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { revealText } from './text-reveal';
beforeEach(() => { Object.defineProperty(HTMLElement.prototype, 'animate', { configurable: true, writable: true, value: vi.fn() }); });
afterEach(() => { vi.restoreAllMocks(); delete (HTMLElement.prototype as any).animate; });
it('staggers new words without replaying existing words or changing inline layout', async () => {
  const parent = document.createElement('p');
  const animations: any[] = [];
  vi.spyOn(HTMLElement.prototype, 'animate').mockImplementation((frames, options) => {
    animations.push({ frames, options });
    return { cancel: vi.fn() } as any;
  });
  const words = ['Hello', 'world', 'again'].map(text => {
    const span = parent.appendChild(document.createElement('span'));
    span.textContent = text;
    return span;
  });
  const actions = words.map(node => revealText(node));
  expect(animations.map(a => a.options.delay)).toEqual([0, 28, 56]);
  expect(animations.every(a => a.frames.every((f: any) => !f.transform && !f.filter))).toBe(true);
  words[2]!.textContent += '!';
  expect(actions.every(a => !('update' in a))).toBe(true);
  expect(animations).toHaveLength(3);
  await Promise.resolve();
  revealText(parent.appendChild(document.createElement('span')));
  expect(animations[3].options.delay).toBe(0);
  actions.forEach(a => a.destroy());
});
it('cancels the reveal when reduced motion is enabled', () => {
  let listener: (() => void) | undefined;
  const media = { matches: false, addEventListener: (_: string, fn: () => void) => { listener = fn; }, removeEventListener: vi.fn() };
  vi.spyOn(window, 'matchMedia').mockReturnValue(media as any);
  const cancel = vi.fn();
  const animate = vi.spyOn(HTMLElement.prototype, 'animate').mockReturnValue({ cancel } as any);
  const node = document.createElement('div');
  const action = revealText(node);
  expect(animate).toHaveBeenCalledOnce();
  expect(action).not.toHaveProperty('update');
  media.matches = true;
  listener!();
  expect(cancel).toHaveBeenCalledOnce();
  revealText(node);
  expect(animate).toHaveBeenCalledOnce();
  action.destroy();
});
