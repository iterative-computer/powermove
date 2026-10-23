// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { growPanelPair } from './geometry';
import { panelFlex, panelMinHeight, SIDE_FLUID_MIN_HEIGHT } from './model';
import { routeSideWheel, sideDockBeside } from './side-scroll';

function scroller(element: HTMLElement, scrollHeight: number, clientHeight: number, scrollTop = 0): HTMLElement {
  Object.defineProperty(element, 'scrollHeight', { configurable: true, value: scrollHeight });
  Object.defineProperty(element, 'clientHeight', { configurable: true, value: clientHeight });
  element.scrollTop = scrollTop;
  return element;
}

function column() {
  document.body.innerHTML = '<div class="dock" data-dock="left"><div class="panel"><div class="body"><div class="row"></div></div></div></div>';
  const dock = scroller(document.querySelector<HTMLElement>('.dock')!, 900, 400);
  dock.style.overflowY = 'auto';
  const body = document.querySelector<HTMLElement>('.body')!;
  body.style.overflowY = 'auto';
  return { dock, body, row: document.querySelector<HTMLElement>('.row')! };
}

const wheel = (target: Element, deltaY: number, modifiers: Partial<Record<'ctrlKey' | 'metaKey' | 'shiftKey', boolean>> = {}) => {
  const event = new WheelEvent('wheel', { deltaY, bubbles: true, cancelable: true });
  // happy-dom drops modifier keys from the init dictionary.
  for (const [key, value] of Object.entries({ target, ...modifiers })) Object.defineProperty(event, key, { value });
  return event;
};

describe('side column wheel routing', () => {
  it('leaves the wheel to a panel that can still scroll that way', () => {
    const { dock, body, row } = column();
    scroller(body, 600, 200, 100);
    const event = wheel(row, 120);
    routeSideWheel(dock, event);
    expect(event.defaultPrevented).toBe(false);
    expect(dock.scrollTop).toBe(0);
  });

  it('moves the column as soon as the panel under the pointer reaches its end', () => {
    const { dock, body, row } = column();
    scroller(body, 600, 200, 400);
    const event = wheel(row, 120);
    routeSideWheel(dock, event);
    expect(event.defaultPrevented).toBe(true);
    expect(dock.scrollTop).toBe(120);
  });

  it('keeps zoom, horizontal and already handled wheels away from the column', () => {
    const { dock, row } = column();
    for (const init of [{ ctrlKey: true }, { metaKey: true }, { shiftKey: true }]) routeSideWheel(dock, wheel(row, 120, init));
    const handled = wheel(row, 120);
    handled.preventDefault();
    routeSideWheel(dock, handled);
    expect(dock.scrollTop).toBe(0);
  });

  it('finds the column beside a gutter point on its own outer side only', () => {
    const { dock } = column();
    dock.getBoundingClientRect = () => ({ left: 12, right: 312, top: 44, bottom: 524 }) as DOMRect;
    expect(sideDockBeside(4, 200, [dock])).toBe(dock);
    expect(sideDockBeside(320, 200, [dock])).toBeNull();
    expect(sideDockBeside(4, 600, [dock])).toBeNull();
  });
});

describe('side column panel floor', () => {
  it('stops fluid side panels at a usable height but keeps fixed and center floors', () => {
    expect(panelMinHeight({ id: 'inspector', flex: true }, {}, 'right')).toBe(SIDE_FLUID_MIN_HEIGHT);
    expect(panelMinHeight({ id: 'assets', size: 220 }, {}, 'left')).toBe(88);
    expect(panelMinHeight({ id: 'viewer', flex: true }, {}, 'center')).toBe(88);
    expect(panelMinHeight({ id: 'tall', flex: true, min: 260 }, {}, 'left')).toBe(260);
  });
});

describe('side column panel length', () => {
  it('lets a divider lengthen a panel past the column instead of capping the pair', () => {
    // Dragging down 800px: the panel below stops at its floor, the one above keeps growing.
    expect(growPanelPair(300, 250, 800, 88, 200)).toEqual({ before: 1100, after: 200 });
    // Dragging up trades height to the panel below, so the divider tracks the pointer.
    expect(growPanelPair(300, 250, -100, 88, 200)).toEqual({ before: 200, after: 350 });
    expect(growPanelPair(300, 250, -400, 120, 200)).toEqual({ before: 120, after: 430 });
  });

  it('fills spare height with a fluid side panel without shrinking it below its set height', () => {
    expect(panelFlex({ id: 'assets', flex: true, size: 640 }, {}, 'left')).toBe('1 0 640px');
    expect(panelFlex({ id: 'assets', flex: true }, {}, 'right')).toBe(`1 0 ${SIDE_FLUID_MIN_HEIGHT}px`);
    expect(panelFlex({ id: 'assets', flex: true, size: 640 }, {}, 'center')).toBe('1 1 auto');
    expect(panelFlex({ id: 'takes', size: 260 }, {}, 'left')).toBe('0 0 260px');
  });
});
