// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { roundedScroll } from './rounded-scroll';

/*
 * The side columns hide their scrollbars, so the only cue that a dock scrolls
 * is the edge fade CSS keys off these flags. Regression guard for both the fade
 * and the overflow it advertises.
 */
function dockElement(scrollHeight: number, clientHeight: number): HTMLElement {
  const node = document.createElement('div');
  node.id = 'dock-right';
  node.className = 'dock col';
  document.body.appendChild(node);
  Object.defineProperty(node, 'scrollHeight', { configurable: true, value: scrollHeight });
  Object.defineProperty(node, 'clientHeight', { configurable: true, value: clientHeight });
  return node;
}

let handle: { destroy(): void } | undefined;

beforeEach(() => { document.body.innerHTML = ''; });
afterEach(() => { handle?.destroy(); handle = undefined; });

const settle = async () => { await new Promise((resolve) => requestAnimationFrame(resolve)); };

describe('dock scroll edges', () => {
  it('marks neither edge when the column fits', async () => {
    const node = dockElement(400, 400);
    handle = roundedScroll(node);
    await settle();
    expect(node.dataset.overflowTop).toBe('0');
    expect(node.dataset.overflowBottom).toBe('0');
  });

  it('fades only the bottom at the top of a scrollable column', async () => {
    const node = dockElement(900, 400);
    handle = roundedScroll(node);
    await settle();
    expect(node.dataset.overflowTop).toBe('0');
    expect(node.dataset.overflowBottom).toBe('1');
  });

  it('fades both edges mid-scroll and only the top at the end', async () => {
    const node = dockElement(900, 400);
    handle = roundedScroll(node);
    await settle();

    node.scrollTop = 250;
    node.dispatchEvent(new Event('scroll'));
    await settle();
    expect(node.dataset.overflowTop).toBe('1');
    expect(node.dataset.overflowBottom).toBe('1');

    node.scrollTop = 500; // scrollHeight - clientHeight
    node.dispatchEvent(new Event('scroll'));
    await settle();
    expect(node.dataset.overflowTop).toBe('1');
    expect(node.dataset.overflowBottom).toBe('0');
  });

  it('clears its flags on destroy', async () => {
    const node = dockElement(900, 400);
    const active = roundedScroll(node);
    // roundedScroll opts out of #dock-center entirely; a side dock always gets one.
    expect(active).toBeDefined();
    await settle();
    active!.destroy();
    expect(node.dataset.overflowTop).toBeUndefined();
    expect(node.dataset.overflowBottom).toBeUndefined();
  });
});
