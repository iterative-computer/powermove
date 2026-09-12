import { afterEach, describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './util';

function utilRegistry() {
  const listeners: Record<string, any[]> = {};
  const listenerOptions: Record<string, any[]> = {};
  const windowStub: any = {
    addEventListener: (type: any, fn: any, options: any) => {
      (listeners[type] = listeners[type] || []).push(fn);
      (listenerOptions[type] = listenerOptions[type] || []).push(options);
    },
    removeEventListener: (type: any, fn: any) => {
      const index = (listeners[type] || []).indexOf(fn);
      if (index >= 0) {
        listeners[type]!.splice(index, 1);
        listenerOptions[type]!.splice(index, 1);
      }
    },
    requestAnimationFrame: () => 0,
    setTimeout,
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    innerWidth: 1024,
    innerHeight: 768,
  };
  const documentStub: any = { body: { style: {} }, addEventListener: windowStub.addEventListener, removeEventListener: windowStub.removeEventListener };
  const consoleStub: any = { error: vi.fn(), warn: vi.fn() };
  windowStub.document = documentStub;
  windowStub.console = consoleStub;
  vi.stubGlobal('window', windowStub);
  vi.stubGlobal('document', documentStub);
  vi.stubGlobal('console', consoleStub);
  const PM: PMRegistry = {};
  install(PM);
  return { PM, listeners, listenerOptions };
}

function fire(listeners: Record<string, any[]>, type: string, event: any) {
  (listeners[type] || []).forEach(fn => fn(event));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('legacy util install', () => {
  it('signals rendering immediately while leaving UI work coalesced', () => {
    const { PM } = utilRegistry();
    const draw = vi.fn();
    const ui = vi.fn();
    PM.bus.on('draw', draw);
    PM.bus.on('draw:ui', ui);

    PM.invalidate('render');
    PM.invalidate('render');

    expect(draw).toHaveBeenCalledTimes(2);
    expect(ui).not.toHaveBeenCalled();
  });

  it('routes pointerup once and ignores later drag events', () => {
    const { PM, listeners } = utilRegistry();
    let ups = 0;
    let moves = 0;
    PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, {
      move: () => moves++,
      up: () => ups++,
    });

    fire(listeners, 'pointermove', { clientX: 20, clientY: 20 });
    fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });
    fire(listeners, 'pointermove', { clientX: 40, clientY: 40 });
    fire(listeners, 'pointerup', { clientX: 50, clientY: 50 });

    expect(moves).toBe(1);
    expect(ups).toBe(1);
    expect((listeners.pointermove || []).length).toBe(0);
  });

  it('cancels idempotently without committing the drag', () => {
    const { PM, listeners } = utilRegistry();
    let ups = 0;
    let cancels = 0;
    const control = PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, {
      up: () => ups++,
      cancel: () => cancels++,
    });

    control.cancel();
    control.cancel();
    fire(listeners, 'pointerup', { clientX: 30, clientY: 30 });

    expect(cancels).toBe(1);
    expect(ups).toBe(0);
    expect((listeners.pointerup || []).length).toBe(0);
  });
});

it('continues scrubbing with a stationary cursor and releases pointer lock on mouseup', async () => {
  const { PM, listeners } = utilRegistry();
  const doc = window.document as any;
  doc.body.requestPointerLock = vi.fn(() => { doc.pointerLockElement = doc.body; fire(listeners, 'pointerlockchange', {}); return Promise.resolve(); });
  doc.exitPointerLock = vi.fn(() => { doc.pointerLockElement = null; });
  const move = vi.fn(), up = vi.fn(), cancel = vi.fn();
  PM.drag({ clientX: 5, clientY: 5, preventDefault() {} }, { infinite: true, move, up, cancel });
  fire(listeners, 'pointermove', { clientX: 15, clientY: 5 });
  fire(listeners, 'mousemove', { movementX: 2000, movementY: 0 });
  expect(move).toHaveBeenLastCalledWith(2010, 0, expect.anything());
  fire(listeners, 'mousemove', { movementX: -4000, movementY: 0 });
  expect(move).toHaveBeenLastCalledWith(-1990, 0, expect.anything());
  fire(listeners, 'mouseup', {});
  expect(up).toHaveBeenCalledTimes(1);
  expect(cancel).not.toHaveBeenCalled();
  expect(doc.exitPointerLock).toHaveBeenCalledTimes(1);
});
