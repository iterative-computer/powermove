// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { installTooltips, parseTip } from './tooltips';

let dispose: (() => void) | undefined;
beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  dispose?.();
  document.body.replaceChildren();
  vi.useRealTimers();
});

const button = (title: string) => {
  const b = document.createElement('button');
  b.title = title;
  b.textContent = title.slice(0, 3);
  document.body.append(b);
  return b;
};
const over = (el: Element, from: Element | null = null) => {
  el.dispatchEvent(new PointerEvent('pointerover', { bubbles: true, relatedTarget: from, pointerType: 'mouse' }));
};
const out = (el: Element, to: Element | null) => {
  el.dispatchEvent(new PointerEvent('pointerout', { bubbles: true, relatedTarget: to, pointerType: 'mouse' }));
};
const tip = () => document.getElementById('pm-tooltip');
const shown = () => !!tip()?.classList.contains('on');

it('reads a name, a hint and a shortcut from a title', () => {
  expect(parseTip('Frame — Fit the selection, or everything (F)')).toEqual({ name: 'Frame', hint: 'Fit the selection, or everything', key: 'F' });
  expect(parseTip('Undo (⌘Z)')).toEqual({ name: 'Undo', hint: '', key: '⌘Z' });
  expect(parseTip('Composition zoom')).toEqual({ name: 'Composition zoom', hint: '', key: '' });
  // Parentheses that are not a shortcut stay in the text.
  expect(parseTip('Export (all frames)').name).toBe('Export (all frames)');
  expect(parseTip('Scale', 'S').key).toBe('S');
});

it('shows after a short wait, then moves along a row at once, and holds titles aside', () => {
  dispose = installTooltips();
  const a = button('Move — Drag the selection'), b = button('Rotate — Turn the selection');
  over(a);
  // The browser's own tip never appears: the title is held while hovered.
  expect(a.hasAttribute('title')).toBe(false);
  expect(a.getAttribute('aria-describedby')).toBe('pm-tooltip');
  expect(shown()).toBe(false);
  vi.advanceTimersByTime(400);
  expect(shown()).toBe(true);
  expect(tip()!.textContent).toContain('Move');
  expect(tip()!.textContent).toContain('Drag the selection');
  // The neighbour shows immediately; the first control gets its title back.
  out(a, b); over(b, a);
  expect(shown()).toBe(true);
  expect(tip()!.textContent).toContain('Rotate');
  expect(a.getAttribute('title')).toBe('Move — Drag the selection');
  expect(a.hasAttribute('aria-describedby')).toBe(false);
  // Leaving, then coming back soon: still warm.
  out(b, document.body);
  expect(shown()).toBe(false);
  expect(b.getAttribute('title')).toBe('Rotate — Turn the selection');
  vi.advanceTimersByTime(200);
  over(a);
  expect(shown()).toBe(true);
  // A click hides it.
  a.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
  expect(shown()).toBe(false);
  expect(a.getAttribute('title')).toBe('Move — Drag the selection');
});

it('waits again after a pause, and ignores controls without a title', () => {
  dispose = installTooltips();
  const a = button('Pan'), plain = document.createElement('button');
  document.body.append(plain);
  over(a); vi.advanceTimersByTime(400); expect(shown()).toBe(true);
  out(a, document.body);
  vi.advanceTimersByTime(2000);
  over(a);
  expect(shown()).toBe(false);
  vi.advanceTimersByTime(400);
  expect(shown()).toBe(true);
  out(a, plain); over(plain, a);
  expect(shown()).toBe(false);
});

it('picks up a title that arrives under the pointer, and leaves iframes alone', async () => {
  dispose = installTooltips();
  const canvas = document.createElement('canvas');
  document.body.append(canvas);
  over(canvas);
  canvas.title = 'Unlock aspect ratio';
  await Promise.resolve();
  // Held at once (no browser tip), shown after the usual wait.
  expect(canvas.hasAttribute('title')).toBe(false);
  expect(canvas.getAttribute('data-tip-title')).toBe('Unlock aspect ratio');
  vi.advanceTimersByTime(400);
  expect(shown()).toBe(true);
  const frame = document.createElement('iframe');
  frame.title = 'Timing & easing';
  document.body.append(frame);
  out(canvas, frame); over(frame, canvas);
  expect(frame.getAttribute('title')).toBe('Timing & easing');
  expect(canvas.getAttribute('title')).toBe('Unlock aspect ratio');
});
