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
  const documentStub: any = { body: { style: {} } };
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
